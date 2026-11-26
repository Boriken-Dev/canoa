//! The `speech_output` kind: a provider that turns text into audio, and the object one utterance runs on.
//!
//! It is the other side of [`speech_provider`](super::SpeechProvider). A client sends text and the provider answers audio
//! in order, a chunk at a time, so a client can start playing before the provider has finished. The provider owns what is
//! specific to it (a program it runs, an endpoint it calls, a voice it holds) and says what the person may choose: its
//! voices, its models, its styles and the speeds it takes, and where the text goes.
//!
//! The models are plain data, the same in every language. What a host sends refuses a key it does not declare, so a field
//! a newer host adds is never dropped silently by an older provider; what a provider answers is read leniently.

use alloc::string::String;
use alloc::vec::Vec;

use coqui_acp_api::ProcessSpawner;
use guatiao::library::{Object, ProviderError};
use guatiao::{FromValue, Schema, ToValue};

use super::AudioFormat;
use crate::store::source::SourceFetch;

pub mod account;

pub use coqui_acp_api::READ_TIMED_OUT;

/// One thing a person chooses between: a model or a style.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::guatiao)]
#[non_exhaustive]
pub struct Choice {
    /// What a request names.
    pub id: String,
    /// What a person is shown.
    pub label: String,
    /// One line on what it does.
    pub description: Option<String>,
}

impl Choice {
    /// A choice named `id` and shown as `label`.
    #[must_use]
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            description: None,
        }
    }
}

/// One voice a provider can speak with.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::guatiao)]
#[non_exhaustive]
pub struct Voice {
    /// What a request names.
    pub id: String,
    /// What a person is shown.
    pub name: String,
    /// The language it speaks, as a BCP 47 tag (`en-US`); absent when it speaks several or the provider does not say.
    pub language: Option<String>,
    /// `female`, `male` or `neutral`, when the provider says.
    pub gender: Option<String>,
    /// One line on it.
    pub description: Option<String>,
    /// The models it works with; empty says every model the provider lists.
    #[map(default)]
    pub models: Vec<String>,
}

impl Voice {
    /// A voice named `id` and shown as `name`.
    #[must_use]
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            language: None,
            gender: None,
            description: None,
            models: Vec::new(),
        }
    }
}

/// Whether the text leaves the computer the provider runs on.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Copy, Default, PartialEq, Eq)]
#[map(crate = ::guatiao, rename_all = "snake_case")]
#[non_exhaustive]
pub enum Leaves {
    /// It stays on the computer.
    #[default]
    No,
    /// It may: the provider hands it to something that decides for itself.
    Maybe,
    /// It does: it goes to the address in [`OutputInfo::endpoint`].
    Yes,
}

/// The speeds a provider takes, as multiples of normal speech (1 is normal).
#[derive(ToValue, FromValue, Schema, Debug, Clone, Copy, PartialEq)]
#[map(crate = ::guatiao)]
#[non_exhaustive]
pub struct SpeedRange {
    /// The slowest.
    pub min: f64,
    /// The fastest.
    pub max: f64,
}

impl SpeedRange {
    /// Speeds from `min` to `max`.
    #[must_use]
    pub const fn new(min: f64, max: f64) -> Self {
        Self { min, max }
    }

    /// `speed` held to the range.
    #[must_use]
    pub const fn clamp(self, speed: f64) -> f64 {
        speed.clamp(self.min, self.max)
    }
}

/// What a provider can do, from [`SpeechOutput::describe`]. Cheap, and the same on every call.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq)]
#[map(crate = ::guatiao)]
#[non_exhaustive]
pub struct OutputInfo {
    /// Its name for people.
    pub title: String,
    /// One line on what it does.
    pub description: Option<String>,
    /// The audio encodings it answers (`pcm_s16le`).
    pub encodings: Vec<String>,
    /// The models it offers, when it has several; empty when it has one or none.
    #[map(default)]
    pub models: Vec<Choice>,
    /// The model it uses when a request names none.
    pub default_model: Option<String>,
    /// The styles it offers (a way of speaking: cheerful, calm); empty when it has none.
    #[map(default)]
    pub styles: Vec<Choice>,
    /// The speeds it takes; absent says it has one speed.
    pub speed: Option<SpeedRange>,
    /// The [`SayRequest`] fields it uses (`voice`, `model`, `style`, `speed`, `language`), so a form offers only what has an
    /// effect.
    #[map(default)]
    pub uses: Vec<String>,
    /// Whether the text leaves the computer.
    #[map(default)]
    pub leaves: Leaves,
    /// The address the text goes to, when it goes to one.
    pub endpoint: Option<String>,
    /// A sentence on where the text goes and what is done with it, for people.
    pub words: Option<String>,
    /// The most characters one request takes; absent says no limit of its own.
    pub max_chars: Option<u32>,
    /// The `https` places the `fetch` that [`SpeechOutput::voices`] is handed may reach: an origin (`https://host`) or a prefix
    /// with a path (`https://host/repo/`) that a URL has to start with, redirects included. None says `voices` reads no document.
    #[map(default)]
    pub endpoints: Vec<String>,
}

impl OutputInfo {
    /// A provider called `title` that answers `encodings`, with no model, style or speed to choose and text that stays here.
    #[must_use]
    pub fn new(title: impl Into<String>, encodings: Vec<String>) -> Self {
        Self {
            title: title.into(),
            description: None,
            encodings,
            models: Vec::new(),
            default_model: None,
            styles: Vec::new(),
            speed: None,
            uses: Vec::new(),
            leaves: Leaves::No,
            endpoint: None,
            words: None,
            max_chars: None,
            endpoints: Vec::new(),
        }
    }
}

/// What a client asks of [`SpeechOutput::voices`].
#[derive(ToValue, FromValue, Schema, Debug, Clone, Default, PartialEq, Eq)]
#[map(deny_unknown_fields)]
#[non_exhaustive]
pub struct VoiceQuery {
    /// Only the voices that speak this language (a BCP 47 tag or its primary subtag); absent asks for all of them.
    pub language: Option<String>,
}

impl VoiceQuery {
    /// Only the voices that speak `language`.
    #[must_use]
    pub fn in_language(language: impl Into<String>) -> Self {
        Self {
            language: Some(language.into()),
        }
    }
}

/// The voices a provider has.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Default, PartialEq, Eq)]
#[map(crate = ::guatiao)]
#[non_exhaustive]
pub struct VoiceList {
    /// Every voice, in the order a person should see them.
    #[map(default)]
    pub voices: Vec<Voice>,
    /// The models as the provider knows them now, in the order a person should see them (a label may carry what a service says of
    /// one, such as the date it ends). Empty leaves the models [`OutputInfo::models`] lists.
    #[map(default)]
    pub models: Vec<Choice>,
}

impl VoiceList {
    /// These voices.
    #[must_use]
    pub const fn new(voices: Vec<Voice>) -> Self {
        Self {
            voices,
            models: Vec::new(),
        }
    }

    /// These voices with the models as the provider knows them now.
    #[must_use]
    pub fn with_models(mut self, models: Vec<Choice>) -> Self {
        self.models = models;
        self
    }

    /// Without the voices that do not speak `language`: a voice with no language stays, and a tag matches by its primary
    /// subtag as well as whole.
    #[must_use]
    pub fn speaking(mut self, language: Option<&str>) -> Self {
        if let Some(language) = language.filter(|tag| !tag.is_empty()) {
            let primary = |tag: &str| {
                tag.split(['-', '_'])
                    .next()
                    .unwrap_or(tag)
                    .to_ascii_lowercase()
            };
            let wanted = primary(language);
            self.voices.retain(|voice| {
                voice
                    .language
                    .as_deref()
                    .is_none_or(|tag| primary(tag) == wanted)
            });
        }
        self
    }
}

/// What an utterance starts with.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq)]
#[map(deny_unknown_fields)]
#[non_exhaustive]
pub struct SayRequest {
    /// The text to speak, as plain text.
    pub text: String,
    /// A voice the provider listed; absent is the provider's own.
    pub voice: Option<String>,
    /// A model the provider listed; absent is its default.
    pub model: Option<String>,
    /// A style the provider listed.
    pub style: Option<String>,
    /// A multiple of normal speech, 1 being normal; a provider holds it to the range it takes.
    pub speed: Option<f64>,
    /// The language of the text, as a BCP 47 tag, for a provider that picks a voice by it.
    pub language: Option<String>,
}

impl SayRequest {
    /// `text` with every choice left to the provider.
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            voice: None,
            model: None,
            style: None,
            speed: None,
            language: None,
        }
    }
}

/// A provider of text to speech. One instance is one configuration of one provider.
#[guatiao::kind(name = "speech_output")]
pub trait SpeechOutput: Send + Sync {
    /// What it can do. Cheap, and the same on every call.
    fn describe(&self) -> Result<OutputInfo, ProviderError>;

    /// The voices it has now, for `query`. A provider that learns them from a program or a server asks it here, so a listing
    /// may take time and fail with the reason.
    ///
    /// `spawner` is the host's: every program a provider runs starts through it, and the provider destroys it, even when it
    /// refuses. `fetch` is the host's too: an `https` document read through the host's own client and proxy, from the places
    /// [`OutputInfo::endpoints`] declares and nowhere else, kept by the host for `[host.content] ttl`. The provider destroys
    /// it as it does the spawner.
    fn voices(
        &self,
        query: VoiceQuery,
        spawner: Object<dyn ProcessSpawner>,
        fetch: Object<dyn SourceFetch>,
    ) -> Result<VoiceList, ProviderError>;

    /// Starts speaking `request`. Nothing is started for a request it refuses: `UNSUPPORTED` for a choice it does not have,
    /// `BAD_VALUE` for a setting or a text it cannot use. The provider may start a process or open a connection here, and the
    /// utterance ends them.
    ///
    /// `spawner` is the host's, as for [`voices`](Self::voices).
    fn say(
        &self,
        request: SayRequest,
        spawner: Object<dyn ProcessSpawner>,
    ) -> Result<Object<dyn SpeechUtterance>, ProviderError>;
}

/// One utterance: audio comes out a chunk at a time, in order, while the provider is still speaking.
///
/// A utterance's methods may be called from different threads. Every one ends with the audio read to its end, an error, or
/// [`stop`]; dropping one is a stop.
///
/// [`stop`]: SpeechUtterance::stop
#[guatiao::kind(object, name = "speech_utterance")]
pub trait SpeechUtterance: Send + Sync {
    /// The format of every byte [`read`](Self::read) answers; the same on every call.
    fn format(&self) -> Result<AudioFormat, ProviderError>;

    /// Copies the next audio into `dst` and answers how many bytes: `0` once the utterance is over and every byte was read, and
    /// [`READ_TIMED_OUT`] after `timeout_ms` with nothing (`0` polls). An error carries why the provider could not finish; the
    /// audio before it was answered first.
    fn read(&self, dst: &mut [u8], timeout_ms: u64) -> Result<i64, ProviderError>;

    /// Ends the utterance: whatever the provider started is ended, and a later read answers `0`.
    fn stop(&self) -> Result<(), ProviderError>;
}

/// The longest piece of text a program is handed on one line, in characters.
const LINE: usize = 400;

/// `text` as the lines a program that speaks one line at a time is handed: one for each sentence, so audio starts after the
/// first sentence rather than the whole text, and a very long sentence is cut at a comma or a space. Empty text, and text with
/// nothing to say in it, is no lines.
#[must_use]
pub fn sentences(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();
    let flush = |current: &mut String, lines: &mut Vec<String>| {
        let said = current.split_whitespace().collect::<Vec<_>>().join(" ");
        current.clear();
        if said.chars().any(char::is_alphanumeric) {
            lines.extend(cut(&said));
        }
    };
    while let Some(c) = chars.next() {
        match c {
            '\n' | '\r' => flush(&mut current, &mut lines),
            '.' | '!' | '?' | '\u{2026}' | '\u{3002}' | '\u{ff01}' | '\u{ff1f}' => {
                current.push(c);
                // A full stop ends a sentence when what follows is a space or nothing: not in `3.5` or `e.g.x`.
                let ends = chars
                    .peek()
                    .is_none_or(|next| next.is_whitespace() || is_closer(*next));
                if ends && !matches!(chars.peek(), Some('.' | '!' | '?')) {
                    while chars.peek().is_some_and(|next| is_closer(*next)) {
                        current.extend(chars.next());
                    }
                    flush(&mut current, &mut lines);
                }
            }
            _ => current.push(c),
        }
    }
    flush(&mut current, &mut lines);
    lines
}

const fn is_closer(c: char) -> bool {
    matches!(
        c,
        '"' | '\'' | ')' | ']' | '\u{201d}' | '\u{2019}' | '\u{bb}'
    )
}

/// One sentence in pieces of at most [`LINE`] characters.
fn cut(sentence: &str) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut rest = sentence;
    while rest.chars().count() > LINE {
        let limit = rest
            .char_indices()
            .nth(LINE)
            .map_or(rest.len(), |(index, _)| index);
        let head = &rest[..limit];
        let at = head
            .rfind([',', ';', ':'])
            .map(|index| index + 1)
            .or_else(|| head.rfind(' ').map(|index| index + 1))
            .filter(|index| *index > 0)
            .unwrap_or(limit);
        pieces.push(rest[..at].trim().to_owned());
        rest = rest[at..].trim_start();
    }
    if !rest.is_empty() {
        pieces.push(rest.to_owned());
    }
    pieces
}

/// `UNSUPPORTED` for a choice the provider does not have, naming the ones it does.
#[must_use]
pub fn unknown_choice(what: &str, wanted: &str, has: &[String]) -> ProviderError {
    crate::status::unsupported(alloc::format!(
        "this provider has no {what} {wanted:?}{}",
        if has.is_empty() {
            String::new()
        } else {
            alloc::format!("; it has {}", has.join(", "))
        }
    ))
}

/// The request's own text, checked: empty text is `BAD_VALUE`, and text past `max` characters is too.
///
/// # Errors
///
/// `BAD_VALUE` with the reason.
pub fn checked_text(request: &SayRequest, max: Option<u32>) -> Result<&str, ProviderError> {
    let text = request.text.trim();
    if text.is_empty() {
        return Err(crate::status::bad_value("there is no text to speak"));
    }
    if let Some(max) = max
        && text.chars().count() > usize::try_from(max).unwrap_or(usize::MAX)
    {
        return Err(crate::status::bad_value(alloc::format!(
            "this provider takes at most {max} characters at once; send the text in parts"
        )));
    }
    Ok(text)
}
