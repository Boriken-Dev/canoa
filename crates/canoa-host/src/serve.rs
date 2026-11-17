//! Each entry on a loopback WebSocket of its own: a peer proves a key the trust store accepts, then every message is sealed
//! in the session the proof ended in, and a fresh agent process serves that connection until either side ends.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use core::net::SocketAddr;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use core::time::Duration;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};
use std::thread;

use crate::portal_link::PortalSockets;
mod door;
mod sessions;
use canoa::{Capability, ClientId, Registry, Service, ServiceId};
use coqui_acp_bridge::agent::Agent as BridgeAgent;
use coqui_acp_bridge::relay::relay;
use coqui_agentic_peer::auth::{self, Identity, Method, Proof, Sealed, Trust};
use coqui_agentic_peer::link::Prefilled;
use coqui_agentic_peer::mux;
use coqui_builder::builder::BuildError;
use coqui_peer_webcrypto::{PeerId, TrustStore};
use coqui_transport::{Channel, Failure};
use coqui_transport_web::Limits;
use coqui_transport_web::websocket::{self, Frames};
use door::Door;
use indexmap::IndexMap;

use crate::accounts::matching::{Catalogs, fits};
use crate::accounts::{Accounts, Gated, Held};
use crate::child::{self, Running};
use crate::client_tools::{self, ToolsLayer, ToolsPeek};
use crate::config::{
    BUILT_IN_HARNESSES, Config, Entry, EntryKind, HarnessOptions, HostProfile, ImportSearch,
    Layers, Listen, Origin, Settings, UsageSource,
};
use crate::control;
use crate::delegate::{self, Delegating, Delegation, Peek, Principal};
use crate::history::{self, Recording};
use crate::inproc::{self, InProcess};
use crate::kb_mcp::{self, KbLayer};
use crate::kind;
use crate::local::{self, Link};
use crate::pipe;
use crate::profile::hosted::{self, Connection, Hosted, Offer, Starting};
use crate::profile::{Who, account_words};
use crate::provider::HostLine;
use crate::scope::{Scope, Scoped};
use crate::secrets::{self, Layer, Raw};
use crate::sources;
use crate::store::mcp::{self as store_mcp, StoreLayer, StorePeek};
use crate::transport;
use crate::usage_splice::{Antigravity, Starter};
use crate::worktree::Worktrees;

/// Requests a harness provider's connection works on at once, unless the entry's `settings` say otherwise.
const HARNESS_MAX_REQUESTS: u64 = 64;
/// Worktree requests one connection may make before its first ACP message.
const MAX_WORKTREE_REQUESTS: usize = 16;

/// The longest ACP message either way, as one line.
pub const MAX_LINE: usize = 16 * 1024 * 1024;
/// Connections at once that have not proven a key, across every entry; past it a new one is dropped.
pub const MAX_UNPROVEN: usize = 64;
/// How long a peer has to prove its key.
pub const PROOF_LIMIT: Duration = Duration::from_secs(15);
/// How long a proven peer has to send its first message.
pub const FIRST_MESSAGE: Duration = Duration::from_secs(60);
/// Messages an unproven peer may have queued.
const UNPROVEN_PENDING: usize = 8;
/// How often a listener looks up to see whether [`Stopper::stop`] was called.
const STOP_POLL: Duration = Duration::from_millis(100);
pub use crate::child::{OUTPUT_LIMIT, START_LIMIT};
/// What a peer is granted on an entry, in [`Registry`] terms.
pub const ACP_CAPABILITY: &str = "acp/1";

/// What [`Serving`] needs beyond the configuration.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ServeOptions {
    /// The program that serves a provider, a tool, a profile's launch or the delegation's stdio bridge: this host's own executable, run as
    /// `PROGRAM [program_args] provider ...`.
    pub provider_program: PathBuf,
    /// What goes before the subcommand on `provider_program`'s command line: nothing for `canoa`, the flag that makes an
    /// application hosting this library behave as one when it starts itself.
    pub program_args: Vec<OsString>,
    /// What one key may carry in each direction, in bytes; the method's own default, 2^38, when `None`.
    pub key_limit: Option<u64>,
    /// How long a question put to the person (a sign-in's form, an install's) waits for its answer: four minutes by default, which
    /// is how long the request stays bound, and the longest the application accepts.
    pub ask_wait: Duration,
    /// Whether a Noise sender changes its key at the limit; WebCrypto has no key change and ends the session there.
    pub rekey: bool,
    /// Whether a configuration with no entry binds, serving nothing, rather than being refused.
    pub nothing_to_serve: bool,
    /// The host's directory for what it keeps, where an agent started only to read its usage gets an empty working
    /// directory; without one, no such agent is started.
    pub state_dir: Option<PathBuf>,
    /// Whether the host's own sources (Claude, Codex, Antigravity, Grok and Pi) are served too, over [`Local`] only.
    pub local_sources: bool,
    /// The programs the control channel's `store/*` methods keep, and the ids of the stores it offers; without it, the built-in
    /// stores over this user's own programs folder.
    pub programs: Option<(Arc<crate::store::programs::Programs>, Vec<String>)>,
    /// The files canoa.cfg was read from. With them, each connection to one of the host's own sources reads them again for
    /// its profile, so a change to `[overlays]`, `[profile]` or `[agents.NAME]` reaches the next session; a file that no longer
    /// parses leaves the profile as it was.
    pub layers: Option<Layers>,
    /// The person's own canoa.cfg, which a key granted `config.write` edits by section; without it those methods are refused.
    pub config_file: Option<PathBuf>,
    /// The user's home, which a key granted `folders` may browse; the environment's when `None`.
    pub home_dir: Option<PathBuf>,
    /// Whether a supervisor starts the host again when it ends with `cli::STATUS_RESTART`; only then is `host/restart` offered.
    pub supervised: bool,
    /// The canoa.cfg files the control channel's `accounts/*` methods read and write; with the state directory, which is the
    /// home they keep accounts in. Without it the host manages no accounts over the control channel.
    pub account_layers: Option<crate::config::Layers>,
    /// Where a host run for a portal listens: every entry and the control channel on a socket in a directory a group may
    /// reach, and a socket that introduces a browser's key. Unix only.
    pub portal: Option<crate::portal_link::PortalSockets>,
}

impl ServeOptions {
    /// Options running providers through `provider_program`, with each method's default session settings.
    #[must_use]
    pub fn new(provider_program: impl Into<PathBuf>) -> Self {
        Self {
            provider_program: provider_program.into(),
            program_args: Vec::new(),
            key_limit: None,
            ask_wait: crate::secrets::ask::ASK_WAIT,
            rekey: true,
            nothing_to_serve: false,
            state_dir: None,
            local_sources: false,
            programs: None,
            layers: None,
            config_file: None,
            home_dir: None,
            supervised: false,
            account_layers: None,

            portal: None,
        }
    }

    /// Runs the host for a portal: see [`crate::portal_link::PortalSockets`]. Every entry's address is replaced by a socket, the
    /// web app is not served, and every key the portal introduces is the user's own browser, so each gets the control
    /// channel's page grants.
    #[must_use]
    pub fn with_portal(mut self, sockets: crate::portal_link::PortalSockets) -> Self {
        self.portal = Some(sockets);
        self
    }

    /// Reads canoa.cfg's profile again from `layers` at each connection to one of the host's own sources.
    #[must_use]
    pub fn with_layers(mut self, layers: Layers) -> Self {
        self.layers = Some(layers);
        self
    }

    /// Lets the control channel's owner manage accounts in these canoa.cfg files, in the state directory's home.
    #[must_use]
    pub fn with_account_layers(mut self, layers: crate::config::Layers) -> Self {
        self.account_layers = Some(layers);
        self
    }

    /// Serves the host's own sources over [`Local`] alongside canoa.cfg's entries. A name canoa.cfg already uses is left to it.
    #[must_use]
    pub const fn with_local_sources(mut self, on: bool) -> Self {
        self.local_sources = on;
        self
    }

    /// Waits `wait` for the person to answer a question the host puts to them, and binds the question for as long.
    #[must_use]
    pub const fn with_ask_wait(mut self, wait: Duration) -> Self {
        self.ask_wait = wait;
        self
    }

    /// Offers the control channel's `store/*` methods over `programs`, through the stores named `providers`.
    #[must_use]
    pub fn with_programs(
        mut self,
        programs: Arc<crate::store::programs::Programs>,
        providers: Vec<String>,
    ) -> Self {
        self.programs = Some((programs, providers));
        self
    }

    /// The canoa.cfg a key granted `config.write` edits, by section.
    #[must_use]
    pub fn with_config_file(mut self, file: impl Into<PathBuf>) -> Self {
        self.config_file = Some(file.into());
        self
    }

    /// Says that a supervisor restarts the host when it ends with `cli::STATUS_RESTART`, which offers `host/restart`.
    #[must_use]
    pub const fn with_supervised(mut self, supervised: bool) -> Self {
        self.supervised = supervised;
        self
    }

    /// The home a key granted `folders` may browse, in place of the environment's.
    #[must_use]
    pub fn with_home_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.home_dir = Some(dir.into());
        self
    }

    /// The directory the host keeps its own working files in, which a private read of an agent's usage needs.
    #[must_use]
    pub fn with_state_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.state_dir = Some(dir.into());
        self
    }

    /// What one key may carry in each direction, in bytes. No floor; above the default the host logs it as a non-default setting.
    #[must_use]
    pub const fn with_key_limit(mut self, bytes: u64) -> Self {
        self.key_limit = Some(bytes);
        self
    }

    /// Whether a Noise sender changes its key at the limit, rather than ending the session.
    #[must_use]
    pub const fn with_rekey(mut self, rekey: bool) -> Self {
        self.rekey = rekey;
        self
    }

    /// What goes before the subcommand on the provider program's command line.
    #[must_use]
    pub fn with_program_args(mut self, args: Vec<OsString>) -> Self {
        self.program_args = args;
        self
    }

    /// Lets a configuration with no entry bind, serving nothing until it is stopped.
    #[must_use]
    pub const fn with_nothing_to_serve(mut self, allowed: bool) -> Self {
        self.nothing_to_serve = allowed;
        self
    }
}

/// Why the host could not start serving.
#[derive(Debug)]
#[non_exhaustive]
pub enum ServeError {
    /// The configuration has no entry.
    NothingToServe,
    /// An entry's address could not be bound.
    Listen {
        /// The entry.
        entry: String,
        /// Why.
        failure: Failure,
    },
    /// An entry's pipe or Unix socket could not be bound.
    Pipe {
        /// The entry.
        entry: String,
        /// Why.
        error: pipe::Error,
    },
    /// An entry's transport provider could not be loaded, found, configured, or started on its endpoint.
    Transport {
        /// The entry.
        entry: String,
        /// Why; names a setting, never a value.
        what: String,
    },
    /// The proof could not be built from the options.
    Proof(String),
    /// A harness entry's provider could not be loaded, found, or configured.
    Harness {
        /// The entry.
        entry: String,
        /// Why; names a setting, never a value.
        what: String,
    },
    /// The accounts canoa.cfg names cannot be served.
    Accounts {
        /// Why; names a key, never a value.
        what: String,
    },
    /// The host's ledger could not be opened.
    Control(control::ControlError),
}

impl fmt::Display for ServeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NothingToServe => f.write_str(
                "canoa.cfg has no [host.agents], [host.providers] or [host.tools] entry to serve",
            ),
            Self::Listen { entry, failure } => {
                write!(f, "host entry {entry} could not listen: {failure}")
            }
            Self::Pipe { entry, error } => {
                write!(f, "host entry {entry} could not listen: {error}")
            }
            Self::Transport { entry, what } => {
                write!(
                    f,
                    "host entry {entry}: its transport provider is unusable: {what}"
                )
            }
            Self::Proof(reason) => write!(f, "the proof could not be built: {reason}"),
            Self::Harness { entry, what } => {
                write!(
                    f,
                    "host entry {entry}: its harness provider is unusable: {what}"
                )
            }
            Self::Accounts { what } => write!(f, "[host.accounts]: {what}"),
            Self::Control(error) => write!(f, "the control channel's ledger {error}"),
        }
    }
}

impl core::error::Error for ServeError {}

/// An entry that is listening.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Listening {
    /// The entry's name.
    pub name: String,
    /// `agent`, `provider` or `tool`.
    pub kind: &'static str,
    /// What a client dials: `ws://ADDRESS`, or `pipe:NAME` or `unix:PATH` for an entry on a pipe, which the served web page
    /// never lists; empty for a local entry.
    pub url: String,
    /// The provider id of a `harness` entry; `None` for any other entry.
    pub harness: Option<String>,
    /// The `transport` provider's id of an entry that listens through one, whose `url` is then the endpoint it says a peer
    /// dials; `None` for an entry on an address or a pipe.
    pub transport: Option<String>,
    /// The entry's `title`, if it has one.
    pub title: Option<String>,
    /// Whether it is reached over [`Local`] only: one of the host's own sources, with no address.
    pub local: bool,
    /// The programs a local entry needs that were not found; empty for any other, and for a source that can start.
    pub missing: Vec<String>,
}

impl Listening {
    /// Whether a browser can dial it: it listens on an address, not on a pipe, a Unix socket or a transport provider, whose
    /// framing a page does not know.
    #[must_use]
    pub fn on_address(&self) -> bool {
        self.transport.is_none() && !self.url.starts_with("pipe:") && !self.url.starts_with("unix:")
    }

    /// The entry as `serve` prints it and `/canoa.json` lists it: `name`, `kind`, `url`, and `harness` and `title` when set.
    pub(crate) fn to_json(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut map = serde_json::Map::new();
        map.insert("name".into(), self.name.clone().into());
        map.insert("kind".into(), self.kind.into());
        if self.local {
            map.insert("local".into(), true.into());
            map.insert("missing".into(), self.missing.clone().into());
        } else {
            map.insert("url".into(), self.url.clone().into());
        }
        if let Some(harness) = &self.harness {
            map.insert("harness".into(), harness.clone().into());
        }
        if let Some(transport) = &self.transport {
            map.insert("transport".into(), transport.clone().into());
        }
        if let Some(title) = &self.title {
            map.insert("title".into(), title.clone().into());
        }
        map
    }
}

/// A count of what is running, with a ceiling.
#[derive(Clone, Debug)]
pub(crate) struct Gate(Arc<AtomicUsize>, usize);

/// One place in a [`Gate`], given back when dropped.
pub(crate) struct Entered(Arc<AtomicUsize>);

impl Drop for Entered {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Gate {
    pub(crate) fn new(ceiling: usize) -> Self {
        Self(Arc::new(AtomicUsize::new(0)), ceiling)
    }

    /// The most it lets in at once.
    const fn ceiling(&self) -> usize {
        self.1
    }

    pub(crate) fn enter(&self) -> Option<Entered> {
        let mut running = self.0.load(Ordering::SeqCst);
        while running < self.1 {
            match self
                .0
                .compare_exchange(running, running + 1, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => return Some(Entered(Arc::clone(&self.0))),
                Err(now) => running = now,
            }
        }
        None
    }
}

/// What serves one connection of an agent entry.
#[derive(Debug)]
enum Agent {
    /// A program of its own on its standard streams.
    Program {
        program: OsString,
        args: Vec<OsString>,
    },
    /// An `acp_harness` provider, in this process.
    Harness(Arc<kind::Harness>),
}

/// What runs for one entry's connections.
#[derive(Debug)]
enum Run {
    Agent {
        agent: Agent,
        scope: Scope,
        /// The profiles a client may choose among; `None` starts the program bare.
        offer: Option<Arc<Offer>>,
        /// Reads the offer again from canoa.cfg each time a connection starts; only the host's own sources have one.
        refresh: Option<Box<Refresh>>,
        worktrees: Worktrees,
    },
    Provider {
        program: OsString,
        args: Vec<OsString>,
        line: HostLine,
    },
    /// The control channel, for the keys `[host.control]` grants.
    Control {
        grants: IndexMap<PeerId, BTreeSet<control::Grant>>,
        /// Every paired key gets these grants whatever `grants` says: a host run for a portal gives the user's own browser
        /// every page grant, and the native door gives a browser key the grants of `[host.native] control`.
        everyone: Option<BTreeSet<control::Grant>>,
    },
    /// `@tools`: where a program offers its tools to this host's sessions, for the keys that hold `tools.provide`. Only the
    /// native door and this process open it.
    Tools { grants: BTreeSet<control::Grant> },
}

/// What builds a local entry's offer again from canoa.cfg's layers, so that installing an overlay, skill or server reaches
/// the next connection with no host restart.
#[derive(Debug)]
struct Refresh {
    layers: Layers,
    entry: String,
    /// The entry's own profile names and the configuration's `[host.profiles]`, which the layers do not change while running.
    names: Vec<String>,
    profiles: IndexMap<String, HostProfile>,
    launcher: OsString,
    before: Vec<OsString>,
    /// The offer the last readable file gave.
    current: Mutex<Option<Arc<Offer>>>,
}

impl Refresh {
    /// The offer for a connection starting now: the file read again, or the last good one when it no longer parses.
    fn offer(&self) -> Option<Arc<Offer>> {
        let mut current = self.current.lock().unwrap_or_else(PoisonError::into_inner);
        match Config::load_layers(&self.layers) {
            Ok(config) => {
                let local = config.local_profiles.for_agent(&self.entry);
                *current = hosted::offer(
                    &self.names,
                    &self.profiles,
                    local.as_ref(),
                    self.launcher.clone(),
                    self.before.clone(),
                );
            }
            Err(error) => {
                tracing::warn!(
                    entry = self.entry,
                    "canoa.cfg could not be read again, so the profile offered stays as it was: {error}"
                );
            }
        }
        current.clone()
    }
}

#[derive(Debug)]
struct Served {
    name: String,
    restricted: bool,
    /// Whether its sessions can be given an MCP server, which the delegation tools are: a tool of the host's takes none.
    delegable: bool,
    run: Run,
    running: Gate,
    /// Set for an entry whose sessions outlive their connections.
    keep: Option<sessions::Keep>,
}

/// What every connection shares.
struct Shared {
    method: Proof,
    store: Arc<TrustStore>,
    registry: Registry,
    unproven: Gate,
    agents: Arc<Mutex<Agents>>,
    /// Held while agents are being ended, so that whoever else ends them returns only once they have.
    ending: Mutex<()>,
    /// Set by [`Stopper::stop`]: every listener and link ends.
    stopped: Arc<AtomicBool>,
    program_args: Vec<OsString>,
    /// A host run for a portal: the only user a connection on a socket may be from.
    portal_uid: Option<u32>,
    /// The ledger and the rest of what belongs to no entry.
    control: Arc<control::Control>,
    /// What lets an agent session hand a task to another source; `None` when `[host.delegate]` is off.
    delegation: Option<Delegation>,
    /// The sessions the host serves, recorded in its home.
    history: Arc<history::Store>,
    /// What gives a connection's sessions the secret tools.
    secrets: Arc<secrets::Hub>,
    /// What gives a connection's sessions the store tools (search and inspect).
    store_tools: Arc<store_mcp::Hub>,
    kb_tools: Arc<kb_mcp::Hub>,
    /// What gives a connection's sessions the tools programs offer (a browser extension's), and takes their offers.
    client_tools: Arc<client_tools::Hub>,
    /// What `[host.control]` grants each key, and what every key holds beside it: the store tools are given only to a key that
    /// may read what they read.
    control_grants: IndexMap<PeerId, BTreeSet<control::Grant>>,
    control_everyone: Option<BTreeSet<control::Grant>>,
    /// The sessions kept for more than one connection.
    hosted: Arc<sessions::Hosted>,
}

impl Shared {
    /// The grants of the key behind a connection that may be given the store tools, or `None` for a task another agent handed
    /// out, which has no person at its end.
    fn store_grants(&self, peer: &PeerId, task: bool) -> Option<BTreeSet<control::Grant>> {
        if task {
            None
        } else if *peer == self.method.peer_id() {
            Some(control::Grant::all().into_iter().collect())
        } else if let Some(everyone) = &self.control_everyone {
            Some(everyone.clone())
        } else {
            self.control_grants.get(peer).cloned()
        }
    }
}

/// Every agent process running, so stopping the host can end them: on Unix nothing else does when the host goes.
#[derive(Default)]
struct Agents {
    stopping: bool,
    next: u64,
    running: BTreeMap<u64, Process>,
}

/// A process the host started for one connection.
enum Process {
    Agent(Arc<BridgeAgent>),
    Provider(Arc<Running>),
    Translator(Arc<InProcess>),
}

impl Process {
    /// Ends it and what it started.
    fn stop(&self) {
        match self {
            Self::Agent(agent) => {
                let _ = agent.stop();
            }
            Self::Provider(running) => running.stop(),
            Self::Translator(agent) => agent.stop(),
        }
    }

    /// Tells an agent in this process to end, so that all of them end at once rather than one after another.
    fn signal(&self) {
        if let Self::Translator(agent) = self {
            agent.signal();
        }
    }
}

impl Shared {
    /// Starts an agent and records it, under the key to [`Shared::forget`] it by; `Ok(None)` once the host is stopping.
    /// Both happen under the lock [`Stopper::stop_agents`] takes, so no agent starts unseen while the host stops.
    fn start(
        &self,
        program: &OsString,
        args: &[OsString],
    ) -> std::io::Result<Option<(u64, Arc<BridgeAgent>)>> {
        self.record(|| {
            let agent = Arc::new(BridgeAgent::spawn_with(
                &crate::spawn::Agents,
                program,
                args,
                MAX_LINE,
            )?);
            Ok((Process::Agent(Arc::clone(&agent)), agent))
        })
    }

    /// Starts a process with `spawn` and records it, as [`Shared::start`] does.
    fn record<T>(
        &self,
        spawn: impl FnOnce() -> std::io::Result<(Process, T)>,
    ) -> std::io::Result<Option<(u64, T)>> {
        let mut agents = self.agents.lock().unwrap_or_else(PoisonError::into_inner);
        if agents.stopping {
            return Ok(None);
        }
        let (process, started) = spawn()?;
        agents.next += 1;
        let key = agents.next;
        agents.running.insert(key, process);
        Ok(Some((key, started)))
    }

    fn forget(&self, key: u64) {
        self.agents
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .running
            .remove(&key);
    }
}

/// How an agent started only to read its usage is started: recorded with the others, so that stopping the host ends it and
/// none starts while it stops. The second part of what it hands back forgets the agent once it has ended.
/// What re-reads an entry's profile at each connection: only an entry of the host's own or one with `own_profile`, when the options hold canoa.cfg's files.
fn refresh_of(
    entry: &Entry,
    names: &[String],
    config: &Config,
    options: &ServeOptions,
) -> Option<Refresh> {
    let layers = options
        .layers
        .clone()
        .filter(|_| entry.local || entry.own_profile)?;
    let launcher = options.provider_program.as_os_str().to_owned();
    let before = options.program_args.clone();
    let local = config.local_profiles.for_agent(&entry.name);
    Some(Refresh {
        layers,
        entry: entry.name.clone(),
        names: names.to_vec(),
        profiles: config.profiles.clone(),
        current: Mutex::new(hosted::offer(
            names,
            &config.profiles,
            local.as_ref(),
            launcher.clone(),
            before.clone(),
        )),
        launcher,
        before,
    })
}

fn private_starter(agents: &Arc<Mutex<Agents>>) -> Starter {
    let agents = Arc::clone(agents);
    Starter(Arc::new(move |program, args| {
        let mut held = agents.lock().unwrap_or_else(PoisonError::into_inner);
        if held.stopping {
            return Ok(None);
        }
        let agent = Arc::new(BridgeAgent::spawn_with(
            &crate::spawn::Agents,
            program,
            args,
            MAX_LINE,
        )?);
        held.next += 1;
        let key = held.next;
        held.running.insert(key, Process::Agent(Arc::clone(&agent)));
        drop(held);
        let agents = Arc::clone(&agents);
        let forget: Box<dyn FnOnce() + Send> = Box::new(move || {
            agents
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .running
                .remove(&key);
        });
        Ok(Some((agent, forget)))
    }))
}

/// Stops what a [`Serving`] runs, from another thread.
#[derive(Clone)]
pub struct Stopper(Arc<Shared>);

impl fmt::Debug for Stopper {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Stopper").finish_non_exhaustive()
    }
}

impl Stopper {
    /// Ends every listener, link and agent: [`Serving::run`] returns with the addresses free, and each link still in its
    /// proof ends by [`PROOF_LIMIT`] at the latest.
    pub fn stop(&self) {
        self.0.stopped.store(true, Ordering::SeqCst);
        if let Some(delegation) = &self.0.delegation {
            delegation.stop();
        }
        self.stop_agents();
    }

    /// Ends every agent process running, with what each started, and every later connection before its agent starts. It
    /// returns once they have ended, also when another thread is ending them. The listeners stay bound: end the process to
    /// free them, or use [`Stopper::stop`].
    pub fn stop_agents(&self) {
        let _ending = self.0.ending.lock().unwrap_or_else(PoisonError::into_inner);
        end_all(&self.0.agents);
    }
}

/// Marks the host as stopping, so no agent starts after, and ends every agent recorded, all signalled before any is stopped.
fn end_all(agents: &Mutex<Agents>) {
    let running = {
        let mut agents = agents.lock().unwrap_or_else(PoisonError::into_inner);
        agents.stopping = true;
        core::mem::take(&mut agents.running)
    };
    for process in running.values() {
        process.signal();
    }
    for process in running.values() {
        process.stop();
    }
}

/// Why [`Local::open`] gave no link.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LocalError {
    /// The host serves no entry of that name.
    NoEntry,
    /// The host is stopping.
    Stopped,
    /// A thread for the connection could not be started.
    Thread,
}

impl fmt::Display for LocalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NoEntry => "this host serves no entry of that name",
            Self::Stopped => "this host is stopping",
            Self::Thread => "a thread for the connection could not be started",
        })
    }
}

impl core::error::Error for LocalError {}

/// A way into every entry of a [`Serving`] from inside its own process. A link opened here is served as a paired key's
/// connection is, with the entry's agent, scope, worktrees and `max_peers`, but without a proof or the trust store: whoever
/// holds a `Local` is this process, and no peer on a network can obtain one. It ignores an entry's `peers` list for that
/// reason, and ends with [`Stopper::stop`] like any connection.
#[derive(Clone)]
pub struct Local {
    shared: Arc<Shared>,
    served: Vec<Arc<Served>>,
}

impl fmt::Debug for Local {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Local").finish_non_exhaustive()
    }
}

impl Local {
    /// The names of the entries it reaches, in the configuration's order.
    #[must_use]
    pub fn entries(&self) -> Vec<&str> {
        self.served
            .iter()
            .map(|served| served.name.as_str())
            .collect()
    }

    /// Opens a connection to `entry`: the client's end of it, whose first message starts the entry's agent as a peer's would.
    /// Dropping or closing the end ends the connection and its agent.
    ///
    /// # Errors
    ///
    /// [`LocalError`]: no such entry, the host is stopping, or no thread could be started.
    pub fn open(&self, entry: &str) -> Result<Link, LocalError> {
        self.open_with(entry, None)
    }

    /// The secret this host shares with an application in its process, for the secret prompts of the connections [`Local::open`]
    /// gives it: 32 bytes minted when the host started, kept in memory, sent over no wire and to no agent. The application derives
    /// its half of the binding from it (`coqui_acp::secret::LinkKey`); `None` when the system has no randomness to mint one from.
    #[must_use]
    pub fn link_secret(&self) -> Option<zeroize::Zeroizing<[u8; 32]>> {
        self.shared.secrets.link_secret()
    }

    /// [`Local::open`], for a task of the delegation when `context` is given: what the task's connection is bounded by.
    pub(crate) fn open_with(
        &self,
        entry: &str,
        context: Option<delegate::ChildContext>,
    ) -> Result<Link, LocalError> {
        if entry == control::NAME {
            // Only this process's own link opens it; a task's connection has no entry of that name, whatever the list holds.
            return if reaches_control(entry, context.as_ref()) {
                self.open_control()
            } else {
                Err(LocalError::NoEntry)
            };
        }
        if entry == client_tools::NAME {
            // A program in this process offering its tools; a task's connection has no such entry.
            return if context.is_none() {
                self.open_tools()
            } else {
                Err(LocalError::NoEntry)
            };
        }
        let served = self
            .served
            .iter()
            .find(|served| served.name == entry)
            .ok_or(LocalError::NoEntry)?;
        if self.shared.stopped.load(Ordering::SeqCst) {
            return Err(LocalError::Stopped);
        }
        let (client, host) = local::pair(MAX_LINE, Arc::clone(&self.shared.stopped));
        let shared = Arc::clone(&self.shared);
        let served = Arc::clone(served);
        thread::Builder::new()
            .name("canoa-local".to_owned())
            .spawn(move || {
                let peer = shared.method.peer_id();
                serve_proven(&shared, &served, &host, &peer, context.as_ref(), true);
                host.close();
            })
            .map_err(|_| LocalError::Thread)?;
        Ok(client)
    }
}

/// Whether `entry` names the control channel for a caller that may have it: this process's own link, never a task's connection,
/// which is bounded by the context it was opened with and so gets no entry of that name.
fn reaches_control(entry: &str, context: Option<&delegate::ChildContext>) -> bool {
    entry == control::NAME && context.is_none()
}

impl Local {
    /// `@tools`, as this process: it offers its tools to the host's sessions.
    fn open_tools(&self) -> Result<Link, LocalError> {
        if self.shared.stopped.load(Ordering::SeqCst) {
            return Err(LocalError::Stopped);
        }
        let (client, host) =
            local::pair(client_tools::MAX_MESSAGE, Arc::clone(&self.shared.stopped));
        let shared = Arc::clone(&self.shared);
        thread::Builder::new()
            .name("canoa-tools".to_owned())
            .spawn(move || {
                let grants = control::Grant::all().into_iter().collect();
                shared
                    .client_tools
                    .provide(&shared.method.peer_id().to_string(), &grants, &host);
                host.close();
            })
            .map_err(|_| LocalError::Thread)?;
        Ok(client)
    }

    /// The control channel, as its owner.
    fn open_control(&self) -> Result<Link, LocalError> {
        if self.shared.stopped.load(Ordering::SeqCst) {
            return Err(LocalError::Stopped);
        }
        let (client, host) = local::pair(control::MAX_REQUEST, Arc::clone(&self.shared.stopped));
        let control = Arc::clone(&self.shared.control);
        thread::Builder::new()
            .name("canoa-control".to_owned())
            .spawn(move || {
                control::serve(&control, &control::Caller::Owner, &host);
                host.close();
            })
            .map_err(|_| LocalError::Thread)?;
        Ok(client)
    }
}

/// The host as the delegation sees it: the agents it serves, who may use them, and a way to open one in process.
struct DelegationHost {
    shared: alloc::sync::Weak<Shared>,
    served: Vec<Arc<Served>>,
    agents: Vec<delegate::Source>,
    identity: Identity,
    usage: Option<Arc<crate::usage_log::UsageLog>>,
}

impl delegate::Host for DelegationHost {
    fn sources(&self) -> Vec<delegate::Source> {
        self.agents.clone()
    }

    fn may_use(&self, who: &Principal, source: &str) -> bool {
        // Another machine's agent takes the task's text off this one: only the person's own sessions may send it there.
        let listed = &self.agents;
        if listed
            .iter()
            .any(|agent| agent.name == source && agent.remote.is_some())
        {
            return matches!(who, Principal::Owner);
        }
        let Some(shared) = self.shared.upgrade() else {
            return false;
        };
        let Some(served) = self.served.iter().find(|served| served.name == source) else {
            return false;
        };
        match who {
            Principal::Owner => true,
            Principal::Key(peer) => shared.allows(peer, served),
            Principal::Remote(peer) => shared.lists(peer, served),
        }
    }

    fn kind(&self, source: &str) -> Option<String> {
        let served = self.served.iter().find(|served| served.name == source)?;
        let entries = &self.agents;
        match &served.run {
            Run::Agent {
                agent: Agent::Harness(_),
                ..
            } => entries
                .iter()
                .find(|agent| agent.name == source)
                .and_then(|agent| agent.harness.as_deref())
                .map(|harness| format!("harness:{harness}")),
            Run::Agent {
                agent: Agent::Program { program, .. },
                ..
            } => Some(format!("program:{}", program.to_string_lossy())),
            _ => None,
        }
    }

    fn usage_of(&self, source: &str, session: &str) -> Option<serde_json::Value> {
        let served = self.served.iter().find(|served| served.name == source)?;
        let Run::Agent { scope, .. } = &served.run else {
            return None;
        };
        self.usage
            .as_ref()?
            .session_usage(scope.account_id()?, session)
    }

    fn prover(&self, peer: &PeerId) -> Result<Proof, String> {
        Proof::builder(self.identity.clone())
            .responders(Trust::pinned([*peer]))
            .build()
            .map_err(|error| error.to_string())
    }

    fn choices(
        &self,
        source: &str,
        who: &Principal,
        account: Option<&str>,
    ) -> Option<serde_json::Value> {
        let shared = self.shared.upgrade()?;
        let caller = match who {
            Principal::Owner => control::Caller::Owner,
            Principal::Key(peer) | Principal::Remote(peer) => control::Caller::Peer {
                id: peer.to_string(),
                grants: BTreeSet::new(),
            },
        };
        shared.control.delegation_choices(&caller, source, account)
    }

    fn open(&self, source: &str, context: delegate::ChildContext) -> Result<Link, LocalError> {
        let shared = self.shared.upgrade().ok_or(LocalError::Stopped)?;
        Local {
            shared,
            served: self.served.clone(),
        }
        .open_with(source, Some(context))
    }
}

/// Every entry bound to its address, not yet taking connections.
pub struct Serving {
    shared: Arc<Shared>,
    listeners: Vec<(Arc<Served>, Bound)>,
    /// The entries reached over [`Local`] only.
    local_only: Vec<Arc<Served>>,
    listening: Vec<Listening>,
    /// The pipe a browser's native messaging host relays to, unless `[host.native]` turns it off.
    door: Option<Door>,
}

/// What an entry listens on.
enum Bound {
    Socket(websocket::Listener),
    Pipe(pipe::Listener),
    Transport(alloc::boxed::Box<transport::Listening>),
}

impl fmt::Debug for Serving {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Serving")
            .field("listening", &self.listening)
            .finish_non_exhaustive()
    }
}

/// The arguments that make `program` serve a provider entry on its standard streams. They hold paths and names only: the
/// settings go on its standard input.
fn provider_args(
    before: &[OsString],
    entry: &Entry,
    origin: &Origin,
    provider: &str,
    directory: &std::path::Path,
    max_sessions: usize,
) -> Vec<OsString> {
    let (flag, file) = match origin {
        Origin::Library(library) => ("--library", library),
        Origin::Source { file, .. } => ("--source", file),
    };
    let mut args = before.to_vec();
    args.extend([
        OsString::from("provider"),
        OsString::from(flag),
        file.as_os_str().to_owned(),
        OsString::from("--provider"),
        OsString::from(provider),
        OsString::from("--max-sessions"),
        OsString::from(max_sessions.to_string()),
        OsString::from("--directory"),
        directory.as_os_str().to_owned(),
        OsString::from("--settings-on-stdin"),
    ]);
    if let Origin::Source { import_search, .. } = origin {
        args.push(OsString::from("--import-search"));
        args.push(OsString::from(import_search.as_str()));
    }
    for root in &entry.working_roots {
        args.push(OsString::from("--working-root"));
        args.push(root.as_os_str().to_owned());
    }
    args
}

/// The arguments that make `program` serve a tool entry: its name and the entry as JSON, which holds no setting a client
/// gave and nothing the tool's own command line does not show.
fn tool_args(before: &[OsString], name: &str, spec: &crate::tool::ToolSpec) -> Vec<OsString> {
    let mut args = before.to_vec();
    args.extend([
        OsString::from("tool"),
        OsString::from("--name"),
        OsString::from(name),
        OsString::from("--spec"),
        OsString::from(serde_json::to_string(spec).unwrap_or_default()),
    ]);
    args
}

fn portal_of(options: &ServeOptions) -> Option<&PortalSockets> {
    options.portal.as_ref()
}

/// Binds one entry's address or pipe, listing what is not the default of it in the log.
fn bind_listen(
    name: &str,
    table: &str,
    listen: &Listen,
    origins: &[String],
    shared: Option<&PortalSockets>,
) -> Result<(Bound, String), ServeError> {
    match listen {
        Listen::Address(address) => {
            let listener = plain_listener(*address, unproven_limits(), origins.to_vec()).map_err(
                |failure| ServeError::Listen {
                    entry: name.to_owned(),
                    failure,
                },
            )?;
            let bound = listener
                .local_addr()
                .map_err(|failure| ServeError::Listen {
                    entry: name.to_owned(),
                    failure,
                })?;
            if listener.plain_beyond_loopback() {
                tracing::info!(
                    "non-default: host.{}.{}.listen is {bound}, default loopback: any machine that reaches it may try the proof",
                    table,
                    name
                );
            }
            Ok((Bound::Socket(listener), url_of(bound)))
        }
        Listen::Pipe(pipe_name) => {
            // Every setting is the crate's default, the access control included; a receive sets its own read timeout.
            let bound = match shared {
                Some(sockets) => sockets.bind(pipe_name),
                None => pipe::Listener::bind(pipe_name),
            };
            let listener = bound.map_err(|error| ServeError::Pipe {
                entry: name.to_owned(),
                error,
            })?;
            for setting in listener.non_defaults() {
                tracing::info!(
                    "non-default: host.{}.{}.listen has {} at {}, default {}",
                    table,
                    name,
                    setting.setting,
                    setting.value,
                    setting.default
                );
            }
            Ok((Bound::Pipe(listener), pipe_name.to_string()))
        }
        Listen::Transport(spec) => {
            let with_origins = settings_with_origins(spec, origins, name, table);
            let listening = transport::Listening::start(
                &spec.id,
                spec.origin.as_ref(),
                with_origins.as_ref().or(spec.settings.as_ref()),
                &spec.endpoint,
            )
            .map_err(|what| ServeError::Transport {
                entry: name.to_owned(),
                what,
            })?;
            if let Some(origin) = &spec.origin {
                say_transport_origin(name, table, origin);
            }
            // The provider says what of its settings is not its default: a weakening, such as plain WebSocket beyond
            // loopback, is never silent.
            for (setting, value, default) in listening.non_defaults() {
                tracing::info!(
                    "non-default: host.{table}.{name}.transport has {setting} at {value}, default {default}"
                );
            }
            let address = if listening.address().is_empty() {
                spec.endpoint.clone()
            } else {
                listening.address().to_owned()
            };
            Ok((Bound::Transport(alloc::boxed::Box::new(listening)), address))
        }
    }
}

impl Serving {
    /// Binds every entry of `config` on its loopback address, proving peers with `identity`, by its method, against `store`.
    /// Nothing is accepted until [`Serving::run`].
    ///
    /// # Errors
    ///
    /// [`ServeError::NothingToServe`] for a configuration with no entry, unless the options allow it;
    /// [`ServeError::Listen`] for an address that cannot be bound.
    pub fn bind(
        config: &Config,
        identity: Identity,
        store: TrustStore,
        options: &ServeOptions,
    ) -> Result<Self, ServeError> {
        let adapted;

        let config = match &options.portal {
            Some(sockets) => {
                adapted = crate::portal_link::adapt(config, sockets)?;
                &adapted
            }
            None => config,
        };
        let mut sources = if options.local_sources {
            sources::entries(config)
        } else {
            Vec::new()
        };
        // A host for a portal has no process of the person's to open them in: its sources listen on the sockets only that person's
        // browser reaches, as its other entries do.
        if let Some(sockets) = portal_of(options) {
            for source in &mut sources {
                source.entry.local = false;
                source.entry.own_profile = true;
                source.entry.listen = Listen::Pipe(sockets.entry(&source.entry.name)?);
            }
        }
        let controlled = config
            .control
            .as_ref()
            .is_some_and(|control| control.listen.is_some());
        if config.entries.is_empty()
            && sources.is_empty()
            && !controlled
            && !options.nothing_to_serve
        {
            return Err(ServeError::NothingToServe);
        }
        let store = Arc::new(store);
        // What proves this host to an agent on another machine it hands a task to.
        let dialer = identity.clone();
        let pairing_identity = Arc::new(identity.clone());
        let secrets = secrets::Hub::new(
            &identity,
            &store,
            delegate::BridgeCommand::new(&options.provider_program, &options.program_args),
        );
        // Turning the key change off is every method's to do: WebCrypto has none and takes only `false`, and ends a session at
        // its limit.
        let mut builder =
            Proof::builder(identity).initiators(Trust::Store(TrustStore::clone(&store)));
        if let Some(bytes) = options.key_limit {
            builder = builder.key_limit(bytes);
        }
        if !options.rekey {
            builder = builder.rekey(false);
        }
        let method = builder
            .build()
            .map_err(|error| ServeError::Proof(error.to_string()))?;
        // Only ServeOptions changes a method's settings here, so each one is configuration, not a surprise.
        for setting in method.non_defaults() {
            tracing::info!(
                "non-default: {} is {}, default {}",
                setting.setting,
                setting.value,
                setting.default
            );
        }
        let accounts = if config.accounts.is_empty() {
            None
        } else {
            let Some(home) = &options.state_dir else {
                return Err(ServeError::Accounts {
                    what: "accounts need the host's directory to keep their state in".to_owned(),
                });
            };
            Some(Accounts::open(config.account_store, home).map_err(|error| {
                ServeError::Accounts {
                    what: error.to_string(),
                }
            })?)
        };
        // The spend the agents report, kept in the home; a host with no home keeps none.
        let usage_log = options.state_dir.as_ref().map(|home| {
            Arc::new(crate::usage_log::UsageLog::open(
                crate::usage_log::dir_of(home),
                config.usage.keep_days,
                crate::usage_splice::unix_now(),
            ))
        });
        let agents = Arc::new(Mutex::new(Agents::default()));
        let mut registry = Registry::new();
        let mut listeners = Vec::new();
        let mut listening = Vec::new();
        let mut local_only = Vec::new();
        let mut movers: BTreeMap<String, Arc<kind::Harness>> = BTreeMap::new();
        let missing_of: BTreeMap<&str, &Vec<String>> = sources
            .iter()
            .map(|source| (source.entry.name.as_str(), &source.missing))
            .collect();
        for entry in config
            .entries
            .iter()
            .chain(sources.iter().map(|source| &source.entry))
        {
            // Every message after the proof is sealed in its session, so where the socket sits is the person's to choose.
            let (listener, url) = if entry.local {
                (None, format!("local://{}", entry.name))
            } else {
                let (bound, url) = bind_listen(
                    &entry.name,
                    table_of(&entry.kind),
                    &entry.listen,
                    &config.origins,
                    portal_of(options),
                )?;
                (Some(bound), url)
            };
            // The host's own sources are launched with what canoa.cfg's top level says, and so is an entry that sets `own_profile`;
            // any other entry on an address, with its `[host.profiles]`.
            let local_profile = (entry.local || entry.own_profile)
                .then(|| config.local_profiles.for_agent(&entry.name))
                .flatten();
            let (kind, run) = match &entry.kind {
                EntryKind::Agent {
                    program,
                    args,
                    profiles,
                    usage,
                } => (
                    "agent",
                    Run::Agent {
                        agent: Agent::Program {
                            program: program.clone(),
                            args: args.clone(),
                        },
                        scope: {
                            let harness = std::path::Path::new(program)
                                .file_stem()
                                .map_or_else(String::new, |stem| {
                                    stem.to_string_lossy().into_owned()
                                });
                            let scope = Scope::new(entry.working_roots.clone())
                                .account(entry.name.clone())
                                .limit_phrases(config.limit_phrases.clone())
                                .usage_log(usage_log.clone(), &harness);
                            match usage {
                                Some(UsageSource::Antigravity) => scope.antigravity(Antigravity {
                                    program: program.clone(),
                                    args: args.clone(),
                                    state_dir: options.state_dir.clone(),
                                    private: profiles.is_empty() && local_profile.is_none(),
                                    per_connection: !profiles.is_empty() || local_profile.is_some(),
                                    starter: Some(private_starter(&agents)),
                                }),
                                None => scope,
                            }
                        },
                        offer: hosted::offer(
                            profiles,
                            &config.profiles,
                            local_profile.as_ref(),
                            options.provider_program.as_os_str().to_owned(),
                            options.program_args.clone(),
                        ),
                        refresh: refresh_of(entry, profiles, config, options).map(Box::new),
                        worktrees: Worktrees::new(entry.working_roots.clone(), entry.worktrees),
                    },
                ),
                EntryKind::Provider {
                    origin,
                    provider,
                    settings,
                    directory,
                    max_sessions,
                } => {
                    let mut line = HostLine::default();
                    line.settings.clone_from(settings);
                    say_origin(entry, origin);
                    if let Origin::Source { settings, .. } = origin {
                        line.source_settings = Some(settings.clone());
                    }
                    (
                        "provider",
                        Run::Provider {
                            program: options.provider_program.as_os_str().to_owned(),
                            args: provider_args(
                                &options.program_args,
                                entry,
                                origin,
                                provider,
                                directory,
                                *max_sessions,
                            ),
                            line,
                        },
                    )
                }
                EntryKind::Harness {
                    harness,
                    origin,
                    executable,
                    options: asks,
                    settings,
                    account_settings,
                    profiles,
                    accounts: offered,
                } => {
                    if let Some(origin) = origin {
                        say_origin(entry, origin);
                    }
                    let configured = harness_settings(
                        entry,
                        harness,
                        executable.as_ref(),
                        asks,
                        settings.as_ref(),
                    );
                    let refused = |what: String| ServeError::Harness {
                        entry: entry.name.clone(),
                        what,
                    };
                    // Each entry has a registry of its own: an id resolves from that entry's file, never another's.
                    let registry = Arc::new(
                        kind::registry(origin.as_ref())
                            .map_err(|error| refused(error.to_string()))?,
                    );
                    let roots: Vec<String> = entry
                        .working_roots
                        .iter()
                        .map(|root| root.to_string_lossy().into_owned())
                        .collect();
                    let build = |settings: &Settings| {
                        kind::Harness::new(
                            &registry,
                            harness,
                            (settings, &account_settings.clone().unwrap_or_default()),
                            !profiles.is_empty() || local_profile.is_some(),
                            roots.clone(),
                        )
                        .map(|built| {
                            built
                                .with_lifetime(asks.cache_lifetime)
                                .with_entry(&entry.name)
                        })
                        .map_err(|error| refused(error.to_string()))
                    };
                    let mut provider = build(&configured)?;
                    let listed = match &accounts {
                        Some(_) if !offered.is_empty() => Some(
                            offered_accounts(config, harness, offered, &provider)
                                .map_err(refused)?,
                        ),
                        _ => None,
                    };
                    // A cloned repository's own settings can set the endpoint a key is sent to, so an entry that offers an
                    // account whose credential is in the environment loads no settings files unless it says which.
                    if let Some(listed) = &listed
                        && harness == "claude"
                        && keeps_key_in_environment(listed, harness)
                    {
                        let named = asks.setting_sources.is_some()
                            || settings
                                .as_ref()
                                .is_some_and(|own| own.as_map().contains_key("setting_sources"));
                        if named {
                            tracing::warn!(
                                "host.agents.{}.setting_sources is set and the entry offers an account whose key is in the environment: a repository's settings can change the address the key is sent to",
                                entry.name
                            );
                        } else {
                            let mut map = configured.as_map().clone();
                            map.insert("setting_sources".to_owned(), serde_json::json!([]));
                            provider = build(&Settings::new(map))?;
                        }
                    }
                    for (setting, value, default) in provider.non_defaults() {
                        tracing::info!(
                            "non-default: host.agents.{}: {setting} is {value}, default {default}",
                            entry.name
                        );
                    }
                    if let Some(host) = &accounts {
                        let listed = listed.unwrap_or_default();
                        let others = other_accounts(config, harness, &listed, &provider);
                        if !listed.is_empty() || !others.is_empty() {
                            let unfit = unfit_accounts(config, harness, &provider);
                            provider = provider.offering(host, listed, others).with_unfit(unfit);
                        }
                    }
                    // The shell the entry's own `shell`, else `[host] shell`, names: told to the harness as its documentation allows.
                    // An entry that names one that is not here does not start; the host's own that is not here is only said.
                    if let Some(setting) = entry.shell.as_deref().or(config.shell.as_deref()) {
                        match crate::shells::agent_applied(
                            setting,
                            harness,
                            &crate::shells::Lookup::here(),
                        ) {
                            Ok((shell, applied)) => {
                                if let Some(words) = &applied.notice {
                                    tracing::info!(
                                        "host.agents.{}: shell {} is not passed: {words}",
                                        entry.name,
                                        shell.id
                                    );
                                }
                                provider = provider.with_shell_env(applied.env);
                            }
                            Err(why) if entry.shell.is_some() => return Err(refused(why)),
                            Err(why) => {
                                tracing::warn!("host.agents.{}: [host] shell: {why}", entry.name)
                            }
                        }
                    }
                    let provider = Arc::new(provider);
                    movers.insert(entry.name.clone(), Arc::clone(&provider));
                    (
                        "agent",
                        Run::Agent {
                            agent: Agent::Harness(provider),
                            scope: Scope::new(entry.working_roots.clone())
                                .account(entry.name.clone())
                                .limit_phrases(config.limit_phrases.clone())
                                .usage_log(usage_log.clone(), harness),
                            offer: hosted::offer(
                                profiles,
                                &config.profiles,
                                local_profile.as_ref(),
                                options.provider_program.as_os_str().to_owned(),
                                options.program_args.clone(),
                            ),
                            refresh: refresh_of(entry, profiles, config, options).map(Box::new),
                            worktrees: Worktrees::new(entry.working_roots.clone(), entry.worktrees),
                        },
                    )
                }
                // A stdio agent like any other, so the scope applies to its clients and its processes end with the link.
                EntryKind::Tool { spec } => (
                    "tool",
                    Run::Agent {
                        agent: Agent::Program {
                            program: options.provider_program.as_os_str().to_owned(),
                            args: tool_args(&options.program_args, &entry.name, spec),
                        },
                        scope: Scope::new(entry.working_roots.clone())
                            .account(entry.name.clone())
                            .limit_phrases(config.limit_phrases.clone()),
                        offer: None,
                        refresh: None,
                        worktrees: Worktrees::new(entry.working_roots.clone(), entry.worktrees),
                    },
                ),
            };
            let id = ServiceId::new(entry.name.clone());
            // The host started this entry from its own configuration, which is what verifies it.
            registry.register(
                Service::new(id.clone(), entry.name.clone(), "acp", url.clone()).verified(),
            );
            for peer in entry.peers.iter().flatten() {
                registry.grant(
                    ClientId::new(peer.to_string()),
                    id.clone(),
                    Capability::new(ACP_CAPABILITY),
                );
            }
            let missing = if entry.local {
                missing_of
                    .get(entry.name.as_str())
                    .map(|missing| (*missing).clone())
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            listening.push(Listening {
                name: entry.name.clone(),
                kind,
                url,
                harness: match &entry.kind {
                    EntryKind::Harness { harness, .. } => Some(harness.clone()),
                    _ => None,
                },
                transport: match &entry.listen {
                    Listen::Transport(spec) => Some(spec.id.clone()),
                    _ => None,
                },
                title: entry.title.clone(),
                local: entry.local,
                missing,
            });
            let served = Arc::new(Served {
                name: entry.name.clone(),
                restricted: entry.peers.is_some(),
                delegable: kind == "agent",
                run,
                running: Gate::new(entry.max_peers),
                keep: entry.shared.then_some(sessions::Keep {
                    idle: entry.shared_idle,
                    permission_window: entry.shared_permission_window,
                }),
            });
            match listener {
                Some(listener) => listeners.push((served, listener)),
                None => local_only.push(served),
            }
        }
        if let Some(settings) = &config.control
            && let Some(listen) = &settings.listen
        {
            let (bound, url) = bind_listen(
                control::NAME,
                "control",
                listen,
                &config.origins,
                portal_of(options),
            )?;
            let id = ServiceId::new(control::NAME.to_owned());
            registry.register(
                Service::new(id.clone(), control::NAME.to_owned(), "control", url.clone())
                    .verified(),
            );
            for peer in settings.grants.keys() {
                registry.grant(
                    ClientId::new(peer.to_string()),
                    id.clone(),
                    Capability::new(ACP_CAPABILITY),
                );
            }
            listening.push(Listening {
                name: control::NAME.to_owned(),
                kind: "control",
                url,
                harness: None,
                transport: None,
                title: None,
                local: false,
                missing: Vec::new(),
            });
            listeners.push((
                Arc::new(Served {
                    name: control::NAME.to_owned(),
                    restricted: portal_of(options).is_none(),
                    delegable: false,
                    run: Run::Control {
                        grants: settings.grants.clone(),
                        everyone: portal_of(options).is_some().then(|| {
                            // A person's browser asking a shared machine to search its link tells it about that machine's
                            // network, not about the person's own.
                            control::Grant::all()
                                .into_iter()
                                .filter(|grant| *grant != control::Grant::Discover)
                                .collect()
                        }),
                    },
                    running: Gate::new(crate::config::DEFAULT_MAX_PEERS),
                    keep: None,
                }),
                bound,
            ));
        }
        let delegation = Delegation::start(
            &config.delegate,
            delegate::BridgeCommand::new(&options.provider_program, &options.program_args),
        );
        let recorded = Arc::new(history::Store::open(
            options.state_dir.as_ref().map(|dir| dir.join("history")),
            config.history,
        ));
        let stopped = Arc::new(AtomicBool::new(false));
        let trust = Arc::clone(&store);
        let hosted = Arc::new(sessions::Hosted::new(method.peer_id()));
        let serving = Self {
            shared: Arc::new(Shared {
                history: Arc::clone(&recorded),
                secrets: Arc::clone(&secrets),
                store_tools: store_mcp::Hub::new(
                    config.store.tools.unwrap_or(true),
                    delegate::BridgeCommand::new(&options.provider_program, &options.program_args),
                ),
                kb_tools: kb_mcp::Hub::new(
                    &config.kb,
                    options.state_dir.clone(),
                    delegate::BridgeCommand::new(&options.provider_program, &options.program_args),
                ),
                client_tools: client_tools::Hub::new(delegate::BridgeCommand::new(
                    &options.provider_program,
                    &options.program_args,
                )),
                hosted: Arc::clone(&hosted),
                control_grants: config
                    .control
                    .as_ref()
                    .map(|settings| settings.grants.clone())
                    .unwrap_or_default(),
                control_everyone: portal_of(options).is_some().then(|| {
                    control::Grant::all()
                        .into_iter()
                        .filter(|grant| *grant != control::Grant::Discover)
                        .collect()
                }),
                method,
                store,
                registry,
                unproven: Gate::new(MAX_UNPROVEN),
                agents,
                ending: Mutex::new(()),
                stopped: Arc::clone(&stopped),
                program_args: options.program_args.clone(),
                portal_uid: options.portal.as_ref().map(|sockets| sockets.uid),
                control: Arc::new({
                    let kept = control::Control::open(
                        options
                            .state_dir
                            .as_ref()
                            .map(|dir| dir.join("control"))
                            .as_deref(),
                        config
                            .control
                            .as_ref()
                            .map_or(control::DEFAULT_MAX_RECORDS, |c| c.max_records),
                        config
                            .control
                            .as_ref()
                            .map_or(control::DEFAULT_MAX_RECORD_BYTES, |c| c.max_record_bytes),
                        config
                            .entries
                            .iter()
                            .filter(|entry| entry.worktrees && !entry.working_roots.is_empty())
                            .map(|entry| control::EntryRoots {
                                peers: entry
                                    .peers
                                    .as_ref()
                                    .map(|peers| peers.iter().map(ToString::to_string).collect()),
                                roots: entry.working_roots.clone(),
                            })
                            .collect(),
                    )
                    .map_err(ServeError::Control)?;
                    let kept = if options.supervised {
                        kept.with_stop(Arc::clone(&stopped))
                    } else {
                        kept
                    };
                    let kept = kept
                        .with_index(crate::store::index::StoreIndex::new(
                            options
                                .state_dir
                                .as_ref()
                                .map(|dir| dir.join("cache").join("store")),
                            config.store.index_ttl.map_or(
                                crate::store::index::DEFAULT_TTL,
                                core::time::Duration::from_secs,
                            ),
                            config
                                .store
                                .index_cache_bytes
                                .unwrap_or(crate::store::index::DEFAULT_CACHE_BYTES),
                        ))
                        .with_registry(control::Registry::new(
                            options
                                .state_dir
                                .as_ref()
                                .map(|dir| dir.join("cache").join("contents")),
                        ))
                        .with_overlays(control::Overlays::new(
                            options
                                .config_file
                                .as_ref()
                                .and_then(|file| file.parent())
                                .map(|dir| dir.join("overlays")),
                            control::OverlayLimits::default(),
                        ))
                        .with_extensions(control::ExtensionInstall::new(
                            options.state_dir.as_ref().map(|dir| dir.join("extensions")),
                        ))
                        .with_terminals(crate::terminal::Terminals::new(
                            config.shell.clone(),
                            options.home_dir.clone().or_else(home_from_environment),
                        ))
                        .with_shells(crate::shells::Plan::new(
                            config.shell.clone(),
                            config
                                .entries
                                .iter()
                                .filter_map(|entry| match &entry.kind {
                                    EntryKind::Harness { harness, .. } => {
                                        Some(crate::shells::AgentShell {
                                            entry: entry.name.clone(),
                                            harness: harness.clone(),
                                            own: entry.shell.clone(),
                                        })
                                    }
                                    _ => None,
                                })
                                .collect(),
                        ))
                        .with_files(control::Files::new(
                            options.home_dir.clone().or_else(home_from_environment),
                            config
                                .entries
                                .iter()
                                .filter(|entry| !entry.working_roots.is_empty())
                                .map(|entry| control::EntryRoots {
                                    peers: entry.peers.as_ref().map(|peers| {
                                        peers.iter().map(ToString::to_string).collect()
                                    }),
                                    roots: entry.working_roots.clone(),
                                })
                                .collect(),
                            options.config_file.clone(),
                            options
                                .state_dir
                                .as_ref()
                                .map(|dir| dir.join("control").join("uploads")),
                            control::UploadLimits {
                                file_bytes: config
                                    .control
                                    .as_ref()
                                    .map_or(control::DEFAULT_MAX_UPLOAD_BYTES, |c| {
                                        c.max_upload_bytes
                                    }),
                                total_bytes: config
                                    .control
                                    .as_ref()
                                    .map_or(control::DEFAULT_MAX_UPLOAD_TOTAL, |c| {
                                        c.max_upload_total_bytes
                                    }),
                                files: config
                                    .control
                                    .as_ref()
                                    .map_or(control::DEFAULT_MAX_UPLOADS, |c| c.max_uploads),
                            },
                        ));
                    let kept = kept.with_trust(Arc::clone(&trust));
                    let kept = kept.with_sessions(control::RunningSessions(Arc::new({
                        let hosted = Arc::clone(&hosted);
                        move |who| hosted.running(who)
                    })));
                    let kept = match &options.state_dir {
                        Some(home) => kept.with_pairing(control::hosting::Pairing {
                            identity: Arc::clone(&pairing_identity),
                            home: home.clone(),
                        }),
                        None => kept,
                    };
                    let kept = match &usage_log {
                        Some(log) => kept.with_usage(Arc::clone(log)),
                        None => kept,
                    };
                    let kept = kept
                        .with_sealer(control::Sealer::new(Arc::clone(&secrets), options.ask_wait));
                    let kept = kept.with_history(control::Moves {
                        store: recorded,
                        targets: movers,
                        homes: control::Homes::of_this_computer(),
                    });
                    let kept = match (&options.account_layers, &options.state_dir) {
                        (Some(layers), Some(home)) => {
                            kept.with_accounts(control::accounts::Accounts::new(
                                layers.clone(),
                                home.clone(),
                                secrets,
                                options.ask_wait,
                                portal_of(options).is_some(),
                            ))
                        }
                        _ => kept,
                    };
                    match &options.programs {
                        Some((programs, providers)) => kept.with_stores(
                            control::Stores::new(Arc::clone(programs), providers.clone())
                                .extended(&config.extensions),
                        ),
                        None if !config.extensions.is_empty() => {
                            match control::Stores::built_in() {
                                Ok(stores) => kept.with_stores(stores.extended(&config.extensions)),
                                Err(_) => kept,
                            }
                        }
                        None => kept,
                    }
                }),
                delegation,
            }),
            listeners,
            local_only,
            listening,
            door: None,
        };
        let mut serving = serving;
        serving
            .shared
            .store_tools
            .serve_with(Arc::new(control::ToolsBackend(Arc::clone(
                &serving.shared.control,
            ))));
        serving.door = serving.open_door(config, options);
        serving.attach_delegation(config, dialer, usage_log.clone());

        if let Some(sockets) = &options.portal {
            let peer = serving.shared.method.peer_id().to_string();
            let method_name = serving.method().name();
            let index = crate::portal_link::index_of(&peer, method_name, &serving.listening);
            crate::portal_link::introduce(
                sockets,
                Arc::clone(&serving.shared.store),
                index,
                Arc::clone(&serving.shared.stopped),
            )?;
        }
        Ok(serving)
    }

    /// Binds the native door in the home, when the configuration and the options leave one. A door that cannot be bound is
    /// logged and left out: the host serves its entries without it.
    fn open_door(&self, config: &Config, options: &ServeOptions) -> Option<Door> {
        let home = options.state_dir.as_ref()?;
        if !config.native.enabled || options.portal.is_some() {
            return None;
        }
        let name = match crate::native::door_name(home) {
            Ok(name) => name,
            Err(error) => {
                tracing::warn!(
                    "the native door has no name, so a browser cannot reach this host: {error}"
                );
                crate::native::note_door(
                    home,
                    Some(&format!("the door for browsers has no name: {error}")),
                );
                return None;
            }
        };
        let listener = match pipe::Listener::bind(&name) {
            Ok(listener) => listener,
            Err(error) => {
                tracing::warn!(
                    "the native door could not be opened, so a browser cannot reach this host: {error}"
                );
                crate::native::note_door(
                    home,
                    Some(&format!(
                        "the door for browsers could not be opened: {error}"
                    )),
                );
                return None;
            }
        };
        crate::native::note_door(home, None);
        let entries: Vec<serde_json::Value> = self
            .listening
            .iter()
            .filter(|listening| listening.kind != "control")
            .map(|listening| {
                serde_json::json!({
                    "name": listening.name,
                    "label": listening.title.clone().unwrap_or_else(|| listening.name.clone()),
                    "kind": listening.kind,
                    "local": listening.local,
                })
            })
            .collect();
        let mut hello = serde_json::json!({
            "name": "Canoa",
            "version": env!("CARGO_PKG_VERSION"),
            "peer": self.peer().to_string(),
            "method": self.method().name(),
            "entries": entries,
            "control": true,
        });
        if let (Some(claim), Some(names)) = (
            hello.as_object_mut(),
            crate::native::this_computer().as_object(),
        ) {
            claim.extend(
                names
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone())),
            );
        }
        let control = Arc::new(Served {
            name: control::NAME.to_owned(),
            restricted: false,
            delegable: false,
            run: Run::Control {
                grants: IndexMap::new(),
                everyone: Some(config.native.control.clone()),
            },
            running: Gate::new(crate::config::DEFAULT_MAX_PEERS),
            keep: None,
        });
        let tools = Arc::new(Served {
            name: client_tools::NAME.to_owned(),
            restricted: false,
            delegable: false,
            run: Run::Tools {
                grants: config.native.control.clone(),
            },
            running: Gate::new(crate::config::DEFAULT_MAX_PEERS),
            keep: None,
        });
        Some(Door::new(
            listener,
            hello,
            self.local().served,
            (control, tools),
            home.clone(),
        ))
    }

    /// Gives the delegation the agents this host serves and the way to open one for a task.
    fn attach_delegation(
        &self,
        config: &Config,
        identity: Identity,
        usage: Option<Arc<crate::usage_log::UsageLog>>,
    ) {
        let Some(delegation) = &self.shared.delegation else {
            return;
        };
        let served = self.local().served;
        let mut agents: Vec<delegate::Source> = self
            .listening
            .iter()
            .filter(|listening| listening.kind == "agent")
            .map(|listening| delegate::Source {
                name: listening.name.clone(),
                title: listening.title.clone(),
                harness: listening.harness.clone(),
                roots: served
                    .iter()
                    .find(|served| served.name == listening.name)
                    .and_then(|served| match &served.run {
                        Run::Agent { scope, .. } => Some(scope.roots().to_vec()),
                        _ => None,
                    })
                    .unwrap_or_default(),
                accounts: match config.entry(&listening.name).map(|entry| &entry.kind) {
                    Some(EntryKind::Harness { accounts, .. }) => accounts.clone(),
                    _ => Vec::new(),
                },
                profiles: match config.entry(&listening.name).map(|entry| &entry.kind) {
                    Some(
                        EntryKind::Agent { profiles, .. } | EntryKind::Harness { profiles, .. },
                    ) => profiles
                        .iter()
                        .map(|name| {
                            let loadouts = config
                                .profiles
                                .get(name)
                                .map(|profile| profile.loadouts.keys().cloned().collect())
                                .unwrap_or_default();
                            (name.clone(), loadouts)
                        })
                        .collect(),
                    _ => Vec::new(),
                },
                missing: listening.missing.clone(),
                remote: None,
            })
            .collect();
        agents.extend(config.remote_agents.iter().map(|agent| delegate::Source {
            name: agent.name.clone(),
            title: agent.title.clone(),
            harness: None,
            roots: Vec::new(),
            accounts: Vec::new(),
            profiles: Vec::new(),
            missing: Vec::new(),
            remote: Some(delegate::Remote {
                url: agent.url.clone(),
                peer: agent.peer,
                source: agent.source.clone(),
            }),
        }));
        delegation.attach(Arc::new(DelegationHost {
            shared: Arc::downgrade(&self.shared),
            served,
            agents,
            identity,
            usage,
        }));
    }

    /// The method every entry proves by: its identity's.
    #[must_use]
    pub fn method(&self) -> Method {
        self.shared.method.method()
    }

    /// This host's peer id, which a client pins: its identity's.
    #[must_use]
    pub fn peer(&self) -> PeerId {
        self.shared.method.peer_id()
    }

    /// Where each entry listens, in the configuration's order.
    #[must_use]
    pub fn listening(&self) -> &[Listening] {
        &self.listening
    }

    /// A way into every entry from inside this process, which outlives [`Serving::run`] taking `self`.
    #[must_use]
    pub fn local(&self) -> Local {
        Local {
            shared: Arc::clone(&self.shared),
            served: self
                .listeners
                .iter()
                .map(|(served, _)| Arc::clone(served))
                .chain(self.local_only.iter().map(Arc::clone))
                .collect(),
        }
    }

    /// What stops this and ends its agents, for a thread other than the one in [`Serving::run`].
    #[must_use]
    pub fn stopper(&self) -> Stopper {
        Stopper(Arc::clone(&self.shared))
    }

    /// Raised when a key asked the host to end so that it starts again (`host/restart`).
    #[must_use]
    pub fn restart_flag(&self) -> Arc<AtomicBool> {
        self.shared.control.restart_flag()
    }

    /// Takes connections on every entry until [`Stopper::stop`] or the process ends. Each handshake and proof runs on a
    /// thread of its own, so a slow peer holds up nobody. It returns with every listener closed and every agent ended.
    pub fn run(self) {
        let stopper = self.stopper();
        let nothing = self.listeners.is_empty();
        let door = self.door.map(|door| {
            let shared = Arc::clone(&self.shared);
            thread::Builder::new()
                .name("canoa-door".to_owned())
                .spawn(move || door.run(&shared))
        });
        let threads: Vec<_> = self
            .listeners
            .into_iter()
            .map(|(served, listener)| {
                let shared = Arc::clone(&self.shared);
                thread::Builder::new()
                    .name(format!("canoa-{}", served.name))
                    .spawn(move || match &listener {
                        Bound::Socket(listener) => accept_loop(&shared, &served, listener),
                        Bound::Pipe(listener) => accept_pipe_loop(&shared, &served, listener),
                        Bound::Transport(listening) => {
                            accept_transport_loop(&shared, &served, listening);
                        }
                    })
            })
            .collect();
        for thread in threads.into_iter().flatten() {
            let _ = thread.join();
        }

        // With no entry there is no listener to wait on, and it still runs until stopped.
        while nothing && !self.shared.stopped.load(Ordering::SeqCst) {
            thread::sleep(STOP_POLL);
        }
        if let Some(Ok(door)) = door {
            let _ = door.join();
        }
        // The stop that ended the listeners may still be ending agents on its own thread.
        stopper.stop_agents();
    }
}

fn url_of(bound: SocketAddr) -> String {
    format!("ws://{bound}")
}

fn unproven_limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_message = auth::MAX_PROOF_MESSAGE;
    limits.pending = UNPROVEN_PENDING;
    limits
}

/// A plain WebSocket listener of binary frames on `address`, for peers that prove themselves in-band.
///
/// Where the socket sits is the person's to choose, so a non-loopback address binds; the caller reports it through
/// [`websocket::Listener::plain_beyond_loopback`].
pub(crate) fn plain_listener(
    address: SocketAddr,
    limits: Limits,
    origins: Vec<String>,
) -> Result<websocket::Listener, Failure> {
    websocket::Listener::builder(address, websocket::ServerTls::Plain)
        .limits(limits)
        .frames(Frames::Binary)
        .origins(origins)
        .plain_beyond_loopback(true)
        .build()
        .map_err(|error| match error {
            BuildError::Failed(failure) => failure,
            // The address and the TLS choice are both given above; no other kind of error exists.
            _ => Failure::Unsupported,
        })
}

fn accept_loop(shared: &Arc<Shared>, served: &Arc<Served>, listener: &websocket::Listener) {
    while !shared.stopped.load(Ordering::SeqCst) {
        let pending = match listener.next(Some(STOP_POLL)) {
            Ok(pending) => pending,
            Err(Failure::Timeout) => continue,
            Err(failure) => {
                tracing::warn!(entry = served.name, "a connection failed: {failure}");
                continue;
            }
        };
        let Some(entered) = shared.unproven.enter() else {
            tracing::warn!(
                entry = served.name,
                "a connection was dropped: too many are unproven"
            );
            continue;
        };
        let shared = Arc::clone(shared);
        let served = Arc::clone(served);
        let spawned = thread::Builder::new()
            .name("canoa-peer".to_owned())
            .spawn(move || match pending.handshake() {
                Ok(channel) => {
                    connection(&shared, &served, &channel, entered, None);
                    channel.close();
                }
                Err(failure) => tracing::debug!("a handshake failed: {failure}"),
            });
        if let Err(error) = spawned {
            tracing::warn!("a connection was dropped: {error}");
        }
    }
}

/// [`accept_loop`] for a transport provider: what it hands over is already a connection, so the proof runs on it at once.
fn accept_transport_loop(
    shared: &Arc<Shared>,
    served: &Arc<Served>,
    listening: &transport::Listening,
) {
    while !shared.stopped.load(Ordering::SeqCst) {
        let (from, link) = match listening.next(STOP_POLL) {
            transport::Next::Nobody => continue,
            transport::Next::Ended(status, reason) => {
                tracing::warn!(
                    entry = served.name,
                    status,
                    "its transport stopped listening: {reason}"
                );
                return;
            }
            transport::Next::Peer(from, link) => (from, link),
        };
        let Some(entered) = shared.unproven.enter() else {
            tracing::warn!(
                entry = served.name,
                "a connection was dropped: too many are unproven"
            );
            continue;
        };
        // Information for the log: the key proof is what admits a peer.
        tracing::debug!(entry = served.name, from, "a connection on a transport");
        let shared = Arc::clone(shared);
        let served = Arc::clone(served);
        let spawned = thread::Builder::new()
            .name("canoa-peer".to_owned())
            .spawn(move || {
                connection(&shared, &served, &link, entered, None);
                link.close();
            });
        if let Err(error) = spawned {
            tracing::warn!("a connection was dropped: {error}");
        }
    }
    listening.close();
}

/// [`accept_loop`] for a pipe or Unix socket: there is no handshake to run before the proof.
fn accept_pipe_loop(shared: &Arc<Shared>, served: &Arc<Served>, listener: &pipe::Listener) {
    while !shared.stopped.load(Ordering::SeqCst) {
        let stream = match listener.accept_timeout(Some(STOP_POLL)) {
            Ok(stream) => stream,
            Err(pipe::Error::TimedOut) => continue,
            Err(error) => {
                tracing::warn!(entry = served.name, "a connection failed: {error}");
                thread::sleep(STOP_POLL);
                continue;
            }
        };
        let Some(entered) = shared.unproven.enter() else {
            tracing::warn!(
                entry = served.name,
                "a connection was dropped: too many are unproven"
            );
            continue;
        };
        if let Some(uid) = shared.portal_uid
            && !crate::portal_link::from_user(&stream, uid)
        {
            tracing::warn!(
                entry = served.name,
                "a connection from a user other than the portal's was refused"
            );
            continue;
        }
        // Information for the log: the key proof is what admits a peer.
        match stream.peer() {
            Ok(peer) => tracing::debug!(entry = served.name, ?peer, "a connection on a pipe"),
            Err(error) => tracing::debug!(entry = served.name, "no peer identity: {error}"),
        }
        let channel = pipe::PipeChannel::new(stream, auth::MAX_PROOF_MESSAGE);
        let shared = Arc::clone(shared);
        let served = Arc::clone(served);
        let spawned = thread::Builder::new()
            .name("canoa-peer".to_owned())
            .spawn(move || {
                connection(&shared, &served, &channel, entered, None);
                channel.close();
            });
        if let Err(error) = spawned {
            tracing::warn!("a connection was dropped: {error}");
        }
    }
}

impl Shared {
    /// Whether the entry names `peer` in its `peers`: an entry that lists no key is open to every paired key, which is not
    /// enough for a key that came to run tasks here.
    fn lists(&self, peer: &PeerId, served: &Served) -> bool {
        served.restricted && self.allows(peer, served)
    }

    /// Whether `peer`, whose key the store accepts, may use this entry.
    fn allows(&self, peer: &PeerId, served: &Served) -> bool {
        !served.restricted
            || self
                .registry
                .resolve(
                    &ClientId::new(peer.to_string()),
                    &ServiceId::new(served.name.clone()),
                    &Capability::new(ACP_CAPABILITY),
                )
                .is_some()
    }
}

/// A connection's proof and what follows it. `admit`, when given, is asked once the key is known and may turn it away: the native
/// door uses it to bind a browser's key to the extension that paired it.
fn connection<C: Channel>(
    shared: &Arc<Shared>,
    served: &Arc<Served>,
    channel: &C,
    unproven: Entered,
    admit: Option<&dyn Fn(&PeerId) -> bool>,
) {
    let established = match auth::prove(
        &shared.method,
        channel,
        PROOF_LIMIT,
        shared.method.method().record_length(MAX_LINE),
    ) {
        Ok(established) => established,
        // A client of the other method has no key here to accept: it has to pair again under this host's method.
        Err(error @ coqui_agentic_peer::Error::OtherMethod { .. }) => {
            return tracing::info!(entry = served.name, "a client was refused: {error}");
        }
        Err(error @ coqui_agentic_peer::Error::Proof(_)) => {
            return tracing::info!(
                entry = served.name,
                "a key was refused: {error}; a key that proved itself and is not paired waits for `canoa accept`"
            );
        }
        Err(error) => {
            return tracing::info!(
                entry = served.name,
                "a connection ended before its proof: {error}"
            );
        }
    };
    drop(unproven);
    let peer = established.peer;
    if !shared.allows(&peer, served) {
        return tracing::info!(entry = served.name, %peer, "a paired key this entry does not list was turned away");
    }
    if let Some(admit) = admit
        && !admit(&peer)
    {
        return tracing::info!(entry = served.name, %peer, "a browser's key was turned away: it was paired by another extension");
    }
    let store = Arc::clone(&shared.store);
    let stopped = Arc::clone(&shared.stopped);
    let checked = peer;
    let sealed = Sealed::new(channel, established.session)
        .with_check(move || !stopped.load(Ordering::SeqCst) && store.is_accepted(&checked));
    serve_proven(shared, served, &sealed, &peer, None, false);
    sealed.close();
}

/// Serves a connection whose peer is known, from its first message to its end; the caller closes `channel` after. `in_process` is
/// true only for the connections [`Local`] opens, whose peer is this process.
fn serve_proven<C: Channel>(
    shared: &Arc<Shared>,
    served: &Arc<Served>,
    sealed: &C,
    peer: &PeerId,
    context: Option<&delegate::ChildContext>,
    in_process: bool,
) {
    let Some(_running) = served.running.enter() else {
        tracing::info!(entry = served.name, %peer, "turned away: max_peers are running");
        return;
    };
    let mut first = sealed.receive(Some(FIRST_MESSAGE));
    // Answered before any agent starts, so a worktree can be removed with no agent of this connection working in it.
    if let Run::Agent { worktrees, .. } = &served.run {
        let mut answered = 0;
        while let Ok(Some(message)) = &first
            && Worktrees::is_request(message)
        {
            if answered == MAX_WORKTREE_REQUESTS {
                tracing::info!(entry = served.name, %peer, "turned away: too many worktree requests");
                return;
            }
            answered += 1;
            if let Some(reply) = worktrees.answer(message)
                && sealed.send(&reply).is_err()
            {
                return;
            }
            first = sealed.receive(Some(FIRST_MESSAGE));
        }
    }
    match first {
        // A link of streams carries A2A and nothing else, over an agent entry's address, and only the delegation decides who
        // may open it (`[host.delegate] a2a`).
        Ok(Some(first)) if mux::starts_mux(&first) => {
            match shared.delegation.as_ref().filter(|_| served.delegable) {
                Some(delegation) => {
                    tracing::info!(entry = served.name, %peer, "a link of streams");
                    delegation.serve_link(sealed, first, &peer.to_string(), MAX_LINE);
                }
                None => {
                    tracing::info!(entry = served.name, %peer, "a link of streams was refused: this entry serves plain ACP");
                }
            }
        }
        Ok(Some(first)) => {
            tracing::info!(entry = served.name, %peer, "proven");
            let copy = first.clone();
            give_agent(
                shared,
                served,
                &Prefilled::new(sealed, first),
                (&copy, peer),
                (context, in_process),
            );
        }
        Ok(None) | Err(_) => {}
    }
}

fn give_agent<C: Channel>(
    shared: &Arc<Shared>,
    served: &Arc<Served>,
    channel: &C,
    (first, peer): (&[u8], &PeerId),
    (context, in_process): (Option<&delegate::ChildContext>, bool),
) {
    // A task another agent handed out has no person at its end to attach to it: it is never kept.
    if let (Some(keep), Run::Agent { .. }, None) = (&served.keep, &served.run, context) {
        return sessions::serve(shared, served, keep, channel, (first, peer), in_process);
    }
    give_agent_flow(
        shared,
        served,
        channel,
        (first, peer),
        (context, in_process),
    );
}

/// Serves one connection with an agent of its own, until either ends.
fn give_agent_flow<C: Channel>(
    shared: &Shared,
    served: &Served,
    channel: &C,
    (first, peer): (&[u8], &PeerId),
    (context, in_process): (Option<&delegate::ChildContext>, bool),
) {
    let (agent, scope, offer, refresh) = match &served.run {
        Run::Agent {
            agent,
            scope,
            offer,
            refresh,
            ..
        } => (agent, scope, offer, refresh),
        Run::Provider {
            program,
            args,
            line,
        } => return give_provider(shared, served, channel, peer, (program, args), line),
        Run::Control { grants, everyone } => {
            let grants = if let Some(everyone) = everyone {
                everyone.clone()
            } else {
                grants
                    .iter()
                    .find(|(key, _)| *key == peer)
                    .map(|(_, grants)| grants.clone())
                    .unwrap_or_default()
            };
            let caller = control::Caller::Peer {
                id: peer.to_string(),
                grants,
            };
            return control::serve(&shared.control, &caller, channel);
        }
        Run::Tools { grants } => {
            // The grants are the door's: a connection that reached it from this process holds all of them.
            let grants = if in_process {
                control::Grant::all().into_iter().collect()
            } else {
                grants.clone()
            };
            return shared
                .client_tools
                .provide(&peer.to_string(), &grants, channel);
        }
    };
    // A task's connection records what its agent spends as the child of the session that started it.
    let child_scope;
    let scope = match context.and_then(|context| context.parent_session.clone()) {
        Some(parent) => {
            child_scope = scope.child_of(Some(parent));
            &child_scope
        }
        None => scope,
    };
    // What lets this connection's sessions hand tasks to other sources: for a task's own connection, what the host bounded it by.
    let conn = shared
        .delegation
        .as_ref()
        .filter(|_| served.delegable)
        .map(|delegation| {
            delegation.connection(match context {
                Some(context) => delegate::ConnInfo {
                    principal: context.principal.clone(),
                    entry: served.name.clone(),
                    roots: context.roots.clone(),
                    depth: context.depth,
                    child: true,
                },
                None => delegate::ConnInfo {
                    principal: if *peer == shared.method.peer_id() {
                        Principal::Owner
                    } else {
                        Principal::Key(*peer)
                    },
                    entry: served.name.clone(),
                    roots: scope.roots().to_vec(),
                    depth: 0,
                    child: false,
                },
            })
        });
    // The person's app, when it offered secret prompts and has an identity of its own to be bound to; the application in this
    // process is bound by the secret it shares with the host, and a task's connection has no person at its other end.
    let link = shared
        .secrets
        .for_connection(peer, in_process, context.is_some());
    // The store tools, for a connection whose key may read what they read.
    let store = shared
        .store_tools
        .for_connection(shared.store_grants(peer, context.is_some()).as_ref());
    let peeked = Peek::new(channel, conn.as_ref());
    let raw = Raw::new(&peeked, link.as_ref());
    let store_peek = StorePeek::new(&raw, store.as_ref());
    // The tools programs offer, for a connection whose key may use them and which is not a task another agent handed out.
    let tools = shared.client_tools.for_connection(
        shared.store_grants(peer, context.is_some()).as_ref(),
        client_tools::Who {
            agent: served.name.clone(),
            via: if in_process {
                client_tools::Via::App
            } else {
                client_tools::Via::Paired(peer.to_string())
            },
        },
    );
    let tools_peek = ToolsPeek::new(&store_peek, tools.as_ref());
    let channel = &tools_peek;
    let fresh = refresh
        .as_ref()
        .map_or_else(|| offer.clone(), |refresh| refresh.offer());
    let offer = fresh.as_ref();
    // A key trusted with accounts may run a session on any account of the entry's provider, not only those the entry lists.
    let any_account = shared
        .store_grants(peer, context.is_some())
        .is_some_and(|grants| grants.contains(&control::Grant::Accounts));
    let account = match &agent {
        Agent::Harness(harness) => harness.account_facts(first, any_account),
        Agent::Program { .. } => None,
    };
    let provider = match &agent {
        Agent::Harness(harness) => Some(harness.id()),
        Agent::Program { .. } => None,
    };
    let harness_id = hosted::harness_identity(provider, &served.name);
    let (account_kind, account_provider) = account
        .as_ref()
        .map_or((None, None), |(_, kind, provides)| {
            account_words(kind, provides, harness_id)
        });
    let version = hosted::harness_version(provider);
    let connection = match offer {
        Some(offer) => match prepare(
            served,
            channel,
            first,
            peer,
            offer,
            // Only this process's own link is asked to approve: a key reaches the configuration's behaviour.
            *peer == shared.method.peer_id(),
            Starting {
                who: Who {
                    harness: harness_id,
                    version: version.as_deref(),
                    account: account.as_ref().map(|(label, ..)| label.as_str()),
                    account_kind,
                    account_provider,
                    // A task another agent started runs as its delegate.
                    role: context.is_some().then_some("delegate"),
                    ..Who::default()
                },
                host: hosted::host_variables(provider),
            },
        ) {
            Some(connection) => Some(connection),
            None => return,
        },
        None => None,
    };
    // A session that was given the store tools is told when to use them, by a skill that goes with it.
    let connection = connection.map(|connection| {
        if store.is_some() && store_mcp::asked_in(first) {
            connection.with_skill(store_mcp::SKILL_NAME, store_mcp::skill_files())
        } else {
            connection
        }
    });
    // The overlays this connection applies, searchable by the agent when its client asked: it is then given the tools and a skill
    // that says when to use them, and an overlay set to be served as an index leaves its files out of the context.
    let kb = connection.as_ref().and_then(|connection| {
        shared.kb_tools.for_connection(
            connection.kb_sources(),
            &connection.kb_caller(),
            kb_mcp::asked_in(first),
        )
    });
    let connection = connection.map(|connection| {
        if kb.is_some() {
            connection
                .with_kb_tools()
                .with_skill(kb_mcp::SKILL_NAME, kb_mcp::skill_files())
        } else {
            connection
        }
    });
    // What the profile suggests about delegation reaches each session this connection opens.
    if let (Some(conn), Some(connection)) = (&conn, &connection) {
        conn.suggest_with(connection.delegate_hints());
    }
    let (program, args) = match agent {
        Agent::Program { program, args } => (program, args),
        Agent::Harness(options) => {
            return give_harness(
                shared,
                served,
                channel,
                (peer, first, any_account),
                options,
                scope,
                (
                    connection.as_ref(),
                    conn.as_ref(),
                    link.as_ref(),
                    store.as_ref(),
                    kb.as_ref(),
                    tools.as_ref(),
                ),
            );
        }
    };
    let (Some(offer), Some(connection)) = (offer, connection) else {
        // Nothing is logged once the host is stopping: its standard error may be a pipe nobody reads.
        let (key, agent) = match shared.start(program, args) {
            Ok(Some(started)) => started,
            Ok(None) => return,
            Err(error) => {
                return tracing::warn!(entry = served.name, "the agent could not start: {error}");
            }
        };
        let scoped = Scoped::new(channel, scope).agent(agent.pid());
        let end = relay(
            &ToolsLayer::new(
                &KbLayer::new(
                    &StoreLayer::new(
                        &Layer::new(&Delegating::new(&scoped, conn.as_ref()), link.as_ref()),
                        store.as_ref(),
                    ),
                    kb.as_ref(),
                ),
                tools.as_ref(),
            ),
            &agent,
        );
        tracing::info!(entry = served.name, %peer, "left: {end:?}");
        shared.forget(key);
        let _ = agent.stop();
        return;
    };
    // The agent's environment reaches it on `launch`'s first line, never on a command line.
    let mut launched = shared.program_args.clone();
    launched.extend([
        OsString::from("launch"),
        OsString::from("--"),
        program.clone(),
    ]);
    launched.extend(args.iter().cloned());
    let (key, agent) = match shared.start(&offer.launcher, &launched) {
        Ok(Some(started)) => started,
        Ok(None) => return,
        Err(error) => {
            return tracing::warn!(entry = served.name, "the agent could not start: {error}");
        }
    };
    let hosted = Hosted::new(channel, scope, &connection).agent(agent.pid());
    let end = relay(
        &ToolsLayer::new(
            &KbLayer::new(
                &StoreLayer::new(
                    &Layer::new(&Delegating::new(&hosted, conn.as_ref()), link.as_ref()),
                    store.as_ref(),
                ),
                kb.as_ref(),
            ),
            tools.as_ref(),
        ),
        &agent,
    );
    tracing::info!(entry = served.name, %peer, "left: {end:?}");
    shared.forget(key);
    let _ = agent.stop();
    // What was written for its sessions goes once the agent has.
    drop(hosted);
}

/// The profile the client chose in its first message, ready for its agent; `None` once it was refused, answered and logged.
fn prepare<C: Channel>(
    served: &Served,
    channel: &C,
    first: &[u8],
    peer: &PeerId,
    offer: &Offer,
    in_process: bool,
    starting: Starting<'_>,
) -> Option<Connection> {
    let refuse = |answer: Option<Vec<u8>>, log: &str| {
        if let Some(answer) = answer {
            let _ = channel.send(&answer);
        }
        tracing::info!(entry = served.name, %peer, "{log}");
    };
    let index = match hosted::choose(offer, first) {
        Ok(index) => index,
        Err(refusal) => {
            refuse(refusal.answer, refusal.log);
            return None;
        }
    };
    let only = match hosted::choose_overlays(first) {
        Ok(only) => only,
        Err(refusal) => {
            refuse(refusal.answer, refusal.log);
            return None;
        }
    };
    let loadout = match hosted::choose_loadout(offer, index, first) {
        Ok(loadout) => loadout,
        Err(refusal) => {
            refuse(refusal.answer, refusal.log);
            return None;
        }
    };
    let preset = match hosted::choose_preset(offer, index, first) {
        Ok(preset) => preset,
        Err(refusal) => {
            refuse(refusal.answer, refusal.log);
            return None;
        }
    };
    let lookup = |name: &str| std::env::var(name).ok();
    match Connection::prepare_with(
        offer,
        index,
        &served.name,
        &lookup,
        (only.as_deref(), loadout.as_deref(), preset.as_deref()),
        starting,
    ) {
        Ok(connection) => Some(connection.asking(in_process)),
        Err(why) => {
            let id = serde_json::from_slice::<serde_json::Value>(first)
                .ok()
                .and_then(|message| message.get("id").cloned());
            let answer = id.map(|id| {
                serde_json::to_vec(&serde_json::json!({"jsonrpc": "2.0", "id": id, "error": {
                    "code": -32603, "message": "the host's profile for this entry cannot be applied",
                }}))
                .unwrap_or_default()
            });
            refuse(
                answer,
                &format!("the profile could not be applied, so no agent started: {why}"),
            );
            None
        }
    }
}

/// The table of canoa.cfg an entry of this kind is in.
const fn table_of(kind: &EntryKind) -> &'static str {
    match kind {
        EntryKind::Agent { .. } | EntryKind::Harness { .. } => "agents",
        EntryKind::Provider { .. } => "providers",
        EntryKind::Tool { .. } => "tools",
    }
}

/// The settings of a built-in `websocket` transport with `[host] origins` as its `listen.origins`, when the host names origins and
/// the entry's settings list none: the browser-origin check an address listener has applies here too. `None` leaves the entry's
/// settings as they are.
fn settings_with_origins(
    spec: &crate::config::TransportListen,
    origins: &[String],
    name: &str,
    table: &str,
) -> Option<Settings> {
    if origins.is_empty() || spec.id != "websocket" || spec.origin.is_some() {
        return None;
    }
    let mut map = spec
        .settings
        .as_ref()
        .map(|settings| settings.as_map().clone())
        .unwrap_or_default();
    let serde_json::Value::Object(listen) = map
        .entry("listen".to_owned())
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()))
    else {
        return None;
    };
    let listed = match listen.get("origins") {
        None => false,
        Some(serde_json::Value::Array(listed)) => !listed.is_empty(),
        Some(_) => true,
    };
    if listed {
        return None;
    }
    listen.insert(
        "origins".to_owned(),
        serde_json::Value::from(origins.to_vec()),
    );
    tracing::info!(
        "non-default: host.{table}.{name}.transport has listen.origins from [host] origins: {}",
        origins.join(", ")
    );
    Some(Settings::new(map))
}

/// What loading `origin` for `entry` departs from the defaults in, said as configuration where the file asked for it, and as
/// a warning where the host fell back.
fn say_origin(entry: &Entry, origin: &Origin) {
    say_origin_of(&entry.name, table_of(&entry.kind), "", origin);
}

/// [`say_origin`] for the origin an entry's `transport` loads: `prefix` is what its keys start with (`transport_`), and the
/// outside code running in the host's own process is said as well.
fn say_transport_origin(name: &str, table: &str, origin: &Origin) {
    tracing::info!(
        "host.{table}.{name}.transport_{} loads an outside transport provider into the host's process",
        if matches!(origin, Origin::Library(_)) {
            "library"
        } else {
            "source"
        }
    );
    say_origin_of(name, table, "transport_", origin);
}

fn say_origin_of(name: &str, table: &str, prefix: &str, origin: &Origin) {
    let Origin::Source {
        settings,
        import_search,
        import_search_named,
        ..
    } = origin
    else {
        return;
    };
    if cfg!(windows) && *import_search == ImportSearch::Path {
        if *import_search_named {
            tracing::info!(
                "non-default: host.{table}.{name}.{prefix}import_search is path, default library: python3.dll                  and what else the source imports itself may come from the working directory or PATH"
            );
        } else {
            // The file does not say it: import_dirs names no directory, so the host fell back.
            tracing::warn!(
                "host.{table}.{name}.{prefix}import_search is path, because import_dirs names no directory:                  python3.dll and what else the source imports itself may come from the working                  directory or PATH; set import_dirs to where it is, or {prefix}import_search = \"library\""
            );
        }
    }
    // Guatiao's source cannot tell its host which non-default setting it took, so the host says it.
    if settings
        .as_map()
        .get("isolation")
        .and_then(|value| value.as_str())
        == Some("inherit")
    {
        tracing::info!(
            "non-default: host.{table}.{name}.{prefix}source_settings.isolation is inherit, default shipped"
        );
    }
}

/// The settings a harness entry's provider is configured with: the entry's `settings`, then the entry's own keys of the
/// same names, and for the built-in `claude`, `codex` and `pi` the host's own values where nothing is set, never the person's
/// configuration on this machine, because a paired key reaches it. What a client may choose stays within the entry's mode.
/// A setting that acts with nothing asked is logged as configuration.
pub(crate) fn harness_settings(
    entry: &Entry,
    harness: &str,
    executable: Option<&OsString>,
    asks: &HarnessOptions,
    own: Option<&Settings>,
) -> Settings {
    let mut map = own.map(|own| own.as_map().clone()).unwrap_or_default();
    if !BUILT_IN_HARNESSES.contains(&harness) {
        return Settings::new(map);
    }
    if let Some(executable) = executable {
        map.insert(
            "executable".to_owned(),
            serde_json::Value::String(executable.to_string_lossy().into_owned()),
        );
    } else if matches!(harness, "claude" | "codex")
        && !map.contains_key("executable")
        && let Some((path, _)) = sources::managed_program(harness)
    {
        // Nothing names the program: the one the Store installed runs, as it is when the host starts.
        map.insert(
            "executable".to_owned(),
            serde_json::Value::String(path.to_string_lossy().into_owned()),
        );
    }
    if let Some(sources) = &asks.setting_sources {
        map.insert(
            "setting_sources".to_owned(),
            serde_json::Value::from(sources.clone()),
        );
    }
    if let Some(modes) = &asks.modes {
        map.insert(
            "modes".to_owned(),
            serde_json::Value::from(modes.iter().map(|mode| mode.id()).collect::<Vec<_>>()),
        );
        tracing::info!(
            "host.agents.{}.modes is {}: a paired client may choose those, beyond the entry's own mode and stricter ones",
            entry.name,
            modes
                .iter()
                .map(|mode| mode.id())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    if asks.bypass {
        map.insert("allow_bypass".to_owned(), serde_json::Value::Bool(true));
        tracing::info!(
            "host.agents.{}.bypass is on: a paired client may put a session in Bypass, which asks for nothing",
            entry.name
        );
    }
    // Requests a connection works on at once: what the host always allowed, not the provider's smaller default.
    map.entry("max_requests".to_owned())
        .or_insert_with(|| serde_json::Value::from(HARNESS_MAX_REQUESTS));
    // The entry's own settings, so each is configuration; a client's choice stays within the entry's mode.
    let mut chosen = |key: &str, from_key: Option<&String>, loose: &[&str], default: &str| {
        let (value, label) = match (from_key, map.get(key).and_then(serde_json::Value::as_str)) {
            (Some(value), _) => (value.clone(), format!("host.agents.{}.{key}", entry.name)),
            (None, Some(value)) => (
                value.to_owned(),
                format!("host.agents.{}.settings.{key}", entry.name),
            ),
            (None, None) => (default.to_owned(), String::new()),
        };
        if loose.contains(&value.as_str()) {
            tracing::info!(
                "non-default: {label} is {value}, default {default}: the command line acts with nothing asked"
            );
        }
        map.insert(key.to_owned(), serde_json::Value::String(value));
    };
    if harness == "claude" {
        chosen(
            "permission_mode",
            asks.permission_mode.as_ref(),
            &["bypassPermissions", "acceptEdits", "auto"],
            "default",
        );
    } else if harness == "codex" {
        chosen(
            "approval_policy",
            asks.approval_policy.as_ref(),
            &["never", "on-failure"],
            "on-request",
        );
        chosen(
            "sandbox",
            asks.sandbox.as_ref(),
            &["danger-full-access"],
            "workspace-write",
        );
    }
    Settings::new(map)
}

/// Whether an offered account's credential reaches the agent in its environment, for a kind that names the model's address:
/// the login kinds keep theirs in a file, and every other kind with an address is a key or a token. An account another
/// provider made, that provides an API standard, is a key whatever its kind says of where it rests.
fn keeps_key_in_environment(listed: &[kind::Offered], runs: &str) -> bool {
    listed.iter().any(|(_, account, info)| {
        let borrowed = account.harness != runs
            && info
                .provides
                .iter()
                .any(|standard| standard.ends_with("-api"));
        borrowed
            || (!info.state_dir
                && info
                    .endpoints
                    .iter()
                    .any(|slot| slot.purpose == "model_api" && slot.key.is_some()))
    })
}

/// The accounts an entry of `harness` offers, each with its kind in the catalog of the provider that made it, once canoa.cfg's
/// settings for it are checked against that kind's schema. An account another provider made is offered when what its kind
/// provides meets what `harness` accepts.
pub(crate) fn offered_accounts(
    config: &Config,
    harness: &str,
    names: &[String],
    provider: &kind::Harness,
) -> Result<Vec<kind::Offered>, String> {
    let catalog = provider
        .catalog()
        .ok_or_else(|| "the provider did not list the accounts it takes".to_owned())?;
    let catalogs = Catalogs::with(config, harness, catalog, provider.listing());
    let mut listed = Vec::new();
    for name in names {
        let Some(account) = config.accounts.get(name) else {
            return Err(format!("account {name} is not in [host.accounts]"));
        };
        let Some(info) = catalogs.kind(account) else {
            return Err(format!(
                "account {name}: the provider has no account kind {}",
                account.kind
            ));
        };
        fits(catalog, harness, account, &info)
            .map_err(|why| format!("account {name} cannot run on {harness}: {why}"))?;
        crate::accounts::check_settings(&info, account.settings.as_ref())
            .map_err(|why| format!("account {name}: {why}"))?;
        listed.push((name.clone(), account.clone(), info));
    }
    Ok(listed)
}

/// The accounts, of `harness` or of another provider whose standards it accepts, that an entry which does not list them can
/// still run a session on, for a key trusted with accounts. One whose key reaches the agent in its environment is left out:
/// only an entry that lists it limits the settings files a repository could use to redirect that key, so it has to be listed.
pub(crate) fn other_accounts(
    config: &Config,
    harness: &str,
    listed: &[kind::Offered],
    provider: &kind::Harness,
) -> Vec<kind::Offered> {
    let Some(catalog) = provider.catalog() else {
        return Vec::new();
    };
    let catalogs = Catalogs::with(config, harness, catalog, provider.listing());
    let mut others = Vec::new();
    for (name, account) in &config.accounts {
        if listed.iter().any(|(each, ..)| each == name) {
            continue;
        }
        let Some(info) = catalogs.kind(account) else {
            continue;
        };
        if fits(catalog, harness, account, &info).is_err() {
            continue;
        }
        if crate::accounts::check_settings(&info, account.settings.as_ref()).is_err() {
            continue;
        }
        let one = (name.clone(), account.clone(), info);
        if keeps_key_in_environment(core::slice::from_ref(&one), harness) {
            continue;
        }
        others.push(one);
    }
    others
}

/// The accounts of other providers that `provider`'s harness cannot run a session on, each with why: the standards the account
/// provides and the ones the harness accepts.
pub(crate) fn unfit_accounts(
    config: &Config,
    harness: &str,
    provider: &kind::Harness,
) -> Vec<(String, String)> {
    let Some(catalog) = provider.catalog() else {
        return Vec::new();
    };
    let catalogs = Catalogs::with(config, harness, catalog, provider.listing());
    config
        .accounts
        .iter()
        .filter_map(|(name, account)| {
            let info = catalogs.kind(account)?;
            fits(catalog, harness, account, &info)
                .err()
                .map(|why| (name.clone(), why))
        })
        .collect()
}

/// What a connection's sessions are given beside the agent: its profile, and the delegation, secret and store tools.
type Joined<'a> = (
    Option<&'a Connection>,
    Option<&'a Arc<delegate::Conn>>,
    Option<&'a Arc<secrets::Link>>,
    Option<&'a Arc<store_mcp::Link>>,
    Option<&'a Arc<kb_mcp::Link>>,
    Option<&'a Arc<client_tools::Link>>,
);

/// Serves one connection with an `acp_harness` provider in this process, which [`Stopper::stop_agents`] ends like any agent.
/// With a profile, its environment goes to the provider's program, and the provider lets Claude Code take the plugin the
/// host writes for its skills: the host removes every `_meta` a client sends, so that plugin is the only one that can reach
/// it.
fn give_harness<C: Channel>(
    shared: &Shared,
    served: &Served,
    channel: &C,
    (peer, first, any_account): (&PeerId, &[u8], bool),
    harness: &kind::Harness,
    scope: &Scope,
    (connection, conn, link, store, kb, tools): Joined<'_>,
) {
    let turn_away = |why: String| {
        let id = serde_json::from_slice::<serde_json::Value>(first)
            .ok()
            .and_then(|message| message.get("id").cloned());
        if let Some(id) = id {
            let answer = serde_json::json!({"jsonrpc": "2.0", "id": id,
                "error": {"code": -32602, "message": why}});
            let _ = channel.send(answer.to_string().as_bytes());
        }
        tracing::info!(entry = served.name, %peer, "turned away: {why}");
    };
    let lifetime = match harness.lifetime_of(first) {
        Ok(lifetime) => lifetime,
        Err(why) => return turn_away(why),
    };
    // The account is chosen, and held for this connection, before anything starts.
    let account = match harness.choose(
        first,
        &format!("a connection to {}", served.name),
        any_account,
    ) {
        Ok(account) => account,
        Err(why) => return turn_away(why),
    };
    let control = InProcess::new();
    let key = match shared.record(|| Ok((Process::Translator(Arc::clone(&control)), ()))) {
        Ok(Some((key, ()))) => key,
        Ok(None) | Err(_) => return,
    };
    let started = harness.connection(
        connection.map(|connection| {
            let environment = connection
                .env()
                .iter()
                .map(|(name, value)| (name.clone(), value.expose().to_owned()))
                .collect();
            (environment, connection.skills())
        }),
        account.as_ref(),
        lifetime,
    );
    let gated = Gated::new(
        channel,
        account.as_ref().map(Held::report),
        account.as_ref().is_some_and(Held::rotates),
    );
    // Usage is reported under the account that ran the connection, whichever entry offered it.
    let accounted;
    let scope = match &account {
        Some(held) => {
            accounted = scope.clone().account(held.id().to_owned());
            &accounted
        }
        None => scope,
    };
    let to_provider = (started.request, started.spawner);
    let end = match connection {
        Some(connection) => inproc::serve_kind(
            &ToolsLayer::new(
                &KbLayer::new(
                    &StoreLayer::new(
                        &Layer::new(
                            &Delegating::new(
                                &Recording::new(
                                    &Hosted::in_process(&gated, scope, connection),
                                    &shared.history,
                                    &served.name,
                                    harness.id(),
                                ),
                                conn,
                            ),
                            link,
                        ),
                        store,
                    ),
                    kb,
                ),
                tools,
            ),
            &*started.harness,
            to_provider,
            started.watch,
            MAX_LINE,
            &control,
        ),
        None => inproc::serve_kind(
            &ToolsLayer::new(
                &KbLayer::new(
                    &StoreLayer::new(
                        &Layer::new(
                            &Delegating::new(
                                &Recording::new(
                                    &Scoped::new(&gated, scope),
                                    &shared.history,
                                    &served.name,
                                    harness.id(),
                                ),
                                conn,
                            ),
                            link,
                        ),
                        store,
                    ),
                    kb,
                ),
                tools,
            ),
            &*started.harness,
            to_provider,
            started.watch,
            MAX_LINE,
            &control,
        ),
    };
    // What the connection held goes back only after the provider has said it is done with the account.
    drop(account);
    tracing::info!(entry = served.name, %peer, "left: {end:?}");
    shared.forget(key);
}

/// Serves one connection with a provider process of its own, which [`Stopper::stop_agents`] ends like any agent.
fn give_provider<C: Channel>(
    shared: &Shared,
    served: &Served,
    channel: &C,
    peer: &PeerId,
    (program, args): (&OsString, &[OsString]),
    line: &HostLine,
) {
    let mut prepared = match child::prepare(program, args) {
        Ok(prepared) => prepared,
        Err(error) => {
            return tracing::warn!(entry = served.name, "the provider could not start: {error}");
        }
    };
    let started = shared.record(|| {
        let (running, streams) = prepared.start()?;
        let running = Arc::new(running);
        Ok((Process::Provider(Arc::clone(&running)), (running, streams)))
    });
    let (key, (running, streams)) = match started {
        Ok(Some(started)) => started,
        Ok(None) => return,
        Err(error) => {
            return tracing::warn!(entry = served.name, "the provider could not start: {error}");
        }
    };
    let end = child::serve(
        &served.name,
        prepared,
        &running,
        streams,
        line,
        &crate::scope::Screening::new(channel),
    );
    tracing::info!(entry = served.name, %peer, "left: {end}");
    shared.forget(key);
    running.stop();
}

/// The user's home as the environment names it.
fn home_from_environment() -> Option<PathBuf> {
    let name = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tasks_connection_cannot_open_the_control_channel_and_this_process_can() {
        let task = delegate::ChildContext {
            principal: delegate::Principal::Owner,
            roots: Vec::new(),
            depth: 1,
            parent_session: None,
        };
        assert!(reaches_control(control::NAME, None));
        assert!(!reaches_control(control::NAME, Some(&task)));
        assert!(!reaches_control("Claude", None));
    }

    #[test]
    fn a_built_in_harness_keeps_the_request_limit_the_host_always_had_unless_the_entry_sets_one() {
        let config = Config::from_str_in(
            "[host.agents.a]\nharness = \"claude\"\n[host.agents.b]\nharness = \"codex\"\nsettings = { max_requests = 8 }\n",
            std::path::Path::new("/canoa"),
        )
        .expect("parses");
        let limit = |name: &str| {
            let entry = config.entry(name).expect("there");
            let EntryKind::Harness {
                harness,
                executable,
                options,
                settings,
                ..
            } = &entry.kind
            else {
                panic!("a harness");
            };
            harness_settings(
                entry,
                harness,
                executable.as_ref(),
                options,
                settings.as_ref(),
            )
            .as_map()["max_requests"]
                .clone()
        };
        assert_eq!(limit("a"), serde_json::json!(HARNESS_MAX_REQUESTS));
        assert_eq!(limit("b"), serde_json::json!(8));
    }

    #[test]
    fn stopping_the_host_ends_an_agent_started_for_a_reading_and_refuses_the_next() {
        let agents = Arc::new(Mutex::new(Agents::default()));
        let starter = private_starter(&agents);
        let (program, args): (OsString, Vec<OsString>) = if cfg!(windows) {
            (
                "ping".into(),
                vec!["-n".into(), "60".into(), "127.0.0.1".into()],
            )
        } else {
            ("sleep".into(), vec!["60".into()])
        };
        let (agent, _forget) = (starter.0)(&program, &args)
            .expect("it starts")
            .expect("the host is not stopping");
        assert_eq!(agents.lock().expect("lock").running.len(), 1);
        assert!(
            agent.try_status().expect("status").is_none(),
            "it is running"
        );
        end_all(&agents);
        let until = std::time::Instant::now() + core::time::Duration::from_secs(10);
        while agent.try_status().expect("status").is_none() && std::time::Instant::now() < until {
            std::thread::sleep(core::time::Duration::from_millis(50));
        }
        assert!(
            agent.try_status().expect("status").is_some(),
            "stopping the host ended it"
        );
        assert!(
            (starter.0)(&program, &args).expect("no error").is_none(),
            "none starts while the host stops"
        );
    }

    #[test]
    fn a_provider_command_line_holds_no_setting() {
        let config = Config::from_str_in(
            "[host.providers.p]\nsource = \"s\"\nprovider = \"p\"\n[host.providers.p.source_settings]\npaths = [\"SECRET-1\"]\n[host.providers.p.settings]\nkey = \"SECRET-2\"\n",
            std::path::Path::new("/canoa"),
        )
        .expect("parses");
        let entry = &config.entries[0];
        let EntryKind::Provider {
            origin,
            provider,
            directory,
            max_sessions,
            ..
        } = &entry.kind
        else {
            panic!("a provider");
        };
        let args = provider_args(&[], entry, origin, provider, directory, *max_sessions);
        assert!(
            args.iter().any(|arg| arg == "--settings-on-stdin"),
            "{args:?}"
        );
        assert!(
            !args
                .iter()
                .any(|arg| arg.to_string_lossy().contains("SECRET")),
            "{args:?}"
        );
    }

    /// An offered account as the entry's harness would see it: made by `harness`, of a kind that keeps nothing in a directory.
    fn offered(harness: &str, provides: &[&str], endpoint: bool) -> kind::Offered {
        let mut info =
            coqui_acp_api::AccountKindInfo::new("k", "K").provides(provides.iter().copied());
        if endpoint {
            info.endpoints = vec![coqui_acp_api::EndpointSlot::new("model_api").key("base_url")];
        }
        let account = crate::config::AccountConfig {
            harness: harness.to_owned(),
            kind: "k".to_owned(),
            id: "id".to_owned(),
            settings: None,
        };
        ("a".to_owned(), account, info)
    }

    #[test]
    fn a_key_another_provider_made_is_a_key_in_the_environment_whatever_its_kind_says_of_an_address()
     {
        // Its own provider's key says it by its address; an API standard's account made elsewhere says it by what it provides.
        assert!(keeps_key_in_environment(
            &[offered("claude", &["anthropic-api"], true)],
            "claude"
        ));
        assert!(!keeps_key_in_environment(
            &[offered("claude", &["claude-code"], false)],
            "claude"
        ));
        assert!(keeps_key_in_environment(
            &[offered("claude", &["anthropic-api"], false)],
            "pi"
        ));
        assert!(!keeps_key_in_environment(
            &[offered("pi", &["pi"], false)],
            "claude"
        ));
    }
}
