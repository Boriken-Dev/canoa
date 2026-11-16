//! Foundry Local as an account: what a harness is given so that it talks to the gateway, and what the account's person chooses.
//!
//! The gateway answers any model name with the one model the account serves, so a harness needs only an address and a bearer.
//! Each harness takes those through an account kind of its own (a gateway token for Claude Code, an API key for Codex), so the
//! account is one the host holds and maps onto that kind for the session.

use alloc::string::{String, ToString as _};
use alloc::vec::Vec;

use serde_json::{Map, Value, json};

/// The id of the account kind, and of the sign-in method that makes it.
pub const KIND: &str = "foundry-local";

/// Where a harness takes the gateway's address and bearer, by the account kind of its own that carries them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Route {
    /// The harness's own account kind that takes an address and a secret.
    pub kind: &'static str,
    /// The key of its credential that holds the bearer.
    pub credential: &'static str,
    /// The key of its settings that holds the address; none when the harness reads it from a file in its folder.
    pub address: Option<&'static str>,
    /// What follows the gateway's address in the address the harness is given.
    pub suffix: &'static str,
    /// Settings of that kind the account fixes.
    pub fixed: &'static [(&'static str, &'static str)],
}

/// How `harness` is given the gateway, when it can be.
#[must_use]
pub fn route(harness: &str) -> Option<Route> {
    match harness {
        "claude" => Some(Route {
            kind: "anthropic-auth-token",
            credential: "auth_token",
            address: Some("base_url"),
            suffix: "",
            fixed: &[],
        }),
        "codex" => Some(Route {
            kind: "codex-api-key",
            credential: "api_key",
            address: Some("openai_base_url"),
            suffix: "/v1",
            fixed: &[],
        }),
        // Pi's own OpenAI provider, with its address overridden in the folder's `models.json` and its key in the variable.
        "pi" => Some(Route {
            kind: "pi-api-key",
            credential: "api_key",
            address: None,
            suffix: "/v1",
            fixed: &[("variable", "OPENAI_API_KEY")],
        }),
        _ => None,
    }
}

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

/// The `config.toml` Codex is run with in its account's folder: the tools and features a local model's gateway cannot serve are
/// off (its hosted web search, and the multi-agent and goal tools whose request shapes only OpenAI's models take).
#[must_use]
pub const fn codex_config() -> &'static str {
    "web_search = \"disabled\"\n\n[features]\nmulti_agent = false\ngoals = false\n"
}

/// The `models.json` of a Pi account's folder: its OpenAI provider is the gateway, at `address`.
#[must_use]
pub fn pi_models(address: &str) -> String {
    json!({"providers": {"openai": {"baseUrl": address}}}).to_string()
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
    fn claude_and_codex_have_a_route_and_others_do_not() {
        assert_eq!(route("claude").expect("a route").credential, "auth_token");
        let codex = route("codex").expect("a route");
        assert_eq!((codex.kind, codex.suffix), ("codex-api-key", "/v1"));
        let pi = route("pi").expect("a route");
        assert_eq!(
            (pi.kind, pi.address, pi.fixed),
            ("pi-api-key", None, &[("variable", "OPENAI_API_KEY")][..])
        );
        assert!(route("grok").is_none());
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

    #[test]
    fn pis_models_file_points_its_openai_provider_at_the_gateway() {
        let file: Value = serde_json::from_str(&pi_models("http://127.0.0.1:1/v1")).expect("JSON");
        assert_eq!(
            file["providers"]["openai"]["baseUrl"],
            "http://127.0.0.1:1/v1"
        );
    }

    #[test]
    fn codexs_config_turns_off_what_a_local_gateway_cannot_serve() {
        let text = codex_config();
        assert!(text.contains("web_search = \"disabled\""));
        assert!(text.contains("multi_agent = false") && text.contains("goals = false"));
    }
}
