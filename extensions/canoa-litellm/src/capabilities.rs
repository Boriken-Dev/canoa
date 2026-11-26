//! What a LiteLLM proxy's `/model/info` says of each model it lists: what the model does (its `mode`) and what it takes and gives
//! (`supports_*` flags, token limits), as the capabilities and limits of the model records a picker filters by.
//!
//! The proxy's `/v1/models` lists ids only, so without this a chat picker offers an embedding model. A proxy that does not answer
//! `/model/info` (an old one, or a key it does not serve) leaves its models as they were listed, which report no capabilities: they
//! are shown apart as such and never taken for chat models.

use canoa::api_gateway::http;
use coqui_acp_api::{CapabilitiesSource, ModelInfo, ModelLimits, ModelListing};
use serde_json::Value;

/// Where a proxy at `base` answers for its models' details.
#[must_use]
pub fn model_info_url(base: &str) -> String {
    format!("{}/model/info", base.trim_end_matches('/'))
}

/// The capability a proxy's `mode` is. A mode that is no operation Canoa asks of a model (`image_generation`, `moderation`) is its
/// own id, so a model of it reports capabilities, none of which is chat, and a chat picker leaves it out.
fn capability_of_mode(mode: &str) -> String {
    match mode {
        "chat" | "completion" | "responses" => "chat".to_owned(),
        "embedding" => "embed".to_owned(),
        "rerank" => "rerank".to_owned(),
        "audio_transcription" => "speech_to_text".to_owned(),
        "audio_speech" => "text_to_speech".to_owned(),
        other => other.to_owned(),
    }
}

/// The capabilities and limits one `model_info` object says. `None` when it says no mode, which is what a deployment the proxy
/// knows nothing of looks like.
fn read(info: &Value) -> Option<(Vec<String>, ModelLimits)> {
    let mode = info.get("mode").and_then(Value::as_str)?;
    let mut capabilities = vec![capability_of_mode(mode)];
    for (flag, capability) in [
        ("supports_vision", "image_input"),
        ("supports_function_calling", "tool_use"),
        ("supports_audio_input", "audio_input"),
        ("supports_audio_output", "audio_output"),
        ("supports_reasoning", "reasoning"),
        ("supports_response_schema", "structured_output"),
    ] {
        if info.get(flag).and_then(Value::as_bool) == Some(true) {
            capabilities.push(capability.to_owned());
        }
    }
    let number = |key: &str| info.get(key).and_then(Value::as_u64);
    let mut limits = ModelLimits::new();
    if let Some(tokens) = number("max_input_tokens") {
        limits.context_tokens = Some(tokens);
        limits.max_input_tokens = Some(tokens);
    }
    limits.max_output_tokens = number("max_output_tokens");
    limits.dimensions = number("output_vector_size");
    Some((capabilities, limits))
}

/// Gives each model of `listing` the capabilities and limits `details` (the body of `/model/info`) says of the model that has its
/// id as `model_name`. A model it does not name, or names with no `mode`, is left as it was. Where a name is deployed more than
/// once the first says.
pub fn apply(listing: &mut ModelListing, details: &Value) {
    let Some(rows) = details.get("data").and_then(Value::as_array) else {
        return;
    };
    for model in &mut listing.models {
        let said = rows
            .iter()
            .filter(|row| row.get("model_name").and_then(Value::as_str) == Some(model.id.as_str()))
            .find_map(|row| read(row.get("model_info")?));
        if let Some((capabilities, limits)) = said {
            let kept = core::mem::replace(model, ModelInfo::new(model.id.clone()));
            *model = kept
                .with_capabilities(capabilities, CapabilitiesSource::Listed)
                .with_limits(limits);
        }
    }
}

/// `listing` with what the proxy at `base` says of its models, asked with `key`. The key goes to the proxy only, as every request
/// of this crate's does; a proxy that does not answer leaves the listing as it is.
#[must_use]
pub fn with_details(mut listing: ModelListing, base: &str, key: &str) -> ModelListing {
    if !listing.known {
        return listing;
    }
    let authorization = format!("Bearer {key}");
    let Ok(reply) = http::get_json_with(
        &model_info_url(base),
        &[("Authorization", authorization.as_str())],
    ) else {
        return listing;
    };
    if (200..300).contains(&reply.status) {
        apply(&mut listing, &reply.body);
    }
    listing
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn listed(ids: &[&str]) -> ModelListing {
        ModelListing::of(ids.iter().map(|id| ModelInfo::new(*id)).collect())
    }

    #[test]
    fn a_models_mode_and_flags_are_its_capabilities_and_its_token_figures_its_limits() {
        let mut listing = listed(&[
            "gpt-x",
            "embed-y",
            "rank-z",
            "draw-w",
            "whisper-v",
            "tts-u",
            "silent",
        ]);
        apply(
            &mut listing,
            &json!({"data": [
                {"model_name": "gpt-x", "model_info": {"mode": "chat", "max_input_tokens": 128000, "max_output_tokens": 16384,
                    "supports_vision": true, "supports_function_calling": true, "supports_reasoning": false,
                    "supports_response_schema": true, "supports_audio_input": true}},
                {"model_name": "embed-y", "model_info": {"mode": "embedding", "max_input_tokens": 8191, "output_vector_size": 1536}},
                {"model_name": "rank-z", "model_info": {"mode": "rerank"}},
                {"model_name": "draw-w", "model_info": {"mode": "image_generation"}},
                {"model_name": "whisper-v", "model_info": {"mode": "audio_transcription"}},
                {"model_name": "tts-u", "model_info": {"mode": "audio_speech"}},
                {"model_name": "silent", "model_info": {"id": "deployment-without-a-mode"}},
            ]}),
        );
        let by = |id: &str| {
            listing
                .models
                .iter()
                .find(|model| model.id == id)
                .expect("listed")
        };
        assert_eq!(
            by("gpt-x").capabilities,
            [
                "chat",
                "image_input",
                "tool_use",
                "audio_input",
                "structured_output"
            ]
        );
        let limits = by("gpt-x").limits.clone().expect("limits");
        assert_eq!(
            (limits.context_tokens, limits.max_output_tokens),
            (Some(128_000), Some(16_384))
        );
        assert_eq!(
            by("gpt-x").capabilities_from,
            Some(CapabilitiesSource::Listed)
        );
        assert_eq!(by("embed-y").capabilities, ["embed"]);
        let limits = by("embed-y").limits.clone().expect("limits");
        assert_eq!(
            (limits.dimensions, limits.max_input_tokens),
            (Some(1536), Some(8191))
        );
        assert_eq!(by("rank-z").capabilities, ["rerank"]);
        assert_eq!(by("whisper-v").capabilities, ["speech_to_text"]);
        assert_eq!(by("tts-u").capabilities, ["text_to_speech"]);
        // A mode Canoa asks nothing of is its own id: reported, and not chat.
        assert_eq!(by("draw-w").capabilities, ["image_generation"]);
        // A deployment that says no mode is not guessed at.
        assert!(by("silent").capabilities.is_empty());
        let ids = |keep: &dyn Fn(&ModelInfo) -> bool| {
            listing
                .models
                .iter()
                .filter(|model| keep(model))
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&|model| model.can("chat")), ["gpt-x"]);
        assert_eq!(ids(&|model| !model.reports_capabilities()), ["silent"]);
    }

    #[test]
    fn what_the_proxy_does_not_say_leaves_the_listing_as_it_was() {
        let mut listing = listed(&["a"]);
        apply(&mut listing, &json!({"error": "not allowed"}));
        apply(
            &mut listing,
            &json!({"data": [{"model_name": "other", "model_info": {"mode": "chat"}}]}),
        );
        assert!(listing.models[0].capabilities.is_empty());
        assert!(listing.models[0].limits.is_none());
    }

    #[test]
    fn the_details_are_asked_at_the_proxys_root() {
        assert_eq!(
            model_info_url("https://proxy.example"),
            "https://proxy.example/model/info"
        );
        assert_eq!(
            model_info_url("https://proxy.example/"),
            "https://proxy.example/model/info"
        );
    }
}
