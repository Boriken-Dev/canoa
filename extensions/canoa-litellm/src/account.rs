//! The account of a LiteLLM proxy: what the person names, and the kind and sign-in methods that say so to a host.
//!
//! The kind provides `openai-api` and `anthropic-api`: LiteLLM serves `/v1/chat/completions` and `/v1/responses` and
//! `/v1/messages` on one address, and one virtual key's spend and limits count for all of them. Its credentials are `api_key` (what
//! a program sends) and, for a key the proxy's sign-in made, the `refresh_token` and `client_id` that renew it, which no harness is
//! given.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use canoa::api_gateway::{check_address, credential_schema, value};
use coqui_acp_api::standard::{ANTHROPIC_API, OPENAI_API};
use coqui_acp_api::{
    AUTH, AccountKindInfo, AccountOffer, EndpointSlot, LoginMethodInfo, MODEL_API, OPEN_URL,
};
use serde_json::{Map, Value, json};

/// The id of the account kind.
pub const KIND: &str = "litellm";

/// The sign-in that opens the proxy's SSO in the person's browser and receives the key it hands back.
pub const SIGN_IN: &str = "litellm-sso";

/// The sign-in that takes a key the person was given.
pub const SET_KEY: &str = "litellm-key";

/// The key of the credentials that holds what a program sends.
pub const API_KEY: &str = "api_key";

/// The key of the credentials that holds the refresh token the proxy's sign-in gave.
pub const REFRESH_TOKEN: &str = "refresh_token";

/// The key of the credentials that holds the client the sign-in registered with the proxy, which the refresh token belongs to.
pub const CLIENT_ID: &str = "client_id";

/// The setting that holds the proxy's address.
pub const BASE_URL: &str = "base_url";

/// The settings of an account: where the proxy is.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Settings {
    /// The proxy's address without a path: `https://llm.example.com`.
    pub base_url: String,
}

/// Why LiteLLM proxy account settings could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The settings map holds an unexpected key.
    UnknownSetting(String),
    /// The required `base_url` setting is missing or empty.
    MissingBaseUrl,
    /// The proxy address is invalid.
    InvalidAddress(canoa::api_gateway::Error),
}

impl From<canoa::api_gateway::Error> for Error {
    fn from(error: canoa::api_gateway::Error) -> Self {
        Self::InvalidAddress(error)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownSetting(extra) => write!(f, "{extra} is not a setting of this kind"),
            Self::MissingBaseUrl => f.write_str("base_url is the address of the LiteLLM proxy"),
            Self::InvalidAddress(error) => error.fmt(f),
        }
    }
}

impl core::error::Error for Error {}

impl Settings {
    /// Reads the settings from a table. A trailing `/` and a final `/v1` are dropped: the address is the proxy's root, where its
    /// management routes are, and each program adds the path of its own API.
    ///
    /// # Errors
    ///
    /// A missing or wrong address, or a key the kind lacks.
    pub fn from_map(map: &Map<String, Value>) -> Result<Self, Error> {
        if let Some(extra) = map.keys().find(|key| key.as_str() != BASE_URL) {
            return Err(Error::UnknownSetting(extra.clone()));
        }
        let given = map
            .get(BASE_URL)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .ok_or(Error::MissingBaseUrl)?;
        check_address(given)?;
        let root = given.trim_end_matches('/');
        let root = root.strip_suffix("/v1").unwrap_or(root);
        Ok(Self {
            base_url: root.trim_end_matches('/').to_owned(),
        })
    }

    /// The settings as the account's table holds them.
    #[must_use]
    pub fn to_value(&self) -> Value {
        json!({ BASE_URL: self.base_url })
    }
}

/// The JSON schema of the account's settings, and of its key.
#[must_use]
pub fn settings_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            BASE_URL: {
                "type": "string",
                "format": "uri",
                "title": "Proxy address",
                "description": "Where the LiteLLM proxy is, without a path: `https://llm.example.com`. Codex, Claude Code and the others add their own.",
            },
            API_KEY: credential_schema("Key", "The virtual key the proxy gave you."),
        },
        "required": [BASE_URL, API_KEY],
        "additionalProperties": false,
    })
}

/// What a sign-in asks before it runs: the proxy's address.
#[must_use]
pub fn inputs_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            BASE_URL: {
                "type": "string",
                "format": "uri",
                "title": "Proxy address",
                "description": "Where the LiteLLM proxy is, without a path: `https://llm.example.com`.",
            },
        },
        "required": [BASE_URL],
    })
}

/// The standards the kind provides: the proxy serves both on the one address and the one key.
#[must_use]
pub fn provides() -> Vec<String> {
    vec![OPENAI_API.to_owned(), ANTHROPIC_API.to_owned()]
}

/// The kind and the two ways to make an account of it.
#[must_use]
pub fn offer() -> AccountOffer {
    let mut kind = AccountKindInfo::new(KIND, "LiteLLM");
    kind.description = "A LiteLLM proxy: one address and one virtual key for the OpenAI API and the Anthropic Messages API, so Codex, Claude Code and any other program that takes either runs on the same account and counts against the same budget.".to_owned();
    kind.settings = value(&settings_schema());
    kind.endpoints = vec![
        EndpointSlot::new(MODEL_API).key(BASE_URL),
        EndpointSlot::new(AUTH).key(BASE_URL),
    ];
    kind.renews = true;
    kind.provides = provides();
    kind.notices = vec![
        "the key is sent to the proxy's address by each program that runs on the account: use a proxy you trust".to_owned(),
    ];
    let mut sign_in = LoginMethodInfo::new(SIGN_IN, "Sign in to the proxy", KIND);
    sign_in.description = "Opens the proxy's own sign-in (its SSO) in your browser and receives the key it hands back to an address on this computer. The proxy has to offer it (a beta of LiteLLM).".to_owned();
    sign_in.inputs = value(&inputs_schema());
    sign_in.steps = vec![OPEN_URL.to_owned()];
    let mut set_key = LoginMethodInfo::new(SET_KEY, "Use a LiteLLM key", KIND);
    set_key.description = "Names the proxy and a virtual key you were given. The key is asked in the form only your application reads and kept as a secret.".to_owned();
    set_key.inputs = value(&inputs_schema());
    let mut offer = AccountOffer::new();
    offer.kinds = vec![kind];
    offer.logins = vec![sign_in, set_key];
    offer.usage = vec![KIND.to_owned()];
    offer
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(value: Value) -> Map<String, Value> {
        value.as_object().cloned().expect("a table")
    }

    #[test]
    fn the_address_is_the_proxys_root_whatever_the_person_wrote_after_it() {
        for (given, root) in [
            ("https://llm.example.com", "https://llm.example.com"),
            ("https://llm.example.com/", "https://llm.example.com"),
            ("https://llm.example.com/v1", "https://llm.example.com"),
            ("https://llm.example.com/v1/", "https://llm.example.com"),
            ("http://127.0.0.1:4000/v1", "http://127.0.0.1:4000"),
            ("http://192.0.2.11:4000/v1", "http://192.0.2.11:4000"),
            ("http://llm.example.com", "http://llm.example.com"),
            ("https://example.com/litellm", "https://example.com/litellm"),
        ] {
            let read = Settings::from_map(&table(json!({"base_url": given}))).expect(given);
            assert_eq!(read.base_url, root, "{given}");
        }
        for bad in [
            json!({}),
            json!({"base_url": "ftp://llm.example.com"}),
            json!({"base_url": "https://u:p@llm.example.com"}),
            json!({"base_url": "https://llm.example.com", "api_key": "k"}),
        ] {
            assert!(Settings::from_map(&table(bad.clone())).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_kind_provides_both_apis_renews_and_reports_usage_and_has_two_ways_in() {
        let offer = offer();
        let kind = offer.kind(KIND).expect("the kind");
        assert_eq!(kind.provides, [OPENAI_API, ANTHROPIC_API]);
        assert!(kind.renews && !kind.rotates && !kind.state_dir);
        assert_eq!(offer.usage, [KIND]);
        let ways: Vec<&str> = offer.logins.iter().map(|way| way.id.as_str()).collect();
        assert_eq!(ways, [SIGN_IN, SET_KEY]);
        assert!(offer.logins.iter().all(|way| way.produces == KIND));
        assert_eq!(offer.logins[0].steps, [OPEN_URL]);
        // The credential is a secret in the schema, so no host keeps it in a table.
        assert_eq!(
            settings_schema()["properties"][API_KEY]["x-sensitive"],
            json!(true)
        );
    }
}
