//! OpenAI's text-to-speech: the `speech_output` provider that posts text to OpenAI's `audio/speech` call and plays what comes back.
//!
//! The call is the generic endpoint provider's; what is OpenAI's is the address, the models, the voices (the newer ones work with
//! one model alone) and the styles it documents, which change on OpenAI's own schedule.

use canoa::speech::builtin::{EndpointSay, EndpointSaySettings};
use canoa::speech::output::{
    OutputInfo, SayRequest, SpeechOutput, SpeechUtterance, Voice, VoiceList, VoiceQuery,
};
use canoa::status::bad_value;
use coqui_acp_api::ProcessSpawner;
use guatiao::library::{Object, ProviderError};
use guatiao::{FromValue, Schema, Secret, ToValue};
use guatiao_intake::Form;

/// The id of the provider, which a table of the host's voice providers names.
pub const SPEECH_PROVIDER: &str = "speech_say_openai";

/// Where the text is posted.
pub const SPEECH: &str = "https://api.openai.com/v1/audio/speech";

/// The models OpenAI documents for the call, newest first.
pub const SPEECH_MODELS: [(&str, &str); 3] = [
    ("gpt-4o-mini-tts", "GPT-4o mini TTS"),
    ("tts-1", "TTS-1"),
    ("tts-1-hd", "TTS-1 HD"),
];

/// The voices OpenAI documents. The last four work with [`STYLED_MODEL`] alone.
pub const VOICES: [&str; 13] = [
    "alloy", "ash", "coral", "echo", "fable", "nova", "onyx", "sage", "shimmer", "ballad", "verse",
    "marin", "cedar",
];

/// The voices only [`STYLED_MODEL`] has.
const NEWER_VOICES: [&str; 4] = ["ballad", "verse", "marin", "cedar"];

/// The model that takes `instructions`, and that the newer voices need.
pub const STYLED_MODEL: &str = "gpt-4o-mini-tts";

/// The styles offered, as `name=instruction`.
pub const STYLES: [&str; 4] = [
    "calm=Speak calmly and slowly, in a soothing tone.",
    "cheerful=Speak in a bright, cheerful tone.",
    "serious=Speak in a serious, measured tone.",
    "whisper=Whisper softly.",
];

const fn default_timeout() -> u32 {
    60
}

/// What the OpenAI voice provider is configured with.
#[derive(ToValue, FromValue, Schema, Form, Debug, Clone)]
#[form(section(id = "openai", label = "OpenAI"))]
#[non_exhaustive]
pub struct OpenAiVoiceSettings {
    /// The API key, sent as `Authorization: Bearer`.
    #[schema(section = "openai", title = "Key")]
    #[form(widget = "password")]
    pub api_key: Option<Secret<String>>,
    /// The model used when a request names none; absent is the first of [`SPEECH_MODELS`].
    #[schema(section = "openai", title = "Model")]
    pub model: Option<String>,
    /// The voice used when a request names none.
    #[schema(section = "openai", title = "Voice")]
    pub voice: Option<String>,
    /// How long the server may take to start answering, and then to go without a sound, in seconds.
    #[map(default = default_timeout())]
    #[schema(advanced, section = "openai", title = "Time to answer")]
    #[form(unit = "s")]
    pub timeout_seconds: u32,
}

impl OpenAiVoiceSettings {
    /// The endpoint provider's settings that call OpenAI with these.
    fn endpoint(&self) -> EndpointSaySettings {
        let mut settings = EndpointSaySettings::new(SPEECH);
        settings.api_key.clone_from(&self.api_key);
        settings.model.clone_from(&self.model);
        settings.voice.clone_from(&self.voice);
        settings.models = SPEECH_MODELS
            .iter()
            .map(|(id, _)| (*id).to_owned())
            .collect();
        settings.voices = VOICES.iter().map(|name| (*name).to_owned()).collect();
        settings.styles = STYLES.iter().map(|style| (*style).to_owned()).collect();
        settings.timeout_seconds = self.timeout_seconds;
        settings
    }
}

/// OpenAI's text-to-speech.
#[derive(guatiao::Provider)]
#[provider(SpeechOutput, id = "speech_say_openai", name = "OpenAI", config = OpenAiVoiceSettings, form = OpenAiVoiceSettings)]
pub struct OpenAiVoices {
    settings: OpenAiVoiceSettings,
    call: EndpointSay,
}

impl From<OpenAiVoiceSettings> for OpenAiVoices {
    fn from(settings: OpenAiVoiceSettings) -> Self {
        let call = EndpointSay::from(settings.endpoint());
        Self { settings, call }
    }
}

impl SpeechOutput for OpenAiVoices {
    fn describe(&self) -> Result<OutputInfo, ProviderError> {
        let mut info = self.call.describe()?;
        info.title = "OpenAI".to_owned();
        info.description = Some("OpenAI's text-to-speech voices".to_owned());
        info.words = Some(
            "The text goes over the internet to api.openai.com with your key, and OpenAI's terms say what it does with it.".to_owned(),
        );
        Ok(info)
    }

    fn voices(
        &self,
        query: VoiceQuery,
        _spawner: Object<dyn ProcessSpawner>,
    ) -> Result<VoiceList, ProviderError> {
        let voices = VOICES
            .iter()
            .map(|name| {
                let mut voice = Voice::new(*name, *name);
                if NEWER_VOICES.contains(name) {
                    voice.models = vec![STYLED_MODEL.to_owned()];
                }
                voice
            })
            .collect();
        Ok(VoiceList::new(voices).speaking(query.language.as_deref()))
    }

    fn say(
        &self,
        request: SayRequest,
        spawner: Object<dyn ProcessSpawner>,
    ) -> Result<Object<dyn SpeechUtterance>, ProviderError> {
        let model = request
            .model
            .as_deref()
            .filter(|model| !model.is_empty())
            .or(self
                .settings
                .model
                .as_deref()
                .filter(|model| !model.is_empty()))
            .unwrap_or(SPEECH_MODELS[0].0);
        let voice = request
            .voice
            .as_deref()
            .filter(|voice| !voice.is_empty())
            .or(self
                .settings
                .voice
                .as_deref()
                .filter(|voice| !voice.is_empty()));
        if let Some(voice) = voice
            && NEWER_VOICES.contains(&voice)
            && model != STYLED_MODEL
        {
            return Err(bad_value(format!(
                "the voice {voice} works with {STYLED_MODEL} only"
            )));
        }
        self.call.say(request, spawner)
    }
}

impl core::fmt::Debug for OpenAiVoices {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OpenAiVoices").finish_non_exhaustive()
    }
}

#[cfg(test)]
#[allow(
    clippy::err_expect,
    reason = "the object an utterance opens is not Debug, which expect_err needs"
)]
mod tests {
    use super::*;

    fn provider() -> OpenAiVoices {
        OpenAiVoices::from(OpenAiVoiceSettings {
            api_key: None,
            model: None,
            voice: None,
            timeout_seconds: default_timeout(),
        })
    }

    #[test]
    fn it_lists_openais_models_voices_and_styles_and_says_where_the_text_goes() {
        let info = provider().describe().expect("described");
        assert_eq!(
            info.models.first().map(|model| model.id.as_str()),
            Some(STYLED_MODEL)
        );
        assert_eq!(info.models.len(), SPEECH_MODELS.len());
        assert_eq!(info.styles.len(), STYLES.len());
        assert_eq!(info.endpoint.as_deref(), Some(SPEECH));
        assert_eq!(info.default_model.as_deref(), Some(STYLED_MODEL));
    }

    fn spawner() -> Object<dyn ProcessSpawner> {
        use coqui_acp_api::ProcessSpawner as _;
        coqui_acp_harness::LocalSpawner::new().into_object()
    }

    #[test]
    fn a_newer_voice_is_refused_with_a_model_that_lacks_it_before_anything_is_sent() {
        let mut request = SayRequest::new("Hello.");
        request.voice = Some("marin".to_owned());
        request.model = Some("tts-1".to_owned());
        let refused = provider().say(request, spawner()).err().expect("refused");
        assert!(refused.to_string().contains(STYLED_MODEL), "{refused}");
    }

    #[test]
    fn the_newer_voices_name_the_one_model_they_work_with() {
        let list = provider()
            .voices(VoiceQuery::default(), spawner())
            .expect("voices");
        let marin = list
            .voices
            .iter()
            .find(|voice| voice.id == "marin")
            .expect("marin");
        assert_eq!(marin.models, [STYLED_MODEL]);
        let alloy = list
            .voices
            .iter()
            .find(|voice| voice.id == "alloy")
            .expect("alloy");
        assert!(alloy.models.is_empty());
    }
}
