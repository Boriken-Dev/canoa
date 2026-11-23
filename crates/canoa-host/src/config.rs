//! The `[host]` table of canoa.cfg: what the host serves, and to whom. The other tables belong to the app and are not read.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use std::ffi::OsString;
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;

use indexmap::IndexMap;

mod entries;
mod entries_parse;
pub(crate) mod layers;
mod profiles;
mod raw;
mod sections;
mod types;

#[cfg(test)]
mod tests;

use crate::format::formats;
pub use entries::{
    AccountConfig, CACHE_LIFETIMES, ConfigError, DEFAULT_SHARED_IDLE_SECONDS, Entry, EntryKind,
    HarnessOptions, HostProfile, ImportSearch, Listen, Origin, PoolConfig, SETTING_SOURCES,
    Settings, TransportListen, WebConfig, WebMode, WebTls, cache_lifetime_label,
    cache_lifetime_name, cache_lifetime_named,
};
pub(crate) use entries::{PoolIssue, usage_id};
use entries_parse::{
    OriginKeys, TransportKeys, built_in_settings, check_unique, entry, extension_check,
    harness_options, local_agents_of, origin_of, remote_agents_of, tool_of, transport_of,
};
pub use entries_parse::{built_in_harnesses, is_built_in_harness};
pub use layers::{ENTRIES, INSTALLED_DIR_VARIABLE, Layers, merge_into};

/// What the host protects when the person wrote no `[permissions] protected` list: the app's recommended list, one file both read.
pub(crate) const RECOMMENDED_PROTECTED: &str = include_str!("protected_default.txt");
pub use profiles::CLIENT_KINDS;
use profiles::{
    client_loadouts_of, default_loadout_of, loadouts_of, local_profiles_of, overlay_kb_of,
    overlay_options_of, presets_of, profile_of, settings_of, skill_pins_of,
};
use raw::{
    RawAccount, RawAgent, RawAsks, RawCommon, RawDelegate, RawEnvironment, RawExtension, RawFile,
    RawHost, RawKb, RawLines, RawPool, RawProvider, network_of,
};
use sections::{
    account_of, check_title, config_error, control_of, directory_of, history_of, invalid,
    native_of, pool_of, read_raw, web_of,
};
pub(crate) use sections::{check_label, check_name, plain_absolute};
pub use types::{
    AgentProfile, Config, ControlConfig, DEFAULT_MAX_MESSAGE_BYTES, DEFAULT_MAX_PEERS,
    DEFAULT_MAX_SESSIONS, DEFAULT_PRESET, DelegateConfig, Extension, ExtensionCheck, HistoryConfig,
    KbConfig, Loadout, LocalProfiles, MAX_NAME, MAX_TITLE, McpChoice, McpListConfig, NativeConfig,
    NetworkConfig, OpenerConfig, Preset, RemoteAgent, ResolvedLoadout, StoreConfig, UnknownLoadout,
    UnknownPreset, UsageConfig, WorktreeConfig,
};
pub use types::{
    DEFAULT_EXEC_SECONDS, DEFAULT_HOOK_HANDLERS, DEFAULT_HOOK_READ_MS, DEFAULT_HOOK_SECONDS,
    DEFAULT_MAX_PREPARED, DEFAULT_MAX_TOOL_SESSIONS, DEFAULT_OUTPUT_BYTES,
    DEFAULT_PREPARED_SECONDS, DEFAULT_SCROLLBACK_BYTES, EnvironmentConfig,
};

struct TopLevel {
    secrets: Option<serde_json::Value>,
    limit_phrases: Option<Vec<String>>,
    protected_paths: Option<Vec<String>>,
    keep_awake: bool,
    local_agents: Vec<Entry>,
    remote_agents: Vec<RemoteAgent>,
    local_profiles: LocalProfiles,
    skill_pins: BTreeMap<String, String>,
    options: BTreeMap<String, coqui_aos::Document>,
    kb_choice: BTreeMap<String, bool>,
}

struct Subsystems {
    store: StoreConfig,
    network: NetworkConfig,
    kb: KbConfig,
    mcp_list: McpListConfig,
    services: IndexMap<String, String>,
    delegate: DelegateConfig,
    control: Option<ControlConfig>,
    native: NativeConfig,
    history: HistoryConfig,
    worktrees: WorktreeConfig,
    usage: UsageConfig,
    web: Option<WebConfig>,
}

impl Config {
    /// Reads the one file `path`, in the format its name or first line gives. Relative `library` and `source` paths are
    /// taken from the file's directory, where a provider runs.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Read`] when the file cannot be read, and as [`Config::from_str_in`].
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let bytes = std::fs::read(path).map_err(|source| ConfigError::Read {
            path: path.to_owned(),
            source,
        })?;
        let name = path.file_name().and_then(|name| name.to_str());
        let raw = read_raw(&bytes, name).map_err(|error| config_error(&error, Some(path)))?;
        let base = path.parent().unwrap_or_else(|| Path::new(""));
        Self::from_raw(raw, base)
    }

    /// Reads canoa.cfg's layers, the highest winning key by key (see [`merge_into`]); a layer that does not exist is
    /// skipped. Relative `library` and `source` paths are taken from the directory of [`Layers::own`].
    ///
    /// # Errors
    ///
    /// [`ConfigError::Read`] for the person's own file when no layer exists, [`ConfigError::Parse`] naming the file of a
    /// layer that is malformed, and as [`Config::from_str_in`].
    pub fn load_layers(layers: &Layers) -> Result<Self, ConfigError> {
        let existing = layers.existing();
        if existing.is_empty() {
            return Self::load(&layers.own);
        }
        // Each layer on its own first, so that a refusal names its file; the merged document is read after.
        for file in &existing {
            formats()
                .load_typed::<RawFile>(file, anycfg::Options::new())
                .map_err(|error| config_error(&error, Some(file)))?;
        }
        let sources: Vec<Option<anycfg::Source<'_>>> = existing
            .iter()
            .map(|file| Some(anycfg::Source::path(file)))
            .collect();
        let raw = anycfg::Loader::new(formats(), "toml")
            .load_typed::<_, _, _, RawFile>(sources, &layers::Merging)
            .map_err(|error| config_error(&error, None))?;
        let base = layers.own.parent().unwrap_or_else(|| Path::new(""));
        Self::from_raw(raw, base)
    }

    /// Parses canoa.cfg's text, in TOML, or in YAML or JSON when its first line says `#!yaml` or `#!json`, with relative
    /// `library` and `source` paths taken from `base`, where a provider runs. Touches no file.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Parse`] for text that is not of this shape, naming the key or giving the position, never a value;
    /// [`ConfigError::Invalid`] for a setting the host refuses.
    pub fn from_str_in(text: &str, base: &Path) -> Result<Self, ConfigError> {
        Self::from_str_named(text, "canoa.cfg", base)
    }

    /// As [`Config::from_str_in`] for text that is a file called `name`, whose extension (`.yaml`, `.json`, `.toml`) fixes
    /// its format; `.cfg` leaves it to the first line.
    pub(crate) fn from_str_named(text: &str, name: &str, base: &Path) -> Result<Self, ConfigError> {
        let raw =
            read_raw(text.as_bytes(), Some(name)).map_err(|error| config_error(&error, None))?;
        Self::from_raw(raw, base)
    }

    fn from_raw(mut raw: RawFile, base: &Path) -> Result<Self, ConfigError> {
        let top = resolve_top_level(&mut raw, base)?;
        let Some(mut host) = raw.host else {
            return Ok(Self {
                limit_phrases: top.limit_phrases,
                protected_paths: top.protected_paths,
                keep_awake: top.keep_awake,
                local_agents: top.local_agents,
                remote_agents: top.remote_agents,
                local_profiles: top.local_profiles,
                skill_pins: top.skill_pins,
                secrets: top.secrets,
                ..Self::default()
            });
        };
        let directory = directory_of(base);
        let profiles = build_host_profiles(&mut host, &directory, &top)?;
        let (accounts, pools) = build_accounts_and_pools(
            core::mem::take(&mut host.accounts),
            core::mem::take(&mut host.pools),
        )?;
        let (entries, extensions) = build_entries_and_extensions(
            &mut host,
            base,
            &directory,
            &profiles,
            &accounts,
            &pools,
            &top.remote_agents,
        )?;
        let mut environments = IndexMap::new();
        for (name, raw) in core::mem::take(&mut host.environments) {
            let environment = environment_of(&name, raw, base, &directory)?;
            environments.insert(name, environment);
        }
        for served in &entries {
            let Some(environment) = &served.environment else {
                continue;
            };
            if environment != crate::environments::HERE && !environments.contains_key(environment) {
                return Err(invalid(
                    &served.name,
                    "environment names an environment [host.environments] does not hold, and is not here",
                ));
            }
            if served.shared {
                return Err(invalid(
                    &served.name,
                    "environment goes with shared = false: a shared session's agent runs on this computer",
                ));
            }
        }
        let openers = raw::openers_of(host.openers.take())?;
        let max_message_bytes = host.max_message_bytes.unwrap_or(DEFAULT_MAX_MESSAGE_BYTES);
        if max_message_bytes == 0 {
            return Err(ConfigError::Invalid {
                entry: None,
                what: "host: max_message_bytes must be at least 1".to_owned(),
            });
        }
        let sub = build_subsystems(&mut host, &directory, &entries)?;
        Ok(Self {
            origins: host.origins,
            entries,
            profiles,
            web: sub.web,
            limit_phrases: top.limit_phrases,
            protected_paths: top.protected_paths,
            accounts,
            pools,
            secrets: top.secrets,
            shell: host.shell,
            openers,
            max_message_bytes,
            environments,
            keep_awake: top.keep_awake,
            local_agents: top.local_agents,
            remote_agents: top.remote_agents,
            local_profiles: top.local_profiles,
            control: sub.control,
            native: sub.native,
            history: sub.history,
            worktrees: sub.worktrees,
            usage: sub.usage,
            store: sub.store,
            network: sub.network,
            kb: sub.kb,
            mcp_list: sub.mcp_list,
            services: sub.services,
            delegate: sub.delegate,
            skill_pins: top.skill_pins,
            extensions,
            makers: crate::makers::Cell::default(),
            providers: crate::control::providers::Cell::default(),
        })
    }

    /// The entry called `name`.
    #[must_use]
    pub fn entry(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.name == name)
    }
}

fn resolve_top_level(raw: &mut RawFile, base: &Path) -> Result<TopLevel, ConfigError> {
    let secrets = raw
        .secrets
        .take()
        .and_then(|table| serde_json::to_value(table).ok());
    let limit_phrases = raw.usage.take().and_then(|usage| usage.limit_phrases);
    let keep_awake = raw
        .settings
        .take()
        .and_then(|settings| settings.power)
        .and_then(|power| power.keep_awake)
        .unwrap_or(false);
    let protected_paths = raw
        .permissions
        .take()
        .and_then(|permissions| permissions.protected)
        .map(RawLines::lines);
    let local_agents = local_agents_of(&raw.agents)?;
    let remote_agents = remote_agents_of(&raw.agents, &local_agents)?;
    let mut local_profiles =
        local_profiles_of(raw.profile.take(), &raw.overlays, &raw.agents, base)?;
    if limit_phrases
        .iter()
        .flatten()
        .any(|phrase| phrase.trim().is_empty())
    {
        return Err(ConfigError::Invalid {
            entry: None,
            what:
                "[usage] limit_phrases holds an empty phrase, which would match every failed turn"
                    .to_owned(),
        });
    }
    let skill_pins = skill_pins_of(core::mem::take(&mut raw.skill_pins))?;
    local_profiles.skill_pins.clone_from(&skill_pins);
    let options = overlay_options_of(&raw.overlays)?;
    local_profiles.options.clone_from(&options);
    let kb_choice = overlay_kb_of(&raw.overlays)?;
    local_profiles.kb_choice.clone_from(&kb_choice);
    local_profiles.loadouts = loadouts_of(&raw.loadouts)?;
    local_profiles.presets = presets_of(&raw.profiles)?;
    local_profiles.default_loadout =
        default_loadout_of(raw.defaults.as_ref(), &local_profiles.loadouts)?;
    local_profiles.client_loadouts = client_loadouts_of(raw.defaults.as_ref())?;
    local_profiles.protected.clone_from(&protected_paths);
    Ok(TopLevel {
        secrets,
        limit_phrases,
        protected_paths,
        keep_awake,
        local_agents,
        remote_agents,
        local_profiles,
        skill_pins,
        options,
        kb_choice,
    })
}

fn build_host_profiles(
    host: &mut RawHost,
    directory: &Path,
    top: &TopLevel,
) -> Result<IndexMap<String, HostProfile>, ConfigError> {
    for origin in &host.origins {
        if origin.is_empty() || origin.chars().any(|c| c.is_control() || c.is_whitespace()) {
            return Err(ConfigError::Invalid {
                entry: None,
                what: "an origin is empty or holds spaces or control characters".to_owned(),
            });
        }
    }
    if host
        .shell
        .as_deref()
        .is_some_and(|shell| shell.is_empty() || shell.chars().any(char::is_control))
    {
        return Err(ConfigError::Invalid {
            entry: None,
            what: "shell is the name or path of a program".to_owned(),
        });
    }
    let mut profiles = IndexMap::new();
    for (name, table) in core::mem::take(&mut host.profiles) {
        let mut profile = profile_of(&name, table, directory)?;
        profile.skill_pins.clone_from(&top.skill_pins);
        profile.options.clone_from(&top.options);
        profile.kb_choice.clone_from(&top.kb_choice);
        profile.protected.clone_from(&top.protected_paths);
        profiles.insert(name, profile);
    }
    Ok(profiles)
}

type AccountsAndPools = (
    IndexMap<String, AccountConfig>,
    IndexMap<String, PoolConfig>,
);

fn build_accounts_and_pools(
    raw_accounts: IndexMap<String, RawAccount>,
    raw_pools: IndexMap<String, RawPool>,
) -> Result<AccountsAndPools, ConfigError> {
    let mut accounts = IndexMap::new();
    for (name, raw) in raw_accounts {
        if name == coqui_acp_api::LOCAL_ACCOUNT {
            return Err(ConfigError::Invalid {
                entry: None,
                what: format!(
                    "account {name}: that name is taken by the automatic account every harness that has a login of its own offers"
                ),
            });
        }
        if crate::pools::automatic_standard(&name).is_some() {
            return Err(ConfigError::Invalid {
                entry: None,
                what: format!(
                    "account {name}: that name is taken by the pool the host makes of every account that provides that standard"
                ),
            });
        }
        let account = account_of(&name, raw)?;
        if accounts
            .values()
            .any(|other: &AccountConfig| other.id == account.id)
        {
            return Err(ConfigError::Invalid {
                entry: None,
                what: format!("account {name}: its id is another account's"),
            });
        }
        accounts.insert(name, account);
    }
    let mut pools = IndexMap::new();
    for (name, raw) in raw_pools {
        let pool = pool_of(&name, raw, &accounts)?;
        pools.insert(name, pool);
    }
    Ok((accounts, pools))
}

fn build_entries_and_extensions(
    host: &mut RawHost,
    base: &Path,
    directory: &Path,
    profiles: &IndexMap<String, HostProfile>,
    accounts: &IndexMap<String, AccountConfig>,
    pools: &IndexMap<String, PoolConfig>,
    remote_agents: &[RemoteAgent],
) -> Result<(Vec<Entry>, Vec<Extension>), ConfigError> {
    let mut entries = Vec::new();
    for (name, agent) in core::mem::take(&mut host.agents) {
        entries.push(build_agent_entry(
            name, agent, base, directory, profiles, accounts, pools,
        )?);
    }
    for (name, provider) in core::mem::take(&mut host.providers) {
        entries.push(build_provider_entry(name, provider, base, directory)?);
    }
    let mut extensions = Vec::new();
    for (name, extension) in core::mem::take(&mut host.extensions) {
        extensions.push(build_extension(name, extension, base, directory)?);
    }
    for (name, table) in core::mem::take(&mut host.tools) {
        check_name(&name)?;
        let (common, spec) = tool_of(&name, base, directory, table)?;
        entries.push(entry(name, EntryKind::Tool { spec }, common)?);
    }
    check_unique(&entries)?;
    if let Some(remote) = remote_agents
        .iter()
        .find(|remote| entries.iter().any(|entry| entry.name == remote.name))
    {
        return Err(invalid(
            &remote.name,
            "an entry and an agent on another machine share this name",
        ));
    }
    Ok((entries, extensions))
}

/// One `[host.environments.NAME]`. Errors name a key, never a value: a setting may be a credential.
fn environment_of(
    name: &str,
    raw: RawEnvironment,
    base: &Path,
    directory: &Path,
) -> Result<EnvironmentConfig, ConfigError> {
    check_label(name)?;
    let refused = |what: String| ConfigError::Invalid {
        entry: None,
        what: format!("environment {name}: {what}"),
    };
    if name == crate::environments::HERE {
        return Err(refused(
            "that name is taken by this computer, which every host has".to_owned(),
        ));
    }
    let Some(kind) = raw.kind.filter(|kind| !kind.is_empty()) else {
        return Err(refused("kind is missing or empty".to_owned()));
    };
    let origin = origin_of(
        name,
        base,
        directory,
        OriginKeys {
            library: raw.library.as_deref(),
            source: raw.source.as_deref(),
            source_settings: raw.source_settings,
            import_search: raw.import_search.as_deref(),
        },
    )?;
    let settings = settings_of(name, raw.settings)?;
    Ok(EnvironmentConfig {
        kind,
        origin,
        settings,
        exec_seconds: raw.exec_seconds,
        output_bytes: raw.output_bytes,
        scrollback_bytes: raw.scrollback_bytes,
        max_tool_sessions: raw.max_tool_sessions,
        max_prepared: raw.max_prepared,
        prepared_seconds: raw.prepared_seconds,
        hook_read_ms: raw.hook_read_ms,
        hook_handlers: raw.hook_handlers,
        hook_seconds: raw.hook_seconds,
    })
}

fn build_agent_entry(
    name: String,
    agent: RawAgent,
    base: &Path,
    directory: &Path,
    profiles: &IndexMap<String, HostProfile>,
    accounts: &IndexMap<String, AccountConfig>,
    pools: &IndexMap<String, PoolConfig>,
) -> Result<Entry, ConfigError> {
    check_name(&name)?;
    let offered = agent.profiles.unwrap_or_default();
    if let Some(title) = &agent.title {
        check_title(&name, title)?;
    }
    let title = agent.title;
    let shared = agent.shared.unwrap_or(false);
    if !shared && agent.shared_idle_seconds.is_some() {
        return Err(invalid(
            &name,
            "shared_idle_seconds goes with shared = true",
        ));
    }
    if !shared && agent.shared_permission_window_seconds.is_some() {
        return Err(invalid(
            &name,
            "shared_permission_window_seconds goes with shared = true",
        ));
    }
    for (set, key) in [
        (agent.shared_keep_log.is_some(), "shared_keep_log"),
        (agent.shared_log_entries.is_some(), "shared_log_entries"),
        (agent.shared_log_bytes.is_some(), "shared_log_bytes"),
    ] {
        if set && !shared {
            return Err(invalid(&name, &format!("{key} goes with shared = true")));
        }
    }
    let shared_keep_log = agent.shared_keep_log.unwrap_or(true);
    let (shared_log_entries, shared_log_bytes) = (agent.shared_log_entries, agent.shared_log_bytes);
    let shared_permission_window = agent
        .shared_permission_window_seconds
        .filter(|seconds| *seconds > 0)
        .map(core::time::Duration::from_secs);
    let shared_idle = match agent.shared_idle_seconds {
        _ if !shared => None,
        Some(0) => None,
        Some(seconds) => Some(core::time::Duration::from_secs(seconds)),
        None => Some(core::time::Duration::from_secs(DEFAULT_SHARED_IDLE_SECONDS)),
    };
    for (index, profile) in offered.iter().enumerate() {
        if !profiles.contains_key(profile) {
            return Err(invalid(
                &name,
                "profiles names a profile [host.profiles] does not hold",
            ));
        }
        if offered[..index].contains(profile) {
            return Err(invalid(&name, "profiles names a profile twice"));
        }
    }
    let offered_accounts = agent.accounts.unwrap_or_default();
    for (index, account) in offered_accounts.iter().enumerate() {
        if account != coqui_acp_api::LOCAL_ACCOUNT
            && !accounts.contains_key(account)
            && !pools.contains_key(account)
            && crate::pools::automatic_standard(account).is_none()
        {
            return Err(invalid(
                &name,
                "accounts names an account [host.accounts] does not hold, or a pool [host.pools] does not hold",
            ));
        }
        if offered_accounts[..index].contains(account) {
            return Err(invalid(&name, "accounts names an account twice"));
        }
    }
    let common = RawCommon {
        listen: agent.listen,
        transport: transport_of(
            &name,
            base,
            directory,
            TransportKeys {
                id: agent.transport,
                library: agent.transport_library,
                source: agent.transport_source,
                source_settings: agent.transport_source_settings,
                import_search: agent.transport_import_search,
                settings: agent.transport_settings,
            },
        )?,
        working_roots: agent.working_roots,
        worktrees: agent.worktrees,
        peers: agent.peers,
        max_peers: agent.max_peers,
    };
    let asks = RawAsks {
        setting_sources: agent.setting_sources,
    };
    let cache_lifetime = agent
        .cache_lifetime
        .as_deref()
        .map(|text| {
            cache_lifetime_named(text).ok_or_else(|| {
                invalid(
                    &name,
                    &format!("cache_lifetime is one of {}", CACHE_LIFETIMES.join(", ")),
                )
            })
        })
        .transpose()?;
    let shell = agent.shell.clone();
    if shell
        .as_deref()
        .is_some_and(|shell| shell.trim().is_empty() || shell.chars().any(char::is_control))
    {
        return Err(invalid(
            &name,
            "shell is the id of a shell/list shell, or the name or path of a program",
        ));
    }
    let kind = match (agent.program, agent.harness) {
        (Some(program), None) => build_program_kind(
            &name,
            program,
            agent.args,
            (
                agent.usage,
                agent
                    .usage_settings
                    .map(|table| settings_of(&name, table))
                    .transpose()?,
            ),
            &asks,
            &offered_accounts,
            offered,
            &AgentForbiddenKeys {
                executable: agent.executable.is_some(),
                cache_lifetime: agent.cache_lifetime.is_some(),
                shell: agent.shell.is_some(),
                library: agent.library.is_some(),
                source: agent.source.is_some(),
                source_settings: agent.source_settings.is_some(),
                import_search: agent.import_search.is_some(),
                settings: agent.settings.is_some(),
                account_settings: agent.account_settings.is_some(),
            },
        )?,
        (None, Some(harness)) => build_harness_kind(
            &name,
            harness,
            HarnessRaw {
                usage: agent.usage,
                usage_settings: agent.usage_settings,
                args: agent.args,
                executable: agent.executable,
                shell: agent.shell,
                library: agent.library,
                source: agent.source,
                source_settings: agent.source_settings,
                import_search: agent.import_search,
                settings: agent.settings,
                account_settings: agent.account_settings,
                asks: &asks,
                cache_lifetime,
                profiles: offered,
                accounts: offered_accounts,
            },
            base,
            directory,
        )?,
        _ => {
            return Err(invalid(
                &name,
                "an agent names a program or harness, one of the two",
            ));
        }
    };
    let mut served = entry(name, kind, common)?;
    served.title = title;
    served.shared = shared;
    served.environment = agent.environment;
    served.shared_idle = shared_idle;
    served.shared_permission_window = shared_permission_window;
    served.shared_keep_log = shared_keep_log;
    served.shared_log_entries = shared_log_entries;
    served.shared_log_bytes = shared_log_bytes;
    served.shell = shell;
    Ok(served)
}

struct AgentForbiddenKeys {
    executable: bool,
    cache_lifetime: bool,
    shell: bool,
    library: bool,
    source: bool,
    source_settings: bool,
    import_search: bool,
    settings: bool,
    account_settings: bool,
}

#[allow(clippy::too_many_arguments)]
fn build_program_kind(
    name: &str,
    program: String,
    args: Option<Vec<String>>,
    (usage, usage_settings): (Option<String>, Option<Settings>),
    asks: &RawAsks,
    offered_accounts: &[String],
    offered: Vec<String>,
    forbidden: &AgentForbiddenKeys,
) -> Result<EntryKind, ConfigError> {
    if program.is_empty() {
        return Err(invalid(name, "program is empty"));
    }
    if forbidden.executable {
        return Err(invalid(name, "executable goes with harness, not program"));
    }
    for (set, key) in [
        (asks.setting_sources.is_some(), "setting_sources"),
        (forbidden.cache_lifetime, "cache_lifetime"),
        (forbidden.shell, "shell"),
        (forbidden.library, "library"),
        (forbidden.source, "source"),
        (forbidden.source_settings, "source_settings"),
        (forbidden.import_search, "import_search"),
        (forbidden.settings, "settings"),
        (forbidden.account_settings, "account_settings"),
        (!offered_accounts.is_empty(), "accounts"),
    ] {
        if set {
            return Err(invalid(
                name,
                &format!("{key} goes with harness, not program"),
            ));
        }
    }
    let usage = match usage.as_deref() {
        None => None,
        Some(text) => {
            // A reader a library or source of this file offers is known only once that is loaded, which the host does
            // when it serves; here the id is held to what an id is.
            if !canoa::usage_reader::ReaderInfo::is_id(text) {
                return Err(invalid(
                    name,
                    &format!(
                        "usage names a reader by an id of letters, digits, `_` and `-` (bundled: {})",
                        crate::usage_splice::Reader::ids().join(", ")
                    ),
                ));
            }
            Some(text.to_owned())
        }
    };
    if usage_settings.is_some() && usage.is_none() {
        return Err(invalid(name, "usage_settings goes with usage"));
    }
    Ok(EntryKind::Agent {
        program: OsString::from(program),
        args: args
            .unwrap_or_default()
            .into_iter()
            .map(OsString::from)
            .collect(),
        profiles: offered,
        usage,
        usage_settings,
    })
}

struct HarnessRaw<'a> {
    usage: Option<String>,
    usage_settings: Option<toml::Table>,
    args: Option<Vec<String>>,
    executable: Option<String>,
    shell: Option<String>,
    library: Option<String>,
    source: Option<String>,
    source_settings: Option<toml::Table>,
    import_search: Option<String>,
    settings: Option<toml::Table>,
    account_settings: Option<toml::Table>,
    asks: &'a RawAsks,
    cache_lifetime: Option<coqui_acp_api::CacheLifetime>,
    profiles: Vec<String>,
    accounts: Vec<String>,
}

fn build_harness_kind(
    name: &str,
    harness: String,
    raw: HarnessRaw<'_>,
    base: &Path,
    directory: &Path,
) -> Result<EntryKind, ConfigError> {
    if raw.usage.is_some() || raw.usage_settings.is_some() {
        return Err(invalid(
            name,
            "usage and usage_settings go with program, not harness",
        ));
    }
    if harness.is_empty() {
        return Err(invalid(name, "harness must not be empty"));
    }
    if raw.args.is_some() {
        return Err(invalid(name, "args go with program; a harness takes none"));
    }
    if raw.executable.as_deref() == Some("") {
        return Err(invalid(name, "executable is empty"));
    }
    let origin = origin_of(
        name,
        base,
        directory,
        OriginKeys {
            library: raw.library.as_deref(),
            source: raw.source.as_deref(),
            source_settings: raw.source_settings,
            import_search: raw.import_search.as_deref(),
        },
    )?;
    let built_in = is_built_in_harness(&harness);
    if built_in && origin.is_some() {
        return Err(invalid(
            name,
            &format!(
                "{} are built in: library and source load an outside provider under its own id",
                built_in_harnesses().collect::<Vec<_>>().join(", ")
            ),
        ));
    }
    if origin.is_none() && !built_in {
        return Err(invalid(
            name,
            &format!(
                "harness is {}, or the id of an outside provider that library or source loads",
                built_in_harnesses().collect::<Vec<_>>().join(", ")
            ),
        ));
    }
    let settings = raw
        .settings
        .map(|table| settings_of(name, table))
        .transpose()?;
    let account_settings = raw
        .account_settings
        .map(|table| settings_of(name, table))
        .transpose()?;
    let settings = if built_in {
        built_in_settings(name, settings, raw.executable.is_some(), directory)?
    } else {
        settings
    };
    let options = if built_in {
        harness_options(name, &harness, None, raw.asks, settings.as_ref())?
    } else {
        for (set, key) in [
            (raw.executable.is_some(), "executable"),
            (raw.shell.is_some(), "shell"),
        ] {
            if set {
                return Err(invalid(
                    name,
                    &format!(
                        "{key} goes with a built-in harness; an outside provider takes its own settings"
                    ),
                ));
            }
        }
        harness_options(name, &harness, origin.as_ref(), raw.asks, settings.as_ref())?
    };
    let options = HarnessOptions {
        cache_lifetime: raw.cache_lifetime,
        ..options
    };
    // A path is canoa.cfg's, as a provider's library is: the command line runs in a session's directory,
    // which a client chooses.
    let executable = raw.executable.map(|executable| {
        if executable.contains(['/', '\\']) {
            directory.join(executable).into_os_string()
        } else {
            OsString::from(executable)
        }
    });
    Ok(EntryKind::Harness {
        harness,
        origin,
        executable,
        options,
        settings,
        account_settings,
        profiles: raw.profiles,
        accounts: raw.accounts,
    })
}

fn build_provider_entry(
    name: String,
    provider: RawProvider,
    base: &Path,
    directory: &Path,
) -> Result<Entry, ConfigError> {
    check_name(&name)?;
    if provider.provider.is_empty() {
        return Err(invalid(&name, "provider must not be empty"));
    }
    let Some(origin) = origin_of(
        &name,
        base,
        directory,
        OriginKeys {
            library: provider.library.as_deref(),
            source: provider.source.as_deref(),
            source_settings: provider.source_settings,
            import_search: provider.import_search.as_deref(),
        },
    )?
    else {
        return Err(invalid(&name, "a provider needs library or source"));
    };
    let settings = provider
        .settings
        .map(|table| settings_of(&name, table))
        .transpose()?;
    let max_sessions = provider.max_sessions.unwrap_or(DEFAULT_MAX_SESSIONS);
    if max_sessions == 0 {
        return Err(invalid(&name, "max_sessions must be at least 1"));
    }
    let common = RawCommon {
        listen: provider.listen,
        transport: transport_of(
            &name,
            base,
            directory,
            TransportKeys {
                id: provider.transport,
                library: provider.transport_library,
                source: provider.transport_source,
                source_settings: provider.transport_source_settings,
                import_search: provider.transport_import_search,
                settings: provider.transport_settings,
            },
        )?,
        working_roots: provider.working_roots,
        worktrees: Some(false),
        peers: provider.peers,
        max_peers: provider.max_peers,
    };
    let kind = EntryKind::Provider {
        origin,
        provider: provider.provider,
        settings,
        directory: directory.to_owned(),
        max_sessions,
    };
    entry(name, kind, common)
}

fn build_extension(
    name: String,
    extension: RawExtension,
    base: &Path,
    directory: &Path,
) -> Result<Extension, ConfigError> {
    check_name(&name)?;
    if extension.provider.is_empty() {
        return Err(invalid(&name, "provider must not be empty"));
    }
    let Some(origin) = origin_of(
        &name,
        base,
        directory,
        OriginKeys {
            library: extension.library.as_deref(),
            source: extension.source.as_deref(),
            source_settings: extension.source_settings,
            import_search: extension.import_search.as_deref(),
        },
    )?
    else {
        return Err(invalid(&name, "an extension needs library or source"));
    };
    let check = extension_check(
        &name,
        base,
        &origin,
        extension.pin,
        extension.sha256,
        extension.root,
    )?;
    Ok(Extension {
        name,
        origin,
        provider: extension.provider,
        check,
    })
}

fn build_subsystems(
    host: &mut RawHost,
    directory: &Path,
    entries: &[Entry],
) -> Result<Subsystems, ConfigError> {
    let store = host
        .store
        .take()
        .map_or_else(StoreConfig::default, |raw| StoreConfig {
            protected: raw.protected.map(RawLines::lines),
            credential_variables: raw.credential_variables.map(RawLines::lines),
            skill_folders: raw.skill_folders.map(RawLines::lines),
            index_ttl: raw.index_ttl,
            index_cache_bytes: raw.index_cache_bytes,
            tools: raw.tools,
        });
    let network = host
        .network
        .take()
        .map(|raw| network_of(raw, directory))
        .transpose()?
        .unwrap_or_default();
    let kb = host
        .kb
        .take()
        .map(RawKb::into_config)
        .transpose()?
        .unwrap_or_default();
    let mcp_list = {
        let raw = host.mcp_list.take();
        let defaults = McpListConfig::default();
        let query_seconds = raw
            .as_ref()
            .and_then(|raw| raw.query_seconds)
            .unwrap_or(defaults.query_seconds);
        let max_items_per_server = raw
            .as_ref()
            .and_then(|raw| raw.max_items_per_server)
            .unwrap_or(defaults.max_items_per_server);
        let max_reply_bytes = raw
            .as_ref()
            .and_then(|raw| raw.max_reply_bytes)
            .unwrap_or(defaults.max_reply_bytes);
        for (key, zero) in [
            ("query_seconds", query_seconds == 0),
            ("max_items_per_server", max_items_per_server == 0),
            ("max_reply_bytes", max_reply_bytes == 0),
        ] {
            if zero {
                return Err(ConfigError::Invalid {
                    entry: None,
                    what: format!("host.mcp_list: {key} must be at least 1"),
                });
            }
        }
        McpListConfig {
            query_seconds,
            max_items_per_server,
            max_reply_bytes,
        }
    };
    let tables = core::mem::take(&mut host.services);
    let mut services = IndexMap::new();
    for (id, table) in tables {
        if !canoa::session_service::is_id(&id) {
            return Err(ConfigError::Invalid {
                entry: None,
                what: format!(
                    "[host.services.{id}] the id is 1 to 32 lower-case letters, digits and _"
                ),
            });
        }
        let text = serde_json::to_string(&table).map_err(|error| ConfigError::Invalid {
            entry: None,
            what: format!("[host.services.{id}] {error}"),
        })?;
        services.insert(id, text);
    }
    let delegate = host
        .delegate
        .take()
        .map(RawDelegate::into_config)
        .transpose()?
        .unwrap_or_default();
    let raw_web = host.web.take();
    let admits = raw_web
        .as_ref()
        .is_some_and(|web| web.office == Some(true) && web.office_local == Some(true));
    let control = host
        .control
        .take()
        .map(|raw| control_of(raw, admits))
        .transpose()?;
    let native = host
        .native
        .take()
        .map(native_of)
        .transpose()?
        .unwrap_or_default();
    let history = host
        .history
        .take()
        .map(history_of)
        .transpose()?
        .unwrap_or_default();
    let worktrees = match host.worktrees.take() {
        None => WorktreeConfig::default(),
        Some(raw) => {
            let default = WorktreeConfig::default();
            let config = WorktreeConfig {
                max_per_repository: raw.max_per_repository.unwrap_or(default.max_per_repository),
                git_seconds: raw.git_seconds.unwrap_or(default.git_seconds),
            };
            if config.max_per_repository == 0 || config.git_seconds == 0 {
                return Err(ConfigError::Invalid {
                    entry: None,
                    what: "host.worktrees: max_per_repository and git_seconds are at least 1; worktrees = false on an entry makes none".to_owned(),
                });
            }
            config
        }
    };
    let usage = match host.usage.take() {
        None => UsageConfig::default(),
        Some(raw) => {
            let keep_days = raw.keep_days.unwrap_or(UsageConfig::default().keep_days);
            if keep_days == 0 {
                return Err(ConfigError::Invalid {
                    entry: None,
                    what: "host.usage: keep_days is at least 1".to_owned(),
                });
            }
            UsageConfig { keep_days }
        }
    };
    if let Some(listen) = control.as_ref().and_then(|control| control.listen.as_ref())
        && listen.is_fixed()
        && entries.iter().any(|entry| &entry.listen == listen)
    {
        return Err(ConfigError::Invalid {
            entry: None,
            what: "host.control: another entry listens on the same address".to_owned(),
        });
    }
    let web = raw_web
        .map(|raw| web_of(raw, directory, entries))
        .transpose()?;
    Ok(Subsystems {
        store,
        network,
        kb,
        mcp_list,
        services,
        delegate,
        control,
        native,
        history,
        worktrees,
        usage,
        web,
    })
}
