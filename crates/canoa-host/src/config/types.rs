//! Core configuration structs, profile resolution types, and host subsystem settings.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;
use std::path::PathBuf;

use coqui_peer_webcrypto::PeerId;
use indexmap::IndexMap;

use super::{AccountConfig, Entry, HostProfile, Listen, Origin, PoolConfig, Settings, WebConfig};
use crate::profile::Profile;

/// The longest ACP message, as one line, either way on a connection, when `[host] max_message_bytes` does not say.
pub const DEFAULT_MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;
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
#[derive(Clone, Debug, PartialEq, Eq)]
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
    /// `[host.pools]`, by name, in the file's order: sets of accounts that provide one standard, which an entry may offer and a
    /// session may run on whichever account of them has usage left. The host adds an `any-<standard>` pool of its own for each
    /// standard an account provides; those are in no file.
    pub pools: IndexMap<String, PoolConfig>,
    /// `[secrets]` as the file has it, which the secret providers read: who keeps the accounts' credentials and the session services'
    /// passwords is among it (`accounts`).
    pub secrets: Option<serde_json::Value>,
    /// `[host] shell`: the program a terminal starts, with no arguments; `None` leaves it to the person's own (`$SHELL`, else
    /// `pwsh` or `powershell` on Windows).
    pub shell: Option<String>,
    /// `[host] openers`: editors and file managers `workspace/openers` offers and `workspace/open` starts; `None` seeds VS Code,
    /// the system's file manager and Reveal.
    pub openers: Option<Vec<OpenerConfig>>,
    /// `[host] max_message_bytes`: the longest ACP message, as one line, either way on a connection (a prompt carrying an
    /// attachment is one message). There is no ceiling here: a message over it is refused with this setting named.
    pub max_message_bytes: usize,
    /// `[settings.power] keep_awake`: whether the host keeps this computer from idle-sleeping while an agent turn runs; `false`
    /// unless set. The Power group of Settings writes it.
    pub keep_awake: bool,
    /// `[host.environments]`, by name, in the file's order: the places an agent may work besides this computer (`here`, which needs no
    /// entry). Each is made by the environment provider its `kind` names.
    pub environments: IndexMap<String, EnvironmentConfig>,
    /// `[host.store]`: what the program stores' installs are held to.
    pub store: StoreConfig,
    /// `[host.content]`: how long and how much a catalog read through the content sources is kept and may hold.
    pub content: ContentConfig,
    /// `[host.disabled]`: the providers the libraries register that this host neither offers nor runs.
    pub disabled: crate::disabled::Disabled,
    /// `[network]`: how the host reaches the network, by which route.
    pub network: NetworkRoutes,
    /// `[host.kb]`: the knowledge index tools and where their embeddings come from.
    pub kb: KbConfig,
    /// `[assist]` and `[sdk]`: the backends that serve the host's own model work.
    pub assist: AssistConfig,
    /// `[notifications]`: who is told about each event, and the notifier entries that add places to tell.
    pub notifications: NotificationsConfig,
    /// `[schedules.ID]`: the prompts the host runs on a schedule, by id, in the file's order.
    pub schedules: IndexMap<String, ScheduleConfig>,
    /// `[host.mcp_list]`: how long and how much listing a session's MCP servers takes.
    pub mcp_list: McpListConfig,
    /// `[host.services.ID]`: the table of each session service as the text of a JSON object whose members keep the file's order,
    /// which the service reads and writes (`canoa::session_service`). The host checks only that an id is a service id.
    pub services: IndexMap<String, String>,
    /// `[host.delegate]`: what an agent may hand to another one.
    pub delegate: DelegateConfig,
    /// `[host.control]`: the control channel's limits, and where a paired key may reach it from; `None` when the file has no
    /// such table, which leaves the channel to this process.
    pub control: Option<ControlConfig>,
    /// `[host.native]`: the door a browser extension's native messaging host reaches this host by.
    pub native: NativeConfig,
    /// `[host.history]`: whether the host records the sessions it serves, and how much it keeps.
    pub history: HistoryConfig,
    /// `[host.worktrees]`: how many worktrees a repository may hold from the host, and how long git may take.
    pub worktrees: WorktreeConfig,
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
    /// The account makers the libraries this config names offer, found when first asked for.
    pub(crate) makers: crate::makers::Cell,
    /// Opened provider registries for this config, cached on first read.
    pub(crate) providers: crate::control::providers::Cell,
}

/// Serves nothing, with every setting at its default.
impl Default for Config {
    fn default() -> Self {
        Self {
            origins: Default::default(),
            entries: Default::default(),
            profiles: Default::default(),
            web: Default::default(),
            limit_phrases: Default::default(),
            protected_paths: Default::default(),
            accounts: Default::default(),
            pools: Default::default(),
            secrets: Default::default(),
            shell: Default::default(),
            openers: Default::default(),
            max_message_bytes: DEFAULT_MAX_MESSAGE_BYTES,
            keep_awake: false,
            environments: Default::default(),
            store: Default::default(),
            content: Default::default(),
            disabled: Default::default(),
            network: Default::default(),
            kb: Default::default(),
            assist: Default::default(),
            notifications: Default::default(),
            schedules: IndexMap::new(),
            mcp_list: Default::default(),
            services: IndexMap::new(),
            delegate: Default::default(),
            control: Default::default(),
            native: Default::default(),
            history: Default::default(),
            worktrees: Default::default(),
            usage: Default::default(),
            local_agents: Default::default(),
            remote_agents: Default::default(),
            local_profiles: Default::default(),
            skill_pins: Default::default(),
            extensions: Default::default(),
            makers: Default::default(),
            providers: Default::default(),
        }
    }
}

/// `[notifications]`: for each event the notifiers it goes to, and the notifier entries the names refer to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct NotificationsConfig {
    /// The event ids the file names, each with the notifier names it goes to (`[]` turns the event off), in the file's order. An
    /// event the file does not name goes where the host's catalog says.
    pub routes: IndexMap<String, Vec<String>>,
    /// `[notifications.notifiers.NAME]`, in the file's order. `desktop` needs no entry and cannot be redefined.
    pub notifiers: IndexMap<String, NotifierConfig>,
}

/// One `[notifications.notifiers.NAME]`: a place notices go, made by a `notifier` provider.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct NotifierConfig {
    /// The id of the provider that sends the notices.
    pub provider: String,
    /// Where the provider's code comes from when it is no bundled one (`library` or `source`, as a harness entry names them).
    pub origin: Option<Origin>,
    /// The provider's settings, checked against the schema it declares when a notice is sent; a sensitive member is not here but
    /// kept by the host as a secret.
    pub settings: Settings,
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
    /// (see [`crate::config::CLIENT_KINDS`]). It wins over `[defaults] loadout`. A name `[loadouts]` does not hold is kept, so a connection can say so.
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
pub(super) fn empty_preset() -> &'static Preset {
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

impl ResolvedLoadout {
    /// The loadout that holds nothing, which a connection names with an empty name to start with none of the person's overlays,
    /// skills and servers, whatever `[defaults] loadout` says.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            name: String::new(),
            overlays: Vec::new(),
            missing_overlays: Vec::new(),
            skills: Vec::new(),
            mcp: Vec::new(),
        }
    }
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
pub(super) fn resolve_loadout(
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
    /// What the extension's Start button runs, recorded by `canoa native install`.
    pub launch: Option<crate::native::Launch>,
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
            launch: None,
        }
    }
}

/// One entry of `[host] openers`: an editor or file manager `workspace/openers` offers and `workspace/open` starts.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct OpenerConfig {
    /// Identifier passed to `workspace/open`.
    pub id: String,
    /// Display label in Open in menus.
    pub label: String,
    /// Program name on `PATH` or file path to start.
    pub program: String,
    /// Arguments passed to `program`; `{path}` is replaced with the target path (and `{parent}` with its parent folder).
    pub args: Vec<String>,
    /// Restriction to items (`Some(true)`), folders (`Some(false)`), or either (`None`).
    pub item: Option<bool>,
}

impl OpenerConfig {
    /// An opener entry taking `{path}` for both folders and items.
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        program: impl Into<String>,
        args: Vec<String>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            program: program.into(),
            args,
            item: None,
        }
    }

    /// Restricts this opener to items (`true`) or folders (`false`).
    #[must_use]
    pub const fn for_item(mut self, item: bool) -> Self {
        self.item = Some(item);
        self
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

/// `[host.worktrees]`: the worktrees the host makes for a session. Every limit is a setting, and none has a ceiling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct WorktreeConfig {
    /// `max_per_repository`: worktrees one repository may hold beside it, from the host, before it makes no more.
    pub max_per_repository: usize,
    /// `git_seconds`: how long one git command may run before the host ends it.
    pub git_seconds: u64,
}

impl WorktreeConfig {
    /// The default of `max_per_repository`.
    pub const DEFAULT_MAX_PER_REPOSITORY: usize = 32;
    /// The default of `git_seconds`.
    pub const DEFAULT_GIT_SECONDS: u64 = 120;
}

impl Default for WorktreeConfig {
    fn default() -> Self {
        Self {
            max_per_repository: Self::DEFAULT_MAX_PER_REPOSITORY,
            git_seconds: Self::DEFAULT_GIT_SECONDS,
        }
    }
}

/// `[host.history]`: the host records each session it serves in the session-history format, in its home beside the ledger, and
/// the owner exports it or moves it to another source over the control channel. Every limit is a setting.
#[derive(Clone, Debug, PartialEq, Eq)]
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
    /// `store`: the id of the `history_store` provider that keeps them; none: the provider that says it is the default.
    pub store: Option<String>,
    /// `store_settings`: that provider's settings, as the JSON of an object.
    pub store_settings: String,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            record: true,
            max_session_bytes: crate::history::DEFAULT_MAX_SESSION_BYTES,
            max_sessions: crate::history::DEFAULT_MAX_SESSIONS,
            max_total_bytes: crate::history::DEFAULT_MAX_TOTAL_BYTES,
            store: None,
            store_settings: "{}".to_owned(),
        }
    }
}

/// `[host.usage]`: the host records the spend each agent reports, by turn, in its home (`usage/`), and answers `usage/summary` and
/// `usage/limits` from it; its `limits` warn and stop on what is recorded and reported. This is not `[usage]`, the app's and the limit
/// phrases' table, whose `warn_permille` the host reads too.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct UsageConfig {
    /// `keep_days`: how many days of records the host keeps; a month's file is removed at start once all of it is older.
    pub keep_days: u32,
    /// `[usage] warn_permille`: the share of a limit's figure, in thousandths, from which a limit that names no threshold of its own
    /// warns; the app's usage control reads the same key.
    pub warn_permille: u32,
    /// `[host.usage.limits.NAME]`, by name, in the file's order.
    pub limits: IndexMap<String, super::LimitConfig>,
}

impl Default for UsageConfig {
    fn default() -> Self {
        Self {
            keep_days: crate::usage_log::DEFAULT_KEEP_DAYS,
            warn_permille: super::DEFAULT_WARN_PERMILLE,
            limits: IndexMap::new(),
        }
    }
}

/// How long the `exec` tool of an environment waits for a program when `exec_seconds` is not set: an hour.
pub const DEFAULT_EXEC_SECONDS: u64 = 3600;

/// What an agent's `terminal/create` keeps of a program's output when it names no limit: 1 MiB.
pub const DEFAULT_OUTPUT_BYTES: u64 = 1024 * 1024;

/// What the tools keep of an interactive terminal's output for the agent to read: 4 MiB.
pub const DEFAULT_SCROLLBACK_BYTES: u64 = 4 * 1024 * 1024;

/// How many sessions of one connection are given an environment's tool server: 256.
pub const DEFAULT_MAX_TOOL_SESSIONS: u64 = 256;

/// How long the hook that takes an agent's own tools over may take for its answer when `hook_seconds` is not set: 30 seconds.
pub const DEFAULT_HOOK_SECONDS: u64 = 30;

/// How many requests of the take-over's channel are handled at once when `hook_handlers` is not set: 16.
pub const DEFAULT_HOOK_HANDLERS: u64 = 16;

/// How long a connection to the take-over's channel has to send its request when `hook_read_ms` is not set: one second.
pub const DEFAULT_HOOK_READ_MS: u64 = 1000;

/// How long a command the take-over's hook rewrote may wait to run when `prepared_seconds` is not set: ten minutes, time for the person to answer the agent's question.
pub const DEFAULT_PREPARED_SECONDS: u64 = 600;

/// How many commands of the take-over's hook may wait to run at once when `max_prepared` is not set: 256.
pub const DEFAULT_MAX_PREPARED: u64 = 256;

/// One `[host.environments.NAME]`: a place an agent works, made by an `environment` provider.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct EnvironmentConfig {
    /// The id of the provider that makes it, which the host lists as an environment kind.
    pub kind: String,
    /// Where the provider's code comes from when it is no bundled one (`library` or `source`, as a harness entry names them).
    pub origin: Option<Origin>,
    /// The provider's settings: every key of the table but `kind` and the ones that name its code, checked against the schema the
    /// provider declares when the environment is first used.
    pub settings: Settings,
    /// How long a program the `exec` tool runs here may take before it is ended (`exec_seconds`), `0` for no limit; `None` is
    /// [`DEFAULT_EXEC_SECONDS`]. The call has to return, so a program that never ends is ended at this and the tool says so.
    pub exec_seconds: Option<u64>,
    /// How much of a program's output the host keeps for an agent's `terminal/output` when the agent names no limit of its own
    /// (`output_bytes`), `0` for all of it; `None` is [`DEFAULT_OUTPUT_BYTES`].
    pub output_bytes: Option<u64>,
    /// How much of an interactive terminal's output the tools keep for the agent to read (`scrollback_bytes`), `0` for all of it;
    /// `None` is [`DEFAULT_SCROLLBACK_BYTES`].
    pub scrollback_bytes: Option<u64>,
    /// How many sessions of one connection may be given this environment's tool server (`max_tool_sessions`), `0` for any number;
    /// `None` is [`DEFAULT_MAX_TOOL_SESSIONS`].
    pub max_tool_sessions: Option<u64>,
    /// How many commands the hook rewrote may wait at once; past it the oldest is dropped (`max_prepared`), `0` for any number; `None` is [`DEFAULT_MAX_PREPARED`].
    pub max_prepared: Option<u64>,
    /// How long a command the hook rewrote may wait for its env-exec, which covers the agent asking the person first (`prepared_seconds`), `0` for as long as the session lasts; `None` is [`DEFAULT_PREPARED_SECONDS`].
    pub prepared_seconds: Option<u64>,
    /// How long a connection to the hook's channel has to send its request, in milliseconds, before its secret is checked (`hook_read_ms`), `0` for as long as it takes; `None` is [`DEFAULT_HOOK_READ_MS`].
    pub hook_read_ms: Option<u64>,
    /// How many requests of the hook's channel are handled at once; one past it is dropped, which refuses that call (`hook_handlers`), `0` for any number; `None` is [`DEFAULT_HOOK_HANDLERS`].
    pub hook_handlers: Option<u64>,
    /// How long the hook that takes an agent's own tools over may take to have the host's answer, in all, and a prompt waits for the hook to report that it is in force (`hook_seconds`), `0` for as long as the agent's hook can wait; `None` is [`DEFAULT_HOOK_SECONDS`].
    pub hook_seconds: Option<u64>,
}

/// `[host.mcp_list]`: what listing the tools and resources of a session's MCP servers (`_canoa/mcp/list`) is held to. Each is at
/// least 1 and has no ceiling of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct McpListConfig {
    /// `query_seconds`: how long one server may take to answer all of its lists (default 5).
    pub query_seconds: u64,
    /// `max_items_per_server`: the tools, and the resources, read from one server (default 1000).
    pub max_items_per_server: usize,
    /// `max_reply_bytes`: the longest answer read from one server, an HTTP body or a line of its output (default 4 MiB).
    pub max_reply_bytes: usize,
}

impl Default for McpListConfig {
    fn default() -> Self {
        Self {
            query_seconds: 5,
            max_items_per_server: 1000,
            max_reply_bytes: 4 * 1024 * 1024,
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
    /// `skill_folders`: folders searched for `SKILL.md` directories when scanning a skills source (a line or list, replacing
    /// the built-in common list when set; loaded agent folders are still included).
    pub skill_folders: Option<Vec<String>>,
    /// `index_ttl`: seconds the catalog index answers with no network (default 300); past it the stale index is served at once
    /// and refreshed in the background. 0 refreshes on every request.
    pub index_ttl: Option<u64>,
    /// `index_cache_bytes`: what the index's cached icons may take in all (default 64 MiB).
    pub index_cache_bytes: Option<u64>,
    /// `latest_ttl`: seconds a publisher's newest version of a program answers with no network (default 3600); past it the
    /// kept version is served at once and asked again in the background. 0 asks on every request.
    pub latest_ttl: Option<u64>,
    /// `models_ttl`: seconds a Store type's list of models answers without running its program (default 300; 0 every time).
    pub models_ttl: Option<u64>,
    /// `catalog_ttl`: seconds the entries of a skills or MCP servers catalog answer with no network (default 300; 0 every time).
    pub catalog_ttl: Option<u64>,
    /// `refresh_every`: seconds between the host asking again what the Store keeps that is past its time (the newest versions, the models of each type, the
    /// the catalogs' index), while at least one client is connected (default 300; 0 never).
    pub refresh_every: Option<u64>,
    /// `connect_timeout`: seconds a lookup (a release list, a checksum file, a model catalog) waits to connect to a publisher
    /// (default 10). A download waits 60.
    pub connect_timeout: Option<u64>,
    /// `lookup_timeout`: seconds a lookup waits for the publisher's answer (default 15). A download waits 60.
    pub lookup_timeout: Option<u64>,
    /// `tools`: whether the host offers the `canoa-store` tools (search and inspect) to a session whose client asks for them and
    /// whose key reads the index (default true).
    pub tools: Option<bool>,
    /// `verify_copies`: whether the host reads every file of a program the Store installed, against what was recorded when it
    /// was installed, when it starts and when it looks the program up (default false: a copy is trusted as installed, and only
    /// its launch file must exist). A download is checked against its publisher's SHA-256 when it is installed either way.
    pub verify_copies: Option<bool>,
}

/// `[host.content]`: how the content sources and catalog generators read, as the Store's catalogs do.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ContentConfig {
    /// `ttl`: seconds a catalog read through a content source or a generator answers with no network (default 3600); an entry's
    /// own `ttl` replaces it, and a pinned catalog never goes stale.
    pub ttl: Option<u64>,
    /// `document_bytes`: the most a document read into memory (a catalog, a listing) may hold unless the reader names less
    /// (default 8 MiB).
    pub document_bytes: Option<u64>,
    /// `cache_bytes`: the most the content cache keeps on disk, the oldest documents going first (default 512 MiB).
    pub cache_bytes: Option<u64>,
}

/// Guatiao-derived schema for `[host.content]`.
#[derive(
    coqui_acp_api::guatiao::ToValue,
    coqui_acp_api::guatiao::FromValue,
    coqui_acp_api::guatiao::Schema,
)]
#[map(crate = ::coqui_acp_api::guatiao, deny_unknown_fields)]
struct ContentSectionSchema {
    ttl: Option<u64>,
    document_bytes: Option<u64>,
    cache_bytes: Option<u64>,
}

impl ContentConfig {
    /// The guatiao schema describing `[host.content]` in `canoa.cfg`.
    ///
    /// # Errors
    ///
    /// Returns [`coqui_acp_api::guatiao::ValueError`] if building the schema value fails.
    pub fn schema() -> Result<coqui_acp_api::guatiao::Value, coqui_acp_api::guatiao::ValueError> {
        <ContentSectionSchema as coqui_acp_api::guatiao::Schema>::schema()
    }
}

/// Guatiao-derived schema for `[host.store]`.
#[derive(
    coqui_acp_api::guatiao::ToValue,
    coqui_acp_api::guatiao::FromValue,
    coqui_acp_api::guatiao::Schema,
)]
#[map(crate = ::coqui_acp_api::guatiao, deny_unknown_fields)]
struct StoreSectionSchema {
    protected: Option<Vec<String>>,
    credential_variables: Option<Vec<String>>,
    index_ttl: Option<u64>,
    index_cache_bytes: Option<u64>,
    latest_ttl: Option<u64>,
    models_ttl: Option<u64>,
    catalog_ttl: Option<u64>,
    refresh_every: Option<u64>,
    connect_timeout: Option<u64>,
    lookup_timeout: Option<u64>,
    tools: Option<bool>,
    verify_copies: Option<bool>,
}

impl StoreConfig {
    /// The guatiao schema describing `[host.store]` in `canoa.cfg`.
    ///
    /// # Errors
    ///
    /// Returns [`coqui_acp_api::guatiao::ValueError`] if building the schema value fails.
    pub fn schema() -> Result<coqui_acp_api::guatiao::Value, coqui_acp_api::guatiao::ValueError> {
        <StoreSectionSchema as coqui_acp_api::guatiao::Schema>::schema()
    }
}

/// `[network]`: how every fetch the host makes, and every program it starts, reaches the network. With no key the host follows
/// the system: the proxy variables of its environment, else the operating system's proxy settings (PAC and WPAD included) and its
/// proxy login, the operating system's certificate verifier and the client certificates of its keystores. The host's way out is
/// one named route; an account and a profile may name others.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct NetworkRoutes {
    /// `route`: the host's route, a name in `routes` or one of the built-in `system` and `direct` (default `system`).
    pub route: Option<String>,
    /// `routes.NAME`: the routes the person made, in the file's order. The built-in `system` and `direct` need no table and their
    /// names are not given to another.
    pub routes: IndexMap<String, RouteConfig>,
    /// `endpoints`: URLs the programs the host starts will use, added to the built-in list a PAC script is asked about to give
    /// them their proxy variables.
    pub endpoints: Vec<String>,
    /// `idle_seconds`: how long a shared lease nobody uses is kept for the next user that asks for the same one (default 0: it
    /// ends with its last user).
    pub idle_seconds: Option<u64>,
    /// `open_seconds`: how long a route may take to open before it is given up on (default 10).
    pub open_seconds: Option<u64>,
    /// `max_leases`: how many leases may be open at once (default: no limit).
    pub max_leases: Option<usize>,
    /// The folder of the file this was read from, which a relative path in a route's settings is made from.
    pub base_dir: Option<String>,
    /// `prefer_webtransport`: the application's own setting, which the host leaves be.
    pub prefer_webtransport: Option<bool>,
    /// `connect_timeout`: the application's own setting, which the host leaves be.
    pub connect_timeout: Option<u64>,
}

/// One `[network.routes.NAME]`: which provider of the `network_route` kind makes the route, and its settings.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct RouteConfig {
    /// The provider's id: `manual`, `pac`, `command`, or one a library offers.
    pub provider: String,
    /// The provider's settings, an object its schema accepts. A password is the name of a host variable, never the password.
    pub settings: serde_json::Value,
}

impl NetworkRoutes {
    /// The name of the route the host leaves by.
    #[must_use]
    pub fn default_route(&self) -> &str {
        self.route.as_deref().unwrap_or(SYSTEM_ROUTE)
    }
}

/// The route every host has, whose name no table takes: the system's way.
pub const SYSTEM_ROUTE: &str = "system";
/// The route every host has, whose name no table takes: straight to the destination.
pub const DIRECT_ROUTE: &str = "direct";

/// What a choice of backend says when the table does not name one: the operation's own choice in `[assist.OPERATION]`.
pub const DEFAULT_CHOICE: &str = "default";

/// `[host.kb]`: the `canoa-kb` tools, which search the knowledge an overlay ships. With no `embedder` an index holds words only
/// (BM25); with one, passages are also embedded by the backend the person chose, which Canoa never chooses for them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct KbConfig {
    /// `tools`: whether the host offers the `canoa-kb` tools to a session whose client asks for them (default true).
    pub tools: Option<bool>,
    /// `chunk_bytes`: the most bytes of a passage, its prefix included (4 to 65536; default 1024).
    pub chunk_bytes: Option<usize>,
    /// `overlap_bytes`: the most bytes of whole lines adjacent passages share (default 0).
    pub overlap_bytes: Option<usize>,
    /// `retry_seconds`: how long a backend that failed is left alone before it is asked again (default 60).
    pub retry_seconds: Option<u64>,
    /// `query_cache`: how many query embeddings are remembered (default 256).
    pub query_cache: Option<usize>,
    /// `[host.kb.embedder]`: what matches by meaning.
    pub embedder: Option<KbEmbedder>,
    /// `[host.kb.ranker]`: what puts the best passages first.
    pub ranker: Option<KbRanker>,
    /// `[host.kb.search]`: a provider that does the whole search its own way; read, and used from the knowledge provider kind on.
    pub search: Option<KbSearch>,
}

/// `[host.kb.embedder]`.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct KbEmbedder {
    /// `use`: a backend id (`openai:work`, `local`), or `default` for what `[assist.embed]` chooses.
    pub choice: String,
    /// `model`: the exact model that makes the vectors; saved with each index.
    pub model: String,
    /// `dimensions`: the vector length asked for, where the model allows it.
    pub dimensions: Option<u32>,
    /// `document_prefix`: text put before each passage when it is embedded, for models that ask for one.
    pub document_prefix: Option<String>,
    /// `query_prefix`: text put before a query when it is embedded.
    pub query_prefix: Option<String>,
}

/// `[host.kb.ranker]`.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct KbRanker {
    /// `use`: a backend id, or `default` for what `[assist.rerank]` chooses (a chat model by `[assist.complete]` where no
    /// backend reranks).
    pub choice: String,
    /// `model`: the exact model that orders the passages.
    pub model: String,
    /// `candidates`: the most passages the model is shown (default 10, 1 to 100).
    pub candidates: Option<usize>,
    /// `timeout_seconds`: how long one ordering may take before the search answers without it (default 30).
    pub timeout_seconds: Option<u64>,
    /// `passage_chars`: the most characters of one passage the model is shown (default 600).
    pub passage_chars: Option<usize>,
}

/// `[host.kb.search]`.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct KbSearch {
    /// `use`: the provider that searches.
    pub choice: String,
}

/// `[assist.OPERATION]`: which backend serves an operation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct AssistChoice {
    /// `use`: the backend that serves it; absent, the first that can.
    pub choice: Option<String>,
    /// `fallback`: the backends asked, in order, when it cannot.
    pub fallback: Vec<String>,
}

/// `[sdk.NAME]`: a server the person offers by address.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct SdkTable {
    /// `provider`: the `sdk_provider` that speaks to it (`server`).
    pub provider: String,
    /// `address`: where it is.
    pub address: Option<String>,
    /// `key_env`: the name of a variable holding the key it asks for; the key itself is never in canoa.cfg.
    pub key_env: Option<String>,
    /// The rest of the table: the provider's own settings, as JSON.
    pub settings: serde_json::Value,
}

/// `[assist_harness.NAME]`: an agent harness the person has, on a profile, offered as a backend that completes a prompt with the
/// harness's own login.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct HarnessTable {
    /// `source`: the agent entry that runs it (a name of `[host.agents]`, or a bundled agent's title).
    pub source: String,
    /// `title`: what the person calls it.
    pub title: Option<String>,
    /// `profile`: the `[host.profiles]` profile the session starts on (its model, effort and instructions).
    pub profile: Option<String>,
    /// `account`: the account of the source the session runs as.
    pub account: Option<String>,
    /// `model`: the model, when the profile's is not the one wanted.
    pub model: Option<String>,
    /// `effort`: the effort, when the profile's is not the one wanted.
    pub effort: Option<String>,
}

/// `[assist_pool]`: the warm sessions the harness backends keep.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct AssistPool {
    /// `idle_seconds`: how long a session nobody asked is kept before it is ended (default 120).
    pub idle_seconds: Option<u64>,
    /// `max_sessions`: the most sessions kept at once, all harnesses together (default 2).
    pub max_sessions: Option<usize>,
}

/// `[assist]`, `[sdk]` and `[assist_harness]`: which backends serve the host's own model work, and the offers the person makes
/// by address or by harness.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct AssistConfig {
    /// `[assist.OPERATION]`, by operation name, in the file's order.
    pub operations: IndexMap<String, AssistChoice>,
    /// `[sdk.NAME]`, by name, in the file's order.
    pub sdk: IndexMap<String, SdkTable>,
    /// `[assist_harness.NAME]`, by name, in the file's order.
    pub harnesses: IndexMap<String, HarnessTable>,
    /// `[assist_pool]`.
    pub pool: AssistPool,
}

/// How often a scheduled task runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Repeat {
    /// At one date and time.
    Once,
    /// Every day at a time.
    Daily,
    /// On some days of the week at a time.
    Weekly,
}

/// `[schedules.ID]`: a prompt the host runs on a schedule, as a session of the entry it names, in a folder, by the person's own
/// rules. Times are read in `time_zone`, else the computer's own, so a daylight-saving change does not move a task.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ScheduleConfig {
    /// `entry`: the agent entry the session runs on.
    pub entry: String,
    /// `folder`: the folder the session works in.
    pub folder: String,
    /// `prompt`: what the session is asked.
    pub prompt: String,
    /// `title`: what the person calls it, when not its id.
    pub title: Option<String>,
    /// `repeat`: `once`, `daily` (the default) or `weekly`.
    pub repeat: Repeat,
    /// `at`: the time of day, `HH:MM`, as `(hour, minute)`.
    pub at: (u8, u8),
    /// `date`: with `once`, the day, `YYYY-MM-DD`, as `(year, month, day)`.
    pub date: Option<(i32, u8, u8)>,
    /// `weekdays`: with `weekly`, the days, from 0 for Sunday, in order and once each.
    pub weekdays: Vec<u8>,
    /// `time_zone`: the IANA name of the zone `at` and `date` are read in (`America/Puerto_Rico`); absent reads them in the
    /// computer's own.
    pub time_zone: Option<String>,
    /// `enabled`: whether it runs (default true).
    pub enabled: bool,
    /// `mode`: the mode the session is put in when the agent offers it; absent leaves the agent's own.
    pub mode: Option<String>,
    /// `timeout_minutes`: how long a run may take before the host ends it (default 120): a hung agent would otherwise hold its
    /// task for ever.
    pub timeout_minutes: u64,
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
    /// `wait_seconds`: the longest one `task_status` call waits for a task to change, whatever its `wait_ms` asks.
    pub wait_seconds: u64,
    /// `dial_seconds`: how long the host has to reach another machine's agent and prove its key, for a task handed there.
    pub dial_seconds: u64,
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
    /// The default of `wait_seconds`.
    pub const DEFAULT_WAIT_SECONDS: u64 = 300;
    /// The default of `dial_seconds`.
    pub const DEFAULT_DIAL_SECONDS: u64 = 20;
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
            wait_seconds: Self::DEFAULT_WAIT_SECONDS,
            dial_seconds: Self::DEFAULT_DIAL_SECONDS,
        }
    }
}
