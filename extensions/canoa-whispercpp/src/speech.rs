//! The whisper.cpp provider: speech to text inside the host's own process, with a model file the Store installed.
//!
//! A recording is held as it arrives and turned into text when the person stops: 16 kHz mono samples go through whisper.cpp
//! (the `whisper-rs` binding) with the language the person said or its own detection, their words as the initial prompt, and no
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
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
};

/// The id of the provider, which a table names and the Store type's speech offer builds.
pub const PROVIDER: &str = "speech_whisper";

/// How long a loaded model is kept without a recording: after this it is freed, and the next recording loads it again.
pub const IDLE: Duration = Duration::from_secs(300);

/// The sample rate whisper.cpp reads.
const RATE: u32 = 16_000;

/// The fewest samples whisper.cpp transcribes: a recording shorter than this (100 ms) has no words.
const SHORTEST: usize = 1_600;

/// The most threads a recording uses unless the settings say another number.
const THREADS: usize = 8;

const fn default_beam() -> u32 {
    5
}

const fn default_flash() -> bool {
    true
}

/// What the whisper.cpp provider is configured with.
#[derive(ToValue, FromValue, Schema, Form, Debug, Clone, PartialEq, Eq)]
#[form(section(id = "whisper", label = "whisper.cpp"))]
#[non_exhaustive]
pub struct WhisperSettings {
    /// The model: one of the Models page, `base` by default for the speech offer. It is the file `ggml-<model>.bin` of the folder.
    #[schema(section = "whisper", title = "Model")]
    #[form(placeholder = "base")]
    pub model: String,
    /// The folder the model files are in: the one the host keeps the Models page's downloads in.
    #[schema(section = "whisper", title = "Models folder")]
    pub models_dir: String,
    /// How many threads a recording uses; none takes the number of processors, up to eight.
    #[schema(advanced, section = "whisper", title = "Threads")]
    pub threads: Option<u32>,
    /// How many candidates the search keeps for each word; 1 is the fastest and 5 is what whisper.cpp's own program does.
    #[map(default = default_beam())]
    #[schema(advanced, section = "whisper", title = "Search width")]
    pub beam_size: u32,
    /// Whether attention is computed in one fused step, as whisper.cpp's own program does by default: on the processor it is a third
    /// faster for the same text.
    #[map(default = default_flash())]
    #[schema(advanced, section = "whisper", title = "Flash attention")]
    pub flash_attention: bool,
}

impl WhisperSettings {
    /// Settings that run `model` from `models_dir`, with the default threads and search.
    #[must_use]
    pub fn new(model: impl Into<String>, models_dir: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            models_dir: models_dir.into(),
            threads: None,
            beam_size: default_beam(),
            flash_attention: default_flash(),
        }
    }

    /// The model file these settings name: the one written into the model's own folder when it was installed, else the
    /// `ggml-<model>.bin` of the folder.
    fn file(&self) -> PathBuf {
        let folder = Path::new(&self.models_dir);
        let written = folder.join(&self.model).join(crate::GGML_FILE);
        if written.is_file() {
            written
        } else {
            folder.join(alloc::format!("ggml-{}.bin", self.model))
        }
    }

    fn threads(&self) -> i32 {
        let wanted = self.threads.map_or_else(
            || {
                std::thread::available_parallelism()
                    .map_or(THREADS, usize::from)
                    .min(THREADS)
            },
            |threads| threads as usize,
        );
        i32::try_from(wanted.max(1)).unwrap_or(1)
    }
}

/// The whisper.cpp provider.
#[derive(guatiao::Provider)]
#[provider(SpeechProvider, id = "speech_whisper", name = "whisper.cpp", config = WhisperSettings, form = WhisperSettings)]
pub struct WhisperSpeech {
    settings: WhisperSettings,
}

impl From<WhisperSettings> for WhisperSpeech {
    fn from(settings: WhisperSettings) -> Self {
        Self { settings }
    }
}

impl core::fmt::Debug for WhisperSpeech {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WhisperSpeech")
            .field("model", &self.settings.model)
            .finish_non_exhaustive()
    }
}

impl SpeechProvider for WhisperSpeech {
    fn describe(&self) -> Result<SpeechInfo, ProviderError> {
        let mut info = SpeechInfo::new("whisper.cpp", vec![PCM_S16LE.to_string()]);
        info.description = Some(
            "Turns the recording into text on this computer, with a whisper.cpp model the Store installed".to_string(),
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
        let file = self.settings.file();
        if !file.is_file() {
            return Err(bad_value(alloc::format!(
                "the model {} is not installed: install it from the Models page ({} is not there)",
                self.settings.model,
                file.display()
            )));
        }
        // The model loads while the person speaks, so the text is there soon after they stop.
        let warm = file.clone();
        let flash = self.settings.flash_attention;
        std::thread::spawn(move || drop(loaded(&warm, flash)));
        Ok(Recording {
            settings: self.settings.clone(),
            file,
            request,
            audio: Mutex::new(Some(Vec::new())),
        }
        .into_object())
    }
}

/// One recording, held until it is transcribed.
struct Recording {
    settings: WhisperSettings,
    file: PathBuf,
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
            &self.file,
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

/// A model held in memory, as the state its transcriptions run in (the state keeps the model loaded); one recording at a time uses it.
struct Loaded {
    file: PathBuf,
    flash: bool,
    state: Mutex<WhisperState>,
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

/// The model in `file`, loaded now or kept from the last recording. A different model frees the one held.
fn loaded(file: &Path, flash: bool) -> Result<Arc<Loaded>, ProviderError> {
    whisper_rs::install_logging_hooks();
    {
        let held = HELD.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((loaded, _)) = held
            .loaded
            .as_ref()
            .filter(|(each, _)| each.file == file && each.flash == flash)
        {
            let loaded = Arc::clone(loaded);
            drop(held);
            touch(&loaded);
            return Ok(loaded);
        }
    }
    let text = file
        .to_str()
        .ok_or_else(|| bad_value("the model's path is not text"))?;
    let mut parameters = WhisperContextParameters::default();
    parameters.flash_attn(flash);
    let context = WhisperContext::new_with_params(text, parameters).map_err(|error| {
        internal(alloc::format!(
            "the model {} could not be loaded: {error}",
            file.display()
        ))
    })?;
    let state = context
        .create_state()
        .map_err(|error| internal(alloc::format!("whisper.cpp could not start: {error}")))?;
    let made = Arc::new(Loaded {
        file: file.to_path_buf(),
        flash,
        state: Mutex::new(state),
    });
    let mut held = HELD.lock().unwrap_or_else(PoisonError::into_inner);
    // Two recordings that asked at once loaded the same model: the first one kept stays.
    if let Some((loaded, _)) = held
        .loaded
        .as_ref()
        .filter(|(each, _)| each.file == file && each.flash == flash)
    {
        return Ok(Arc::clone(loaded));
    }
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
    file: &Path,
    settings: &WhisperSettings,
    samples: &[f32],
    language: Option<&str>,
    prompt: &str,
) -> Result<Transcript, ProviderError> {
    let model = loaded(file, settings.flash_attention)?;
    let strategy = if settings.beam_size > 1 {
        SamplingStrategy::BeamSearch {
            beam_size: i32::try_from(settings.beam_size).unwrap_or(5),
            patience: -1.0,
        }
    } else {
        SamplingStrategy::Greedy { best_of: 1 }
    };
    let mut params = FullParams::new(strategy);
    params.set_language(Some(language.unwrap_or("auto")));
    params.set_n_threads(settings.threads());
    params.set_translate(false);
    params.set_no_timestamps(true);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_special(false);
    params.set_print_timestamps(false);
    if !prompt.is_empty() {
        params.set_initial_prompt(prompt);
    }
    let started = Instant::now();
    let mut state = model.state.lock().unwrap_or_else(PoisonError::into_inner);
    state
        .full(params, samples)
        .map_err(|error| internal(alloc::format!("whisper.cpp could not transcribe: {error}")))?;
    let mut text = String::new();
    for segment in state.as_iter() {
        let piece = segment.to_str_lossy().map_err(|error| {
            internal(alloc::format!(
                "whisper.cpp's text could not be read: {error}"
            ))
        })?;
        text.push_str(&piece);
    }
    let detected = whisper_rs::get_lang_str(state.full_lang_id_from_state());
    drop(state);
    touch(&model);
    let mut transcript = Transcript::new(text.trim());
    let mut front = guatiao::Map::new();
    if let Some(detected) = detected {
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
    fn the_model_file_is_ggml_and_the_id() {
        let own = WhisperSettings::new("small-q5_1", "/models");
        assert_eq!(own.file(), Path::new("/models/ggml-small-q5_1.bin"));
        let other = WhisperSettings::new("mine", "/models");
        assert_eq!(other.file(), Path::new("/models/ggml-mine.bin"));
    }

    #[test]
    fn a_model_written_at_install_is_the_file_in_its_own_folder() {
        let root =
            std::env::temp_dir().join(format!("canoa-whispercpp-file-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("tiny")).expect("a model folder");
        let settings = WhisperSettings::new("tiny", root.to_string_lossy());
        assert_eq!(
            settings.file(),
            root.join("ggml-tiny.bin"),
            "the file of the folder until one is written"
        );
        std::fs::write(root.join("tiny").join(crate::GGML_FILE), b"g").expect("written");
        assert_eq!(settings.file(), root.join("tiny").join(crate::GGML_FILE));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn threads_are_the_settings_or_the_processors_up_to_eight() {
        let mut settings = WhisperSettings::new("base", "/models");
        assert!((1..=8).contains(&settings.threads()));
        settings.threads = Some(3);
        assert_eq!(settings.threads(), 3);
        settings.threads = Some(0);
        assert_eq!(settings.threads(), 1);
    }
}
