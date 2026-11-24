//! What the providers that run a program share: one program, started through the host's spawner, that is handed text and
//! writes audio, and the utterance that reads what it writes.
//!
//! Text goes to its standard input a line at a time from a thread of its own, so a program that does not read slows nothing
//! down. Its standard output is read as it is written into a buffer of [`BUFFER`] bytes, no more: a client that plays slowly
//! stops the program from running far ahead of it, and nothing grows. Its standard error is kept, the last of it, for the
//! message of a failure.

use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use coqui_acp_api::{ProcessSpawner, READ_TIMED_OUT, SpawnSpec, SpawnedProcess};
use guatiao::Status;
use guatiao::library::{Object, ProviderError};

use crate::speech::AudioFormat;
use crate::speech::output::SpeechUtterance;
use crate::status::internal;

/// The most audio held between the program and the client, in bytes: about two seconds of 24 kHz mono.
pub const BUFFER: usize = 96 * 1024;

/// The most standard error kept, in bytes, for an error message: the last of it.
const MAX_ERROR: usize = 2048;

/// How long a read of the program's output waits before looking at whether the utterance was stopped, in milliseconds.
const POLL_MS: u64 = 100;

/// How long, after the program closed its output, its exit is waited for, in milliseconds.
const EXIT_MS: u64 = 3000;

/// How the program's output begins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Head {
    /// Samples from the first byte.
    Raw,
    /// A WAV stream: everything before the `data` chunk is dropped.
    Wav,
}

#[derive(Default)]
struct State {
    audio: Vec<u8>,
    /// Where in `audio` the next read starts.
    at: usize,
    ended: bool,
    error: Option<String>,
    stopped: bool,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// What a program wrote to standard error: the last of it, and whether its end was reached.
#[derive(Default)]
struct Errors {
    tail: Mutex<Vec<u8>>,
    ended: AtomicBool,
}

/// One program speaking one text.
pub struct Spoken {
    /// Ends whatever feeds the buffer.
    stopper: Box<dyn Fn() + Send + Sync>,
    shared: Arc<Shared>,
    format: AudioFormat,
    gone: Arc<AtomicBool>,
}

impl core::fmt::Debug for Spoken {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Spoken").finish_non_exhaustive()
    }
}

impl Spoken {
    /// Starts `spec` through `spawner` and hands it `lines` on its standard input, each ended by a newline, unless `lines` is
    /// `None` (the text is in its arguments), and reads its output as `format`. A program that makes no sound for `idle` is
    /// ended with the reason.
    ///
    /// # Errors
    ///
    /// `INTERNAL` with the reason when the program cannot start.
    pub fn start(
        spawner: &Object<dyn ProcessSpawner>,
        spec: SpawnSpec,
        lines: Option<Vec<String>>,
        format: AudioFormat,
        head: Head,
        idle: Duration,
    ) -> Result<Self, ProviderError> {
        let program = spec.program.clone();
        let process: Arc<Object<dyn SpawnedProcess>> =
            Arc::new(spawner.spawn(spec).map_err(|error| {
                internal(alloc::format!(
                    "could not start {program:?}: {}",
                    error.message
                ))
            })?);
        let shared = Arc::new(Shared {
            state: Mutex::default(),
            changed: Condvar::new(),
        });
        let gone = Arc::new(AtomicBool::new(false));
        let errors: Arc<Errors> = Arc::default();

        {
            let (process, gone, errors) =
                (Arc::clone(&process), Arc::clone(&gone), Arc::clone(&errors));
            thread::spawn(move || {
                let mut chunk = [0_u8; 2048];
                while !gone.load(Ordering::Relaxed) {
                    match process.read_error(&mut chunk, POLL_MS) {
                        Ok(0) | Err(_) => break,
                        Ok(count) => {
                            if let Ok(count) = usize::try_from(count) {
                                let mut kept =
                                    errors.tail.lock().unwrap_or_else(PoisonError::into_inner);
                                kept.extend_from_slice(&chunk[..count.min(chunk.len())]);
                                let excess = kept.len().saturating_sub(MAX_ERROR);
                                kept.drain(..excess);
                            }
                        }
                    }
                }
                errors.ended.store(true, Ordering::Relaxed);
            });
        }
        if let Some(lines) = lines {
            let (process, gone) = (Arc::clone(&process), Arc::clone(&gone));
            thread::spawn(move || {
                for line in lines {
                    if gone.load(Ordering::Relaxed) {
                        break;
                    }
                    let mut bytes = line.into_bytes();
                    bytes.push(b'\n');
                    if process.write(&bytes).is_err() {
                        break;
                    }
                }
                process.close_input().ok();
            });
        } else {
            process.close_input().ok();
        }
        {
            let (process, shared, gone, errors) = (
                Arc::clone(&process),
                Arc::clone(&shared),
                Arc::clone(&gone),
                Arc::clone(&errors),
            );
            thread::spawn(move || pump(&process, &shared, &gone, &errors, head, idle));
        }
        let killed = Arc::clone(&process);
        Ok(Self {
            stopper: Box::new(move || {
                killed.kill().ok();
            }),
            shared,
            format,
            gone,
        })
    }

    /// Audio that is read from `reader`, a body a server is still sending, rather than from a program. A reader that gives
    /// nothing for `idle` fails the utterance; the first `head` bytes are dropped as `head` says.
    pub fn from_reader(
        mut reader: impl std::io::Read + Send + 'static,
        format: AudioFormat,
        head: Head,
        idle: Duration,
    ) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::default(),
            changed: Condvar::new(),
        });
        let gone = Arc::new(AtomicBool::new(false));
        {
            let (shared, gone) = (Arc::clone(&shared), Arc::clone(&gone));
            thread::spawn(move || {
                let mut chunk = [0_u8; 4096];
                let mut wav = (head == Head::Wav).then(Vec::<u8>::new);
                let mut last = Instant::now();
                let failure = loop {
                    if gone.load(Ordering::Relaxed) || !wait_room(&shared, &mut last) {
                        return;
                    }
                    match reader.read(&mut chunk) {
                        Ok(0) => break None,
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                        Err(error) => {
                            break Some(alloc::format!("the server's answer stopped: {error}"));
                        }
                        Ok(count) => {
                            last = Instant::now();
                            let samples: Vec<u8>;
                            let bytes: &[u8] = if let Some(held) = wav.as_mut() {
                                held.extend_from_slice(&chunk[..count]);
                                match data_start(held) {
                                    Some(start) => {
                                        samples = held.split_off(start);
                                        wav = None;
                                        &samples
                                    }
                                    None => continue,
                                }
                            } else {
                                &chunk[..count]
                            };
                            if !append(&shared, bytes) {
                                return;
                            }
                        }
                    }
                    if last.elapsed() >= idle {
                        break Some(alloc::format!(
                            "the server sent nothing for {} seconds",
                            idle.as_secs()
                        ));
                    }
                };
                conclude(&shared, failure);
            });
        }
        Self {
            stopper: Box::new(|| {}),
            shared,
            format,
            gone,
        }
    }

    /// Ends the program and everything it started, and whatever the client had not read.
    fn end(&self) {
        self.gone.store(true, Ordering::Relaxed);
        {
            let mut state = self.shared.lock();
            state.stopped = true;
            state.audio.clear();
            state.at = 0;
        }
        self.shared.changed.notify_all();
        (self.stopper)();
    }
}

/// Waits while the client has not read what is buffered: false once the utterance was stopped.
fn wait_room(shared: &Shared, last: &mut Instant) -> bool {
    let mut state = shared.lock();
    while state.audio.len() - state.at >= BUFFER && !state.stopped {
        state = shared
            .changed
            .wait_timeout(state, Duration::from_millis(POLL_MS))
            .unwrap_or_else(PoisonError::into_inner)
            .0;
        // The clock that fails a silent program is not running while the client is the one holding it up.
        *last = Instant::now();
    }
    !state.stopped
}

/// Adds `bytes` to the buffer: false once the utterance was stopped.
fn append(shared: &Shared, bytes: &[u8]) -> bool {
    let mut state = shared.lock();
    if state.stopped {
        return false;
    }
    state.audio.extend_from_slice(bytes);
    drop(state);
    shared.changed.notify_all();
    true
}

/// Marks the utterance over, with `failure` as why when it failed.
fn conclude(shared: &Shared, failure: Option<String>) {
    let mut state = shared.lock();
    if !state.stopped {
        state.error = failure;
    }
    state.ended = true;
    drop(state);
    shared.changed.notify_all();
}

/// Reads the program's output into the buffer until it ends, then says how it ended.
fn pump(
    process: &Object<dyn SpawnedProcess>,
    shared: &Shared,
    gone: &AtomicBool,
    errors: &Errors,
    head: Head,
    idle: Duration,
) {
    let mut chunk = [0_u8; 4096];
    let mut wav = (head == Head::Wav).then(Vec::<u8>::new);
    let mut last = Instant::now();
    let failure = loop {
        if gone.load(Ordering::Relaxed) {
            return;
        }
        if !wait_room(shared, &mut last) {
            return;
        }
        match process.read(&mut chunk, POLL_MS) {
            Ok(0) => break None,
            Err(error) => break Some(error.message.to_string()),
            Ok(count) if count > 0 => {
                last = Instant::now();
                let Ok(count) = usize::try_from(count) else {
                    continue;
                };
                let count = count.min(chunk.len());
                let samples: Vec<u8>;
                let bytes: &[u8] = if let Some(held) = wav.as_mut() {
                    held.extend_from_slice(&chunk[..count]);
                    match data_start(held) {
                        Some(start) => {
                            samples = held.split_off(start);
                            wav = None;
                            &samples
                        }
                        None => continue,
                    }
                } else {
                    &chunk[..count]
                };
                if !append(shared, bytes) {
                    return;
                }
            }
            Ok(_) => {
                if last.elapsed() >= idle {
                    process.kill().ok();
                    break Some(alloc::format!(
                        "the program made no sound for {} seconds",
                        idle.as_secs()
                    ));
                }
            }
        }
    };
    let exit = process.wait(EXIT_MS).ok();
    let mut state = shared.lock();
    if !state.stopped {
        state.error = failure.or_else(|| match exit {
            Some(exit) if exit.running => {
                process.kill().ok();
                Some("the program did not end after its last sound".to_owned())
            }
            Some(exit) if exit.code != Some(0) => {
                // What it said on standard error is read by a thread of its own, which may not have reached the end yet.
                for _ in 0..100 {
                    if errors.ended.load(Ordering::Relaxed) {
                        break;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                let tail = errors.tail.lock().unwrap_or_else(PoisonError::into_inner);
                let how = exit.code.map_or_else(
                    || "was ended by a signal".to_owned(),
                    |code| alloc::format!("exited with {code}"),
                );
                Some(alloc::format!(
                    "the program {how}: {}",
                    String::from_utf8_lossy(&tail).trim()
                ))
            }
            _ => None,
        });
    }
    state.ended = true;
    drop(state);
    shared.changed.notify_all();
}

/// Where the samples of a WAV stream start, once the stream has shown its `data` chunk header.
fn data_start(head: &[u8]) -> Option<usize> {
    if head.len() < 12 {
        return None;
    }
    if &head[..4] != b"RIFF" || &head[8..12] != b"WAVE" {
        // Not a WAV stream: what it is, is samples.
        return Some(0);
    }
    let mut at = 12;
    while at + 8 <= head.len() {
        let size = usize::try_from(u32::from_le_bytes([
            head[at + 4],
            head[at + 5],
            head[at + 6],
            head[at + 7],
        ]))
        .unwrap_or(usize::MAX);
        if &head[at..at + 4] == b"data" {
            return Some(at + 8);
        }
        at = at
            .saturating_add(8)
            .saturating_add(size)
            .saturating_add(size & 1);
    }
    None
}

impl SpeechUtterance for Spoken {
    fn format(&self) -> Result<AudioFormat, ProviderError> {
        Ok(self.format.clone())
    }

    fn read(&self, dst: &mut [u8], timeout_ms: u64) -> Result<i64, ProviderError> {
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        let mut state = self.shared.lock();
        loop {
            if state.at < state.audio.len() {
                let count = dst.len().min(state.audio.len() - state.at);
                dst[..count].copy_from_slice(&state.audio[state.at..state.at + count]);
                state.at += count;
                if state.at == state.audio.len() {
                    state.audio.clear();
                    state.at = 0;
                }
                drop(state);
                self.shared.changed.notify_all();
                return i64::try_from(count).map_err(|_| internal("a chunk too long to count"));
            }
            if state.stopped {
                return Ok(0);
            }
            if state.ended {
                return match state.error.take() {
                    Some(why) => Err(internal(why)),
                    None => Ok(0),
                };
            }
            let now = Instant::now();
            if now >= deadline {
                return Ok(READ_TIMED_OUT);
            }
            state = self
                .shared
                .changed
                .wait_timeout(state, deadline - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    fn stop(&self) -> Result<(), ProviderError> {
        self.end();
        Ok(())
    }
}

impl Drop for Spoken {
    fn drop(&mut self) {
        self.end();
    }
}

/// `GONE` is how a spawner says a process has ended: not an error of an utterance.
#[must_use]
pub fn is_gone(error: &ProviderError) -> bool {
    error.status == Status::GUATIAO_ERR_GONE
}

/// Runs `spec` to its end, as a program that is asked a question and answers it on its standard output, and answers what it
/// wrote there as text, at most `limit` bytes of it.
///
/// # Errors
///
/// `INTERNAL` with the reason when it cannot start, does not end within `wait`, or ends with a failure.
pub fn collect(
    spawner: &Object<dyn ProcessSpawner>,
    spec: SpawnSpec,
    limit: usize,
    wait: Duration,
) -> Result<String, ProviderError> {
    let program = spec.program.clone();
    let process = spawner.spawn(spec).map_err(|error| {
        internal(alloc::format!(
            "could not start {program:?}: {}",
            error.message
        ))
    })?;
    process.close_input().ok();
    let started = Instant::now();
    let mut out = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        if started.elapsed() >= wait {
            process.kill().ok();
            return Err(internal(alloc::format!(
                "{program:?} did not answer in {} seconds",
                wait.as_secs()
            )));
        }
        match process.read(&mut chunk, POLL_MS) {
            Ok(0) | Err(_) => break,
            Ok(count) => {
                if let Ok(count) = usize::try_from(count) {
                    let room = limit.saturating_sub(out.len());
                    out.extend_from_slice(&chunk[..count.min(chunk.len()).min(room)]);
                } else if process.wait(0).is_ok_and(|exit| !exit.running) {
                    // It has exited but a service it started holds the pipe open: take what is buffered and stop.
                    while let Ok(count) = process.read(&mut chunk, 50) {
                        let Ok(count) = usize::try_from(count) else {
                            break;
                        };
                        if count == 0 {
                            break;
                        }
                        let room = limit.saturating_sub(out.len());
                        out.extend_from_slice(&chunk[..count.min(chunk.len()).min(room)]);
                    }
                    break;
                }
            }
        }
    }
    let exit = process.wait(EXIT_MS).map_err(|error| {
        internal(alloc::format!(
            "could not wait for {program:?}: {}",
            error.message
        ))
    })?;
    if exit.running || exit.code != Some(0) {
        process.kill().ok();
        let mut err = Vec::new();
        while let Ok(count) = process.read_error(&mut chunk, 50) {
            let Ok(count) = usize::try_from(count) else {
                break;
            };
            if count == 0 {
                break;
            }
            err.extend_from_slice(&chunk[..count.min(chunk.len())]);
            if err.len() > MAX_ERROR {
                break;
            }
        }
        return Err(internal(alloc::format!(
            "{program:?} failed: {}",
            String::from_utf8_lossy(&err).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}
