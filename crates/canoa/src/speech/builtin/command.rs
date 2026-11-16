//! The command provider: a program the person names, audio on its standard input, text on its standard output.
//!
//! The program is started, through the spawner the host hands the kind, when a recording opens and ends when it finishes. Its standard input gets the audio as it
//! arrives, so a program that reads a stream starts work at once, and a pipe that is full slows the caller down instead of
//! growing a queue here. Its standard output is read as it is written and kept up to [`MAX_OUTPUT`] bytes.

use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use coqui_acp_api::{Environment, ProcessSpawner, SpawnSpec, SpawnedProcess};
use guatiao::library::{Object, ProviderError};
use guatiao::{FromValue, Schema, Status, ToValue};
use guatiao_intake::Form;

use super::{named_hints, wav_header};
use crate::speech::{
    Heard, OpenRequest, PCM_S16LE, SpeechInfo, SpeechProvider, SpeechSession, Transcript,
    bad_value, internal, unsupported_format,
};

/// The most standard output kept, in bytes. More is dropped, and the transcript says so in its front matter.
pub const MAX_OUTPUT: usize = 4 << 20;

/// The most standard error kept, in bytes, for an error message: the last of it.
const MAX_ERROR: usize = 2048;

/// How long, after the program exits, its output is waited for: something it started may still hold the pipe.
const OUTPUT_GRACE: Duration = Duration::from_secs(2);

/// How the audio is written to the program.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Copy, Default, PartialEq, Eq)]
#[map(rename_all = "snake_case")]
pub enum CommandInput {
    /// A WAV stream: a header whose sizes say "until the end" and then the samples. What `ffmpeg -i -` and most
    /// command line recognizers read.
    #[default]
    Wav,
    /// The samples alone, 16-bit little-endian, at the rate and channels the recording opened with.
    Pcm,
}

const fn default_input() -> CommandInput {
    CommandInput::Wav
}

const fn default_timeout() -> u32 {
    120
}

/// What the command provider is configured with.
///
/// Each of `args` may name `{language}`, `{vocabulary}`, `{accent}`, `{voice}`, `{rate}` and `{channels}`, which are
/// replaced with what the recording opened with (an absent hint is the empty text). The hints that are present are also set
/// as the variables `CANOA_SPEECH_LANGUAGE`, `CANOA_SPEECH_VOCABULARY`, `CANOA_SPEECH_ACCENT` and `CANOA_SPEECH_VOICE`.
#[derive(ToValue, FromValue, Schema, Form, Debug, Clone, PartialEq)]
#[form(section(id = "command", label = "Command"))]
#[non_exhaustive]
pub struct CommandSettings {
    /// The program to run: a name found on the search path, or a path.
    #[schema(section = "command", title = "Program")]
    #[form(placeholder = "whisper-cli")]
    pub program: String,
    /// Its arguments, one per entry.
    #[map(default)]
    #[schema(section = "command", title = "Arguments")]
    pub args: Vec<String>,
    /// How it wants the audio.
    #[map(default = default_input())]
    #[schema(section = "command", title = "Audio on standard input")]
    pub input: CommandInput,
    /// Whether each change in its output while the person speaks is shown as it appears; off, the text comes when they stop.
    #[map(default)]
    #[schema(section = "command", title = "Show text while speaking")]
    pub partials: bool,
    /// How long it may take after the audio ends, in seconds, before it is stopped.
    #[map(default = default_timeout())]
    #[schema(advanced, section = "command", title = "Time to finish")]
    #[form(unit = "s")]
    pub timeout_seconds: u32,
}

impl CommandSettings {
    /// Settings that run `program` with no arguments, WAV on its input, no partial text and two minutes to finish.
    #[must_use]
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            input: default_input(),
            partials: false,
            timeout_seconds: default_timeout(),
        }
    }
}

/// The command provider.
#[derive(guatiao::Provider)]
#[provider(SpeechProvider, id = "speech_command", config = CommandSettings, form = CommandSettings)]
pub struct CommandSpeech {
    settings: CommandSettings,
}

impl From<CommandSettings> for CommandSpeech {
    fn from(settings: CommandSettings) -> Self {
        Self { settings }
    }
}

impl SpeechProvider for CommandSpeech {
    fn describe(&self) -> Result<SpeechInfo, ProviderError> {
        let mut info = SpeechInfo::new("Command", vec![PCM_S16LE.to_string()]);
        info.description = Some("Runs a program that reads audio and writes text".to_string());
        info.partials = self.settings.partials;
        info.hints = ["language", "vocabulary", "profile"]
            .map(ToString::to_string)
            .to_vec();
        Ok(info)
    }

    fn open(
        &self,
        request: OpenRequest,
        spawner: Object<dyn ProcessSpawner>,
    ) -> Result<Object<dyn SpeechSession>, ProviderError> {
        if request.format.encoding != PCM_S16LE {
            return Err(unsupported_format(&request.format, &[PCM_S16LE]));
        }
        if self.settings.program.trim().is_empty() {
            return Err(bad_value("the command to run is not set"));
        }
        let session = Running::start(&self.settings, &request, &spawner)?;
        Ok(session.into_object())
    }
}

/// What the program has written so far.
#[derive(Default)]
struct Capture {
    bytes: Vec<u8>,
    dropped: bool,
}

type Process = Arc<Object<dyn SpawnedProcess>>;

/// One program, running for one recording.
struct Running {
    process: Process,
    /// Whether the program still takes input: false once it closed it or stopped reading.
    input: Mutex<bool>,
    out: Arc<Mutex<Capture>>,
    err: Arc<Mutex<Vec<u8>>>,
    readers: Mutex<Option<Receiver<()>>>,
    stopped: Arc<AtomicBool>,
    partials: bool,
    shown: Mutex<String>,
    wait: Duration,
}

/// How long a read waits for output before looking at whether the recording was abandoned, in milliseconds.
const POLL_MS: u64 = 100;

impl Running {
    fn start(
        settings: &CommandSettings,
        request: &OpenRequest,
        spawner: &Object<dyn ProcessSpawner>,
    ) -> Result<Self, ProviderError> {
        let named = named_hints(&request.hints);
        let said = |key: &str| {
            named
                .iter()
                .find(|(name, _)| *name == key)
                .map_or("", |(_, text)| text.as_str())
        };
        let rate = request.format.rate.to_string();
        let channels = request.format.channels.to_string();
        let mut spec = SpawnSpec::new(&settings.program);
        for arg in &settings.args {
            spec = spec.arg(
                arg.replace("{language}", said("language"))
                    .replace("{vocabulary}", said("vocabulary"))
                    .replace("{accent}", said("accent"))
                    .replace("{voice}", said("voice"))
                    .replace("{rate}", &rate)
                    .replace("{channels}", &channels),
            );
        }
        let mut environment = Environment::new();
        for (name, text) in &named {
            environment.insert(alloc::format!("CANOA_SPEECH_{}", name.to_uppercase()), text);
        }
        spec = spec.environment(environment);
        let process: Process = Arc::new(spawner.spawn(spec).map_err(|error| {
            internal(alloc::format!(
                "could not start {:?}: {}",
                settings.program,
                error.message
            ))
        })?);

        let out = Arc::new(Mutex::new(Capture::default()));
        let err: Arc<Mutex<Vec<u8>>> = Arc::default();
        let stopped = Arc::new(AtomicBool::new(false));
        let (done, readers) = channel();
        {
            let (process, out, stopped, done) = (
                Arc::clone(&process),
                Arc::clone(&out),
                Arc::clone(&stopped),
                done.clone(),
            );
            thread::spawn(move || {
                drain(
                    &stopped,
                    |buffer, wait| process.read(buffer, wait),
                    |chunk| {
                        let mut out = out.lock().unwrap_or_else(PoisonError::into_inner);
                        let room = MAX_OUTPUT.saturating_sub(out.bytes.len());
                        out.bytes.extend_from_slice(&chunk[..chunk.len().min(room)]);
                        out.dropped |= chunk.len() > room;
                    },
                );
                done.send(()).ok();
            });
        }
        {
            let (process, err, stopped) =
                (Arc::clone(&process), Arc::clone(&err), Arc::clone(&stopped));
            thread::spawn(move || {
                drain(
                    &stopped,
                    |buffer, wait| process.read_error(buffer, wait),
                    |chunk| {
                        let mut err = err.lock().unwrap_or_else(PoisonError::into_inner);
                        err.extend_from_slice(chunk);
                        let excess = err.len().saturating_sub(MAX_ERROR);
                        err.drain(..excess);
                    },
                );
                done.send(()).ok();
            });
        }
        let running = Self {
            process,
            input: Mutex::new(true),
            out,
            err,
            readers: Mutex::new(Some(readers)),
            stopped,
            partials: settings.partials,
            shown: Mutex::new(String::new()),
            wait: Duration::from_secs(u64::from(settings.timeout_seconds)),
        };
        if settings.input == CommandInput::Wav {
            running.write(&wav_header(&request.format, None))?;
        }
        Ok(running)
    }

    /// Writes to the program's input unless it stopped taking it, which is not an error here: how it ended is said at `finish`.
    fn write(&self, audio: &[u8]) -> Result<(), ProviderError> {
        if !*self.input.lock().unwrap_or_else(PoisonError::into_inner) {
            return Ok(());
        }
        // Written without a lock held, so a cancel can end a program that stopped reading while this waits for it.
        match self.process.write(audio) {
            Ok(()) => Ok(()),
            Err(error) if error.status == Status::GUATIAO_ERR_GONE => {
                *self.input.lock().unwrap_or_else(PoisonError::into_inner) = false;
                Ok(())
            }
            Err(error) => Err(internal(alloc::format!(
                "the command's input failed: {}",
                error.message
            ))),
        }
    }

    fn text(&self) -> String {
        let out = self.out.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8_lossy(&out.bytes).trim().to_string()
    }

    fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
        self.process.kill().ok();
    }
}

/// Reads until the program has no more to say or the recording is abandoned, a chunk at a time, into `keep`.
fn drain(
    stopped: &AtomicBool,
    mut read: impl FnMut(&mut [u8], u64) -> Result<i64, ProviderError>,
    mut keep: impl FnMut(&[u8]),
) {
    let mut chunk = [0_u8; 8192];
    while !stopped.load(Ordering::Relaxed) {
        match read(&mut chunk, POLL_MS) {
            Ok(0) | Err(_) => break,
            Ok(count) => {
                if let Ok(count) = usize::try_from(count) {
                    keep(&chunk[..count.min(chunk.len())]);
                }
                // A timeout or a link that is down is a negative answer: look again.
            }
        }
    }
}

impl SpeechSession for Running {
    fn push(&self, audio: &[u8]) -> Result<Heard, ProviderError> {
        if !audio.is_empty() {
            self.write(audio)?;
        }
        if !self.partials {
            return Ok(Heard::nothing());
        }
        let text = self.text();
        let mut shown = self.shown.lock().unwrap_or_else(PoisonError::into_inner);
        if text.is_empty() || *shown == text {
            return Ok(Heard::nothing());
        }
        shown.clone_from(&text);
        Ok(Heard::partial(text))
    }

    fn finish(&self) -> Result<Transcript, ProviderError> {
        if self.stopped.load(Ordering::Relaxed) {
            return Err(internal("the recording has ended"));
        }
        self.process.close_input().ok();
        *self.input.lock().unwrap_or_else(PoisonError::into_inner) = false;
        let started = Instant::now();
        let exit = loop {
            let exit = self.process.wait(10).map_err(|error| {
                internal(alloc::format!(
                    "could not wait for the command: {}",
                    error.message
                ))
            })?;
            if !exit.running {
                break exit;
            }
            if started.elapsed() >= self.wait {
                self.stop();
                return Err(internal(alloc::format!(
                    "the command did not finish in {} seconds",
                    self.wait.as_secs()
                )));
            }
        };
        if let Some(readers) = self
            .readers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            for _ in 0..2 {
                if readers.recv_timeout(OUTPUT_GRACE).is_err() {
                    break;
                }
            }
        }
        self.stopped.store(true, Ordering::Relaxed);
        if exit.code != Some(0) {
            let err = self.err.lock().unwrap_or_else(PoisonError::into_inner);
            let how = exit.code.map_or_else(
                || "was ended by a signal".to_string(),
                |code| alloc::format!("exited with {code}"),
            );
            return Err(internal(alloc::format!(
                "the command {how}: {}",
                String::from_utf8_lossy(&err).trim()
            )));
        }
        let dropped = self
            .out
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .dropped;
        let transcript = Transcript::new(self.text());
        Ok(if dropped {
            transcript.with("output_truncated", true)
        } else {
            transcript
        })
    }

    fn cancel(&self) -> Result<(), ProviderError> {
        self.stop();
        Ok(())
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.stop();
    }
}

impl core::fmt::Debug for CommandSpeech {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CommandSpeech")
            .field("program", &self.settings.program)
            .finish_non_exhaustive()
    }
}
