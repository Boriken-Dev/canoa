//! One client connection's secret prompts: whether its app offered them, the sessions that were given the tools, what the host
//! asks the app and the answers it waits for.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "locks, threads and the clock are std's"
)]

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::{Duration, Instant};

use coqui_acp::secret::{
    CAPABILITY, Capability, Form, IdentityId, Limits, REQUEST_METHOD, RequestParams, RequestReply,
    SHOW_DELIVER_METHOD, SHOW_OPEN_METHOD, ShowDeliver, ShowDone, ShowOpen, ShowOpened,
};
use coqui_acp_secrets::{
    Action, Approval, Decision, Error, Event, Frontend, Mode, Secrets, SessionConfig,
};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use super::{Grant, Hub, lock};
use crate::delegate::privilege::{Modes, modes_of};

/// The name of the MCP server the host adds to a session.
pub(crate) const SERVER_NAME: &str = "canoa-secrets";
/// The prefix of the id of every request the host makes of the app, which tells an answer to one from any other message.
const ID_PREFIX: &str = "canoa-secret-";
/// How long the host waits for the person, past the longest a binding lasts.
const WAIT: Duration = Duration::from_secs(300);
/// Requests the host remembers an answer for.
const MAX_PENDING: usize = 64;
/// Sessions of one connection that hold a bearer.
const MAX_SESSIONS: usize = 256;
/// The mode ids that ask for nothing, as the person's app names them.
const BYPASS: [&str; 6] = [
    "bypass",
    "bypasspermissions",
    "full_access",
    "full-access",
    "fullaccess",
    "yolo",
];

/// Whether a session in the mode `id` asks for nothing, which is what a secret's use then does too.
pub(super) fn is_bypass(id: &str) -> bool {
    BYPASS.contains(&id.to_ascii_lowercase().as_str())
}

/// What the host's audit log hears: names, never a value.
pub(super) fn observe(event: &Event) {
    match event {
        Event::Stored {
            session_id,
            handle,
            secret,
        } => tracing::info!(session = %session_id, %handle, field = %secret, "a secret was kept"),
        Event::Asked { session_id, action } => {
            tracing::info!(session = %session_id, "a secret's use was put to the person: {}", describe(action));
        }
        Event::Bypassed { session_id, action } => {
            tracing::info!(session = %session_id, "a secret's use was approved by the mode: {}", describe(action));
        }
        Event::Answered {
            session_id,
            decision,
            remembered,
        } => {
            tracing::info!(session = %session_id, ?decision, remembered, "a secret's use was answered")
        }
        Event::Wrote { session_id, path } => {
            tracing::info!(session = %session_id, path = %path.display(), "a secret was written");
        }
        Event::Ran {
            session_id,
            command,
        } => tracing::info!(session = %session_id, %command, "a command ran with a secret"),
        Event::Shown { session_id } => tracing::info!(session = %session_id, "a secret was shown"),
        Event::Ended {
            session_id,
            cleared,
        } => tracing::info!(session = %session_id, cleared, "a session's secrets were cleared"),
        _ => {}
    }
}

/// What an [`Action`] does, in a line.
fn describe(action: &Action) -> String {
    match action {
        Action::WriteFile { path, .. } => format!("write {}", path.display()),
        Action::RunCommand {
            variable, command, ..
        } => format!("run {command} with ${variable}"),
        Action::ShowCommand { command, .. } => format!("show the output of {command}"),
        Action::ShowEnv { name } => format!("show ${name}"),
        _ => "an action".to_owned(),
    }
}

/// A person's answer to a request the host asked.
struct Waiting {
    answer: Mutex<Option<Value>>,
    ready: Condvar,
}

/// A request of the client's the host remembers until the agent answers it.
enum Pending {
    Initialize,
    Open {
        digest: Option<[u8; 32]>,
        cwd: PathBuf,
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

/// A session that was given the tools.
struct Held {
    digest: [u8; 32],
    modes: Mutex<Modes>,
}

#[derive(Default)]
struct State {
    pending: HashMap<String, Pending>,
    sessions: HashMap<String, Arc<Held>>,
}

/// One client connection's secrets.
pub(crate) struct Link {
    hub: Arc<Hub>,
    secrets: Secrets,
    owner: IdentityId,
    binding: &'static str,
    offered: Mutex<Option<Capability>>,
    /// The agent takes MCP servers over HTTP.
    http: AtomicBool,
    /// The ACP version the agent answered in when it takes a stdio server, else 0.
    stdio: AtomicU64,
    closed: AtomicBool,
    state: Mutex<State>,
    outbox: Mutex<VecDeque<Vec<u8>>>,
    waiting: Mutex<HashMap<String, Arc<Waiting>>>,
    next: AtomicU64,
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

impl Link {
    pub(super) fn new(
        hub: Arc<Hub>,
        secrets: Secrets,
        owner: IdentityId,
        binding: &'static str,
    ) -> Arc<Self> {
        Arc::new(Self {
            hub,
            secrets,
            owner,
            binding,
            offered: Mutex::new(None),
            http: AtomicBool::new(false),
            stdio: AtomicU64::new(0),
            closed: AtomicBool::new(false),
            state: Mutex::new(State::default()),
            outbox: Mutex::new(VecDeque::new()),
            waiting: Mutex::new(HashMap::new()),
            next: AtomicU64::new(0),
        })
    }

    pub(super) fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// What the host added to the stream to the client.
    pub(super) fn drain(&self) -> Vec<Vec<u8>> {
        lock(&self.outbox).drain(..).collect()
    }

    /// A message of the client's as it came off the wire, before the scope removes its `_meta`: its offer in `initialize`, or an
    /// answer to a request of the host's. Whether it was the host's answer, which then goes no further.
    pub(super) fn taken_from_client(&self, message: &[u8]) -> bool {
        if contains(message, b"initialize") && contains(message, CAPABILITY.as_bytes()) {
            self.read_offer(message);
            return false;
        }
        if lock(&self.waiting).is_empty() || !contains(message, ID_PREFIX.as_bytes()) {
            return false;
        }
        let Ok(value) = serde_json::from_slice::<Value>(message) else {
            return false;
        };
        if value.get("method").is_some() {
            return false;
        }
        let Some(id) = value.get("id").and_then(Value::as_str) else {
            return false;
        };
        let Some(waiting) = lock(&self.waiting).remove(id) else {
            return false;
        };
        *lock(&waiting.answer) = Some(value);
        waiting.ready.notify_all();
        true
    }

    fn read_offer(&self, message: &[u8]) {
        let Ok(value) = serde_json::from_slice::<Value>(message) else {
            return;
        };
        if value.get("method") != Some(&json!("initialize")) {
            return;
        }
        let params = &value["params"];
        let meta = ["capabilities", "clientCapabilities"]
            .iter()
            .find_map(|key| params.get(*key)?.get("_meta")?.as_object());
        *lock(&self.offered) = meta.and_then(Capability::from_meta);
    }

    /// Whether a session opened now is given the tools.
    fn offering(&self) -> bool {
        lock(&self.offered)
            .as_ref()
            .is_some_and(|offer| offer.bindings.iter().any(|name| name == self.binding))
            && (self.http.load(Ordering::SeqCst) || self.stdio.load(Ordering::SeqCst) > 0)
            && !self.is_closed()
    }

    /// A message from the client after the scope has checked it. A session request gets the host's entry added.
    pub(crate) fn client_said(self: &Arc<Self>, message: Vec<u8>) -> Vec<u8> {
        if !(contains(&message, b"session/") || contains(&message, b"initialize")) {
            return message;
        }
        let Ok(mut value) = serde_json::from_slice::<Value>(&message) else {
            return message;
        };
        let id = value.get("id").map(Value::to_string);
        let Some(method) = value
            .get("method")
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            return message;
        };
        let Some(id) = id else {
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

    fn remember(&self, id: String, pending: Pending) {
        let mut state = lock(&self.state);
        if state.pending.len() < MAX_PENDING {
            state.pending.insert(id, pending);
        }
    }

    /// Adds the host's MCP server, with a bearer of its own, to a session request. Whether the request was changed.
    fn open(&self, value: &mut Value, method: &str, id: String) -> bool {
        let known = (method != "session/new")
            .then(|| value.pointer("/params/sessionId").and_then(Value::as_str))
            .flatten()
            .map(str::to_owned);
        let cwd = value
            .pointer("/params/cwd")
            .and_then(Value::as_str)
            .map(PathBuf::from);
        let offered = self.offering() && lock(&self.state).sessions.len() < MAX_SESSIONS;
        let (Some(cwd), true, Some(url)) =
            (cwd, offered, offered.then(|| self.hub.url()).flatten())
        else {
            return false;
        };
        let mut secret = [0_u8; 32];
        if getrandom::fill(&mut secret).is_err() {
            return false;
        }
        let token: String = secret.iter().map(|byte| format!("{byte:02x}")).collect();
        let digest: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        let Some(servers) = value
            .get_mut("params")
            .and_then(Value::as_object_mut)
            .map(|params| params.entry("mcpServers").or_insert_with(|| json!([])))
            .and_then(Value::as_array_mut)
        else {
            return false;
        };
        servers.retain(|server| server.get("name").and_then(Value::as_str) != Some(SERVER_NAME));
        let version = self.stdio.load(Ordering::SeqCst);
        servers.push(if self.http.load(Ordering::SeqCst) {
            json!({
                "type": "http",
                "name": SERVER_NAME,
                "url": url,
                "headers": [{"name": "Authorization", "value": format!("Bearer {token}")}],
            })
        } else {
            // The same stdio bridge the delegation tools use, with the address and bearer in its environment. Version 1 writes
            // a stdio server with no `type`; version 2 names it.
            let bridge = &self.hub.bridge;
            let mut entry = json!({
                "name": SERVER_NAME,
                "command": bridge.program,
                "args": bridge.args,
                "env": [
                    {"name": crate::delegate::bridge::URL_VAR, "value": url},
                    {"name": crate::delegate::bridge::TOKEN_VAR, "value": token},
                ],
            });
            if version >= 2 {
                entry["type"] = json!("stdio");
            }
            entry
        });
        self.hub.tokens.add(digest);
        self.remember(
            id,
            Pending::Open {
                digest: Some(digest),
                cwd,
                known,
            },
        );
        true
    }

    /// A message from the agent, before it goes to the client.
    pub(crate) fn agent_said(self: &Arc<Self>, message: &[u8]) {
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
                let http = result
                    .pointer("/agentCapabilities/mcpCapabilities/http")
                    .or_else(|| result.pointer("/capabilities/session/mcp/http"))
                    .is_some_and(|http| http == &json!(true) || http.is_object());
                self.http.store(http, Ordering::SeqCst);
                let version = result
                    .get("protocolVersion")
                    .and_then(Value::as_u64)
                    .unwrap_or(1);
                // Version 1 has stdio in its baseline; version 2 says it.
                let stdio = version < 2
                    || result
                        .pointer("/capabilities/session/mcp/stdio")
                        .is_some_and(|said| said == &json!(true) || said.is_object());
                self.stdio
                    .store(if stdio { version } else { 0 }, Ordering::SeqCst);
            }
            (
                Some(Pending::Open {
                    digest: Some(digest),
                    cwd,
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
                match named {
                    Some(named) => self.bind(digest, &named, cwd, modes_of(result)),
                    None => self.hub.tokens.remove(&digest),
                }
            }
            (
                Some(Pending::Open {
                    digest: Some(digest),
                    ..
                }),
                None,
            ) => self.hub.tokens.remove(&digest),
            (Some(Pending::Mode { session, mode }), Some(_)) => {
                if let Some(held) = self.held(&session) {
                    lock(&held.modes).current = Some(mode);
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
                    let mut modes = lock(&held.modes);
                    if result.get("configOptions").is_some() {
                        let fresh = modes_of(result);
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
        let Some(held) = self.held(id) else {
            return;
        };
        let update = &value["params"]["update"];
        let mut modes = lock(&held.modes);
        match update.get("sessionUpdate").and_then(Value::as_str) {
            Some("current_mode_update") => {
                if let Some(mode) = update.get("currentModeId").and_then(Value::as_str) {
                    modes.current = Some(mode.to_owned());
                }
            }
            Some("config_option_update") => {
                let fresh = modes_of(update);
                if fresh.option.is_some() {
                    modes.current = fresh.current;
                }
            }
            _ => {}
        }
    }

    fn held(&self, session: &str) -> Option<Arc<Held>> {
        lock(&self.state).sessions.get(session).cloned()
    }

    /// The session the agent has named is given its tools, under the bearer its request carried.
    fn bind(self: &Arc<Self>, digest: [u8; 32], session: &str, cwd: PathBuf, modes: Modes) {
        self.end(session);
        let Some(offer) = lock(&self.offered).clone() else {
            return;
        };
        let front = Arc::new(Front {
            link: Arc::downgrade(self),
            session: session.to_owned(),
            owner: self.owner,
            offer,
        });
        let config = SessionConfig::new(front, vec![cwd]);
        let Some(grant) = self.hub.tokens.find(&digest) else {
            return;
        };
        match self.secrets.open_session(session, config) {
            Ok(tools) => {
                *lock(&grant.tools) = Some(tools);
                lock(&self.state).sessions.insert(
                    session.to_owned(),
                    Arc::new(Held {
                        digest,
                        modes: Mutex::new(modes),
                    }),
                );
                tracing::debug!("a session was given the secret tools");
            }
            Err(error) => {
                tracing::info!("a session was given no secret tools: {error}");
                self.hub.tokens.remove(&digest);
            }
        }
    }

    /// Clears a session's secrets and revokes its bearer.
    fn end(&self, session: &str) {
        let held = lock(&self.state).sessions.remove(session);
        if let Some(held) = held {
            self.hub.tokens.remove(&held.digest);
            self.secrets.end_session(session);
        }
    }

    /// The connection ended: every session's secrets are cleared, every bearer revoked, and a prompt waiting is let go.
    pub(crate) fn close(&self) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        let sessions: Vec<(String, Arc<Held>)> = lock(&self.state).sessions.drain().collect();
        for (session, held) in sessions {
            self.hub.tokens.remove(&held.digest);
            self.secrets.end_session(&session);
        }
        let pending: Vec<[u8; 32]> = lock(&self.state)
            .pending
            .drain()
            .filter_map(|(_, pending)| match pending {
                Pending::Open { digest, .. } => digest,
                _ => None,
            })
            .collect();
        for digest in pending {
            self.hub.tokens.remove(&digest);
        }
        for waiting in lock(&self.waiting).values() {
            waiting.ready.notify_all();
        }
    }

    /// Asks the app and waits for the person: the `result` of its answer.
    pub(super) fn ask(&self, method: &str, params: Value) -> Result<Value, Error> {
        if self.is_closed() {
            return Err(Error::Frontend("the person's app is gone"));
        }
        let mut salt = [0_u8; 8];
        getrandom::fill(&mut salt).map_err(|_| Error::Frontend("no randomness"))?;
        let id = format!(
            "{ID_PREFIX}{}-{}",
            self.next.fetch_add(1, Ordering::SeqCst),
            salt.iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        let waiting = Arc::new(Waiting {
            answer: Mutex::new(None),
            ready: Condvar::new(),
        });
        lock(&self.waiting).insert(id.clone(), Arc::clone(&waiting));
        let request = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        lock(&self.outbox).push_back(request.to_string().into_bytes());
        let until = Instant::now() + WAIT;
        let mut answer = lock(&waiting.answer);
        while answer.is_none() && !self.is_closed() {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            answer = waiting
                .ready
                .wait_timeout(answer, left)
                .map_or_else(|error| error.into_inner().0, |(guard, _)| guard);
        }
        let taken = answer.take();
        drop(answer);
        lock(&self.waiting).remove(&id);
        let Some(reply) = taken else {
            return Err(Error::Frontend("the person's app did not answer"));
        };
        if reply.get("error").is_some() {
            return Err(Error::Frontend("the person's app refused the question"));
        }
        reply
            .get("result")
            .cloned()
            .ok_or(Error::Frontend("the person's app gave no answer"))
    }

    /// Whether the session asks for nothing now.
    pub(super) fn bypass(&self, session: &str) -> bool {
        self.held(session)
            .is_some_and(|held| lock(&held.modes).current.as_deref().is_some_and(is_bypass))
    }
}

/// The person's app, as a session's one frontend.
struct Front {
    link: Weak<Link>,
    session: String,
    owner: IdentityId,
    offer: Capability,
}

impl Front {
    fn link(&self) -> Result<Arc<Link>, Error> {
        self.link
            .upgrade()
            .ok_or(Error::Frontend("the person's app is gone"))
    }
}

fn malformed(_: coqui_acp::secret::Error) -> Error {
    Error::Frontend("the person's app answered in no shape the extension has")
}

impl Frontend for Front {
    fn identity(&self) -> IdentityId {
        self.owner
    }

    fn capability(&self) -> Capability {
        self.offer.clone()
    }

    fn request(&self, params: &RequestParams) -> Result<RequestReply, Error> {
        let reply = self.link()?.ask(REQUEST_METHOD, params.to_value())?;
        let form = Form::parse_text(&params.context.schema, &Limits::new()).map_err(malformed)?;
        RequestReply::from_value(&reply, |name| {
            form.field(name).is_some_and(|field| field.secret)
        })
        .map_err(malformed)
    }

    fn show_open(&self, open: &ShowOpen) -> Result<ShowOpened, Error> {
        let reply = self.link()?.ask(SHOW_OPEN_METHOD, open.to_value())?;
        ShowOpened::from_value(&reply).map_err(malformed)
    }

    fn show_deliver(&self, deliver: &ShowDeliver) -> Result<ShowDone, Error> {
        let reply = self.link()?.ask(SHOW_DELIVER_METHOD, deliver.to_value())?;
        ShowDone::from_value(&reply).map_err(malformed)
    }

    fn approve(&self, approval: &Approval) -> Result<Decision, Error> {
        let link = self.link()?;
        let reply = link.ask("session/request_permission", permission(approval))?;
        let outcome = &reply["outcome"];
        if outcome["outcome"] != "selected" {
            return Ok(Decision::Deny);
        }
        Ok(match outcome["optionId"].as_str() {
            Some("allow-once") => Decision::Once,
            Some("allow-session") => Decision::Session,
            _ => Decision::Deny,
        })
    }

    fn mode(&self) -> Mode {
        match self.link.upgrade() {
            Some(link) if link.bypass(&self.session) => Mode::Bypass,
            _ => Mode::Ask,
        }
    }
}

/// The permission request the person is asked, as any other card: what would be done, in full, and what that trusts.
fn permission(approval: &Approval) -> Value {
    let (title, kind) = match &approval.action {
        Action::WriteFile { .. } => (
            format!("Write the secret {} to a file", approval.secret),
            "edit",
        ),
        Action::RunCommand { .. } => (
            format!("Run a command with the secret {}", approval.secret),
            "execute",
        ),
        Action::ShowCommand { .. } => (
            format!(
                "Show the output of a command as the secret {}",
                approval.secret
            ),
            "execute",
        ),
        Action::ShowEnv { .. } => (
            format!(
                "Show an environment variable as the secret {}",
                approval.secret
            ),
            "read",
        ),
        _ => (format!("Use the secret {}", approval.secret), "other"),
    };
    let mut lines = vec![title.clone()];
    match &approval.action {
        Action::WriteFile {
            path,
            mode,
            mode_applied,
            overwrite,
            under_roots,
        } => {
            lines.push(format!("File: {}", path.display()));
            lines.push(format!(
                "Mode: {mode:o}{}",
                if *mode_applied {
                    ""
                } else {
                    " (not applied on this system)"
                }
            ));
            if *overwrite {
                lines.push("An existing file is replaced.".to_owned());
            }
            if *under_roots {
                lines.push(
                    "The file is in the agent's working folder: its own tools can read it back."
                        .to_owned(),
                );
            }
        }
        Action::RunCommand {
            variable,
            command,
            program,
            target,
            cwd,
        } => {
            lines.push(format!("Command: {command}"));
            lines.push(format!("The secret is in ${variable}."));
            lines.push(format!("Program: {}", program.display()));
            if let Some(target) = target {
                lines.push(format!("A link to: {}", target.display()));
            }
            lines.push(format!("Folder: {}", cwd.display()));
        }
        Action::ShowCommand {
            command,
            program,
            target,
            cwd,
        } => {
            lines.push(format!("Command: {command}"));
            lines.push(format!("Program: {}", program.display()));
            if let Some(target) = target {
                lines.push(format!("A link to: {}", target.display()));
            }
            lines.push(format!("Folder: {}", cwd.display()));
        }
        Action::ShowEnv { name } => lines.push(format!("Variable: ${name}")),
        _ => {}
    }
    if let Some(notice) = approval.notice {
        lines.push(notice.to_owned());
    }
    json!({
        "sessionId": approval.session_id,
        "toolCall": {
            "toolCallId": format!("secret-{}", approval.handle.as_deref().unwrap_or(&approval.secret)),
            "title": title,
            "kind": kind,
            "status": "pending",
            "content": [{"type": "content", "content": {"type": "text", "text": lines.join("\n")}}],
        },
        "options": [
            {"optionId": "allow-once", "name": "Allow once", "kind": "allow_once"},
            {"optionId": "allow-session", "name": "Allow for this session", "kind": "allow_always"},
            {"optionId": "reject", "name": "Reject", "kind": "reject_once"},
        ],
        "_meta": {"dev.boriken.canoa/secret": {"field": approval.secret}},
    })
}

/// A grant, once the agent has named its session.
pub(super) fn tools_of(grant: &Grant) -> Option<coqui_acp_secrets::SessionTools> {
    lock(&grant.tools).clone()
}
