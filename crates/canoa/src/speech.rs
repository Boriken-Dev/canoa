//! The `speech_provider` kind: a provider that turns a stream of audio into text, and the object a recording runs on.
//!
//! It is not ACP. The app captures audio wherever it runs and the host passes it on in small chunks; the provider owns
//! what is specific to it (a program it runs, an endpoint it calls, a model it holds) and answers partial text while the
//! person speaks, then the final text with a front matter: what it learned about the speech that the next model, or the
//! person, may want. The person's own hints (language, vocabulary, a voice profile) go the other way, once, when a
//! recording opens.
//!
//! A provider's own settings are its configuration: a schema the library declares, with a form that says how to show it,
//! so any program that draws a guatiao provider's settings draws these.
//!
//! The models are plain data, the same in every language. What a host sends refuses a key it does not declare, so a field
//! a newer host adds is never dropped silently by an older provider; what a provider answers is read leniently.
//!
//! The audio is borrowed for the one call that carries it: a provider that keeps it copies it, once, itself.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use guatiao::library::{Object, ProviderError};
use guatiao::{FromValue, Schema, ToValue, Value};

#[cfg(feature = "speech-builtin")]
#[cfg_attr(docsrs, doc(cfg(feature = "speech-builtin")))]
pub mod builtin;

/// An [`AudioFormat::encoding`]: raw 16-bit signed little-endian samples, interleaved by channel.
pub const PCM_S16LE: &str = "pcm_s16le";
/// An [`AudioFormat::encoding`]: Opus packets, each one a chunk's whole content.
pub const OPUS: &str = "opus";

/// How audio is sent: its encoding, sample rate and channels.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(deny_unknown_fields)]
#[non_exhaustive]
pub struct AudioFormat {
    /// [`PCM_S16LE`] or [`OPUS`]; another word is a format a newer host may offer.
    pub encoding: String,
    /// Samples per second per channel: 16000 is what most recognizers want.
    pub rate: u32,
    /// How many channels; 1 is mono.
    #[map(default = default_channels())]
    pub channels: u8,
}

const fn default_channels() -> u8 {
    1
}

impl AudioFormat {
    /// `encoding` at `rate` samples a second, mono.
    #[must_use]
    pub fn new(encoding: impl Into<String>, rate: u32) -> Self {
        Self {
            encoding: encoding.into(),
            rate,
            channels: 1,
        }
    }

    /// 16-bit mono PCM at `rate`.
    #[must_use]
    pub fn pcm(rate: u32) -> Self {
        Self::new(PCM_S16LE, rate)
    }
}

/// What the person says about their own voice, for a provider that can use it. Every field is the person's own, entered by
/// them; nothing here is inferred.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Default, PartialEq, Eq)]
#[map(deny_unknown_fields)]
#[non_exhaustive]
pub struct VoiceProfile {
    /// How they speak the language, such as `Puerto Rican Spanish`.
    pub accent: Option<String>,
    /// The kind of voice, in their words, such as `male`, `female` or `child`.
    pub voice: Option<String>,
    /// Anything else a provider may know by name; a provider ignores a key it does not know.
    #[map(default)]
    pub traits: BTreeMap<String, String>,
}

/// What the provider is told before the first chunk. Every part is optional and a provider uses what it can.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Default, PartialEq, Eq)]
#[map(deny_unknown_fields)]
#[non_exhaustive]
pub struct Hints {
    /// The language spoken, as a BCP 47 tag (`en`, `es-PR`); absent asks the provider to find out.
    pub language: Option<String>,
    /// Words and names likely to be said, which a recognizer may favour.
    #[map(default)]
    pub vocabulary: Vec<String>,
    /// The person's voice profile.
    pub profile: Option<VoiceProfile>,
}

/// What a recording opens with.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(deny_unknown_fields)]
#[non_exhaustive]
pub struct OpenRequest {
    /// The format every chunk is in.
    pub format: AudioFormat,
    /// What the person told it.
    #[map(default)]
    pub hints: Hints,
}

impl OpenRequest {
    /// A recording in `format` with no hints.
    #[must_use]
    pub fn new(format: AudioFormat) -> Self {
        Self {
            format,
            hints: Hints::default(),
        }
    }
}

/// What a provider can do, from [`SpeechProvider::describe`]. Cheap, and the same on every call.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::guatiao)]
#[non_exhaustive]
pub struct SpeechInfo {
    /// Its name for people.
    pub title: String,
    /// One line on what it does.
    pub description: Option<String>,
    /// The [`AudioFormat::encoding`]s it takes; a recording in another is refused with `UNSUPPORTED`.
    pub encodings: Vec<String>,
    /// Whether it answers partial text while the person is still speaking.
    #[map(default)]
    pub partials: bool,
    /// The longest recording it takes, in seconds; absent says it has no limit of its own.
    pub max_seconds: Option<u32>,
    /// The [`Hints`] fields it uses (`language`, `vocabulary`, `profile`), so a form offers only what has an effect.
    #[map(default)]
    pub hints: Vec<String>,
}

impl SpeechInfo {
    /// A provider called `title` that takes `encodings`, answers no partial text and uses no hint.
    #[must_use]
    pub fn new(title: impl Into<String>, encodings: Vec<String>) -> Self {
        Self {
            title: title.into(),
            description: None,
            encodings,
            partials: false,
            max_seconds: None,
            hints: Vec::new(),
        }
    }
}

/// What a chunk of audio made the provider hear.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Default, PartialEq, Eq)]
#[map(crate = ::guatiao)]
#[non_exhaustive]
pub struct Heard {
    /// The whole text so far, replacing the last partial, when it changed; absent when it did not.
    pub partial: Option<String>,
}

impl Heard {
    /// Nothing new.
    #[must_use]
    pub const fn nothing() -> Self {
        Self { partial: None }
    }

    /// The text so far is `text`.
    #[must_use]
    pub fn partial(text: impl Into<String>) -> Self {
        Self {
            partial: Some(text.into()),
        }
    }
}

/// The final answer of a recording.
///
/// `front_matter` is a map of what the provider knows about the speech. Keys a provider uses when it knows them, so a reader
/// finds the same thing under the same name: `language` (what was detected, a BCP 47 tag), `confidence` (0 to 1),
/// `duration_ms`, `speaker` (what it heard of the voice, in words) and `notes` (anything it wants the next model to know).
/// Any other key is the provider's own.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq)]
#[map(crate = ::guatiao)]
#[non_exhaustive]
pub struct Transcript {
    /// The text, as the person said it.
    pub text: String,
    /// What else the provider knows, a map of text keys to values.
    #[map(default = empty_map())]
    pub front_matter: Value,
}

fn empty_map() -> Value {
    Value::from(guatiao::Map::new())
}

impl Transcript {
    /// `text` and an empty front matter.
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            front_matter: empty_map(),
        }
    }

    /// Sets one front matter key, replacing the one it had; a front matter that is not a map is replaced by one.
    #[must_use]
    pub fn with(mut self, key: &str, value: impl Into<Value>) -> Self {
        let mut map = match <&guatiao::Map>::try_from(&self.front_matter) {
            Ok(map) => map.clone(),
            Err(_) => guatiao::Map::new(),
        };
        map.set(key, value.into());
        self.front_matter = Value::from(map);
        self
    }
}

/// A provider of speech to text. One instance is one configuration of one provider.
#[guatiao::kind(name = "speech_provider")]
pub trait SpeechProvider: Send + Sync {
    /// What it can do. Cheap, and the same on every call.
    fn describe(&self) -> Result<SpeechInfo, ProviderError>;

    /// Starts a recording. Nothing is started for a recording it refuses: `UNSUPPORTED` (with the encodings it takes) for a
    /// format it does not read, `BAD_VALUE` for a setting that cannot be used. The provider may start a process or open a
    /// connection here, and its session ends them.
    fn open(&self, request: OpenRequest) -> Result<Object<dyn SpeechSession>, ProviderError>;
}

/// One recording: audio goes in a chunk at a time, text comes out.
///
/// A session's methods may be called from different threads, one at a time. Every session ends with [`finish`] or
/// [`cancel`], and dropping one is a cancel.
///
/// [`finish`]: SpeechSession::finish
/// [`cancel`]: SpeechSession::cancel
#[guatiao::kind(object, name = "speech_session")]
pub trait SpeechSession: Send + Sync {
    /// Takes the next chunk, in order, in the format the recording opened with. It returns when the provider has taken the
    /// chunk, so a provider that cannot keep up slows the caller down rather than growing a queue. The chunk is borrowed
    /// for this call.
    fn push(&self, audio: &[u8]) -> Result<Heard, ProviderError>;

    /// Ends the audio and answers the final text, when the provider has it. A provider that fails answers the reason in the
    /// error, never an empty text.
    fn finish(&self) -> Result<Transcript, ProviderError>;

    /// Abandons the recording: whatever the provider started is ended, and nothing is answered.
    fn cancel(&self) -> Result<(), ProviderError>;
}

/// `UNSUPPORTED` for a recording in a format the provider does not read, naming the ones it does.
#[must_use]
pub fn unsupported_format(format: &AudioFormat, takes: &[&str]) -> ProviderError {
    ProviderError::new(
        guatiao::Status::GUATIAO_ERR_UNSUPPORTED,
        &alloc::format!(
            "the encoding {:?} is not one this provider reads; it reads {}",
            format.encoding,
            takes.join(", ")
        ),
    )
}

/// `BAD_VALUE` with `why`.
#[must_use]
pub fn bad_value(why: impl Into<String>) -> ProviderError {
    ProviderError::new(guatiao::Status::GUATIAO_ERR_BAD_VALUE, &why.into())
}

/// `INTERNAL` with `why`.
#[must_use]
pub fn internal(why: impl Into<String>) -> ProviderError {
    ProviderError::new(guatiao::Status::GUATIAO_ERR_INTERNAL, &why.into())
}
