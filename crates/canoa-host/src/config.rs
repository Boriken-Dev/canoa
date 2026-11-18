//! The `[host]` table of canoa.cfg: what the host serves, and to whom. The other tables belong to the app and are not read.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;
use core::net::{IpAddr, Ipv4Addr, SocketAddr};
use core::time::Duration;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use canoa::network::{ClientCertificate, GitTls, NetworkSettings, ProxyChoice, Secret};
use coqui_acp_api::CacheLifetime;
use coqui_peer_webcrypto::PeerId;
use coqui_transport_pipe::PipeName;
use indexmap::IndexMap;
use serde::Deserialize;

use crate::format::formats;
use crate::profile::{LayerKind, Profile};
use crate::tool::ToolSpec;

pub(crate) mod layers;
pub use layers::{ENTRIES, INSTALLED_DIR_VARIABLE, Layers, merge_into};

/// Connections an entry serves at once when the file does not say.
pub const DEFAULT_MAX_PEERS: usize = 4;
/// Sessions one connection to a provider may hold when the file does not say.
pub const DEFAULT_MAX_SESSIONS: usize = 8;
/// The longest entry name, which appears in logs and in what `serve` prints.
pub const MAX_NAME: usize = 64;
/// The longest `title` of an agent entry, in characters.
pub const MAX_TITLE: usize = 64;

/// What the host serves: every entry of `[host.agents]`, `[host.providers]` and `[host.tools]`, in the file's order, agents
/// first, then providers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Config {
    /// Browser origins that may connect; empty lets any connect, and the proof still decides.
    pub origins: Vec<String>,
    /// The entries.
    pub entries: Vec<Entry>,
    /// `[host.profiles]`, by name, in the file's order: what an agent entry may be launched with.
    pub profiles: IndexMap<String, HostProfile>,
    /// `[host.web]`: where the web app is served; `None` when the file has no such table.
    pub web: Option<WebConfig>,
    /// `[usage] limit_phrases`: the words that make a failed turn of an agent that reports no usage a limit. `None` is
    /// unset, which means coqui-acp-api's defaults; a list replaces them, and an empty one infers nothing.
    pub limit_phrases: Option<Vec<String>>,
    /// `[permissions] protected`: the places a session may not read or change without leave, in gitignore syntax, as the app
    /// reads it. `None` is unset, which leaves each provider its own recommended list; a list replaces it.
    pub protected_paths: Option<Vec<String>>,
    /// `[host.accounts]`, by name, in the file's order: the accounts a harness entry may offer.
    pub accounts: IndexMap<String, AccountConfig>,
    /// Where the accounts' credentials are kept: `[host] account_store`.
    pub account_store: AccountStore,
    /// `[host] shell`: the program a terminal starts, with no arguments; `None` leaves it to the person's own (`$SHELL`, else
    /// `pwsh` or `powershell` on Windows).
    pub shell: Option<String>,
    /// `[host.store]`: what the program stores' installs are held to.
    pub store: StoreConfig,
    /// `[host.network]`: how the host reaches the network; the system's proxy, login, certificate verifier and client
    /// certificates unless it says otherwise.
    pub network: NetworkConfig,
    /// `[host.kb]`: the knowledge index tools and where their embeddings come from.
    pub kb: KbConfig,
    /// `[host.mail]`: the mailboxes the `canoa-mail` tools reach.
    pub mail: MailConfig,
    /// `[host.delegate]`: what an agent may hand to another one.
    pub delegate: DelegateConfig,
    /// `[host.control]`: the control channel's limits, and where a paired key may reach it from; `None` when the file has no
    /// such table, which leaves the channel to this process.
    pub control: Option<ControlConfig>,
    /// `[host.native]`: the door a browser extension's native messaging host reaches this host by.
    pub native: NativeConfig,
    /// `[host.history]`: whether the host records the sessions it serves, and how much it keeps.
    pub history: HistoryConfig,
    /// `[host.usage]`: how long the host keeps the spend it records.
    pub usage: UsageConfig,
    /// `[agents.NAME]` with a `program`: the person's own agent commands, as entries reached over [`crate::Local`] only (each has
    /// [`Entry::local`] set), in the file's order. They are served when the host serves its own sources.
    pub local_agents: Vec<Entry>,
    /// `[agents.NAME]` with an `a2a` table: another machine's A2A agents, which a task may name as a source when
    /// `[host.delegate] sources` lists them. Served to no one, and not offered as a session.
    pub remote_agents: Vec<RemoteAgent>,
    /// The top-level `[profile]`, `[overlays]` and `[agents.NAME.profile|mcp]`: what the host's own sources are launched with.
    pub local_profiles: LocalProfiles,
    /// `[skill_pins]`, by lower-case skill name: the tree digest each skill from elsewhere must have.
    pub skill_pins: BTreeMap<String, String>,
    /// `[host.extensions.NAME]`: the `store_extension` providers that add types to the Store, in the file's order.
    pub extensions: Vec<Extension>,
}

/// One `[host.extensions.NAME]`: a `store_extension` provider the host loads, whose store types the Store lists.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Extension {
    /// The entry's name.
    pub name: String,
    /// Where its code is.
    pub origin: Origin,
    /// The `store_extension` provider id.
    pub provider: String,
    /// What the extension's bytes must have each time the host loads them, when the Store installed it.
    pub check: Option<ExtensionCheck>,
}

/// What an installed extension's bytes must have, as the Store recorded when it checked them against its catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ExtensionCheck {
    /// The tree digest (`sha256:` and 64 hexadecimal digits) of the folder `root`, which holds the library.
    Tree {
        /// The folder, absolute.
        root: PathBuf,
        /// The digest.
        pin: String,
    },
    /// The SHA-256 of the library file, as 64 hexadecimal digits.
    File {
        /// The digest.
        sha256: String,
    },
}

/// One `[agents.NAME] a2a = { url, peer, source, title }`: an agent on another machine that a task may be handed to.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct RemoteAgent {
    /// The source's name, which a task names.
    pub name: String,
    /// What is shown for the name; `None` shows the name.
    pub title: Option<String>,
    /// Where the other host listens: a `ws://` address. The link is sealed by the proof, so the socket needs no TLS.
    pub url: String,
    /// The key the other host must prove. A task is sent to no other.
    pub peer: PeerId,
    /// The source on the other host's card that the task names; `None` leaves the choice to it, which it makes only when
    /// it offers one.
    pub source: Option<String>,
}

/// What canoa.cfg's top level says a session of the host's own sources is launched with. Nothing here is served to a network
/// peer: it applies to the entries reached over [`crate::Local`] only.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct LocalProfiles {
    /// `[profile]`: the person's own layer.
    pub profile: Option<Profile>,
    /// The names in `[overlays]`, in the file's order: overlays the app installed in `overlays/` beside canoa.cfg.
    pub overlays: Vec<String>,
    /// `[agents.NAME.profile]` and `[agents.NAME.mcp]`, by the agent's label.
    pub agents: IndexMap<String, AgentProfile>,
    /// canoa.cfg's directory, absolute.
    pub directory: PathBuf,
    /// `[skill_pins]`, by lower-case skill name.
    pub skill_pins: BTreeMap<String, String>,
    /// `[overlays.NAME] options`: the values the person gave an overlay's declared options, by overlay name.
    pub options: BTreeMap<String, coqui_aos::Document>,
    /// `[overlays.NAME] kb = "index" or "files"`: whether the overlay's declared rules and context files are served through the
    /// `canoa-kb` tools instead of being put in its context whole (`true`) or kept whole (`false`). An overlay with no key follows
    /// its own `[knowledge] serve` hint.
    pub kb_choice: BTreeMap<String, bool>,
    /// `[loadouts.NAME]`: the named collections of overlays, skills and MCP servers a session may start with, in the file's order.
    pub loadouts: IndexMap<String, Loadout>,
    /// `[defaults] loadout`: the loadout a session starts with when it names none.
    pub default_loadout: Option<String>,
    /// `[defaults.clients]`: the loadout a session starts with when it names none and its client says which kind it is, by kind
    /// (see [`CLIENT_KINDS`]). It wins over `[defaults] loadout`. A name `[loadouts]` does not hold is kept, so a connection can say so.
    pub client_loadouts: BTreeMap<String, String>,
    /// `[profiles.NAME]`: the named presets of a session's own values, in the file's order.
    pub presets: IndexMap<String, Preset>,
    /// `[permissions] protected`: the places a hook's allow never answers for.
    pub protected: Option<Vec<String>>,
}

/// One `[profiles.NAME]`: the values a new session starts with, chosen by name. It is the session's layer, which sits above the
/// person's own `[profile]` and the agent's entry; the account and the permissions are never part of one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Preset {
    /// The label of the source it runs on; the session keeps its own source when none is named.
    pub source: Option<String>,
    /// The model it starts with.
    pub model: Option<String>,
    /// The effort it starts with.
    pub effort: Option<String>,
    /// The mode it starts in.
    pub mode: Option<String>,
    /// The starting instructions: text added to the session's initial context after every other layer's.
    pub context: Option<String>,
    /// How long the prompt cache lives, one of the names `cache_lifetime` takes elsewhere.
    pub cache_lifetime: Option<String>,
    /// The loadout it starts with, by name; the default loadout when none is named.
    pub loadout: Option<String>,
    /// The access it runs the agent with (`access = { sandbox, network, approvals }`), part by part.
    pub access: crate::profile::AccessSetting,
    /// What it holds for each item a provider adds (`[profiles.NAME.items.ID]`), by item id: the values of the item's form, by field
    /// name. Whether a provider offers the item, and whether the values fit its form, is known when a session starts; a profile
    /// keeps the values of an item nothing offers.
    pub items: IndexMap<String, serde_json::Map<String, serde_json::Value>>,
}

impl Preset {
    /// What this profile holds for every item, by item id: the session's own settings (the model, effort, mode, starting
    /// instructions and access) as the items that carry them take them, then the items providers add.
    #[must_use]
    pub fn item_values(&self) -> IndexMap<String, serde_json::Map<String, serde_json::Value>> {
        use serde_json::{Map, Value};
        let mut out = IndexMap::new();
        for (id, value) in [
            ("model", &self.model),
            ("effort", &self.effort),
            ("mode", &self.mode),
            ("context", &self.context),
        ] {
            if let Some(text) = value {
                let mut values = Map::new();
                values.insert(id.to_owned(), Value::String(text.clone()));
                out.insert(id.to_owned(), values);
            }
        }
        if !self.access.is_empty() {
            let mut values = Map::new();
            if let Some(sandbox) = self.access.sandbox {
                values.insert("sandbox".to_owned(), Value::String(sandbox.id().to_owned()));
            }
            if let Some(network) = self.access.network {
                values.insert("network".to_owned(), Value::Bool(network));
            }
            if let Some(approvals) = self.access.approvals {
                values.insert(
                    "approvals".to_owned(),
                    Value::String(approvals.id().to_owned()),
                );
            }
            out.insert("access".to_owned(), values);
        }
        for (id, values) in &self.items {
            out.insert(id.clone(), values.clone());
        }
        out
    }

    /// This preset as the session's layer: the settings its model, effort, mode, access and starting-instructions items make, as
    /// the items Canoa carries apply them. The items a provider adds are applied when a session starts.
    #[must_use]
    pub fn session_layer(&self) -> Profile {
        let own: IndexMap<_, _> = self
            .item_values()
            .into_iter()
            .filter(|(id, _)| crate::profile::items::SETTING_IDS.contains(&id.as_str()))
            .collect();
        let applied = crate::profile::items::ItemCatalog::built_in().apply_all(&own, None);
        crate::profile::items::session_layer(&applied).0
    }
}

/// The name of the profile that is always there: `[profiles.default]`. It cannot be removed, and a session that names no other
/// profile starts with it.
pub const DEFAULT_PRESET: &str = "default";

/// A profile that sets nothing: what the default profile is until the person edits it.
fn empty_preset() -> &'static Preset {
    static EMPTY: std::sync::OnceLock<Preset> = std::sync::OnceLock::new();
    EMPTY.get_or_init(Preset::default)
}

/// A session asked for a preset `[profiles]` does not hold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownPreset(pub String);

impl fmt::Display for UnknownPreset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "there is no profile named {}", self.0)
    }
}

impl core::error::Error for UnknownPreset {}

/// One `[loadouts.NAME]`: a named collection a session is started with. Overlays apply whole, in the order `[overlays]` composes
/// them; the loadout cannot reorder them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Loadout {
    /// What the loadout is for, in the person's words.
    pub description: String,
    /// Overlays by the name `[overlays]` gives them.
    pub overlays: Vec<String>,
    /// The skills the person chose themselves, which the host delivers from the app's record of each once the record and the
    /// files agree.
    pub skills: Vec<String>,
    /// The servers of `[profile.mcp]` the session starts with; the person's own servers it does not name are off.
    pub mcp: Vec<String>,
}

/// What a session starts with once its loadout is resolved: the loadout as the file said it at that moment.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ResolvedLoadout {
    /// The loadout's name.
    pub name: String,
    /// Its installed overlays, in the order `[overlays]` composes them.
    pub overlays: Vec<String>,
    /// Overlays it names that `[overlays]` does not list.
    pub missing_overlays: Vec<String>,
    /// Its chosen skills.
    pub skills: Vec<String>,
    /// Its servers of `[profile.mcp]`.
    pub mcp: Vec<String>,
}

/// A session asked for a loadout `[loadouts]` does not hold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownLoadout(pub String);

impl fmt::Display for UnknownLoadout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "there is no loadout named {}", self.0)
    }
}

impl core::error::Error for UnknownLoadout {}

/// The loadout `name` (or `[defaults] loadout` when it is `None`) as it stands, against the overlays `[overlays]` lists. `None`
/// when none is asked for and no default is set: the session then starts with everything the profile lists.
fn resolve_loadout(
    loadouts: &IndexMap<String, Loadout>,
    default: Option<&str>,
    installed: &[String],
    name: Option<&str>,
) -> Result<Option<ResolvedLoadout>, UnknownLoadout> {
    let Some(name) = name.or(default) else {
        return Ok(None);
    };
    let loadout = loadouts
        .get(name)
        .ok_or_else(|| UnknownLoadout(name.to_owned()))?;
    let overlays = installed
        .iter()
        .filter(|each| loadout.overlays.contains(each))
        .cloned()
        .collect();
    let missing_overlays = loadout
        .overlays
        .iter()
        .filter(|each| !installed.contains(each))
        .cloned()
        .collect();
    Ok(Some(ResolvedLoadout {
        name: name.to_owned(),
        overlays,
        missing_overlays,
        skills: loadout.skills.clone(),
        mcp: loadout.mcp.clone(),
    }))
}

/// One agent's `[agents.NAME.profile]` layer and `[agents.NAME.mcp]` choices.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct AgentProfile {
    /// The agent's layer.
    pub profile: Option<Profile>,
    /// For an MCP server of the profile: on or off, or the definition digest an overlay's or a project's server is allowed at.
    pub mcp: BTreeMap<String, McpChoice>,
    /// An overlay's name and the tree digest (`sha256:` and 64 hex digits) its hooks are allowed at: `[agents.NAME.hooks]`.
    pub hooks: BTreeMap<String, String>,
}

/// What the person chose for an MCP server of an agent.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum McpChoice {
    /// `true`: a server of the person's own is on, which is what it is by default.
    On,
    /// `false`: off.
    Off,
    /// `sha256:` and the digest of the definition an overlay's or a project's server is allowed at.
    At(String),
}

impl LocalProfiles {
    /// The profile `agent` is launched with, or `None` when nothing in canoa.cfg is for it.
    #[must_use]
    pub fn for_agent(&self, agent: &str) -> Option<HostProfile> {
        let layer = self.agents.get(agent);
        let own = layer.and_then(|layer| layer.profile.clone());
        if self.profile.is_none()
            && self.overlays.is_empty()
            && self.loadouts.is_empty()
            && self.presets.is_empty()
            && own.is_none()
            && layer.is_none_or(|layer| layer.mcp.is_empty() && layer.hooks.is_empty())
        {
            return None;
        }
        let mcp = layer.map(|layer| layer.mcp.clone()).unwrap_or_default();
        let allow_mcp = mcp
            .iter()
            .filter_map(|(name, choice)| match choice {
                McpChoice::At(digest) => Some((name.clone(), digest.clone())),
                _ => None,
            })
            .collect();
        let off_mcp = mcp
            .iter()
            .filter(|(_, choice)| **choice == McpChoice::Off)
            .map(|(name, _)| name.clone())
            .collect();
        Some(HostProfile {
            profile: self.profile.clone().unwrap_or_default(),
            overlays: self.overlays.clone(),
            allow_mcp,
            allow_hooks: layer.map(|layer| layer.hooks.clone()).unwrap_or_default(),
            agent: own.map(|profile| (agent.to_owned(), profile)),
            off_mcp,
            directory: self.directory.clone(),
            skill_pins: self.skill_pins.clone(),
            options: self.options.clone(),
            kb_choice: self.kb_choice.clone(),
            chosen_skills: Vec::new(),
            loadouts: self.loadouts.clone(),
            default_loadout: self.default_loadout.clone(),
            client_loadouts: self.client_loadouts.clone(),
            presets: self.presets.clone(),
            items: crate::profile::ItemSet::default(),
            session: None,
            protected: self.protected.clone(),
        })
    }

    /// The loadout `name` (`[defaults] loadout` when `None`) as the file says it now; `None` when none applies.
    ///
    /// # Errors
    ///
    /// [`UnknownLoadout`] when `name` is not in `[loadouts]`.
    pub fn resolve_loadout(
        &self,
        name: Option<&str>,
    ) -> Result<Option<ResolvedLoadout>, UnknownLoadout> {
        resolve_loadout(
            &self.loadouts,
            self.default_loadout.as_deref(),
            &self.overlays,
            name,
        )
    }
}

/// `[host.native]`: the door on this user's own machine that the browser extension's native messaging host relays to.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct NativeConfig {
    /// Whether the door is open. On unless the person turns it off.
    pub enabled: bool,
    /// What a browser key paired through the door may do on `@control`.
    pub control: BTreeSet<crate::control::Grant>,
}

impl NativeConfig {
    /// The grants a browser key has on the door's control channel when the file says nothing: its own session records, its own
    /// worktrees, folders, settings of the app's own tables, and what the program stores find, and the right to offer the
    /// browser's tools to this host's sessions (`tools.provide`). It never installs, restarts the host, uploads, or changes what
    /// an agent may do.
    #[must_use]
    pub fn default_control() -> BTreeSet<crate::control::Grant> {
        use crate::control::Grant;
        [
            Grant::LedgerRead,
            Grant::LedgerWrite,
            Grant::Worktree,
            Grant::StoreRead,
            Grant::Folders,
            Grant::ConfigRead,
            Grant::ConfigWrite,
            Grant::Discover,
            Grant::ToolsProvide,
        ]
        .into_iter()
        .collect()
    }
}

impl Default for NativeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            control: Self::default_control(),
        }
    }
}

/// `[host.control]`.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ControlConfig {
    /// Where a paired key reaches the control channel; `None` leaves it to this process.
    pub listen: Option<Listen>,
    /// What each paired key may do on it, in the file's order. A key not here is turned away.
    pub grants: IndexMap<PeerId, BTreeSet<crate::control::Grant>>,
    /// Ledger records the host keeps.
    pub max_records: usize,
    /// The largest ledger record, in bytes.
    pub max_record_bytes: usize,
    /// The largest file a key may upload, in bytes.
    pub max_upload_bytes: u64,
    /// What one key may keep uploaded, in bytes.
    pub max_upload_total_bytes: u64,
    /// The files one key may keep uploaded.
    pub max_uploads: usize,
}

/// `[host.history]`: the host records each session it serves in the session-history format, in its home beside the ledger, and
/// the owner exports it or moves it to another source over the control channel. Every limit is a setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct HistoryConfig {
    /// `record`: whether sessions are recorded at all (on by default).
    pub record: bool,
    /// `max_session_bytes`: what one session's history may take on disk; a turn that would pass it is not kept.
    pub max_session_bytes: u64,
    /// `max_sessions`: how many sessions have a history; a new one past it is not recorded.
    pub max_sessions: usize,
    /// `max_total_bytes`: what every history together may take; a turn that would pass it is not kept.
    pub max_total_bytes: u64,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            record: true,
            max_session_bytes: crate::history::DEFAULT_MAX_SESSION_BYTES,
            max_sessions: crate::history::DEFAULT_MAX_SESSIONS,
            max_total_bytes: crate::history::DEFAULT_MAX_TOTAL_BYTES,
        }
    }
}

/// `[host.usage]`: the host records the spend each agent reports, by turn, in its home (`usage/`), and answers `usage/summary` and
/// `usage/limits` from it. This is not `[usage]`, the app's and the limit phrases' table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct UsageConfig {
    /// `keep_days`: how many days of records the host keeps; a month's file is removed at start once all of it is older.
    pub keep_days: u32,
}

impl Default for UsageConfig {
    fn default() -> Self {
        Self {
            keep_days: crate::usage_log::DEFAULT_KEEP_DAYS,
        }
    }
}

/// `[host.store]`: the lists, in gitignore syntax, that the host holds a program store's installs to. Each is `None` when the
/// file has no such key, which means the recommended list; a list written here replaces it (an empty one protects nothing).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct StoreConfig {
    /// `protected`: the places an install may not declare a change at, under or holding. A string with a pattern a line, or a
    /// list of such strings.
    pub protected: Option<Vec<String>>,
    /// `credential_variables`: the variables an install is never given, matched against a name, in the same form.
    pub credential_variables: Option<Vec<String>>,
    /// `index_ttl`: seconds the catalog index answers with no network (default 300); past it the stale index is served at once
    /// and refreshed in the background. 0 refreshes on every request.
    pub index_ttl: Option<u64>,
    /// `index_cache_bytes`: what the index's cached icons may take in all (default 64 MiB).
    pub index_cache_bytes: Option<u64>,
    /// `tools`: whether the host offers the `canoa-store` tools (search and inspect) to a session whose client asks for them and
    /// whose key reads the index (default true).
    pub tools: Option<bool>,
}

/// `[host.network]`: how every fetch the host makes, and every program it starts, reaches the network. With no key the host follows
/// the system: the proxy variables of its environment, else the operating system's proxy settings (PAC and WPAD included) and its
/// proxy login, the operating system's certificate verifier and the client certificates of its keystores. Each key is an
/// override.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct NetworkConfig {
    /// `proxy`: `system` (the default), `none`, `pac:LOCATION` (a URL, a `file:` URL or a path), or a proxy for every URL as
    /// `[scheme://][user[:password]@]host[:port]`.
    pub proxy: ProxyChoice,
    /// `proxy_user`: a login for every HTTP proxy that names none of its own.
    pub proxy_user: Option<String>,
    /// `proxy_password_env`: the name of a variable holding that login's password; the password itself is never in canoa.cfg.
    pub proxy_password_env: Option<String>,
    /// `no_proxy`: the hosts that go direct, replacing the system's list: names (a name covers its subdomains), `*.suffix`,
    /// networks (`10.0.0.0/8`), `host:port`, `<-loopback>` to send loopback through the proxy too. A string or a list; an empty
    /// one sends everything but loopback through the proxy.
    pub no_proxy: Option<Vec<String>>,
    /// `negotiate`: whether a proxy that asks for Negotiate or NTLM is answered with the logon session (default true on Windows,
    /// the only place this build can).
    pub negotiate: Option<bool>,
    /// `client_certificate`: `system` (the default: what a browser here would send, from the operating system's keystores),
    /// `none`, or the path of a PEM file holding a certificate chain and its unencrypted private key.
    pub client_certificate: ClientCertificate,
    /// `ca_file`: trust exactly the certificates of this PEM file, replacing the system's trust.
    pub ca_file: Option<PathBuf>,
    /// `git_tls`: `system` (the default: Schannel on Windows, so git uses the system's certificate store) or `git` (git's own, and
    /// the person's git configuration).
    pub git_tls: GitTls,
    /// `endpoints`: URLs the programs the host starts will use, added to the built-in list a PAC script is asked about to give them
    /// their proxy variables.
    pub endpoints: Vec<String>,
    /// `proxy_idle_seconds`: how long the proxy the host runs for sessions that use `system` or `pac` is kept after its last session
    /// ends, for the next session that asks for the same one (default 0: it ends with its last session).
    pub proxy_idle_seconds: Option<u64>,
}

impl NetworkConfig {
    /// The settings the network is built from, with the password read from its variable.
    ///
    /// # Errors
    ///
    /// `proxy_password_env` names a variable that is not set.
    pub fn settings(&self) -> Result<NetworkSettings, ConfigError> {
        let mut settings = NetworkSettings::default();
        settings.proxy.clone_from(&self.proxy);
        settings.proxy_user.clone_from(&self.proxy_user);
        if let Some(name) = &self.proxy_password_env {
            let password = std::env::var(name).map_err(|_| ConfigError::Invalid {
                entry: None,
                what: "network: proxy_password_env names a variable that is not set".to_owned(),
            })?;
            settings.proxy_password = Some(Secret::new(password));
        }
        settings.no_proxy.clone_from(&self.no_proxy);
        settings.negotiate = self.negotiate;
        settings
            .client_certificate
            .clone_from(&self.client_certificate);
        settings.ca_file.clone_from(&self.ca_file);
        settings.git_tls = self.git_tls;
        settings.endpoints.clone_from(&self.endpoints);
        Ok(settings)
    }
}

/// `[host.kb]`: the `canoa-kb` tools, which search the knowledge an overlay ships. With no `embeddings` an index holds words only
/// (BM25); with one, passages are also embedded by the endpoint the person names, which Canoa never chooses for them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct KbConfig {
    /// `tools`: whether the host offers the `canoa-kb` tools to a session whose client asks for them (default true).
    pub tools: Option<bool>,
    /// `embeddings`: the address of an OpenAI-compatible embeddings server (`http://127.0.0.1:60570`, with or without `/v1`).
    pub embeddings: Option<String>,
    /// `model`: the exact model id that server loaded; required with `embeddings`, and saved with each index.
    pub model: Option<String>,
    /// `api_key_env`: the name of a variable holding a key the server asks for, sent as a bearer; the key itself is never in
    /// canoa.cfg.
    pub api_key_env: Option<String>,
    /// `document_prefix`: text put before each passage when it is embedded, for models that ask for one.
    pub document_prefix: Option<String>,
    /// `query_prefix`: text put before a query when it is embedded.
    pub query_prefix: Option<String>,
    /// `chunk_bytes`: the most bytes of a passage, its prefix included (4 to 65536; default 1024).
    pub chunk_bytes: Option<usize>,
    /// `overlap_bytes`: the most bytes of whole lines adjacent passages share (default 0).
    pub overlap_bytes: Option<usize>,
    /// `llm_endpoint`: the address of an OpenAI-compatible server whose model orders the best passages of a search (the `llm`
    /// mode); the passages and the query are sent to it.
    pub llm_endpoint: Option<String>,
    /// `llm_model`: the exact model id that server loaded; required with `llm_endpoint`.
    pub llm_model: Option<String>,
    /// `llm_api_key_env`: the name of a variable holding a key that server asks for, sent as a bearer.
    pub llm_api_key_env: Option<String>,
    /// `llm_candidates`: the most passages the model is shown (default 10, 1 to 100).
    pub llm_candidates: Option<usize>,
    /// `llm_timeout_secs`: how long one ordering may take before the search answers without it (default 30).
    pub llm_timeout_secs: Option<u64>,
}

/// `[host.mail]`: the `canoa-mail` tools, which let an agent search, read, draft and organize the person's mail. Nothing is
/// offered until an account is named; the tools then reach every session of a key that holds the `mail` grant.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct MailConfig {
    /// `tools`: whether the host offers the mail tools at all (default true).
    pub tools: Option<bool>,
    /// `[host.mail.accounts.NAME]`, in the file's order.
    pub accounts: IndexMap<String, MailAccount>,
}

/// What reaches a mailbox.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MailBackend {
    /// Exchange Server's web services (EWS).
    Ews,
    /// Exchange Online through Microsoft Graph, with a token from the person's Windows sign-in.
    Graph,
    /// Classic Outlook for Windows, through a PowerShell helper that drives its object model.
    Outlook,
}

/// How an Exchange account signs in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MailAuth {
    /// A user name and a password the environment holds.
    Basic,
    /// The person's Windows logon.
    Windows,
}

/// One `[host.mail.accounts.NAME]`. It holds no password: `password_env` names the variable that does, and `id` names the entry
/// of the host's account store that does.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct MailAccount {
    /// What reaches the mailbox.
    pub backend: MailBackend,
    /// The mailbox's address, which Autodiscover asks about (`ews`), or the account Windows is asked to sign in as when it
    /// holds several (`graph`).
    pub email: Option<String>,
    /// The EWS address; Autodiscover finds it from `email` when this is not given (`ews`). For `graph`, Graph's address
    /// (default `https://graph.microsoft.com/v1.0`).
    pub server: Option<String>,
    /// The Autodiscover address to ask instead of the two the address's domain suggests (`ews`).
    pub autodiscover: Option<String>,
    /// How an Exchange account signs in.
    pub auth: MailAuth,
    /// The account's user name for Basic (`ews`).
    pub user: Option<String>,
    /// The variable that holds the password for Basic (`ews`); it is used before a stored password.
    pub password_env: Option<String>,
    /// The id the password for Basic rests under in the host's account store, which `canoa mail add` writes (`ews`).
    pub id: Option<String>,
    /// The Outlook profile to log on to; the one Outlook is running with otherwise (`outlook`).
    pub profile: Option<String>,
    /// The program and arguments that run the helper, instead of Windows PowerShell with the helper script (`outlook`,
    /// `graph`).
    pub command: Option<Vec<String>>,
    /// The application (client) id of the app registration in Entra ID that the Windows sign-in is asked for a token for
    /// (`graph`).
    pub client_id: Option<String>,
    /// The tenant the registration is in: its id or domain, `organizations` when not given (`graph`).
    pub tenant: Option<String>,
    /// How long one call may take, in seconds (default 60).
    pub timeout_secs: u64,
}

/// `[host.delegate]`: what an agent session may hand to another source, as tasks the host runs. Every limit is a setting with a
/// default and no floor; a value away from its default is logged when the host starts.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct DelegateConfig {
    /// `enabled`: whether the host offers the delegation tools at all. A client still has to ask for them.
    pub enabled: bool,
    /// `sources`: the entries a task may name. `None` is every entry the delegating connection may use; a list narrows that, an
    /// empty one allows none.
    pub sources: Option<Vec<String>>,
    /// `max_children`: tasks one session has running at once.
    pub max_children: usize,
    /// `max_running`: tasks the whole host has running at once.
    pub max_running: usize,
    /// `max_depth`: how many levels of tasks may delegate in turn; 1 lets a session hand tasks out and no task hand any on.
    pub max_depth: usize,
    /// `a2a`: the paired keys this host serves A2A to, over any entry's address those keys may use. A key not listed is
    /// refused the protocol; being paired gives it none.
    pub a2a: Vec<PeerId>,
    /// `task_bytes`: the longest task text.
    pub task_bytes: usize,
    /// `result_bytes`: the most of a task's reply that one `task_status` answer carries.
    pub result_bytes: usize,
    /// `history_bytes`: the updates of a task the host keeps, oldest dropped first.
    pub history_bytes: usize,
    /// `context_bytes`: the most of a conversation's turns, or of the files it touched, a task may be handed as its context.
    pub context_bytes: usize,
}

impl DelegateConfig {
    /// The default of `max_children`.
    pub const DEFAULT_MAX_CHILDREN: usize = 8;
    /// The default of `max_running`.
    pub const DEFAULT_MAX_RUNNING: usize = 16;
    /// The default of `max_depth`.
    pub const DEFAULT_MAX_DEPTH: usize = 1;
    /// The default of `task_bytes`.
    pub const DEFAULT_TASK_BYTES: usize = 64 * 1024;
    /// The default of `result_bytes`.
    pub const DEFAULT_RESULT_BYTES: usize = 64 * 1024;
    /// The default of `history_bytes`.
    pub const DEFAULT_HISTORY_BYTES: usize = 1024 * 1024;
    /// The default of `context_bytes`.
    pub const DEFAULT_CONTEXT_BYTES: usize = 32 * 1024;
}

impl Default for DelegateConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            sources: None,
            max_children: Self::DEFAULT_MAX_CHILDREN,
            max_running: Self::DEFAULT_MAX_RUNNING,
            max_depth: Self::DEFAULT_MAX_DEPTH,
            a2a: Vec::new(),
            task_bytes: Self::DEFAULT_TASK_BYTES,
            result_bytes: Self::DEFAULT_RESULT_BYTES,
            history_bytes: Self::DEFAULT_HISTORY_BYTES,
            context_bytes: Self::DEFAULT_CONTEXT_BYTES,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDelegate {
    enabled: Option<bool>,
    sources: Option<Vec<String>>,
    max_children: Option<usize>,
    max_running: Option<usize>,
    max_depth: Option<usize>,
    a2a: Option<Vec<String>>,
    task_bytes: Option<usize>,
    result_bytes: Option<usize>,
    history_bytes: Option<usize>,
    context_bytes: Option<usize>,
}

impl RawDelegate {
    fn into_config(self) -> Result<DelegateConfig, ConfigError> {
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
        })
    }
}

/// A list of lines as canoa.cfg writes it: one string, a line a line, or a list of strings.
#[derive(Deserialize)]
#[serde(untagged)]
enum RawLines {
    One(String),
    Many(Vec<String>),
}

impl RawLines {
    fn lines(self) -> Vec<String> {
        match self {
            Self::One(text) => text.lines().map(str::to_owned).collect(),
            Self::Many(lines) => lines,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStore {
    protected: Option<RawLines>,
    credential_variables: Option<RawLines>,
    index_ttl: Option<u64>,
    index_cache_bytes: Option<u64>,
    tools: Option<bool>,
}

/// Where the credentials of `[host.accounts]` are kept, never in canoa.cfg.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum AccountStore {
    /// The operating system's credential store when it answers, sealed files when there is none (a Linux service with no
    /// session bus), and files for good once they hold an account; the host says which and why.
    #[default]
    Auto,
    /// Only the operating system's credential store (Windows Credential Manager, macOS Keychain, Secret Service); a host
    /// without one cannot keep an account.
    Os,
    /// A file per account in the host's home, sealed to a key beside it: a weakening the host logs.
    File,
}

/// One `[host.accounts.NAME]`: which login an entry may run a session on. It holds no credential.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct AccountConfig {
    /// The provider, as an entry's `harness`.
    pub harness: String,
    /// One of the kinds the provider's catalog lists.
    pub kind: String,
    /// The account's id, which Canoa writes when it makes the account: never reused, and the key of its stored credential.
    pub id: String,
    /// The kind's settings that are not secrets, checked against the provider's schema when the host starts.
    pub settings: Option<Settings>,
}

/// `[host.web]`: the web app on a listener of its own, beside the entries.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct WebConfig {
    /// The site's directory, absolute; `None` serves the site built in (feature `web`).
    pub dir: Option<PathBuf>,
    /// Where it listens: loopback unless `tls` is given.
    pub listen: SocketAddr,
    /// HTTPS, from a certificate and key; plain HTTP without it.
    pub tls: Option<WebTls>,
    /// Host names a request may carry besides `localhost` and an IP address, as a proxy or a certificate names the host.
    pub hosts: Vec<String>,
    /// Origins the page may connect to besides this host's entries (`https://fonts.gstatic.com`): each widens the page's
    /// policy, and is logged as configuration.
    pub connect: Vec<String>,
    /// What one connection may take.
    pub limits: crate::web::Limits,
    /// Directories the site may not hold, such as canoa.cfg's and the identity's: a site whose directory is one of them,
    /// or above one, is refused. Not read from the file.
    pub keep_out: Vec<PathBuf>,
    /// Where the person sits in front of the page, told to it in `/canoa.json`; `None` says local to a page reached over
    /// loopback and remote to one reached by any other name.
    pub mode: Option<WebMode>,
    /// Serves the Office add-in's pages under `/office/` with a policy of their own, which lets Office on the web frame them and
    /// lets them load Office.js. Off by default: every route then has the one policy.
    pub office: bool,
    /// Origins besides the ones built in that may frame the add-in's pages (`https://*.officeapps.live.com`), each widening the
    /// policy of those pages and logged as configuration. Only with `office`.
    pub office_frames: Vec<String>,
    /// The add-in runs in Office on this computer, so the key of the page it frames is accepted without a pairing step: the page,
    /// reached at a loopback name and its own origin, asks `POST /office/admit` and gets what `[host.native] control` gives a
    /// browser key. Only with `office`, and only with a loopback `listen`: behind a proxy every request looks local.
    pub office_local: bool,
    /// With `office_local`, the add-in's requests from the page (admitting its key, its tools, starting a host) carry a secret kept in
    /// the home, which the manifest rendered for this computer holds, so that another account on a shared computer cannot make
    /// them. Off by default: the computer is the person's own.
    pub office_secret: bool,
}

/// Whether the browser showing the web app is at the computer that serves it (the host starts an editor or file manager
/// there) or is only the interface to a computer elsewhere (files go through the app, an editor connects over SSH).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WebMode {
    /// The browser sits at this computer.
    Local,
    /// The browser is somewhere else.
    Remote,
}

impl WebMode {
    /// Its name in canoa.cfg and in `/canoa.json`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Remote => "remote",
        }
    }
}

impl WebConfig {
    /// The defaults: the site built in, on [`crate::web::DEFAULT_LISTEN`], plain HTTP, no other host name, default limits.
    #[must_use]
    pub fn new() -> Self {
        Self {
            dir: None,
            listen: crate::web::DEFAULT_LISTEN,
            tls: None,
            hosts: Vec::new(),
            connect: Vec::new(),
            limits: crate::web::Limits::default(),
            keep_out: Vec::new(),
            mode: None,
            office: false,
            office_frames: Vec::new(),
            office_local: false,
            office_secret: false,
        }
    }
}

impl Default for WebConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// The PEM files HTTPS takes, absolute.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct WebTls {
    /// The certificate, then any intermediates in order.
    pub cert: PathBuf,
    /// The certificate's private key.
    pub key: PathBuf,
}

impl WebTls {
    /// The certificate chain at `cert` and its key at `key`.
    #[must_use]
    pub fn new(cert: impl Into<PathBuf>, key: impl Into<PathBuf>) -> Self {
        Self {
            cert: cert.into(),
            key: key.into(),
        }
    }
}

/// A profile of `[host.profiles]`: the host's own layer, the overlays below it, and the overlays' MCP servers it allows.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct HostProfile {
    /// The profile document, read as the person's own layer: its relative paths are taken from `directory`.
    pub profile: Profile,
    /// AOS overlays the app installed in `overlays/` beside canoa.cfg, by name, applied below the profile.
    pub overlays: Vec<String>,
    /// An overlay's MCP server name and the definition digest (`sha256:` and 64 hex digits) it may start at. No other
    /// overlay server starts.
    pub allow_mcp: BTreeMap<String, String>,
    /// An overlay's name and the tree digest (`sha256:` and 64 hex digits) its hooks are allowed at. No other overlay's hooks
    /// run unless the person allows them for a session when the host asks.
    pub allow_hooks: BTreeMap<String, String>,
    /// The agent's own layer above the profile, with the agent's label: `[agents.NAME.profile]`.
    pub agent: Option<(String, Profile)>,
    /// Servers of the profile turned off, whichever layer they come from.
    pub off_mcp: BTreeSet<String>,
    /// canoa.cfg's directory, absolute.
    pub directory: PathBuf,
    /// `[skill_pins]`: the tree digest a skill from elsewhere must have, by lower-case skill name. A skill is
    /// delivered only when this digest, its record's and the overlay's own pin (if it has one) are all the same.
    pub skill_pins: BTreeMap<String, String>,
    /// `[overlays.NAME] options`: the values the person gave an overlay's declared options, by overlay name. The overlay's
    /// manifest says which exist and of what type; a value of another type refuses the text that reads it.
    pub options: BTreeMap<String, coqui_aos::Document>,
    /// `[overlays.NAME] kb = "index"`: see [`LocalProfiles::kb_choice`].
    pub kb_choice: BTreeMap<String, bool>,
    /// The skills the session's loadout names, for the host's own sources only: each is delivered from the app's record of it
    /// when the record and the files agree. Empty until a connection resolves its loadout.
    pub chosen_skills: Vec<String>,
    /// `[loadouts.NAME]`, which a connection resolves once when it starts.
    pub loadouts: IndexMap<String, Loadout>,
    /// `[defaults] loadout`.
    pub default_loadout: Option<String>,
    /// `[defaults.clients]`: see [`LocalProfiles::client_loadouts`].
    pub client_loadouts: BTreeMap<String, String>,
    /// `[profiles.NAME]`, which a connection resolves once when it starts.
    pub presets: IndexMap<String, Preset>,
    /// The profile items this host offers, which a session's profile is applied through.
    pub items: crate::profile::ItemSet,
    /// The session's layer, above the agent's: the preset the connection named, once resolved.
    pub session: Option<Profile>,
    /// `[permissions] protected`: the places a hook's allow never answers for, in gitignore syntax; none leaves it to the
    /// host's recommended places.
    pub protected: Option<Vec<String>>,
}

impl HostProfile {
    /// The preset `name` as this profile holds it.
    ///
    /// # Errors
    ///
    /// [`UnknownPreset`] when `name` is not in `[profiles]`.
    pub fn resolve_preset(&self, name: &str) -> Result<&Preset, UnknownPreset> {
        match self.presets.get(name) {
            Some(held) => Ok(held),
            None if name == DEFAULT_PRESET => Ok(empty_preset()),
            None => Err(UnknownPreset(name.to_owned())),
        }
    }

    /// The preset a session starts with when it names none: `[profiles.default]`, or one that sets nothing when the file has no
    /// such table.
    #[must_use]
    pub fn default_preset(&self) -> &Preset {
        self.presets
            .get(DEFAULT_PRESET)
            .unwrap_or_else(|| empty_preset())
    }

    /// The loadout `[defaults.clients]` names for a client of `kind`, if it names one.
    #[must_use]
    pub fn client_loadout(&self, kind: &str) -> Option<&str> {
        self.client_loadouts.get(kind).map(String::as_str)
    }

    /// The loadout `name` (`[defaults] loadout` when `None`) as this profile holds it; `None` when none applies.
    ///
    /// # Errors
    ///
    /// [`UnknownLoadout`] when `name` is not in `[loadouts]`.
    pub fn resolve_loadout(
        &self,
        name: Option<&str>,
    ) -> Result<Option<ResolvedLoadout>, UnknownLoadout> {
        resolve_loadout(
            &self.loadouts,
            self.default_loadout.as_deref(),
            &self.overlays,
            name,
        )
    }
}

/// Where an entry listens, as an entry's `listen` writes it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Listen {
    /// A WebSocket on this address, which a browser can reach.
    Address(SocketAddr),
    /// A Windows named pipe (`pipe:NAME`) or a Unix socket (`unix:PATH`), which only a native client on this machine can
    /// reach, as the operating system's access control allows. The proof is the same as on an address.
    Pipe(PipeName),
    /// An endpoint of a `transport` provider an entry names (`transport = "ID"`), which carries the entry's connections in
    /// place of a WebSocket, a pipe or a Unix socket. The proof is the same as on an address.
    Transport(alloc::boxed::Box<TransportListen>),
}

/// A `transport` provider and the endpoint an entry listens on through it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct TransportListen {
    /// The provider's id.
    pub id: String,
    /// Where an outside provider's code is loaded from; `None` for `pipe` and `websocket`, which are built in.
    pub origin: Option<Origin>,
    /// The provider's configuration, checked against its schema when the host starts; `None` takes its defaults.
    pub settings: Option<Settings>,
    /// What the provider listens on, as it reads it: the entry's `listen`, which the host does not interpret.
    pub endpoint: String,
}

impl Listen {
    /// The text of an address, `pipe:NAME` or `unix:PATH`.
    ///
    /// # Errors
    ///
    /// What the text is not, or why the pipe's name cannot be used here, in a clause that quotes nothing.
    pub fn parse(text: &str) -> Result<Self, String> {
        if let Some(path) = text.strip_prefix("unix:")
            && !Path::new(path).is_absolute()
        {
            // A relative path would be taken from wherever the host happens to run.
            return Err("listen: a unix: path must be absolute".to_owned());
        }
        if text.starts_with("pipe:") || text.starts_with("unix:") {
            return text
                .parse::<PipeName>()
                .map(Self::Pipe)
                .map_err(|error| format!("listen is not usable here: {error}"));
        }
        text.parse::<SocketAddr>()
            .map(Self::Address)
            .map_err(|_| "listen is not an address and port, pipe:NAME or unix:PATH".to_owned())
    }

    /// Whether two entries listening here and there are listening in one place: the same address, pipe or socket, or the same
    /// endpoint of providers (the transport and its settings do not make another place).
    fn same_place(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Transport(a), Self::Transport(b)) => a.endpoint == b.endpoint,
            (Self::Transport(_), _) | (_, Self::Transport(_)) => false,
            _ => self == other,
        }
    }

    /// The address, for a listener a browser can reach; `None` for a pipe.
    #[must_use]
    pub const fn address(&self) -> Option<SocketAddr> {
        match self {
            Self::Address(address) => Some(*address),
            Self::Pipe(_) | Self::Transport(_) => None,
        }
    }

    /// Whether this names a place that is one entry's alone: a pipe, or an address with a port.
    fn is_fixed(&self) -> bool {
        match self {
            Self::Address(address) => address.port() != 0,
            Self::Pipe(_) => true,
            // A provider's endpoint is its own text; one that asks for port 0 takes whatever is free.
            Self::Transport(transport) => !url::Url::parse(&transport.endpoint)
                .is_ok_and(|endpoint| endpoint.port() == Some(0)),
        }
    }
}

impl fmt::Display for Listen {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Address(address) => address.fmt(f),
            Self::Pipe(name) => name.fmt(f),
            Self::Transport(transport) => f.write_str(&transport.endpoint),
        }
    }
}

impl From<SocketAddr> for Listen {
    fn from(address: SocketAddr) -> Self {
        Self::Address(address)
    }
}

/// One agent the host offers on an address of its own.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Entry {
    /// Its name in the file, unique across both tables.
    pub name: String,
    /// What runs for each connection.
    pub kind: EntryKind,
    /// Where it listens: an address, where port 0 takes any free port and loopback is the default unless the file says
    /// otherwise (the link is sealed by the paired key's method, so another address is configuration, which `serve` logs, not
    /// an exception), or a pipe or Unix socket.
    pub listen: Listen,
    /// Directories a session's `cwd` must be under. Empty takes any absolute path.
    pub working_roots: Vec<PathBuf>,
    /// Whether a client may have the host make a git worktree for a session inside `working_roots`; an agent or tool entry
    /// with no working root gets none either way. On unless the entry sets `worktrees = false`; always off for a provider.
    pub worktrees: bool,
    /// The paired peers that may use it; `None` lets every paired peer.
    pub peers: Option<Vec<PeerId>>,
    /// Whether canoa.cfg's top level (`[profile]`, `[overlays]`, `[agents.NAME]`) is this entry's profile, as it is the host's own
    /// sources': the person's own layer, with the overlays they installed. Off unless `own_profile = true`; never with `profiles`.
    pub own_profile: bool,
    /// Connections served at once; another is turned away while they run.
    pub max_peers: usize,
    /// What a client shows for an agent entry instead of its name; `None` shows the name. Always `None` for a provider or tool.
    pub title: Option<String>,
    /// Whether it is served only over [`crate::Local`]: it listens on no address, and no paired key reaches it. The host's
    /// own sources are these; canoa.cfg cannot make one.
    pub local: bool,
    /// Whether a session outlives the connection that opened it: the host keeps its agent running, and any key the entry
    /// admits may attach to it by `session/load` or `session/resume` and take part (`shared = true`). Off, each connection
    /// has an agent of its own that ends with it. Only an agent or harness entry may set it.
    pub shared: bool,
    /// How long a shared session that no client is attached to is kept, after which the host ends it
    /// (`shared_idle_seconds`, a day by default); `None` keeps it until a client ends it or the host stops (`0`).
    pub shared_idle: Option<core::time::Duration>,
    /// How long a permission request of a shared session may wait with nobody answering before the host logs that it does
    /// (`shared_permission_window_seconds`); `None` (unset or `0`) logs nothing. The request stays open: nothing is answered
    /// for the people attached.
    pub shared_permission_window: Option<core::time::Duration>,
    /// Whether a shared session's log of updates is kept in the host's home, so that it survives the host's restart and the
    /// cursors clients hold stay valid (`shared_keep_log`, on by default). Off, a session is gone with the host, and a cursor
    /// from before is refused when it comes back.
    pub shared_keep_log: bool,
    /// The updates a shared session keeps for replay, the oldest going first (`shared_log_entries`); `None` takes the
    /// hub's default of 100000.
    pub shared_log_entries: Option<usize>,
    /// The serialized bytes of updates a shared session keeps for replay (`shared_log_bytes`); `None` takes the hub's default
    /// of 64 MiB.
    pub shared_log_bytes: Option<usize>,
    /// The shell a harness entry's command line is told to use (`shell = "git-bash"`): an id `shell/list` gives, or a program (an
    /// absolute path, or a name found on the `PATH`). `None` takes `[host] shell`. Each harness is told as its documentation
    /// allows (`shells::for_harness`), and one that takes no setting is left alone and the host's log says so.
    pub shell: Option<String>,
}

/// How long a shared session with no client attached is kept unless the entry says otherwise.
pub const DEFAULT_SHARED_IDLE_SECONDS: u64 = 24 * 60 * 60;

/// What a built-in harness entry's own keys ask of its command line. The access (sandbox, network, approvals) is not here: it
/// is the profile's, the same for every harness, and each provider turns it into its command line's own words. An outside
/// provider takes its own settings through `settings`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct HarnessOptions {
    /// The setting sources Claude Code loads, of `user`, `project` and `local`; empty loads none, `None` its own default.
    pub setting_sources: Option<Vec<String>>,
    /// How long the provider's prompt cache should live for this entry's sessions (`cache_lifetime = "default" | "off" |
    /// "short" | "long"`), when the harness has such a control. A client's own choice for one session wins; unset leaves the client's
    /// default for its kind of session (the chat asks `long`), and the harness's own where it names none.
    pub cache_lifetime: Option<CacheLifetime>,
}

/// The values of `cache_lifetime`, in canoa.cfg and on `canoa agent`.
pub const CACHE_LIFETIMES: &[&str] = &["default", "off", "short", "long"];

/// The lifetime `name` stands for, one of [`CACHE_LIFETIMES`].
#[must_use]
pub fn cache_lifetime_named(name: &str) -> Option<CacheLifetime> {
    match name {
        "default" => Some(CacheLifetime::Default),
        "off" => Some(CacheLifetime::Off),
        "short" => Some(CacheLifetime::Short),
        "long" => Some(CacheLifetime::Long),
        _ => None,
    }
}

/// The name of `lifetime` in [`CACHE_LIFETIMES`].
#[must_use]
pub const fn cache_lifetime_name(lifetime: CacheLifetime) -> &'static str {
    match lifetime {
        CacheLifetime::Off => "off",
        CacheLifetime::Short => "short",
        CacheLifetime::Long => "long",
        _ => "default",
    }
}

/// Claude Code's setting sources.
pub const SETTING_SOURCES: &[&str] = &["user", "project", "local"];

/// Where the host reads a stdio agent's usage figures from, beyond its failed turns (`usage = "..."` on the entry).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum UsageSource {
    /// Antigravity (`agy acp`): its quota, read from the loopback port its own language server opens once a session is open.
    Antigravity,
}

impl UsageSource {
    /// The source named `name` in canoa.cfg.
    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "antigravity" => Some(Self::Antigravity),
            _ => None,
        }
    }
}
/// What an entry runs for each connection.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EntryKind {
    /// A program speaking ACP on its standard input and output.
    Agent {
        /// The program, found as the operating system finds a command.
        program: OsString,
        /// Its arguments.
        args: Vec<OsString>,
        /// The `[host.profiles]` a client may choose among, the first when it names none; empty launches it bare.
        profiles: Vec<String>,
        /// How the host reads this agent's usage when the agent reports none itself; `None` reads only its failed turns.
        usage: Option<UsageSource>,
    },
    /// An `acp_harness` provider, chosen by its id and run in this process: the host is the ACP client's end, and the provider
    /// starts the agent's program through the host's spawner. `claude` and `codex` are built in; any other id is an outside
    /// provider that `origin` loads.
    Harness {
        /// The provider's id.
        harness: String,
        /// Where an outside provider's code comes from; `None` is the built-in provider of that id.
        origin: Option<Origin>,
        /// The `claude` or `codex` provider's program: a name the operating system finds as a command, or a path made absolute
        /// from canoa.cfg's directory; the provider's own (`claude`, `codex`) when `None`.
        executable: Option<OsString>,
        /// What the `claude` or `codex` provider asks before it acts, as canoa.cfg sets it.
        options: HarnessOptions,
        /// The provider's own configuration (`settings`), checked against the schema it declares when the host starts.
        settings: Option<Settings>,
        /// The configuration of the account provider the same library offers (`account_settings`), which signs in, renews,
        /// watches and reads plan limits for the provider's accounts; checked against its schema when the host starts.
        account_settings: Option<Settings>,
        /// The `[host.profiles]` a client may choose among, as an agent's.
        profiles: Vec<String>,
        /// The `[host.accounts]` a client may choose among, the first when it names none; empty runs the harness on
        /// the login it has where it runs.
        accounts: Vec<String>,
    },
    /// A Guatiao `agent_provider`, from a library or a source, served through the ACP facade in a process of its own.
    Provider {
        /// Where its code comes from.
        origin: Origin,
        /// The provider's id.
        provider: String,
        /// The provider's configuration, checked against the schema it declares when a connection loads it.
        settings: Option<Settings>,
        /// The directory the provider's process runs in, absolute: canoa.cfg's.
        directory: PathBuf,
        /// Sessions one connection may hold.
        max_sessions: usize,
    },
    /// A command-line tool with no ACP of its own, served as an ACP agent by the rules its entry gives.
    Tool {
        /// The program, how a prompt reaches it and what its events become.
        spec: crate::tool::ToolSpec,
    },
}

/// Where a provider's code comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Origin {
    /// A Guatiao library file, absolute.
    Library(PathBuf),
    /// A Guatiao source file, absolute, such as `guatiao_source_py`, which serves libraries written in another language.
    Source {
        /// The source file.
        file: PathBuf,
        /// What the source is added with: for Python, the directories of its manifests and its import paths.
        settings: Settings,
        /// Where Windows looks for what the source imports itself (Python's `python3.dll`) that its own directory, the
        /// executable's and the system's leave missing.
        import_search: ImportSearch,
        /// Whether `import_search` is in the file; false when it was taken from [`ImportSearch::default_for`].
        import_search_named: bool,
    },
}

/// Where Windows looks for an import a source names itself (Python's `python3.dll`) that the source's own directory, the
/// executable's and the system's leave missing: `import_search` on a source entry. Elsewhere the platform's loader
/// decides, and both are the same.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ImportSearch {
    /// `library`: the source's `import_dirs` only.
    Library,
    /// `path`: `import_dirs`, then the Windows directory, the working directory and `PATH`, so whoever can write any of
    /// them chooses the code the source loads. `serve` logs it: as configuration when the file names it, as a warning when
    /// it was taken as the default.
    Path,
}

impl ImportSearch {
    /// `library` or `path`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Library => "library",
            Self::Path => "path",
        }
    }

    /// The search a source entry takes when it names none: [`ImportSearch::Library`] when its `import_dirs` name at
    /// least one directory, since they say where to look; [`ImportSearch::Path`] otherwise, so a source that relied on
    /// `PATH` still loads, and is logged as a warning.
    #[must_use]
    pub fn default_for(settings: &Settings) -> Self {
        let listed = settings
            .as_map()
            .get("import_dirs")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|dirs| !dirs.is_empty());
        if listed { Self::Library } else { Self::Path }
    }

    /// The search `text` names: `library` or `path`.
    #[must_use]
    pub fn named(text: &str) -> Option<Self> {
        match text {
            "library" => Some(Self::Library),
            "path" => Some(Self::Path),
            _ => None,
        }
    }
}

/// A table of settings from canoa.cfg, as JSON, the form a provider process receives it in. `Debug` shows its keys,
/// never its values, which may be secrets.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Settings(serde_json::Map<String, serde_json::Value>);

impl Settings {
    /// Settings holding `map`.
    #[must_use]
    pub fn new(map: serde_json::Map<String, serde_json::Value>) -> Self {
        Self(map)
    }

    /// The table.
    #[must_use]
    pub fn as_map(&self) -> &serde_json::Map<String, serde_json::Value> {
        &self.0
    }
}

impl fmt::Debug for Settings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.0.keys()).finish()
    }
}

/// Why canoa.cfg could not be used. No variant quotes a value from the file, which may hold a secret.
#[derive(Debug)]
#[non_exhaustive]
pub enum ConfigError {
    /// The file could not be read.
    Read {
        /// The file.
        path: PathBuf,
        /// What the operating system said.
        source: std::io::Error,
    },
    /// The file is not a canoa.cfg of the expected shape, in TOML, YAML or JSON.
    Parse {
        /// The file, when the text came from one.
        file: Option<PathBuf>,
        /// 1-based line, when the format's parser knows it; a setting of the wrong shape has none, and is named instead.
        line: Option<usize>,
        /// 1-based column, with the line.
        column: Option<usize>,
        /// What was wrong, in words of the schema's.
        what: String,
    },
    /// An entry, or `[host]` itself when `entry` is `None`, has a setting the host refuses.
    Invalid {
        /// The entry's name.
        entry: Option<String>,
        /// Which setting and why.
        what: String,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => write!(f, "could not read {}: {source}", path.display()),
            Self::Parse {
                file,
                line,
                column,
                what,
            } => {
                if let Some(file) = file {
                    write!(f, "{}: ", file.display())?;
                }
                match (line, column) {
                    (Some(line), Some(column)) => write!(
                        f,
                        "the configuration is malformed at line {line}, column {column}: {what}"
                    ),
                    (Some(line), None) => {
                        write!(f, "the configuration is malformed at line {line}: {what}")
                    }
                    _ => write!(f, "the configuration is malformed: {what}"),
                }
            }
            Self::Invalid {
                entry: Some(entry),
                what,
            } => write!(f, "host entry {entry}: {what}"),
            Self::Invalid { entry: None, what } => write!(f, "[host]: {what}"),
        }
    }
}

impl core::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[derive(Deserialize)]
struct RawFile {
    #[serde(default)]
    host: Option<RawHost>,
    #[serde(default)]
    usage: Option<RawUsage>,
    /// `[permissions]`: the app's table, of which the host reads `protected` and lets the rest go by.
    #[serde(default)]
    permissions: Option<RawPermissions>,
    /// `[agents.NAME]`: the app's table, of which the host reads `program` and `args` and lets the rest go by.
    #[serde(default)]
    agents: IndexMap<String, toml::Value>,
    /// `[profile]`: the person's own layer.
    #[serde(default)]
    profile: Option<toml::Table>,
    /// `[overlays]`: the app's table, of which the host reads the names.
    #[serde(default)]
    overlays: IndexMap<String, toml::Value>,
    #[serde(default)]
    skill_pins: toml::Table,
    /// `[loadouts.NAME]`: the named collections a session starts with.
    #[serde(default)]
    loadouts: IndexMap<String, toml::Value>,
    /// `[defaults]`: the app's table, of which the host reads `loadout`.
    #[serde(default)]
    defaults: Option<toml::Table>,
    /// `[profiles.NAME]`: the named presets of a session's own values.
    #[serde(default)]
    profiles: IndexMap<String, toml::Value>,
}

/// `[permissions]`: the app reads the modes and the rules, so only the key the host passes on is named.
#[derive(Deserialize)]
struct RawPermissions {
    #[serde(default)]
    protected: Option<RawLines>,
}

/// `[usage]`: the app reads `warn_permille`, so only the key the host acts on is named and the others are let go by.
#[derive(Deserialize)]
struct RawUsage {
    #[serde(default)]
    limit_phrases: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHost {
    #[serde(default)]
    origins: Vec<String>,
    #[serde(default)]
    agents: IndexMap<String, RawAgent>,
    #[serde(default)]
    providers: IndexMap<String, RawProvider>,
    #[serde(default)]
    tools: IndexMap<String, toml::Table>,
    #[serde(default)]
    extensions: IndexMap<String, RawExtension>,
    #[serde(default)]
    profiles: IndexMap<String, toml::Table>,
    #[serde(default)]
    accounts: IndexMap<String, RawAccount>,
    account_store: Option<String>,
    shell: Option<String>,
    web: Option<RawWeb>,
    control: Option<RawControl>,
    native: Option<RawNative>,
    history: Option<RawHistory>,
    usage: Option<RawHostUsage>,
    store: Option<RawStore>,
    network: Option<RawNetwork>,
    delegate: Option<RawDelegate>,
    kb: Option<RawKb>,
    mail: Option<RawMail>,
}

/// `[host.network]`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawNetwork {
    proxy: Option<String>,
    proxy_user: Option<String>,
    proxy_password_env: Option<String>,
    no_proxy: Option<RawLines>,
    negotiate: Option<bool>,
    client_certificate: Option<String>,
    ca_file: Option<String>,
    git_tls: Option<String>,
    #[serde(default)]
    endpoints: Vec<String>,
    proxy_idle_seconds: Option<u64>,
}

/// `[host.network]`, its paths made absolute from canoa.cfg's directory. Errors name a key, never a value.
fn network_of(raw: RawNetwork, directory: &Path) -> Result<NetworkConfig, ConfigError> {
    let refused = |what: &str| ConfigError::Invalid {
        entry: None,
        what: format!("network: {what}"),
    };
    let path = |text: String, key: &str| {
        if text.is_empty() {
            return Err(refused(&format!("{key} is empty")));
        }
        let path = directory.join(text);
        Ok(std::path::absolute(&path).unwrap_or(path))
    };
    let proxy = match raw.proxy.as_deref().map(str::trim) {
        None => ProxyChoice::System,
        Some("") => return Err(refused("proxy is empty")),
        Some(text) if text.eq_ignore_ascii_case("system") => ProxyChoice::System,
        Some(text) if text.eq_ignore_ascii_case("none") => ProxyChoice::None,
        Some(text) => match text.strip_prefix("pac:") {
            Some("") => return Err(refused("proxy pac: names no location")),
            Some(location) => ProxyChoice::Pac(location.to_owned()),
            None => ProxyChoice::Proxy(text.to_owned()),
        },
    };
    let client_certificate = match raw.client_certificate {
        None => ClientCertificate::System,
        Some(text) if text.eq_ignore_ascii_case("system") => ClientCertificate::System,
        Some(text) if text.eq_ignore_ascii_case("none") => ClientCertificate::None,
        Some(text) => ClientCertificate::File(path(text, "client_certificate")?),
    };
    let git_tls = match raw.git_tls.as_deref() {
        None | Some("system") => GitTls::System,
        Some("git") => GitTls::Git,
        Some(_) => return Err(refused("git_tls is system or git")),
    };
    if raw.proxy_user.as_deref().is_some_and(str::is_empty) {
        return Err(refused("proxy_user is empty"));
    }
    if raw.proxy_password_env.is_some() && raw.proxy_user.is_none() {
        return Err(refused("proxy_password_env goes with proxy_user"));
    }
    Ok(NetworkConfig {
        proxy,
        proxy_user: raw.proxy_user,
        proxy_password_env: raw.proxy_password_env,
        no_proxy: raw.no_proxy.map(|lines| {
            lines
                .lines()
                .iter()
                .flat_map(|line| line.split(|c: char| c == ',' || c.is_whitespace()))
                .filter(|entry| !entry.is_empty())
                .map(str::to_owned)
                .collect()
        }),
        negotiate: raw.negotiate,
        client_certificate,
        ca_file: raw.ca_file.map(|file| path(file, "ca_file")).transpose()?,
        git_tls,
        endpoints: raw.endpoints,
        proxy_idle_seconds: raw.proxy_idle_seconds,
    })
}

/// `[host.kb]`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawKb {
    tools: Option<bool>,
    embeddings: Option<String>,
    model: Option<String>,
    api_key_env: Option<String>,
    document_prefix: Option<String>,
    query_prefix: Option<String>,
    chunk_bytes: Option<usize>,
    overlap_bytes: Option<usize>,
    llm_endpoint: Option<String>,
    llm_model: Option<String>,
    llm_api_key_env: Option<String>,
    llm_candidates: Option<usize>,
    llm_timeout_secs: Option<u64>,
}

impl RawKb {
    fn into_config(self) -> Result<KbConfig, ConfigError> {
        let invalid = |what: &str| ConfigError::Invalid {
            entry: None,
            what: format!("[host.kb] {what}"),
        };
        if let Some(address) = &self.embeddings {
            let parsed =
                url::Url::parse(address).map_err(|_| invalid("embeddings is not an address"))?;
            if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
                return Err(invalid("embeddings is an http or https address"));
            }
            if self.model.as_deref().is_none_or(str::is_empty) {
                return Err(invalid("embeddings needs the model its server loaded"));
            }
        } else if self.model.is_some() || self.api_key_env.is_some() {
            return Err(invalid("model and api_key_env need embeddings"));
        }
        if let Some(address) = &self.llm_endpoint {
            let parsed =
                url::Url::parse(address).map_err(|_| invalid("llm_endpoint is not an address"))?;
            if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
                return Err(invalid("llm_endpoint is an http or https address"));
            }
            if self.llm_model.as_deref().is_none_or(str::is_empty) {
                return Err(invalid("llm_endpoint needs the model its server loaded"));
            }
        } else if self.llm_model.is_some() || self.llm_api_key_env.is_some() {
            return Err(invalid("llm_model and llm_api_key_env need llm_endpoint"));
        }
        if self
            .llm_candidates
            .is_some_and(|count| !(1..=100).contains(&count))
        {
            return Err(invalid("llm_candidates is 1 to 100"));
        }
        if self.llm_timeout_secs == Some(0) {
            return Err(invalid("llm_timeout_secs is at least 1"));
        }
        let chunk = self.chunk_bytes.unwrap_or(1024);
        if !(4..=65536).contains(&chunk) {
            return Err(invalid("chunk_bytes is 4 to 65536"));
        }
        let prefix = self.document_prefix.as_deref().map_or(0, str::len);
        if chunk.saturating_sub(prefix) < 4
            || self.overlap_bytes.unwrap_or(0) >= chunk - prefix.min(chunk)
        {
            return Err(invalid(
                "overlap_bytes is below the room a passage has after its document prefix",
            ));
        }
        Ok(KbConfig {
            tools: self.tools,
            embeddings: self.embeddings,
            model: self.model,
            api_key_env: self.api_key_env,
            document_prefix: self.document_prefix,
            query_prefix: self.query_prefix,
            chunk_bytes: self.chunk_bytes,
            overlap_bytes: self.overlap_bytes,
            llm_endpoint: self.llm_endpoint,
            llm_model: self.llm_model,
            llm_api_key_env: self.llm_api_key_env,
            llm_candidates: self.llm_candidates,
            llm_timeout_secs: self.llm_timeout_secs,
        })
    }
}

/// `[host.mail]`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMail {
    tools: Option<bool>,
    #[serde(default)]
    accounts: IndexMap<String, RawMailAccount>,
}

/// `[host.mail.accounts.NAME]`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMailAccount {
    backend: Option<String>,
    email: Option<String>,
    server: Option<String>,
    autodiscover: Option<String>,
    auth: Option<String>,
    user: Option<String>,
    password_env: Option<String>,
    id: Option<String>,
    profile: Option<String>,
    command: Option<Vec<String>>,
    client_id: Option<String>,
    tenant: Option<String>,
    timeout_secs: Option<u64>,
}

/// Whether `address` is an http or https address a mailbox may be reached at: https anywhere, http only on this computer, since
/// a password sent over plain http crosses the network in the clear.
fn mail_address(address: &str) -> bool {
    let Ok(parsed) = url::Url::parse(address) else {
        return false;
    };
    match (parsed.scheme(), parsed.host()) {
        ("https", Some(_)) => true,
        ("http", Some(url::Host::Domain(name))) => name.eq_ignore_ascii_case("localhost"),
        ("http", Some(url::Host::Ipv4(ip))) => ip.is_loopback(),
        ("http", Some(url::Host::Ipv6(ip))) => ip.is_loopback(),
        _ => false,
    }
}

impl RawMail {
    fn into_config(self) -> Result<MailConfig, ConfigError> {
        let mut accounts = IndexMap::new();
        for (name, raw) in self.accounts {
            let invalid = |what: &str| ConfigError::Invalid {
                entry: None,
                what: format!("[host.mail.accounts.{name}] {what}"),
            };
            if name.is_empty()
                || name.len() > 32
                || !name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
            {
                return Err(ConfigError::Invalid {
                    entry: None,
                    what: format!(
                        "[host.mail.accounts.{name}] the name is 1 to 32 lower-case letters, digits, - and _"
                    ),
                });
            }
            let backend = match raw.backend.as_deref() {
                Some("ews") => MailBackend::Ews,
                Some("graph") => MailBackend::Graph,
                Some("outlook") => MailBackend::Outlook,
                _ => return Err(invalid("backend is ews, graph or outlook")),
            };
            let timeout_secs = raw.timeout_secs.unwrap_or(60);
            if !(1..=3600).contains(&timeout_secs) {
                return Err(invalid("timeout_secs is 1 to 3600"));
            }
            if backend != MailBackend::Graph && (raw.client_id.is_some() || raw.tenant.is_some()) {
                return Err(invalid("client_id and tenant are for graph"));
            }
            let auth = match backend {
                MailBackend::Graph => {
                    if raw.autodiscover.is_some()
                        || raw.auth.is_some()
                        || raw.user.is_some()
                        || raw.password_env.is_some()
                        || raw.profile.is_some()
                    {
                        return Err(invalid(
                            "graph signs in as Windows does: it takes client_id, tenant, email, server and command, not autodiscover, auth, user, password_env or profile",
                        ));
                    }
                    if raw.client_id.as_deref().is_none_or(str::is_empty) {
                        return Err(invalid(
                            "graph needs the client_id of an app registration in Entra ID (a public client with the delegated permission Mail.ReadWrite), or use the outlook backend, which needs none",
                        ));
                    }
                    if raw.tenant.as_deref().is_some_and(str::is_empty) {
                        return Err(invalid("tenant is an id or a domain"));
                    }
                    if raw
                        .server
                        .as_deref()
                        .is_some_and(|address| !mail_address(address))
                    {
                        return Err(invalid(
                            "server is an https address (http only on this computer, since a token would cross the network in the clear)",
                        ));
                    }
                    if raw
                        .command
                        .as_ref()
                        .is_some_and(|c| c.first().is_none_or(String::is_empty))
                    {
                        return Err(invalid("command names a program"));
                    }
                    MailAuth::Windows
                }
                MailBackend::Outlook => {
                    if raw.email.is_some()
                        || raw.server.is_some()
                        || raw.autodiscover.is_some()
                        || raw.auth.is_some()
                        || raw.user.is_some()
                        || raw.password_env.is_some()
                        || raw.id.is_some()
                    {
                        return Err(invalid(
                            "outlook signs in as Outlook does: it takes profile and command, not email, server, autodiscover, auth, user, password_env or id",
                        ));
                    }
                    if raw
                        .command
                        .as_ref()
                        .is_some_and(|c| c.first().is_none_or(String::is_empty))
                    {
                        return Err(invalid("command names a program"));
                    }
                    MailAuth::Windows
                }
                MailBackend::Ews => {
                    if raw.profile.is_some() || raw.command.is_some() {
                        return Err(invalid("profile and command are for outlook"));
                    }
                    if raw.server.is_none() && raw.email.is_none() {
                        return Err(invalid(
                            "ews needs a server, or an email for Autodiscover to find it by",
                        ));
                    }
                    for (key, address) in
                        [("server", &raw.server), ("autodiscover", &raw.autodiscover)]
                    {
                        if address.as_deref().is_some_and(|a| !mail_address(a)) {
                            return Err(invalid(&format!(
                                "{key} is an https address (http only on this computer, since a password would cross the network in the clear)"
                            )));
                        }
                    }
                    let auth = match raw.auth.as_deref() {
                        Some("basic") => MailAuth::Basic,
                        Some("windows") => MailAuth::Windows,
                        None if raw.user.is_some() => MailAuth::Basic,
                        None => MailAuth::Windows,
                        Some(_) => return Err(invalid("auth is basic or windows")),
                    };
                    match auth {
                        MailAuth::Basic => {
                            if raw.user.as_deref().is_none_or(str::is_empty)
                                || (raw.password_env.as_deref().is_none_or(str::is_empty)
                                    && raw.id.as_deref().is_none_or(str::is_empty))
                            {
                                return Err(invalid(
                                    "basic needs a user and a password: `canoa mail add` stores one under an id, or password_env names the variable holding it",
                                ));
                            }
                        }
                        MailAuth::Windows => {
                            if raw.user.is_some() || raw.password_env.is_some() || raw.id.is_some()
                            {
                                return Err(invalid(
                                    "windows signs in as you: it takes no user, password_env or id",
                                ));
                            }
                        }
                    }
                    auth
                }
            };
            accounts.insert(
                name,
                MailAccount {
                    backend,
                    email: raw.email,
                    server: raw.server,
                    autodiscover: raw.autodiscover,
                    auth,
                    user: raw.user,
                    password_env: raw.password_env,
                    id: raw.id,
                    profile: raw.profile,
                    command: raw.command,
                    client_id: raw.client_id,
                    tenant: raw.tenant,
                    timeout_secs,
                },
            );
        }
        Ok(MailConfig {
            tools: self.tools,
            accounts,
        })
    }
}

/// `[host.native]`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawNative {
    enabled: Option<bool>,
    control: Option<Vec<String>>,
}

/// `[host.usage]`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHostUsage {
    keep_days: Option<u32>,
}

/// `[host.history]`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHistory {
    record: Option<bool>,
    max_session_bytes: Option<u64>,
    max_sessions: Option<usize>,
    max_total_bytes: Option<u64>,
}

/// `[host.control]`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawControl {
    listen: Option<String>,
    #[serde(default)]
    grants: IndexMap<String, Vec<String>>,
    max_records: Option<usize>,
    max_record_bytes: Option<usize>,
    max_upload_bytes: Option<u64>,
    max_upload_total_bytes: Option<u64>,
    max_uploads: Option<usize>,
}

/// `[host.accounts.NAME]`. A key it does not list is collected, so that it is refused by name and a credential written
/// there is never quoted.
#[derive(Deserialize)]
struct RawAccount {
    harness: Option<String>,
    kind: Option<String>,
    id: Option<String>,
    settings: Option<toml::Table>,
    #[serde(flatten)]
    rest: toml::Table,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWeb {
    dir: Option<String>,
    listen: Option<String>,
    cert: Option<String>,
    key: Option<String>,
    #[serde(default)]
    hosts: Vec<String>,
    #[serde(default)]
    connect: Vec<String>,
    mode: Option<String>,
    office: Option<bool>,
    #[serde(default)]
    office_frames: Vec<String>,
    office_local: Option<bool>,
    office_secret: Option<bool>,
    max_connections: Option<usize>,
    max_connections_per_address: Option<usize>,
    connection_seconds: Option<u64>,
    max_request_bytes: Option<usize>,
    max_requests: Option<usize>,
    request_seconds: Option<u64>,
    idle_seconds: Option<u64>,
    send_seconds: Option<u64>,
    immutable_seconds: Option<u64>,
}

/// The settings both tables share. serde does not hold `deny_unknown_fields` across `flatten`, so each table lists them.
struct RawCommon {
    listen: Option<String>,
    transport: Option<TransportSpec>,
    working_roots: Vec<String>,
    worktrees: Option<bool>,
    peers: Option<Vec<String>>,
    max_peers: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAgent {
    program: Option<String>,
    args: Option<Vec<String>>,
    harness: Option<String>,
    library: Option<String>,
    source: Option<String>,
    source_settings: Option<toml::Table>,
    import_search: Option<String>,
    settings: Option<toml::Table>,
    account_settings: Option<toml::Table>,
    executable: Option<String>,
    setting_sources: Option<Vec<String>>,
    cache_lifetime: Option<String>,
    shell: Option<String>,
    profiles: Option<Vec<String>>,
    own_profile: Option<bool>,
    accounts: Option<Vec<String>>,
    usage: Option<String>,
    title: Option<String>,
    shared: Option<bool>,
    shared_idle_seconds: Option<u64>,
    shared_permission_window_seconds: Option<u64>,
    shared_keep_log: Option<bool>,
    shared_log_entries: Option<usize>,
    shared_log_bytes: Option<usize>,
    listen: Option<String>,
    transport: Option<String>,
    transport_library: Option<String>,
    transport_source: Option<String>,
    transport_source_settings: Option<toml::Table>,
    transport_import_search: Option<String>,
    transport_settings: Option<toml::Table>,
    #[serde(default)]
    working_roots: Vec<String>,
    worktrees: Option<bool>,
    peers: Option<Vec<String>>,
    max_peers: Option<usize>,
}

/// A harness entry's keys for what its command line asks, taken out of [`RawAgent`].
struct RawAsks {
    setting_sources: Option<Vec<String>>,
}

/// `[host.extensions.NAME]`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawExtension {
    library: Option<String>,
    source: Option<String>,
    source_settings: Option<toml::Table>,
    import_search: Option<String>,
    provider: String,
    pin: Option<String>,
    sha256: Option<String>,
    root: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProvider {
    library: Option<String>,
    source: Option<String>,
    source_settings: Option<toml::Table>,
    import_search: Option<String>,
    settings: Option<toml::Table>,
    provider: String,
    max_sessions: Option<usize>,
    listen: Option<String>,
    transport: Option<String>,
    transport_library: Option<String>,
    transport_source: Option<String>,
    transport_source_settings: Option<toml::Table>,
    transport_import_search: Option<String>,
    transport_settings: Option<toml::Table>,
    #[serde(default)]
    working_roots: Vec<String>,
    peers: Option<Vec<String>>,
    max_peers: Option<usize>,
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

    fn from_raw(raw: RawFile, base: &Path) -> Result<Self, ConfigError> {
        let limit_phrases = raw.usage.and_then(|usage| usage.limit_phrases);
        let protected_paths = raw
            .permissions
            .and_then(|permissions| permissions.protected)
            .map(RawLines::lines);
        let local_agents = local_agents_of(&raw.agents)?;
        let remote_agents = remote_agents_of(&raw.agents, &local_agents)?;
        let mut local_profiles = local_profiles_of(raw.profile, &raw.overlays, &raw.agents, base)?;
        if limit_phrases
            .iter()
            .flatten()
            .any(|phrase| phrase.trim().is_empty())
        {
            return Err(ConfigError::Invalid {
                entry: None,
                what: "[usage] limit_phrases holds an empty phrase, which would match every failed turn".to_owned(),
            });
        }
        let skill_pins = skill_pins_of(raw.skill_pins)?;
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
        let Some(host) = raw.host else {
            return Ok(Self {
                limit_phrases,
                protected_paths,
                local_agents,
                remote_agents,
                local_profiles,
                skill_pins,
                ..Self::default()
            });
        };
        for origin in &host.origins {
            if origin.is_empty() || origin.chars().any(|c| c.is_control() || c.is_whitespace()) {
                return Err(ConfigError::Invalid {
                    entry: None,
                    what: "an origin is empty or holds spaces or control characters".to_owned(),
                });
            }
        }
        let directory = directory_of(base);
        let mut profiles = IndexMap::new();
        for (name, table) in host.profiles {
            let mut profile = profile_of(&name, table, &directory)?;
            profile.skill_pins.clone_from(&skill_pins);
            profile.options.clone_from(&options);
            profile.kb_choice.clone_from(&kb_choice);
            profile.protected.clone_from(&protected_paths);
            profiles.insert(name, profile);
        }
        let account_store = match host.account_store.as_deref() {
            None | Some("auto") => AccountStore::Auto,
            Some("os") => AccountStore::Os,
            Some("file") => AccountStore::File,
            Some(_) => {
                return Err(ConfigError::Invalid {
                    entry: None,
                    what: "account_store is auto, os or file".to_owned(),
                });
            }
        };
        if host.shell.as_deref().is_some_and(|shell| {
            shell.is_empty() || shell.contains(' ') || shell.chars().any(char::is_control)
        }) {
            return Err(ConfigError::Invalid {
                entry: None,
                what: "shell is the name or path of a program".to_owned(),
            });
        }
        let store = host
            .store
            .map_or_else(StoreConfig::default, |raw| StoreConfig {
                protected: raw.protected.map(RawLines::lines),
                credential_variables: raw.credential_variables.map(RawLines::lines),
                index_ttl: raw.index_ttl,
                index_cache_bytes: raw.index_cache_bytes,
                tools: raw.tools,
            });
        let network = host
            .network
            .map(|raw| network_of(raw, &directory))
            .transpose()?
            .unwrap_or_default();
        let kb = host
            .kb
            .map(RawKb::into_config)
            .transpose()?
            .unwrap_or_default();
        let mail = host
            .mail
            .map(RawMail::into_config)
            .transpose()?
            .unwrap_or_default();
        let delegate = host
            .delegate
            .map(RawDelegate::into_config)
            .transpose()?
            .unwrap_or_default();
        let mut accounts = IndexMap::new();
        for (name, raw) in host.accounts {
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
        let mut entries = Vec::new();
        for (name, agent) in host.agents {
            check_name(&name)?;
            let offered = agent.profiles.unwrap_or_default();
            let own_profile = agent.own_profile.unwrap_or(false);
            if own_profile && agent.peers.is_none() {
                return Err(invalid(
                    &name,
                    "own_profile gives the person's own layer to the keys that reach this entry: name them in peers",
                ));
            }
            if own_profile && !offered.is_empty() {
                return Err(invalid(
                    &name,
                    "own_profile is the person's own layer and cannot go with profiles",
                ));
            }
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
            let (shared_log_entries, shared_log_bytes) =
                (agent.shared_log_entries, agent.shared_log_bytes);
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
                if !accounts.contains_key(account) {
                    return Err(invalid(
                        &name,
                        "accounts names an account [host.accounts] does not hold",
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
                    &directory,
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
                (Some(program), None) => {
                    if program.is_empty() {
                        return Err(invalid(&name, "program is empty"));
                    }
                    if agent.executable.is_some() {
                        return Err(invalid(&name, "executable goes with harness, not program"));
                    }
                    for (set, key) in [
                        (asks.setting_sources.is_some(), "setting_sources"),
                        (agent.cache_lifetime.is_some(), "cache_lifetime"),
                        (agent.shell.is_some(), "shell"),
                        (agent.library.is_some(), "library"),
                        (agent.source.is_some(), "source"),
                        (agent.source_settings.is_some(), "source_settings"),
                        (agent.import_search.is_some(), "import_search"),
                        (agent.settings.is_some(), "settings"),
                        (agent.account_settings.is_some(), "account_settings"),
                        (!offered_accounts.is_empty(), "accounts"),
                    ] {
                        if set {
                            return Err(invalid(
                                &name,
                                &format!("{key} goes with harness, not program"),
                            ));
                        }
                    }
                    let usage = match agent.usage.as_deref() {
                        None => None,
                        Some(text) => Some(
                            UsageSource::named(text)
                                .ok_or_else(|| invalid(&name, "usage is antigravity"))?,
                        ),
                    };
                    EntryKind::Agent {
                        program: OsString::from(program),
                        args: agent
                            .args
                            .unwrap_or_default()
                            .into_iter()
                            .map(OsString::from)
                            .collect(),
                        profiles: offered,
                        usage,
                    }
                }
                (None, Some(harness)) => {
                    if agent.usage.is_some() {
                        return Err(invalid(&name, "usage goes with program, not harness"));
                    }
                    if harness.is_empty() {
                        return Err(invalid(&name, "harness must not be empty"));
                    }
                    if agent.args.is_some() {
                        return Err(invalid(&name, "args go with program; a harness takes none"));
                    }
                    if agent.executable.as_deref() == Some("") {
                        return Err(invalid(&name, "executable is empty"));
                    }
                    let origin = origin_of(
                        &name,
                        base,
                        &directory,
                        OriginKeys {
                            library: agent.library.as_deref(),
                            source: agent.source.as_deref(),
                            source_settings: agent.source_settings,
                            import_search: agent.import_search.as_deref(),
                        },
                    )?;
                    let built_in = is_built_in_harness(&harness);
                    if built_in && origin.is_some() {
                        return Err(invalid(
                            &name,
                            &format!(
                                "{} are built in: library and source load an outside provider under its own id",
                                built_in_harnesses().collect::<Vec<_>>().join(", ")
                            ),
                        ));
                    }
                    if origin.is_none() && !built_in {
                        return Err(invalid(
                            &name,
                            &format!(
                                "harness is {}, or the id of an outside provider that library or source loads",
                                built_in_harnesses().collect::<Vec<_>>().join(", ")
                            ),
                        ));
                    }
                    let settings = agent
                        .settings
                        .map(|table| settings_of(&name, table))
                        .transpose()?;
                    let account_settings = agent
                        .account_settings
                        .map(|table| settings_of(&name, table))
                        .transpose()?;
                    let settings = if built_in {
                        built_in_settings(
                            &name,
                            &harness,
                            settings,
                            agent.executable.is_some(),
                            &directory,
                        )?
                    } else {
                        settings
                    };
                    let options = if built_in {
                        harness_options(&name, &harness, &asks, settings.as_ref())?
                    } else {
                        for (set, key) in [
                            (asks.setting_sources.is_some(), "setting_sources"),
                            (agent.executable.is_some(), "executable"),
                            (agent.shell.is_some(), "shell"),
                        ] {
                            if set {
                                return Err(invalid(
                                    &name,
                                    &format!(
                                        "{key} goes with a built-in harness; an outside provider takes its own settings"
                                    ),
                                ));
                            }
                        }
                        HarnessOptions::default()
                    };
                    let options = HarnessOptions {
                        cache_lifetime,
                        ..options
                    };
                    // A path is canoa.cfg's, as a provider's library is: the command line runs in a session's directory,
                    // which a client chooses.
                    let executable = agent.executable.map(|executable| {
                        if executable.contains(['/', '\\']) {
                            directory.join(executable).into_os_string()
                        } else {
                            OsString::from(executable)
                        }
                    });
                    EntryKind::Harness {
                        harness,
                        origin,
                        executable,
                        options,
                        settings,
                        account_settings,
                        profiles: offered,
                        accounts: offered_accounts,
                    }
                }
                _ => {
                    return Err(invalid(
                        &name,
                        "an agent names a program or harness, one of the two",
                    ));
                }
            };
            let mut served = entry(name, kind, common)?;
            served.own_profile = own_profile;
            served.title = title;
            served.shared = shared;
            served.shared_idle = shared_idle;
            served.shared_permission_window = shared_permission_window;
            served.shared_keep_log = shared_keep_log;
            served.shared_log_entries = shared_log_entries;
            served.shared_log_bytes = shared_log_bytes;
            served.shell = shell;
            entries.push(served);
        }
        for (name, provider) in host.providers {
            check_name(&name)?;
            if provider.provider.is_empty() {
                return Err(invalid(&name, "provider must not be empty"));
            }
            let Some(origin) = origin_of(
                &name,
                base,
                &directory,
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
                    &directory,
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
                directory: directory.clone(),
                max_sessions,
            };
            entries.push(entry(name, kind, common)?);
        }
        let mut extensions = Vec::new();
        for (name, extension) in host.extensions {
            check_name(&name)?;
            if extension.provider.is_empty() {
                return Err(invalid(&name, "provider must not be empty"));
            }
            let Some(origin) = origin_of(
                &name,
                base,
                &directory,
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
            extensions.push(Extension {
                name,
                origin,
                provider: extension.provider,
                check,
            });
        }
        for (name, table) in host.tools {
            check_name(&name)?;
            let (common, spec) = tool_of(&name, base, &directory, table)?;
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
        let admits = host
            .web
            .as_ref()
            .is_some_and(|web| web.office == Some(true) && web.office_local == Some(true));
        let control = host
            .control
            .map(|raw| control_of(raw, admits))
            .transpose()?;
        let native = host.native.map(native_of).transpose()?.unwrap_or_default();
        let history = host
            .history
            .map(history_of)
            .transpose()?
            .unwrap_or_default();
        let usage = match host.usage {
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
        let web = host
            .web
            .map(|raw| web_of(raw, &directory, &entries))
            .transpose()?;
        Ok(Self {
            origins: host.origins,
            entries,
            profiles,
            web,
            limit_phrases,
            protected_paths,
            accounts,
            account_store,
            shell: host.shell,
            local_agents,
            remote_agents,
            local_profiles,
            control,
            native,
            history,
            usage,
            store,
            network,
            kb,
            mail,
            delegate,
            skill_pins,
            extensions,
        })
    }

    /// The entry called `name`.
    #[must_use]
    pub fn entry(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.name == name)
    }
}

/// `[host.history]`. Errors name a key, never a value.
fn history_of(raw: RawHistory) -> Result<HistoryConfig, ConfigError> {
    let refused = |what: &str| ConfigError::Invalid {
        entry: None,
        what: format!("host.history: {what}"),
    };
    let default = HistoryConfig::default();
    let config = HistoryConfig {
        record: raw.record.unwrap_or(default.record),
        max_session_bytes: raw.max_session_bytes.unwrap_or(default.max_session_bytes),
        max_sessions: raw.max_sessions.unwrap_or(default.max_sessions),
        max_total_bytes: raw.max_total_bytes.unwrap_or(default.max_total_bytes),
    };
    if config.max_session_bytes == 0 || config.max_sessions == 0 || config.max_total_bytes == 0 {
        return Err(refused(
            "max_session_bytes, max_sessions and max_total_bytes are at least 1; record = false records nothing",
        ));
    }
    if config.max_session_bytes > crate::history::MAX_SESSION_BYTES_CEILING {
        return Err(refused(
            "max_session_bytes is at most 256 MiB: a history is read whole to be moved",
        ));
    }
    Ok(config)
}

/// `[host.native]`. Errors name a key, never a value.
fn native_of(raw: RawNative) -> Result<NativeConfig, ConfigError> {
    let refused = |what: &str| ConfigError::Invalid {
        entry: None,
        what: format!("host.native: {what}"),
    };
    let default = NativeConfig::default();
    let control = match raw.control {
        None => default.control,
        Some(names) => {
            let mut set = BTreeSet::new();
            for name in names {
                set.insert(
                    crate::control::Grant::named(&name)
                        .ok_or_else(|| refused("control names a grant that does not exist"))?,
                );
            }
            // Changing what a program may be given or run reaches as far as the agent's own access.
            if set.iter().any(|grant| {
                matches!(
                    grant,
                    crate::control::Grant::ConfigWriteSessions
                        | crate::control::Grant::OverlayWrite
                        | crate::control::Grant::HostRestart
                        | crate::control::Grant::Upload
                        | crate::control::Grant::Terminal
                        | crate::control::Grant::VoiceSetup
                        | crate::control::Grant::Hosting
                )
            }) {
                return Err(refused(
                    "control cannot hold config.write.sessions, overlay.write, host.restart, upload, terminal, voice.setup or hosting: a browser key does not get them through the door",
                ));
            }
            set
        }
    };
    Ok(NativeConfig {
        enabled: raw.enabled.unwrap_or(default.enabled),
        control,
    })
}

/// `[host.control]`; errors name a key, never a value. `admits` is whether the add-in's own page is admitted (`[host.web] office_local`), whose key is not known
/// to canoa.cfg: a channel with no key under `grants` is then not one that turns every key away.
fn control_of(raw: RawControl, admits: bool) -> Result<ControlConfig, ConfigError> {
    let refused = |what: &str| ConfigError::Invalid {
        entry: None,
        what: format!("host.control: {what}"),
    };
    let listen = raw
        .listen
        .map(|text| Listen::parse(&text).map_err(|what| refused(&format!("listen: {what}"))))
        .transpose()?;
    let mut grants = IndexMap::new();
    for (peer, names) in raw.grants {
        let id = peer
            .parse::<PeerId>()
            .map_err(|_| refused("a key under grants is not a peer id"))?;
        let mut set = BTreeSet::new();
        for name in names {
            set.insert(crate::control::Grant::named(&name).ok_or_else(|| {
                if name == "store.write" {
                    refused("store.write is no grant: name store.programs to let a key install programs")
                } else {
                    refused("a grant is not one this host knows")
                }
            })?);
        }
        grants.insert(id, set);
    }
    let max_records = raw
        .max_records
        .unwrap_or(crate::control::DEFAULT_MAX_RECORDS);
    let max_record_bytes = raw
        .max_record_bytes
        .unwrap_or(crate::control::DEFAULT_MAX_RECORD_BYTES);
    if max_records == 0 || max_record_bytes == 0 {
        return Err(refused("max_records and max_record_bytes are at least 1"));
    }
    let max_upload_bytes = raw
        .max_upload_bytes
        .unwrap_or(crate::control::DEFAULT_MAX_UPLOAD_BYTES);
    let max_upload_total_bytes = raw
        .max_upload_total_bytes
        .unwrap_or(crate::control::DEFAULT_MAX_UPLOAD_TOTAL);
    let max_uploads = raw
        .max_uploads
        .unwrap_or(crate::control::DEFAULT_MAX_UPLOADS);
    if max_upload_bytes == 0 || max_upload_total_bytes == 0 || max_uploads == 0 {
        return Err(refused(
            "max_upload_bytes, max_upload_total_bytes and max_uploads are at least 1",
        ));
    }
    if listen.is_some() && grants.is_empty() && !admits {
        return Err(refused(
            "listen without a key under grants would turn every key away",
        ));
    }
    Ok(ControlConfig {
        listen,
        grants,
        max_records,
        max_record_bytes,
        max_upload_bytes,
        max_upload_total_bytes,
        max_uploads,
    })
}

fn invalid(name: &str, what: &str) -> ConfigError {
    ConfigError::Invalid {
        entry: Some(name.to_owned()),
        what: what.to_owned(),
    }
}

/// The harnesses the host carries: providers of `acp_harness` its bundled agents register in this process.
pub fn built_in_harnesses() -> impl Iterator<Item = &'static str> {
    crate::bundled::harnesses()
}

/// Whether the host itself carries the harness provider `id`.
#[must_use]
pub fn is_built_in_harness(id: &str) -> bool {
    crate::bundled::is_harness(id)
}

/// The keys that say where an outside provider's code is.
struct OriginKeys<'a> {
    library: Option<&'a str>,
    source: Option<&'a str>,
    source_settings: Option<toml::Table>,
    import_search: Option<&'a str>,
}

/// `pin` with `root`, or `sha256` alone: what the Store recorded of an extension it installed.
fn extension_check(
    name: &str,
    base: &Path,
    origin: &Origin,
    pin: Option<String>,
    sha256: Option<String>,
    root: Option<String>,
) -> Result<Option<ExtensionCheck>, ConfigError> {
    let hex = |text: &str| text.len() == 64 && text.bytes().all(|byte| byte.is_ascii_hexdigit());
    match (pin, sha256, root) {
        (None, None, None) => Ok(None),
        (Some(pin), None, Some(root)) => {
            if !pin.strip_prefix("sha256:").is_some_and(hex) {
                return Err(invalid(name, "pin is sha256: and 64 hexadecimal digits"));
            }
            if root.is_empty() {
                return Err(invalid(name, "root must not be empty"));
            }
            let root = base.join(root);
            Ok(Some(ExtensionCheck::Tree {
                root: std::path::absolute(&root).unwrap_or(root),
                pin,
            }))
        }
        (None, Some(sha256), None) => {
            if !hex(&sha256) {
                return Err(invalid(name, "sha256 is 64 hexadecimal digits"));
            }
            if !matches!(origin, Origin::Library(_)) {
                return Err(invalid(name, "sha256 goes with library, not source"));
            }
            Ok(Some(ExtensionCheck::File { sha256 }))
        }
        _ => Err(invalid(
            name,
            "pin goes with root, and sha256 stands alone: an extension has one or the other",
        )),
    }
}

/// Where an entry's provider is loaded from, `None` when the entry names no library or source. Relative paths are taken
/// from `base`, canoa.cfg's directory.
fn origin_of(
    name: &str,
    base: &Path,
    directory: &Path,
    keys: OriginKeys<'_>,
) -> Result<Option<Origin>, ConfigError> {
    let file = |path: &str| {
        if path.is_empty() {
            return Err(invalid(name, "library or source must not be empty"));
        }
        let path = base.join(path);
        Ok(std::path::absolute(&path).unwrap_or(path))
    };
    match (keys.library, keys.source, keys.source_settings) {
        (Some(_), Some(_), _) => Err(invalid(name, "library and source exclude each other")),
        (None, None, None) if keys.import_search.is_none() => Ok(None),
        (None, None, _) => Err(invalid(
            name,
            "source_settings and import_search go with source",
        )),
        (Some(_), None, Some(_)) => {
            Err(invalid(name, "source_settings go with source, not library"))
        }
        (Some(_), None, None) if keys.import_search.is_some() => {
            Err(invalid(name, "import_search goes with source, not library"))
        }
        (Some(library), None, None) => Ok(Some(Origin::Library(file(library)?))),
        (None, Some(source), settings) => {
            let mut settings = settings_of(name, settings.unwrap_or_default())?;
            absolute_paths(name, directory, &mut settings)?;
            let import_search = match keys.import_search {
                None => ImportSearch::default_for(&settings),
                Some(text) => ImportSearch::named(text)
                    .ok_or_else(|| invalid(name, "import_search is library or path"))?,
            };
            Ok(Some(Origin::Source {
                file: file(source)?,
                settings,
                import_search,
                import_search_named: keys.import_search.is_some(),
            }))
        }
    }
}

/// The `settings` of a built-in entry: a key the profile's access replaces is refused, since access is the same for every
/// harness, and a relative `executable` is made absolute against canoa.cfg's directory, as the entry's own key is, since the
/// command line runs in a session's directory, which a client chooses.
fn built_in_settings(
    name: &str,
    harness: &str,
    settings: Option<Settings>,
    executable_key: bool,
    directory: &Path,
) -> Result<Option<Settings>, ConfigError> {
    let Some(settings) = settings else {
        return Ok(None);
    };
    let _ = harness;
    let mut map = settings.as_map().clone();
    if executable_key && map.contains_key("executable") {
        return Err(invalid(
            name,
            "executable is set twice: as a key and in settings",
        ));
    }
    for key in [
        "permission_mode",
        "approval_policy",
        "sandbox",
        "access",
        "allow_client_access",
    ] {
        if map.contains_key(key) {
            return Err(invalid(
                name,
                &format!(
                    "settings.{key} is not taken: the access is the profile's (access = {{ sandbox, network, approvals }}), the same for every harness"
                ),
            ));
        }
    }
    match map.get("executable") {
        None => {}
        Some(serde_json::Value::String(path)) if path.is_empty() => {
            return Err(invalid(name, "settings.executable is empty"));
        }
        Some(serde_json::Value::String(path)) => {
            if path.contains(['/', '\\']) && !Path::new(path).is_absolute() {
                let absolute = directory.join(path).to_string_lossy().into_owned();
                map.insert("executable".to_owned(), serde_json::Value::String(absolute));
            }
        }
        Some(_) => return Err(invalid(name, "settings.executable must be text")),
    }
    Ok(Some(Settings::new(map)))
}

/// A built-in entry's own keys, each checked against what the provider takes, and refused for the others.
/// A key that `settings` also holds is refused: the entry would say it twice.
fn harness_options(
    name: &str,
    harness: &str,
    agent: &RawAsks,
    settings: Option<&Settings>,
) -> Result<HarnessOptions, ConfigError> {
    if agent.setting_sources.is_some() && harness != "claude" {
        return Err(invalid(
            name,
            &format!("setting_sources is not a setting of harness {harness}"),
        ));
    }
    if agent.setting_sources.is_some()
        && settings.is_some_and(|settings| settings.as_map().contains_key("setting_sources"))
    {
        return Err(invalid(
            name,
            "setting_sources is set twice: as a key and in settings",
        ));
    }
    if let Some(sources) = &agent.setting_sources
        && sources
            .iter()
            .any(|source| !SETTING_SOURCES.contains(&source.as_str()))
    {
        return Err(invalid(
            name,
            &format!("setting_sources holds only {}", SETTING_SOURCES.join(", ")),
        ));
    }
    Ok(HarnessOptions {
        setting_sources: agent.setting_sources.clone(),
        cache_lifetime: None,
    })
}

/// `base` made absolute; an empty one is the current directory, as a path joined to it is taken from there.
fn directory_of(base: &Path) -> PathBuf {
    if base.as_os_str().is_empty() {
        return std::env::current_dir().unwrap_or_default();
    }
    std::path::absolute(base).unwrap_or_else(|_| base.to_owned())
}

/// A TOML table as JSON. A date, which JSON has no type for, and a float that is not a number are refused.
fn settings_of(name: &str, table: toml::Table) -> Result<Settings, ConfigError> {
    object_of(table).map(Settings).ok_or_else(|| {
        invalid(
            name,
            "a setting holds a date, or a float that is not a number, which a provider cannot be given",
        )
    })
}

/// A `[host.profiles]` table: `overlays` and `allow_mcp` taken out, the rest a profile of the person's own layer. Errors
/// name the profile and a setting, never a value.
fn profile_of(
    name: &str,
    mut table: toml::Table,
    directory: &Path,
) -> Result<HostProfile, ConfigError> {
    let refused = |what: String| ConfigError::Invalid {
        entry: None,
        what: format!("profile {name}: {what}"),
    };
    if check_name(name).is_err() {
        return Err(ConfigError::Invalid {
            entry: None,
            what: format!(
                "a profile name has 1 to {MAX_NAME} letters, digits, hyphens or underscores"
            ),
        });
    }
    let overlays = match table.remove("overlays") {
        None => Vec::new(),
        Some(toml::Value::Array(items)) => items
            .into_iter()
            .map(|item| match item {
                toml::Value::String(text) if coqui_aos::valid_overlay_name(&text) => Ok(text),
                _ => Err(refused(
                    "overlays holds a name that is not an overlay's".to_owned(),
                )),
            })
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => return Err(refused("overlays has the wrong type".to_owned())),
    };
    let mut allow_mcp = BTreeMap::new();
    match table.remove("allow_mcp") {
        None => {}
        Some(toml::Value::Table(allowed)) => {
            for (server, digest) in allowed {
                let digest = match digest {
                    toml::Value::String(text) if is_digest(&text) => text,
                    _ => {
                        return Err(refused(format!(
                            "allow_mcp.{server} is not sha256: and 64 hex digits"
                        )));
                    }
                };
                if check_name(&server).is_err() {
                    return Err(refused(
                        "allow_mcp names a server that is not a server's name".to_owned(),
                    ));
                }
                allow_mcp.insert(server, digest);
            }
        }
        Some(_) => return Err(refused("allow_mcp has the wrong type".to_owned())),
    }
    let allow_hooks = match table.remove("allow_hooks") {
        None => BTreeMap::new(),
        Some(toml::Value::Table(allowed)) => {
            hook_allowances(allowed).map_err(|what| refused(format!("allow_hooks{what}")))?
        }
        Some(_) => return Err(refused("allow_hooks has the wrong type".to_owned())),
    };
    let profile =
        Profile::from_table(table, LayerKind::User).map_err(|error| refused(error.to_string()))?;
    Ok(HostProfile {
        profile,
        overlays,
        allow_mcp,
        allow_hooks,
        agent: None,
        off_mcp: BTreeSet::new(),
        directory: directory.to_owned(),
        skill_pins: BTreeMap::new(),
        options: BTreeMap::new(),
        kb_choice: BTreeMap::new(),
        chosen_skills: Vec::new(),
        loadouts: IndexMap::new(),
        default_loadout: None,
        client_loadouts: BTreeMap::new(),
        presets: IndexMap::new(),
        items: crate::profile::ItemSet::default(),
        session: None,
        protected: None,
    })
}

/// The top-level `[profile]`, `[overlays]` and each agent's `profile` and `mcp`. Errors name a table and a setting, never a value.
fn local_profiles_of(
    profile: Option<toml::Table>,
    overlays: &IndexMap<String, toml::Value>,
    agents: &IndexMap<String, toml::Value>,
    base: &Path,
) -> Result<LocalProfiles, ConfigError> {
    let invalid = |what: String| ConfigError::Invalid { entry: None, what };
    let directory = directory_of(base);
    let profile = profile
        .map(|table| {
            Profile::from_table(table, LayerKind::User)
                .map_err(|error| invalid(format!("profile: {error}")))
        })
        .transpose()?;
    let mut names = Vec::new();
    for name in overlays.keys() {
        if !coqui_aos::valid_overlay_name(name) {
            return Err(invalid(
                "overlays holds a name that is not an overlay's".to_owned(),
            ));
        }
        names.push(name.clone());
    }
    let mut by_agent = IndexMap::new();
    for (label, table) in agents {
        let Some(table) = table.as_table() else {
            continue;
        };
        let mut layer = AgentProfile::default();
        match table.get("profile") {
            None => {}
            Some(toml::Value::Table(own)) => {
                layer.profile = Some(
                    Profile::from_table(own.clone(), LayerKind::Agent)
                        .map_err(|error| invalid(format!("agents.{label}.profile: {error}")))?,
                );
            }
            Some(_) => return Err(invalid(format!("agents.{label}.profile is not a table"))),
        }
        match table.get("mcp") {
            None => {}
            Some(toml::Value::Table(choices)) => {
                for (server, choice) in choices {
                    let choice = match choice {
                        toml::Value::Boolean(true) => McpChoice::On,
                        toml::Value::Boolean(false) => McpChoice::Off,
                        toml::Value::String(digest) if is_digest(digest) => {
                            McpChoice::At(digest.clone())
                        }
                        _ => {
                            return Err(invalid(format!(
                                "agents.{label}.mcp.{server} is true, false or a sha256: digest"
                            )));
                        }
                    };
                    layer.mcp.insert(server.clone(), choice);
                }
            }
            Some(_) => return Err(invalid(format!("agents.{label}.mcp is not a table"))),
        }
        match table.get("hooks") {
            None => {}
            Some(toml::Value::Table(allowed)) => {
                layer.hooks = hook_allowances(allowed.clone())
                    .map_err(|what| invalid(format!("agents.{label}.hooks{what}")))?;
            }
            Some(_) => return Err(invalid(format!("agents.{label}.hooks is not a table"))),
        }
        if layer.profile.is_some() || !layer.mcp.is_empty() || !layer.hooks.is_empty() {
            by_agent.insert(label.clone(), layer);
        }
    }
    Ok(LocalProfiles {
        profile,
        overlays: names,
        agents: by_agent,
        directory,
        skill_pins: BTreeMap::new(),
        options: BTreeMap::new(),
        kb_choice: BTreeMap::new(),
        loadouts: IndexMap::new(),
        default_loadout: None,
        client_loadouts: BTreeMap::new(),
        presets: IndexMap::new(),
        protected: None,
    })
}

/// A table of overlay names and the tree digests their hooks are allowed at. The error completes a setting's name and never
/// quotes a value.
fn hook_allowances(allowed: toml::Table) -> Result<BTreeMap<String, String>, String> {
    let mut out = BTreeMap::new();
    for (overlay, digest) in allowed {
        if !coqui_aos::valid_overlay_name(&overlay) {
            return Err(" names something that is not an overlay's name".to_owned());
        }
        match digest {
            toml::Value::String(text) if is_digest(&text) => {
                out.insert(overlay, text);
            }
            _ => return Err(format!(".{overlay} is not sha256: and 64 hex digits")),
        }
    }
    Ok(out)
}

/// `[loadouts.NAME]`: each a description and lists of overlay, skill and server names. Errors name the table and the key, never
/// a value.
fn loadouts_of(
    raw: &IndexMap<String, toml::Value>,
) -> Result<IndexMap<String, Loadout>, ConfigError> {
    let invalid = |what: String| ConfigError::Invalid { entry: None, what };
    let mut out = IndexMap::new();
    for (name, table) in raw {
        if !crate::profile::entry_name(name) {
            return Err(invalid(
                "loadouts holds a name of more than letters, digits, - and _, or of over 64 characters".to_owned(),
            ));
        }
        let toml::Value::Table(table) = table else {
            return Err(invalid(format!("loadouts.{name} is not a table")));
        };
        let mut loadout = Loadout::default();
        for (key, value) in table {
            let refused = |what: &str| invalid(format!("loadouts.{name}.{key} {what}"));
            match key.as_str() {
                "description" => {
                    let toml::Value::String(text) = value else {
                        return Err(refused("is not text"));
                    };
                    loadout.description.clone_from(text);
                }
                "overlays" | "skills" | "mcp" => {
                    let toml::Value::Array(items) = value else {
                        return Err(refused("is not a list of names"));
                    };
                    let mut names: Vec<String> = Vec::new();
                    for item in items {
                        let toml::Value::String(item) = item else {
                            return Err(refused("is not a list of names"));
                        };
                        let fits = match key.as_str() {
                            "overlays" => coqui_aos::valid_overlay_name(item),
                            "skills" => crate::profile::hosted::fetched::confined(item),
                            _ => crate::profile::entry_name(item),
                        };
                        if !fits {
                            return Err(refused("names something that is not a name of its kind"));
                        }
                        if !names.iter().any(|held| held.eq_ignore_ascii_case(item)) {
                            names.push(item.clone());
                        }
                    }
                    match key.as_str() {
                        "overlays" => loadout.overlays = names,
                        "skills" => loadout.skills = names,
                        _ => loadout.mcp = names,
                    }
                }
                _ => {
                    return Err(refused(
                        "is not a loadout setting: description, overlays, skills and mcp are",
                    ));
                }
            }
        }
        out.insert(name.clone(), loadout);
    }
    Ok(out)
}

/// Whether `name` is one a profile may have: 1 to 64 of ASCII letters, digits, spaces, `-`, `_` and `.`, with no space at either
/// end. A profile is chosen by its name in a footer, so it may read as words.
fn preset_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with(' ')
        && !name.ends_with(' ')
        && name
            .chars()
            .all(|each| each.is_ascii_alphanumeric() || matches!(each, ' ' | '-' | '_' | '.'))
}

/// `[profiles.NAME]`: each a source, model, effort, mode, starting instructions, cache lifetime and loadout, all optional. Errors
/// name the table and the key, never a value. A loadout the file no longer holds is not refused here: a session that names the
/// preset starts with the default loadout and says so.
fn presets_of(
    raw: &IndexMap<String, toml::Value>,
) -> Result<IndexMap<String, Preset>, ConfigError> {
    let invalid = |what: String| ConfigError::Invalid { entry: None, what };
    let mut out = IndexMap::new();
    for (name, table) in raw {
        if !preset_name(name) {
            return Err(invalid(
                "profiles holds a name of more than letters, digits, spaces, - _ and ., or one that begins or ends with a space, or of over 64 characters".to_owned(),
            ));
        }
        let toml::Value::Table(table) = table else {
            return Err(invalid(format!("profiles.{name} is not a table")));
        };
        let mut preset = Preset::default();
        let mut scalars = toml::Table::new();
        for (key, value) in table {
            let refused = |what: &str| invalid(format!("profiles.{name}.{key} {what}"));
            if key == "items" {
                let toml::Value::Table(items) = value else {
                    return Err(refused("is not a table of items"));
                };
                for (id, values) in items {
                    let word = !id.is_empty()
                        && id.len() <= 32
                        && id.bytes().all(|byte| {
                            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'
                        });
                    if !word || crate::profile::items::SETTING_IDS.contains(&id.as_str()) {
                        return Err(invalid(format!(
                            "profiles.{name}.items.{id} is not the id of an item a provider adds"
                        )));
                    }
                    let toml::Value::Table(_) = values else {
                        return Err(invalid(format!(
                            "profiles.{name}.items.{id} is not a table"
                        )));
                    };
                    let Ok(serde_json::Value::Object(values)) = serde_json::to_value(values) else {
                        return Err(invalid(format!(
                            "profiles.{name}.items.{id} cannot be read"
                        )));
                    };
                    preset.items.insert(id.clone(), values);
                }
                continue;
            }
            if key == "access" {
                let mut only = toml::Table::new();
                only.insert(key.clone(), value.clone());
                let own = Profile::from_table(only, LayerKind::Session)
                    .map_err(|error| invalid(format!("profiles.{name}: {error}")))?;
                preset.access = own.access;
                continue;
            }
            let toml::Value::String(text) = value else {
                return Err(refused("is not text"));
            };
            if text.is_empty() || text.contains('\0') {
                return Err(refused("is empty or holds a NUL"));
            }
            match key.as_str() {
                "source" => preset.source = Some(text.clone()),
                "model" | "effort" | "mode" => {
                    scalars.insert(key.clone(), value.clone());
                }
                "context" => preset.context = Some(text.clone()),
                "cache_lifetime" => {
                    if cache_lifetime_named(text).is_none() {
                        return Err(refused("is not default, off, short or long"));
                    }
                    preset.cache_lifetime = Some(text.clone());
                }
                "loadout" => {
                    if !crate::profile::entry_name(text) {
                        return Err(refused("is not a loadout's name"));
                    }
                    preset.loadout = Some(text.clone());
                }
                _ => {
                    return Err(refused(
                        "is not a profile setting: source, model, effort, mode, access, context, cache_lifetime, loadout and items are",
                    ));
                }
            }
        }
        let own = Profile::from_table(scalars, LayerKind::Session)
            .map_err(|error| invalid(format!("profiles.{name}: {error}")))?;
        preset.model = own.model;
        preset.effort = own.effort;
        preset.mode = own.mode;
        out.insert(name.clone(), preset);
    }
    Ok(out)
}

/// The kinds of client that can tell the host which it is, in `initialize`'s `_meta`, and so have a default loadout of their own in
/// `[defaults.clients]`: a browser's side panel, and the Outlook, Word, Excel and PowerPoint task panes.
pub const CLIENT_KINDS: [&str; 5] = ["browser", "outlook", "word", "excel", "powerpoint"];

/// `[defaults.clients]`: the loadout a session whose client is of each kind starts with when it names none. A kind that is not
/// one of [`CLIENT_KINDS`], and a value that is not text, refuse the file. A name `[loadouts]` no longer holds (deleted by hand) does
/// not: a connection of that kind says so in its report and gets the default.
fn client_loadouts_of(
    defaults: Option<&toml::Table>,
) -> Result<BTreeMap<String, String>, ConfigError> {
    let invalid = |what: String| ConfigError::Invalid { entry: None, what };
    let Some(value) = defaults.and_then(|table| table.get("clients")) else {
        return Ok(BTreeMap::new());
    };
    let table = value
        .as_table()
        .ok_or_else(|| invalid("[defaults.clients] is a table".to_owned()))?;
    let mut out = BTreeMap::new();
    for (kind, value) in table {
        if !CLIENT_KINDS.contains(&kind.as_str()) {
            return Err(invalid(format!(
                "[defaults.clients] {kind} is not a kind of client: {}",
                CLIENT_KINDS.join(", ")
            )));
        }
        let named = value.as_str().ok_or_else(|| {
            invalid(format!(
                "[defaults.clients] {kind} is the name of a loadout"
            ))
        })?;
        out.insert(kind.clone(), named.to_owned());
    }
    Ok(out)
}

/// `[defaults] loadout`: the loadout a session with none named starts with. The rest of the table is the app's.
fn default_loadout_of(
    defaults: Option<&toml::Table>,
    loadouts: &IndexMap<String, Loadout>,
) -> Result<Option<String>, ConfigError> {
    let Some(value) = defaults.and_then(|table| table.get("loadout")) else {
        return Ok(None);
    };
    let named = value
        .as_str()
        .filter(|name| loadouts.contains_key(*name))
        .ok_or_else(|| ConfigError::Invalid {
            entry: None,
            what: "[defaults] loadout is not the name of a loadout in [loadouts]".to_owned(),
        })?;
    Ok(Some(named.to_owned()))
}

/// `[skill_pins]`: a skill's name (compared without regard to ASCII case) and the `sha256:` tree digest it must have.
fn skill_pins_of(table: toml::Table) -> Result<BTreeMap<String, String>, ConfigError> {
    let refused = |what: &str| ConfigError::Invalid {
        entry: None,
        what: format!("[skill_pins] {what}"),
    };
    let mut out = BTreeMap::new();
    for (name, value) in table {
        if !crate::profile::hosted::fetched::confined(&name) {
            return Err(refused("names something that is not a skill's name"));
        }
        let toml::Value::String(text) = value else {
            return Err(refused("holds a value that is not a digest"));
        };
        let Ok(digest) = coqui_aos::Digest::parse(&text) else {
            return Err(refused(
                "holds a value that is not sha256: and 64 hex digits",
            ));
        };
        if out
            .insert(name.to_ascii_lowercase(), digest.to_string())
            .is_some()
        {
            return Err(refused("names a skill twice, spelt in another case"));
        }
    }
    Ok(out)
}

/// `[overlays.NAME] options`: a table of option names and values, by overlay. Whether an option is declared and of what
/// type is for the overlay's manifest to say, so the errors name a table and never a value.
fn overlay_options_of(
    overlays: &IndexMap<String, toml::Value>,
) -> Result<BTreeMap<String, coqui_aos::Document>, ConfigError> {
    let mut out = BTreeMap::new();
    for (name, entry) in overlays {
        let Some(options) = entry.as_table().and_then(|entry| entry.get("options")) else {
            continue;
        };
        let invalid = || ConfigError::Invalid {
            entry: None,
            what: format!("overlays.{name}.options is not a table of values"),
        };
        let toml::Value::Table(table) = options else {
            return Err(invalid());
        };
        let Ok(serde_json::Value::Object(values)) = serde_json::to_value(table) else {
            return Err(invalid());
        };
        out.insert(name.clone(), values);
    }
    Ok(out)
}

/// `[overlays.NAME] kb`: `"index"` serves the overlay's knowledge through the `canoa-kb` tools, `"files"` puts it in the context
/// whole. An overlay with no key is absent: its own `[knowledge] serve` hint decides.
fn overlay_kb_of(
    overlays: &IndexMap<String, toml::Value>,
) -> Result<BTreeMap<String, bool>, ConfigError> {
    let mut out = BTreeMap::new();
    for (name, entry) in overlays {
        let Some(choice) = entry.as_table().and_then(|entry| entry.get("kb")) else {
            continue;
        };
        match choice.as_str() {
            Some("index") => {
                out.insert(name.clone(), true);
            }
            Some("files") => {
                out.insert(name.clone(), false);
            }
            _ => {
                return Err(ConfigError::Invalid {
                    entry: None,
                    what: format!("overlays.{name}.kb is \"index\" or \"files\""),
                });
            }
        }
    }
    Ok(out)
}

/// `sha256:` and 64 lower-case hex digits.
fn is_digest(text: &str) -> bool {
    text.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    })
}

/// The source settings read as lists of paths: `dirs` and `manifests` by every Guatiao bridge source, `paths` by Python's,
/// `import_dirs` by the registry. A source may resolve a relative one against whatever working directory it has later, or
/// against its own file's, so each is made absolute here.
const PATH_LISTS: [&str; 4] = ["dirs", "manifests", "paths", "import_dirs"];

/// The source settings read as one path: `home`, the Python installation Python's source runs from.
const PATHS: [&str; 1] = ["home"];

fn absolute_paths(
    name: &str,
    directory: &Path,
    settings: &mut Settings,
) -> Result<(), ConfigError> {
    let absolute = |text: &mut String| -> Result<(), ConfigError> {
        let path = directory.join(&*text);
        let path = std::path::absolute(&path).unwrap_or(path);
        *text = path
            .to_str()
            .ok_or_else(|| invalid(name, "canoa.cfg's directory is not Unicode, so a relative path cannot be made absolute"))?
            .to_owned();
        Ok(())
    };
    for key in PATH_LISTS {
        let Some(serde_json::Value::Array(items)) = settings.0.get_mut(key) else {
            continue;
        };
        for item in items {
            if let serde_json::Value::String(text) = item {
                absolute(text)?;
            }
        }
    }
    for key in PATHS {
        if let Some(serde_json::Value::String(text)) = settings.0.get_mut(key) {
            absolute(text)?;
        }
    }
    Ok(())
}

fn object_of(table: toml::Table) -> Option<serde_json::Map<String, serde_json::Value>> {
    table
        .into_iter()
        .map(|(key, value)| Some((key, json_of(value)?)))
        .collect()
}

fn json_of(value: toml::Value) -> Option<serde_json::Value> {
    use serde_json::Value as Json;
    Some(match value {
        toml::Value::String(text) => Json::String(text),
        toml::Value::Integer(number) => Json::from(number),
        toml::Value::Float(number) => Json::Number(serde_json::Number::from_f64(number)?),
        toml::Value::Boolean(flag) => Json::Bool(flag),
        toml::Value::Datetime(_) => return None,
        toml::Value::Array(items) => {
            Json::Array(items.into_iter().map(json_of).collect::<Option<_>>()?)
        }
        toml::Value::Table(table) => Json::Object(object_of(table)?),
    })
}

/// A tool entry: the settings every entry has, taken out by hand since serde does not hold `deny_unknown_fields` across
/// `flatten`, then the rest as the tool's. Errors name a setting, never a value.
fn tool_of(
    name: &str,
    base: &Path,
    directory: &Path,
    mut table: toml::Table,
) -> Result<(RawCommon, ToolSpec), ConfigError> {
    let wrong = |key: &str| invalid(name, &format!("{key} has the wrong type"));
    let text = |value: toml::Value, key: &str| match value {
        toml::Value::String(text) => Ok(text),
        _ => Err(wrong(key)),
    };
    let texts = |value: toml::Value, key: &str| match value {
        toml::Value::Array(items) => items.into_iter().map(|item| text(item, key)).collect(),
        _ => Err(wrong(key)),
    };
    let listen = table
        .remove("listen")
        .map(|value| text(value, "listen"))
        .transpose()?;
    let tabled = |value: toml::Value, key: &str| match value {
        toml::Value::Table(table) => Ok(table),
        _ => Err(wrong(key)),
    };
    let transport_id = table
        .remove("transport")
        .map(|value| text(value, "transport"))
        .transpose()?;
    let transport_library = table
        .remove("transport_library")
        .map(|value| text(value, "transport_library"))
        .transpose()?;
    let transport_source = table
        .remove("transport_source")
        .map(|value| text(value, "transport_source"))
        .transpose()?;
    let transport_import_search = table
        .remove("transport_import_search")
        .map(|value| text(value, "transport_import_search"))
        .transpose()?;
    let transport_source_settings = table
        .remove("transport_source_settings")
        .map(|value| tabled(value, "transport_source_settings"))
        .transpose()?;
    let transport_settings = table
        .remove("transport_settings")
        .map(|value| tabled(value, "transport_settings"))
        .transpose()?;
    let transport = transport_of(
        name,
        base,
        directory,
        TransportKeys {
            id: transport_id,
            library: transport_library,
            source: transport_source,
            source_settings: transport_source_settings,
            import_search: transport_import_search,
            settings: transport_settings,
        },
    )?;
    let working_roots = table
        .remove("working_roots")
        .map(|value| texts(value, "working_roots"))
        .transpose()?
        .unwrap_or_default();
    let worktrees = match table.remove("worktrees") {
        None => None,
        Some(toml::Value::Boolean(on)) => Some(on),
        Some(_) => return Err(wrong("worktrees")),
    };
    let peers = table
        .remove("peers")
        .map(|value| texts(value, "peers"))
        .transpose()?;
    let max_peers = match table.remove("max_peers") {
        None => None,
        Some(toml::Value::Integer(count)) => Some(
            usize::try_from(count).map_err(|_| invalid(name, "max_peers must be at least 1"))?,
        ),
        Some(_) => return Err(wrong("max_peers")),
    };
    let spec: ToolSpec =
        toml::Value::Table(table)
            .try_into()
            .map_err(|error: toml::de::Error| {
                let message = error.message();
                let what = if message.starts_with("missing field") {
                    message.to_string()
                } else if message.starts_with("unknown field") {
                    "a setting this host does not know".to_owned()
                } else if message.starts_with("unknown variant") {
                    "a setting has a value this host does not know".to_owned()
                } else {
                    "a setting has the wrong type".to_owned()
                };
                invalid(name, &what)
            })?;
    spec.check().map_err(|what| invalid(name, &what))?;
    let common = RawCommon {
        listen,
        transport,
        working_roots,
        worktrees,
        peers,
        max_peers,
    };
    Ok((common, spec))
}

/// A title is shown to a person in place of the name: not blank, no control characters, at most [`MAX_TITLE`] characters.
fn check_title(name: &str, title: &str) -> Result<(), ConfigError> {
    if title.trim().is_empty() {
        return Err(invalid(name, "title is blank"));
    }
    if title.chars().any(char::is_control) {
        return Err(invalid(name, "title holds a control character"));
    }
    if title.chars().count() > MAX_TITLE {
        return Err(invalid(
            name,
            &format!("title is longer than {MAX_TITLE} characters"),
        ));
    }
    Ok(())
}

/// One `[host.accounts.NAME]`. Errors name a key, never a value: a credential written here is refused, and not quoted.
fn account_of(name: &str, raw: RawAccount) -> Result<AccountConfig, ConfigError> {
    check_name(name)?;
    let refused = |what: String| ConfigError::Invalid {
        entry: None,
        what: format!("account {name}: {what}"),
    };
    if let Some(key) = raw.rest.keys().next() {
        return Err(refused(format!(
            "{key} is not a key of an account, and a credential never goes in canoa.cfg: `canoa accounts add` keeps it in the account store"
        )));
    }
    let text = |value: Option<String>, key: &str| match value {
        Some(value) if !value.is_empty() => Ok(value),
        _ => Err(refused(format!("{key} is missing or empty"))),
    };
    let id = text(raw.id, "id")?;
    if !coqui_acp_api::is_account_id(&id) {
        return Err(refused(
            "id is 1 to 128 bytes with no control character".to_owned(),
        ));
    }
    let settings = raw
        .settings
        .map(|table| settings_of(name, table))
        .transpose()?;
    Ok(AccountConfig {
        harness: text(raw.harness, "harness")?,
        kind: text(raw.kind, "kind")?,
        id,
        settings,
    })
}

/// Checked before anything quotes the name, since a key in the file may hold any character.
pub(crate) fn check_name(name: &str) -> Result<(), ConfigError> {
    if name.is_empty()
        || name.len() > MAX_NAME
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(ConfigError::Invalid {
            entry: None,
            what: format!(
                "an entry name has 1 to {MAX_NAME} letters, digits, hyphens or underscores"
            ),
        });
    }
    Ok(())
}

/// `[agents.NAME]` tables that name a `program`, as entries reached over [`crate::Local`] only. A table with no `program`
/// (the app keeps an agent's profile there) names no command and is let go by.
fn local_agents_of(agents: &IndexMap<String, toml::Value>) -> Result<Vec<Entry>, ConfigError> {
    let mut out = Vec::new();
    for (name, table) in agents {
        let Some(table) = table.as_table() else {
            continue;
        };
        let Some(program) = table.get("program") else {
            continue;
        };
        // A name the app shows, so any text a person can read: it is never part of an address.
        if name.is_empty() || name.chars().count() > MAX_NAME || name.chars().any(char::is_control)
        {
            return Err(ConfigError::Invalid {
                entry: None,
                what: format!(
                    "an agent's name has 1 to {MAX_NAME} characters, none of them control characters"
                ),
            });
        }
        let bad = |what: &str| ConfigError::Invalid {
            entry: Some(name.clone()),
            what: format!("agents.{name}: {what}"),
        };
        if name == crate::control::NAME {
            return Err(bad("that name is the host's control channel"));
        }
        let program = program
            .as_str()
            .filter(|program| !program.is_empty())
            .ok_or_else(|| bad("program is not text"))?;
        let args: Vec<String> = match table.get("args") {
            None => Vec::new(),
            Some(toml::Value::Array(args)) => args
                .iter()
                .map(|arg| arg.as_str().map(str::to_owned))
                .collect::<Option<_>>()
                .ok_or_else(|| bad("args is not a list of text"))?,
            Some(_) => return Err(bad("args is not a list of text")),
        };
        let usage = (is_agy(program) && args == ["acp"]).then_some(UsageSource::Antigravity);
        let mut entry = entry(
            name.clone(),
            EntryKind::Agent {
                program: OsString::from(program),
                args: args.into_iter().map(OsString::from).collect(),
                profiles: Vec::new(),
                usage,
            },
            RawCommon {
                listen: None,
                transport: None,
                working_roots: Vec::new(),
                worktrees: None,
                peers: None,
                max_peers: None,
            },
        )?;
        entry.local = true;
        out.push(entry);
    }
    Ok(out)
}

/// `[agents.NAME]` tables that carry an `a2a` table: agents on another machine. A name a program has is not reused, and one
/// table is not both.
fn remote_agents_of(
    agents: &IndexMap<String, toml::Value>,
    local: &[Entry],
) -> Result<Vec<RemoteAgent>, ConfigError> {
    let mut out = Vec::new();
    for (name, table) in agents {
        let Some(table) = table.as_table() else {
            continue;
        };
        let Some(remote) = table.get("a2a") else {
            continue;
        };
        let bad = |what: &str| ConfigError::Invalid {
            entry: Some(name.clone()),
            what: format!("agents.{name}.a2a: {what}"),
        };
        if name.is_empty() || name.chars().count() > MAX_NAME || name.chars().any(char::is_control)
        {
            return Err(bad(
                "the name is empty, too long or holds a control character",
            ));
        }
        if table.contains_key("program") || local.iter().any(|entry| &entry.name == name) {
            return Err(bad("an agent is a program or another machine's, not both"));
        }
        let remote = remote
            .as_table()
            .ok_or_else(|| bad("is not a table of url and peer"))?;
        if let Some(key) = remote
            .keys()
            .find(|key| !["url", "peer", "source", "title"].contains(&key.as_str()))
        {
            return Err(bad(&format!("{key} is not a key of it")));
        }
        let text = |key: &str| -> Result<Option<&str>, ConfigError> {
            match remote.get(key) {
                None => Ok(None),
                Some(toml::Value::String(text)) if !text.is_empty() => Ok(Some(text)),
                Some(_) => Err(bad(&format!("{key} is not text"))),
            }
        };
        let url = text("url")?.ok_or_else(|| bad("url is needed"))?;
        if !url.starts_with("ws://") {
            return Err(bad("url is a ws:// address"));
        }
        let peer = text("peer")?
            .ok_or_else(|| bad("peer is needed: the key the other host proves"))?
            .parse::<PeerId>()
            .map_err(|_| bad("peer is not a peer id"))?;
        let title = text("title")?.map(str::to_owned);
        if let Some(title) = &title {
            check_title(name, title)?;
        }
        out.push(RemoteAgent {
            name: name.clone(),
            title,
            url: url.to_owned(),
            peer,
            source: text("source")?.map(str::to_owned),
        });
    }
    Ok(out)
}

/// Whether `program` is Antigravity's `agy`, whatever its directory or extension: its quota is read beside its turns.
fn is_agy(program: &str) -> bool {
    let name = program.rsplit(['/', '\\']).next().unwrap_or(program);
    let stem = name.rsplit_once('.').map_or(name, |(stem, extension)| {
        if matches!(
            extension.to_ascii_lowercase().as_str(),
            "exe" | "cmd" | "bat"
        ) {
            stem
        } else {
            name
        }
    });
    stem.eq_ignore_ascii_case("agy")
}

/// The keys of an entry that name a `transport` provider and where it loads from.
struct TransportKeys {
    id: Option<String>,
    library: Option<String>,
    source: Option<String>,
    source_settings: Option<toml::Table>,
    import_search: Option<String>,
    settings: Option<toml::Table>,
}

/// An entry's `transport` provider, before its endpoint is joined to it.
struct TransportSpec {
    id: String,
    origin: Option<Origin>,
    settings: Option<Settings>,
}

/// The transport an entry names, `None` when it names none. Relative paths are taken from `base`, as a harness's are; the id is
/// `pipe` or `websocket` (built in) or an outside provider's, which `transport_library` or `transport_source` loads.
fn transport_of(
    name: &str,
    base: &Path,
    directory: &Path,
    keys: TransportKeys,
) -> Result<Option<TransportSpec>, ConfigError> {
    let Some(id) = keys.id else {
        let stray = [
            (keys.library.is_some(), "transport_library"),
            (keys.source.is_some(), "transport_source"),
            (keys.source_settings.is_some(), "transport_source_settings"),
            (keys.import_search.is_some(), "transport_import_search"),
            (keys.settings.is_some(), "transport_settings"),
        ];
        return match stray.iter().find(|(set, _)| *set) {
            Some((_, key)) => Err(invalid(name, &format!("{key} goes with transport"))),
            None => Ok(None),
        };
    };
    if id.is_empty() {
        return Err(invalid(name, "transport must not be empty"));
    }
    let origin = origin_of(
        name,
        base,
        directory,
        OriginKeys {
            library: keys.library.as_deref(),
            source: keys.source.as_deref(),
            source_settings: keys.source_settings,
            import_search: keys.import_search.as_deref(),
        },
    )
    .map_err(|error| match error {
        ConfigError::Invalid { entry, what } => ConfigError::Invalid {
            entry,
            what: format!("transport: {what}"),
        },
        other => other,
    })?;
    let built_in = crate::transport::BUILT_IN_TRANSPORTS.contains(&id.as_str());
    if built_in && origin.is_some() {
        return Err(invalid(
            name,
            "pipe and websocket are built in: transport_library and transport_source load an outside provider under its own id",
        ));
    }
    if origin.is_none() && !built_in {
        return Err(invalid(
            name,
            "transport is pipe, websocket, or the id of an outside provider that transport_library or transport_source loads",
        ));
    }
    let settings = keys
        .settings
        .map(|table| settings_of(name, table))
        .transpose()?;
    Ok(Some(TransportSpec {
        id,
        origin,
        settings,
    }))
}

fn entry(name: String, kind: EntryKind, common: RawCommon) -> Result<Entry, ConfigError> {
    let listen = match (common.transport, common.listen) {
        (None, None) => Listen::Address(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0)),
        (None, Some(text)) => Listen::parse(&text).map_err(|what| invalid(&name, &what))?,
        (Some(_), None) => {
            return Err(invalid(
                &name,
                "listen is the endpoint the transport listens on, and is required with transport",
            ));
        }
        (Some(spec), Some(endpoint)) => {
            if endpoint.is_empty() {
                return Err(invalid(&name, "listen is empty"));
            }
            Listen::Transport(alloc::boxed::Box::new(TransportListen {
                id: spec.id,
                origin: spec.origin,
                settings: spec.settings,
                endpoint,
            }))
        }
    };
    let mut working_roots = Vec::new();
    for root in common.working_roots {
        let root = PathBuf::from(root);
        if !plain_absolute(&root) {
            return Err(invalid(
                &name,
                "a working root must be an absolute path with no . or .. in it",
            ));
        }
        working_roots.push(root);
    }
    let peers = common
        .peers
        .map(|peers| {
            peers
                .iter()
                .map(|peer| peer.parse::<PeerId>())
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| invalid(&name, "a peer is not a peer id"))
        })
        .transpose()?;
    let max_peers = common.max_peers.unwrap_or(DEFAULT_MAX_PEERS);
    if max_peers == 0 {
        return Err(invalid(&name, "max_peers must be at least 1"));
    }
    Ok(Entry {
        name,
        kind,
        listen,
        working_roots,
        worktrees: common.worktrees.unwrap_or(true),
        peers,
        max_peers,
        title: None,
        own_profile: false,
        local: false,
        shared: false,
        shared_idle: None,
        shared_permission_window: None,
        shared_keep_log: true,
        shared_log_entries: None,
        shared_log_bytes: None,
        shell: None,
    })
}

/// `[host.web]`, its paths made absolute from canoa.cfg's directory. Errors name a setting, never a value.
fn web_of(raw: RawWeb, directory: &Path, entries: &[Entry]) -> Result<WebConfig, ConfigError> {
    let refused = |what: &str| ConfigError::Invalid {
        entry: None,
        what: format!("web: {what}"),
    };
    let path = |text: String, setting: &str| {
        if text.is_empty() {
            return Err(refused(&format!("{setting} is empty")));
        }
        let path = directory.join(text);
        Ok(std::path::absolute(&path).unwrap_or(path))
    };
    let mut web = WebConfig::new();
    web.dir = raw.dir.map(|dir| path(dir, "dir")).transpose()?;
    if let Some(listen) = raw.listen {
        web.listen = listen
            .parse()
            .map_err(|_| refused("listen is not an address and port"))?;
    }
    web.tls = match (raw.cert, raw.key) {
        (Some(cert), Some(key)) => Some(WebTls::new(path(cert, "cert")?, path(key, "key")?)),
        (None, None) => None,
        _ => return Err(refused("cert and key go together")),
    };
    // Unlike an entry's link, the page is not sealed: whoever is on the path can rewrite the code that holds the person's
    // key, and a browser gives plain HTTP no secure context for WebCrypto.
    if web.tls.is_none() && !web.listen.ip().is_loopback() {
        return Err(refused(
            "listen is beyond loopback, where plain HTTP is refused: give cert and key",
        ));
    }
    for host in raw.hosts {
        let plain = !host.is_empty()
            && host.len() <= 253
            && host
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'.');
        if !plain {
            return Err(refused(
                "a host is not a name of letters, digits, hyphens and dots",
            ));
        }
        web.hosts.push(host.to_ascii_lowercase());
    }
    for source in raw.connect {
        if !crate::web::connect_source(&source) {
            return Err(refused(
                "a connect source is not http, https, ws or wss and a host, with an optional port and path",
            ));
        }
        web.connect.push(source);
    }
    web.office = raw.office.unwrap_or(false);
    if !raw.office_frames.is_empty() && !web.office {
        return Err(refused("office_frames needs office = true"));
    }
    for source in raw.office_frames {
        if !crate::web::frame_source(&source) {
            return Err(refused(
                "an office_frames source is not https, a host (a leading *. is allowed) and an optional port",
            ));
        }
        web.office_frames.push(source);
    }
    web.office_local = raw.office_local.unwrap_or(false);
    if web.office_local && !web.office {
        return Err(refused("office_local needs office = true"));
    }
    if web.office_local && !web.listen.ip().is_loopback() {
        return Err(refused(
            "office_local needs a loopback listen address: behind another address or a proxy every request looks local",
        ));
    }
    web.office_secret = raw.office_secret.unwrap_or(false);
    if web.office_secret && !web.office_local {
        return Err(refused("office_secret needs office_local = true"));
    }
    web.mode = match raw.mode.as_deref() {
        None => None,
        Some("local") => Some(WebMode::Local),
        Some("remote") => Some(WebMode::Remote),
        Some(_) => return Err(refused("mode is local or remote")),
    };
    let count = |value: Option<usize>, setting: &str, default: usize| match value {
        Some(0) => Err(refused(&format!("{setting} must be at least 1"))),
        Some(value) => Ok(value),
        None => Ok(default),
    };
    let seconds = |value: Option<u64>, setting: &str, default: Duration| match value {
        Some(0) => Err(refused(&format!("{setting} must be at least 1"))),
        Some(value) => Ok(Duration::from_secs(value)),
        None => Ok(default),
    };
    let limits = web.limits;
    web.limits = limits
        .with_max_connections(count(
            raw.max_connections,
            "max_connections",
            limits.max_connections,
        )?)
        .with_max_connections_per_address(count(
            raw.max_connections_per_address,
            "max_connections_per_address",
            limits.max_connections_per_address,
        )?)
        .with_connection_time(seconds(
            raw.connection_seconds,
            "connection_seconds",
            limits.connection_time,
        )?)
        .with_max_request_bytes(count(
            raw.max_request_bytes,
            "max_request_bytes",
            limits.max_request_bytes,
        )?)
        .with_max_requests(count(
            raw.max_requests,
            "max_requests",
            limits.max_requests,
        )?)
        .with_request_time(seconds(
            raw.request_seconds,
            "request_seconds",
            limits.request_time,
        )?)
        .with_idle_time(seconds(raw.idle_seconds, "idle_seconds", limits.idle_time)?)
        .with_send_time(seconds(raw.send_seconds, "send_seconds", limits.send_time)?)
        .with_immutable_age(
            raw.immutable_seconds
                .map_or(limits.immutable_age, Duration::from_secs),
        );
    if web.listen.port() != 0
        && entries
            .iter()
            .any(|entry| entry.listen.address() == Some(web.listen))
    {
        return Err(refused("listen is an entry's address too"));
    }
    Ok(web)
}

fn check_unique(entries: &[Entry]) -> Result<(), ConfigError> {
    for (index, entry) in entries.iter().enumerate() {
        for earlier in &entries[..index] {
            if earlier.name == entry.name {
                return Err(invalid(&entry.name, "two entries share this name"));
            }
            if entry.listen.is_fixed() && earlier.listen.same_place(&entry.listen) {
                return Err(invalid(
                    &entry.name,
                    "another entry listens on the same address",
                ));
            }
        }
    }
    Ok(())
}

/// Absolute, with no `.` or `..`: such a path is compared as written, so one of those could step outside it. The text is
/// checked as well as the components, since a Unix path's components drop a `.` in the middle.
pub(crate) fn plain_absolute(path: &Path) -> bool {
    path.is_absolute()
        && path
            .components()
            .all(|component| !matches!(component, Component::CurDir | Component::ParentDir))
        && !path
            .to_string_lossy()
            .split(['/', '\\'])
            .any(|segment| segment == "." || segment == "..")
}

fn read_raw(bytes: &[u8], name: Option<&str>) -> Result<RawFile, anycfg::Error> {
    let options = name.map_or_else(anycfg::Options::new, |name| {
        anycfg::Options::new().name(name)
    });
    formats().from_slice_typed::<RawFile>(bytes, options)
}

/// anycfg's refusal as ours: the position or the key's path, and its words only where they come from the schema. A value,
/// or a key nobody declared, is never told.
fn config_error(error: &anycfg::Error, file: Option<&Path>) -> ConfigError {
    use anycfg::Error;
    let parse = |file: Option<&Path>, line, column, what: String| ConfigError::Parse {
        file: file.map(Path::to_owned),
        line,
        column,
        what,
    };
    match error {
        Error::Source { cause, .. } => config_error(cause, file),
        Error::File { path, cause } => config_error(cause, Some(path)),
        Error::Parse {
            format,
            line,
            column,
            message,
        } => parse(
            file,
            *line,
            *column,
            message
                .clone()
                .unwrap_or_else(|| format!("not valid {}", format.to_uppercase())),
        ),
        Error::Conversion { path, message } => {
            let message = message.as_deref().unwrap_or("not of the expected shape");
            let (path, what) = if message.starts_with("unknown field") {
                // The unknown key is the last of the path, and is not told.
                let parent = path
                    .as_deref()
                    .and_then(|path| path.rsplit_once('.'))
                    .map(|(parent, _)| parent.to_owned());
                (parent, "a setting this host does not know".to_owned())
            } else if message.starts_with("duplicate field") {
                (path.clone(), "a key is given twice".to_owned())
            } else {
                (path.clone(), message.to_owned())
            };
            let what = match path {
                Some(path) if !path.is_empty() => format!("{path}: {what}"),
                _ => what,
            };
            parse(file, None, None, what)
        }
        Error::UnsupportedFormat { registered } => parse(
            file,
            Some(1),
            None,
            format!(
                "the first line names a format this build does not read; it reads {}",
                registered.join(", ")
            ),
        ),
        other => parse(file, None, None, other.to_string()),
    }
}

#[cfg(test)]
mod tests;
