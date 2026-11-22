//! The hosted `Channel` wrapper: request/response interception, MCP/hook approval prompting, and report injection.

use alloc::borrow::ToOwned;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::time::Duration;
use std::sync::{Mutex, PoisonError};
use std::time::Instant;

use coqui_transport::{Channel, Failure};
use serde_json::{Value, json};

use crate::scope::{Scope, Verdict};
use crate::usage_splice::{Client, Splice};

use super::hooks::{self, Book, Seen};
use super::offer::{INVALID_PARAMS, INVALID_REQUEST, error};
use super::overlays::harness_of;
use super::parts::{Parts, add_parts, described, parts_with};
use super::{Connection, Item, PROFILE_META, State};

/// The session requests a profile's parts go with.
pub(super) const SESSION_METHODS: [&str; 3] = ["session/new", "session/load", "session/resume"];
/// `initialize` and session requests a connection may have waiting for the agent's answer.
pub(super) const MAX_PENDING: usize = 64;
pub(super) const INTERNAL_ERROR: i64 = -32603;
const REPLY_LIMIT: Duration = Duration::from_secs(30);
const REPLY_POLL: Duration = Duration::from_millis(10);
/// The method the host asks the client that opened a session which of the profile's MCP servers it may start.
pub const APPROVE_METHOD: &str = "_canoa/profile/approve";
/// The `_meta` key of `initialize` with which a client says it answers [`APPROVE_METHOD`]; one that does not is never asked.
pub const APPROVE_META: &str = "dev.boriken.canoa/approve";
/// How long the person has to answer an approval.
const APPROVE_WAIT: Duration = Duration::from_secs(120);

/// The report with the servers the person allowed for this session shown as applied.
pub(super) fn mark_allowed(report: &mut Value, allowed: &[String]) {
    let Some(items) = report.get_mut("items").and_then(Value::as_array_mut) else {
        return;
    };
    for item in items {
        let named = item
            .get("key")
            .and_then(Value::as_str)
            .and_then(|key| key.strip_prefix("mcp."))
            .is_some_and(|name| allowed.iter().any(|each| each == name));
        if named {
            item["state"] = json!(State::Applied.as_str());
            item["how"] = json!("ACP mcpServers; allowed for this session when the host asked");
        }
    }
}

/// The report with what the hooks of this session did in place of what the profile said of them before any was asked.
pub(super) fn replace_hook_items(report: &mut Value, notes: &[Item]) {
    let Some(items) = report.get_mut("items").and_then(Value::as_array_mut) else {
        return;
    };
    if notes.is_empty() {
        return;
    }
    items.retain(|item| {
        !item
            .get("key")
            .and_then(Value::as_str)
            .is_some_and(|key| key.starts_with("hooks."))
    });
    items.extend(notes.iter().map(Item::json));
}

/// Puts the report under [`PROFILE_META`] in an answer's `_meta`.
fn add_report(result: &mut Value, report: &Value) {
    let Some(result) = result.as_object_mut() else {
        return;
    };
    let meta = result.entry("_meta").or_insert_with(|| json!({}));
    if !meta.is_object() {
        *meta = json!({});
    }
    meta[PROFILE_META] = report.clone();
}

/// Whether `message` names a profile under `params._meta`.
pub(super) fn names_profile(message: &[u8]) -> bool {
    serde_json::from_slice::<Value>(message)
        .ok()
        .and_then(|value| {
            value
                .get("params")?
                .get("_meta")?
                .get(PROFILE_META)
                .cloned()
        })
        .is_some()
}

#[derive(Debug)]
enum Pending {
    Initialize,
    /// A session request, with the servers the client allowed for it.
    Session(Vec<String>),
}

#[derive(Debug)]
pub(super) struct Hold {
    launch: Option<Vec<u8>>,
    first: bool,
    pending: BTreeMap<String, Pending>,
    pub(super) parts: Option<Parts>,
    /// Sessions that have already received the fallback initial prompt briefing.
    briefed: BTreeSet<String>,
    /// The working directory of each session the agent has answered a `session/new` or `session/load` for, for the briefing's
    /// templates; dropped when the session closes.
    pub(super) session_cwds: BTreeMap<String, Option<String>>,
    /// The session id (a `session/load` names it) and working directory of each session request the agent has not answered.
    session_asks: BTreeMap<String, (Option<String>, Option<String>)>,
    /// Messages the client sent while the host waited for an approval, in order, to be read before any other.
    queued: alloc::collections::VecDeque<Vec<u8>>,
    asked: u64,
    /// Whether the client said in `initialize` that it answers approvals.
    approves: bool,
    /// What the overlays' hooks remember between messages.
    pub(super) book: Book,
}

/// What the person allowed when the host asked: the MCP servers, and the overlays whose hooks run in this session.
#[derive(Debug, Default)]
struct Approved {
    servers: Vec<String>,
    hooks: BTreeSet<String>,
}

/// A connection's channel to its agent with a profile applied: the launch line first, then each message the client sends
/// as [`Scope`] passes it, with the profile's parts added to session requests; and the report added to the agent's
/// answers to `initialize` and to each session request.
pub(crate) struct Hosted<'a, C: Channel + ?Sized> {
    pub(super) inner: &'a C,
    pub(super) scope: &'a Scope,
    pub(super) connection: &'a Connection,
    pub(super) hold: Mutex<Hold>,
    splice: Splice,
    sites: crate::site::Sites,
    /// An unreadable line has been logged.
    noted: core::sync::atomic::AtomicBool,
}

impl<C: Channel + ?Sized> core::fmt::Debug for Hosted<'_, C> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Hosted").finish_non_exhaustive()
    }
}

impl<'a, C: Channel + ?Sized> Hosted<'a, C> {
    /// This channel serves the agent running as process `pid`.
    #[must_use]
    pub(crate) fn agent(mut self, pid: u32) -> Self {
        self.splice.set_agent(pid);
        self
    }

    pub(crate) fn new(inner: &'a C, scope: &'a Scope, connection: &'a Connection) -> Self {
        Self {
            inner,
            scope,
            connection,
            hold: Mutex::new(Hold {
                launch: Some(connection.launch_line()),
                first: true,
                pending: BTreeMap::new(),
                parts: None,
                briefed: BTreeSet::new(),
                session_cwds: BTreeMap::new(),
                session_asks: BTreeMap::new(),
                queued: alloc::collections::VecDeque::new(),
                asked: 0,
                approves: false,
                book: Book::default(),
            }),
            splice: scope.splice(),
            sites: scope.sites(),
            noted: core::sync::atomic::AtomicBool::new(false),
        }
    }

    /// The same, for an agent in this process, which takes its environment some other way than a first line.
    pub(crate) fn in_process(inner: &'a C, scope: &'a Scope, connection: &'a Connection) -> Self {
        let hosted = Self::new(inner, scope, connection);
        hosted.hold().launch = None;
        hosted
    }

    pub(super) fn hold(&self) -> std::sync::MutexGuard<'_, Hold> {
        self.hold.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn reply(&self, message: &[u8]) -> Result<(), Failure> {
        let until = Instant::now() + REPLY_LIMIT;
        loop {
            match self.inner.send(message) {
                Err(Failure::NoProgress) if Instant::now() < until => {
                    std::thread::sleep(REPLY_POLL)
                }
                other => return other,
            }
        }
    }

    /// For a session request, asks the client which of the servers canoa.cfg does not allow it lets this session start, with
    /// [`APPROVE_METHOD`]: what the person chose, or nothing when the client does not answer it, answers with an error, takes too
    /// long or goes away. The client's other messages meanwhile wait their turn.
    fn approvals(&self, bytes: &[u8]) -> Approved {
        let Ok(request) = serde_json::from_slice::<Value>(bytes) else {
            return Approved::default();
        };
        let session = request
            .get("method")
            .and_then(Value::as_str)
            .is_some_and(|method| SESSION_METHODS.contains(&method));
        if !session || request.get("id").is_none() {
            return Approved::default();
        }
        let hooks = self.connection.hooks.approvable();
        let (ask_id, servers) = {
            let mut hold = self.hold();
            if !hold.approves || !self.connection.in_process {
                return Approved::default();
            }
            let Some(parts) = hold
                .parts
                .as_ref()
                .filter(|parts| !parts.approvable.is_empty() || !hooks.is_empty())
            else {
                return Approved::default();
            };
            let servers: Vec<Value> = parts
                .approvable
                .iter()
                .map(|each| json!({"name": each.name, "layer": each.layer, "digest": each.digest}))
                .collect();
            hold.asked += 1;
            // Unguessable, so nothing else can answer for the person.
            let mut nonce = [0_u8; 12];
            let _ = getrandom::fill(&mut nonce);
            let hex: String = data_encoding::HEXLOWER.encode(&nonce);
            (format!("canoa-approve-{}-{hex}", hold.asked), servers)
        };
        let ask = json!({
            "jsonrpc": "2.0",
            "id": ask_id,
            "method": APPROVE_METHOD,
            "params": {
                "profile": self.connection.name,
                "harness": self.connection.entry,
                "servers": servers,
                "hooks": hooks,
            },
        });
        let Ok(ask) = serde_json::to_vec(&ask) else {
            return Approved::default();
        };
        if self.reply(&ask).is_err() {
            return Approved::default();
        }
        let until = Instant::now() + APPROVE_WAIT;
        while Instant::now() < until {
            match self.inner.receive(Some(Duration::from_millis(50))) {
                Ok(Some(message)) => {
                    let value: Value = serde_json::from_slice(&message).unwrap_or(Value::Null);
                    if value.get("method").is_none() && value.get("id") == Some(&json!(ask_id)) {
                        let held: Vec<String> = self
                            .hold()
                            .parts
                            .as_ref()
                            .map(|parts| {
                                parts
                                    .approvable
                                    .iter()
                                    .map(|each| each.name.clone())
                                    .collect()
                            })
                            .unwrap_or_default();
                        let result = value.get("result");
                        return Approved {
                            servers: result
                                .and_then(|result| result.get("allow"))
                                .and_then(Value::as_array)
                                .map(|names| {
                                    names
                                        .iter()
                                        .filter_map(Value::as_str)
                                        .filter(|name| held.iter().any(|each| each == name))
                                        .map(str::to_owned)
                                        .collect()
                                })
                                .unwrap_or_default(),
                            // Only an overlay the question listed can be allowed by the answer.
                            hooks: self.connection.hooks.offered(&hooks::names(
                                result.and_then(|result| result.get("allowHooks")),
                            )),
                        };
                    }
                    self.hold().queued.push_back(message);
                }
                Err(Failure::Timeout) => {}
                Ok(None) | Err(_) => return Approved::default(),
            }
        }
        Approved::default()
    }

    /// Records the request, and adds the profile's parts to a session request; `Err` is the answer refusing it.
    pub(super) fn forward(&self, bytes: Vec<u8>, allowed: Vec<String>) -> Result<Vec<u8>, Vec<u8>> {
        let Ok(mut value) = serde_json::from_slice::<Value>(&bytes) else {
            return Ok(bytes);
        };
        let method = value
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let Some(id) = value.get("id").cloned() else {
            return Ok(bytes);
        };
        let mut hold = self.hold();
        let tracked = method == "initialize" || SESSION_METHODS.contains(&method.as_str());
        if tracked && hold.pending.len() >= MAX_PENDING {
            return Err(error(
                &id,
                INVALID_REQUEST,
                "too many requests wait for the agent's answer",
            ));
        }
        if method == "initialize" {
            hold.pending.insert(id.to_string(), Pending::Initialize);
            return Ok(bytes);
        }
        if !tracked {
            if method == "session/close"
                && let Some(session_id) = value.pointer("/params/sessionId").and_then(Value::as_str)
            {
                hold.session_cwds.remove(session_id);
                hold.briefed.remove(session_id);
            }
            if method == "session/prompt"
                && let Some(session_id) = value
                    .pointer("/params/sessionId")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                && !hold.briefed.contains(&session_id)
                && let Some(parts) = &hold.parts
                && let Some(briefing) = parts
                    .briefing_for(
                        hold.session_cwds
                            .get(&session_id)
                            .and_then(|cwd| cwd.as_deref()),
                    )
                    .map_err(|why| error(&id, INVALID_PARAMS, &why))?
                && let Some(prompt) = value
                    .pointer_mut("/params/prompt")
                    .and_then(Value::as_array_mut)
            {
                prompt.insert(0, json!({"type": "text", "text": briefing}));
                hold.briefed.insert(session_id);
                return serde_json::to_vec(&value)
                    .map_err(|_| error(&id, INTERNAL_ERROR, "the request could not be written"));
            }
            return Ok(bytes);
        }
        let Some(parts) = &hold.parts else {
            return Err(error(
                &id,
                INVALID_REQUEST,
                "initialize comes first on an entry with a profile",
            ));
        };
        let params = value
            .as_object_mut()
            .map(|object| object.entry("params").or_insert_with(|| json!({})))
            .and_then(Value::as_object_mut);
        let Some(params) = params else {
            return Err(error(&id, INVALID_PARAMS, "params must be an object"));
        };
        add_parts(params, parts).map_err(|why| error(&id, INVALID_PARAMS, &why))?;
        let cwd = params.get("cwd").and_then(Value::as_str).map(str::to_owned);
        let session_id = params
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if let Some(servers) = params.get_mut("mcpServers").and_then(Value::as_array_mut) {
            servers.extend(
                parts
                    .approvable
                    .iter()
                    .filter(|each| allowed.contains(&each.name))
                    .map(|each| each.server.clone()),
            );
        }
        hold.session_asks.insert(id.to_string(), (session_id, cwd));
        hold.pending
            .insert(id.to_string(), Pending::Session(allowed));
        serde_json::to_vec(&value)
            .map_err(|_| error(&id, INTERNAL_ERROR, "the request could not be written"))
    }
}

impl<C: Channel + ?Sized> Hosted<'_, C> {
    /// An answer to `initialize` or a session request gets the report; the first also settles what each session is given.
    fn send_profiled(&self, message: &[u8]) -> Result<(), Failure> {
        // `_canoa/` and the secret extension are the host's own to ask of its client: an agent's request of that name is never
        // passed on, or the client could be made to show the host's questions, or to answer the host's real ones, for the agent.
        let screened;
        let message = match crate::scope::screen(message) {
            crate::scope::Screened::Drop(why, answer) => {
                crate::scope::note_dropped(why, &self.noted);
                // A client waiting for this answer is told it will not come.
                return answer.map_or(Ok(()), |answer| self.inner.send(&answer));
            }
            crate::scope::Screened::Pass(Some(changed)) => {
                screened = changed;
                &screened[..]
            }
            crate::scope::Screened::Pass(None) => message,
        };
        let stamped = self.scope.stamp(message);
        let annotated = self.sites.annotate(&stamped);
        let message = &*annotated;
        let mut hold = self.hold();
        if hold.pending.is_empty() {
            drop(hold);
            return self.inner.send(message);
        }
        let parsed = serde_json::from_slice::<Value>(message)
            .ok()
            .filter(|value| value.get("method").is_none());
        let Some(mut value) = parsed else {
            drop(hold);
            return self.inner.send(message);
        };
        let pending = value
            .get("id")
            .and_then(|id| hold.pending.remove(&id.to_string()));
        match pending {
            // What sessions are given is settled by the first answer; a later `initialize` changes nothing.
            Some(Pending::Initialize) if hold.parts.is_some() => {
                if let (Some(result), Some(parts)) = (value.get_mut("result"), &hold.parts) {
                    add_report(result, &parts.report);
                }
            }
            Some(Pending::Initialize) => {
                if let Some(result) = value.get("result") {
                    let (takes, reported) = described(result);
                    let harness = harness_of(reported, &self.connection.entry);
                    let lookup = |name: &str| std::env::var(name).ok();
                    let parts = parts_with(self.connection, harness, reported, &takes, &lookup);
                    if let Some(result) = value.get_mut("result") {
                        add_report(result, &parts.report);
                    }
                    hold.parts = Some(parts);
                }
            }
            Some(Pending::Session(allowed)) => {
                let key = value.get("id").map(ToString::to_string).unwrap_or_default();
                if let Some((asked, cwd)) = hold.session_asks.remove(&key)
                    && value.get("error").is_none()
                    && let Some(session_id) = value
                        .pointer("/result/sessionId")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .or(asked)
                {
                    hold.session_cwds.insert(session_id, cwd);
                }
                let notes = hold.book.notes_for(&key);
                if let (Some(result), Some(parts)) = (value.get_mut("result"), &hold.parts) {
                    let mut report = parts.report.clone();
                    mark_allowed(&mut report, &allowed);
                    replace_hook_items(&mut report, &notes);
                    add_report(result, &report);
                }
            }
            None => {
                drop(hold);
                return self.inner.send(message);
            }
        }
        drop(hold);
        match serde_json::to_vec(&value) {
            Ok(bytes) => self.inner.send(&bytes),
            Err(_) => self.inner.send(message),
        }
    }
}

impl<C: Channel + ?Sized> Drop for Hosted<'_, C> {
    /// A connection that ends without closing its sessions ends them: best effort, for the hooks that listen.
    fn drop(&mut self) {
        self.end_all();
    }
}

impl<C: Channel + ?Sized> Channel for Hosted<'_, C> {
    /// The agent's message to the client: profiled, and with this host's say on usage when the agent reports none.
    fn send(&self, message: &[u8]) -> Result<(), Failure> {
        self.splice.relay(message, |sent| {
            let later = match self.on_agent(sent) {
                Seen::Dropped => return Ok(()),
                Seen::Passed(later) => later,
            };
            let sent_on = self.send_profiled(sent);
            if let Some(later) = later {
                self.later(later);
            }
            sent_on
        })
    }

    /// The launch line first; then the client's messages, a refused one reading as a timeout as [`crate::scope`] has it.
    fn receive(&self, timeout: Option<Duration>) -> Result<Option<Vec<u8>>, Failure> {
        // What the harness's own hooks have reported since the last look: run before anything else moves.
        self.drain_forwarded();
        let first = {
            let mut hold = self.hold();
            if let Some(line) = hold.launch.take() {
                return Ok(Some(line));
            }
            // Canoa's own answers to the agent, made by a hook's denial, go before anything the client sent.
            if let Some(answer) = hold.book.next_injected() {
                return Ok(Some(answer));
            }
            core::mem::replace(&mut hold.first, false)
        };
        self.splice.tick(|sent| self.send_profiled(sent));
        let queued = self.hold().queued.pop_front();
        let message = match queued {
            Some(message) => message,
            None => match self.inner.receive(timeout)? {
                Some(message) => message,
                None => return Ok(None),
            },
        };
        if !first && names_profile(&message) {
            if let Some(id) = serde_json::from_slice::<Value>(&message)
                .ok()
                .and_then(|value| value.get("id").cloned())
            {
                self.reply(&error(
                    &id,
                    INVALID_PARAMS,
                    "a profile is chosen in initialize, once",
                ))?;
            }
            tracing::info!(
                entry = self.connection.entry,
                "a client named a profile after initialize, which was refused"
            );
            return Err(Failure::Timeout);
        }
        // The scope removes every `_meta` of the client's, so its opt-in is read before.
        if let Ok(value) = serde_json::from_slice::<Value>(&message)
            && value.get("method") == Some(&json!("initialize"))
        {
            self.hold().approves = value
                .pointer("/params/_meta")
                .and_then(|meta| meta.get(APPROVE_META))
                == Some(&json!(true));
        }
        let forwarded = match self.sites.check(self.scope, &message) {
            Verdict::Forward(bytes) => {
                let approved = self.approvals(&bytes);
                match self.on_client(bytes, &approved.hooks) {
                    Ok(bytes) => self.forward(bytes, approved.servers),
                    Err(answer) => Err(answer),
                }
            }
            Verdict::Refuse(answer) => Err(answer.unwrap_or_default()),
        };
        match forwarded {
            Ok(bytes) => match self.splice.read_client_as(&bytes, &message) {
                Client::Answer(answer) => {
                    self.abandon_opening(&bytes);
                    self.reply(&answer)?;
                    Err(Failure::Timeout)
                }
                Client::Held => {
                    self.abandon_opening(&bytes);
                    Err(Failure::Timeout)
                }
                Client::Forward => Ok(Some(bytes)),
            },
            Err(answer) => {
                self.abandon_opening(&message);
                if !answer.is_empty() {
                    self.reply(&answer)?;
                }
                tracing::info!(entry = self.connection.entry, "a message was refused");
                Err(Failure::Timeout)
            }
        }
    }

    fn close(&self) {
        self.inner.close();
        self.end_all();
    }

    fn set_max_message(&self, bytes: usize) {
        self.inner.set_max_message(bytes);
    }
}
