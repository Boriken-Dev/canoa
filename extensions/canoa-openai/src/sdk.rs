//! OpenAI's `sdk_provider`: chat and embeddings for the accounts that hold an OpenAI API key, and what each model can do.
//!
//! OpenAI's `GET /v1/models` lists an id, a date and an owner and nothing about what a model does, so this crate declares it: a rule
//! by model id says the capabilities and limits. The listing says which models the key may use; the rule says what each can do. An
//! id the rule does not match reports no capabilities, which a consumer lists apart and still accepts typed.

use canoa::sdk::{
    Capability, Dialect, LentHttp, OpenAiShaped, OpenRequest, Sdk, SdkInfo, SdkProvider, capability,
};
use coqui_acp_api::guatiao::Provider;
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::{CapabilitiesSource, ModelInfo, ModelLimits, standard};

use crate::{ADDRESS, CREDENTIAL, SERVICE, STYLES};

/// Whether a model takes no temperature other than its default: the reasoning families.
fn fixed_temperature(model: &str) -> bool {
    model.starts_with("gpt-5")
        || model.starts_with("o1")
        || model.starts_with("o3")
        || model.starts_with("o4")
}

/// What a model of this id can do, from OpenAI's documentation. A change in OpenAI's catalog is a release of this crate.
#[must_use]
pub fn rule(model: ModelInfo) -> ModelInfo {
    let id = model.id.clone();
    let declared = CapabilitiesSource::Declared;
    if id.starts_with("text-embedding-") {
        let third = id.starts_with("text-embedding-3-");
        let dimensions = if id == "text-embedding-3-large" {
            3072
        } else {
            1536
        };
        return model
            .with_capabilities([capability::EMBED], declared)
            .with_limits(ModelLimits::new().embedding(dimensions, third, Some(8191)));
    }
    if id.contains("transcribe") || id.starts_with("whisper-") {
        return model.with_capabilities([capability::SPEECH_TO_TEXT, "audio_input"], declared);
    }
    if id.starts_with("tts-") || id.contains("-tts") {
        // Which voices a model takes is prose in OpenAI's documentation, so none is declared: the service answers a pair it refuses.
        // The same prose says `instructions` do not work with the `tts-` models.
        let styled = !id.starts_with("tts-");
        let styles: Vec<&str> = if styled {
            STYLES
                .iter()
                .filter_map(|style| style.split('=').next())
                .collect()
        } else {
            Vec::new()
        };
        return model
            .with_capabilities([capability::TEXT_TO_SPEECH, "audio_output"], declared)
            .with_limits(ModelLimits::new().speaking(Vec::<&str>::new(), styles));
    }
    if id.contains("realtime") || id.contains("-audio-") {
        return model.with_capabilities(["audio_input", "audio_output"], declared);
    }
    if id.starts_with("gpt-") || id.starts_with("chatgpt-") || is_o_series(&id) {
        let mut can = vec![capability::CHAT, "tool_use", "structured_output"];
        if fixed_temperature(&id) {
            can.push("reasoning");
        }
        if id.starts_with("gpt-4o") || id.starts_with("gpt-4.1") || id.starts_with("gpt-5") {
            can.push("image_input");
        }
        return model.with_capabilities(can, declared);
    }
    model
}

fn is_o_series(id: &str) -> bool {
    let mut chars = id.chars();
    chars.next() == Some('o') && chars.next().is_some_and(|digit| digit.is_ascii_digit())
}

/// OpenAI's chat and embeddings, for the accounts that hold an API key.
#[derive(Default, Debug, Clone, Copy, Provider)]
#[provider(SdkProvider, id = "openai_sdk", crate = ::coqui_acp_api::guatiao)]
pub struct OpenAiSdk;

impl SdkProvider for OpenAiSdk {
    fn describe(&self) -> Result<SdkInfo, ProviderError> {
        let mut info = SdkInfo::new(SERVICE, "OpenAI");
        info.summary = "Chat and embeddings by OpenAI's API.".to_owned();
        info.location = "The text goes over the internet to the address of the account (api.openai.com for an OpenAI key) with your key, and the service's terms say what it does with it.".to_owned();
        info.standards = vec![standard::OPENAI_API.to_owned()];
        info.credential = CREDENTIAL.to_owned();
        info.capabilities = [capability::CHAT, capability::EMBED]
            .into_iter()
            .map(Capability::new)
            .collect();
        Ok(info)
    }

    fn open(
        &self,
        request: OpenRequest,
        http: Object<dyn LentHttp>,
    ) -> Result<Object<dyn Sdk>, ProviderError> {
        let own = request.address.trim_end_matches('/') == ADDRESS;
        let mut client = OpenAiShaped::new(http, &request.address);
        if own {
            // OpenAI's own service words its token cap and its fixed-temperature models; a gateway that speaks the shape takes the plain one.
            client = client.dialect(Dialect {
                max_tokens_key: "max_completion_tokens",
                fixed_temperature,
            });
        }
        Ok(client.rule(rule).into_object())
    }
}

#[cfg(test)]
mod tests {
    use canoa::sdk::contract::{self, Fixtures};
    use canoa::sdk::testing::{Recorded, Route};
    use canoa::sdk::{ChatRequest, models_for};

    use super::*;

    const MODELS: &str = include_str!("../tests/fixtures/models.json");
    const CHAT: &str = include_str!("../tests/fixtures/chat.json");
    const EMBEDDINGS: &str = include_str!("../tests/fixtures/embeddings.json");

    fn routes() -> Vec<Route> {
        vec![
            Route::new("GET", "/models", 200, MODELS),
            Route::new("POST", "/chat/completions", 200, CHAT),
            Route::new("POST", "/embeddings", 200, EMBEDDINGS),
        ]
    }

    #[test]
    fn the_provider_passes_the_contract_over_recorded_answers() {
        let fixtures = Fixtures {
            address: ADDRESS.to_owned(),
            settings: "{}".to_owned(),
            routes: routes(),
            model: "gpt-5-mini".to_owned(),
        };
        contract::run(&OpenAiSdk, &fixtures).expect("the contract");
    }

    #[test]
    fn the_listing_of_a_key_is_given_the_capabilities_the_rule_declares() {
        let (http, _) = Recorded::new(routes());
        let sdk = OpenAiSdk
            .open(OpenRequest::new(ADDRESS, "{}"), http.into_object())
            .expect("opens");
        let models = sdk.models().expect("listed").models;
        let embed = models_for(&models, "embed");
        assert_eq!(
            embed
                .offered
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            ["text-embedding-3-small", "text-embedding-3-large"]
        );
        let small = &embed.offered[0];
        let limits = small.limits.as_ref().expect("limits");
        assert_eq!(limits.dimensions, Some(1536));
        assert!(limits.dimensions_adjustable);
        let chat = models_for(&models, "chat");
        assert!(chat.offered.iter().any(|m| m.id == "gpt-5-mini"));
        assert!(
            !chat
                .offered
                .iter()
                .any(|m| m.id.contains("embedding") || m.id.contains("tts"))
        );
        assert_eq!(
            chat.unreported
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            ["some-new-model"],
            "an id the rule does not know is kept apart, and still accepted when typed"
        );
        let stt = models_for(&models, "speech_to_text");
        assert_eq!(stt.offered.len(), 2);
        let tts = models_for(&models, "text_to_speech");
        let styled = tts
            .offered
            .iter()
            .find(|m| m.id == "gpt-4o-mini-tts")
            .expect("styled");
        let plain = tts.offered.iter().find(|m| m.id == "tts-1").expect("plain");
        assert_eq!(styled.limits.as_ref().expect("limits").styles.len(), 4);
        let dated = tts
            .offered
            .iter()
            .find(|m| m.id == "gpt-4o-mini-tts-2025-12-15")
            .expect("a dated snapshot is a speech model too");
        assert_eq!(dated.limits.as_ref().expect("limits").styles.len(), 4);
        assert!(plain.limits.as_ref().expect("limits").styles.is_empty());
        assert!(
            styled.limits.as_ref().expect("limits").voices.is_empty()
                && plain.limits.as_ref().expect("limits").voices.is_empty(),
            "no voice is declared per model"
        );
    }

    #[test]
    fn openais_own_service_is_asked_with_the_newer_token_cap_and_no_temperature_for_a_reasoning_model()
     {
        let (http, asked) = Recorded::new(routes());
        let sdk = OpenAiSdk
            .open(OpenRequest::new(ADDRESS, "{}"), http.into_object())
            .expect("opens");
        let mut request = ChatRequest::ask("gpt-5-mini", "hi");
        request.max_tokens = Some(96);
        request.temperature = Some(0.0);
        sdk.chat(request).expect("answered");
        let body = asked.requests()[0].body.clone().expect("a body");
        assert!(body.contains("\"max_completion_tokens\":96"), "{body}");
        assert!(!body.contains("temperature"), "{body}");

        let (http, asked) =
            Recorded::new(vec![Route::new("POST", "/v1/chat/completions", 200, CHAT)]);
        let sdk = OpenAiSdk
            .open(
                OpenRequest::new("http://127.0.0.1:4000", "{}"),
                http.into_object(),
            )
            .expect("opens");
        let mut request = ChatRequest::ask("local", "hi");
        request.max_tokens = Some(96);
        request.temperature = Some(0.0);
        sdk.chat(request).expect("answered");
        let body = asked.requests()[0].body.clone().expect("a body");
        assert!(
            body.contains("\"max_tokens\":96") && body.contains("temperature"),
            "{body}"
        );
    }

    #[test]
    fn a_refused_key_is_an_error_that_says_to_sign_in_again() {
        let (http, _) = Recorded::new(vec![Route::new("GET", "/models", 401, "{\"error\":{}}")]);
        let sdk = OpenAiSdk
            .open(OpenRequest::new(ADDRESS, "{}"), http.into_object())
            .expect("opens");
        let error = sdk.models().expect_err("refused");
        assert!(
            format!("{error}").contains(canoa::sdk::SIGNED_OUT_PREFIX),
            "{error}"
        );
    }
}
