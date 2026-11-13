//! An agent session handing a task to another agent. The host runs one delegation agent (A2A, through `coqui-a2a`): each source
//! it may use is a skill on its card, and each task is an A2A task whose work is a normal host session of that source, opened
//! with [`crate::Local`], whose only prompt is the task.
//!
//! An agent reaches it through three MCP tools (`delegate_task`, `task_status`, `cancel`) on a loopback HTTP server behind a
//! bearer minted for that one session. The bearer is written into the session's `mcpServers` as the host relays `session/new`,
//! revoked when the connection ends, kept in memory only, and never logged. An agent that takes no HTTP MCP server is given the
//! same tools as a stdio server instead, [`bridge`], with the bearer in its environment. Every limit on what a task may be given (its source,
//! its folder, its mode, how many) is checked in [`agent`] before a process starts, so every way in takes one path.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "locks, threads and paths are std's"
)]

mod agent;
pub(crate) mod bridge;
mod channel;
mod child;
pub(crate) mod privilege;
mod server;
mod tools;

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::time::Duration;

use coqui_a2a::error::{A2aError, ErrorKind};
use coqui_a2a::tasks::TaskStore;
use coqui_a2a::wire::ServiceParameters;
use coqui_a2a::{Caller, Server, Watch};
use coqui_peer_webcrypto::PeerId;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

pub(crate) use channel::{Delegating, Peek};

use crate::config::DelegateConfig;
use crate::local::Link;
use crate::serve::LocalError;
use privilege::Modes;

/// The `_meta` key of `initialize` that asks for the delegation tools, and of a child's permission request that names its task.
pub(crate) const META: &str = "dev.boriken.canoa/delegate";
/// The name of the MCP server the host adds to a session.
pub(crate) const SERVER_NAME: &str = "canoa-delegate";
/// Sessions of one connection that hold a bearer at once.
const MAX_SESSIONS: usize = 256;
/// Requests of one connection the host remembers an answer for.
const MAX_PENDING: usize = 64;

/// Who a connection is, for what it may delegate to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Principal {
    /// This process's own link: every source.
    Owner,
    /// A paired key: the entries that list it.
    Key(PeerId),
}

/// A source the host can run a task on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Source {
    pub(crate) name: String,
    pub(crate) title: Option<String>,
    /// The provider id of a `harness` entry.
    pub(crate) harness: Option<String>,
}

/// What a task's connection is given when the host opens it, which a client's messages cannot set.
#[derive(Clone, Debug)]
pub(crate) struct ChildContext {
    pub(crate) principal: Principal,
    pub(crate) roots: Vec<PathBuf>,
    pub(crate) depth: usize,
}

/// What the delegation needs of the host it runs in.
pub(crate) trait Host: Send + Sync {
    /// The agents a task may name, in the order they are offered.
    fn sources(&self) -> Vec<Source>;
    /// Whether a connection of `who` may use `source`.
    fn may_use(&self, who: &Principal, source: &str) -> bool;
    /// What kind of agent `source` is: equal for entries that run the same provider or program, so that a mode id means the same
    /// in both. `None` when it is not known.
    fn kind(&self, source: &str) -> Option<String>;
    /// A connection to `source` for a task.
    fn open(&self, source: &str, context: ChildContext) -> Result<Link, LocalError>;
}

/// What a connection that may delegate is.
#[derive(Clone, Debug)]
pub(crate) struct ConnInfo {
    pub(crate) principal: Principal,
    pub(crate) entry: String,
    pub(crate) roots: Vec<PathBuf>,
    pub(crate) depth: usize,
    /// Whether the connection is a task's own, which is offered the tools without asking.
    pub(crate) child: bool,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A caller in process, owning what a `Caller` borrows.
pub(crate) struct Who {
    peer: String,
    claims: std::collections::BTreeMap<String, String>,
    parameters: ServiceParameters,
    watch: Watch,
}

impl Who {
    pub(crate) fn new(peer: String) -> Self {
        Self {
            peer,
            claims: std::collections::BTreeMap::new(),
            parameters: ServiceParameters::default(),
            watch: Watch::new(),
        }
    }

    pub(crate) fn caller(&self) -> Caller<'_> {
        Caller::local(&self.peer, &self.claims, &self.parameters, &self.watch)
    }
}

impl Drop for Who {
    fn drop(&mut self) {
        self.watch.close();
    }
}

/// The command line of the stdio bridge: this program, with what goes before its subcommand, then `mcp-bridge`. It holds no
/// secret: the address and the bearer travel in the environment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BridgeCommand {
    pub(crate) program: String,
    pub(crate) args: Vec<String>,
}

impl BridgeCommand {
    /// `program`, run as `PROGRAM [before] mcp-bridge`.
    pub(crate) fn new(program: &std::path::Path, before: &[std::ffi::OsString]) -> Self {
        let mut args: Vec<String> = before
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        args.push("mcp-bridge".to_owned());
        Self {
            program: program.to_string_lossy().into_owned(),
            args,
        }
    }
}

/// How an agent is given the tools.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Way {
    Http,
    Stdio,
}

/// What the host keeps for every connection and session.
#[derive(Default)]
struct State {
    /// Bearer digests to the session they open.
    tokens: HashMap<[u8; 32], Arc<Session>>,
    /// A2A callers (`local:CONNECTION:SESSION`) to their sessions.
    sessions: HashMap<String, Arc<Session>>,
    /// Tasks running, across the host.
    running: usize,
}

/// The delegation of one host.
pub(crate) struct Engine {
    pub(crate) config: DelegateConfig,
    host: OnceLock<Arc<dyn Host>>,
    url: OnceLock<String>,
    /// What runs the stdio bridge for an agent that takes no HTTP MCP server.
    bridge: OnceLock<BridgeCommand>,
    a2a: Server,
    next: AtomicU64,
    stopped: AtomicBool,
    state: Mutex<State>,
}

impl Engine {
    pub(crate) fn new(config: DelegateConfig) -> Arc<Self> {
        Arc::new_cyclic(|weak| Self {
            config,
            host: OnceLock::new(),
            url: OnceLock::new(),
            bridge: OnceLock::new(),
            a2a: Server::new(agent::DelegationAgent::new(weak.clone())),
            next: AtomicU64::new(1),
            stopped: AtomicBool::new(false),
            state: Mutex::new(State::default()),
        })
    }

    fn host(&self) -> Option<&Arc<dyn Host>> {
        self.host.get()
    }

    pub(crate) fn stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }

    /// The session a bearer opens. The table is keyed by the bearer's digest, so no comparison of the secret itself is timed.
    pub(crate) fn session_of_token(&self, bearer: &str) -> Option<Arc<Session>> {
        let digest: [u8; 32] = Sha256::digest(bearer.as_bytes()).into();
        lock(&self.state).tokens.get(&digest).cloned()
    }

    fn session_of_peer(&self, peer: &str) -> Option<Arc<Session>> {
        lock(&self.state).sessions.get(peer).cloned()
    }

    /// Takes the host's place for a task: one more of the session's and of the host's, or the limit it would pass.
    fn reserve(self: &Arc<Self>, session: &Arc<Session>) -> Result<Reservation, A2aError> {
        let mut state = lock(&self.state);
        if state.running >= self.config.max_running {
            return Err(limit(
                "max_running",
                format!(
                    "this host already runs {} tasks (max_running)",
                    self.config.max_running
                ),
            ));
        }
        if session.live.load(Ordering::SeqCst) >= self.config.max_children {
            return Err(limit(
                "max_children",
                format!(
                    "this session already has {} tasks running (max_children)",
                    self.config.max_children
                ),
            ));
        }
        state.running += 1;
        session.live.fetch_add(1, Ordering::SeqCst);
        Ok(Reservation {
            engine: Arc::clone(self),
            session: Arc::clone(session),
        })
    }

    /// Ends the delegation: no bearer is valid after, and every task ends.
    pub(crate) fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        let sessions: Vec<Arc<Session>> = {
            let mut state = lock(&self.state);
            state.tokens.clear();
            state.sessions.drain().map(|(_, session)| session).collect()
        };
        for session in sessions {
            session.stop_children();
        }
    }

    /// Ends the tasks of `session` the way the person's Stop does.
    pub(crate) fn cancel_session(self: &Arc<Self>, session: &Arc<Session>) {
        let Some(peer) = session.peer() else { return };
        let tasks: Vec<String> = lock(&session.children).keys().cloned().collect();
        if tasks.is_empty() {
            return;
        }
        let engine = Arc::clone(self);
        let spawned = std::thread::Builder::new()
            .name("canoa-delegate-stop".to_owned())
            .spawn(move || {
                let who = Who::new(peer);
                let (engine, who) = (&engine, &who);
                std::thread::scope(|scope| {
                    for id in &tasks {
                        scope.spawn(move || {
                            let _ = engine.a2a.call(
                                &who.caller(),
                                coqui_a2a::wire::Method::CancelTask,
                                json!({"id": id}),
                            );
                        });
                    }
                });
            });
        if let Err(error) = spawned {
            tracing::warn!("delegation: a stop could not start its thread: {error}");
        }
    }
}

fn limit(setting: &str, what: String) -> A2aError {
    A2aError::new(ErrorKind::InvalidRequest, what).with_metadata("limit", setting)
}

/// A place in the host's counts, given back when the task's process is gone.
pub(crate) struct Reservation {
    engine: Arc<Engine>,
    session: Arc<Session>,
}

impl Drop for Reservation {
    fn drop(&mut self) {
        let mut state = lock(&self.engine.state);
        state.running = state.running.saturating_sub(1);
        drop(state);
        self.session.live.fetch_sub(1, Ordering::SeqCst);
    }
}

/// One agent session that was given the tools.
pub(crate) struct Session {
    conn: Weak<Conn>,
    conn_id: u64,
    pub(crate) cwd: PathBuf,
    digest: [u8; 32],
    id: Mutex<Option<String>>,
    modes: Mutex<Modes>,
    pub(crate) store: TaskStore,
    pub(crate) children: Mutex<HashMap<String, Arc<child::Child>>>,
    live: AtomicUsize,
    /// Calls of the tool server this session's bearer has under way.
    pub(crate) calls: AtomicUsize,
}

impl Session {
    /// The A2A caller of this session, once the agent has named it.
    pub(crate) fn peer(&self) -> Option<String> {
        lock(&self.id)
            .as_ref()
            .map(|id| format!("local:{}:{id}", self.conn_id))
    }

    pub(crate) fn session_id(&self) -> Option<String> {
        lock(&self.id).clone()
    }

    /// The mode the session is in as far as the host has seen it.
    pub(crate) fn mode(&self) -> Option<String> {
        lock(&self.modes).current.clone()
    }

    pub(crate) fn conn(&self) -> Option<Arc<Conn>> {
        self.conn.upgrade()
    }

    /// Ends every task of the session now, with no grace.
    fn stop_children(&self) {
        let children: Vec<Arc<child::Child>> = lock(&self.children).values().cloned().collect();
        for child in children {
            child.stop_now();
        }
    }
}

/// A person's answer to a request the host asked for a task.
struct Waiting {
    answer: Mutex<Option<Value>>,
    ready: Condvar,
}

/// A request the host remembers until the agent answers it.
enum Pending {
    Initialize,
    Open {
        session: Option<Arc<Session>>,
        known: Option<String>,
    },
    Mode {
        session: String,
        mode: String,
    },
    Option {
        session: String,
        id: String,
        value: String,
    },
}

#[derive(Default)]
struct ConnState {
    /// The client asked for the delegation tools in `initialize`.
    asked: bool,
    /// The agent takes MCP servers over HTTP.
    http: bool,
    /// The agent takes MCP servers over stdio, which version 1 requires of every agent.
    stdio: bool,
    /// The ACP version the agent answered `initialize` in: it decides how a stdio server is written.
    version: u64,
    pending: HashMap<String, Pending>,
    sessions: Vec<Arc<Session>>,
}

/// One client connection that may delegate: what it is, the sessions that hold a bearer, and the messages the host adds to its
/// stream (a task's permission request) and the answers it waits for.
pub(crate) struct Conn {
    id: u64,
    engine: Arc<Engine>,
    info: ConnInfo,
    state: Mutex<ConnState>,
    outbox: Mutex<VecDeque<Vec<u8>>>,
    answers: Mutex<HashMap<String, Arc<Waiting>>>,
    closed: AtomicBool,
    asked: AtomicU64,
}

impl Engine {
    pub(crate) fn connect(self: &Arc<Self>, info: ConnInfo) -> Arc<Conn> {
        let asked = info.child;
        Arc::new(Conn {
            id: self.next.fetch_add(1, Ordering::SeqCst),
            engine: Arc::clone(self),
            info,
            state: Mutex::new(ConnState {
                asked,
                ..ConnState::default()
            }),
            outbox: Mutex::new(VecDeque::new()),
            answers: Mutex::new(HashMap::new()),
            closed: AtomicBool::new(false),
            asked: AtomicU64::new(0),
        })
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

impl Conn {
    /// The entry this connection is to.
    pub(crate) fn entry(&self) -> &str {
        &self.info.entry
    }

    pub(crate) fn principal(&self) -> &Principal {
        &self.info.principal
    }

    pub(crate) fn roots(&self) -> &[PathBuf] {
        &self.info.roots
    }

    pub(crate) fn depth(&self) -> usize {
        self.info.depth
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst) || self.engine.stopped()
    }

    /// Reads the client's `initialize` before the scope removes its `_meta`.
    pub(crate) fn peek(&self, message: &[u8]) {
        if !contains(message, b"initialize") || !contains(message, META.as_bytes()) {
            return;
        }
        let Ok(value) = serde_json::from_slice::<Value>(message) else {
            return;
        };
        if value.get("method") == Some(&json!("initialize")) {
            let asked = value
                .pointer("/params/_meta")
                .and_then(|meta| meta.get(META))
                == Some(&json!(true));
            lock(&self.state).asked = asked;
        }
    }

    /// How a session opened now is given the tools, or `None` when it is given none: over HTTP when the agent takes it, else
    /// through the stdio bridge.
    fn offering(&self, state: &ConnState) -> Option<Way> {
        let ready = state.asked
            && self.info.depth < self.engine.config.max_depth
            && self.engine.config.enabled
            && self.engine.url.get().is_some()
            && self.engine.host().is_some()
            && !self.is_closed();
        if !ready {
            None
        } else if state.http {
            Some(Way::Http)
        } else if state.stdio && self.engine.bridge.get().is_some() {
            Some(Way::Stdio)
        } else {
            None
        }
    }

    /// What the host adds to the stream to the client.
    pub(crate) fn drain(&self) -> Vec<Vec<u8>> {
        lock(&self.outbox).drain(..).collect()
    }

    /// A message from the client, as the scope passed it. `None` when it was an answer to the host's own question.
    pub(crate) fn client_said(self: &Arc<Self>, message: Vec<u8>) -> Option<Vec<u8>> {
        let waiting = !lock(&self.answers).is_empty();
        if !(waiting || contains(&message, b"session/") || contains(&message, b"initialize")) {
            return Some(message);
        }
        let Ok(mut value) = serde_json::from_slice::<Value>(&message) else {
            return Some(message);
        };
        let id = value.get("id").map(Value::to_string);
        let Some(method) = value
            .get("method")
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            if let Some(id) = value.get("id").and_then(Value::as_str)
                && self.answer(id, &value)
            {
                return None;
            }
            return Some(message);
        };
        match (method.as_str(), id) {
            ("initialize", Some(id)) => self.remember(id, Pending::Initialize),
            ("session/new" | "session/load" | "session/resume", Some(id)) => {
                if self.open(&mut value, &method, id) {
                    return serde_json::to_vec(&value).ok().or(Some(message));
                }
            }
            ("session/set_mode", Some(id)) => {
                let params = &value["params"];
                if let (Some(session), Some(mode)) =
                    (params["sessionId"].as_str(), params["modeId"].as_str())
                {
                    let pending = Pending::Mode {
                        session: session.to_owned(),
                        mode: mode.to_owned(),
                    };
                    self.remember(id, pending);
                }
            }
            ("session/set_config_option", Some(id)) => {
                let params = &value["params"];
                if let (Some(session), Some(option), Some(chosen)) = (
                    params["sessionId"].as_str(),
                    params["configId"].as_str(),
                    params["value"].as_str(),
                ) {
                    let pending = Pending::Option {
                        session: session.to_owned(),
                        id: option.to_owned(),
                        value: chosen.to_owned(),
                    };
                    self.remember(id, pending);
                }
            }
            ("session/cancel", _) => {
                if let Some(id) = value.pointer("/params/sessionId").and_then(Value::as_str)
                    && let Some(session) = self.session(id)
                {
                    self.engine.cancel_session(&session);
                }
            }
            _ => {}
        }
        Some(message)
    }

    fn remember(&self, id: String, pending: Pending) {
        let mut state = lock(&self.state);
        if state.pending.len() < MAX_PENDING {
            state.pending.insert(id, pending);
        }
    }

    fn session(&self, id: &str) -> Option<Arc<Session>> {
        lock(&self.state)
            .sessions
            .iter()
            .find(|session| session.session_id().as_deref() == Some(id))
            .cloned()
    }

    /// Adds the host's MCP server, with a bearer of its own, to a session request. Whether the request was changed.
    fn open(self: &Arc<Self>, value: &mut Value, method: &str, id: String) -> bool {
        let known = (method != "session/new")
            .then(|| value.pointer("/params/sessionId").and_then(Value::as_str))
            .flatten()
            .map(str::to_owned);
        let cwd = value
            .pointer("/params/cwd")
            .and_then(Value::as_str)
            .map(PathBuf::from);
        let mut state = lock(&self.state);
        let way = self
            .offering(&state)
            .filter(|_| state.sessions.len() < MAX_SESSIONS);
        let (Some(cwd), Some(way)) = (cwd, way) else {
            if state.pending.len() < MAX_PENDING {
                state.pending.insert(
                    id,
                    Pending::Open {
                        session: None,
                        known,
                    },
                );
            }
            return false;
        };
        let Some(url) = self.engine.url.get() else {
            return false;
        };
        let mut secret = [0_u8; 32];
        if getrandom::fill(&mut secret).is_err() {
            return false;
        }
        let token: String = secret.iter().map(|byte| format!("{byte:02x}")).collect();
        let digest: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        let session = self.engine.new_session(self, cwd, digest);
        tracing::debug!(
            entry = self.info.entry,
            "a session was given the delegation tools"
        );
        let entry = match (way, self.engine.bridge.get()) {
            (Way::Stdio, Some(bridge)) => {
                // Version 1 writes a stdio server with no `type`; version 2 names it.
                let mut entry = json!({
                    "name": SERVER_NAME,
                    "command": bridge.program,
                    "args": bridge.args,
                    "env": [
                        {"name": bridge::URL_VAR, "value": url},
                        {"name": bridge::TOKEN_VAR, "value": token},
                    ],
                });
                if state.version >= 2 {
                    entry["type"] = json!("stdio");
                }
                entry
            }
            _ => json!({
                "type": "http",
                "name": SERVER_NAME,
                "url": url,
                "headers": [{"name": "Authorization", "value": format!("Bearer {token}")}],
            }),
        };
        let Some(params) = value.get_mut("params").and_then(Value::as_object_mut) else {
            return false;
        };
        let servers = params.entry("mcpServers").or_insert_with(|| json!([]));
        let Some(servers) = servers.as_array_mut() else {
            return false;
        };
        servers.retain(|server| server.get("name").and_then(Value::as_str) != Some(SERVER_NAME));
        servers.push(entry);
        state.sessions.push(Arc::clone(&session));
        if state.pending.len() < MAX_PENDING {
            state.pending.insert(
                id,
                Pending::Open {
                    session: Some(Arc::clone(&session)),
                    known: known.clone(),
                },
            );
        }
        drop(state);
        if let Some(known) = known {
            self.engine.bind(&session, &known);
        }
        true
    }

    /// A message from the agent, before it goes to the client.
    pub(crate) fn agent_said(&self, message: &[u8]) {
        let marked =
            contains(message, b"current_mode_update") || contains(message, b"config_option_update");
        if !marked && lock(&self.state).pending.is_empty() {
            return;
        }
        let Ok(value) = serde_json::from_slice::<Value>(message) else {
            return;
        };
        if value.get("method").is_some() {
            if marked {
                self.mode_notice(&value);
            }
            return;
        }
        let Some(id) = value.get("id").map(Value::to_string) else {
            return;
        };
        let pending = lock(&self.state).pending.remove(&id);
        let ok = value.get("result").filter(|_| value.get("error").is_none());
        match (pending, ok) {
            (Some(Pending::Initialize), Some(result)) => {
                let takes = |capability: Option<&Value>| {
                    capability.is_some_and(|said| said == &json!(true) || said.is_object())
                };
                let version = result
                    .get("protocolVersion")
                    .and_then(Value::as_u64)
                    .unwrap_or(1);
                let mut state = lock(&self.state);
                state.version = version;
                state.http = takes(
                    result
                        .pointer("/agentCapabilities/mcpCapabilities/http")
                        .or_else(|| result.pointer("/capabilities/session/mcp/http")),
                );
                // Version 1 has stdio in its baseline; version 2 says it.
                state.stdio =
                    version < 2 || takes(result.pointer("/capabilities/session/mcp/stdio"));
            }
            (
                Some(Pending::Open {
                    session: Some(session),
                    known,
                }),
                Some(result),
            ) => {
                let named = known.or_else(|| {
                    result
                        .get("sessionId")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                });
                *lock(&session.modes) = privilege::modes_of(result);
                match named {
                    Some(named) => self.engine.bind(&session, &named),
                    None => self.revoke(&session),
                }
            }
            (
                Some(Pending::Open {
                    session: Some(session),
                    ..
                }),
                None,
            ) => self.revoke(&session),
            (Some(Pending::Mode { session, mode }), Some(_)) => {
                if let Some(session) = self.session(&session) {
                    lock(&session.modes).current = Some(mode);
                }
            }
            (
                Some(Pending::Option {
                    session,
                    id,
                    value: chosen,
                }),
                Some(result),
            ) => {
                if let Some(session) = self.session(&session) {
                    let mut modes = lock(&session.modes);
                    if result.get("configOptions").is_some() {
                        let fresh = privilege::modes_of(result);
                        if fresh.option.is_some() {
                            modes.current = fresh.current;
                        }
                    } else if modes.option.as_deref() == Some(id.as_str()) {
                        modes.current = Some(chosen);
                    }
                }
            }
            _ => {}
        }
    }

    fn mode_notice(&self, value: &Value) {
        let Some(id) = value.pointer("/params/sessionId").and_then(Value::as_str) else {
            return;
        };
        let Some(session) = self.session(id) else {
            return;
        };
        let update = &value["params"]["update"];
        let mut modes = lock(&session.modes);
        match update.get("sessionUpdate").and_then(Value::as_str) {
            Some("current_mode_update") => {
                if let Some(mode) = update.get("currentModeId").and_then(Value::as_str) {
                    modes.current = Some(mode.to_owned());
                }
            }
            Some("config_option_update") => {
                let fresh = privilege::modes_of(&json!({"configOptions": update["configOptions"]}));
                if fresh.option.is_some() && fresh.current.is_some() {
                    modes.current = fresh.current;
                }
            }
            _ => {}
        }
    }

    fn revoke(&self, session: &Arc<Session>) {
        self.engine.revoke(session);
        lock(&self.state)
            .sessions
            .retain(|held| !Arc::ptr_eq(held, session));
    }

    /// Asks the person, on this connection, a question a task's agent asked: `params` as the task's agent sent them, moved to the
    /// parent's session and the task's tool call id, with `_meta` naming the task. The person's answer, or `None` when `stop`
    /// says to give up or the connection ends first.
    pub(crate) fn ask(
        &self,
        session: &str,
        task: &Value,
        mut params: Value,
        stop: &dyn Fn() -> bool,
    ) -> Option<Value> {
        let number = self.asked.fetch_add(1, Ordering::SeqCst) + 1;
        let mut nonce = [0_u8; 8];
        let _ = getrandom::fill(&mut nonce);
        let hex: String = nonce.iter().map(|byte| format!("{byte:02x}")).collect();
        let request_id = format!("canoa-delegate-{number}-{hex}");
        let object = params.as_object_mut()?;
        object.insert("sessionId".to_owned(), json!(session));
        if let Some(call) = object.get_mut("toolCall").and_then(Value::as_object_mut) {
            let original = call
                .get("toolCallId")
                .and_then(Value::as_str)
                .unwrap_or("call")
                .to_owned();
            let task_id = task["taskId"].as_str().unwrap_or("");
            call.insert(
                "toolCallId".to_owned(),
                json!(format!("{task_id}:{original}")),
            );
        }
        object.insert("_meta".to_owned(), json!({ META: task }));
        let request = json!({
            "jsonrpc": "2.0", "id": request_id, "method": "session/request_permission", "params": params,
        });
        let waiting = Arc::new(Waiting {
            answer: Mutex::new(None),
            ready: Condvar::new(),
        });
        lock(&self.answers).insert(request_id.clone(), Arc::clone(&waiting));
        if self.is_closed() {
            lock(&self.answers).remove(&request_id);
            return None;
        }
        lock(&self.outbox).push_back(serde_json::to_vec(&request).ok()?);
        let mut held = lock(&waiting.answer);
        let answer = loop {
            if let Some(answer) = held.take() {
                break Some(answer);
            }
            if stop() || self.is_closed() {
                break None;
            }
            held = waiting
                .ready
                .wait_timeout(held, Duration::from_millis(100))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        };
        drop(held);
        lock(&self.answers).remove(&request_id);
        answer
    }

    /// The client's answer to one of the host's questions, if `id` is one. Whether it was.
    fn answer(&self, id: &str, value: &Value) -> bool {
        let Some(waiting) = lock(&self.answers).get(id).cloned() else {
            return false;
        };
        let result = value.get("result").filter(|_| value.get("error").is_none());
        *lock(&waiting.answer) = result.cloned().or_else(|| Some(Value::Null));
        waiting.ready.notify_all();
        true
    }

    /// Ends the connection's part: no bearer of it is valid after, and every task of its sessions ends.
    pub(crate) fn close(&self) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        let sessions: Vec<Arc<Session>> = core::mem::take(&mut lock(&self.state).sessions);
        for session in sessions {
            self.engine.revoke(&session);
            session.stop_children();
        }
        for waiting in lock(&self.answers).values() {
            waiting.ready.notify_all();
        }
    }
}

impl Engine {
    fn new_session(&self, conn: &Arc<Conn>, cwd: PathBuf, digest: [u8; 32]) -> Arc<Session> {
        let per_session = self.config.max_children.saturating_mul(4).max(1);
        let session = Arc::new_cyclic(|weak: &Weak<Session>| {
            let hook = weak.clone();
            Session {
                conn: Arc::downgrade(conn),
                conn_id: conn.id,
                cwd,
                digest,
                id: Mutex::new(None),
                modes: Mutex::new(Modes::default()),
                store: TaskStore::builder()
                    .max_tasks(per_session)
                    .max_tasks_per_owner(per_session)
                    // A byte of text is at most six of JSON (`\u00XX`).
                    .max_history_bytes(
                        self.config
                            .task_bytes
                            .saturating_mul(6)
                            .saturating_add(4096),
                    )
                    .max_artifact_bytes(
                        self.config
                            .history_bytes
                            .saturating_mul(6)
                            .saturating_add(4096),
                    )
                    .cancel_hook(move |task| match hook.upgrade() {
                        Some(session) => child::cancel(&session, &task.id),
                        None => Ok(()),
                    })
                    .build(),
                children: Mutex::new(HashMap::new()),
                live: AtomicUsize::new(0),
                calls: AtomicUsize::new(0),
            }
        });
        lock(&self.state)
            .tokens
            .insert(digest, Arc::clone(&session));
        session
    }

    /// Names a session, which is what makes its bearer able to delegate.
    fn bind(&self, session: &Arc<Session>, id: &str) {
        *lock(&session.id) = Some(id.to_owned());
        if let Some(peer) = session.peer() {
            let mut state = lock(&self.state);
            if state.tokens.contains_key(&session.digest)
                && let Some(old) = state.sessions.insert(peer, Arc::clone(session))
                && !Arc::ptr_eq(&old, session)
            {
                // A session opened again under the same id takes the place of the one before it.
                state.tokens.remove(&old.digest);
                drop(state);
                old.stop_children();
            }
        }
    }

    fn revoke(&self, session: &Arc<Session>) {
        let mut state = lock(&self.state);
        state.tokens.remove(&session.digest);
        if let Some(peer) = session.peer()
            && state
                .sessions
                .get(&peer)
                .is_some_and(|held| Arc::ptr_eq(held, session))
        {
            state.sessions.remove(&peer);
        }
    }
}

/// The delegation of a running host: the engine, and the loopback server that gives agents its tools.
pub(crate) struct Delegation {
    engine: Arc<Engine>,
    server: Mutex<Option<server::Listener>>,
}

impl Delegation {
    /// Starts the tool server for `config`; `None` when delegation is off or its server could not listen.
    pub(crate) fn start(config: &DelegateConfig, bridge: BridgeCommand) -> Option<Self> {
        if !config.enabled {
            tracing::info!("non-default: host.delegate.enabled is false, default true");
            return None;
        }
        let defaults = DelegateConfig::default();
        for (setting, value, default) in [
            ("max_children", config.max_children, defaults.max_children),
            ("max_running", config.max_running, defaults.max_running),
            ("max_depth", config.max_depth, defaults.max_depth),
            ("task_bytes", config.task_bytes, defaults.task_bytes),
            ("result_bytes", config.result_bytes, defaults.result_bytes),
            (
                "history_bytes",
                config.history_bytes,
                defaults.history_bytes,
            ),
        ] {
            if value != default {
                tracing::info!(
                    "non-default: host.delegate.{setting} is {value}, default {default}"
                );
            }
        }
        if let Some(sources) = &config.sources {
            tracing::info!(
                "non-default: host.delegate.sources is {}, default every source a connection may use",
                sources.join(", ")
            );
        }
        let engine = Engine::new(config.clone());
        let _ = engine.bridge.set(bridge);
        match server::Listener::start(&engine) {
            Ok(listener) => {
                let url = format!("http://127.0.0.1:{}/mcp", listener.addr().port());
                let _ = engine.url.set(url);
                Some(Self {
                    engine,
                    server: Mutex::new(Some(listener)),
                })
            }
            Err(error) => {
                tracing::warn!("delegation is off: its tool server could not listen: {error}");
                None
            }
        }
    }

    /// Gives the delegation the host its tasks run on.
    pub(crate) fn attach(&self, host: Arc<dyn Host>) {
        let _ = self.engine.host.set(host);
    }

    /// A connection that may delegate.
    pub(crate) fn connection(&self, info: ConnInfo) -> Arc<Conn> {
        self.engine.connect(info)
    }

    /// Stops the tool server, revokes every bearer and ends every task.
    pub(crate) fn stop(&self) {
        self.engine.stop();
        lock(&self.server).take();
    }
}

impl Drop for Delegation {
    fn drop(&mut self) {
        self.stop();
    }
}

/// How long a call may wait for a task to leave a state it is in, at most.
pub(crate) const MAX_WAIT: Duration = Duration::from_secs(300);

#[cfg(test)]
mod tests;
