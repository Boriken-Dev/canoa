//! The account kind of an address that speaks one or several credential standards, and the pieces any account provider of such an
//! address needs.
//!
//! A gateway or proxy (a service in front of many models) is an address and a key, and speaks the wire APIs it says: the OpenAI
//! API, the Anthropic Messages API, any other a harness accepts. An account of the kind [`ACCOUNT_KIND`] holds the address, the standards
//! the person says it speaks and the key (a secret, never a setting), and every harness that accepts one of those standards
//! runs on it, the address moved to the harness's own endpoint setting.
//!
//! The standards are the account's own: the kind's settings schema marks the property that names them `"x-provides": true`, which a
//! host reads as the standards the account provides in place of the kind's. A gateway that serves its standards at different
//! addresses, or wants a key as a bearer token for one and as a key for another, says so for each in the property marked
//! `"x-by-standard": true`: the address and the credential key a program is given when it applies the account as that standard.
//!
//! [`ApiGatewayAccounts`] is the `account_provider` of the kind and [`library`] what a host loads. The rest is what an account
//! provider of another service in front of models (a sign-in that hands back a key, a budget to report) builds on: [`http`] reaches
//! the address through the person's own network, [`openai_models`] lists what it serves, [`parse_standards`] and [`check_address`]
//! say what a setting may be, and [`key_form`] asks for the key in the form only the person's application reads.

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString as _};
use alloc::vec::Vec;
use core::fmt;

use coqui_acp_api::guatiao::Provider;
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::standard::{ANTHROPIC_API, OPENAI_API, is_standard, well_known};
use coqui_acp_api::{
    AccountKeeper, AccountKindInfo, AccountModelsQuery, AccountOffer, AccountProvider,
    AccountProviderInfo, AccountRef, AccountUsage, AccountUsageQuery, AccountWatch, Credentials,
    EndpointSlot, LoginAnswer, LoginForm, LoginMethodInfo, LoginOutcome, LoginPrompter,
    LoginRequest, MODEL_API, ModelInfo, ModelListing, ProcessSpawner, RenewReport, RenewRequest,
    UnknownKind, WatchRequest,
};
use serde_json::{Map, Value, json};

pub mod http;

/// The id of the account kind, and of the sign-in method that makes it.
pub const ACCOUNT_KIND: &str = "api-gateway";

/// The name this library is registered under.
pub const LIBRARY: &str = "canoa-api-gateway";

/// The key of the credentials that holds the key the address is shown.
pub const CREDENTIAL: &str = "api_key";

/// The setting that holds the address.
pub const BASE_URL: &str = "base_url";

/// The setting that holds the standards the address speaks, which a host reads as what the account provides.
pub const STANDARDS: &str = "standards";

/// The annotation of the setting that names the standards.
pub const PROVIDES_NOTE: &str = "x-provides";

/// The setting that holds, for a standard the address speaks, the address a program is given and the credential key its key is given
/// under.
pub const BY_STANDARD: &str = "by_standard";

/// The annotation of the setting that holds them: a host gives each program the address and the key form of the standard it applies.
pub const BY_STANDARD_NOTE: &str = "x-by-standard";

/// The sign-in input that names an address for some standards: `standard=address` pairs.
pub const ADDRESSES: &str = "addresses";

/// The sign-in input that names how the key is sent for some standards: `standard=credential` pairs.
pub const CREDENTIALS: &str = "credentials";

/// Why API gateway settings or inputs could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// A standard identifier in `standards` is invalid.
    InvalidStandardId,
    /// `standards` named no standard.
    NoStandards,
    /// An entry in `addresses` or `credentials` is not formatted as `standard=value`.
    MalformedPair(String),
    /// A well-known standard does not take the given credential key.
    WrongStandardCredential {
        /// The standard identifier.
        standard: String,
        /// The credential key that was supplied.
        key: String,
        /// The credential key names accepted by the standard.
        expected: String,
    },
    /// A credential key does not have a valid key identifier format.
    InvalidCredentialKey(String),
    /// An address is not a valid `http` or `https` URL.
    Address(crate::address::Error),
    /// A settings map contains an unexpected key.
    UnknownSetting(String),
    /// The required `base_url` setting is missing or empty.
    MissingBaseUrl,
    /// The required `standards` setting is missing or not text/array.
    MissingStandards,
    /// The `by_standard` setting is not a table.
    ByStandardNotObject,
    /// An entry under `by_standard` is not a table.
    ByStandardEntryNotObject {
        /// The standard identifier whose entry was not a table.
        standard: String,
    },
    /// A per-standard entry in `by_standard` contains an unexpected key.
    UnknownStandardSetting {
        /// The standard identifier.
        standard: String,
        /// The unexpected key name.
        key: String,
    },
    /// A per-standard setting in `by_standard` is not text.
    StandardSettingNotText {
        /// The standard identifier.
        standard: String,
        /// The setting key name.
        field: String,
    },
    /// An `addresses` or `credentials` sign-in input is not text.
    PairsNotText {
        /// The input field name (`addresses` or `credentials`).
        input: String,
    },
    /// A `standard=value` pair inside `addresses` or `credentials` is malformed.
    InvalidPair {
        /// The input field name (`addresses` or `credentials`).
        input: String,
        /// The malformed pair entry.
        part: String,
    },
    /// A standard was referenced in `by_standard`, `addresses`, or `credentials`, but is not listed in `standards`.
    StandardNotSpoken(String),
    /// A per-standard override address is not a valid `http` or `https` URL.
    InvalidStandardAddress {
        /// The standard identifier.
        standard: String,
        /// Why the address was rejected.
        error: crate::address::Error,
    },
}

impl From<crate::address::Error> for Error {
    fn from(error: crate::address::Error) -> Self {
        Self::Address(error)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidStandardId => f.write_str(
                "a standard is named by lower-case letters, digits and single hyphens, starting with a letter (`openai-api`)",
            ),
            Self::NoStandards => f.write_str("name at least one standard the address speaks"),
            Self::MalformedPair(part) => write!(f, "`{part}` is not `standard=value`"),
            Self::WrongStandardCredential {
                standard,
                key,
                expected,
            } => write!(f, "{standard} takes its key as {expected}, not {key}"),
            Self::InvalidCredentialKey(key) => write!(
                f,
                "`{key}` is not a credential key (lower-case letters, digits and underscores, starting with a letter)"
            ),
            Self::Address(error) => error.fmt(f),
            Self::UnknownSetting(extra) => write!(f, "{extra} is not a setting of this kind"),
            Self::MissingBaseUrl => f.write_str("base_url is the address of the gateway"),
            Self::MissingStandards => f.write_str("standards lists what the address speaks"),
            Self::ByStandardNotObject => {
                write!(f, "{BY_STANDARD} is a table of the standards' ids")
            }
            Self::ByStandardEntryNotObject { standard } => {
                write!(f, "{BY_STANDARD}.{standard} is a table")
            }
            Self::UnknownStandardSetting { standard, key } => {
                write!(f, "{key} is not a setting of {standard} in {BY_STANDARD}")
            }
            Self::StandardSettingNotText { standard, field } => {
                write!(f, "{BY_STANDARD}.{standard}.{field} is text")
            }
            Self::PairsNotText { input } => {
                write!(f, "{input} is `standard=value` pairs, as text")
            }
            Self::InvalidPair { input, part } => {
                write!(f, "{input}: `{part}` is not `standard=value`")
            }
            Self::StandardNotSpoken(standard) => write!(
                f,
                "{standard} is named for an address or a key, and the address does not speak it"
            ),
            Self::InvalidStandardAddress { standard, error } => {
                write!(f, "the address for {standard}: {error}")
            }
        }
    }
}

impl core::error::Error for Error {}

/// A JSON value as a Guatiao one, for a schema or a setting.
///
/// # Panics
///
/// Never for what this crate builds; JSON has nothing a Guatiao value cannot hold.
#[must_use]
pub fn value(json: &Value) -> coqui_acp_api::guatiao::Value {
    guatiao_serde::from_serde(json.clone()).expect("JSON is a value")
}

/// The schema of a property that is a credential: text, marked `writeOnly` and `x-sensitive`, with no default.
#[must_use]
pub fn credential_schema(title: &str, description: &str) -> Value {
    json!({
        "type": "string",
        "title": title,
        "description": description,
        "writeOnly": true,
        "x-sensitive": true,
    })
}

/// The form a sign-in asks the person to fill for a key: one secret field named [`CREDENTIAL`]. The answer comes back sealed in
/// [`LoginAnswer::secrets`].
#[must_use]
pub fn key_form(title: &str, what: &str) -> LoginForm {
    LoginForm::new(
        title,
        value(&json!({
            "type": "object",
            "properties": {CREDENTIAL: credential_schema("Key", what)},
            "required": [CREDENTIAL],
        })),
    )
}

/// The key `answer` holds, trimmed.
///
/// # Errors
///
/// `BAD_VALUE`, saying nothing was entered.
pub fn key_of(answer: &LoginAnswer) -> Result<String, ProviderError> {
    let key = answer
        .secrets
        .expose()
        .get(CREDENTIAL)
        .unwrap_or_default()
        .trim()
        .to_owned();
    if key.is_empty() {
        return Err(crate::status::bad_value(
            "login refused: nothing was entered",
        ));
    }
    Ok(key)
}

/// Splits `text` (ids separated by commas or white space) into standard ids, each once, in the order given.
///
/// # Errors
///
/// No id, or one that is not an id a standard can be.
pub fn parse_standards(text: &str) -> Result<Vec<String>, Error> {
    let mut found: Vec<String> = Vec::new();
    for id in text
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|id| !id.is_empty())
    {
        if !is_standard(id) {
            return Err(Error::InvalidStandardId);
        }
        if !found.iter().any(|each| each == id) {
            found.push(id.to_owned());
        }
    }
    if found.is_empty() {
        return Err(Error::NoStandards);
    }
    Ok(found)
}

/// Splits `text` (`name=value` pairs separated by commas or white space) into its pairs, in the order given.
///
/// # Errors
///
/// A part that is not `name=value`.
pub fn parse_pairs(text: &str) -> Result<Vec<(String, String)>, Error> {
    text.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|part| !part.is_empty())
        .map(|part| match part.split_once('=') {
            Some((name, value)) if !name.is_empty() && !value.is_empty() => {
                Ok((name.to_owned(), value.to_owned()))
            }
            _ => Err(Error::MalformedPair(part.to_owned())),
        })
        .collect()
}

/// Whether `key` is a credential key the standard `standard` takes: one of the keys it names when it is a well-known standard, else
/// any key written as a name (lower-case letters, digits and underscores, starting with a letter).
///
/// # Errors
///
/// Why it is not.
pub fn check_credential(standard: &str, key: &str) -> Result<(), Error> {
    if let Some(known) = well_known(standard) {
        return if known.credentials.contains(&key) {
            Ok(())
        } else {
            Err(Error::WrongStandardCredential {
                standard: standard.to_owned(),
                key: key.to_owned(),
                expected: known.credentials.join(" or "),
            })
        };
    }
    let name = key
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_lowercase())
        && key
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if name {
        Ok(())
    } else {
        Err(Error::InvalidCredentialKey(key.to_owned()))
    }
}

/// Whether `url` is an address a key may be sent to: an `http` or `https` one with a host and no login in it. Plain `http` is the
/// person's choice wherever it goes; [`crate::address::plain_http_note`] says what it costs.
///
/// # Errors
///
/// Why it is not.
pub fn check_address(url: &str) -> Result<(), Error> {
    crate::address::parse(url).map(|_| ()).map_err(Error::from)
}

/// The address of the model list of an OpenAI-API address `base`: `/v1/models`, or `/models` when `base` already ends in `/v1`.
#[must_use]
pub fn models_url(base: &str) -> String {
    let base = base.trim_end_matches('/');
    if base.ends_with("/v1") {
        format!("{base}/models")
    } else {
        format!("{base}/v1/models")
    }
}

const ANTHROPIC_VERSION: &str = "2023-06-01";
const AUTH_TOKEN_CREDENTIAL: &str = "auth_token";

/// The most models Anthropic's model list gives in one page; it lists twenty unless asked for more.
const ANTHROPIC_PAGE: u32 = 1000;

fn list_models(
    base: &str,
    headers: &[(&str, &str)],
    format_name: &str,
    paged: bool,
) -> ModelListing {
    if let Err(why) = check_address(base) {
        return ModelListing::unavailable(UnknownKind::Failed, why.to_string());
    }
    let mut models = Vec::new();
    let mut after: Option<String> = None;
    loop {
        let mut url = models_url(base);
        if paged {
            url.push_str(&format!("?limit={ANTHROPIC_PAGE}"));
            if let Some(after) = &after {
                url.push_str("&after_id=");
                url.extend(percent_encoding::utf8_percent_encode(
                    after,
                    percent_encoding::NON_ALPHANUMERIC,
                ));
            }
        }
        let reply = match http::get_json_with(&url, headers) {
            Ok(reply) => reply,
            Err(why) => return ModelListing::unavailable(UnknownKind::Failed, why.to_string()),
        };
        if !(200..300).contains(&reply.status) {
            return ModelListing::unavailable(
                UnknownKind::Failed,
                format!("the address answered {} to its model list", reply.status),
            );
        }
        let Some(list) = reply.body.get("data").and_then(Value::as_array) else {
            return ModelListing::unavailable(
                UnknownKind::Failed,
                format!("the address's model list is not an {format_name} one (no `data`)"),
            );
        };
        models.extend(list.iter().filter_map(|each| {
            let id = each.get("id").and_then(Value::as_str)?;
            let mut model = ModelInfo::new(id);
            if let Some(name) = each.get("display_name").and_then(Value::as_str)
                && !name.is_empty()
            {
                model.name = Some(name.to_owned());
            }
            Some(model)
        }));
        // A page says there is more and where it ended; an address that does not page answers whole and says neither.
        let next = reply
            .body
            .get("last_id")
            .and_then(Value::as_str)
            .filter(|_| paged && reply.body.get("has_more").and_then(Value::as_bool) == Some(true))
            .filter(|last| after.as_deref() != Some(*last));
        match next {
            Some(last) => after = Some(last.to_owned()),
            None => break,
        }
    }
    ModelListing::of(models)
}

/// The models an OpenAI-API address serves, as its `GET /v1/models` lists them with `key`.
///
/// Anything it cannot say is an unknown with the reason (the address does not answer, refuses the key, answers what is not a list),
/// never a guess.
#[must_use]
pub fn openai_models(base: &str, key: &str) -> ModelListing {
    let authorization = format!("Bearer {key}");
    list_models(base, &[("Authorization", &authorization)], "OpenAI", false)
}

/// The models an Anthropic-API address serves, as its `GET /v1/models` lists them, page after page while it says there is more, with `key` (sent as `Authorization: Bearer`
/// when `bearer` is set, else `x-api-key`) and `anthropic-version: 2023-06-01`.
///
/// Anything it cannot say is an unknown with the reason (the address does not answer, refuses the key, answers what is not a list),
/// never a guess.
#[must_use]
pub fn anthropic_models(base: &str, key: &str, bearer: bool) -> ModelListing {
    let authorization;
    let headers = if bearer {
        authorization = format!("Bearer {key}");
        [
            ("Authorization", authorization.as_str()),
            ("anthropic-version", ANTHROPIC_VERSION),
        ]
    } else {
        [("x-api-key", key), ("anthropic-version", ANTHROPIC_VERSION)]
    };
    list_models(base, &headers, "Anthropic", true)
}

/// The settings of an account of this kind, as its table holds them.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Settings {
    /// The address, as the person wrote it: the one a program is given for a standard that names none of its own.
    pub base_url: String,
    /// The standards it speaks.
    pub standards: Vec<String>,
    /// What an account says of a standard it speaks, by the standard's id.
    pub by_standard: BTreeMap<String, StandardSettings>,
}

/// What an account says of one standard it speaks: where a program that applies the account as that standard is sent, and the
/// credential key its key is given under there.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct StandardSettings {
    /// The address for this standard, in place of the account's own.
    pub base_url: Option<String>,
    /// The key the account's credential is given under for this standard (`auth_token` for a bearer token).
    pub credential: Option<String>,
}

impl Settings {
    /// Reads the settings from a table, where `standards` is a list of ids or text of ids separated by commas or white space.
    /// `by_standard` is a table keyed by the standards' ids; `addresses` and `credentials`, as a sign-in's inputs have them, are text
    /// of `standard=address` and `standard=credential` pairs, and name the same things.
    ///
    /// # Errors
    ///
    /// A missing or wrong address or standard, a standard the address does not speak named for an address or a key, or a key the
    /// kind lacks.
    pub fn from_map(map: &Map<String, Value>) -> Result<Self, Error> {
        if let Some(extra) = map.keys().find(|key| {
            ![BASE_URL, STANDARDS, BY_STANDARD, ADDRESSES, CREDENTIALS].contains(&key.as_str())
        }) {
            return Err(Error::UnknownSetting(extra.clone()));
        }
        let base_url = map
            .get(BASE_URL)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .ok_or(Error::MissingBaseUrl)?;
        check_address(base_url)?;
        let standards = match map.get(STANDARDS) {
            Some(Value::String(text)) => parse_standards(text)?,
            Some(Value::Array(listed)) => parse_standards(
                &listed
                    .iter()
                    .map(|each| each.as_str().unwrap_or("?"))
                    .collect::<Vec<_>>()
                    .join(" "),
            )?,
            _ => return Err(Error::MissingStandards),
        };
        let mut by_standard: BTreeMap<String, StandardSettings> = BTreeMap::new();
        match map.get(BY_STANDARD) {
            None | Some(Value::Null) => {}
            Some(Value::Object(table)) => {
                for (standard, entry) in table {
                    let Value::Object(entry) = entry else {
                        return Err(Error::ByStandardEntryNotObject {
                            standard: standard.clone(),
                        });
                    };
                    if let Some(extra) = entry
                        .keys()
                        .find(|key| ![BASE_URL, "credential"].contains(&key.as_str()))
                    {
                        return Err(Error::UnknownStandardSetting {
                            standard: standard.clone(),
                            key: extra.clone(),
                        });
                    }
                    let text = |field: &str| {
                        entry
                            .get(field)
                            .map(|value| {
                                value.as_str().map(str::to_owned).ok_or_else(|| {
                                    Error::StandardSettingNotText {
                                        standard: standard.clone(),
                                        field: field.to_owned(),
                                    }
                                })
                            })
                            .transpose()
                    };
                    let slot = by_standard.entry(standard.clone()).or_default();
                    slot.base_url = text(BASE_URL)?;
                    slot.credential = text("credential")?;
                }
            }
            Some(_) => return Err(Error::ByStandardNotObject),
        }
        for (input, address) in [(ADDRESSES, true), (CREDENTIALS, false)] {
            let Some(text) = map.get(input) else { continue };
            let text = text.as_str().ok_or_else(|| Error::PairsNotText {
                input: input.to_owned(),
            })?;
            let pairs = parse_pairs(text).map_err(|error| match error {
                Error::MalformedPair(part) => Error::InvalidPair {
                    input: input.to_owned(),
                    part,
                },
                other => other,
            })?;
            for (standard, value) in pairs {
                let slot = by_standard.entry(standard).or_default();
                if address {
                    slot.base_url = Some(value);
                } else {
                    slot.credential = Some(value);
                }
            }
        }
        for (standard, slot) in &mut by_standard {
            if !standards.contains(standard) {
                return Err(Error::StandardNotSpoken(standard.clone()));
            }
            if let Some(url) = &slot.base_url {
                crate::address::parse(url).map_err(|error| Error::InvalidStandardAddress {
                    standard: standard.clone(),
                    error,
                })?;
                slot.base_url = Some(url.trim_end_matches('/').to_owned());
            }
            if let Some(key) = &slot.credential {
                check_credential(standard, key)?;
            }
        }
        by_standard.retain(|_, slot| slot.base_url.is_some() || slot.credential.is_some());
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            standards,
            by_standard,
        })
    }

    /// The settings as the account's table holds them.
    #[must_use]
    pub fn to_value(&self) -> Value {
        let mut value = json!({BASE_URL: self.base_url, STANDARDS: self.standards});
        if !self.by_standard.is_empty() {
            let table: Map<String, Value> = self
                .by_standard
                .iter()
                .map(|(standard, slot)| {
                    let mut entry = Map::new();
                    if let Some(url) = &slot.base_url {
                        entry.insert(BASE_URL.to_owned(), json!(url));
                    }
                    if let Some(key) = &slot.credential {
                        entry.insert("credential".to_owned(), json!(key));
                    }
                    (standard.clone(), Value::Object(entry))
                })
                .collect();
            value[BY_STANDARD] = Value::Object(table);
        }
        value
    }

    /// The address a program that applies the account as `standard` is given: the one the account names for it, else its own.
    #[must_use]
    pub fn address_for(&self, standard: &str) -> &str {
        self.by_standard
            .get(standard)
            .and_then(|slot| slot.base_url.as_deref())
            .unwrap_or(&self.base_url)
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
                "title": "Address",
                "description": "Where the gateway is, as each program that runs on the account takes it: `https://llm.example.com`. A program adds its own path to it, and some expect it to end in `/v1` where others do not: name the address of a standard that differs in `by_standard`.",
            },
            STANDARDS: {
                "type": "array",
                "items": {"type": "string"},
                "title": "Standards it speaks",
                "description": "The credential standards the address accepts a key for, such as `openai-api` and `anthropic-api`. Every program that takes one of them runs on this account.",
                PROVIDES_NOTE: true,
            },
            BY_STANDARD: {
                "type": "object",
                "title": "Address and key form for each standard",
                "description": "For a standard the address speaks: `base_url`, where a program that applies the account as that standard is sent in place of the address above (`https://llm.example.com/v1`), and `credential`, the key the account's key is given under for it (`auth_token` sends it as a bearer token where the standard takes one, `api_key` as a key).",
                "additionalProperties": {
                    "type": "object",
                    "properties": {
                        BASE_URL: {"type": "string", "format": "uri"},
                        "credential": {"type": "string"},
                    },
                    "additionalProperties": false,
                },
                BY_STANDARD_NOTE: true,
            },
            CREDENTIAL: credential_schema("Key", "The key the gateway gave you."),
        },
        "required": [BASE_URL, STANDARDS, CREDENTIAL],
        "additionalProperties": false,
    })
}

/// What the sign-in asks before it runs: the address and the standards, as text.
#[must_use]
pub fn inputs_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            BASE_URL: {
                "type": "string",
                "format": "uri",
                "title": "Address",
                "description": "Where the gateway is: `https://llm.example.com`.",
            },
            STANDARDS: {
                "type": "string",
                "title": "Standards it speaks",
                "description": "Ids separated by commas, such as `openai-api, anthropic-api`. Every program that takes one of them runs on this account.",
            },
            ADDRESSES: {
                "type": "string",
                "title": "Address for a standard",
                "description": "`standard=address` pairs, separated by commas, for a standard the address above does not serve: `openai-api=https://llm.example.com/v1`. Optional.",
            },
            CREDENTIALS: {
                "type": "string",
                "title": "How the key is sent for a standard",
                "description": "`standard=credential` pairs, separated by commas, for a standard that takes its key another way than `api_key`: `anthropic-api=auth_token` sends it as a bearer token. Optional.",
            },
        },
        "required": [BASE_URL, STANDARDS],
    })
}

/// The kind and the sign-in method that make an account of it.
#[must_use]
pub fn offer() -> AccountOffer {
    let mut kind = AccountKindInfo::new(ACCOUNT_KIND, "API gateway");
    kind.description = "An address that speaks one or several of the APIs agents use, and a key for it (a gateway or proxy in front of many models). Every program that takes one of the standards you name runs on it.".to_owned();
    kind.settings = value(&settings_schema());
    kind.endpoints = vec![EndpointSlot::new(MODEL_API).key(BASE_URL)];
    kind.notices = vec![
        "the key is sent to the address you gave, by each program that runs on the account: use an address you trust".to_owned(),
    ];
    let mut method =
        LoginMethodInfo::new(ACCOUNT_KIND, "A gateway's address and key", ACCOUNT_KIND);
    method.description = "Names an address, the standards it speaks and a key for it. The key is asked in the form only your application reads and kept as a secret.".to_owned();
    method.inputs = value(&inputs_schema());
    let mut offer = AccountOffer::new();
    offer.kinds = vec![kind];
    offer.logins = vec![method];
    offer
}

/// The accounts that name an address and a key: the standards the address speaks are the account's own.
#[derive(Default, Debug, Clone, Copy, Provider)]
#[provider(AccountProvider, id = "api_gateway_accounts", crate = ::coqui_acp_api::guatiao)]
pub struct ApiGatewayAccounts;

impl AccountProvider for ApiGatewayAccounts {
    fn describe(&self) -> Result<AccountProviderInfo, ProviderError> {
        let mut info = AccountProviderInfo::new("API gateway");
        info.description = "Accounts that name an address and a key and the standards the address speaks, for any gateway or proxy.".to_owned();
        Ok(info)
    }

    fn offers(&self) -> Result<AccountOffer, ProviderError> {
        Ok(offer())
    }

    fn provides(&self, account: AccountRef) -> Result<Vec<String>, ProviderError> {
        account.check_id()?;
        if account.kind != KIND {
            return Err(refused("this provider keeps no account of that kind"));
        }
        // The account's own list: the address speaks whichever standards the person named.
        let settings = guatiao_serde::adapt::from_value::<Map<String, Value>>(&account.settings)
            .map_err(|_| refused("the account's settings are not a table"))?;
        Ok(Settings::from_map(&settings).map_err(refused)?.standards)
    }

    fn login(
        &self,
        request: LoginRequest,
        spawner: Object<dyn ProcessSpawner>,
        prompter: Object<dyn LoginPrompter>,
    ) -> Result<LoginOutcome, ProviderError> {
        drop(spawner);
        if request.method != ACCOUNT_KIND {
            return Err(crate::status::bad_value(
                "an API gateway has no such way to sign in",
            ));
        }
        let inputs = guatiao_serde::adapt::from_value::<Map<String, Value>>(&request.inputs)
            .map_err(|_| crate::status::bad_value("the inputs are not a table"))?;
        let settings = Settings::from_map(&inputs)
            .map_err(|error| crate::status::bad_value(error.to_string()))?;
        let label = settings
            .base_url
            .split("://")
            .nth(1)
            .and_then(|rest| rest.split('/').next())
            .map(str::to_owned);
        let form = key_form(
            &format!(
                "Enter the key for {}",
                label.as_deref().unwrap_or("the gateway")
            ),
            "The key the gateway gave you. It is sent to that address only.",
        );
        let answer = prompter.ask(form)?;
        let key = key_of(&answer)?;
        let mut credentials = Credentials::new();
        credentials.insert(CREDENTIAL, key);
        let mut outcome = LoginOutcome::new(ACCOUNT_KIND, credentials);
        outcome.settings = value(&settings.to_value());
        outcome.label = label;
        Ok(outcome)
    }

    fn renew(
        &self,
        request: RenewRequest,
        spawner: Object<dyn ProcessSpawner>,
    ) -> Result<RenewReport, ProviderError> {
        drop((request, spawner));
        Err(crate::status::unsupported(
            "a key you were given is not renewed here",
        ))
    }

    fn watch(
        &self,
        request: WatchRequest,
        spawner: Object<dyn ProcessSpawner>,
        keeper: Object<dyn AccountKeeper>,
    ) -> Result<Object<dyn AccountWatch>, ProviderError> {
        drop((request, spawner, keeper));
        Err(crate::status::unsupported(
            "a gateway key has no login to watch",
        ))
    }

    fn usage(
        &self,
        query: AccountUsageQuery,
        spawner: Object<dyn ProcessSpawner>,
        keeper: Object<dyn AccountKeeper>,
    ) -> Result<AccountUsage, ProviderError> {
        drop((spawner, keeper));
        let mut usage = AccountUsage::unsupported(
            "a gateway of unknown make reports no plan limits through a standard API",
        );
        usage.account = Some(query.account.id);
        Ok(usage)
    }

    fn models(
        &self,
        query: AccountModelsQuery,
        spawner: Object<dyn ProcessSpawner>,
        keeper: Object<dyn AccountKeeper>,
    ) -> Result<ModelListing, ProviderError> {
        drop((spawner, keeper));
        let grant = query.account;
        grant.check_id()?;
        if grant.kind != ACCOUNT_KIND {
            return Err(crate::status::bad_value(
                "this provider keeps no account of that kind",
            ));
        }
        let standard = if grant.provides.iter().any(|each| each == OPENAI_API) {
            OPENAI_API
        } else if grant.provides.iter().any(|each| each == ANTHROPIC_API) {
            ANTHROPIC_API
        } else {
            return Ok(ModelListing::unknown(format!(
                "the address's models are listed through the OpenAI or Anthropic API, and this account is used as {}",
                if grant.provides.is_empty() {
                    "no standard".to_owned()
                } else {
                    grant.provides.join(", ")
                }
            )));
        };
        let settings = guatiao_serde::adapt::from_value::<Map<String, Value>>(&grant.settings)
            .map_err(|_| crate::status::bad_value("the account's settings are not a table"))?;
        let Some(base) = settings.get(BASE_URL).and_then(Value::as_str) else {
            return Err(crate::status::bad_value("the account names no address"));
        };
        let key = grant
            .credentials
            .expose()
            .get(CREDENTIAL)
            .ok_or_else(|| crate::status::bad_value("the account holds no key"))?
            .to_string();
        let override_entry = settings
            .get(BY_STANDARD)
            .and_then(|table| table.get(standard));
        let address = override_entry
            .and_then(|entry| entry.get(BASE_URL))
            .and_then(Value::as_str)
            .unwrap_or(base);
        if standard == ANTHROPIC_API {
            let bearer = override_entry
                .and_then(|entry| entry.get("credential"))
                .and_then(Value::as_str)
                == Some(AUTH_TOKEN_CREDENTIAL);
            Ok(anthropic_models(address, &key, bearer))
        } else {
            Ok(openai_models(address, &key))
        }
    }
}

coqui_acp_api::guatiao::local_providers!(
    id = "canoa-api-gateway",
    version = env!("CARGO_PKG_VERSION"),
    providers = [ApiGatewayAccounts]
);
