//! The Piper provider: a local neural voice engine, run as a program, with the voices in a folder.
//!
//! A voice is a model file `NAME.onnx` and its `NAME.onnx.json` in the voices folder; the voice's id is `NAME`. The model's
//! json says its language, its sample rate and how many speakers it holds, and a speaker of a model with several is asked for
//! as `NAME#N`. The text is handed to the program a sentence at a time and its raw samples are the audio, so a client hears the
//! first sentence while the rest is being made. The text never leaves the computer.

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;
use std::path::{Path, PathBuf};

use coqui_acp_api::{ProcessSpawner, SpawnSpec};
use guatiao::library::{Object, ProviderError};
use guatiao::{FromValue, Schema, ToValue};
use guatiao_intake::Form;

use super::spoken::{Head, Spoken};
use crate::speech::output::{
    Leaves, OutputInfo, SayRequest, SpeechOutput, SpeechUtterance, SpeedRange, Voice, VoiceList,
    VoiceQuery, checked_text, sentences, unknown_choice,
};
use crate::speech::{AudioFormat, PCM_S16LE};
use crate::status::{bad_value, internal};

/// The sample rate of a model whose json does not say.
const DEFAULT_RATE: u32 = 22_050;

/// The most speakers of one model that are listed by name; the rest are still asked for as `NAME#N`.
const LISTED_SPEAKERS: usize = 32;

/// The slowest and fastest speech, as multiples of normal.
const SPEED: SpeedRange = SpeedRange::new(0.5, 2.0);

const fn default_timeout() -> u32 {
    60
}

/// What the Piper provider is configured with.
#[derive(ToValue, FromValue, Schema, Form, Debug, Clone, PartialEq)]
#[form(section(id = "piper", label = "Piper"))]
#[non_exhaustive]
pub struct PiperSettings {
    /// The Piper program: a path, or a name found on the search path.
    #[schema(section = "piper", title = "Program")]
    #[form(placeholder = "piper")]
    pub program: String,
    /// The folder of voices: each `NAME.onnx` with its `NAME.onnx.json`.
    #[schema(section = "piper", title = "Voices folder")]
    pub voices_dir: String,
    /// The voice used when a request names none; absent is the first voice, in the request's language when it has one.
    #[schema(section = "piper", title = "Default voice")]
    pub default_voice: Option<String>,
    /// How long the program may go without a sound before it is stopped, in seconds.
    #[map(default = default_timeout())]
    #[schema(advanced, section = "piper", title = "Time to start")]
    #[form(unit = "s")]
    pub timeout_seconds: u32,
}

impl PiperSettings {
    /// Settings that run `program` over the voices in `voices_dir`, with a minute to start.
    #[must_use]
    pub fn new(program: impl Into<String>, voices_dir: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            voices_dir: voices_dir.into(),
            default_voice: None,
            timeout_seconds: default_timeout(),
        }
    }
}

/// The Piper provider.
#[derive(guatiao::Provider)]
#[provider(SpeechOutput, id = "speech_say_piper", config = PiperSettings, form = PiperSettings)]
pub struct PiperSpeech {
    settings: PiperSettings,
}

impl From<PiperSettings> for PiperSpeech {
    fn from(settings: PiperSettings) -> Self {
        Self { settings }
    }
}

/// What a model's json says that matters here.
struct Model {
    rate: u32,
    language: Option<String>,
    speakers: u32,
    names: Vec<(String, u32)>,
}

fn read_model(json: &Path) -> Model {
    let mut model = Model {
        rate: DEFAULT_RATE,
        language: None,
        speakers: 1,
        names: Vec::new(),
    };
    let Some(doc) = std::fs::read(json)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
    else {
        return model;
    };
    if let Some(rate) = doc["audio"]["sample_rate"]
        .as_u64()
        .and_then(|rate| u32::try_from(rate).ok())
        .filter(|rate| *rate > 0)
    {
        model.rate = rate;
    }
    model.language = doc["language"]["code"]
        .as_str()
        .map(|code| code.replace('_', "-"));
    model.speakers = doc["num_speakers"]
        .as_u64()
        .and_then(|count| u32::try_from(count).ok())
        .unwrap_or(1)
        .max(1);
    if let Some(map) = doc["speaker_id_map"].as_object() {
        let mut names: Vec<(String, u32)> = map
            .iter()
            .filter_map(|(name, id)| Some((name.clone(), u32::try_from(id.as_u64()?).ok()?)))
            .collect();
        names.sort_by_key(|(_, id)| *id);
        model.names = names;
    }
    model
}

/// A voice's id is the name of its model file: not a path.
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
        && !id.starts_with('.')
}

impl PiperSpeech {
    fn dir(&self) -> Result<PathBuf, ProviderError> {
        let dir = self.settings.voices_dir.trim();
        if dir.is_empty() {
            return Err(bad_value("the voices folder is not set"));
        }
        Ok(PathBuf::from(dir))
    }

    /// Every voice in the folder, with what its model says.
    fn installed(&self) -> Result<Vec<(Voice, Model)>, ProviderError> {
        let dir = self.dir()?;
        let entries = std::fs::read_dir(&dir).map_err(|error| {
            internal(alloc::format!(
                "the voices folder {} cannot be read: {error}",
                dir.display()
            ))
        })?;
        let mut found = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(stem) = path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_suffix(".onnx"))
            else {
                continue;
            };
            if !valid_id(stem) || !path.is_file() {
                continue;
            }
            let model = read_model(&dir.join(alloc::format!("{stem}.onnx.json")));
            let quality = stem
                .rsplit('-')
                .next()
                .filter(|q| matches!(*q, "x_low" | "low" | "medium" | "high"));
            let mut voice = Voice::new(stem, stem.replace(['-', '_'], " "));
            voice.language.clone_from(&model.language);
            voice.description = Some(match (quality, model.speakers) {
                (Some(quality), 1) => alloc::format!("{quality} quality"),
                (Some(quality), speakers) => {
                    alloc::format!("{quality} quality, {speakers} speakers")
                }
                (None, 1) => String::new(),
                (None, speakers) => alloc::format!("{speakers} speakers"),
            })
            .filter(|text| !text.is_empty());
            found.push((voice, model));
        }
        found.sort_by(|a, b| a.0.id.cmp(&b.0.id));
        Ok(found)
    }

    fn program(&self) -> Result<&str, ProviderError> {
        let program = self.settings.program.trim();
        if program.is_empty() {
            return Err(bad_value(
                "the Piper program is not set: install it from the Store or name it",
            ));
        }
        Ok(program)
    }
}

impl SpeechOutput for PiperSpeech {
    fn describe(&self) -> Result<OutputInfo, ProviderError> {
        let mut info = OutputInfo::new("Piper", vec![PCM_S16LE.to_string()]);
        info.description = Some("A local neural voice engine with the voices you keep".to_owned());
        info.speed = Some(SPEED);
        info.uses = ["voice", "speed", "language"]
            .map(ToString::to_string)
            .to_vec();
        info.leaves = Leaves::No;
        info.words = Some(
            "Piper runs on the host's computer with voices kept there; the text is sent nowhere."
                .to_owned(),
        );
        Ok(info)
    }

    fn voices(
        &self,
        query: VoiceQuery,
        _spawner: Object<dyn ProcessSpawner>,
    ) -> Result<VoiceList, ProviderError> {
        let mut voices = Vec::new();
        for (voice, model) in self.installed()? {
            if model.speakers > 1 && !model.names.is_empty() {
                // A voice per speaker, named as the model names them.
                voices.push(voice.clone());
                for (name, id) in model.names.iter().take(LISTED_SPEAKERS) {
                    let mut speaker = Voice::new(
                        alloc::format!("{}#{id}", voice.id),
                        alloc::format!("{} ({name})", voice.name),
                    );
                    speaker.language.clone_from(&voice.language);
                    voices.push(speaker);
                }
            } else {
                voices.push(voice);
            }
        }
        Ok(VoiceList::new(voices).speaking(query.language.as_deref()))
    }

    fn say(
        &self,
        request: SayRequest,
        spawner: Object<dyn ProcessSpawner>,
    ) -> Result<Object<dyn SpeechUtterance>, ProviderError> {
        let text = checked_text(&request, None)?;
        let installed = self.installed()?;
        let ids: Vec<String> = installed
            .iter()
            .map(|(voice, _)| voice.id.clone())
            .collect();
        let named = request
            .voice
            .clone()
            .or_else(|| self.settings.default_voice.clone())
            .filter(|voice| !voice.is_empty());
        let (id, speaker) = match &named {
            Some(named) => match named.split_once('#') {
                Some((id, speaker)) => (
                    id.to_owned(),
                    Some(speaker.parse::<u32>().map_err(|_| {
                        bad_value(alloc::format!(
                            "{named:?} names no speaker: it is NAME#NUMBER"
                        ))
                    })?),
                ),
                None => (named.clone(), None),
            },
            None => {
                let primary = |tag: &str| {
                    tag.split(['-', '_'])
                        .next()
                        .unwrap_or(tag)
                        .to_ascii_lowercase()
                };
                let wanted = request.language.as_deref().map(primary);
                let pick = installed
                    .iter()
                    .find(|(voice, _)| {
                        wanted.is_some() && voice.language.as_deref().map(primary) == wanted
                    })
                    .or_else(|| installed.first())
                    .ok_or_else(|| {
                        bad_value("the voices folder holds no voice: install one from the Store")
                    })?;
                (pick.0.id.clone(), None)
            }
        };
        let (_, model) = installed
            .iter()
            .find(|(voice, _)| voice.id == id)
            .ok_or_else(|| unknown_choice("voice", &id, &ids))?;
        if speaker.is_some_and(|speaker| speaker >= model.speakers) {
            return Err(bad_value(alloc::format!(
                "{id} has {} speakers, numbered from 0",
                model.speakers
            )));
        }
        let mut spec = SpawnSpec::new(self.program()?)
            .arg("--model")
            .arg(
                self.dir()?
                    .join(alloc::format!("{id}.onnx"))
                    .to_string_lossy(),
            )
            .arg("--output-raw")
            .arg("--quiet");
        if let Some(speed) = request.speed {
            spec = spec
                .arg("--length_scale")
                .arg(alloc::format!("{:.3}", 1.0 / SPEED.clamp(speed)));
        }
        if let Some(speaker) = speaker.filter(|speaker| *speaker > 0) {
            spec = spec.arg("--speaker").arg(speaker.to_string());
        }
        let idle = Duration::from_secs(u64::from(self.settings.timeout_seconds.max(1)));
        Ok(Spoken::start(
            &spawner,
            spec,
            Some(sentences(text)),
            AudioFormat::pcm(model.rate),
            Head::Raw,
            idle,
        )?
        .into_object())
    }
}

impl core::fmt::Debug for PiperSpeech {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PiperSpeech")
            .field("program", &self.settings.program)
            .finish_non_exhaustive()
    }
}
