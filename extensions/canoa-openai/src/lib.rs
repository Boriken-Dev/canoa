//! OpenAI for Canoa: the speech services the accounts that hold an OpenAI API key can call: speech-to-text, and its text-to-speech
//! voices ([`OpenAiVoices`]).
//!
//! [`library`] is what a Canoa host loads: an `account_speech` provider that says which accounts those are (a kind that provides the
//! OpenAI API at OpenAI's own address), where a recording is posted, which credential holds the key and which models transcribe.
//! The host names none of it. The models and their end dates are OpenAI's, which it changes on its own schedule.
#![forbid(unsafe_code)]

mod sdk;
mod voices;

pub use sdk::{OpenAiSdk, rule};

pub use voices::{
    OpenAiVoiceSettings, OpenAiVoices, SPEECH, SPEECH_MODELS, SPEECH_PROVIDER, STYLED_MODEL,
    STYLES, VOICES,
};

use canoa::speech::account::{
    AccountSpeech, AccountSpeechInfo, AccountSpeechModel, AccountSpeechService,
};
use canoa::speech::output::account::{
    AccountSpeechOutput, AccountSpeechOutputInfo, AccountSpeechOutputService,
};
use coqui_acp_api::guatiao::Provider;
use coqui_acp_api::guatiao::library::ProviderError;
use coqui_acp_api::standard;

/// The name this crate's library is registered under.
pub const LIBRARY: &str = "canoa-openai";

/// The id of the service, which an offer made of an account is named after (`openai:NAME`).
pub const SERVICE: &str = "openai";

/// Where the model API of an account's kind is called at when nothing moves it, which is what makes the account OpenAI's.
pub const ADDRESS: &str = "https://api.openai.com/v1";

/// Where a recording is posted.
pub const TRANSCRIPTIONS: &str = "https://api.openai.com/v1/audio/transcriptions";

/// The credential that holds an API key of the OpenAI API standard.
pub const CREDENTIAL: &str = "api_key";

/// The models OpenAI's transcriptions take, the recommended one first. The older three are announced to end on 2027-02-26.
pub const MODELS: [(&str, &str); 4] = [
    ("gpt-transcribe", "GPT Transcribe (recommended)"),
    (
        "gpt-4o-mini-transcribe",
        "GPT-4o mini Transcribe (ends 2027-02-26)",
    ),
    ("gpt-4o-transcribe", "GPT-4o Transcribe (ends 2027-02-26)"),
    ("whisper-1", "Whisper (ends 2027-02-26)"),
];

/// OpenAI's speech-to-text, for the accounts that hold an API key.
#[derive(Default, Debug, Clone, Copy, Provider)]
#[provider(AccountSpeech, id = "openai_speech", crate = ::coqui_acp_api::guatiao)]
pub struct OpenAiSpeech;

impl AccountSpeech for OpenAiSpeech {
    fn describe(&self) -> Result<AccountSpeechInfo, ProviderError> {
        let mut service = AccountSpeechService::new(
            SERVICE,
            "OpenAI",
            standard::OPENAI_API,
            ADDRESS,
            TRANSCRIPTIONS,
            CREDENTIAL,
        );
        service.summary = "Speech to text by OpenAI's service.".to_owned();
        service.location = "The audio goes over the internet to api.openai.com, and OpenAI's terms say what it does with it.".to_owned();
        service.models = MODELS
            .iter()
            .map(|(id, label)| AccountSpeechModel::new(*id, *label))
            .collect();
        Ok(AccountSpeechInfo::new(vec![service]))
    }
}

/// OpenAI's text-to-speech description, for the accounts that hold an API key.
#[derive(Default, Debug, Clone, Copy, Provider)]
#[provider(AccountSpeechOutput, id = "openai_speech_output", crate = ::coqui_acp_api::guatiao)]
pub struct OpenAiSpeechOutput;

impl AccountSpeechOutput for OpenAiSpeechOutput {
    fn describe(&self) -> Result<AccountSpeechOutputInfo, ProviderError> {
        let mut service = AccountSpeechOutputService::new(
            SERVICE,
            "OpenAI",
            standard::OPENAI_API,
            ADDRESS,
            SPEECH,
            CREDENTIAL,
            SPEECH_PROVIDER,
        );
        service.summary = "Text to speech by OpenAI's voices.".to_owned();
        service.location = "The text goes over the internet to api.openai.com with your key, and OpenAI's terms say what it does with it.".to_owned();
        Ok(AccountSpeechOutputInfo::new(vec![service]))
    }
}

// What this crate offers as a library the host links: handed to `Registry::register_local`, the same way a loaded library is read.
coqui_acp_api::guatiao::local_providers!(
    id = "canoa-openai",
    version = env!("CARGO_PKG_VERSION"),
    providers = [OpenAiSpeech, OpenAiVoices, OpenAiSpeechOutput, OpenAiSdk]
);

#[cfg(test)]
mod tests {
    use coqui_acp_api::guatiao::library::Registry;

    use super::*;

    #[test]
    fn the_description_is_one_service_a_host_can_offer_with_the_recommended_model_first() {
        let info = OpenAiSpeech.describe().expect("described");
        assert_eq!(info.problem(), None);
        let [service] = info.services.as_slice() else {
            panic!("one service: {info:?}");
        };
        assert_eq!(service.id, SERVICE);
        assert_eq!(service.standard, "openai-api");
        assert_eq!(service.address, ADDRESS);
        assert_eq!(service.url, TRANSCRIPTIONS);
        assert_eq!(service.credential, "api_key");
        assert_eq!(service.models[0].id, "gpt-transcribe");
        assert!(service.models.iter().any(|model| model.id == "whisper-1"));
    }

    #[test]
    fn the_library_offers_the_provider() {
        let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
        let loading = registry
            .register_local(LIBRARY, library)
            .expect("registers");
        assert!(loading.loaded().is_some(), "accepted");
        assert!(
            registry
                .offer::<dyn AccountSpeech>("openai_speech")
                .is_some()
        );
        assert!(
            registry
                .offer::<dyn canoa::speech::output::SpeechOutput>(SPEECH_PROVIDER)
                .is_some()
        );
        assert!(
            registry
                .offer::<dyn AccountSpeechOutput>("openai_speech_output")
                .is_some()
        );
        let output = OpenAiSpeechOutput.describe().expect("described");
        assert_eq!(output.problem(), None);
        assert_eq!(output.services[0].id, SERVICE);
        assert_eq!(output.services[0].provider, SPEECH_PROVIDER);
    }
}
