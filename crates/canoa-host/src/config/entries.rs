//! Entry, listen target, origin, harness option, account/pool, web, and error types.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;
use core::net::SocketAddr;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use coqui_acp_api::CacheLifetime;
use coqui_acp_api::guatiao::{FromValue as _, Value as GuatiaoValue};
use coqui_peer_webcrypto::PeerId;
use coqui_transport_pipe::PipeName;
use indexmap::IndexMap;

use super::types::{
    DEFAULT_PRESET, Loadout, Preset, ResolvedLoadout, UnknownLoadout, UnknownPreset, empty_preset,
    resolve_loadout,
};
use crate::profile::Profile;

/// One `[host.accounts.NAME]`: which login an entry may run a session on. It holds no credential.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct AccountConfig {
    /// The harness whose own login this is: set only on the automatic `local` account, which every harness has under one kind.
    /// An account in `[host.accounts]` names no harness: its kind says which account provider made it.
    pub owner: Option<String>,
    /// One of the kinds an account provider offers; no two providers offer the same id.
    pub kind: String,
    /// The account's id, which Canoa writes when it makes the account: never reused, and the key of its stored credential.
    pub id: String,
    /// The kind's settings that are not secrets, checked against the provider's schema when the host starts.
    pub settings: Option<Settings>,
    /// The `network_route` the account's own requests leave by (model lists, usage, sign-in); the host's route when absent.
    pub route: Option<String>,
}

impl AccountConfig {
    /// The automatic `local` account of the harness `harness`: its own login, run as its configuration stands. It is in no
    /// `[host.accounts]`.
    pub(crate) fn local(harness: &str) -> Self {
        Self {
            owner: Some(harness.to_owned()),
            kind: coqui_acp_api::HARNESS_LOGIN.to_owned(),
            id: coqui_acp_api::LOCAL_ACCOUNT.to_owned(),
            settings: None,
            route: None,
        }
    }

    /// The id the usage log and the usage reports name this account by; see [`usage_id`].
    pub(crate) fn usage_id(&self) -> String {
        usage_id(
            self.owner.as_deref().unwrap_or_default(),
            &self.id,
            &self.kind,
        )
    }
}

/// The id usage is recorded and reported under for the account `id` of `kind` that `harness` runs. It is the account's own id,
/// except for the automatic `local` account: every harness has one, so it is told apart by its harness.
pub(crate) fn usage_id(harness: &str, id: &str, kind: &str) -> String {
    if kind == coqui_acp_api::HARNESS_LOGIN && id == coqui_acp_api::LOCAL_ACCOUNT {
        format!("{id}:{harness}")
    } else {
        id.to_owned()
    }
}

/// One `[host.pools.NAME]`: accounts that provide one standard, as one choice for a session. Which kinds provide the standard is
/// the account providers' to say, so a member is checked against it where the providers are known (`pools`), not here.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct PoolConfig {
    /// The credential standard every member provides: any id `coqui_acp_api::standard::is_standard` accepts.
    pub standard: String,
    /// The members, by name in `[host.accounts]` (or the automatic `local`), in the order the pool takes them.
    pub accounts: Vec<String>,
    /// How the pool orders the accounts that can serve: `round_robin` unless the file says otherwise.
    pub strategy: coqui_acp_pool::Strategy,
}

/// Validation failure for a pool definition under [`Config`].
pub(crate) use coqui_acp_pool::PoolIssue;

/// What the host holds that a pool's name and members are checked against: the accounts and the automatic `local`.
struct AccountNames<'a>(&'a IndexMap<String, AccountConfig>);

impl coqui_acp_pool::Known for AccountNames<'_> {
    fn taken(&self, name: &str) -> bool {
        name == coqui_acp_api::LOCAL_ACCOUNT || self.0.contains_key(name)
    }

    fn member(&self, name: &str) -> bool {
        self.taken(name)
    }
}

impl PoolConfig {
    /// Checks pool name, standard id and member names against `accounts`.
    pub(crate) fn validate(
        &self,
        name: &str,
        accounts: &IndexMap<String, AccountConfig>,
    ) -> Result<(), PoolIssue> {
        coqui_acp_pool::check_pool(
            name,
            Some(&self.standard),
            &self.accounts,
            &AccountNames(accounts),
            &coqui_acp_pool::Settings::default(),
        )
    }
}

/// How a role hands a session to a member of another harness when its own runs out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Handover {
    /// The host moves it at once.
    #[default]
    Auto,
    /// The host holds the move until the person says yes.
    Ask,
}

/// One member of a role, `{ agent, account, model, effort, profile, ... }`: an agent entry with the account, model, effort and profile
/// a session of it runs on. Every value is what the host discovers; none is named by the host.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct RoleMember {
    /// The agent entry of canoa.cfg.
    pub agent: String,
    /// An account, an account pool, or `None` for the entry's first.
    pub account: Option<String>,
    /// The model a session of it asks for.
    pub model: Option<String>,
    /// The reasoning effort.
    pub effort: Option<String>,
    /// A `[host.profiles]` name.
    pub profile: Option<String>,
    /// Higher goes first under the `priority` strategy.
    pub priority: i32,
    /// The runs it takes at once; `None` is no cap.
    pub max_concurrent: Option<u32>,
    /// The use, in thousandths of a bucket, at which it stops taking turns; `None` is 1000.
    pub stop_permille: Option<u16>,
}

impl RoleMember {
    /// The name a role's pool knows this member by: its agent, account, model, effort and profile, joined by `/`.
    #[must_use]
    pub fn id(&self) -> String {
        let part = |value: &Option<String>| value.clone().unwrap_or_default();
        format!(
            "{}/{}/{}/{}/{}",
            self.agent,
            part(&self.account),
            part(&self.model),
            part(&self.effort),
            part(&self.profile)
        )
    }
}

/// One `[host.roles.NAME]`: a pool named by what the work is, whose members are the agents, accounts and models able to do it, in
/// order.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct RoleConfig {
    /// What the person sees; the name when unset.
    pub title: Option<String>,
    /// How members are ordered: `priority` unless the file says otherwise.
    pub strategy: coqui_acp_pool::Strategy,
    /// Capability names a member's model must report; a model that reports none passes.
    pub needs: Vec<String>,
    /// Whether a running session stays on its member while it can serve.
    pub stick: bool,
    /// What a move to another harness waits for.
    pub handover: Handover,
    /// What continues a session that moved; `None` is the library's one line.
    pub resume_prompt: Option<String>,
    /// Whether a delegated task that hit a limit starts again on the next member.
    pub retry_task: bool,
    /// The members, in order.
    pub members: Vec<RoleMember>,
}

impl RoleConfig {
    /// The role as the shared pool code sees it: each member keyed by [`RoleMember::id`] and grouped by its agent entry's kind.
    pub(crate) fn pool(
        &self,
        name: &str,
        group_of: &dyn Fn(&str) -> String,
        settings: coqui_acp_pool::Settings,
    ) -> coqui_acp_pool::Pool<RoleMember> {
        let mut pool = coqui_acp_pool::Pool::new(name)
            .strategy(self.strategy)
            .settings(settings);
        for member in &self.members {
            let mut seat = coqui_acp_pool::Member::new(member.id(), member.clone())
                .group(group_of(&member.agent))
                .priority(member.priority);
            if let Some(runs) = member.max_concurrent {
                seat = seat.max_concurrent(runs);
            }
            if let Some(stop) = member.stop_permille {
                seat = seat.stop_permille(stop);
            }
            pool = pool.member(seat);
        }
        pool
    }
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
    /// `[overlays.NAME] kb = "index"`: see [`crate::config::LocalProfiles::kb_choice`].
    pub kb_choice: BTreeMap<String, bool>,
    /// The skills the session's loadout names, for the host's own sources only: each is delivered from the app's record of it
    /// when the record and the files agree. Empty until a connection resolves its loadout.
    pub chosen_skills: Vec<String>,
    /// `[loadouts.NAME]`, which a connection resolves once when it starts.
    pub loadouts: IndexMap<String, Loadout>,
    /// `[defaults] loadout`.
    pub default_loadout: Option<String>,
    /// `[defaults.clients]`: see [`crate::config::LocalProfiles::client_loadouts`].
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
    /// `[permissions.approvers]`: who else answers a permission request, besides the window.
    pub approvers: crate::approve::Approvers,
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
    pub(super) fn same_place(&self, other: &Self) -> bool {
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
    pub(super) fn is_fixed(&self) -> bool {
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
    /// The environment an agent or harness entry's program runs in (`environment = "build-box"`), one `[host.environments]` names
    /// or `here`; `None` runs it on this computer. A client may ask for another when it connects.
    pub environment: Option<String>,
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
    CacheLifetime::from_value(&GuatiaoValue::from(name)).ok()
}

/// The name of `lifetime` in [`CACHE_LIFETIMES`].
#[must_use]
pub fn cache_lifetime_name(lifetime: CacheLifetime) -> &'static str {
    CACHE_LIFETIMES
        .iter()
        .copied()
        .find(|name| cache_lifetime_named(name) == Some(lifetime))
        .unwrap_or("default")
}

/// What a menu calls `lifetime`: the host's own words, the same whichever provider honours it.
#[must_use]
pub const fn cache_lifetime_label(lifetime: CacheLifetime) -> &'static str {
    match lifetime {
        CacheLifetime::Off => "Off",
        CacheLifetime::Short => "5 minutes",
        CacheLifetime::Long => "1 hour",
        _ => "Default",
    }
}

/// Claude Code's setting sources.
pub const SETTING_SOURCES: &[&str] = &["user", "project", "local"];

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
        /// The id of the `usage_reader` provider that reads this agent's quota when the agent reports none itself (`usage = "..."`
        /// on the entry), one a bundled agent offers; `None` reads only its failed turns.
        usage: Option<String>,
        /// The configuration of that reader (`usage_settings`), which a reader that reads what the agent stores takes: where the
        /// login is kept and the service's addresses. `None` is the reader's defaults.
        usage_settings: Option<Settings>,
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
pub struct Settings(pub(super) serde_json::Map<String, serde_json::Value>);

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
