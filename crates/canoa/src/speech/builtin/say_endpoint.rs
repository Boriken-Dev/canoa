//! The endpoint provider for speech output: an HTTP text-to-speech endpoint, in the shape of the `audio/speech` call that
//! servers copying OpenAI's answer.
//!
//! It makes the call: a `POST` of JSON with the `model`, the `input` text, the `voice`, `response_format` `pcm`, and the `speed`
//! and `instructions` (the style) the request chose; the answer is the audio as raw 16-bit mono samples at the setting's `rate`,
//! read as it arrives so playing starts before the server has finished.
//!
//! The lists of models, voices and styles are what the settings say: a library that knows a service's own lists builds this
//! provider over them. The text, and the key when one is set, go to the address and nowhere else.

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use crate::store::source::SourceFetch;
use coqui_acp_api::ProcessSpawner;
use guatiao::library::{Object, ProviderError};
use guatiao::{FromValue, Schema, Secret, ToValue};
use guatiao_intake::Form;

use super::spoken::{Head, Spoken};
use crate::speech::output::{
    Choice, Leaves, OutputInfo, SayRequest, SpeechOutput, SpeechUtterance, SpeedRange, Voice,
    VoiceList, VoiceQuery, checked_text, unknown_choice,
};
use crate::speech::{AudioFormat, PCM_S16LE};
use crate::status::{bad_value, internal};

/// The most of an error answer's body that a message carries, in bytes.
const MAX_ERROR_BODY: usize = 300;

/// The speeds the call takes.
const SPEED: SpeedRange = SpeedRange::new(0.25, 4.0);

/// The longest a whole answer may take to arrive, in seconds.
const BODY_SECONDS: u64 = 900;

const fn default_rate() -> u32 {
    24_000
}

const fn default_timeout() -> u32 {
    60
}

const fn default_max_chars() -> u32 {
    4096
}

/// What the endpoint provider is configured with.
#[derive(ToValue, FromValue, Schema, Form, Debug, Clone)]
#[form(section(id = "endpoint", label = "Endpoint"))]
#[non_exhaustive]
pub struct EndpointSaySettings {
    /// The address the text is posted to, such as `https://speech.example.test/v1/audio/speech`.
    #[schema(section = "endpoint", title = "Address")]
    #[form(placeholder = "https://speech.example.test/v1/audio/speech")]
    pub url: String,
    /// A key sent as `Authorization: Bearer`, when the endpoint asks for one.
    #[schema(section = "endpoint", title = "Key")]
    #[form(widget = "password")]
    pub api_key: Option<Secret<String>>,
    /// The model used when a request names none.
    #[schema(section = "endpoint", title = "Model")]
    pub model: Option<String>,
    /// The voice used when a request names none.
    #[schema(section = "endpoint", title = "Voice")]
    pub voice: Option<String>,
    /// The models to offer.
    #[map(default)]
    #[schema(section = "endpoint", title = "Models offered")]
    pub models: Vec<String>,
    /// The voices to offer.
    #[map(default)]
    #[schema(section = "endpoint", title = "Voices offered")]
    pub voices: Vec<String>,
    /// The styles to offer, each `name=what to ask the voice to sound like`, sent as the call's `instructions`.
    #[map(default)]
    #[schema(section = "endpoint", title = "Styles offered")]
    pub styles: Vec<String>,
    /// The samples a second of the audio it answers.
    #[map(default = default_rate())]
    #[schema(advanced, section = "endpoint", title = "Sample rate")]
    #[form(unit = "Hz")]
    pub rate: u32,
    /// The most characters sent in one call.
    #[map(default = default_max_chars())]
    #[schema(advanced, section = "endpoint", title = "Longest text")]
    pub max_chars: u32,
    /// How long the server may take to start answering, and then to go without a sound, in seconds.
    #[map(default = default_timeout())]
    #[schema(advanced, section = "endpoint", title = "Time to answer")]
    #[form(unit = "s")]
    pub timeout_seconds: u32,
}

impl EndpointSaySettings {
    /// Settings that post to `url`, with no key, model or voice, 24 kHz audio, 4096 characters and a minute to answer.
    #[must_use]
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            api_key: None,
            model: None,
            voice: None,
            models: Vec::new(),
            voices: Vec::new(),
            styles: Vec::new(),
            rate: default_rate(),
            max_chars: default_max_chars(),
            timeout_seconds: default_timeout(),
        }
    }

    /// The models offered.
    fn model_list(&self) -> Vec<Choice> {
        self.models
            .iter()
            .filter(|name| !name.trim().is_empty())
            .map(|name| Choice::new(name.trim(), name.trim()))
            .collect()
    }

    fn voice_list(&self) -> Vec<Voice> {
        self.voices
            .iter()
            .filter(|name| !name.trim().is_empty())
            .map(|name| Voice::new(name.trim(), name.trim()))
            .collect()
    }

    /// The styles as (name, instruction).
    fn style_list(&self) -> Vec<(String, String)> {
        self.styles
            .iter()
            .map(String::as_str)
            .filter(|entry| !entry.trim().is_empty())
            .map(|entry| match entry.split_once('=') {
                Some((name, text)) => (name.trim().to_owned(), text.trim().to_owned()),
                None => (entry.trim().to_owned(), entry.trim().to_owned()),
            })
            .collect()
    }
}

/// The host part of an `http` or `https` address.
fn host_of(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
}

/// The endpoint provider for speech output.
#[derive(guatiao::Provider)]
#[provider(SpeechOutput, id = "speech_say_endpoint", config = EndpointSaySettings, form = EndpointSaySettings)]
pub struct EndpointSay {
    settings: EndpointSaySettings,
}

impl From<EndpointSaySettings> for EndpointSay {
    fn from(settings: EndpointSaySettings) -> Self {
        Self { settings }
    }
}

impl SpeechOutput for EndpointSay {
    fn describe(&self) -> Result<OutputInfo, ProviderError> {
        let settings = &self.settings;
        let mut info = OutputInfo::new("Endpoint", vec![PCM_S16LE.to_string()]);
        info.description =
            Some("Sends the text to a text-to-speech address and model you name".to_owned());
        info.models = settings.model_list();
        info.default_model = settings
            .model
            .clone()
            .filter(|model| !model.is_empty())
            .or_else(|| info.models.first().map(|choice| choice.id.clone()));
        info.styles = settings
            .style_list()
            .into_iter()
            .map(|(name, text)| {
                let mut choice = Choice::new(&name, &name);
                choice.description = Some(text);
                choice
            })
            .collect();
        info.speed = Some(SPEED);
        info.uses = [
            (!settings.voice_list().is_empty() || settings.voice.is_some()).then_some("voice"),
            (!info.models.is_empty() || settings.model.is_some()).then_some("model"),
            (!info.styles.is_empty()).then_some("style"),
            Some("speed"),
        ]
        .into_iter()
        .flatten()
        .map(ToString::to_string)
        .collect();
        info.leaves = Leaves::Yes;
        info.endpoint = Some(settings.url.clone());
        info.words = Some(alloc::format!(
            "The text goes to {}{}, which turns it into audio and sends it back.",
            host_of(&settings.url).unwrap_or_else(|| "the address you named".into()),
            if settings.api_key.is_some() {
                ", with your key"
            } else {
                ""
            }
        ));
        info.max_chars = Some(settings.max_chars);
        Ok(info)
    }

    fn voices(
        &self,
        query: VoiceQuery,
        _spawner: Object<dyn ProcessSpawner>,
        _fetch: Object<dyn SourceFetch>,
    ) -> Result<VoiceList, ProviderError> {
        Ok(VoiceList::new(self.settings.voice_list()).speaking(query.language.as_deref()))
    }

    fn say(
        &self,
        request: SayRequest,
        _spawner: Object<dyn ProcessSpawner>,
    ) -> Result<Object<dyn SpeechUtterance>, ProviderError> {
        let settings = &self.settings;
        if settings.url.trim().is_empty() {
            return Err(bad_value("the endpoint's address is not set"));
        }
        let text = checked_text(&request, Some(settings.max_chars))?;
        let models = settings.model_list();
        let model_ids: Vec<String> = models.iter().map(|choice| choice.id.clone()).collect();
        let model = request
            .model
            .clone()
            .filter(|model| !model.is_empty())
            .or_else(|| settings.model.clone().filter(|model| !model.is_empty()))
            .or_else(|| model_ids.first().cloned());
        if let Some(model) = &model
            && !model_ids.is_empty()
            && !model_ids.contains(model)
        {
            return Err(unknown_choice("model", model, &model_ids));
        }
        let voices = settings.voice_list();
        let voice_ids: Vec<String> = voices.iter().map(|voice| voice.id.clone()).collect();
        let voice = request
            .voice
            .clone()
            .filter(|voice| !voice.is_empty())
            .or_else(|| settings.voice.clone().filter(|voice| !voice.is_empty()));
        if let Some(voice) = &voice
            && !voice_ids.is_empty()
            && !voice_ids.contains(voice)
        {
            return Err(unknown_choice("voice", voice, &voice_ids));
        }
        let styles = settings.style_list();
        let instructions = match request.style.as_deref().filter(|style| !style.is_empty()) {
            Some(style) => {
                let (_, text) = styles
                    .iter()
                    .find(|(name, _)| name == style)
                    .ok_or_else(|| {
                        unknown_choice(
                            "style",
                            style,
                            &styles
                                .iter()
                                .map(|(name, _)| name.clone())
                                .collect::<Vec<_>>(),
                        )
                    })?;
                Some(text.clone())
            }
            None => None,
        };
        let mut body = serde_json::Map::new();
        body.insert("input".to_owned(), text.into());
        body.insert("response_format".to_owned(), "pcm".into());
        if let Some(model) = &model {
            body.insert("model".to_owned(), model.as_str().into());
        }
        if let Some(voice) = &voice {
            body.insert("voice".to_owned(), voice.as_str().into());
        }
        if let Some(speed) = request.speed {
            body.insert("speed".to_owned(), SPEED.clamp(speed).into());
        }
        if let Some(instructions) = instructions {
            body.insert("instructions".to_owned(), instructions.into());
        }
        let wait = Duration::from_secs(u64::from(settings.timeout_seconds.max(1)));
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(None)
            .timeout_connect(Some(wait))
            .timeout_recv_response(Some(wait))
            .timeout_recv_body(Some(Duration::from_secs(BODY_SECONDS)))
            .build();
        let agent = crate::network::Network::current()
            .agent(config)
            .map_err(|error| {
                internal(alloc::format!("the endpoint could not be reached: {error}"))
            })?;
        let mut call = agent
            .post(&settings.url)
            .header("Content-Type", "application/json");
        if let Some(key) = &settings.api_key {
            call = call.header("Authorization", alloc::format!("Bearer {}", key.expose()));
        }
        let mut answer = call
            .send(serde_json::Value::Object(body).to_string())
            .map_err(|error| {
                internal(alloc::format!("the endpoint could not be reached: {error}"))
            })?;
        let status = answer.status();
        if !status.is_success() {
            let text = answer
                .body_mut()
                .with_config()
                .limit(64 * 1024)
                .read_to_string()
                .unwrap_or_default();
            let shown: String = text.trim().chars().take(MAX_ERROR_BODY).collect();
            return Err(internal(alloc::format!(
                "the endpoint answered {}: {shown}",
                status.as_u16()
            )));
        }
        Ok(Spoken::from_reader(
            answer.into_body().into_reader(),
            AudioFormat::pcm(settings.rate.max(1)),
            Head::Raw,
            wait,
        )
        .into_object())
    }
}

impl core::fmt::Debug for EndpointSay {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("EndpointSay")
            .field("url", &self.settings.url)
            .finish_non_exhaustive()
    }
}
