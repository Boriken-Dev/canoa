//! Each entry on a loopback WebSocket of its own: a peer proves a key the trust store accepts, then every message is sealed
//! in the session the proof ended in, and a fresh agent process serves that connection until either side ends.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use canoa::brand::CONFIG_FILE;
use core::fmt;
use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};
use std::thread;

use cabuya::auth::{Method, Proof};
use canoa::{Capability, ClientId, Registry, ServiceId};
use coqui_acp_bridge::agent::Agent as BridgeAgent;
use coqui_peer_webcrypto::{PeerId, TrustStore};
use coqui_transport_web::websocket;
use door::Door;
use indexmap::IndexMap;
pub use limit::MAX_MESSAGE_META;

use crate::child::Running;
use crate::client_tools;
use crate::config::{Config, Entry, EntryKind, HostProfile, Layers};
use crate::control;
use crate::delegate::Delegation;
use crate::environments;
use crate::history;
use crate::inproc::InProcess;
use crate::kb_mcp;
use crate::kind;
use crate::pipe;
use crate::profile::hosted::{self, Offer};
use crate::provider::HostLine;
use crate::scope::Scope;
use crate::secrets;
use crate::store::mcp as store_mcp;
use crate::transport;
use crate::usage_splice::Starter;
use crate::worktree::Worktrees;

mod accounts;
mod admission;
mod bind;
mod connection;
mod door;
mod kept;
mod limit;
mod options;
mod sessions;
mod watch;

#[cfg(test)]
mod tests;

pub(crate) use accounts::{
    builtin_provider_settings, entry_accounts, harness_settings, keeps_key_in_environment,
    offered_accounts, other_accounts, unfit_accounts,
};
pub(crate) use admission::{Admission, NotAdmitted};
pub use admission::{Local, LocalError};
pub(crate) use connection::{Reach, plain_listener};
use connection::{accept_loop, accept_pipe_loop, accept_transport_loop, connection};
pub(crate) use options::{Entered, Gate};
pub use options::{Listening, ServeError, ServeOptions};

/// Worktree requests one connection may make before its first ACP message.
const MAX_WORKTREE_REQUESTS: usize = 16;

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
        /// Every paired key gets these grants whatever `grants` says.
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
    /// The profile items the host offers, opened once when it started.
    items: crate::profile::ItemSet,
    /// The offer the last readable file gave.
    current: Mutex<Option<Arc<Offer>>>,
}

impl Refresh {
    /// The offer for a connection starting now: the file read again, or the last good one when it no longer parses.
    fn offer(&self) -> Option<Arc<Offer>> {
        let mut current = self.current.lock().unwrap_or_else(PoisonError::into_inner);
        match Config::load_layers(&self.layers) {
            Ok(config) => {
                let local = config
                    .local_profiles
                    .for_agent(&self.entry)
                    .map(|mut each| {
                        each.items = self.items.clone();
                        each
                    });
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
                    "{CONFIG_FILE} could not be read again, so the profile offered stays as it was: {error}"
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
    /// The environment the entry's agent runs in, when it names one: a client may ask for another.
    environment: Option<String>,
}

/// What every connection shares.
struct Shared {
    method: Proof,
    /// The sessions roles serve, moved to the next member when a turn of one ends out of usage.
    roles: Arc<crate::roles::Hub>,
    /// The session services the loaded libraries offer.
    services: Arc<crate::services::Services>,
    /// What this host exports of its sessions, when the person turned that on.
    telemetry: Arc<crate::telemetry::Telemetry>,
    store: Arc<TrustStore>,
    registry: Registry,
    unproven: Gate,
    agents: Arc<Mutex<Agents>>,
    /// Held while agents are being ended, so that whoever else ends them returns only once they have.
    ending: Mutex<()>,
    /// Set by [`Stopper::stop`]: every listener and link ends.
    stopped: Arc<AtomicBool>,
    program_args: Vec<OsString>,
    /// The program `canoa launch` is run through, which gives an agent its environment on its first line of input.
    launcher: OsString,
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
    /// The host's own model work: knowledge search and the jobs that need a model ask it, and its harness sessions end with the host.
    assist: Arc<crate::assist::Assist>,
    /// What gives a connection's sessions the environments they use as tools, and the environments themselves.
    env_tools: Arc<environments::mcp::Hub>,
    /// What gives a connection's sessions the tools programs offer (a browser extension's), and takes their offers.
    client_tools: Arc<client_tools::Hub>,
    /// What `[host.control]` grants each key, and what every key holds beside it.
    control_grants: IndexMap<PeerId, BTreeSet<control::Grant>>,
    control_everyone: Option<BTreeSet<control::Grant>>,
    /// The keys of pages this host's own web server admitted (`[host.web] office_local`).
    admitted: Arc<Mutex<BTreeSet<PeerId>>>,
    page_grants: BTreeSet<control::Grant>,
    /// Where the add-in on this computer keeps what it needs between runs (its secret, the keys it admitted); none without a home.
    office_dir: Option<PathBuf>,
    /// The sessions kept for more than one connection.
    hosted: Arc<sessions::Hosted>,
    /// The longest ACP message, as one line, either way (`[host] max_message_bytes`).
    max_line: usize,
    /// What listing a session's MCP servers is held to (`[host.mcp_list]`).
    mcp_list: crate::config::McpListConfig,
    /// Keeps the computer awake while an agent turn runs, when `[settings.power] keep_awake` is on.
    keep_awake: Arc<crate::awake::KeepAwake>,
    /// Tells the person what happens on the host.
    notices: Arc<crate::notify::Hub>,
    /// This run, recorded as ended when the host ends.
    #[expect(dead_code, reason = "held for its drop")]
    run_record: Option<crate::notify::run::Record>,
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
            let mut grants = self.control_grants.get(peer).cloned();
            if self.is_admitted(peer) {
                grants
                    .get_or_insert_with(BTreeSet::new)
                    .extend(self.page_grants.iter().copied());
            }
            grants
        }
    }

    /// Whether the host's own web server admitted `peer` as the key of the page it serves.
    fn is_admitted(&self, peer: &PeerId) -> bool {
        self.admitted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(peer)
    }

    /// Starts an agent and records it, under the key to [`Shared::forget`] it by; `Ok(None)` once the host is stopping.
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
                self.max_line,
            )?);
            Ok((Process::Agent(Arc::clone(&agent)), agent))
        })
    }

    /// Starts an agent in the environment `env`, with `identity` in its environment, and records it as [`Shared::start`] does.
    fn start_in(
        &self,
        env: &Arc<environments::Env>,
        program: &OsString,
        args: &[OsString],
        identity: &[(String, String)],
    ) -> std::io::Result<Option<(u64, Arc<BridgeAgent>)>> {
        self.record(|| {
            let spawner = environments::agent::InEnvironment::new(Arc::clone(env))
                .with_variables(identity.to_vec());
            let agent = Arc::new(BridgeAgent::spawn_with(
                &spawner,
                program,
                args,
                self.max_line,
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

    /// Whether the entry names `peer` in its `peers`: an entry that lists no key is open to every paired key, which is not
    /// enough for a key that came to run tasks here.
    fn lists(&self, peer: &PeerId, served: &Served) -> bool {
        served.restricted && self.allows(peer, served)
    }

    /// Whether `peer`, whose key the store accepts, may use this entry.
    fn allows(&self, peer: &PeerId, served: &Served) -> bool {
        !served.restricted
            || (matches!(served.run, Run::Control { .. }) && self.is_admitted(peer))
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

/// What re-reads an entry's profile at each connection: an entry of the host's own or one that names no profile.
fn refresh_of(
    entry: &Entry,
    names: &[String],
    config: &Config,
    options: &ServeOptions,
    items: &crate::profile::ItemSet,
) -> Option<Refresh> {
    let layers = options
        .layers
        .clone()
        .filter(|_| entry.local || names.is_empty())?;
    let launcher = options.provider_program.as_os_str().to_owned();
    let before = options.program_args.clone();
    let local = config
        .local_profiles
        .for_agent(&entry.name)
        .map(|mut each| {
            each.items = items.clone();
            each
        });
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
        items: items.clone(),
    })
}

/// Which profile an entry's sessions start with and why, in words: what `canoa check` prints for the entry.
pub(crate) fn applied_profile(config: &Config, entry: &Entry) -> String {
    let named = entry_profiles(entry);
    if !named.is_empty() {
        return format!("{} (named by the entry)", named.join(", "));
    }
    match &entry.kind {
        EntryKind::Agent { .. } | EntryKind::Harness { .. } => {
            if config.local_profiles.for_agent(&entry.name).is_some() {
                format!("default ({CONFIG_FILE}'s top level; the entry names none)")
            } else {
                format!("none (the entry names none and {CONFIG_FILE} has no profile)")
            }
        }
        _ => "none (the entry's kind takes no profile)".to_owned(),
    }
}

/// The `[host.profiles]` an entry names; none for an entry of a kind that has no profile to name.
fn entry_profiles(entry: &Entry) -> &[String] {
    match &entry.kind {
        EntryKind::Agent { profiles, .. } | EntryKind::Harness { profiles, .. } => profiles,
        _ => &[],
    }
}

fn private_starter(agents: &Arc<Mutex<Agents>>, max_line: usize) -> Starter {
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
            max_line,
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
        self.0.assist.close();
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

impl Serving {
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

    /// What lets the host's own web server admit the key of the page it serves (`[host.web] office_local`).
    #[must_use]
    pub(crate) fn admission(&self, secret: bool) -> Admission {
        let admission = Admission::new(
            Arc::clone(&self.shared.store),
            Arc::clone(&self.shared.admitted),
        );
        match &self.shared.office_dir {
            Some(dir) => admission.remembering(dir.join("admitted.json"), secret),
            None => admission,
        }
    }

    /// Where the add-in on this computer keeps its secret, in the host's home; `None` for a host with no home.
    #[must_use]
    pub(crate) fn office_dir(&self) -> Option<PathBuf> {
        self.shared.office_dir.clone()
    }

    /// Tells the native door's `hello` where this host's web app is, so a program on this computer that reached the door finds the
    /// page with no address configured: `{"web_app": {"url", "office", "office_local"}}`. Nothing is added with no door. `url` is what
    /// [`crate::web::WebServer::url`] says; a program decides for itself whether it will open it.
    pub fn note_web(&self, url: &str, office: bool, office_local: bool) {
        let Some(door) = &self.door else { return };
        let hello = door.hello();
        let mut hello = hello
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(claim) = hello.as_object_mut() {
            claim.insert(
                "web_app".to_owned(),
                serde_json::json!({ "url": url, "office": office, "office_local": office_local }),
            );
        }
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

    /// Starts the host's scheduled tasks (`[schedules.ID]`): from here the host runs each at its time, whether or not any
    /// application is connected, until it stops. A host with no home or no canoa.cfg has none.
    pub(crate) fn start_schedules(&self) {
        if let Some(schedules) = self.shared.control.schedules() {
            schedules.start(self.local(), Arc::clone(&self.shared.stopped));
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
        self.shared.telemetry.stop();
    }
}

/// The user's home as the environment names it.
fn home_from_environment() -> Option<PathBuf> {
    let name = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}
