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
use indexmap::IndexMap;

use crate::accounts::{Accounts, Gated, Held};
use crate::child::{self, Running};
use crate::config::{
    BUILT_IN_HARNESSES, Config, Entry, EntryKind, HarnessOptions, ImportSearch, Listen, Origin,
    Settings, UsageSource,
};
use crate::control;
use crate::delegate::{self, Delegating, Delegation, Peek, Principal};
use crate::history::{self, Recording};
use crate::inproc::{self, InProcess};
use crate::kind;
use crate::local::{self, Link};
use crate::pipe;
use crate::profile::hosted::{self, Connection, Hosted, Offer};
use crate::provider::HostLine;
use crate::scope::{Scope, Scoped};
use crate::sources;
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
}

impl ServeOptions {
    /// Options running providers through `provider_program`, with each method's default session settings.
    #[must_use]
    pub fn new(provider_program: impl Into<PathBuf>) -> Self {
        Self {
            provider_program: provider_program.into(),
            program_args: Vec::new(),
            key_limit: None,
            rekey: true,
            nothing_to_serve: false,
            state_dir: None,
            local_sources: false,
            programs: None,
        }
    }

    /// Serves the host's own sources over [`Local`] alongside canoa.cfg's entries. A name canoa.cfg already uses is left to it.
    #[must_use]
    pub const fn with_local_sources(mut self, on: bool) -> Self {
        self.local_sources = on;
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
    /// The entry's `title`, if it has one.
    pub title: Option<String>,
    /// Whether it is reached over [`Local`] only: one of the host's own sources, with no address.
    pub local: bool,
    /// The programs a local entry needs that were not found; empty for any other, and for a source that can start.
    pub missing: Vec<String>,
}

impl Listening {
    /// Whether a browser can dial it: it listens on an address, not on a pipe or a Unix socket.
    #[must_use]
    pub fn on_address(&self) -> bool {
        !self.url.starts_with("pipe:") && !self.url.starts_with("unix:")
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
    },
}

#[derive(Debug)]
struct Served {
    name: String,
    restricted: bool,
    run: Run,
    running: Gate,
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
    /// The ledger and the rest of what belongs to no entry.
    control: Arc<control::Control>,
    /// What lets an agent session hand a task to another source; `None` when `[host.delegate]` is off.
    delegation: Option<Delegation>,
    /// The sessions the host serves, recorded in its home.
    history: Arc<history::Store>,
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
            let agent = Arc::new(BridgeAgent::spawn(program, args, MAX_LINE)?);
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
fn private_starter(agents: &Arc<Mutex<Agents>>) -> Starter {
    let agents = Arc::clone(agents);
    Starter(Arc::new(move |program, args| {
        let mut held = agents.lock().unwrap_or_else(PoisonError::into_inner);
        if held.stopping {
            return Ok(None);
        }
        let agent = Arc::new(BridgeAgent::spawn(program, args, MAX_LINE)?);
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

    /// [`Local::open`], for a task of the delegation when `context` is given: what the task's connection is bounded by.
    pub(crate) fn open_with(
        &self,
        entry: &str,
        context: Option<delegate::ChildContext>,
    ) -> Result<Link, LocalError> {
        if entry == control::NAME {
            return self.open_control();
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
                serve_proven(&shared, &served, &host, &peer, context.as_ref());
                host.close();
            })
            .map_err(|_| LocalError::Thread)?;
        Ok(client)
    }
}

impl Local {
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
}

impl delegate::Host for DelegationHost {
    fn sources(&self) -> Vec<delegate::Source> {
        self.agents.clone()
    }

    fn may_use(&self, who: &Principal, source: &str) -> bool {
        let Some(shared) = self.shared.upgrade() else {
            return false;
        };
        let Some(served) = self.served.iter().find(|served| served.name == source) else {
            return false;
        };
        match who {
            Principal::Owner => true,
            Principal::Key(peer) => shared.allows(peer, served),
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
}

/// What an entry listens on.
enum Bound {
    Socket(websocket::Listener),
    Pipe(pipe::Listener),
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

/// Binds one entry's address or pipe, listing what is not the default of it in the log.
fn bind_listen(
    name: &str,
    table: &str,
    listen: &Listen,
    origins: &[String],
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
            let listener = pipe::Listener::bind(pipe_name).map_err(|error| ServeError::Pipe {
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
        let sources = if options.local_sources {
            sources::entries(config)
        } else {
            Vec::new()
        };
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
                )?;
                (Some(bound), url)
            };
            // The host's own sources are launched with what canoa.cfg's top level says; an entry on an address, with its
            // `[host.profiles]`.
            let local_profile = entry
                .local
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
                            let scope = Scope::new(entry.working_roots.clone())
                                .account(entry.name.clone())
                                .limit_phrases(config.limit_phrases.clone());
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
                        ),
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
                    let roots = entry
                        .working_roots
                        .iter()
                        .map(|root| root.to_string_lossy().into_owned())
                        .collect();
                    let provider = kind::Harness::new(
                        &registry,
                        harness,
                        &configured,
                        !profiles.is_empty() || local_profile.is_some(),
                        roots,
                    )
                    .map_err(|error| refused(error.to_string()))?
                    .with_lifetime(asks.cache_lifetime);
                    for (setting, value, default) in provider.non_defaults() {
                        tracing::info!(
                            "non-default: host.agents.{}: {setting} is {value}, default {default}",
                            entry.name
                        );
                    }
                    let provider = match &accounts {
                        Some(host) if !offered.is_empty() => {
                            let listed = offered_accounts(config, harness, offered, &provider)
                                .map_err(refused)?;
                            provider.offering(host, listed)
                        }
                        _ => provider,
                    };
                    let provider = Arc::new(provider);
                    movers.insert(entry.name.clone(), Arc::clone(&provider));
                    (
                        "agent",
                        Run::Agent {
                            agent: Agent::Harness(provider),
                            scope: Scope::new(entry.working_roots.clone())
                                .account(entry.name.clone())
                                .limit_phrases(config.limit_phrases.clone()),
                            offer: hosted::offer(
                                profiles,
                                &config.profiles,
                                local_profile.as_ref(),
                                options.provider_program.as_os_str().to_owned(),
                            ),
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
                title: entry.title.clone(),
                local: entry.local,
                missing,
            });
            let served = Arc::new(Served {
                name: entry.name.clone(),
                restricted: entry.peers.is_some(),
                run,
                running: Gate::new(entry.max_peers),
            });
            match listener {
                Some(listener) => listeners.push((served, listener)),
                None => local_only.push(served),
            }
        }
        if let Some(settings) = &config.control
            && let Some(listen) = &settings.listen
        {
            let (bound, url) = bind_listen(control::NAME, "control", listen, &config.origins)?;
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
                title: None,
                local: false,
                missing: Vec::new(),
            });
            listeners.push((
                Arc::new(Served {
                    name: control::NAME.to_owned(),
                    restricted: true,
                    run: Run::Control {
                        grants: settings.grants.clone(),
                    },
                    running: Gate::new(crate::config::DEFAULT_MAX_PEERS),
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
        let serving = Self {
            shared: Arc::new(Shared {
                history: Arc::clone(&recorded),
                method,
                store,
                registry,
                unproven: Gate::new(MAX_UNPROVEN),
                agents,
                ending: Mutex::new(()),
                stopped: Arc::new(AtomicBool::new(false)),
                program_args: options.program_args.clone(),
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
                    let kept = kept.with_history(control::Moves {
                        store: recorded,
                        targets: movers,
                    });
                    match &options.programs {
                        Some((programs, providers)) => kept.with_stores(control::Stores::new(
                            Arc::clone(programs),
                            providers.clone(),
                        )),
                        None => kept,
                    }
                }),
                delegation,
            }),
            listeners,
            local_only,
            listening,
        };
        serving.attach_delegation();
        Ok(serving)
    }

    /// Gives the delegation the agents this host serves and the way to open one for a task.
    fn attach_delegation(&self) {
        let Some(delegation) = &self.shared.delegation else {
            return;
        };
        let agents = self
            .listening
            .iter()
            .filter(|listening| listening.kind == "agent")
            .map(|listening| delegate::Source {
                name: listening.name.clone(),
                title: listening.title.clone(),
                harness: listening.harness.clone(),
            })
            .collect();
        delegation.attach(Arc::new(DelegationHost {
            shared: Arc::downgrade(&self.shared),
            served: self.local().served,
            agents,
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

    /// Takes connections on every entry until [`Stopper::stop`] or the process ends. Each handshake and proof runs on a
    /// thread of its own, so a slow peer holds up nobody. It returns with every listener closed and every agent ended.
    pub fn run(self) {
        let stopper = self.stopper();
        let nothing = self.listeners.is_empty();
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
                    connection(&shared, &served, &channel, entered);
                    channel.close();
                }
                Err(failure) => tracing::debug!("a handshake failed: {failure}"),
            });
        if let Err(error) = spawned {
            tracing::warn!("a connection was dropped: {error}");
        }
    }
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
                connection(&shared, &served, &channel, entered);
                channel.close();
            });
        if let Err(error) = spawned {
            tracing::warn!("a connection was dropped: {error}");
        }
    }
}

impl Shared {
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

fn connection<C: Channel>(shared: &Shared, served: &Served, channel: &C, unproven: Entered) {
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
    let store = Arc::clone(&shared.store);
    let stopped = Arc::clone(&shared.stopped);
    let checked = peer;
    let sealed = Sealed::new(channel, established.session)
        .with_check(move || !stopped.load(Ordering::SeqCst) && store.is_accepted(&checked));
    serve_proven(shared, served, &sealed, &peer, None);
    sealed.close();
}

/// Serves a connection whose peer is known, from its first message to its end; the caller closes `channel` after.
fn serve_proven<C: Channel>(
    shared: &Shared,
    served: &Served,
    sealed: &C,
    peer: &PeerId,
    context: Option<&delegate::ChildContext>,
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
        Ok(Some(first)) if mux::starts_mux(&first) => {
            tracing::info!(entry = served.name, %peer, "a link of streams was refused: this host serves plain ACP");
        }
        Ok(Some(first)) => {
            tracing::info!(entry = served.name, %peer, "proven");
            let copy = first.clone();
            give_agent(
                shared,
                served,
                &Prefilled::new(sealed, first),
                (&copy, peer),
                context,
            );
        }
        Ok(None) | Err(_) => {}
    }
}

fn give_agent<C: Channel>(
    shared: &Shared,
    served: &Served,
    channel: &C,
    (first, peer): (&[u8], &PeerId),
    context: Option<&delegate::ChildContext>,
) {
    let (agent, scope, offer) = match &served.run {
        Run::Agent {
            agent,
            scope,
            offer,
            ..
        } => (agent, scope, offer),
        Run::Provider {
            program,
            args,
            line,
        } => return give_provider(shared, served, channel, peer, (program, args), line),
        Run::Control { grants } => {
            let grants = grants
                .iter()
                .find(|(key, _)| *key == peer)
                .map(|(_, grants)| grants.clone())
                .unwrap_or_default();
            let caller = control::Caller::Peer {
                id: peer.to_string(),
                grants,
            };
            return control::serve(&shared.control, &caller, channel);
        }
    };
    // What lets this connection's sessions hand tasks to other sources: for a task's own connection, what the host bounded it by.
    let conn = shared.delegation.as_ref().map(|delegation| {
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
    let peeked = Peek::new(channel, conn.as_ref());
    let channel = &peeked;
    let connection = match offer {
        Some(offer) => match prepare(
            served,
            channel,
            first,
            peer,
            offer,
            // Only this process's own link is asked to approve: a key reaches the configuration's behaviour.
            *peer == shared.method.peer_id(),
        ) {
            Some(connection) => Some(connection),
            None => return,
        },
        None => None,
    };
    let (program, args) = match agent {
        Agent::Program { program, args } => (program, args),
        Agent::Harness(options) => {
            return give_harness(
                shared,
                served,
                channel,
                (peer, first),
                options,
                scope,
                (connection.as_ref(), conn.as_ref()),
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
        let end = relay(&Delegating::new(&scoped, conn.as_ref()), &agent);
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
    let end = relay(&Delegating::new(&hosted, conn.as_ref()), &agent);
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
    let lookup = |name: &str| std::env::var(name).ok();
    match Connection::prepare(offer, index, &served.name, &lookup) {
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

/// What loading `origin` for `entry` departs from the defaults in, said as configuration where the file asked for it, and as
/// a warning where the host fell back.
fn say_origin(entry: &Entry, origin: &Origin) {
    let Origin::Source {
        settings,
        import_search,
        import_search_named,
        ..
    } = origin
    else {
        return;
    };
    let table = table_of(&entry.kind);
    if cfg!(windows) && *import_search == ImportSearch::Path {
        if *import_search_named {
            tracing::info!(
                "non-default: host.{table}.{}.import_search is path, default library: python3.dll                  and what else the source imports itself may come from the working directory or PATH",
                entry.name
            );
        } else {
            // The file does not say it: import_dirs names no directory, so the host fell back.
            tracing::warn!(
                "host.{table}.{}.import_search is path, because import_dirs names no directory:                  python3.dll and what else the source imports itself may come from the working                  directory or PATH; set import_dirs to where it is, or import_search = \"library\"",
                entry.name
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
            "non-default: host.{table}.{}.source_settings.isolation is inherit, default shipped",
            entry.name
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

/// The accounts an entry of `harness` offers, each with its kind in the provider's catalog, once canoa.cfg's settings for
/// it are checked against that kind's schema.
fn offered_accounts(
    config: &Config,
    harness: &str,
    names: &[String],
    provider: &kind::Harness,
) -> Result<Vec<kind::Offered>, String> {
    let catalog = provider
        .catalog()
        .ok_or_else(|| "the provider did not list the accounts it takes".to_owned())?;
    let mut listed = Vec::new();
    for name in names {
        let Some(account) = config.accounts.get(name) else {
            return Err(format!("account {name} is not in [host.accounts]"));
        };
        if account.harness != harness {
            return Err(format!(
                "account {name} is of harness {}, not {harness}",
                account.harness
            ));
        }
        let Some(info) = catalog.kind(&account.kind) else {
            return Err(format!(
                "account {name}: the provider has no account kind {}",
                account.kind
            ));
        };
        crate::accounts::check_settings(info, account.settings.as_ref())
            .map_err(|why| format!("account {name}: {why}"))?;
        listed.push((name.clone(), account.clone(), info.clone()));
    }
    Ok(listed)
}

/// Serves one connection with an `acp_harness` provider in this process, which [`Stopper::stop_agents`] ends like any agent.
/// With a profile, its environment goes to the provider's program, and the provider lets Claude Code take the plugin the
/// host writes for its skills: the host removes every `_meta` a client sends, so that plugin is the only one that can reach
/// it.
fn give_harness<C: Channel>(
    shared: &Shared,
    served: &Served,
    channel: &C,
    (peer, first): (&PeerId, &[u8]),
    harness: &kind::Harness,
    scope: &Scope,
    (connection, conn): (Option<&Connection>, Option<&Arc<delegate::Conn>>),
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
    let account = match harness.choose(first, &format!("a connection to {}", served.name)) {
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
    let to_provider = (started.request, started.spawner, started.keeper);
    let end = match connection {
        Some(connection) => inproc::serve_kind(
            &Delegating::new(
                &Recording::new(
                    &Hosted::in_process(&gated, scope, connection),
                    &shared.history,
                    &served.name,
                    harness.id(),
                ),
                conn,
            ),
            &*started.harness,
            to_provider,
            MAX_LINE,
            &control,
        ),
        None => inproc::serve_kind(
            &Delegating::new(
                &Recording::new(
                    &Scoped::new(&gated, scope),
                    &shared.history,
                    &served.name,
                    harness.id(),
                ),
                conn,
            ),
            &*started.harness,
            to_provider,
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
    let end = child::serve(&served.name, prepared, &running, streams, line, channel);
    tracing::info!(entry = served.name, %peer, "left: {end}");
    shared.forget(key);
    running.stop();
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
