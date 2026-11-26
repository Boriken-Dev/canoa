//! The Candle provider: speech to text inside the host's own process, with the model files the Store installed.
//!
//! A recording is held as it arrives and turned into text when the person stops: 16 kHz mono samples go through Whisper written in
//! Rust ([`crate::Engine`]) with the language the person said or its own detection, their words as the initial prompt, and no
//! timestamps. The loaded model is kept between recordings and let go when the person changes models or after [`IDLE`] without one,
//! so the first recording after a pause pays the load and the ones after it do not.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a provider that holds a model, a thread and files names what it uses from std"
)]

use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::Instant;

use canoa::speech::{
    AudioFormat, Heard, OpenRequest, PCM_S16LE, SpeechInfo, SpeechProvider, SpeechSession,
    Transcript, unsupported_format,
};
use canoa::status::{bad_value, internal};
use coqui_acp_api::ProcessSpawner;
use guatiao::library::{Object, ProviderError};
use guatiao::{FromValue, Schema, ToValue};
use guatiao_intake::Form;

use crate::engine::Engine;
use crate::{CONFIG, FILE_NAMES, TOKENIZER, WEIGHTS};

/// The id of the provider, which a table names and the Store type's speech offer builds.
pub const PROVIDER: &str = "speech_whisper_candle";

/// How long a loaded model is kept without a recording: after this it is freed, and the next recording loads it again.
pub const IDLE: Duration = Duration::from_secs(300);

/// The sample rate Whisper reads.
const RATE: u32 = 16_000;

/// The fewest samples Whisper transcribes: a recording shorter than this (100 ms) has no words.
const SHORTEST: usize = 1_600;

/// The most threads a recording uses unless the settings say another number.
const THREADS: usize = 8;

/// What the Candle provider is configured with.
#[derive(ToValue, FromValue, Schema, Form, Debug, Clone, PartialEq, Eq)]
#[form(section(id = "whispercandle", label = "Whisper (Candle)"))]
#[non_exhaustive]
pub struct WhisperCandleSettings {
    /// The model: the id of an installed Whisper model, `base` by default for the speech offer. Its files are in the folder `<model>/`.
    #[schema(section = "whispercandle", title = "Model")]
    #[form(placeholder = "base")]
    pub model: String,
    /// The folder the model folders are in: the one the host keeps the Whisper models in.
    #[schema(section = "whispercandle", title = "Models folder")]
    pub models_dir: String,
    /// The language to read the recording in when the person has not said one: a code such as `es`, or none to let the model tell.
    #[schema(section = "whispercandle", title = "Language")]
    #[form(placeholder = "auto")]
    pub language: Option<String>,
    /// How many threads a recording uses; none takes the number of processors, up to eight.
    #[schema(advanced, section = "whispercandle", title = "Threads")]
    pub threads: Option<u32>,
}

impl WhisperCandleSettings {
    /// Settings that run `model` from `models_dir`, with the language detected and the default threads.
    #[must_use]
    pub fn new(model: impl Into<String>, models_dir: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            models_dir: models_dir.into(),
            language: None,
            threads: None,
        }
    }

    /// The folder of the model these settings name.
    fn folder(&self) -> PathBuf {
        Path::new(&self.models_dir).join(&self.model)
    }

    fn threads(&self) -> usize {
        self.threads
            .map_or_else(
                || {
                    std::thread::available_parallelism()
                        .map_or(THREADS, usize::from)
                        .min(THREADS)
                },
                |threads| threads as usize,
            )
            .max(1)
    }
}

/// The Candle provider.
#[derive(guatiao::Provider)]
#[provider(SpeechProvider, id = "speech_whisper_candle", name = "Whisper (Candle)", config = WhisperCandleSettings, form = WhisperCandleSettings)]
pub struct WhisperCandleSpeech {
    settings: WhisperCandleSettings,
}

impl From<WhisperCandleSettings> for WhisperCandleSpeech {
    fn from(settings: WhisperCandleSettings) -> Self {
        Self { settings }
    }
}

impl core::fmt::Debug for WhisperCandleSpeech {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WhisperCandleSpeech")
            .field("model", &self.settings.model)
            .finish_non_exhaustive()
    }
}

impl SpeechProvider for WhisperCandleSpeech {
    fn describe(&self) -> Result<SpeechInfo, ProviderError> {
        let mut info = SpeechInfo::new("Whisper (Candle)", vec![PCM_S16LE.to_string()]);
        info.description = Some(
            "Turns the recording into text on this computer, with a Whisper model the Store installed, in Rust".to_string(),
        );
        info.hints = ["language", "vocabulary"].map(ToString::to_string).to_vec();
        Ok(info)
    }

    fn open(
        &self,
        request: OpenRequest,
        _spawner: Object<dyn ProcessSpawner>,
    ) -> Result<Object<dyn SpeechSession>, ProviderError> {
        if request.format.encoding != PCM_S16LE {
            return Err(unsupported_format(&request.format, &[PCM_S16LE]));
        }
        if request.format.rate == 0 {
            return Err(bad_value("the recording has no sample rate"));
        }
        let folder = self.settings.folder();
        if let Some(missing) = FILE_NAMES
            .iter()
            .map(|name| folder.join(name))
            .find(|file| !file.is_file())
        {
            return Err(bad_value(alloc::format!(
                "the model {} is not installed: install it from the Whisper models in the Store ({} is not there)",
                self.settings.model,
                missing.display()
            )));
        }
        // The model loads while the person speaks, so the text is there soon after they stop.
        let warm = folder.clone();
        let threads = self.settings.threads();
        std::thread::spawn(move || drop(loaded(&warm, threads)));
        Ok(Recording {
            settings: self.settings.clone(),
            folder,
            request,
            audio: Mutex::new(Some(Vec::new())),
        }
        .into_object())
    }
}

/// One recording, held until it is transcribed.
struct Recording {
    settings: WhisperCandleSettings,
    folder: PathBuf,
    request: OpenRequest,
    audio: Mutex<Option<Vec<u8>>>,
}

impl SpeechSession for Recording {
    fn push(&self, audio: &[u8]) -> Result<Heard, ProviderError> {
        let mut held = self.audio.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(held) = held.as_mut() else {
            return Err(internal("the recording has ended"));
        };
        held.extend_from_slice(audio);
        Ok(Heard::nothing())
    }

    fn finish(&self) -> Result<Transcript, ProviderError> {
        let audio = self
            .audio
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .ok_or_else(|| internal("the recording has ended"))?;
        let samples = samples_of(&audio, &self.request.format);
        if samples.len() < SHORTEST {
            return Ok(Transcript::new(""));
        }
        let hints = &self.request.hints;
        let language = hints
            .language
            .as_deref()
            .and_then(primary_language)
            .map(str::to_owned);
        let prompt = hints.vocabulary.join(", ");
        transcribe(
            &self.folder,
            &self.settings,
            &samples,
            language.as_deref(),
            &prompt,
        )
    }

    fn cancel(&self) -> Result<(), ProviderError> {
        drop(
            self.audio
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take(),
        );
        Ok(())
    }
}

/// The primary subtag of a BCP 47 tag (`es` for `es-PR`), when there is one.
fn primary_language(tag: &str) -> Option<&str> {
    let primary = tag.split(['-', '_']).next()?.trim();
    (!primary.is_empty()).then_some(primary)
}

/// The recording as 16 kHz mono samples between -1 and 1: channels averaged, and a rate that is not 16 kHz resampled by linear
/// interpolation.
fn samples_of(audio: &[u8], format: &AudioFormat) -> Vec<f32> {
    let channels = usize::from(format.channels.max(1));
    let frame = 2 * channels;
    let mono: Vec<f32> = audio
        .chunks_exact(frame)
        .map(|frame| {
            let sum: f32 = frame
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| f32::from(i16::from_le_bytes(*pair)) / 32768.0)
                .sum();
            sum / channels as f32
        })
        .collect();
    if format.rate == RATE || mono.is_empty() {
        return mono;
    }
    let step = f64::from(format.rate) / f64::from(RATE);
    let count = (mono.len() as f64 / step).floor() as usize;
    (0..count)
        .map(|index| {
            let at = index as f64 * step;
            let low = at.floor() as usize;
            let high = (low + 1).min(mono.len() - 1);
            let fraction = (at - low as f64) as f32;
            mono[low] * (1.0 - fraction) + mono[high] * fraction
        })
        .collect()
}

/// A model held in memory, with the threads it runs on; one recording at a time uses it.
struct Loaded {
    folder: PathBuf,
    threads: usize,
    pool: rayon::ThreadPool,
    engine: Mutex<Engine>,
}

/// The model kept between recordings, and whether a thread is waiting to free it.
struct Held {
    loaded: Option<(Arc<Loaded>, Instant)>,
    watching: bool,
}

static HELD: Mutex<Held> = Mutex::new(Held {
    loaded: None,
    watching: false,
});

/// Held while a model loads, so a recording that asks for the model its own warm-up thread is loading waits for it: a large model
/// read twice at once takes twice the memory.
static LOADING: Mutex<()> = Mutex::new(());

/// The model held when it is `folder` run on `threads`, marked used now.
fn kept(folder: &Path, threads: usize) -> Option<Arc<Loaded>> {
    let held = HELD.lock().unwrap_or_else(PoisonError::into_inner);
    let loaded = held
        .loaded
        .as_ref()
        .filter(|(each, _)| each.folder == folder && each.threads == threads)
        .map(|(each, _)| Arc::clone(each))?;
    drop(held);
    touch(&loaded);
    Some(loaded)
}

/// The model in `folder`, loaded now or kept from the last recording. A different model frees the one held.
fn loaded(folder: &Path, threads: usize) -> Result<Arc<Loaded>, ProviderError> {
    if let Some(loaded) = kept(folder, threads) {
        return Ok(loaded);
    }
    let _loading = LOADING.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(loaded) = kept(folder, threads) {
        return Ok(loaded);
    }
    let read = |name: &str| {
        std::fs::read(folder.join(name)).map_err(|error| {
            internal(alloc::format!(
                "the model file {} could not be read: {error}",
                folder.join(name).display()
            ))
        })
    };
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .map_err(|error| internal(alloc::format!("the threads could not start: {error}")))?;
    let (config, tokenizer, weights) = (read(CONFIG)?, read(TOKENIZER)?, read(WEIGHTS)?);
    let engine = pool
        .install(|| Engine::load(&config, &tokenizer, weights))
        .map_err(|error| {
            internal(alloc::format!(
                "the model {} could not be loaded: {error}",
                folder.display()
            ))
        })?;
    let made = Arc::new(Loaded {
        folder: folder.to_path_buf(),
        threads,
        pool,
        engine: Mutex::new(engine),
    });
    let mut held = HELD.lock().unwrap_or_else(PoisonError::into_inner);
    held.loaded = Some((Arc::clone(&made), Instant::now()));
    if !held.watching {
        held.watching = true;
        std::thread::spawn(watch);
    }
    Ok(made)
}

/// Marks the model used now.
fn touch(loaded: &Arc<Loaded>) {
    let mut held = HELD.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((each, used)) = held.loaded.as_mut()
        && Arc::ptr_eq(each, loaded)
    {
        *used = Instant::now();
    }
}

/// Waits until the model has been idle for [`IDLE`] and frees it, then ends.
fn watch() {
    loop {
        let wait = {
            let mut held = HELD.lock().unwrap_or_else(PoisonError::into_inner);
            let Some((_, used)) = held.loaded.as_ref() else {
                held.watching = false;
                return;
            };
            match IDLE
                .checked_sub(used.elapsed())
                .filter(|left| !left.is_zero())
            {
                Some(left) => left,
                None => {
                    held.loaded = None;
                    held.watching = false;
                    return;
                }
            }
        };
        std::thread::sleep(wait);
    }
}

/// Frees the model held, if any. Recordings running keep what they use until they end.
pub fn unload() {
    HELD.lock().unwrap_or_else(PoisonError::into_inner).loaded = None;
}

/// Whether a model is held in memory now.
#[must_use]
pub fn is_loaded() -> bool {
    HELD.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .loaded
        .is_some()
}

fn transcribe(
    folder: &Path,
    settings: &WhisperCandleSettings,
    samples: &[f32],
    language: Option<&str>,
    prompt: &str,
) -> Result<Transcript, ProviderError> {
    let model = loaded(folder, settings.threads())?;
    let started = Instant::now();
    let heard = {
        let mut guard = model.engine.lock().unwrap_or_else(PoisonError::into_inner);
        let engine: &mut Engine = &mut guard;
        // A language the recording names that the model does not know is detected instead of refused.
        let language = language
            .or(settings.language.as_deref())
            .filter(|code| !code.eq_ignore_ascii_case("auto") && engine.knows(code));
        model
            .pool
            .install(|| engine.transcribe(samples, language, prompt))
            .map_err(|error| internal(alloc::format!("Whisper could not transcribe: {error}")))?
    };
    touch(&model);
    let mut transcript = Transcript::new(heard.text);
    let mut front = guatiao::Map::new();
    if let Some(detected) = heard.language {
        front.set("language", guatiao::Value::from(detected));
    }
    let milliseconds = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
    front.set("duration_ms", guatiao::Value::from(milliseconds));
    transcript.front_matter = guatiao::Value::from(front);
    Ok(transcript)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn format(rate: u32, channels: u8) -> AudioFormat {
        let mut format = AudioFormat::new(PCM_S16LE, rate);
        format.channels = channels;
        format
    }

    fn pcm(samples: &[i16]) -> Vec<u8> {
        samples
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect()
    }

    #[test]
    fn the_primary_subtag_of_a_language_is_the_code_whisper_takes() {
        assert_eq!(primary_language("es-PR"), Some("es"));
        assert_eq!(primary_language("en"), Some("en"));
        assert_eq!(primary_language("pt_BR"), Some("pt"));
        assert_eq!(primary_language(" "), None);
    }

    #[test]
    fn samples_at_16_khz_mono_are_scaled_and_kept() {
        let samples = samples_of(&pcm(&[0, 16384, -32768]), &format(16_000, 1));
        assert_eq!(samples, [0.0, 0.5, -1.0]);
    }

    #[test]
    fn channels_are_averaged_and_a_slower_or_faster_rate_is_resampled_to_16_khz() {
        let stereo = samples_of(&pcm(&[16384, -16384, 8192, 8192]), &format(16_000, 2));
        assert_eq!(stereo, [0.0, 0.25]);
        let double = vec![1000_i16; 3200];
        let down = samples_of(&pcm(&double), &format(32_000, 1));
        assert_eq!(down.len(), 1600);
        assert!(
            down.iter()
                .all(|sample| (sample - 1000.0 / 32768.0).abs() < 1e-6)
        );
        let up = samples_of(&pcm(&vec![1000_i16; 800]), &format(8_000, 1));
        assert_eq!(up.len(), 1600);
    }

    #[test]
    fn the_model_folder_is_the_models_folder_and_the_id() {
        let own = WhisperCandleSettings::new("tiny", "/models");
        assert_eq!(own.folder(), Path::new("/models/tiny"));
    }

    #[test]
    fn threads_are_the_settings_or_the_processors_up_to_eight() {
        let mut settings = WhisperCandleSettings::new("base", "/models");
        assert!((1..=8).contains(&settings.threads()));
        settings.threads = Some(3);
        assert_eq!(settings.threads(), 3);
        settings.threads = Some(0);
        assert_eq!(settings.threads(), 1);
    }
}
