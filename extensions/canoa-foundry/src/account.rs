//! Foundry Local as an account: what the person chooses, and the kind and sign-in method that say so to a host.
//!
//! The gateway answers any model name with the one model the account serves, so a harness needs only an address and a bearer.
//! Each harness takes those through an account kind of its own, which its description says (`agent_vendor`'s `gateway_route`), so the
//! account is one the host holds and maps onto that kind for the session.

use alloc::string::{String, ToString as _};
use alloc::vec::Vec;
use core::fmt;

use coqui_acp_api::{
    AccountKindInfo, AccountOffer, EndpointSlot, LoginMethodInfo, MODEL_API, standard,
};
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
    /// How long one answer may take, in seconds, when the person set it: the service engine's default (30 minutes) otherwise.
    pub answer_seconds: Option<u64>,
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

/// Why Foundry Local account settings could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// A text setting was given a non-text or empty value.
    NotText(String),
    /// The settings map holds an unexpected key.
    UnknownSetting(String),
    /// `answer_seconds` is not a positive whole number.
    InvalidAnswerSeconds,
    /// The required `model` setting is missing.
    MissingModel,
    /// `engine` is neither `service` nor `sdk`.
    InvalidEngine,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotText(key) => write!(f, "{key} is text"),
            Self::UnknownSetting(extra) => write!(f, "{extra} is not a setting of this kind"),
            Self::InvalidAnswerSeconds => {
                f.write_str("answer_seconds is a whole number of seconds, at least 1")
            }
            Self::MissingModel => f.write_str("model names the model to serve"),
            Self::InvalidEngine => f.write_str("engine is service or sdk"),
        }
    }
}

impl core::error::Error for Error {}

impl Settings {
    /// Reads the settings of an account from its table.
    ///
    /// # Errors
    ///
    /// A missing model, or a value that is not text.
    pub fn from_map(map: &Map<String, Value>) -> Result<Self, Error> {
        let text = |key: &str| -> Result<Option<String>, Error> {
            match map.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(text)) if !text.is_empty() => Ok(Some(text.clone())),
                Some(_) => Err(Error::NotText(key.to_owned())),
            }
        };
        if let Some(extra) = map.keys().find(|key| {
            !["model", "endpoint", "library", "engine", "answer_seconds"].contains(&key.as_str())
        }) {
            return Err(Error::UnknownSetting(extra.clone()));
        }
        let answer_seconds = match map.get("answer_seconds") {
            None | Some(Value::Null) => None,
            Some(value) => Some(
                value
                    .as_u64()
                    .filter(|seconds| *seconds >= 1)
                    .ok_or(Error::InvalidAnswerSeconds)?,
            ),
        };
        Ok(Self {
            model: text("model")?.ok_or(Error::MissingModel)?,
            endpoint: text("endpoint")?,
            library: text("library")?,
            engine: match text("engine")?.as_deref() {
                None | Some("service") => EngineKind::Service,
                Some("sdk") => EngineKind::Sdk,
                Some(_) => return Err(Error::InvalidEngine),
            },
            window: None,
            answer_seconds,
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
            "answer_seconds": {
                "type": "integer",
                "minimum": 1,
                "title": "Longest answer (seconds)",
                "description": "How long one answer may take before the request to the service is ended, for the `service` engine. A large prompt on a slow model needs a long time; the default is 1800 (30 minutes), and there is no ceiling.",
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

fn schema(json: &Value) -> coqui_acp_api::guatiao::Value {
    guatiao_serde::from_serde(json.clone()).expect("a schema is a value")
}

/// The kind and the sign-in method that make an account of this kind, as an account provider offers them: the settings are the
/// sign-in's inputs, and no credential is kept.
#[must_use]
pub fn offer() -> AccountOffer {
    let mut kind = AccountKindInfo::new(KIND, "Foundry Local");
    kind.description = "A model run by Foundry Local on this computer, served to the agent through a gateway the host runs for each session. No credential is kept: the model's name and where the service is are the account.".to_owned();
    kind.settings = schema(&settings_schema());
    kind.endpoints = vec![EndpointSlot::new(MODEL_API).key("endpoint")];
    // Codex runs in a folder of its own for the account, where the host writes its configuration.
    kind.state_dir = true;
    kind.notices = notices();
    // The gateway answers both wire formats, so an agent that speaks either can use the account.
    kind.provides = vec![
        standard::ANTHROPIC_API.to_owned(),
        standard::OPENAI_API.to_owned(),
    ];
    let mut method = LoginMethodInfo::new(KIND, "Use a Foundry Local model", KIND);
    method.description = "Names a model Foundry Local serves. It is not loaded or checked here: load it in Foundry Local before a session.".to_owned();
    method.inputs = schema(&settings_schema());
    let mut offer = AccountOffer::new();
    offer.kinds = vec![kind];
    offer.logins = vec![method];
    offer
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
    fn how_long_an_answer_may_take_is_a_setting_with_no_ceiling() {
        let none = Settings::from_map(&table(json!({"model": "m"}))).expect("reads");
        assert_eq!(none.answer_seconds, None);
        let long = Settings::from_map(&table(json!({"model": "m", "answer_seconds": 86400})))
            .expect("reads");
        assert_eq!(long.answer_seconds, Some(86400));
        for bad in [json!(0), json!("soon"), json!(-1), json!(1.5)] {
            assert!(
                Settings::from_map(&table(json!({"model": "m", "answer_seconds": bad}))).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn the_offer_is_one_kind_and_the_sign_in_that_makes_it() {
        let offer = offer();
        let kind = offer.kind(KIND).expect("the kind");
        assert!(kind.state_dir && !kind.rotates && !kind.renews);
        assert_eq!(
            kind.provides,
            [standard::ANTHROPIC_API, standard::OPENAI_API]
        );
        assert_eq!(offer.logins.len(), 1);
        assert_eq!(offer.logins[0].id, KIND);
        assert_eq!(offer.logins[0].produces, KIND);
        assert!(offer.usage.is_empty(), "a local model has no plan limits");
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
