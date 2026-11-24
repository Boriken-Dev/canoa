//! `[secrets]` of canoa.cfg: which provider answers which name, the providers the person set up, and how long what they give is kept.
//!
//! ```toml
//! [secrets]
//! default = "form"           # the entry that answers a name no `names` line gives (default `form`)
//! accounts = "auto"          # who keeps the accounts' credentials and mail passwords: auto, or an entry that keeps (default auto)
//! store = "system"           # the entry "remember" keeps a typed secret in (default `system`)
//! keep = "session"           # what the form offers first: once, session, close, 15m, 1h, 8h, 1d
//! max_wait = 240             # the most seconds a request waits for the person, whatever the agent asks (default 240, and never
//!                            # more than a sealed question lasts)
//!
//! [secrets.names]
//! github_token = "work_vault"
//!
//! [secrets.providers.work_vault]
//! provider = "bitwarden"     # the id of a secret_provider
//! keep = "close"             # how long a value this entry gave is kept in memory (default session)
//! command = "bw"             # every other key is the provider's own setting
//!
//! [secrets.providers.work_vault.from]   # a setting of the provider's that is another secret, never asked for
//! session = { entry = "form", name = "bw_session" }
//! ```
//!
//! The entries `form` and `system` are there without being written.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "the settings are the host's"
)]

use std::collections::BTreeMap;

#[cfg(test)]
use coqui_acp_api::guatiao::ValueError;
use coqui_acp_api::guatiao::{FromValue, Schema, ToValue, Value as GuatiaoValue};
use serde_json::{Map, Value};

use super::builtin::{FILE, FORM, SYSTEM};

/// How long a secret is kept in memory, or where.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Keep {
    /// Not kept: it is given to the one request that has it.
    Once,
    /// Until the session that asked for it ends.
    Session,
    /// Until Canoa closes.
    Close,
    /// For this many seconds.
    Ttl(u64),
    /// In the store entry the settings name, for good.
    Store,
}

impl Keep {
    /// Reads the id a form offers or a table gives; `None` for a word that is none of them.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        Some(match text.trim() {
            "once" => Self::Once,
            "session" => Self::Session,
            "close" => Self::Close,
            "store" => Self::Store,
            "15m" => Self::Ttl(15 * 60),
            "1h" => Self::Ttl(60 * 60),
            "8h" => Self::Ttl(8 * 60 * 60),
            "1d" => Self::Ttl(24 * 60 * 60),
            other => Self::Ttl(
                other
                    .strip_prefix("ttl:")?
                    .parse::<u64>()
                    .ok()
                    .filter(|seconds| *seconds > 0)?,
            ),
        })
    }

    /// The id a form offers it by.
    pub(crate) fn id(self) -> String {
        match self {
            Self::Once => "once".to_owned(),
            Self::Session => "session".to_owned(),
            Self::Close => "close".to_owned(),
            Self::Store => "store".to_owned(),
            Self::Ttl(900) => "15m".to_owned(),
            Self::Ttl(3600) => "1h".to_owned(),
            Self::Ttl(28800) => "8h".to_owned(),
            Self::Ttl(86400) => "1d".to_owned(),
            Self::Ttl(seconds) => format!("ttl:{seconds}"),
        }
    }

    /// What the form calls it.
    pub(crate) fn label(self) -> String {
        match self {
            Self::Once => "This once".to_owned(),
            Self::Session => "This session".to_owned(),
            Self::Close => "Until Canoa closes".to_owned(),
            Self::Store => "Remember".to_owned(),
            Self::Ttl(900) => "15 minutes".to_owned(),
            Self::Ttl(3600) => "1 hour".to_owned(),
            Self::Ttl(28800) => "8 hours".to_owned(),
            Self::Ttl(86400) => "1 day".to_owned(),
            Self::Ttl(seconds) => format!("{seconds} seconds"),
        }
    }
}

/// A secret another entry gives, which a setting of a provider is made of.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Taken {
    /// The entry that gives it.
    pub(crate) entry: String,
    /// The secret's name.
    pub(crate) name: String,
}

/// One provider the person set up, or one the host has without being told.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Entry {
    /// The id of the `secret_provider`.
    pub(crate) provider: String,
    /// How long a value it gave is kept in memory.
    pub(crate) keep: Keep,
    /// The provider's own settings: plain data, never a secret.
    pub(crate) config: Map<String, Value>,
    /// The settings of the provider that are another secret (a field its schema marks `x-from-secret`), by the setting's name.
    pub(crate) from: BTreeMap<String, Taken>,
}

/// What `[secrets]` says.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Settings {
    /// The entry that answers a name no line of `names` gives.
    pub(crate) default: String,
    /// The entry "remember" keeps a secret in, when there is one.
    pub(crate) store: Option<String>,
    /// What the form offers first.
    pub(crate) first: Keep,
    /// The most seconds a request waits for the person, whatever the agent asks for.
    pub(crate) max_wait: u64,
    /// The entry that answers each name given a line.
    pub(crate) names: BTreeMap<String, String>,
    /// The entries, `form`, `system` and `file` included.
    pub(crate) entries: BTreeMap<String, Entry>,
    /// Who keeps the accounts' credentials and the mail accounts' passwords: `auto`, or an entry that keeps what it is given.
    pub(crate) accounts: String,
}

impl Default for Settings {
    fn default() -> Self {
        defaults()
    }
}

fn defaults() -> Settings {
    Settings {
        default: FORM.to_owned(),
        store: Some(SYSTEM.to_owned()),
        first: Keep::Session,
        max_wait: DEFAULT_WAIT,
        names: BTreeMap::new(),
        entries: implicit(),
        accounts: AUTO.to_owned(),
    }
}

fn implicit() -> BTreeMap<String, Entry> {
    [FORM, SYSTEM, FILE]
        .into_iter()
        .map(|id| {
            (
                id.to_owned(),
                Entry {
                    provider: id.to_owned(),
                    keep: Keep::Session,
                    config: Map::new(),
                    from: BTreeMap::new(),
                },
            )
        })
        .collect()
}

/// What `[secrets] accounts` says when the person names nobody: the operating system's store when it answers, else sealed files.
pub(crate) const AUTO: &str = "auto";

/// How many seconds a request waits for the person unless `[secrets] max_wait` says otherwise.
pub(crate) const DEFAULT_WAIT: u64 = 240;

/// The keys of an entry's table that are the host's and not the provider's own settings.
const HOST_KEYS: [&str; 3] = ["provider", "keep", "from"];

/// The longest name an entry or a secret may be given.
const MAX_ENTRY: usize = 64;

/// Guatiao-derived schema for `[secrets]` in `canoa.cfg`.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Default, PartialEq)]
#[map(crate = ::coqui_acp_api::guatiao, deny_unknown_fields)]
struct RawSection {
    /// Entry that answers a secret name when `[secrets.names]` does not override it.
    default: Option<String>,
    /// Entry `"remember"` stores a typed secret in, or `"none"`.
    store: Option<String>,
    /// Default lifetime option offered by the prompt form (`once`, `session`, `close`, `15m`, `1h`, `8h`, `1d`, `ttl:SECONDS`).
    keep: Option<String>,
    /// Entry keeping account credentials and mail passwords (`auto` or an entry name).
    accounts: Option<String>,
    /// Most seconds a prompt waits for the person (at least 1).
    max_wait: Option<u64>,
    /// Per-secret routing from secret name to provider entry name.
    #[map(default)]
    names: BTreeMap<String, String>,
    /// Configured `secret_provider` entries keyed by entry name.
    #[map(default)]
    providers: BTreeMap<String, GuatiaoValue>,
}

/// Guatiao-derived reference from one provider setting to another secret entry.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, deny_unknown_fields)]
struct RawTaken {
    /// Entry that gives the secret.
    entry: String,
    /// Secret name requested from that entry.
    name: String,
}

/// Host-owned keys decoded from `[secrets.providers.<name>]`.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq)]
#[map(crate = ::coqui_acp_api::guatiao)]
struct RawEntryFields {
    /// Id of the `secret_provider`.
    provider: String,
    /// How long values from this entry are kept in memory.
    keep: Option<String>,
    /// Settings populated from another secret entry rather than literals.
    #[map(default)]
    from: BTreeMap<String, RawTaken>,
}

impl Settings {
    /// The guatiao schema describing `[secrets]` in `canoa.cfg`.
    #[cfg(test)]
    pub(crate) fn schema() -> Result<GuatiaoValue, ValueError> {
        <RawSection as Schema>::schema()
    }

    /// Reads the `[secrets]` section as JSON, or the defaults for `None`.
    ///
    /// # Errors
    ///
    /// What is wrong with the section, naming the key, so nothing is guessed at.
    pub(crate) fn read(section: Option<&Value>) -> Result<Self, String> {
        let mut settings = defaults();
        let Some(section) = section.filter(|section| !section.is_null()) else {
            return Ok(settings);
        };
        if !section.is_object() {
            return Err("[secrets] is a table".to_owned());
        }
        let gval = guatiao_serde::from_serde(section.clone())
            .map_err(|error| format!("[secrets] {error}"))?;
        let raw = RawSection::from_value(&gval).map_err(|error| format!("[secrets] {error}"))?;
        if let Some(default_entry) = raw.default {
            settings.default = non_empty(default_entry, "default")?;
        }
        if let Some(store) = raw.store {
            let store = non_empty(store, "store")?;
            settings.store = (store != "none").then_some(store);
        }
        if let Some(keep) = raw.keep {
            settings.first = Keep::parse(&non_empty(keep, "keep")?)
                .filter(|keep| *keep != Keep::Store)
                .ok_or("[secrets] keep is once, session, close, 15m, 1h, 8h, 1d or ttl:SECONDS")?;
        }
        if let Some(accounts) = raw.accounts {
            settings.accounts = non_empty(accounts, "accounts")?;
        }
        if let Some(max_wait) = raw.max_wait {
            if max_wait == 0 {
                return Err(
                    "[secrets] max_wait is a whole number of seconds, at least 1".to_owned(),
                );
            }
            settings.max_wait = max_wait;
        }
        for (name, target) in raw.names {
            if let Some(why) = canoa::secret_provider::name_problem(&name) {
                return Err(format!("[secrets.names] {name}: {why}"));
            }
            settings
                .names
                .insert(name.clone(), non_empty(target, &format!("names.{name}"))?);
        }
        for (name, table) in raw.providers {
            settings.entries.insert(name.clone(), entry(&name, &table)?);
        }
        for (what, name) in settings
            .names
            .iter()
            .map(|(name, entry)| (format!("[secrets.names] {name}"), entry))
            .chain(std::iter::once((
                "[secrets] default".to_owned(),
                &settings.default,
            )))
            .chain(
                settings
                    .store
                    .iter()
                    .map(|entry| ("[secrets] store".to_owned(), entry)),
            )
        {
            if !settings.entries.contains_key(name) {
                return Err(format!(
                    "{what} names {name}, which is no provider of [secrets.providers]"
                ));
            }
        }
        if settings.accounts != AUTO && !settings.entries.contains_key(&settings.accounts) {
            return Err(format!(
                "[secrets] accounts names {}, which is no provider of [secrets.providers]",
                settings.accounts
            ));
        }
        for (entry_name, entry) in &settings.entries {
            for (setting, taken) in &entry.from {
                if !settings.entries.contains_key(&taken.entry) {
                    return Err(format!(
                        "[secrets.providers.{entry_name}.from] {setting} names {}, which is no provider of [secrets.providers]",
                        taken.entry
                    ));
                }
            }
        }
        Ok(settings)
    }

    /// The entry that answers `name`.
    pub(crate) fn entry_for(&self, name: &str) -> &str {
        self.names.get(name).unwrap_or(&self.default)
    }
}

fn non_empty(text: String, key: &str) -> Result<String, String> {
    if text.is_empty() {
        Err(format!("[secrets] {key} is text"))
    } else {
        Ok(text)
    }
}

fn entry(name: &str, value: &GuatiaoValue) -> Result<Entry, String> {
    if name.is_empty()
        || name.len() > MAX_ENTRY
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return Err(format!(
            "[secrets.providers.{name}] is not 1 to {MAX_ENTRY} letters, digits, - and _"
        ));
    }
    let json_val = guatiao_serde::adapt::from_value::<Value>(value)
        .map_err(|error| format!("[secrets.providers.{name}] {error}"))?;
    let Some(obj) = json_val.as_object() else {
        return Err(format!("[secrets.providers.{name}] is a table"));
    };
    if !obj.contains_key("provider") {
        return Err(format!("[secrets.providers.{name}] names no provider"));
    }
    let parsed = RawEntryFields::from_value(value).map_err(|error| {
        let msg = error.to_string();
        if msg.contains("keep") {
            format!("[secrets.providers.{name}] keep is once, session, close, 15m, 1h, 8h, 1d or ttl:SECONDS")
        } else if msg.contains("from") {
            format!("[secrets.providers.{name}.from] is a table of an entry and a name")
        } else {
            format!("[secrets.providers.{name}] {msg}")
        }
    })?;
    let keep = match parsed.keep.as_deref() {
        None => Keep::Session,
        Some(text) => Keep::parse(text)
            .filter(|keep| *keep != Keep::Store)
            .ok_or_else(|| {
                format!(
                    "[secrets.providers.{name}] keep is once, session, close, 15m, 1h, 8h, 1d or ttl:SECONDS"
                )
            })?,
    };
    let config: Map<String, Value> = obj
        .iter()
        .filter(|(key, _)| !HOST_KEYS.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let mut from = BTreeMap::new();
    for (setting, taken) in parsed.from {
        if config.contains_key(&setting) {
            return Err(format!(
                "[secrets.providers.{name}] gives {setting} twice: as a setting and in from"
            ));
        }
        if taken.entry.is_empty() || taken.name.is_empty() {
            return Err(format!(
                "[secrets.providers.{name}.from] {setting} is a table of an entry and a name"
            ));
        }
        if let Some(why) = canoa::secret_provider::name_problem(&taken.name) {
            return Err(format!("[secrets.providers.{name}.from] {setting}: {why}"));
        }
        from.insert(
            setting,
            Taken {
                entry: taken.entry,
                name: taken.name,
            },
        );
    }
    Ok(Entry {
        provider: parsed.provider,
        keep,
        config,
        from,
    })
}

/// Where `[secrets]` is read from: the section of the canoa.cfg at `path`, as it is each time it is asked for, or nothing when there is
/// no file or no section. A file that cannot be read or parsed is an error the caller reports, and the defaults then answer.
pub(crate) fn source_of(path: Option<std::path::PathBuf>) -> super::vault::Source {
    std::sync::Arc::new(move || {
        let Some(path) = &path else {
            return Ok(None);
        };
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("canoa.cfg cannot be read: {error}")),
        };
        let table: toml::Table = text.parse().map_err(|error: toml::de::Error| {
            format!("canoa.cfg cannot be parsed: {}", error.message())
        })?;
        table
            .get("secrets")
            .map(|section| serde_json::to_value(section).map_err(|error| error.to_string()))
            .transpose()
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn with_no_section_the_form_answers_and_the_system_store_is_there_to_remember_in() {
        let settings = Settings::read(None).unwrap();
        assert_eq!(settings.default, "form");
        assert_eq!(settings.store.as_deref(), Some("system"));
        assert!(settings.entries.contains_key("form") && settings.entries.contains_key("system"));
        assert_eq!(settings.entry_for("anything"), "form");
    }

    #[test]
    fn names_providers_and_keeps_are_read_and_a_setting_that_names_another_secret_is_kept_as_written()
     {
        let settings = Settings::read(Some(&json!({
            "default": "form",
            "store": "none",
            "keep": "1h",
            "names": {"github_token": "vault"},
            "providers": {"vault": {
                "provider": "bitwarden", "keep": "ttl:90", "command": "bw",
                "session": {"from": "form", "name": "bw_session"}
            }}
        })))
        .unwrap();
        assert_eq!(settings.store, None);
        assert_eq!(settings.first, Keep::Ttl(3600));
        assert_eq!(settings.entry_for("github_token"), "vault");
        let vault = &settings.entries["vault"];
        assert_eq!(vault.provider, "bitwarden");
        assert_eq!(vault.keep, Keep::Ttl(90));
        assert_eq!(vault.config["command"], "bw");
        assert_eq!(vault.config["session"]["from"], "form");
        assert!(!vault.config.contains_key("provider") && !vault.config.contains_key("keep"));
    }

    #[test]
    fn what_cannot_be_read_is_refused_by_name_and_nothing_is_guessed() {
        for (section, why) in [
            (json!({"surprise": 1}), "'surprise' is not a declared field"),
            (json!({"keep": "forever"}), "keep is"),
            (json!({"keep": "store"}), "keep is"),
            (json!({"default": "ghost"}), "no provider"),
            (json!({"names": {"x": "ghost"}}), "ghost"),
            (json!({"providers": {"a b": {"provider": "x"}}}), "letters"),
            (json!({"providers": {"v": {}}}), "names no provider"),
            (
                json!({"providers": {"v": {"provider": "x", "keep": "ttl:0"}}}),
                "keep is",
            ),
            (json!({"names": {"": "form"}}), "name"),
        ] {
            let error = Settings::read(Some(&section)).expect_err("refused");
            assert!(error.contains(why), "{section}: {error}");
        }
    }

    #[test]
    fn a_keep_is_the_id_the_form_offers_and_back() {
        for keep in [
            Keep::Once,
            Keep::Session,
            Keep::Close,
            Keep::Store,
            Keep::Ttl(900),
            Keep::Ttl(86400),
            Keep::Ttl(77),
        ] {
            assert_eq!(Keep::parse(&keep.id()), Some(keep), "{keep:?}");
        }
        assert_eq!(Keep::parse("ttl:abc"), None);
    }

    #[test]
    fn secrets_and_host_store_sections_expose_closed_guatiao_schemas() {
        let secrets_schema =
            guatiao_serde::adapt::from_value::<Value>(&Settings::schema().expect("schema"))
                .expect("json");
        assert_eq!(secrets_schema["additionalProperties"], false);
        for field in [
            "default",
            "store",
            "keep",
            "accounts",
            "max_wait",
            "names",
            "providers",
        ] {
            assert!(
                secrets_schema["properties"].get(field).is_some(),
                "missing field {field}"
            );
        }

        let host_store_schema = guatiao_serde::adapt::from_value::<Value>(
            &crate::config::StoreConfig::schema().expect("schema"),
        )
        .expect("json");
        assert_eq!(host_store_schema["additionalProperties"], false);
        for field in [
            "protected",
            "credential_variables",
            "index_ttl",
            "index_cache_bytes",
            "latest_ttl",
            "models_ttl",
            "catalog_ttl",
            "refresh_every",
            "connect_timeout",
            "lookup_timeout",
            "tools",
            "verify_copies",
        ] {
            assert!(
                host_store_schema["properties"].get(field).is_some(),
                "missing field {field}"
            );
        }
    }
}
