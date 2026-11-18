//! `looks/list`: the looks the loaded agents carry, as the token documents the app draws its families from.

use serde_json::{Value, json};

use crate::config::Config;

/// `{"looks": [document, ...]}`: each document has `id`, `label`, `vendor` (the agent it belongs to) and, per brightness (`light`,
/// `dark`), the colour tokens, `radius`, `controlRadius`, `railWidth` and a `chat` table. Nothing in them is secret. Only the agents
/// that are loaded have one.
pub(super) fn list(config: &Config) -> Value {
    let registries = super::providers::loaded(config);
    let loaded: Vec<(&str, &coqui_acp_api::guatiao::library::Registry)> = registries
        .iter()
        .map(|(origin, registry)| (origin.as_str(), registry))
        .collect();
    json!({ "looks": super::providers::looks_of(&loaded) })
}
