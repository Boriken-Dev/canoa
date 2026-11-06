//! Each entry on a loopback WebSocket of its own: a peer proves a key the trust store accepts, then every message is sealed
//! in the session the proof ended in, and a fresh agent process serves that connection until either side ends.

use alloc::collections::BTreeMap;
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
use coqui_peer_webcrypto::{PeerId, TrustStore};
use coqui_web::websocket::{self, Frames};
use coqui_web::{Channel, Failure, Limits};

use crate::child::{self, Running};
use crate::config::{Config, Entry, EntryKind, ImportSearch, Origin};
use crate::harness::AgentOptions;
use crate::inproc::{self, InProcess};
use crate::profile::hosted::{self, Connection, Hosted, Offer};
use crate::provider::HostLine;
use crate::scope::{Scope, Scoped};
use crate::worktree::Worktrees;

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
    /// The program that serves a provider, a tool or a profile's launch: this host's own executable, run as
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
        }
    }

    /// What one key may carry in each direction, in bytes. No floor; above the default the host logs it as weakened.
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
    /// The address a client dials.
    pub url: String,
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
    /// A harness's translator, in this process.
    Harness(AgentOptions),
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
    agents: Mutex<Agents>,
    /// Held while agents are being ended, so that whoever else ends them returns only once they have.
    ending: Mutex<()>,
    /// Set by [`Stopper::stop`]: every listener and link ends.
    stopped: Arc<AtomicBool>,
    program_args: Vec<OsString>,
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
    InProcess(Arc<InProcess>),
}

impl Process {
    /// Ends it and what it started.
    fn stop(&self) {
        match self {
            Self::Agent(agent) => {
                let _ = agent.stop();
            }
            Self::Provider(running) => running.stop(),
            Self::InProcess(agent) => agent.stop(),
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
        self.stop_agents();
    }

    /// Ends every agent process running, with what each started, and every later connection before its agent starts. It
    /// returns once they have ended, also when another thread is ending them. The listeners stay bound: end the process to
    /// free them, or use [`Stopper::stop`].
    pub fn stop_agents(&self) {
        let _ending = self.0.ending.lock().unwrap_or_else(PoisonError::into_inner);
        let running = {
            let mut agents = self.0.agents.lock().unwrap_or_else(PoisonError::into_inner);
            agents.stopping = true;
            core::mem::take(&mut agents.running)
        };
        for process in running.values() {
            process.stop();
        }
    }
}

/// Every entry bound to its address, not yet taking connections.
pub struct Serving {
    shared: Arc<Shared>,
    listeners: Vec<(Arc<Served>, websocket::Listener)>,
    listening: Vec<Listening>,
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
        if config.entries.is_empty() && !options.nothing_to_serve {
            return Err(ServeError::NothingToServe);
        }
        let store = Arc::new(store);
        let mut method =
            Proof::new(identity).with_initiators(Trust::Store(TrustStore::clone(&store)));
        if let Some(bytes) = options.key_limit {
            method = method.with_key_limit(bytes);
        }
        // Turning the key change off is every method's to do: WebCrypto has none, and ends a session at its limit.
        if !options.rekey
            && let Ok(off) = method.clone().with_rekey(false)
        {
            method = off;
        }
        for weakened in method.weakened() {
            tracing::warn!(
                "weakened: {} is {}, looser than {}",
                weakened.setting,
                weakened.value,
                weakened.strict
            );
        }
        let mut registry = Registry::new();
        let mut listeners = Vec::new();
        let mut listening = Vec::new();
        for entry in &config.entries {
            let listener = websocket::Listener::bind(entry.listen, None)
                .and_then(|listener| Ok((listener.local_addr()?, listener)))
                .map_err(|failure| ServeError::Listen {
                    entry: entry.name.clone(),
                    failure,
                })?;
            let (bound, listener) = listener;
            let listener = listener
                .with_limits(unproven_limits())
                .with_frames(Frames::Binary)
                .with_origins(config.origins.clone());
            let url = url_of(bound);
            let (kind, run) = match &entry.kind {
                EntryKind::Agent {
                    program,
                    args,
                    profiles,
                } => (
                    "agent",
                    Run::Agent {
                        agent: Agent::Program {
                            program: program.clone(),
                            args: args.clone(),
                        },
                        scope: Scope::new(entry.working_roots.clone()),
                        offer: hosted::offer(
                            profiles,
                            &config.profiles,
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
                    if let Origin::Source {
                        settings,
                        import_search,
                        ..
                    } = origin
                    {
                        if cfg!(windows) && *import_search == ImportSearch::Path {
                            tracing::warn!(
                                "weakened: host.providers.{}.import_search is path, looser than library: python3.dll and \
                                 what else the source imports itself may come from the working directory or PATH; set \
                                 import_dirs to where it is, or import_search = \"library\"",
                                entry.name
                            );
                        }
                        // Guatiao's source cannot tell its host which weaker setting it took, so the host says it.
                        if settings
                            .as_map()
                            .get("isolation")
                            .and_then(|value| value.as_str())
                            == Some("inherit")
                        {
                            tracing::warn!(
                                "weakened: host.providers.{}.source_settings.isolation is inherit, looser than shipped",
                                entry.name
                            );
                        }
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
                    executable,
                    profiles,
                } => (
                    "agent",
                    Run::Agent {
                        agent: Agent::Harness(harness_options(
                            entry,
                            *harness,
                            executable.as_ref(),
                        )),
                        scope: Scope::new(entry.working_roots.clone()),
                        offer: hosted::offer(
                            profiles,
                            &config.profiles,
                            options.provider_program.as_os_str().to_owned(),
                        ),
                        worktrees: Worktrees::new(entry.working_roots.clone(), entry.worktrees),
                    },
                ),
                // A stdio agent like any other, so the scope applies to its clients and its processes end with the link.
                EntryKind::Tool { spec } => (
                    "tool",
                    Run::Agent {
                        agent: Agent::Program {
                            program: options.provider_program.as_os_str().to_owned(),
                            args: tool_args(&options.program_args, &entry.name, spec),
                        },
                        scope: Scope::new(entry.working_roots.clone()),
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
            listening.push(Listening {
                name: entry.name.clone(),
                kind,
                url,
            });
            let served = Served {
                name: entry.name.clone(),
                restricted: entry.peers.is_some(),
                run,
                running: Gate::new(entry.max_peers),
            };
            listeners.push((Arc::new(served), listener));
        }
        Ok(Self {
            shared: Arc::new(Shared {
                method,
                store,
                registry,
                unproven: Gate::new(MAX_UNPROVEN),
                agents: Mutex::new(Agents::default()),
                ending: Mutex::new(()),
                stopped: Arc::new(AtomicBool::new(false)),
                program_args: options.program_args.clone(),
            }),
            listeners,
            listening,
        })
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
                    .spawn(move || accept_loop(&shared, &served, &listener))
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
    Limits::default()
        .with_max_message(auth::MAX_PROOF_MESSAGE)
        .with_pending(UNPROVEN_PENDING)
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
    let sealed = Sealed::new(channel, established.session)
        .with_check(move || !stopped.load(Ordering::SeqCst) && store.is_accepted(&peer));
    let Some(_running) = served.running.enter() else {
        tracing::info!(entry = served.name, %peer, "turned away: max_peers are running");
        return sealed.close();
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
                return sealed.close();
            }
            answered += 1;
            if let Some(reply) = worktrees.answer(message)
                && sealed.send(&reply).is_err()
            {
                return sealed.close();
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
                &Prefilled::new(&sealed, first),
                &copy,
                &peer,
            );
        }
        Ok(None) | Err(_) => {}
    }
    sealed.close();
}

fn give_agent<C: Channel>(
    shared: &Shared,
    served: &Served,
    channel: &C,
    first: &[u8],
    peer: &PeerId,
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
    };
    let connection = match offer {
        Some(offer) => match prepare(served, channel, first, peer, offer) {
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
                peer,
                options,
                scope,
                connection.as_ref(),
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
        let end = relay(&Scoped::new(channel, scope), &agent);
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
    let hosted = Hosted::new(channel, scope, &connection);
    let end = relay(&hosted, &agent);
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
        Ok(connection) => Some(connection),
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

/// A harness entry's translator options. The host's scope already holds a client's `cwd` and more directories to the
/// entry's roots and adds directories of its own after it, so the translator takes any; a resumed Codex thread's own
/// directory, which no request shows the host, is held to the roots by the translator.
fn harness_options(
    entry: &Entry,
    harness: crate::Harness,
    executable: Option<&OsString>,
) -> AgentOptions {
    let mut options = AgentOptions::new(harness);
    options.executable = executable.cloned();
    options.resume_roots.clone_from(&entry.working_roots);
    options
}

/// Serves one connection with a harness's translator in this process, which [`Stopper::stop_agents`] ends like any agent.
/// With a profile, its environment goes to the command line's process, and Claude Code takes the plugin the host writes
/// for its skills: the host removes every `_meta` a client sends, so that plugin is the only one that can reach it.
fn give_harness<C: Channel>(
    shared: &Shared,
    served: &Served,
    channel: &C,
    peer: &PeerId,
    options: &AgentOptions,
    scope: &Scope,
    connection: Option<&Connection>,
) {
    let mut options = options.clone();
    if let Some(connection) = connection {
        options.env = connection
            .env()
            .iter()
            .map(|(name, value)| (OsString::from(name), OsString::from(value.expose())))
            .collect();
        options.client_plugins = true;
    }
    let control = InProcess::new();
    let key = match shared.record(|| Ok((Process::InProcess(Arc::clone(&control)), ()))) {
        Ok(Some((key, ()))) => key,
        Ok(None) | Err(_) => return,
    };
    let entry = served.name.clone();
    let notice: coqui_acp_harness::acp::NoticeHook =
        Arc::new(move |text: &str| tracing::info!(entry, "{text}"));
    let settings = options.settings(notice);
    let end = match connection {
        Some(connection) => inproc::serve(
            &Hosted::in_process(channel, scope, connection),
            options.agent(),
            settings,
            MAX_LINE,
            &control,
        ),
        None => inproc::serve(
            &Scoped::new(channel, scope),
            options.agent(),
            settings,
            MAX_LINE,
            &control,
        ),
    };
    tracing::info!(entry = served.name, %peer, "left: {end:?}");
    shared.forget(key);
}

// Serves one connection with a provider process of its own, which [`Stopper::stop_agents`] ends like any agent.
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
