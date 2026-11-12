//! The `[host]` table of canoa.cfg: what the host serves, and to whom. The other tables belong to the app and are not read.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;
use core::net::{IpAddr, Ipv4Addr, SocketAddr};
use core::time::Duration;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use coqui_acp_api::CacheLifetime;
use coqui_peer_webcrypto::PeerId;
use coqui_transport_pipe::PipeName;
use indexmap::IndexMap;
use serde::Deserialize;

use crate::profile::{LayerKind, Profile};
use crate::tool::ToolSpec;

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
    /// `[host.accounts]`, by name, in the file's order: the accounts a harness entry may offer.
    pub accounts: IndexMap<String, AccountConfig>,
    /// Where the accounts' credentials are kept: `[host] account_store`.
    pub account_store: AccountStore,
    /// `[host.control]`: the control channel's limits, and where a paired key may reach it from; `None` when the file has no
    /// such table, which leaves the channel to this process.
    pub control: Option<ControlConfig>,
    /// `[agents.NAME]` with a `program`: the person's own agent commands, as entries reached over [`crate::Local`] only (each has
    /// [`Entry::local`] set), in the file's order. They are served when the host serves its own sources.
    pub local_agents: Vec<Entry>,
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
}

/// Where the credentials of `[host.accounts]` are kept, never in canoa.cfg.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum AccountStore {
    /// The operating system's credential store (Windows Credential Manager, macOS Keychain, Secret Service).
    #[default]
    Os,
    /// A file per account in the host's home, sealed to a key beside it: for a host with no credential store, and a
    /// weakening the host logs.
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
    /// canoa.cfg's directory, absolute.
    pub directory: PathBuf,
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

    /// The address, for a listener a browser can reach; `None` for a pipe.
    #[must_use]
    pub const fn address(&self) -> Option<SocketAddr> {
        match self {
            Self::Address(address) => Some(*address),
            Self::Pipe(_) => None,
        }
    }

    /// Whether this names a place that is one entry's alone: a pipe, or an address with a port.
    const fn is_fixed(&self) -> bool {
        match self {
            Self::Address(address) => address.port() != 0,
            Self::Pipe(_) => true,
        }
    }
}

impl fmt::Display for Listen {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Address(address) => address.fmt(f),
            Self::Pipe(name) => name.fmt(f),
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
}

/// What a `claude` or `codex` harness entry's keys ask before the command line acts. Each one unset is the host's own value,
/// never the person's configuration on this machine: Claude Code's permission mode `default`, Codex's approval policy
/// `on-request` and sandbox `workspace-write`. They are the provider's settings of the same names; an outside provider
/// takes its own through `settings`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct HarnessOptions {
    /// Claude Code's permission mode: `default`, `acceptEdits`, `plan`, `dontAsk`, `bypassPermissions`, `auto` or `manual`.
    pub permission_mode: Option<String>,
    /// Codex's approval policy: `untrusted`, `on-failure`, `on-request` or `never`.
    pub approval_policy: Option<String>,
    /// Codex's sandbox: `read-only`, `workspace-write` or `danger-full-access`.
    pub sandbox: Option<String>,
    /// Whether a client may put a session in Bypass, the mode that asks for nothing (`bypass = true`). Off, the `mode`
    /// option does not offer it and a client that sets it is refused. A key paired to someone else's machine reaches this
    /// entry, so on is the host person's decision for that entry: with it, that key could take full control of this machine.
    pub bypass: bool,
    /// The setting sources Claude Code loads, of `user`, `project` and `local`; empty loads none, `None` its own default.
    pub setting_sources: Option<Vec<String>>,
    /// The modes a paired client may choose (`modes = ["manual", "accept_edits", ...]`), ids as `coqui_acp_harness::Mode`
    /// names them. Unset, a client may choose the mode the entry's own settings amount to and every stricter one, so it
    /// cannot loosen what canoa.cfg set; naming modes widens that, and Bypass still needs `bypass = true`.
    pub modes: Option<Vec<coqui_acp_harness::Mode>>,
    /// How long the provider's prompt cache should live for this entry's sessions (`cache_lifetime = "default" | "short" |
    /// "long"`), when the harness has such a control. A client's own choice for one session wins; unset leaves the client's
    /// default for its kind of session (the chat asks `long`), and the harness's own where it names none.
    pub cache_lifetime: Option<CacheLifetime>,
}

/// The values of `cache_lifetime`, in canoa.cfg and on `canoa agent`.
pub const CACHE_LIFETIMES: &[&str] = &["default", "short", "long"];

/// The lifetime `name` stands for, one of [`CACHE_LIFETIMES`].
#[must_use]
pub fn cache_lifetime_named(name: &str) -> Option<CacheLifetime> {
    match name {
        "default" => Some(CacheLifetime::Default),
        "short" => Some(CacheLifetime::Short),
        "long" => Some(CacheLifetime::Long),
        _ => None,
    }
}

/// The name of `lifetime` in [`CACHE_LIFETIMES`].
#[must_use]
pub const fn cache_lifetime_name(lifetime: CacheLifetime) -> &'static str {
    match lifetime {
        CacheLifetime::Short => "short",
        CacheLifetime::Long => "long",
        _ => "default",
    }
}

/// Claude Code's setting sources.
pub const SETTING_SOURCES: &[&str] = &["user", "project", "local"];

/// Claude Code's permission modes, as `--permission-mode` takes them.
pub const PERMISSION_MODES: &[&str] = &[
    "default",
    "acceptEdits",
    "plan",
    "dontAsk",
    "bypassPermissions",
    "auto",
    "manual",
];
/// Codex's approval policies.
pub const APPROVAL_POLICIES: &[&str] = &["untrusted", "on-failure", "on-request", "never"];
/// Codex's sandboxes.
pub const SANDBOXES: &[&str] = &["read-only", "workspace-write", "danger-full-access"];

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
    /// The file is not TOML of the expected shape.
    Parse {
        /// 1-based line.
        line: usize,
        /// 1-based column.
        column: usize,
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
            Self::Parse { line, column, what } => {
                write!(
                    f,
                    "the configuration is malformed at line {line}, column {column}: {what}"
                )
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
    /// `[agents.NAME]`: the app's table, of which the host reads `program` and `args` and lets the rest go by.
    #[serde(default)]
    agents: IndexMap<String, toml::Value>,
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
    profiles: IndexMap<String, toml::Table>,
    #[serde(default)]
    accounts: IndexMap<String, RawAccount>,
    account_store: Option<String>,
    web: Option<RawWeb>,
    control: Option<RawControl>,
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
    executable: Option<String>,
    permission_mode: Option<String>,
    approval_policy: Option<String>,
    sandbox: Option<String>,
    bypass: Option<bool>,
    setting_sources: Option<Vec<String>>,
    modes: Option<Vec<String>>,
    cache_lifetime: Option<String>,
    profiles: Option<Vec<String>>,
    accounts: Option<Vec<String>>,
    usage: Option<String>,
    title: Option<String>,
    listen: Option<String>,
    #[serde(default)]
    working_roots: Vec<String>,
    worktrees: Option<bool>,
    peers: Option<Vec<String>>,
    max_peers: Option<usize>,
}

/// A harness entry's keys for what its command line asks, taken out of [`RawAgent`].
struct RawAsks {
    permission_mode: Option<String>,
    approval_policy: Option<String>,
    sandbox: Option<String>,
    bypass: Option<bool>,
    setting_sources: Option<Vec<String>>,
    modes: Option<Vec<String>>,
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
    #[serde(default)]
    working_roots: Vec<String>,
    peers: Option<Vec<String>>,
    max_peers: Option<usize>,
}

impl Config {
    /// Reads `path`. Relative `library` and `source` paths are taken from the file's directory, where a provider runs.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Read`] when the file cannot be read, and as [`Config::from_str_in`].
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let bytes = std::fs::read(path).map_err(|source| ConfigError::Read {
            path: path.to_owned(),
            source,
        })?;
        let text = core::str::from_utf8(&bytes).map_err(|error| {
            let (line, column) = position(&bytes, error.valid_up_to());
            ConfigError::Parse {
                line,
                column,
                what: "the file is not UTF-8".to_owned(),
            }
        })?;
        let base = path.parent().unwrap_or_else(|| Path::new(""));
        Self::from_str_in(text, base)
    }

    /// Parses canoa.cfg's text, with relative `library` and `source` paths taken from `base`, where a provider runs.
    /// Touches no file.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Parse`] for text that is not TOML of this shape, with its position; [`ConfigError::Invalid`] for a
    /// setting the host refuses.
    pub fn from_str_in(text: &str, base: &Path) -> Result<Self, ConfigError> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let raw: RawFile = toml::from_str(text).map_err(|error| parse_error(text, &error))?;
        let limit_phrases = raw.usage.and_then(|usage| usage.limit_phrases);
        let local_agents = local_agents_of(&raw.agents)?;
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
        let Some(host) = raw.host else {
            return Ok(Self {
                limit_phrases,
                local_agents,
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
            let profile = profile_of(&name, table, &directory)?;
            profiles.insert(name, profile);
        }
        let account_store = match host.account_store.as_deref() {
            None | Some("os") => AccountStore::Os,
            Some("file") => AccountStore::File,
            Some(_) => {
                return Err(ConfigError::Invalid {
                    entry: None,
                    what: "account_store is os or file".to_owned(),
                });
            }
        };
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
            if let Some(title) = &agent.title {
                check_title(&name, title)?;
            }
            let title = agent.title;
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
                working_roots: agent.working_roots,
                worktrees: agent.worktrees,
                peers: agent.peers,
                max_peers: agent.max_peers,
            };
            let asks = RawAsks {
                permission_mode: agent.permission_mode,
                approval_policy: agent.approval_policy,
                sandbox: agent.sandbox,
                bypass: agent.bypass,
                setting_sources: agent.setting_sources,
                modes: agent.modes,
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
            let kind = match (agent.program, agent.harness) {
                (Some(program), None) => {
                    if program.is_empty() {
                        return Err(invalid(&name, "program is empty"));
                    }
                    if agent.executable.is_some() {
                        return Err(invalid(&name, "executable goes with harness, not program"));
                    }
                    for (set, key) in [
                        (asks.permission_mode.is_some(), "permission_mode"),
                        (asks.approval_policy.is_some(), "approval_policy"),
                        (asks.sandbox.is_some(), "sandbox"),
                        (asks.bypass.is_some(), "bypass"),
                        (asks.setting_sources.is_some(), "setting_sources"),
                        (asks.modes.is_some(), "modes"),
                        (agent.cache_lifetime.is_some(), "cache_lifetime"),
                        (agent.library.is_some(), "library"),
                        (agent.source.is_some(), "source"),
                        (agent.source_settings.is_some(), "source_settings"),
                        (agent.import_search.is_some(), "import_search"),
                        (agent.settings.is_some(), "settings"),
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
                    let built_in = BUILT_IN_HARNESSES.contains(&harness.as_str());
                    if built_in && origin.is_some() {
                        return Err(invalid(
                            &name,
                            "claude and codex are built in: library and source load an outside provider under its own id",
                        ));
                    }
                    if origin.is_none() && !built_in {
                        return Err(invalid(
                            &name,
                            "harness is claude, codex, or the id of an outside provider that library or source loads",
                        ));
                    }
                    let settings = agent
                        .settings
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
                            (asks.permission_mode.is_some(), "permission_mode"),
                            (asks.approval_policy.is_some(), "approval_policy"),
                            (asks.sandbox.is_some(), "sandbox"),
                            (asks.bypass.is_some(), "bypass"),
                            (asks.setting_sources.is_some(), "setting_sources"),
                            (asks.modes.is_some(), "modes"),
                            (agent.executable.is_some(), "executable"),
                        ] {
                            if set {
                                return Err(invalid(
                                    &name,
                                    &format!(
                                        "{key} goes with harness claude or codex; an outside provider takes its own settings"
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
            served.title = title;
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
        for (name, table) in host.tools {
            check_name(&name)?;
            let (common, spec) = tool_of(&name, table)?;
            entries.push(entry(name, EntryKind::Tool { spec }, common)?);
        }
        check_unique(&entries)?;
        let control = host.control.map(control_of).transpose()?;
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
            accounts,
            account_store,
            local_agents,
            control,
        })
    }

    /// The entry called `name`.
    #[must_use]
    pub fn entry(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.name == name)
    }
}

/// `[host.control]`. Errors name a key, never a value.
fn control_of(raw: RawControl) -> Result<ControlConfig, ConfigError> {
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
                    refused("store.write is no grant: only the person running this host installs")
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
    if listen.is_some() && grants.is_empty() {
        return Err(refused(
            "listen without a key under grants would turn every key away",
        ));
    }
    Ok(ControlConfig {
        listen,
        grants,
        max_records,
        max_record_bytes,
    })
}

fn invalid(name: &str, what: &str) -> ConfigError {
    ConfigError::Invalid {
        entry: Some(name.to_owned()),
        what: what.to_owned(),
    }
}

/// The harnesses the host carries: providers of `acp_harness` registered in this process.
pub const BUILT_IN_HARNESSES: &[&str] = &["claude", "codex"];

/// The keys that say where an outside provider's code is.
struct OriginKeys<'a> {
    library: Option<&'a str>,
    source: Option<&'a str>,
    source_settings: Option<toml::Table>,
    import_search: Option<&'a str>,
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

/// The `settings` of a `claude` or `codex` entry: each of its keys that an entry key also says is caught as set twice, a mode,
/// policy or sandbox must be text and one the provider takes, and a relative `executable` is made absolute against canoa.cfg's
/// directory, as the entry's own key is, since the command line runs in a session's directory, which a client chooses.
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
    let mut map = settings.as_map().clone();
    if executable_key && map.contains_key("executable") {
        return Err(invalid(
            name,
            "executable is set twice: as a key and in settings",
        ));
    }
    let checked: &[(&str, &[&str])] = if harness == "claude" {
        &[("permission_mode", PERMISSION_MODES)]
    } else {
        &[
            ("approval_policy", APPROVAL_POLICIES),
            ("sandbox", SANDBOXES),
        ]
    };
    for (key, allowed) in checked {
        match map.get(*key) {
            None => {}
            Some(serde_json::Value::String(value)) if allowed.contains(&value.as_str()) => {}
            Some(serde_json::Value::String(_)) => {
                return Err(invalid(
                    name,
                    &format!("settings.{key} is one of {}", allowed.join(", ")),
                ));
            }
            Some(_) => return Err(invalid(name, &format!("settings.{key} must be text"))),
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

/// A `claude` or `codex` entry's own keys, each checked against what the provider takes, and refused for the other one.
/// A key that `settings` also holds is refused: the entry would say it twice.
fn harness_options(
    name: &str,
    harness: &str,
    agent: &RawAsks,
    settings: Option<&Settings>,
) -> Result<HarnessOptions, ConfigError> {
    let claudes = [
        ("permission_mode", agent.permission_mode.is_some()),
        ("setting_sources", agent.setting_sources.is_some()),
    ];
    let codexs = [
        ("approval_policy", agent.approval_policy.is_some()),
        ("sandbox", agent.sandbox.is_some()),
    ];
    let theirs = if harness == "claude" {
        &codexs[..]
    } else {
        &claudes[..]
    };
    if let Some((key, _)) = theirs.iter().find(|(_, set)| *set) {
        return Err(invalid(
            name,
            &format!("{key} is not a setting of harness {harness}"),
        ));
    }
    if let Some(settings) = settings {
        for (key, set) in [
            ("permission_mode", agent.permission_mode.is_some()),
            ("approval_policy", agent.approval_policy.is_some()),
            ("sandbox", agent.sandbox.is_some()),
            ("setting_sources", agent.setting_sources.is_some()),
            ("modes", agent.modes.is_some()),
        ] {
            if set && settings.as_map().contains_key(key) {
                return Err(invalid(
                    name,
                    &format!("{key} is set twice: as a key and in settings"),
                ));
            }
        }
        if agent.bypass.is_some() && settings.as_map().contains_key("allow_bypass") {
            return Err(invalid(
                name,
                "bypass is set twice: as a key and as allow_bypass in settings",
            ));
        }
    }
    let one_of = |value: &Option<String>, allowed: &[&str], key: &str| match value {
        Some(value) if !allowed.contains(&value.as_str()) => Err(invalid(
            name,
            &format!("{key} is one of {}", allowed.join(", ")),
        )),
        _ => Ok(value.clone()),
    };
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
    let modes = agent
        .modes
        .as_ref()
        .map(|ids| {
            ids.iter()
                .map(|id| {
                    coqui_acp_harness::Mode::from_id(id).ok_or_else(|| {
                        invalid(
                            name,
                            &format!(
                                "modes holds only {}",
                                coqui_acp_harness::Mode::ALL
                                    .map(coqui_acp_harness::Mode::id)
                                    .join(", ")
                            ),
                        )
                    })
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?;
    Ok(HarnessOptions {
        setting_sources: agent.setting_sources.clone(),
        modes,
        permission_mode: one_of(&agent.permission_mode, PERMISSION_MODES, "permission_mode")?,
        approval_policy: one_of(&agent.approval_policy, APPROVAL_POLICIES, "approval_policy")?,
        sandbox: one_of(&agent.sandbox, SANDBOXES, "sandbox")?,
        bypass: agent.bypass.unwrap_or(false),
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
    let profile =
        Profile::from_table(table, LayerKind::User).map_err(|error| refused(error.to_string()))?;
    Ok(HostProfile {
        profile,
        overlays,
        allow_mcp,
        directory: directory.to_owned(),
    })
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
fn tool_of(name: &str, mut table: toml::Table) -> Result<(RawCommon, ToolSpec), ConfigError> {
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

fn entry(name: String, kind: EntryKind, common: RawCommon) -> Result<Entry, ConfigError> {
    let listen = match common.listen {
        None => Listen::Address(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0)),
        Some(text) => Listen::parse(&text).map_err(|what| invalid(&name, &what))?,
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
        local: false,
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
            if entry.listen.is_fixed() && earlier.listen == entry.listen {
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

/// The 1-based line and column of byte `offset`.
fn position(text: &[u8], offset: usize) -> (usize, usize) {
    let before = &text[..offset.min(text.len())];
    let line = before.iter().filter(|byte| **byte == b'\n').count() + 1;
    let start = before
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |at| at + 1);
    let column = String::from_utf8_lossy(&before[start..]).chars().count() + 1;
    (line, column)
}

/// The parser's position, and its words only where they come from the schema: a value, or a key nobody declared, is never told.
fn parse_error(text: &str, error: &toml::de::Error) -> ConfigError {
    let (line, column) = error
        .span()
        .map_or((1, 1), |span| position(text.as_bytes(), span.start));
    let message = error.message();
    let what = if message.starts_with("missing field") {
        message.to_string()
    } else if message.starts_with("unknown field") {
        "a setting this host does not know".to_owned()
    } else if message.starts_with("invalid type") {
        "a setting has the wrong type".to_owned()
    } else if message.starts_with("duplicate key") {
        "a key is given twice".to_owned()
    } else {
        "not TOML of the expected shape".to_owned()
    };
    ConfigError::Parse { line, column, what }
}

#[cfg(test)]
mod tests;
