//! The `canoa-store` MCP server: what lets an agent search the Store and read one entry (`store_search`, `store_inspect`), and,
//! only with the person's approval of each change, install, update, remove and roll back (`store_install`, `store_update`,
//! `store_remove`, `store_rollback`, `loadout_update`).
//!
//! A change is put to the person as a `session/request_permission` on the client the session belongs to, showing what would be
//! applied and the digest it is bound to; nothing is applied until their answer comes back from that client, and what is applied
//! is what was shown. A session the person put in a mode that asks for nothing is not asked, as for any other tool.
//!
//! It is offered the way `canoa-delegate` is. A client asks for it in `initialize` (`_meta["dev.boriken.canoa/store"]` true), the
//! host adds one server to each session request, over loopback HTTP behind a bearer minted for that session or, for an agent
//! that takes only stdio, through `PROGRAM mcp-bridge` with the bearer in its environment, and the bearer ends with the
//! connection. A connection gets the server only when the key behind it holds the grants `store/index` names for what it would
//! read (`overlay.read` for catalog entries, `store.programs` for programs); a task another agent handed out, which has no
//! person at its end, gets none; `[host.store] tools = false` turns it off.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "locks, threads and the clock are std's"
)]

mod approve;
mod change;
mod http;
mod tools;

use core::time::Duration;
use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::time::Instant;

use coqui_transport::{Channel, Failure};
use serde_json::{Value, json};

pub(crate) use self::approve::{Approval, Card};
use self::approve::{Held, Questions};
pub(crate) use self::change::{Action, Change, Prepared};
use crate::control::Grant;
use crate::delegate::BridgeCommand;
use crate::delegate::privilege::{Modes, modes_of};

/// How often a read looks up to send what the host added.
const POLL: Duration = Duration::from_millis(50);

/// The `_meta` key of `initialize` that asks for the store tools.
pub(crate) const META: &str = "dev.boriken.canoa/store";
/// The name of the MCP server the host adds to a session.
pub(crate) const SERVER_NAME: &str = "canoa-store";
/// The built-in skill that says when to use the tools, delivered with a session that has them.
pub(crate) const SKILL_NAME: &str = "canoa-store";
const SKILL: &str = include_str!("mcp/canoa-store.md");
/// Requests the host remembers an answer for.
const MAX_PENDING: usize = 64;
/// Sessions of one connection that hold a bearer.
const MAX_SESSIONS: usize = 256;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The files of the built-in skill, by path, for a session's skills.
pub(crate) fn skill_files() -> std::collections::BTreeMap<String, Vec<u8>> {
    [("SKILL.md".to_owned(), SKILL.as_bytes().to_vec())].into()
}

/// What the tools read of the host.
pub(crate) trait Backend: Send + Sync {
    /// `store/index` as a key holding `grants` would be answered; `refresh` asks the sources again, which the key's own
    /// grants decide.
    fn index(&self, grants: &BTreeSet<Grant>, refresh: bool) -> Result<Value, String>;
    /// A program store's plan for `action`, which downloads and changes nothing and is kept for nobody.
    fn plan(&self, provider: &str, action: &str, version: Option<&str>) -> Result<Value, String>;
    /// The overlays and skills canoa.cfg names.
    fn installed(&self) -> Installed;
    /// Works out `change` for the session `session`, whose key holds `grants`, and keeps what it would apply on the host,
    /// under that session alone. Nothing is installed, removed or written by it.
    fn prepare(
        &self,
        session: &str,
        grants: &BTreeSet<Grant>,
        change: &Change,
    ) -> Result<Prepared, String>;
    /// Applies what [`Backend::prepare`] kept, if it is still what `prepared.digest` says; otherwise refuses and changes nothing.
    fn commit(
        &self,
        session: &str,
        grants: &BTreeSet<Grant>,
        prepared: &Prepared,
        cancel: &Arc<AtomicBool>,
    ) -> Result<Value, String>;
    /// Gives up what [`Backend::prepare`] kept, when the change is not going to be applied.
    fn discard(&self, session: &str, prepared: &Prepared);
}

/// Another server a connection's sessions are given beside `canoa-store`: its own tools, answered the way the store's are, with the
/// same way to ask the person about a change. It is given to every session of a key it is offered to, whether or not the client
/// asked for the store tools.
pub(crate) trait Extra: Send + Sync {
    /// The name of the MCP server the host adds to a session.
    fn server(&self) -> &str;
    /// Whether a connection whose key holds `grants` is given the server.
    fn offered(&self, grants: &BTreeSet<Grant>) -> bool;
    /// The tools and their input, as `tools/list` answers.
    fn list(&self) -> Value;
    /// Runs the tool `name` and answers as `tools/call` does.
    fn call(&self, access: &Access, name: &str, args: &Value) -> Value;
}

/// What is installed on the host, by name.
#[derive(Clone, Debug, Default)]
pub(crate) struct Installed {
    pub(crate) overlays: BTreeSet<String>,
    pub(crate) skills: BTreeSet<String>,
}

/// What one bearer may do: the grants of the key behind it, kept to those the tools use, and the session it was minted for.
pub(crate) struct Access {
    pub(crate) grants: BTreeSet<Grant>,
    calls: AtomicUsize,
    /// The session the agent named when it opened, which the bearer's changes are asked about and bound to.
    session: Mutex<Option<String>>,
    link: Weak<Link>,
    /// One change at a time for a session, so an agent cannot pile questions on the person.
    changing: Mutex<()>,
    /// Set when the session or the connection ends, so a change under way stops.
    pub(crate) cancel: Arc<AtomicBool>,
    /// Changes of this session that ended with nothing to show the person.
    unshown: AtomicUsize,
    /// The server this bearer opens when it is not the store's.
    extra: Option<Arc<dyn Extra>>,
}

impl Access {
    /// The session this bearer belongs to, once the agent has named it.
    pub(crate) fn session(&self) -> Option<String> {
        lock(&self.session).clone()
    }

    /// The right to make one change now, or `None` while another of the session's waits for the person.
    pub(crate) fn begin_change(&self) -> Option<MutexGuard<'_, ()>> {
        match self.changing.try_lock() {
            Ok(guard) => Some(guard),
            Err(std::sync::TryLockError::Poisoned(error)) => Some(error.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) => None,
        }
    }

    /// Puts `card` to the person on the client this session belongs to. Whether they allowed it; an error says why nobody could
    /// be asked, and then nothing may be applied.
    pub(crate) fn approve(&self, card: &Card, ask_anyway: bool) -> Result<Approval, String> {
        let session = self
            .session()
            .ok_or("the host does not know this session yet, so no one can be asked")?;
        let link = self
            .link
            .upgrade()
            .ok_or("the person's app is gone, so no one can be asked")?;
        link.approve(&session, card, ask_anyway)
    }

    /// Says something in the session, in the person's view.
    pub(crate) fn note(&self, text: &str) {
        if let (Some(session), Some(link)) = (self.session(), self.link.upgrade()) {
            link.note(&session, text);
        }
    }

    /// One more change ended with nothing to show the person.
    pub(crate) fn spend(&self) {
        self.unshown.fetch_add(1, Ordering::SeqCst);
    }

    /// Whether the session has used up its changes that end with nothing to show.
    pub(crate) fn spent(&self) -> bool {
        self.unshown.load(Ordering::SeqCst) >= change::MAX_UNSHOWN
    }
}

/// The bearers in force, by the digest of the token: the token itself is never kept.
pub(crate) type Tokens = crate::loopback::Tokens<Access>;

/// What the host holds for every connection: whether the tools are on, what answers them, and the server.
pub(crate) struct Hub {
    enabled: bool,
    backend: OnceLock<Arc<dyn Backend>>,
    extras: Mutex<Vec<Arc<dyn Extra>>>,
    /// What runs the stdio bridge, for an agent that takes no HTTP MCP server.
    bridge: BridgeCommand,
    tokens: Arc<Tokens>,
    server: Mutex<Option<http::Listener>>,
}

impl Hub {
    pub(crate) fn new(enabled: bool, bridge: BridgeCommand) -> Arc<Self> {
        Arc::new(Self {
            enabled,
            backend: OnceLock::new(),
            extras: Mutex::new(Vec::new()),
            bridge,
            tokens: Arc::new(Tokens::default()),
            server: Mutex::new(None),
        })
    }

    /// Says what answers the tools. Once.
    pub(crate) fn serve_with(&self, backend: Arc<dyn Backend>) {
        let _ = self.backend.set(backend);
    }

    /// Adds a server to give every connection it is offered to.
    pub(crate) fn serve_extra(&self, extra: Arc<dyn Extra>) {
        lock(&self.extras).push(extra);
    }

    fn backend(&self) -> Option<Arc<dyn Backend>> {
        self.backend.get().cloned()
    }

    /// The server's address, started the first time it is wanted.
    fn url(self: &Arc<Self>) -> Option<String> {
        let mut server = lock(&self.server);
        if server.is_none() {
            *server = http::start(self).ok();
        }
        server
            .as_ref()
            .map(|listener| format!("http://{}/mcp", listener.addr()))
    }

    /// The store tools of one connection whose key holds `grants`, or `None` when it is given none: they are off, it is a task
    /// another agent handed out (`grants` is `None`), or the key reads neither half of the index.
    pub(crate) fn for_connection(
        self: &Arc<Self>,
        grants: Option<&BTreeSet<Grant>>,
    ) -> Option<Arc<Link>> {
        let grants = grants?;
        let extras: Vec<Arc<dyn Extra>> = lock(&self.extras)
            .iter()
            .filter(|extra| extra.offered(grants))
            .cloned()
            .collect();
        let store = self.enabled && self.backend.get().is_some();
        if !store && extras.is_empty() {
            return None;
        }
        let kept: BTreeSet<Grant> = grants
            .iter()
            .copied()
            .filter(|grant| {
                matches!(
                    grant,
                    Grant::OverlayRead
                        | Grant::OverlayWrite
                        | Grant::StorePrograms
                        | Grant::StoreRead
                        | Grant::ConfigWrite
                        | Grant::ConfigWriteSessions
                )
            })
            .collect();
        let store =
            store && (kept.contains(&Grant::OverlayRead) || kept.contains(&Grant::StorePrograms));
        if !store && extras.is_empty() {
            return None;
        }
        Some(Arc::new(Link {
            hub: Arc::clone(self),
            grants: kept,
            store,
            extras,
            state: Mutex::new(State::default()),
            questions: Questions::default(),
            closed: AtomicBool::new(false),
        }))
    }
}

/// Whether an `initialize` asks for the store tools.
pub(crate) fn asked_in(message: &[u8]) -> bool {
    if !contains(message, b"initialize") || !contains(message, META.as_bytes()) {
        return false;
    }
    let Ok(value) = serde_json::from_slice::<Value>(message) else {
        return false;
    };
    value.get("method") == Some(&json!("initialize"))
        && value
            .pointer("/params/_meta")
            .and_then(|meta| meta.get(META))
            == Some(&json!(true))
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// A request of the client's the host remembers until the agent answers it.
enum Pending {
    Initialize,
    Open {
        digests: Vec<[u8; 32]>,
        /// The session the request names, for a load or a resume; a new session is named by the agent's answer.
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
struct State {
    asked: bool,
    /// The agent takes MCP servers over HTTP.
    http: bool,
    /// The ACP version the agent answered `initialize` in when it takes stdio servers, else 0. Version 1 has them in its
    /// baseline.
    stdio: u64,
    pending: HashMap<String, Pending>,
    digests: Vec<[u8; 32]>,
    sessions: HashMap<String, Arc<Held>>,
}

/// One client connection that may be given the store tools.
pub(crate) struct Link {
    hub: Arc<Hub>,
    grants: BTreeSet<Grant>,
    /// Whether the connection is given the store's own tools (when the client asks for them).
    store: bool,
    extras: Vec<Arc<dyn Extra>>,
    state: Mutex<State>,
    questions: Questions,
    closed: AtomicBool,
}

impl Link {
    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// Whether the connection's key may use the store's own tools.
    pub(crate) fn serves_store(&self) -> bool {
        self.store
    }

    /// Whether the connection is given the extra server `name`.
    pub(crate) fn offers(&self, name: &str) -> bool {
        self.extras.iter().any(|extra| extra.server() == name)
    }

    /// What the host added to the stream to the client.
    fn drain(&self) -> Vec<Vec<u8>> {
        self.questions.drain()
    }

    /// Ends every bearer of the connection and lets a question that waits go.
    pub(crate) fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        let mut state = lock(&self.state);
        for digest in state.digests.drain(..) {
            self.revoke_token(&digest);
        }
        state.sessions.clear();
        drop(state);
        self.questions.wake();
    }

    /// Reads the client's `initialize` before the scope removes its `_meta`.
    fn peek(&self, message: &[u8]) {
        if contains(message, META.as_bytes()) {
            lock(&self.state).asked = asked_in(message);
        }
    }

    /// A message of the client's as it came off the wire: whether it was the person's answer to a question the host asked, which
    /// then goes no further.
    fn taken_from_client(&self, message: &[u8]) -> bool {
        // JSON may escape any character, so a message with a backslash is read whatever its bytes say.
        if !(contains(message, approve::ID_PREFIX.as_bytes()) || message.contains(&b'\\')) {
            return false;
        }
        serde_json::from_slice::<Value>(message).is_ok_and(|value| self.questions.answered(&value))
    }

    fn remember(&self, id: String, pending: Pending) {
        let mut state = lock(&self.state);
        if state.pending.len() < MAX_PENDING {
            state.pending.insert(id, pending);
        }
    }

    fn held(&self, session: &str) -> Option<Arc<Held>> {
        lock(&self.state).sessions.get(session).cloned()
    }

    /// Asks the person about a change of `session`, or lets it through when the person put the session in a mode that asks for
    /// nothing. Whether it may be applied.
    fn approve(&self, session: &str, card: &Card, ask_anyway: bool) -> Result<Approval, String> {
        let held = self
            .held(session)
            .ok_or("the host does not hold this session, so no one can be asked")?;
        if held.bypass.load(Ordering::SeqCst) && !ask_anyway {
            tracing::info!(
                session,
                "a store change was allowed by the session's mode: {}",
                card.title
            );
            return Ok(Approval::Silent);
        }
        tracing::info!(
            session,
            "a store change was put to the person: {}",
            card.title
        );
        let allowed = self.questions.ask(session, card, &|| self.is_closed())?;
        tracing::info!(session, allowed, "a store change was answered");
        Ok(if allowed {
            Approval::Allowed
        } else {
            Approval::Declined
        })
    }

    /// A message in the session, from the host: a line of the agent's own stream is never forged, this is the host speaking to the
    /// client.
    fn note(&self, session: &str, text: &str) {
        let update = json!({"jsonrpc": "2.0", "method": "session/update", "params": {
            "sessionId": session,
            "update": {"sessionUpdate": "agent_message_chunk",
                "content": {"type": "text", "text": format!("\n{text}\n")}},
        }});
        self.questions.push(update.to_string().into_bytes());
    }

    /// A message from the client after the scope has checked it. A session request gets the host's entry added.
    fn client_said(self: &Arc<Self>, message: Vec<u8>) -> Vec<u8> {
        // JSON may escape any character, so a message with a backslash is read whatever its bytes say.
        if !(contains(&message, b"session/")
            || contains(&message, b"initialize")
            || message.contains(&b'\\'))
        {
            return message;
        }
        let Ok(mut value) = serde_json::from_slice::<Value>(&message) else {
            return message;
        };
        let (Some(method), Some(id)) = (
            value
                .get("method")
                .and_then(Value::as_str)
                .map(str::to_owned),
            value.get("id").map(Value::to_string),
        ) else {
            return message;
        };
        match method.as_str() {
            "initialize" => self.remember(id, Pending::Initialize),
            "session/new" | "session/load" | "session/resume" => {
                if self.open(&mut value, &method, id) {
                    return serde_json::to_vec(&value).unwrap_or(message);
                }
            }
            "session/set_mode" => {
                let params = &value["params"];
                if let Some(held) = params["sessionId"].as_str().and_then(|id| self.held(id)) {
                    // Whatever the client asks, the session no longer counts as one that asks for nothing until the agent
                    // says it took a mode that does. Nothing here waits on room to remember the request.
                    held.bypass.store(false, Ordering::SeqCst);
                }
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
            "session/set_config_option" => {
                let params = &value["params"];
                if let Some(held) = params["sessionId"].as_str().and_then(|id| self.held(id))
                    && held.modes.option.as_deref() == params["configId"].as_str()
                {
                    held.bypass.store(false, Ordering::SeqCst);
                }
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
            "session/close" => {
                if let Some(session) = value.pointer("/params/sessionId").and_then(Value::as_str) {
                    self.end(session);
                }
            }
            _ => {}
        }
        message
    }

    /// Adds the host's MCP servers, each with a bearer of its own, to a session request: the store's when the client asked for
    /// it and the connection's key may use it, and every extra server offered to the key. Whether the request was changed.
    fn open(self: &Arc<Self>, value: &mut Value, method: &str, id: String) -> bool {
        let known = (method != "session/new")
            .then(|| value.pointer("/params/sessionId").and_then(Value::as_str))
            .flatten()
            .map(str::to_owned);
        let (asked, http, stdio, count) = {
            let state = lock(&self.state);
            (state.asked, state.http, state.stdio, state.digests.len())
        };
        let mut wanted: Vec<(&str, Option<Arc<dyn Extra>>)> = Vec::new();
        if self.store && asked {
            wanted.push((SERVER_NAME, None));
        }
        wanted.extend(
            self.extras
                .iter()
                .map(|extra| (extra.server(), Some(Arc::clone(extra)))),
        );
        if wanted.is_empty()
            || self.is_closed()
            || count + wanted.len() > MAX_SESSIONS
            || !(http || stdio > 0)
        {
            return false;
        }
        let Some(url) = self.hub.url() else {
            return false;
        };
        let mut made = Vec::new();
        for (server, extra) in wanted {
            let Some((token, digest)) = crate::loopback::mint_bearer() else {
                return false;
            };
            let bridge = (!http).then_some(&self.hub.bridge);
            let entry = crate::loopback::server_entry(server, &url, &token, bridge, stdio);
            made.push((server, extra, digest, entry));
        }
        let Some(servers) = value
            .get_mut("params")
            .and_then(Value::as_object_mut)
            .map(|params| params.entry("mcpServers").or_insert_with(|| json!([])))
            .and_then(Value::as_array_mut)
        else {
            return false;
        };
        let mut digests = Vec::new();
        for (server, extra, digest, entry) in made {
            servers.retain(|have| have.get("name").and_then(Value::as_str) != Some(server));
            servers.push(entry);
            // A load or a resume names its session; a new one is named by the agent's answer.
            let access = Arc::new(Access {
                grants: self.grants.clone(),
                calls: AtomicUsize::new(0),
                session: Mutex::new(known.clone()),
                link: Arc::downgrade(self),
                changing: Mutex::new(()),
                cancel: Arc::new(AtomicBool::new(false)),
                unshown: AtomicUsize::new(0),
                extra,
            });
            self.hub.tokens.add(digest, access);
            digests.push(digest);
        }
        lock(&self.state).digests.extend(digests.iter().copied());
        self.remember(id, Pending::Open { digests, known });
        tracing::debug!("a session was given the store tools");
        true
    }

    /// A message from the agent, before it goes to the client.
    fn agent_said(&self, message: &[u8]) {
        let marked =
            contains(message, b"current_mode_update") || contains(message, b"config_option_update");
        if !marked && lock(&self.state).pending.is_empty() && !message.contains(&b'\\') {
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
                state.http = takes(
                    result
                        .pointer("/agentCapabilities/mcpCapabilities/http")
                        .or_else(|| result.pointer("/capabilities/session/mcp/http")),
                );
                // Version 1 has stdio in its baseline; version 2 says it.
                let stdio = version < 2 || takes(result.pointer("/capabilities/session/mcp/stdio"));
                state.stdio = if stdio { version } else { 0 };
            }
            (Some(Pending::Open { digests, known, .. }), Some(result)) => {
                let named = known.or_else(|| {
                    result
                        .get("sessionId")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                });
                match named {
                    Some(named) => self.bind(&digests, &named, modes_of(result)),
                    None => digests.iter().for_each(|digest| self.revoke(digest)),
                }
            }
            // A session the agent did not open has no use for its bearer.
            (Some(Pending::Open { digests, .. }), None) => {
                digests.iter().for_each(|digest| self.revoke(digest));
            }
            (Some(Pending::Mode { session, mode }), Some(_)) => {
                if let Some(held) = self.held(&session) {
                    held.bypass.store(
                        crate::secrets::is_bypass(&mode) && held.lists(&mode),
                        Ordering::SeqCst,
                    );
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
                if let Some(held) = self.held(&session) {
                    if held.modes.option.as_deref() == Some(id.as_str()) {
                        held.bypass.store(
                            crate::secrets::is_bypass(&chosen) && held.lists(&chosen),
                            Ordering::SeqCst,
                        );
                    }
                    // An echo of the options can only lower it.
                    if let Some(current) = modes_of(result)
                        .current
                        .filter(|_| result.get("configOptions").is_some())
                    {
                        held.reported(&current);
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
        let Some(held) = self.held(id) else {
            return;
        };
        let update = &value["params"]["update"];
        match update.get("sessionUpdate").and_then(Value::as_str) {
            Some("current_mode_update") => {
                if let Some(mode) = update.get("currentModeId").and_then(Value::as_str) {
                    held.reported(mode);
                }
            }
            Some("config_option_update") => {
                if let Some(current) = modes_of(update).current {
                    held.reported(&current);
                }
            }
            _ => {}
        }
    }

    fn revoke_token(&self, digest: &[u8; 32]) {
        if let Some(access) = self.hub.tokens.find(digest) {
            access.cancel.store(true, Ordering::SeqCst);
        }
        self.hub.tokens.remove(digest);
    }

    fn revoke(&self, digest: &[u8; 32]) {
        self.revoke_token(digest);
        lock(&self.state).digests.retain(|held| held != digest);
    }

    /// The session the agent has named is given its bearer's name. A session named twice keeps the newer bearer only.
    fn bind(&self, digests: &[[u8; 32]], session: &str, modes: Modes) {
        self.end(session);
        let mut bound = Vec::new();
        for digest in digests {
            if let Some(access) = self.hub.tokens.find(digest) {
                *lock(&access.session) = Some(session.to_owned());
                bound.push(*digest);
            }
        }
        if bound.is_empty() {
            return;
        }
        lock(&self.state).sessions.insert(
            session.to_owned(),
            Arc::new(Held {
                digests: bound,
                modes,
                bypass: AtomicBool::new(false),
            }),
        );
    }

    /// A session ended: its bearer is revoked, so nothing more of it is asked or applied.
    fn end(&self, session: &str) {
        let held = lock(&self.state).sessions.remove(session);
        if let Some(held) = held {
            held.digests.iter().for_each(|digest| self.revoke(digest));
        }
    }
}

/// A channel on the client's own stream that reads its `initialize` for the opt-in, sends the questions the host puts to the
/// person, and keeps the person's answers from going any further. Everything else passes on.
pub(crate) struct StorePeek<'a, C: Channel + ?Sized> {
    inner: &'a C,
    link: Option<Arc<Link>>,
}

impl<'a, C: Channel + ?Sized> StorePeek<'a, C> {
    pub(crate) fn new(inner: &'a C, link: Option<&Arc<Link>>) -> Self {
        Self {
            inner,
            link: link.map(Arc::clone),
        }
    }

    /// Sends what the host added to the client's stream. It goes on the wire as it is: the scope is for what a client and an
    /// agent say to each other, and these are the host's own words.
    fn flush(&self) -> Result<(), Failure> {
        if let Some(link) = &self.link {
            for message in link.drain() {
                self.inner.send(&message)?;
            }
        }
        Ok(())
    }
}

impl<C: Channel + ?Sized> Channel for StorePeek<'_, C> {
    fn send(&self, message: &[u8]) -> Result<(), Failure> {
        self.inner.send(message)?;
        self.flush()
    }

    fn receive(&self, timeout: Option<Duration>) -> Result<Option<Vec<u8>>, Failure> {
        let Some(link) = &self.link else {
            return self.inner.receive(timeout);
        };
        let until = timeout.map(|timeout| Instant::now() + timeout);
        loop {
            self.flush()?;
            let slice = match until {
                Some(until) => {
                    let left = until.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        return Err(Failure::Timeout);
                    }
                    left.min(POLL)
                }
                None => POLL,
            };
            match self.inner.receive(Some(slice)) {
                Ok(Some(message)) => {
                    link.peek(&message);
                    if !link.taken_from_client(&message) {
                        return Ok(Some(message));
                    }
                }
                Ok(None) => return Ok(None),
                Err(Failure::Timeout) => {}
                Err(failure) => return Err(failure),
            }
        }
    }

    fn close(&self) {
        self.inner.close();
    }

    fn set_max_message(&self, bytes: usize) {
        self.inner.set_max_message(bytes);
    }
}

/// A channel that gives a session the store server, sitting outside the scope and the profile where a session request has been
/// checked, and that ends the bearers with the connection.
pub(crate) struct StoreLayer<'a, C: Channel + ?Sized> {
    inner: &'a C,
    link: Option<Arc<Link>>,
}

impl<'a, C: Channel + ?Sized> StoreLayer<'a, C> {
    pub(crate) fn new(inner: &'a C, link: Option<&Arc<Link>>) -> Self {
        Self {
            inner,
            link: link.map(Arc::clone),
        }
    }
}

impl<C: Channel + ?Sized> Drop for StoreLayer<'_, C> {
    fn drop(&mut self) {
        if let Some(link) = &self.link {
            link.close();
        }
    }
}

impl<C: Channel + ?Sized> Channel for StoreLayer<'_, C> {
    fn send(&self, message: &[u8]) -> Result<(), Failure> {
        if let Some(link) = &self.link {
            link.agent_said(message);
        }
        self.inner.send(message)
    }

    fn receive(&self, timeout: Option<Duration>) -> Result<Option<Vec<u8>>, Failure> {
        let received = self.inner.receive(timeout)?;
        Ok(match (received, &self.link) {
            (Some(message), Some(link)) => Some(link.client_said(message)),
            (received, _) => received,
        })
    }

    fn close(&self) {
        if let Some(link) = &self.link {
            link.close();
        }
        self.inner.close();
    }

    fn set_max_message(&self, bytes: usize) {
        self.inner.set_max_message(bytes);
    }
}

#[cfg(test)]
mod extra_tests;
#[cfg(test)]
mod tests;
