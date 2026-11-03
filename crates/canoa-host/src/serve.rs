//! Each entry on a loopback WebSocket of its own: a peer proves a key the trust store accepts, then every message is sealed
//! in the session the proof ended in, and a fresh agent process serves that connection until either side ends.

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use core::net::SocketAddr;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};
use std::thread;

use canoa::{Capability, ClientId, Registry, Service, ServiceId};
use coqui_acp_bridge::agent::Agent;
use coqui_acp_bridge::relay::relay;
use coqui_agentic_peer::auth::{self, Sealed};
use coqui_agentic_peer::link::Prefilled;
use coqui_agentic_peer::mux;
use coqui_peer_webcrypto::{Identity, PeerId, Trust, TrustStore, WebCrypto};
use coqui_web::websocket::{self, Frames};
use coqui_web::{Channel, Failure, Limits};

use crate::child::{self, Running};
use crate::config::{Config, Entry, EntryKind, Origin};
use crate::provider::HostLine;
use crate::scope::{Scope, Scoped};

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
pub use crate::child::{OUTPUT_LIMIT, START_LIMIT};
/// What a peer is granted on an entry, in [`Registry`] terms.
pub const ACP_CAPABILITY: &str = "acp/1";

/// What [`Serving`] needs beyond the configuration.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ServeOptions {
    /// The program that serves a provider: this host's own executable, run as `PROGRAM provider ...`.
    pub provider_program: PathBuf,
}

impl ServeOptions {
    /// Options running providers through `provider_program`.
    #[must_use]
    pub fn new(provider_program: impl Into<PathBuf>) -> Self {
        Self {
            provider_program: provider_program.into(),
        }
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
                "Canoa.cfg has no [host.agents], [host.providers] or [host.tools] entry to serve",
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
struct Gate(Arc<AtomicUsize>, usize);

struct Entered(Arc<AtomicUsize>);

impl Drop for Entered {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Gate {
    fn new(ceiling: usize) -> Self {
        Self(Arc::new(AtomicUsize::new(0)), ceiling)
    }

    fn enter(&self) -> Option<Entered> {
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

/// What runs for one entry's connections.
#[derive(Debug)]
enum Run {
    Agent {
        program: OsString,
        args: Vec<OsString>,
        scope: Scope,
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
    method: WebCrypto,
    store: Arc<TrustStore>,
    registry: Registry,
    unproven: Gate,
    agents: Mutex<Agents>,
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
    Agent(Arc<Agent>),
    Provider(Arc<Running>),
}

impl Process {
    /// Ends it and what it started.
    fn stop(&self) {
        match self {
            Self::Agent(agent) => {
                let _ = agent.stop();
            }
            Self::Provider(running) => running.stop(),
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
    ) -> std::io::Result<Option<(u64, Arc<Agent>)>> {
        self.record(|| {
            let agent = Arc::new(Agent::spawn(program, args, MAX_LINE)?);
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
    /// Ends every agent process running, with what each started, and every later connection before its agent starts.
    /// The listeners stay bound: end the process to free them.
    pub fn stop_agents(&self) {
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
    let mut args = vec![
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
    ];
    for root in &entry.working_roots {
        args.push(OsString::from("--working-root"));
        args.push(root.as_os_str().to_owned());
    }
    args
}

/// The arguments that make `program` serve a tool entry: its name and the entry as JSON, which holds no setting a client
/// gave and nothing the tool's own command line does not show.
fn tool_args(name: &str, spec: &crate::tool::ToolSpec) -> Vec<OsString> {
    vec![
        OsString::from("tool"),
        OsString::from("--name"),
        OsString::from(name),
        OsString::from("--spec"),
        OsString::from(serde_json::to_string(spec).unwrap_or_default()),
    ]
}

impl Serving {
    /// Binds every entry of `config` on its loopback address, proving peers with `identity` against `store`. Nothing is
    /// accepted until [`Serving::run`].
    ///
    /// # Errors
    ///
    /// [`ServeError::NothingToServe`] for a configuration with no entry, [`ServeError::Listen`] for an address that cannot
    /// be bound.
    pub fn bind(
        config: &Config,
        identity: Identity,
        store: TrustStore,
        options: &ServeOptions,
    ) -> Result<Self, ServeError> {
        if config.entries.is_empty() {
            return Err(ServeError::NothingToServe);
        }
        let store = Arc::new(store);
        let method =
            WebCrypto::new(identity).with_initiators(Trust::Store(TrustStore::clone(&store)));
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
                EntryKind::Agent { program, args } => (
                    "agent",
                    Run::Agent {
                        program: program.clone(),
                        args: args.clone(),
                        scope: Scope::new(entry.working_roots.clone()),
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
                    if let Origin::Source { settings, .. } = origin {
                        line.source_settings = Some(settings.clone());
                    }
                    (
                        "provider",
                        Run::Provider {
                            program: options.provider_program.as_os_str().to_owned(),
                            args: provider_args(entry, origin, provider, directory, *max_sessions),
                            line,
                        },
                    )
                }
                // A stdio agent like any other, so the scope applies to its clients and its processes end with the link.
                EntryKind::Tool { spec } => (
                    "tool",
                    Run::Agent {
                        program: options.provider_program.as_os_str().to_owned(),
                        args: tool_args(&entry.name, spec),
                        scope: Scope::new(entry.working_roots.clone()),
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
            }),
            listeners,
            listening,
        })
    }

    /// Where each entry listens, in the configuration's order.
    #[must_use]
    pub fn listening(&self) -> &[Listening] {
        &self.listening
    }

    /// What ends the agents this serves, for a thread other than the one in [`Serving::run`].
    #[must_use]
    pub fn stopper(&self) -> Stopper {
        Stopper(Arc::clone(&self.shared))
    }

    /// Takes connections on every entry until the process ends. Each handshake and proof runs on a thread of its own, so
    /// a slow peer holds up nobody.
    pub fn run(self) {
        let threads: Vec<_> = self
            .listeners
            .into_iter()
            .map(|(served, listener)| {
                let shared = Arc::clone(&self.shared);
                thread::Builder::new()
                    .name(format!("canoa-host-{}", served.name))
                    .spawn(move || accept_loop(&shared, &served, &listener))
            })
            .collect();
        for thread in threads.into_iter().flatten() {
            let _ = thread.join();
        }
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
    loop {
        let pending = match listener.next(None) {
            Ok(pending) => pending,
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
            .name("canoa-host-peer".to_owned())
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
        MAX_LINE + auth::RECORD_OVERHEAD,
    ) {
        Ok(established) => established,
        Err(error) => {
            return tracing::info!(
                entry = served.name,
                "a key was refused: {error}; a key that proved itself and is not paired waits for `canoa-host accept`"
            );
        }
    };
    drop(unproven);
    let peer = established.peer;
    if !shared.allows(&peer, served) {
        return tracing::info!(entry = served.name, %peer, "a paired key this entry does not list was turned away");
    }
    let store = Arc::clone(&shared.store);
    let sealed =
        Sealed::new(channel, established.session).with_check(move || store.is_accepted(&peer));
    let Some(_running) = served.running.enter() else {
        tracing::info!(entry = served.name, %peer, "turned away: max_peers are running");
        return sealed.close();
    };
    match sealed.receive(Some(FIRST_MESSAGE)) {
        Ok(Some(first)) if mux::starts_mux(&first) => {
            tracing::info!(entry = served.name, %peer, "a link of streams was refused: this host serves plain ACP");
        }
        Ok(Some(first)) => {
            tracing::info!(entry = served.name, %peer, "proven");
            give_agent(shared, served, &Prefilled::new(&sealed, first), &peer);
        }
        Ok(None) | Err(_) => {}
    }
    sealed.close();
}

fn give_agent<C: Channel>(shared: &Shared, served: &Served, channel: &C, peer: &PeerId) {
    let (program, args, scope) = match &served.run {
        Run::Agent {
            program,
            args,
            scope,
        } => (program, args, scope),
        Run::Provider {
            program,
            args,
            line,
        } => return give_provider(shared, served, channel, peer, (program, args), line),
    };
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
        let args = provider_args(entry, origin, provider, directory, *max_sessions);
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
