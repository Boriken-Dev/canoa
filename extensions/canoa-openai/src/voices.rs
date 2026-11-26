//! OpenAI's text-to-speech: the `speech_output` provider that posts text to OpenAI's `audio/speech` call and plays what comes back.
//!
//! The call is the generic endpoint provider's. What is OpenAI's is the address and what its service lists now: the models the
//! account's key can call (the host lists them from `GET /v1/models` and hands them in `models`) and the voices and end dates its
//! API description gives, which `voices` reads through the host's fetch. Nothing here names a model or a voice, and none is
//! refused: the call takes any voice string and any model the key can use, and OpenAI's own error says when a pair does not work.

use canoa::speech::builtin::{EndpointSay, EndpointSaySettings};
use canoa::speech::output::{
    Choice, OutputInfo, SayRequest, SpeechOutput, SpeechUtterance, Voice, VoiceList, VoiceQuery,
};
use canoa::store::source::SourceFetch;
use coqui_acp_api::ProcessSpawner;
use guatiao::library::{Object, ProviderError};
use guatiao::{FromValue, Schema, Secret, ToValue};
use guatiao_intake::Form;

use crate::description::{self, Speech};

/// The id of the provider, which a table of the host's voice providers names.
pub const SPEECH_PROVIDER: &str = "speech_say_openai";

/// Where the text is posted.
pub const SPEECH: &str = "https://api.openai.com/v1/audio/speech";

/// The styles offered, as `name=instruction`: Canoa's own wording of what `instructions` asks of a model that takes them.
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
    /// The model used when a request names none; absent is the first of `models`.
    #[schema(section = "openai", title = "Model")]
    pub model: Option<String>,
    /// The voice used when a request names none.
    #[schema(section = "openai", title = "Voice")]
    pub voice: Option<String>,
    /// The models the key can call for speech, as OpenAI lists them. The host writes them for an account's offer; none leaves the
    /// choice to be typed. A model not in the list is still sent.
    #[map(default)]
    #[schema(advanced, section = "openai", title = "Models offered")]
    pub models: Vec<String>,
    /// How long the server may take to start answering, and then to go without a sound, in seconds.
    #[map(default = default_timeout())]
    #[schema(advanced, section = "openai", title = "Time to answer")]
    #[form(unit = "s")]
    pub timeout_seconds: u32,
}

impl OpenAiVoiceSettings {
    /// The endpoint provider's settings that call OpenAI with these. It is given no list of models or voices, so it refuses none.
    fn endpoint(&self) -> EndpointSaySettings {
        self.endpoint_at(SPEECH)
    }

    /// The same, posting to `url`.
    fn endpoint_at(&self, url: &str) -> EndpointSaySettings {
        let mut settings = EndpointSaySettings::new(url);
        settings.api_key.clone_from(&self.api_key);
        settings.model.clone_from(&self.model);
        settings.voice.clone_from(&self.voice);
        settings.styles = STYLES.iter().map(|style| (*style).to_owned()).collect();
        settings.timeout_seconds = self.timeout_seconds;
        settings
    }

    /// The model a request that names none uses: the setting, else the first the key lists.
    fn default_model(&self) -> Option<&str> {
        self.model
            .as_deref()
            .filter(|model| !model.is_empty())
            .or_else(|| self.models.iter().map(String::as_str).next())
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

impl OpenAiVoices {
    /// The models the key lists, each labelled with the date OpenAI says it ends, when it does.
    fn models(&self, speech: &Speech) -> Vec<Choice> {
        self.settings
            .models
            .iter()
            .map(|id| {
                let label = speech
                    .shutdowns
                    .get(id)
                    .map_or_else(|| id.clone(), |date| format!("{id} (ends {date})"));
                Choice::new(id.clone(), label)
            })
            .collect()
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
        info.models = self
            .settings
            .models
            .iter()
            .map(|id| Choice::new(id.clone(), id.clone()))
            .collect();
        info.default_model = self.settings.default_model().map(str::to_owned);
        info.endpoints = vec![description::REPOSITORY.to_owned()];
        Ok(info)
    }

    fn voices(
        &self,
        query: VoiceQuery,
        spawner: Object<dyn ProcessSpawner>,
        fetch: Object<dyn SourceFetch>,
    ) -> Result<VoiceList, ProviderError> {
        drop(spawner);
        let document = fetch.get_json(description::DESCRIPTION, description::MOST)?;
        let speech = description::read(&document).map_err(canoa::status::bad_value)?;
        let voices = speech
            .voices
            .iter()
            .map(|name| Voice::new(name.clone(), name.clone()))
            .collect();
        Ok(VoiceList::new(voices)
            .with_models(self.models(&speech))
            .speaking(query.language.as_deref()))
    }

    fn say(
        &self,
        mut request: SayRequest,
        spawner: Object<dyn ProcessSpawner>,
    ) -> Result<Object<dyn SpeechUtterance>, ProviderError> {
        if request.model.as_deref().is_none_or(str::is_empty)
            && let Some(model) = self.settings.default_model()
        {
            request.model = Some(model.to_owned());
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
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "the object an utterance opens is not Debug, which expect_err needs; a test names what it uses from std"
)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    use guatiao::Status;

    use super::*;
    use crate::description::tests::RECORDED;

    fn settings(models: &[&str]) -> OpenAiVoiceSettings {
        OpenAiVoiceSettings {
            api_key: None,
            model: None,
            voice: None,
            models: models.iter().map(|model| (*model).to_owned()).collect(),
            timeout_seconds: default_timeout(),
        }
    }

    fn spawner() -> Object<dyn ProcessSpawner> {
        coqui_acp_harness::LocalSpawner::new().into_object()
    }

    /// The host's fetch with OpenAI's description recorded, or with nothing reachable; it notes what it was asked.
    struct Recorded {
        answer: Result<&'static str, &'static str>,
        asked: Arc<Mutex<Vec<String>>>,
    }

    impl SourceFetch for Recorded {
        fn get_text(&self, url: &str, _max: u64) -> Result<String, ProviderError> {
            self.asked.lock().expect("lock").push(url.to_owned());
            match self.answer {
                Ok(text) if url == description::DESCRIPTION => Ok(text.to_owned()),
                Ok(_) => Err(ProviderError::new(
                    Status::GUATIAO_ERR_UNSUPPORTED,
                    format!("no such address: {url}").as_str(),
                )),
                Err(why) => Err(ProviderError::new(Status::GUATIAO_ERR_INTERNAL, why)),
            }
        }

        fn get_json(&self, url: &str, max: u64) -> Result<String, ProviderError> {
            self.get_text(url, max)
        }
    }

    fn recorded(
        answer: Result<&'static str, &'static str>,
    ) -> (Object<dyn SourceFetch>, Arc<Mutex<Vec<String>>>) {
        let asked = Arc::new(Mutex::new(Vec::new()));
        let fetch = Recorded {
            answer,
            asked: Arc::clone(&asked),
        };
        (fetch.into_object(), asked)
    }

    #[test]
    fn the_voices_and_the_end_dates_come_from_the_description_read_through_the_hosts_fetch() {
        let provider = OpenAiVoices::from(settings(&[
            "tts-1",
            "gpt-4o-mini-tts-2025-12-15",
            "new-tts",
        ]));
        let info = provider.describe().expect("described");
        assert!(
            info.endpoints
                .iter()
                .any(|place| description::DESCRIPTION.starts_with(place.as_str())),
            "the address the provider reads is one it declares: {:?}",
            info.endpoints
        );
        let (fetch, asked) = recorded(Ok(RECORDED));
        let list = provider
            .voices(VoiceQuery::default(), spawner(), fetch)
            .expect("listed");
        assert_eq!(list.voices.len(), 13);
        assert!(list.voices.iter().any(|voice| voice.id == "marin"));
        assert!(
            list.voices.iter().all(|voice| voice.models.is_empty()),
            "no voice is tied to a model"
        );
        let labels: Vec<_> = list
            .models
            .iter()
            .map(|model| (model.id.as_str(), model.label.as_str()))
            .collect();
        assert_eq!(
            labels,
            [
                ("tts-1", "tts-1 (ends 2027-01-06)"),
                (
                    "gpt-4o-mini-tts-2025-12-15",
                    "gpt-4o-mini-tts-2025-12-15 (ends 2027-01-06)"
                ),
                ("new-tts", "new-tts"),
            ]
        );
        assert_eq!(*asked.lock().expect("lock"), [description::DESCRIPTION]);
    }

    fn provider_error(settings: OpenAiVoiceSettings, fetch: Object<dyn SourceFetch>) -> String {
        OpenAiVoices::from(settings)
            .voices(VoiceQuery::default(), spawner(), fetch)
            .err()
            .expect("refused")
            .to_string()
    }

    #[test]
    fn with_the_description_out_of_reach_the_voice_list_fails_with_the_reason_and_names_no_voice() {
        let (fetch, _) = recorded(Err("raw.githubusercontent.com could not be reached"));
        let failed = provider_error(settings(&[]), fetch);
        assert!(failed.contains("could not be reached"), "{failed}");
        let (fetch, _) = recorded(Ok("{\"components\":{}}"));
        let failed = provider_error(settings(&[]), fetch);
        assert!(failed.contains("CreateSpeechRequest"), "{failed}");
    }

    /// An endpoint on the loopback that answers one call with a refusal in OpenAI's words, and hands over the body it was sent.
    fn refusing_endpoint() -> (String, std::sync::mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
        let url = format!(
            "http://{}/v1/audio/speech",
            listener.local_addr().expect("address")
        );
        let (sent, received) = std::sync::mpsc::channel();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("a connection");
            let mut seen = Vec::new();
            let mut chunk = [0_u8; 4096];
            loop {
                let read = stream.read(&mut chunk).expect("read");
                seen.extend_from_slice(&chunk[..read]);
                let text = String::from_utf8_lossy(&seen).into_owned();
                if let Some((head, body)) = text.split_once("\r\n\r\n") {
                    let wanted = head
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|n| n.trim().parse::<usize>().expect("a length"))
                        })
                        .unwrap_or(0);
                    if body.len() >= wanted {
                        sent.send(body.to_owned()).expect("sent");
                        break;
                    }
                }
                if read == 0 {
                    break;
                }
            }
            let body = r#"{"error":{"message":"the voice marin is not available with tts-1"}}"#;
            write!(
                stream,
                "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .expect("written");
        });
        (url, received)
    }

    fn at(url: &str, models: &[&str]) -> OpenAiVoices {
        let mut held = settings(models);
        held.timeout_seconds = 5;
        OpenAiVoices {
            call: EndpointSay::from(held.endpoint_at(url)),
            settings: held,
        }
    }

    #[test]
    fn a_voice_typed_by_name_and_a_pair_openai_may_refuse_are_sent_and_the_services_own_error_is_shown()
     {
        let (url, received) = refusing_endpoint();
        let provider = at(&url, &[]);
        let mut request = SayRequest::new("Hello.");
        request.voice = Some("marin".to_owned());
        request.model = Some("tts-1".to_owned());
        let refused = provider
            .say(request, spawner())
            .err()
            .expect("the service refuses");
        let sent = received
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("a call");
        let sent: serde_json::Value = serde_json::from_str(&sent).expect("JSON");
        assert_eq!(sent["voice"], "marin");
        assert_eq!(sent["model"], "tts-1");
        let why = refused.to_string();
        assert!(why.contains("not available with tts-1"), "{why}");
        assert!(
            !why.contains("works with"),
            "no local restriction answered: {why}"
        );
    }

    #[test]
    fn a_request_naming_no_model_uses_the_first_the_key_lists() {
        let (url, received) = refusing_endpoint();
        let provider = at(&url, &["first-tts", "second-tts"]);
        let _ = provider.say(SayRequest::new("Hello."), spawner());
        let sent = received
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("a call");
        let sent: serde_json::Value = serde_json::from_str(&sent).expect("JSON");
        assert_eq!(sent["model"], "first-tts");
    }

    /// The real description at OpenAI's repository, through `curl`.
    struct Live;

    impl SourceFetch for Live {
        fn get_text(&self, url: &str, _max: u64) -> Result<String, ProviderError> {
            let out = std::process::Command::new("curl")
                .args(["-sfL", "--retry", "2", url])
                .output()
                .expect("curl runs");
            assert!(out.status.success(), "{url} could not be fetched");
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        }

        fn get_json(&self, url: &str, max: u64) -> Result<String, ProviderError> {
            self.get_text(url, max)
        }
    }

    #[test]
    #[ignore = "reaches raw.githubusercontent.com"]
    fn the_real_description_lists_voices_and_the_models_it_ends() {
        let provider = OpenAiVoices::from(settings(&["tts-1"]));
        let list = provider
            .voices(VoiceQuery::default(), spawner(), Live.into_object())
            .expect("listed");
        assert!(list.voices.len() >= 10, "{:?}", list.voices);
        assert!(list.voices.iter().any(|voice| voice.id == "alloy"));
        assert_eq!(list.models.len(), 1);
    }
}
