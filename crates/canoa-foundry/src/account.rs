//! Foundry Local as an account: what a harness is given so that it talks to the gateway, and what the account's person chooses.
//!
//! The gateway answers any model name with the one model the account serves, so a harness needs only an address and a bearer.
//! Each harness takes those through an account kind of its own, which its description says (`agent_vendor`'s `gateway_route`), so the
//! account is one the host holds and maps onto that kind for the session.

use alloc::string::{String, ToString as _};
use alloc::vec::Vec;

use serde_json::{Map, Value, json};

/// The id of the account kind, and of the sign-in method that makes it.
pub const KIND: &str = "foundry-local";

/// The settings of the account: the model it serves, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Settings {
    /// The model variant to serve, by the id Foundry Local lists it under.
    pub model: String,
    /// The Foundry Local service, `http://127.0.0.1:PORT`; the running one when absent.
    pub endpoint: Option<String>,
    /// The native library's file or folder, for the SDK engine.
    pub library: Option<String>,
    /// What runs the model.
    pub engine: EngineKind,
    /// The window, in tokens, the person set for this model: not a key of the account's table, which has no such key, but what the
    /// host finds for [`Settings::model`] in its own configuration before it opens an engine. Only the SDK engine uses it.
    pub window: Option<u64>,
}

/// What runs the model of an account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum EngineKind {
    /// The Foundry Local service the person runs, which holds the model.
    Service,
    /// Foundry Local's SDK, in the host's own process, which then holds the model.
    Sdk,
}

impl Settings {
    /// Reads the settings of an account from its table.
    ///
    /// # Errors
    ///
    /// A missing model, or a value that is not text.
    pub fn from_map(map: &Map<String, Value>) -> Result<Self, String> {
        let text = |key: &str| -> Result<Option<String>, String> {
            match map.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(text)) if !text.is_empty() => Ok(Some(text.clone())),
                Some(_) => Err(alloc::format!("{key} is text")),
            }
        };
        if let Some(extra) = map
            .keys()
            .find(|key| !["model", "endpoint", "library", "engine"].contains(&key.as_str()))
        {
            return Err(alloc::format!("{extra} is not a setting of this kind"));
        }
        Ok(Self {
            model: text("model")?.ok_or_else(|| "model names the model to serve".to_string())?,
            endpoint: text("endpoint")?,
            library: text("library")?,
            engine: match text("engine")?.as_deref() {
                None | Some("service") => EngineKind::Service,
                Some("sdk") => EngineKind::Sdk,
                Some(_) => return Err("engine is service or sdk".to_string()),
            },
            window: None,
        })
    }
}

/// The JSON schema of the account's settings, and of the sign-in inputs that make them: the same three fields.
#[must_use]
pub fn settings_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "model": {
                "type": "string",
                "title": "Model",
                "description": "The model variant to serve, as `foundry model list` shows it (`qwen3-14b-generic-gpu:2`). Load it in Foundry Local first: this never loads one.",
            },
            "endpoint": {
                "type": "string",
                "format": "uri",
                "title": "Service address",
                "description": "The Foundry Local service, `http://127.0.0.1:PORT`. The running service is used when this is empty.",
            },
            "engine": {
                "type": "string",
                "enum": ["service", "sdk"],
                "title": "Engine",
                "description": "`service` (the default) uses the Foundry Local service you run, which holds the model. `sdk` runs the model inside this program through Foundry Local's SDK, which needs the native library below and loads the model into this program's memory.",
            },
            "library": {
                "type": "string",
                "title": "Native library",
                "description": "For the `sdk` engine: the folder or file of Foundry Local's native library, a copy made outside the installed package, which a program cannot load from.",
            },
        },
        "required": ["model"],
        "additionalProperties": false,
    })
}

/// What each setting's absence or presence means, for a line a person reads when the account is added.
#[must_use]
pub fn notices() -> Vec<String> {
    alloc::vec![
        "the model runs in Foundry Local on this computer: load it there first, and unload it when you are done, since it holds memory other work needs".to_string(),
        "the agent is told any model name is this model; what the model cannot do (tools, images) is refused with the reason".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(value: Value) -> Map<String, Value> {
        value.as_object().cloned().expect("a table")
    }

    #[test]
    fn settings_need_a_model_and_take_nothing_else() {
        let ok = Settings::from_map(&table(
            json!({"model": "m", "endpoint": "http://127.0.0.1:1"}),
        ))
        .expect("reads");
        assert_eq!(ok.model, "m");
        assert_eq!(ok.endpoint.as_deref(), Some("http://127.0.0.1:1"));
        assert!(Settings::from_map(&table(json!({}))).is_err());
        assert!(Settings::from_map(&table(json!({"model": 3}))).is_err());
        assert_eq!(ok.engine, EngineKind::Service);
        let sdk =
            Settings::from_map(&table(json!({"model": "m", "engine": "sdk"}))).expect("reads");
        assert_eq!(sdk.engine, EngineKind::Sdk);
        assert!(Settings::from_map(&table(json!({"model": "m", "engine": "cloud"}))).is_err());
        assert!(Settings::from_map(&table(json!({"model": "m", "key": "x"}))).is_err());
    }

    #[test]
    fn the_schema_names_the_same_fields_the_settings_read() {
        let schema = settings_schema();
        let names: Vec<&str> = schema["properties"]
            .as_object()
            .expect("properties")
            .keys()
            .map(String::as_str)
            .collect();
        for name in ["model", "endpoint", "library", "engine"] {
            assert!(names.contains(&name), "{name}");
        }
        assert_eq!(schema["required"], json!(["model"]));
    }
}
