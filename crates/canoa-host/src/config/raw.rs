//! Serde deserialization shapes for `canoa.cfg` tables and direct subsection conversions.

use alloc::string::String;
use alloc::vec::Vec;

use coqui_peer_webcrypto::PeerId;
use indexmap::IndexMap;
use serde::Deserialize;

use super::entries_parse::TransportSpec;
use super::{
    AssistChoice, AssistConfig, AssistPool, ConfigError, DEFAULT_CHOICE, DIRECT_ROUTE,
    DelegateConfig, HarnessTable, KbConfig, KbEmbedder, KbRanker, KbSearch, NetworkRoutes,
    OpenerConfig, RouteConfig, SYSTEM_ROUTE, SdkTable,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawDelegate {
    pub(super) enabled: Option<bool>,
    pub(super) sources: Option<Vec<String>>,
    pub(super) max_children: Option<usize>,
    pub(super) max_running: Option<usize>,
    pub(super) max_depth: Option<usize>,
    pub(super) a2a: Option<Vec<String>>,
    pub(super) task_bytes: Option<usize>,
    pub(super) result_bytes: Option<usize>,
    pub(super) history_bytes: Option<usize>,
    pub(super) context_bytes: Option<usize>,
    pub(super) wait_seconds: Option<u64>,
    pub(super) dial_seconds: Option<u64>,
}

impl RawDelegate {
    pub(super) fn into_config(self) -> Result<DelegateConfig, ConfigError> {
        let defaults = DelegateConfig::default();
        if let Some(sources) = &self.sources
            && sources.iter().any(|name| name.trim().is_empty())
        {
            return Err(ConfigError::Invalid {
                entry: None,
                what: "host.delegate: sources holds an empty name".to_owned(),
            });
        }
        let a2a = self
            .a2a
            .unwrap_or_default()
            .iter()
            .map(|peer| peer.parse::<PeerId>())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| ConfigError::Invalid {
                entry: None,
                what: "host.delegate: a2a holds something that is not a peer id".to_owned(),
            })?;
        Ok(DelegateConfig {
            enabled: self.enabled.unwrap_or(defaults.enabled),
            sources: self.sources,
            max_children: self.max_children.unwrap_or(defaults.max_children),
            max_running: self.max_running.unwrap_or(defaults.max_running),
            max_depth: self.max_depth.unwrap_or(defaults.max_depth),
            a2a,
            task_bytes: self.task_bytes.unwrap_or(defaults.task_bytes),
            result_bytes: self.result_bytes.unwrap_or(defaults.result_bytes),
            history_bytes: self.history_bytes.unwrap_or(defaults.history_bytes),
            context_bytes: self.context_bytes.unwrap_or(defaults.context_bytes),
            wait_seconds: self.wait_seconds.unwrap_or(defaults.wait_seconds),
            dial_seconds: self.dial_seconds.unwrap_or(defaults.dial_seconds),
        })
    }
}

/// A list of lines as canoa.cfg writes it: one string, a line a line, or a list of strings.
#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum RawLines {
    One(String),
    Many(Vec<String>),
}

impl RawLines {
    pub(super) fn lines(self) -> Vec<String> {
        match self {
            Self::One(text) => text.lines().map(str::to_owned).collect(),
            Self::Many(lines) => lines,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawStore {
    pub(super) protected: Option<RawLines>,
    pub(super) credential_variables: Option<RawLines>,
    pub(super) skill_folders: Option<RawLines>,
    pub(super) index_ttl: Option<u64>,
    pub(super) index_cache_bytes: Option<u64>,
    pub(super) latest_ttl: Option<u64>,
    pub(super) models_ttl: Option<u64>,
    pub(super) catalog_ttl: Option<u64>,
    pub(super) refresh_every: Option<u64>,
    pub(super) connect_timeout: Option<u64>,
    pub(super) lookup_timeout: Option<u64>,
    pub(super) tools: Option<bool>,
    pub(super) verify_copies: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawContent {
    pub(super) ttl: Option<u64>,
    pub(super) document_bytes: Option<u64>,
    pub(super) cache_bytes: Option<u64>,
}

#[derive(Deserialize)]
pub(super) struct RawFile {
    #[serde(default)]
    pub(super) host: Option<RawHost>,
    #[serde(default)]
    pub(super) usage: Option<RawUsage>,
    /// `[permissions]`: the app's table, of which the host reads `protected` and lets the rest go by.
    #[serde(default)]
    pub(super) permissions: Option<RawPermissions>,
    /// `[settings.ID]`: the tables of Settings' sections kept in canoa.cfg, of which the host reads `power`.
    #[serde(default)]
    pub(super) settings: Option<RawSettings>,
    /// `[agents.NAME]`: the app's table, of which the host reads `program` and `args` and lets the rest go by.
    #[serde(default)]
    pub(super) agents: IndexMap<String, toml::Value>,
    /// `[profile]`: the person's own layer.
    #[serde(default)]
    pub(super) profile: Option<toml::Table>,
    /// `[overlays]`: the app's table, of which the host reads the names.
    #[serde(default)]
    pub(super) overlays: IndexMap<String, toml::Value>,
    #[serde(default)]
    pub(super) skill_pins: toml::Table,
    /// `[project_trust]`: the project profiles the person trusts, by directory, each with the SHA-256 it was trusted at.
    #[serde(default)]
    pub(super) project_trust: IndexMap<String, String>,
    /// `[loadouts.NAME]`: the named collections a session starts with.
    #[serde(default)]
    pub(super) loadouts: IndexMap<String, toml::Value>,
    /// `[defaults]`: the app's table, of which the host reads `loadout`.
    #[serde(default)]
    pub(super) defaults: Option<toml::Table>,
    /// `[profiles.NAME]`: the named presets of a session's own values.
    #[serde(default)]
    pub(super) profiles: IndexMap<String, toml::Value>,
    /// `[secrets]`: the secret providers' table, which they read; the host keeps it for who keeps the accounts' credentials.
    #[serde(default)]
    pub(super) secrets: Option<toml::Table>,
    /// `[assist.OPERATION]`: which backend serves each operation.
    #[serde(default)]
    pub(super) assist: IndexMap<String, toml::Value>,
    /// `[sdk.NAME]`: the offers the person makes by address.
    #[serde(default)]
    pub(super) sdk: IndexMap<String, toml::Value>,
    /// `[assist_harness.NAME]`: the harnesses the person offers as a backend.
    #[serde(default)]
    pub(super) assist_harness: IndexMap<String, RawAssistHarness>,
    /// `[assist_pool]`: the warm sessions the harness backends keep.
    #[serde(default)]
    pub(super) assist_pool: Option<RawAssistPool>,
    /// `[notifications]`: the notifiers of each event, and the notifier entries.
    #[serde(default)]
    pub(super) notifications: Option<RawNotifications>,
    /// `[network]`: how the host reaches the network, which also holds the application's own `prefer_webtransport` and
    /// `connect_timeout`.
    #[serde(default)]
    pub(super) network: Option<RawNetwork>,
}

/// `[notifications]`: `notifiers`, and every other key is an event's list of notifier names, which the reader checks against the
/// host's catalog of events.
#[derive(Deserialize)]
pub(super) struct RawNotifications {
    #[serde(default)]
    pub(super) notifiers: IndexMap<String, RawNotifier>,
    #[serde(flatten)]
    pub(super) rest: toml::Table,
}

/// `[notifications.notifiers.NAME]`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawNotifier {
    pub(super) provider: Option<String>,
    pub(super) library: Option<String>,
    pub(super) source: Option<String>,
    pub(super) source_settings: Option<toml::Table>,
    pub(super) import_search: Option<String>,
    #[serde(default)]
    pub(super) settings: Option<toml::Table>,
}

/// `[settings]`: each section's table is the section's; the host reads the Power group's.
#[derive(Deserialize)]
pub(super) struct RawSettings {
    #[serde(default)]
    pub(super) power: Option<RawPower>,
}

/// `[settings.power]`.
#[derive(Deserialize)]
pub(super) struct RawPower {
    pub(super) keep_awake: Option<bool>,
}

/// `[permissions]`: the app reads the modes and the rules, so only the key the host passes on is named.
#[derive(Deserialize)]
pub(super) struct RawPermissions {
    #[serde(default)]
    pub(super) protected: Option<RawLines>,
}

/// `[usage]`: the app reads `warn_permille`, so only the key the host acts on is named and the others are let go by.
#[derive(Deserialize)]
pub(super) struct RawUsage {
    #[serde(default)]
    pub(super) limit_phrases: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawHost {
    #[serde(default)]
    pub(super) origins: Vec<String>,
    #[serde(default)]
    pub(super) agents: IndexMap<String, RawAgent>,
    #[serde(default)]
    pub(super) providers: IndexMap<String, RawProvider>,
    #[serde(default)]
    pub(super) tools: IndexMap<String, toml::Table>,
    #[serde(default)]
    pub(super) extensions: IndexMap<String, RawExtension>,
    #[serde(default)]
    pub(super) profiles: IndexMap<String, toml::Table>,
    #[serde(default)]
    pub(super) accounts: IndexMap<String, RawAccount>,
    #[serde(default)]
    pub(super) pools: IndexMap<String, RawPool>,
    pub(super) shell: Option<String>,
    pub(super) openers: Option<Vec<RawOpener>>,
    pub(super) max_message_bytes: Option<usize>,
    pub(super) web: Option<RawWeb>,
    pub(super) control: Option<RawControl>,
    pub(super) native: Option<RawNative>,
    pub(super) history: Option<RawHistory>,
    pub(super) worktrees: Option<RawWorktrees>,
    pub(super) usage: Option<RawHostUsage>,
    pub(super) store: Option<RawStore>,
    pub(super) content: Option<RawContent>,
    /// `[host.disabled]`: provider ids by kind name that are not offered or run.
    #[serde(default)]
    pub(super) disabled: IndexMap<String, Vec<String>>,
    /// `[host.network]` moved to `[network]`; it is read only to be refused by name.
    pub(super) network: Option<toml::Value>,
    pub(super) delegate: Option<RawDelegate>,
    pub(super) kb: Option<RawKb>,
    /// `[host.services.ID]`: the table of each session service, which the service reads.
    #[serde(default)]
    pub(super) services: IndexMap<String, toml::Table>,
    pub(super) mcp_list: Option<RawMcpList>,
    #[serde(default)]
    pub(super) environments: IndexMap<String, RawEnvironment>,
}

/// `[host.environments.NAME]`: `kind`, and every other key is the provider's own setting.
#[derive(Deserialize)]
pub(super) struct RawEnvironment {
    pub(super) kind: Option<String>,
    pub(super) library: Option<String>,
    pub(super) source: Option<String>,
    pub(super) source_settings: Option<toml::Table>,
    pub(super) import_search: Option<String>,
    pub(super) exec_seconds: Option<u64>,
    pub(super) output_bytes: Option<u64>,
    pub(super) scrollback_bytes: Option<u64>,
    pub(super) max_tool_sessions: Option<u64>,
    pub(super) max_prepared: Option<u64>,
    pub(super) prepared_seconds: Option<u64>,
    pub(super) hook_read_ms: Option<u64>,
    pub(super) hook_handlers: Option<u64>,
    pub(super) hook_seconds: Option<u64>,
    #[serde(flatten)]
    pub(super) settings: toml::Table,
}

/// `[host.mcp_list]`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawMcpList {
    pub(super) query_seconds: Option<u64>,
    pub(super) max_items_per_server: Option<usize>,
    pub(super) max_reply_bytes: Option<usize>,
}

/// `[network]`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawNetwork {
    pub(super) route: Option<String>,
    #[serde(default)]
    pub(super) routes: IndexMap<String, RawRoute>,
    #[serde(default)]
    pub(super) endpoints: Vec<String>,
    pub(super) idle_seconds: Option<u64>,
    pub(super) open_seconds: Option<u64>,
    pub(super) max_leases: Option<usize>,
    pub(super) prefer_webtransport: Option<bool>,
    pub(super) connect_timeout: Option<u64>,
}

/// `[network.routes.NAME]`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawRoute {
    pub(super) provider: Option<String>,
    pub(super) settings: Option<toml::Table>,
}

/// `[network]`. Errors name a key, never a value.
pub(super) fn network_of(raw: RawNetwork) -> Result<NetworkRoutes, ConfigError> {
    let refused = |what: &str| ConfigError::Invalid {
        entry: None,
        what: format!("network: {what}"),
    };
    let named = |name: &str, key: &str| {
        if name.trim().is_empty() {
            return Err(refused(&format!("{key} is empty")));
        }
        Ok(())
    };
    if let Some(route) = &raw.route {
        named(route, "route")?;
    }
    if raw.open_seconds == Some(0) {
        return Err(refused("open_seconds is at least 1"));
    }
    if raw.max_leases == Some(0) {
        return Err(refused("max_leases is at least 1"));
    }
    let mut routes = IndexMap::new();
    for (name, route) in raw.routes {
        if let Some(why) = canoa::config_store::id_problem(&name) {
            return Err(refused(&format!("route {name}: {why}")));
        }
        if name == SYSTEM_ROUTE || name == DIRECT_ROUTE {
            return Err(refused(&format!(
                "{name} is a route every host has: it needs no table, and its name is not given to another"
            )));
        }
        let provider = route
            .provider
            .filter(|provider| !provider.trim().is_empty())
            .ok_or_else(|| refused(&format!("route {name} names no provider")))?;
        let settings = serde_json::to_value(route.settings.unwrap_or_default())
            .map_err(|_| refused(&format!("route {name}: its settings cannot be read")))?;
        routes.insert(name, RouteConfig { provider, settings });
    }
    if let Some(route) = &raw.route
        && route != SYSTEM_ROUTE
        && route != DIRECT_ROUTE
        && !routes.contains_key(route)
    {
        return Err(refused(&format!(
            "route names {route}, which [network.routes] does not hold"
        )));
    }
    Ok(NetworkRoutes {
        route: raw.route,
        routes,
        endpoints: raw.endpoints,
        idle_seconds: raw.idle_seconds,
        open_seconds: raw.open_seconds,
        max_leases: raw.max_leases,
        base_dir: None,
        prefer_webtransport: raw.prefer_webtransport,
        connect_timeout: raw.connect_timeout,
    })
}

/// `[host.kb]`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawKb {
    pub(super) tools: Option<bool>,
    pub(super) chunk_bytes: Option<usize>,
    pub(super) overlap_bytes: Option<usize>,
    pub(super) retry_seconds: Option<u64>,
    pub(super) query_cache: Option<usize>,
    pub(super) embedder: Option<RawKbEmbedder>,
    pub(super) ranker: Option<RawKbRanker>,
    pub(super) search: Option<RawKbSearch>,
}

/// `[host.kb.embedder]`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawKbEmbedder {
    pub(super) r#use: Option<String>,
    pub(super) model: Option<String>,
    pub(super) dimensions: Option<u32>,
    pub(super) document_prefix: Option<String>,
    pub(super) query_prefix: Option<String>,
}

/// `[host.kb.ranker]`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawKbRanker {
    pub(super) r#use: Option<String>,
    pub(super) model: Option<String>,
    pub(super) candidates: Option<usize>,
    pub(super) timeout_seconds: Option<u64>,
    pub(super) passage_chars: Option<usize>,
}

/// `[host.kb.search]`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawKbSearch {
    pub(super) r#use: Option<String>,
}

impl RawKb {
    pub(super) fn into_config(self) -> Result<KbConfig, ConfigError> {
        let invalid = |what: &str| ConfigError::Invalid {
            entry: None,
            what: format!("[host.kb] {what}"),
        };
        let choice = |text: Option<String>, table: &str| -> Result<String, ConfigError> {
            match text {
                None => Ok(DEFAULT_CHOICE.to_owned()),
                Some(text) if text.trim().is_empty() => {
                    Err(invalid(&format!("{table} use is empty")))
                }
                Some(text) => Ok(text),
            }
        };
        let embedder = self
            .embedder
            .map(|raw| -> Result<KbEmbedder, ConfigError> {
                let model = raw
                    .model
                    .filter(|model| !model.trim().is_empty())
                    .ok_or_else(|| invalid("embedder needs the model that makes the vectors"))?;
                if raw.dimensions == Some(0) {
                    return Err(invalid("embedder dimensions is at least 1"));
                }
                Ok(KbEmbedder {
                    choice: choice(raw.r#use, "embedder")?,
                    model,
                    dimensions: raw.dimensions,
                    document_prefix: raw.document_prefix,
                    query_prefix: raw.query_prefix,
                })
            })
            .transpose()?;
        let ranker = self
            .ranker
            .map(|raw| -> Result<KbRanker, ConfigError> {
                let model = raw
                    .model
                    .filter(|model| !model.trim().is_empty())
                    .ok_or_else(|| invalid("ranker needs the model that orders the passages"))?;
                if raw
                    .candidates
                    .is_some_and(|count| !(1..=100).contains(&count))
                {
                    return Err(invalid("ranker candidates is 1 to 100"));
                }
                if raw.timeout_seconds == Some(0) {
                    return Err(invalid("ranker timeout_seconds is at least 1"));
                }
                if raw.passage_chars == Some(0) {
                    return Err(invalid("ranker passage_chars is at least 1"));
                }
                Ok(KbRanker {
                    choice: choice(raw.r#use, "ranker")?,
                    model,
                    candidates: raw.candidates,
                    timeout_seconds: raw.timeout_seconds,
                    passage_chars: raw.passage_chars,
                })
            })
            .transpose()?;
        let search = self
            .search
            .map(|raw| -> Result<KbSearch, ConfigError> {
                Ok(KbSearch {
                    choice: choice(raw.r#use, "search")?,
                })
            })
            .transpose()?;
        let chunk = self.chunk_bytes.unwrap_or(1024);
        if !(4..=65536).contains(&chunk) {
            return Err(invalid("chunk_bytes is 4 to 65536"));
        }
        let prefix = embedder
            .as_ref()
            .and_then(|each| each.document_prefix.as_deref())
            .map_or(0, str::len);
        if chunk.saturating_sub(prefix) < 4
            || self.overlap_bytes.unwrap_or(0) >= chunk - prefix.min(chunk)
        {
            return Err(invalid(
                "overlap_bytes is below the room a passage has after its document prefix",
            ));
        }
        Ok(KbConfig {
            tools: self.tools,
            chunk_bytes: self.chunk_bytes,
            overlap_bytes: self.overlap_bytes,
            retry_seconds: self.retry_seconds,
            query_cache: self.query_cache,
            embedder,
            ranker,
            search,
        })
    }
}

/// `[assist.OPERATION]`: which backend serves an operation, and which come after it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawAssist {
    pub(super) r#use: Option<String>,
    #[serde(default)]
    pub(super) fallback: Vec<String>,
}

/// `[assist_harness.NAME]`: a harness on a profile, offered as a backend.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawAssistHarness {
    pub(super) source: Option<String>,
    pub(super) title: Option<String>,
    pub(super) profile: Option<String>,
    pub(super) account: Option<String>,
    pub(super) model: Option<String>,
    pub(super) effort: Option<String>,
}

/// `[assist_pool]`: the warm sessions the harness backends keep.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawAssistPool {
    pub(super) idle_seconds: Option<u64>,
    pub(super) max_sessions: Option<usize>,
}

/// The `[assist.*]`, `[sdk.*]`, `[assist_harness.*]` and `[assist_pool]` tables, read from the top level of the file.
pub(super) fn assist_of(
    assist: &IndexMap<String, toml::Value>,
    sdk: &IndexMap<String, toml::Value>,
    harnesses: &IndexMap<String, RawAssistHarness>,
    pool: Option<&RawAssistPool>,
) -> Result<AssistConfig, ConfigError> {
    let invalid = |what: String| ConfigError::Invalid { entry: None, what };
    let word = |text: &str, extra: &[char]| {
        (1..=32).contains(&text.len())
            && text
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || extra.contains(&c))
    };
    let mut out = AssistConfig::default();
    if let Some(pool) = pool {
        if pool.idle_seconds == Some(0) {
            return Err(invalid(
                "[assist_pool] idle_seconds is at least 1".to_owned(),
            ));
        }
        if pool.max_sessions == Some(0) {
            return Err(invalid(
                "[assist_pool] max_sessions is at least 1".to_owned(),
            ));
        }
        out.pool = AssistPool {
            idle_seconds: pool.idle_seconds,
            max_sessions: pool.max_sessions,
        };
    }
    for (name, raw) in harnesses {
        if !word(name, &['_', '-']) {
            return Err(invalid(format!(
                "[assist_harness.{name}] is not an offer name (lower-case letters, digits, _ and -)"
            )));
        }
        let text = |key: &str, value: &Option<String>| -> Result<Option<String>, ConfigError> {
            match value {
                None => Ok(None),
                Some(text) if !text.trim().is_empty() => Ok(Some(text.clone())),
                Some(_) => Err(invalid(format!(
                    "[assist_harness.{name}] {key} is a non-empty text"
                ))),
            }
        };
        let source = text("source", &raw.source)?.ok_or_else(|| {
            invalid(format!(
                "[assist_harness.{name}] names no source (the agent that runs it)"
            ))
        })?;
        out.harnesses.insert(
            name.clone(),
            HarnessTable {
                source,
                title: text("title", &raw.title)?,
                profile: text("profile", &raw.profile)?,
                account: text("account", &raw.account)?,
                model: text("model", &raw.model)?,
                effort: text("effort", &raw.effort)?,
            },
        );
    }
    for (operation, value) in assist {
        if !word(operation, &['_']) {
            return Err(invalid(format!(
                "[assist.{operation}] is not an operation name (lower-case letters, digits and _)"
            )));
        }
        let raw: RawAssist = value.clone().try_into().map_err(|error: toml::de::Error| {
            invalid(format!("[assist.{operation}] {}", error.message()))
        })?;
        if raw
            .r#use
            .as_deref()
            .is_some_and(|text| text.trim().is_empty())
            || raw.fallback.iter().any(|text| text.trim().is_empty())
        {
            return Err(invalid(format!(
                "[assist.{operation}] names an empty backend"
            )));
        }
        out.operations.insert(
            operation.clone(),
            AssistChoice {
                choice: raw.r#use,
                fallback: raw.fallback,
            },
        );
    }
    for (name, value) in sdk {
        if !word(name, &['_', '-']) {
            return Err(invalid(format!(
                "[sdk.{name}] is not an offer name (lower-case letters, digits, _ and -)"
            )));
        }
        let Some(table) = value.as_table() else {
            return Err(invalid(format!("[sdk.{name}] is not a table")));
        };
        let text = |key: &str| -> Result<Option<String>, ConfigError> {
            match table.get(key) {
                None => Ok(None),
                Some(toml::Value::String(text)) if !text.trim().is_empty() => {
                    Ok(Some(text.clone()))
                }
                Some(_) => Err(invalid(format!("[sdk.{name}] {key} is a non-empty text"))),
            }
        };
        let provider =
            text("provider")?.ok_or_else(|| invalid(format!("[sdk.{name}] names no provider")))?;
        let address = text("address")?;
        if let Some(address) = &address {
            let parsed = url::Url::parse(address)
                .map_err(|_| invalid(format!("[sdk.{name}] address is not an address")))?;
            if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
                return Err(invalid(format!(
                    "[sdk.{name}] address is an http or https address"
                )));
            }
        }
        let key_env = text("key_env")?;
        let mut settings = serde_json::Map::new();
        for (key, value) in table {
            if !matches!(key.as_str(), "provider" | "address" | "key_env") {
                settings.insert(
                    key.clone(),
                    serde_json::to_value(value)
                        .map_err(|_| invalid(format!("[sdk.{name}] {key} cannot be read")))?,
                );
            }
        }
        out.sdk.insert(
            name.clone(),
            SdkTable {
                provider,
                address,
                key_env,
                settings: serde_json::Value::Object(settings),
            },
        );
    }
    Ok(out)
}

/// `[host.native]`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawNative {
    pub(super) enabled: Option<bool>,
    pub(super) control: Option<Vec<String>>,
    pub(super) program: Option<String>,
    pub(super) args: Option<Vec<String>>,
}

/// `[host.usage]`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawHostUsage {
    pub(super) keep_days: Option<u32>,
}

/// `[host.worktrees]`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawWorktrees {
    pub(super) max_per_repository: Option<usize>,
    pub(super) git_seconds: Option<u64>,
}

/// `[host.history]`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawHistory {
    pub(super) record: Option<bool>,
    pub(super) max_session_bytes: Option<u64>,
    pub(super) max_sessions: Option<usize>,
    pub(super) max_total_bytes: Option<u64>,
    pub(super) store: Option<String>,
    pub(super) store_settings: Option<toml::Table>,
}

/// `[host.control]`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawControl {
    pub(super) listen: Option<String>,
    #[serde(default)]
    pub(super) grants: IndexMap<String, Vec<String>>,
    pub(super) max_records: Option<usize>,
    pub(super) max_record_bytes: Option<usize>,
    pub(super) max_upload_bytes: Option<u64>,
    pub(super) max_upload_total_bytes: Option<u64>,
    pub(super) max_uploads: Option<usize>,
}

/// `[host.accounts.NAME]`. A key it does not list is collected, so that it is refused by name and a credential written
/// there is never quoted.
#[derive(Deserialize)]
pub(super) struct RawAccount {
    pub(super) kind: Option<String>,
    pub(super) id: Option<String>,
    pub(super) settings: Option<toml::Table>,
    pub(super) route: Option<String>,
    #[serde(flatten)]
    pub(super) rest: toml::Table,
}

/// `[host.pools.NAME]`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawPool {
    pub(super) standard: Option<String>,
    #[serde(default)]
    pub(super) accounts: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawWeb {
    pub(super) dir: Option<String>,
    pub(super) listen: Option<String>,
    pub(super) cert: Option<String>,
    pub(super) key: Option<String>,
    #[serde(default)]
    pub(super) hosts: Vec<String>,
    #[serde(default)]
    pub(super) connect: Vec<String>,
    pub(super) mode: Option<String>,
    pub(super) office: Option<bool>,
    #[serde(default)]
    pub(super) office_frames: Vec<String>,
    pub(super) office_local: Option<bool>,
    pub(super) office_secret: Option<bool>,
    pub(super) max_connections: Option<usize>,
    pub(super) max_connections_per_address: Option<usize>,
    pub(super) connection_seconds: Option<u64>,
    pub(super) max_request_bytes: Option<usize>,
    pub(super) max_requests: Option<usize>,
    pub(super) request_seconds: Option<u64>,
    pub(super) idle_seconds: Option<u64>,
    pub(super) send_seconds: Option<u64>,
    pub(super) immutable_seconds: Option<u64>,
}

/// The settings both tables share. serde does not hold `deny_unknown_fields` across `flatten`, so each table lists them.
pub(super) struct RawCommon {
    pub(super) listen: Option<String>,
    pub(super) transport: Option<TransportSpec>,
    pub(super) working_roots: Vec<String>,
    pub(super) worktrees: Option<bool>,
    pub(super) peers: Option<Vec<String>>,
    pub(super) max_peers: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawAgent {
    pub(super) program: Option<String>,
    pub(super) args: Option<Vec<String>>,
    pub(super) harness: Option<String>,
    pub(super) library: Option<String>,
    pub(super) source: Option<String>,
    pub(super) source_settings: Option<toml::Table>,
    pub(super) import_search: Option<String>,
    pub(super) settings: Option<toml::Table>,
    pub(super) account_settings: Option<toml::Table>,
    pub(super) executable: Option<String>,
    pub(super) setting_sources: Option<Vec<String>>,
    pub(super) cache_lifetime: Option<String>,
    pub(super) shell: Option<String>,
    pub(super) profiles: Option<Vec<String>>,
    pub(super) accounts: Option<Vec<String>>,
    pub(super) usage: Option<String>,
    pub(super) usage_settings: Option<toml::Table>,
    pub(super) title: Option<String>,
    pub(super) shared: Option<bool>,
    pub(super) environment: Option<String>,
    pub(super) shared_idle_seconds: Option<u64>,
    pub(super) shared_permission_window_seconds: Option<u64>,
    pub(super) shared_keep_log: Option<bool>,
    pub(super) shared_log_entries: Option<usize>,
    pub(super) shared_log_bytes: Option<usize>,
    pub(super) listen: Option<String>,
    pub(super) transport: Option<String>,
    pub(super) transport_library: Option<String>,
    pub(super) transport_source: Option<String>,
    pub(super) transport_source_settings: Option<toml::Table>,
    pub(super) transport_import_search: Option<String>,
    pub(super) transport_settings: Option<toml::Table>,
    #[serde(default)]
    pub(super) working_roots: Vec<String>,
    pub(super) worktrees: Option<bool>,
    pub(super) peers: Option<Vec<String>>,
    pub(super) max_peers: Option<usize>,
}

/// A harness entry's keys for what its command line asks, taken out of [`RawAgent`].
pub(super) struct RawAsks {
    pub(super) setting_sources: Option<Vec<String>>,
}

/// `[host.extensions.NAME]`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawExtension {
    pub(super) library: Option<String>,
    pub(super) source: Option<String>,
    pub(super) source_settings: Option<toml::Table>,
    pub(super) import_search: Option<String>,
    pub(super) provider: String,
    pub(super) pin: Option<String>,
    pub(super) sha256: Option<String>,
    pub(super) root: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawProvider {
    pub(super) library: Option<String>,
    pub(super) source: Option<String>,
    pub(super) source_settings: Option<toml::Table>,
    pub(super) import_search: Option<String>,
    pub(super) settings: Option<toml::Table>,
    pub(super) provider: String,
    pub(super) max_sessions: Option<usize>,
    pub(super) listen: Option<String>,
    pub(super) transport: Option<String>,
    pub(super) transport_library: Option<String>,
    pub(super) transport_source: Option<String>,
    pub(super) transport_source_settings: Option<toml::Table>,
    pub(super) transport_import_search: Option<String>,
    pub(super) transport_settings: Option<toml::Table>,
    #[serde(default)]
    pub(super) working_roots: Vec<String>,
    pub(super) peers: Option<Vec<String>>,
    pub(super) max_peers: Option<usize>,
}

/// One `[[host.openers]]` entry.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawOpener {
    pub(super) id: String,
    pub(super) label: String,
    pub(super) program: String,
    #[serde(default = "default_opener_args")]
    pub(super) args: Vec<String>,
    pub(super) item: Option<bool>,
}

fn default_opener_args() -> Vec<String> {
    vec!["{path}".to_owned()]
}

pub(super) fn openers_of(
    raw: Option<Vec<RawOpener>>,
) -> Result<Option<Vec<OpenerConfig>>, ConfigError> {
    let Some(entries) = raw else {
        return Ok(None);
    };
    let refused = |what: &str| ConfigError::Invalid {
        entry: None,
        what: format!("host.openers: {what}"),
    };
    let mut out = Vec::with_capacity(entries.len());
    for entry in entries {
        let valid_id = (1..=64).contains(&entry.id.len())
            && entry
                .id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_'));
        if !valid_id {
            return Err(refused(
                "id must be 1 to 64 lower-case letters, digits, '-' or '_'",
            ));
        }
        if out.iter().any(|seen: &OpenerConfig| seen.id == entry.id) {
            return Err(refused(&format!("duplicate opener id {:?}", entry.id)));
        }
        if entry.label.trim().is_empty() || entry.label.chars().any(char::is_control) {
            return Err(refused("label must be non-empty text"));
        }
        if entry.program.trim().is_empty() || entry.program.chars().any(char::is_control) {
            return Err(refused("program must be a non-empty program name or path"));
        }
        if entry.args.is_empty()
            || !entry
                .args
                .iter()
                .any(|arg| arg.contains("{path}") || arg.contains("{parent}"))
            || entry
                .args
                .iter()
                .any(|arg| arg.chars().any(|c| matches!(c, '\0' | '\r' | '\n')))
        {
            return Err(refused("args must carry {path} (or {parent})"));
        }
        out.push(OpenerConfig {
            id: entry.id,
            label: entry.label,
            program: entry.program,
            args: entry.args,
            item: entry.item,
        });
    }
    Ok(Some(out))
}
