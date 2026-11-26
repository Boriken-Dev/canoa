//! The events Canoa sees over ACP, and the hooks it fires for them. The client's `session/new`, `session/load` and
//! `session/resume` start a session, its `session/prompt` is a prompt and its `session/close` the end of one; the agent's
//! answer to a prompt ends a turn, its `session/request_permission` asks about a tool call, and the `tool_call_update` that
//! ends one reports it done. `cd` and `pre-compact` are not shown over ACP, so nothing here fires them.

use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use std::path::Path;
use std::sync::{Mutex, PoisonError};

use canoa::approver::{self, Choice};
use coqui_aos::{Decision, FiredEvent, HookEvent};
use coqui_transport::{Channel, Failure};
use serde_json::{Map, Value, json};

use super::{Firing, report_item};
use super::{allow, forward::Forward};
use crate::approve;
use crate::approve::catalog::Catalog;
use crate::approve::chain::{Asking, Chain, Outcome, Presence, Settle, Settled};
use crate::profile::hosted::project_context;
use crate::profile::hosted::{
    Hosted, Item, LayerId, LayerKind, SESSION_METHODS, State, Written, error,
};

mod native;
mod project;

/// The outputs Canoa applies for each event over ACP.
const SESSION_START: [&str; 2] = ["context", "refuse"];
const PROMPT: [&str; 2] = ["context", "block"];
pub(super) const PRE_TOOL: [&str; 3] = ["deny", "ask", "allow"];
const POST_TOOL: [&str; 1] = ["context"];
const NOTHING: [&str; 0] = [];

/// The most tool calls remembered between their start and their end, and turns waiting for their answer.
const MAX_TRACKED: usize = 256;
/// The most notes kept for the next report.
const MAX_ITEMS: usize = 64;
/// JSON-RPC's code for an error of the server's own.
const SERVER_ERROR: i64 = -32000;

/// A session's hooks: Canoa's own id for it, which no hook can tie to the agent's, the overlays the person allowed for it,
/// and where it works.
#[derive(Clone, Debug, Default)]
pub(in crate::profile::hosted) struct SessionHooks {
    id: String,
    allowed: BTreeSet<String>,
    cwd: String,
    /// Where the session opened: the walk of its project's context files starts here.
    root: String,
    /// The directories whose context files the session has been given.
    fed: Vec<String>,
    /// Whether the harness's own hooks tell the host where the session works, so a deeper directory's files can follow.
    follows: bool,
}

/// A session request on its way to the agent.
#[derive(Debug)]
struct Opening {
    hooks: SessionHooks,
    /// The channel and plugin of the harness's own hooks made for this request: the session's once it is answered, else
    /// ended with the request.
    native: Option<native::Enrolled>,
    /// The agent's id for it when the request names it (load and resume).
    named: Option<String>,
    notes: Vec<Item>,
}

/// A tool call as the agent has shown it so far.
#[derive(Clone, Debug)]
pub(in crate::profile::hosted) struct Tool {
    kind: String,
    name: String,
    input: Value,
}

/// A permission request as the agent asked it, for the approvers.
struct Asked<'a> {
    id: &'a Value,
    /// The agent's id for the session.
    agent: &'a str,
    session: &'a SessionHooks,
    call: Option<&'a Value>,
    tool: &'a Tool,
    options: Option<&'a [Value]>,
    protected: bool,
}

/// A request the approvers are answering beside the window.
#[derive(Debug)]
pub(in crate::profile::hosted) struct Pending {
    asking: Asking,
    /// The request's id, which the agent is answered under.
    id: Value,
    agent: String,
    call: Option<Value>,
    options: Vec<Value>,
}

/// What a connection's hooks remember between messages.
#[derive(Debug, Default)]
pub(in crate::profile::hosted) struct Book {
    /// By the agent's session id.
    sessions: BTreeMap<String, SessionHooks>,
    /// By the request's id.
    opening: BTreeMap<String, Opening>,
    /// What the answer to a session request reports of its hooks, by the request's id.
    notes: BTreeMap<String, Vec<Item>>,
    /// Prompts waiting for the agent's answer, by the request's id: the agent's session.
    turns: BTreeMap<String, String>,
    /// Context for a session's next prompt, by Canoa's id for it: `(overlay, text)`.
    carry: BTreeMap<String, Vec<(String, String)>>,
    /// The project's context files for a session's next prompt, by Canoa's id for it: `(path, text)`.
    project: BTreeMap<String, Vec<(String, String)>>,
    /// Bytes of hook context each session (by Canoa's id for it) has been given.
    used: BTreeMap<String, usize>,
    /// Tool calls shown and not yet ended, by session and call.
    tools: BTreeMap<(String, String), Tool>,
    /// Messages Canoa answers the agent with, read before the client's.
    inject: VecDeque<Vec<u8>>,
    /// What went wrong with hooks outside a session request, for the reports to come.
    items: Vec<Item>,
    /// Where a harness's own hooks report to this connection, once a session has asked for it.
    forward: Option<Forward>,
    /// The plugins of hooks written for each session, by Canoa's id for it: removed with it.
    plugins: BTreeMap<String, Vec<(Written, serde_json::Value)>>,
    /// The approvers of this connection, made when the first request needs them.
    chain: Option<Arc<Chain>>,
    /// Requests approvers are answering beside the window, by the request's id.
    asking: BTreeMap<String, Pending>,
    /// Requests approvers settled whose late answer from the client is dropped, by the request's id.
    settled: BTreeSet<String>,
    /// What the approvers answered, which `settle_approvals` reads.
    outbox: Arc<Mutex<VecDeque<(String, Settled)>>>,
    /// Messages for the client, sent before it is read from again.
    to_client: VecDeque<Vec<u8>>,
}

impl Book {
    /// How many requests approvers are answering beside the window.
    #[cfg(test)]
    pub(in crate::profile::hosted) fn waiting(&self) -> usize {
        self.asking.len()
    }

    /// The oldest message Canoa has for the agent.
    pub(in crate::profile::hosted) fn next_injected(&mut self) -> Option<Vec<u8>> {
        self.inject.pop_front()
    }

    /// What the report of an answer to the session request `id` adds about hooks.
    pub(in crate::profile::hosted) fn notes_for(&mut self, id: &str) -> Vec<Item> {
        let mut notes = self.notes.remove(id).unwrap_or_default();
        notes.extend(core::mem::take(&mut self.items));
        notes
    }

    /// The bytes of hook context the session holds.
    fn used(&self, session: &str) -> usize {
        self.used.get(session).copied().unwrap_or(0)
    }

    /// Counts `context`, which the engine kept within the session's bound, among what the session holds.
    fn took(&mut self, session: &str, context: Vec<(String, String)>) -> Vec<(String, String)> {
        let used = self.used.entry(session.to_owned()).or_default();
        for (_, text) in &context {
            *used = used.saturating_add(text.len());
        }
        context
    }

    fn remember(&mut self, items: impl IntoIterator<Item = Item>) {
        for item in items {
            if self.items.len() < MAX_ITEMS {
                self.items.push(item);
            }
        }
    }
}

/// What to do after a message from the agent has been passed on.
#[derive(Debug)]
pub(in crate::profile::hosted) enum Later {
    /// A prompt's answer: the turn ended in this agent session.
    TurnEnd(String),
    /// A tool call ended.
    PostTool(String, Tool, &'static str),
}

/// What becomes of a message from the agent.
#[derive(Debug)]
pub(in crate::profile::hosted) enum Seen {
    /// Not passed on: Canoa has answered it.
    Dropped,
    /// Passed on, and then this.
    Passed(Option<Later>),
}

/// The text of a prompt's content blocks, joined by line breaks.
fn prompt_text(blocks: Option<&Value>) -> String {
    blocks
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Each part of context as two content blocks: a label Canoa wrote, and the hook's text on its own, so nothing a hook
/// prints can pass for the label.
fn blocks_of(context: &[(String, String)], event: &str) -> Vec<Value> {
    context
        .iter()
        .flat_map(|(overlay, text)| {
            [
                json!({"type": "text", "text": format!("Added by the {event} hook of the overlay {overlay}, not by the user:")}),
                json!({"type": "text", "text": text}),
            ]
        })
        .collect()
}

/// Adds `blocks` to the end of the system prompt a new session's request carries, which it makes when it has none.
fn append_system(params: &mut Map<String, Value>, blocks: Vec<Value>) {
    match params
        .get_mut("systemPrompt")
        .and_then(Value::as_object_mut)
    {
        Some(prompt) => match prompt.get_mut("content").and_then(Value::as_array_mut) {
            Some(content) => content.extend(blocks),
            None => {
                prompt.insert("content".to_owned(), Value::Array(blocks));
            }
        },
        None => {
            params.insert(
                "systemPrompt".to_owned(),
                json!({"mode": "append", "content": blocks}),
            );
        }
    }
}

/// The tool of a tool call: the name the agent keeps in the call's `_meta` at `pointer` (its description's) when it has one, else the
/// call's title.
fn tool_of(call: &Value, pointer: Option<&str>) -> Tool {
    let name = pointer
        .and_then(|pointer| call.get("_meta")?.pointer(pointer))
        .and_then(Value::as_str)
        .or_else(|| call.get("title").and_then(Value::as_str))
        .unwrap_or("tool");
    Tool {
        kind: call
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("other")
            .to_owned(),
        name: name.chars().take(256).collect(),
        input: call.get("rawInput").cloned().unwrap_or_else(|| json!({})),
    }
}

impl<C: Channel + ?Sized> Hosted<'_, C> {
    /// The hooks' session for the agent's `agent`. One the host never saw open (a fork the agent made, say) is taken in on
    /// its first prompt or question, with no allowance of the person's and the host's directory for where it works, so a
    /// guard on prompts and tool calls still reaches it; its `session-start` was not seen and is not fired.
    fn session_for(&self, agent: &str) -> SessionHooks {
        let mut hold = self.hold();
        if let Some(known) = hold.book.sessions.get(agent) {
            return known.clone();
        }
        let mut random = [0_u8; 12];
        let _ = getrandom::fill(&mut random);
        let session = SessionHooks {
            id: format!("canoa-session-{}", data_encoding::HEXLOWER.encode(&random)),
            allowed: BTreeSet::new(),
            cwd: self
                .connection
                .host
                .directory
                .to_string_lossy()
                .into_owned(),
            root: String::new(),
            fed: Vec::new(),
            follows: false,
        };
        if hold.book.sessions.len() < MAX_TRACKED {
            hold.book.sessions.insert(agent.to_owned(), session.clone());
        }
        session
    }

    /// Where the harness keeps a tool's own name in a tool call's `_meta`, from its description.
    fn tool_name_pointer(&self) -> Option<&'static str> {
        crate::vendors::by_aos(&self.harness_label())?.tool_name
    }

    /// The harness as the profile's report names it.
    fn harness_label(&self) -> String {
        self.hold()
            .parts
            .as_ref()
            .and_then(|parts| parts.report.get("harness")?.as_str().map(str::to_owned))
            .unwrap_or_else(|| self.connection.entry.clone())
    }

    /// Fires `event` for a session and logs what went wrong; the hooks outside the session's request cannot report to a
    /// client, so the notes wait for the next report as well.
    fn fire(
        &self,
        event: HookEvent,
        session: &SessionHooks,
        extra: &[(&str, Value)],
        select: Option<&str>,
        applies: &[&str],
    ) -> FiredEvent {
        let mut known = Map::new();
        known.insert("harness".to_owned(), json!(self.harness_label()));
        known.insert("cwd".to_owned(), json!(session.cwd));
        known.insert("session".to_owned(), json!(session.id));
        for (key, value) in extra {
            known.insert((*key).to_owned(), value.clone());
        }
        let context_used = self.hold().book.used(&session.id);
        let fired = self.connection.hooks.fire(
            &self.connection.overlays,
            &Firing {
                event,
                known: &known,
                select,
                person: &session.allowed,
                applies,
                pre_env: &self.connection.pre_env.runs,
                context_used,
            },
        );
        for report in &fired.reports {
            tracing::warn!(
                entry = self.connection.entry,
                "{}: {}",
                report.id,
                report.about.join(", ")
            );
        }
        fired
    }

    /// The client's message, before it goes to the agent: `Err` is the answer that ends it here.
    pub(in crate::profile::hosted) fn on_client(
        &self,
        bytes: Vec<u8>,
        offered: &BTreeSet<String>,
    ) -> Result<Vec<u8>, Vec<u8>> {
        let Ok(mut value) = serde_json::from_slice::<Value>(&bytes) else {
            return Ok(bytes);
        };
        let method = value
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let id = value.get("id").cloned();
        // A client's answer to a request approvers were answering ends their wait; one that comes after they settled it is dropped,
        // since the agent has its answer. A cancelled turn ends the waits of its session.
        if method.is_empty() && id.as_ref().is_some_and(|id| self.client_answered(id)) {
            return Err(Vec::new());
        }
        if method == "session/cancel" {
            let agent = value.pointer("/params/sessionId").and_then(Value::as_str);
            self.end_asking(agent);
        }
        let rewritten = if SESSION_METHODS.contains(&method.as_str()) {
            self.session_start(&mut value, &method, id.as_ref(), offered)?
        } else if method == "session/prompt" && self.tracking() {
            self.prompt(&mut value, id.as_ref())?
        } else if method == "session/close" && self.tracking() {
            self.session_end(&value);
            false
        } else {
            false
        };
        if !rewritten {
            return Ok(bytes);
        }
        serde_json::to_vec(&value).map_err(|_| Vec::new())
    }

    /// Whether the host has anything to do with what the agent says and what the client asks after a session opens: an overlay has
    /// hooks, or a session it opened has something to follow or to report.
    fn tracking(&self) -> bool {
        self.connection.hooks.any() || !self.connection.host.approvers.is_empty() || {
            let hold = self.hold();
            !hold.book.sessions.is_empty() || !hold.book.opening.is_empty()
        }
    }

    /// A session opens. `Ok(true)` when the request was changed.
    fn session_start(
        &self,
        value: &mut Value,
        method: &str,
        id: Option<&Value>,
        allowed: &BTreeSet<String>,
    ) -> Result<bool, Vec<u8>> {
        let Some(id) = id else {
            return Ok(false);
        };
        let Some(params) = value.get_mut("params").and_then(Value::as_object_mut) else {
            return Ok(false);
        };
        let named = (method != "session/new")
            .then(|| params.get("sessionId").and_then(Value::as_str))
            .flatten()
            .map(str::to_owned);
        let mut random = [0_u8; 12];
        let _ = getrandom::fill(&mut random);
        // A session the host already holds keeps its record, its id and its allowances.
        let known = named
            .as_ref()
            .and_then(|agent| self.hold().book.sessions.get(agent).cloned());
        let mut session = known.unwrap_or_else(|| SessionHooks {
            id: format!("canoa-session-{}", data_encoding::HEXLOWER.encode(&random)),
            allowed: allowed.clone(),
            cwd: params
                .get("cwd")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            root: String::new(),
            fed: Vec::new(),
            follows: false,
        });
        let takes = self
            .hold()
            .parts
            .as_ref()
            .is_some_and(|parts| parts.system_prompt_taken);
        let into_system = method == "session/new" && takes;
        let opened_in = params
            .get("cwd")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let project = self.open_project(&mut session, &opened_in, into_system);
        let mut changed = false;
        if !project.files.is_empty() {
            if into_system {
                append_system(params, project_context::blocks(&project.files));
                changed = true;
            } else {
                self.hold()
                    .book
                    .project
                    .entry(session.id.clone())
                    .or_default()
                    .extend(project.files);
            }
        }
        let allowed = &session.allowed;
        let reason = if method == "session/new" {
            "new"
        } else {
            "resume"
        };
        let mut notes = project.items;
        if self
            .connection
            .hooks
            .listens(HookEvent::SessionStart, allowed)
        {
            let fired = self.fire(
                HookEvent::SessionStart,
                &session,
                &[("reason", json!(reason))],
                Some(reason),
                &SESSION_START,
            );
            notes.extend(fired.reports.iter().map(report_item));
            if fired.decision == Some(Decision::Refuse) {
                tracing::info!(
                    entry = self.connection.entry,
                    "a session was refused: an overlay's hook that guards sessions failed"
                );
                return Err(error(
                    id,
                    SERVER_ERROR,
                    "an overlay's hook that guards sessions failed, so the session did not open",
                ));
            }
            let context = self.hold().book.took(&session.id, fired.context);
            if !context.is_empty() {
                if into_system {
                    append_system(params, blocks_of(&context, "session-start"));
                    changed = true;
                } else {
                    // Where the agent takes no system prompt, or the session already has one, the first prompt carries it.
                    self.hold()
                        .book
                        .carry
                        .entry(session.id.clone())
                        .or_default()
                        .extend(context);
                }
            }
        }
        let plugin = self.native_plugin(params, &session);
        let (native, failed) = match plugin {
            native::Plugin::Nothing => (None, None),
            native::Plugin::Kept => {
                changed = true;
                (None, None)
            }
            native::Plugin::Added(enrolled) => {
                changed = true;
                (Some(enrolled), None)
            }
            native::Plugin::Failed { why, guarded } => {
                if guarded {
                    tracing::info!(
                        entry = self.connection.entry,
                        "a session was refused: the hooks of its harness that an overlay's guard needs could not be written: {why}"
                    );
                    return Err(error(
                        id,
                        SERVER_ERROR,
                        "an overlay's hook that guards this session could not be given to its harness, so the session did not open",
                    ));
                }
                (None, Some(why))
            }
        };
        notes.extend(self.connection.hooks.items_with(
            allowed,
            Some(&self.harness_label()),
            failed.as_deref(),
        ));
        // With no overlay hook to run and no project context to follow or to report, there is nothing to keep of the session.
        if !self.connection.hooks.any() && !session.follows && notes.is_empty() && native.is_none()
        {
            return Ok(changed);
        }
        let mut hold = self.hold();
        // A request the agent never answers is not kept for ever, nor is the channel it was given.
        if hold.book.opening.len() >= MAX_TRACKED {
            let dropped = core::mem::take(&mut hold.book.opening);
            for each in dropped.into_values() {
                if let (Some(forward), Some(native)) = (&hold.book.forward, &each.native) {
                    forward.revoke_secret(&native.secret);
                }
            }
            hold.book.notes.clear();
        }
        hold.book.opening.insert(
            id.to_string(),
            Opening {
                native,
                hooks: session,
                named,
                notes,
            },
        );
        Ok(changed)
    }

    /// A prompt is sent. `Ok(true)` when it was changed.
    fn prompt(&self, value: &mut Value, id: Option<&Value>) -> Result<bool, Vec<u8>> {
        let Some(id) = id else {
            return Ok(false);
        };
        let Some(params) = value.get_mut("params").and_then(Value::as_object_mut) else {
            return Ok(false);
        };
        let Some(agent) = params
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            return Ok(false);
        };
        let session = self.session_for(&agent);
        let mut fresh = Vec::new();
        if self
            .connection
            .hooks
            .listens(HookEvent::Prompt, &session.allowed)
        {
            let text = prompt_text(params.get("prompt"));
            let fired = self.fire(
                HookEvent::Prompt,
                &session,
                &[("prompt", json!(text))],
                None,
                &PROMPT,
            );
            self.hold()
                .book
                .remember(fired.reports.iter().map(report_item));
            if fired.decision == Some(Decision::Block) {
                let by = fired.by.unwrap_or_default();
                let why = fired.reason.map_or_else(
                    || "it failed".to_owned(),
                    |reason| format!("saying: {reason}"),
                );
                tracing::info!(
                    entry = self.connection.entry,
                    "a prompt was blocked by an overlay's hook"
                );
                let notice = json!({"jsonrpc": "2.0", "method": "session/update", "params": {
                    "sessionId": agent,
                    "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text",
                        "text": format!("The hook of the overlay {by} blocked this message, {why}")}},
                }});
                if let Ok(notice) = serde_json::to_vec(&notice) {
                    let _ = self.reply(&notice);
                }
                let answer =
                    json!({"jsonrpc": "2.0", "id": id, "result": {"stopReason": "refusal"}});
                return Err(serde_json::to_vec(&answer).unwrap_or_default());
            }
            fresh = self.hold().book.took(&session.id, fired.context);
        }
        let mut context = self
            .hold()
            .book
            .carry
            .remove(&session.id)
            .unwrap_or_default();
        context.extend(fresh);
        let files = self
            .hold()
            .book
            .project
            .remove(&session.id)
            .unwrap_or_default();
        let mut changed = false;
        if !context.is_empty() || !files.is_empty() {
            // After the user's own blocks, so the prompt still begins as the user wrote it.
            let mut blocks = project_context::blocks(&files);
            blocks.extend(blocks_of(&context, "hook"));
            if let Some(prompt) = params.get_mut("prompt").and_then(Value::as_array_mut) {
                prompt.extend(blocks);
                changed = true;
            }
        }
        let mut hold = self.hold();
        if hold.book.turns.len() < MAX_TRACKED {
            hold.book.turns.insert(id.to_string(), agent);
        }
        Ok(changed)
    }

    /// A session closes.
    fn session_end(&self, value: &Value) {
        let Some(agent) = value
            .pointer("/params/sessionId")
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            return;
        };
        self.end_asking(Some(&agent));
        let session = self.hold().book.sessions.remove(&agent);
        if let Some(session) = session {
            self.end(&session);
        }
    }

    fn end(&self, session: &SessionHooks) {
        {
            let mut hold = self.hold();
            hold.book.carry.remove(&session.id);
            hold.book.project.remove(&session.id);
        }
        self.revoke_native(&session.id);
        if self
            .connection
            .hooks
            .listens(HookEvent::SessionEnd, &session.allowed)
        {
            self.fire(HookEvent::SessionEnd, session, &[], None, &NOTHING);
        }
    }

    /// The connection ends: every session still open ends with it, best effort.
    pub(in crate::profile::hosted) fn end_all(&self) {
        self.end_asking(None);
        let open: Vec<SessionHooks> = {
            let mut hold = self.hold();
            core::mem::take(&mut hold.book.sessions)
                .into_values()
                .collect()
        };
        for session in open {
            self.end(&session);
        }
    }

    /// The agent's message, before it goes to the client.
    pub(in crate::profile::hosted) fn on_agent(&self, message: &[u8]) -> Seen {
        if !self.tracking() {
            return Seen::Passed(None);
        }
        let Ok(value) = serde_json::from_slice::<Value>(message) else {
            return Seen::Passed(None);
        };
        let id = value.get("id").cloned();
        match value.get("method").and_then(Value::as_str) {
            None => Seen::Passed(self.answered(&value)),
            Some("session/request_permission") if id.is_some() => self.asked(&value, id.as_ref()),
            Some("session/update") => Seen::Passed(self.updated(&value)),
            Some(_) => Seen::Passed(None),
        }
    }

    /// An answer: the session a request opened, or the end of a turn.
    fn answered(&self, value: &Value) -> Option<Later> {
        let key = value.get("id")?.to_string();
        let mut hold = self.hold();
        if let Some(opening) = hold.book.opening.remove(&key) {
            let agent = opening.named.clone().or_else(|| {
                value
                    .pointer("/result/sessionId")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            });
            let mut keep = None;
            if value.get("error").is_none()
                && let Some(agent) = agent
            {
                // A session already held keeps its record: loading it again does not replace it.
                if hold.book.sessions.len() < MAX_TRACKED || hold.book.sessions.contains_key(&agent)
                {
                    keep = Some(opening.hooks.id.clone());
                    hold.book.sessions.entry(agent).or_insert(opening.hooks);
                }
            }
            // A request that ended in an error, or whose session the table has no room for, ends its channel and plugin.
            if let Some(native) = opening.native {
                match keep {
                    Some(id) => hold
                        .book
                        .plugins
                        .entry(id)
                        .or_default()
                        .push((native.written, native.meta)),
                    None => {
                        if let Some(forward) = &hold.book.forward {
                            forward.revoke_secret(&native.secret);
                        }
                    }
                }
            }
            hold.book.notes.insert(key, opening.notes);
            return None;
        }
        let agent = hold.book.turns.remove(&key)?;
        Some(Later::TurnEnd(agent))
    }

    /// The agent asks permission for a tool call: the overlays' `pre-tool` hooks first, then the approvers the person added.
    fn asked(&self, value: &Value, id: Option<&Value>) -> Seen {
        let id = id.cloned().unwrap_or(Value::Null);
        let params = value.get("params").unwrap_or(&Value::Null);
        let Some(agent) = params.get("sessionId").and_then(Value::as_str) else {
            return Seen::Passed(None);
        };
        let listens = |session: &SessionHooks| {
            self.connection
                .hooks
                .listens(HookEvent::PreTool, &session.allowed)
        };
        let approvers = !self.connection.host.approvers.is_empty();
        let session = self.session_for(agent);
        if !approvers && !listens(&session) {
            return Seen::Passed(None);
        }
        // ACP v1 puts the call at `toolCall`, v2 at `subject.toolCall`; a v2 subject that is no tool call (a command, say)
        // has none, and no allow answers for what the host cannot read.
        let call = params
            .get("toolCall")
            .or_else(|| params.pointer("/subject/toolCall"))
            .filter(|call| call.is_object());
        let tool = tool_of(call.unwrap_or(&Value::Null), self.tool_name_pointer());
        let options = params
            .get("options")
            .and_then(Value::as_array)
            .map(Vec::as_slice);
        // An allow never answers for a place the person protects, a request to widen the sandbox, or a call the host cannot read:
        // those go to the person as they would have.
        let protected = call.is_none_or(|call| {
            self.connection
                .hooks
                .protects(call, Path::new(&session.cwd))
        });
        if listens(&session)
            && let Some(seen) = self.hooked(&id, &session, &tool, options, protected)
        {
            return seen;
        }
        if !approvers {
            return Seen::Passed(None);
        }
        let asked = Asked {
            id: &id,
            agent,
            session: &session,
            call,
            tool: &tool,
            options,
            protected,
        };
        self.approve(&asked)
    }

    /// The hooks' answer to a request, when they give one; none when the question goes on. No decision is not an approval: it goes to
    /// the client as it would have without the hooks, and `ask` is that same question.
    fn hooked(
        &self,
        id: &Value,
        session: &SessionHooks,
        tool: &Tool,
        options: Option<&[Value]>,
        protected: bool,
    ) -> Option<Seen> {
        let fired = self.fire(
            HookEvent::PreTool,
            session,
            &[(
                "tool",
                json!({"kind": tool.kind, "name": tool.name, "input": tool.input}),
            )],
            Some(&tool.kind),
            &PRE_TOOL,
        );
        let mut hold = self.hold();
        hold.book.remember(fired.reports.iter().map(report_item));
        let outcome = match fired.decision {
            Some(Decision::Deny) => approve::options::reject(options),
            Some(Decision::Allow) => {
                // An allow answers for the person only with the option that allows this once, never one that remembers the
                // choice.
                let Some(allowing) = approve::options::allow_once(options).filter(|_| !protected)
                else {
                    let by = fired.by.clone().unwrap_or_default();
                    let why = if protected {
                        "the call names a place the person protects, asks to widen the sandbox, or is not one the host can read"
                    } else {
                        "the agent offers no unambiguous option that allows it once"
                    };
                    hold.book.remember([Item::new(
                        format!("hooks.{by}.pre-tool"),
                        LayerId::new(LayerKind::Overlay, by),
                        State::Unsupported,
                        format!("hook-not-applied: allow, {why}, so the person is asked"),
                    )]);
                    return None;
                };
                allowing
            }
            _ => return None,
        };
        if let Some(answer) = approve::options::answer(id, &outcome) {
            hold.book.inject.push_back(answer);
        }
        tracing::info!(
            entry = self.connection.entry,
            "a tool call was {} by an overlay's hook",
            if fired.decision == Some(Decision::Allow) {
                "allowed"
            } else {
                "denied"
            }
        );
        Some(Seen::Dropped)
    }

    /// Puts a request to the approvers the person added: the ones that answer on the spot first, in order, and then the ones that
    /// wait for someone beside the window, whose first answer is read by `settle_approvals`.
    fn approve(&self, asked: &Asked<'_>) -> Seen {
        let host = &self.connection.host;
        let chain = {
            let mut hold = self.hold();
            Arc::clone(hold.book.chain.get_or_insert_with(|| {
                let catalog = Catalog::of(host.items.registries());
                Arc::new(Chain::new(&host.approvers, &catalog))
            }))
        };
        if chain.is_empty() {
            return Seen::Passed(None);
        }
        let mut random = [0_u8; 8];
        let _ = getrandom::fill(&mut random);
        let mut request = self.request_of(
            format!("canoa-approval-{}", data_encoding::HEXLOWER.encode(&random)),
            asked,
        );
        request.protected = asked.protected;
        let can_allow = approve::options::allow_once(asked.options).is_some();
        if let Some((by, choice)) = chain.instant(&request, can_allow) {
            let outcome = match choice {
                Choice::Allow => approve::options::allow_once(asked.options),
                Choice::Reject => None,
            }
            .unwrap_or_else(|| approve::options::reject(asked.options));
            let settled = Settled {
                by,
                outcome: match choice {
                    Choice::Allow => Outcome::Allowed,
                    Choice::Reject => Outcome::Rejected,
                },
            };
            let mut hold = self.hold();
            if let Some(answer) = approve::options::answer(asked.id, &outcome) {
                hold.book.inject.push_back(answer);
            }
            if let Some(notice) = approve::notice::approved(asked.agent, asked.call, &settled) {
                hold.book.to_client.push_back(notice);
            }
            tracing::info!(
                entry = self.connection.entry,
                by = settled.by,
                "a permission request was answered by an approver"
            );
            return Seen::Dropped;
        }
        let key = asked.id.to_string();
        let outbox = Arc::clone(&self.hold().book.outbox);
        let settle: Settle = {
            let key = key.clone();
            Arc::new(move |settled| {
                outbox
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push_back((key.clone(), settled));
            })
        };
        // The window is attached while the client's connection is; whether it reports attending the session comes with the
        // notifications' `notify/attended`.
        let presence = Presence {
            window: true,
            attended: false,
        };
        if let Some(waiting) = chain.asking(&request, can_allow, presence, settle) {
            let mut hold = self.hold();
            if hold.book.asking.len() < MAX_TRACKED {
                hold.book.asking.insert(
                    key,
                    Pending {
                        asking: waiting,
                        id: asked.id.clone(),
                        agent: asked.agent.to_owned(),
                        call: asked.call.cloned(),
                        options: asked.options.map(<[Value]>::to_vec).unwrap_or_default(),
                    },
                );
            }
        }
        Seen::Passed(None)
    }

    /// What the approvers have answered since the last look: the agent is answered with it, and the client is told the request is
    /// closed and by whom. Called before a message of the client's is read.
    pub(in crate::profile::hosted) fn settle_approvals(&self) -> Result<(), Failure> {
        let notices: Vec<Vec<u8>> = {
            let mut hold = self.hold();
            if hold.book.asking.is_empty() && hold.book.to_client.is_empty() {
                return Ok(());
            }
            let settled: Vec<(String, Settled)> = hold
                .book
                .outbox
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .drain(..)
                .collect();
            for (key, settled) in settled {
                // A request the window answered first is no longer here: its late answer settles nothing.
                let Some(pending) = hold.book.asking.remove(&key) else {
                    continue;
                };
                let options = Some(pending.options.as_slice());
                let outcome = match settled.outcome {
                    Outcome::Allowed => approve::options::allow_once(options)
                        .unwrap_or_else(|| approve::options::reject(options)),
                    Outcome::Rejected | Outcome::Refused => approve::options::reject(options),
                };
                if let Some(answer) = approve::options::answer(&pending.id, &outcome) {
                    hold.book.inject.push_back(answer);
                }
                for notice in [
                    approve::notice::approved(&pending.agent, pending.call.as_ref(), &settled),
                    approve::notice::closed(&pending.agent, &pending.id, &settled, &outcome),
                ]
                .into_iter()
                .flatten()
                {
                    hold.book.to_client.push_back(notice);
                }
                if hold.book.settled.len() >= MAX_TRACKED {
                    hold.book.settled.clear();
                }
                hold.book.settled.insert(key);
                tracing::info!(
                    entry = self.connection.entry,
                    by = settled.by,
                    "a permission request was settled without the window"
                );
            }
            hold.book.to_client.drain(..).collect()
        };
        for notice in notices {
            self.send_profiled(&notice)?;
        }
        Ok(())
    }

    /// The client answered the request `id` itself, or sent a message the approvers' answer already made unnecessary: true when
    /// the message is a late answer to a request an approver settled, which the agent must not get twice.
    fn client_answered(&self, id: &Value) -> bool {
        let key = id.to_string();
        let mut hold = self.hold();
        if let Some(pending) = hold.book.asking.remove(&key) {
            // The window answered first: every approver withdraws what it waits for.
            pending.asking.end();
            return false;
        }
        // Every late answer is dropped, not only the first: two clients of a shared session may each send one.
        hold.book.settled.contains(&key)
    }

    /// Ends the requests of the agent's session `agent` that approvers are answering (a cancelled turn, a closed session), or of
    /// every session when none is named.
    fn end_asking(&self, agent: Option<&str>) {
        let mut hold = self.hold();
        let keys: Vec<String> = hold
            .book
            .asking
            .iter()
            .filter(|(_, pending)| agent.is_none_or(|agent| pending.agent == agent))
            .map(|(key, _)| key.clone())
            .collect();
        for key in keys {
            if let Some(pending) = hold.book.asking.remove(&key) {
                pending.asking.end();
            }
        }
    }

    /// The request as an approver is shown it: the tool's kind, name and title, the command and the placed files (which an approver
    /// is given only when its entry asks), and the options without their ids, every text of the agent's made safe to show.
    fn request_of(&self, id: String, asked: &Asked<'_>) -> approver::Request {
        let title = asked
            .call
            .and_then(|call| call.get("title"))
            .and_then(Value::as_str)
            .unwrap_or(&asked.tool.name);
        let mut tool = approver::Tool::new(&asked.tool.kind, &asked.tool.name, title);
        tool.command = asked
            .tool
            .input
            .get("command")
            .and_then(Value::as_str)
            .map(str::to_owned);
        tool.paths = asked
            .call
            .map(|call| allow::placed(call, Path::new(&asked.session.cwd)))
            .unwrap_or_default();
        let mut request = approver::Request::new(
            id,
            approver::SessionRef::new(&asked.session.id, &self.connection.entry),
            tool,
        );
        request.options = approve::options::refs(asked.options)
            .into_iter()
            .map(|(kind, name)| approver::OptionRef::new(kind, name))
            .collect();
        request.protected = asked.protected;
        request.clean()
    }

    /// A session update: the start of a tool call is remembered, its end is `post-tool`.
    fn updated(&self, value: &Value) -> Option<Later> {
        let params = value.get("params")?;
        let agent = params.get("sessionId").and_then(Value::as_str)?.to_owned();
        let update = params.get("update")?;
        let call = update.get("toolCallId").and_then(Value::as_str)?.to_owned();
        let kind = update.get("sessionUpdate").and_then(Value::as_str)?;
        let key = (agent.clone(), call);
        let pointer = self.tool_name_pointer();
        let mut hold = self.hold();
        match kind {
            "tool_call" => {
                if hold.book.tools.len() >= MAX_TRACKED {
                    hold.book.tools.clear();
                }
                hold.book.tools.insert(key, tool_of(update, pointer));
                None
            }
            "tool_call_update" => {
                let status = match update.get("status").and_then(Value::as_str) {
                    Some("completed") => "completed",
                    Some("failed") => "failed",
                    _ => return None,
                };
                let mut tool = hold
                    .book
                    .tools
                    .remove(&key)
                    .unwrap_or_else(|| tool_of(update, pointer));
                if tool.kind == "other"
                    && let Some(kind) = update.get("kind").and_then(Value::as_str)
                {
                    kind.clone_into(&mut tool.kind);
                }
                Some(Later::PostTool(agent, tool, status))
            }
            _ => None,
        }
    }

    /// What follows a message passed on to the client.
    pub(in crate::profile::hosted) fn later(&self, later: Later) {
        match later {
            Later::TurnEnd(agent) => {
                let Some(session) = self.hold().book.sessions.get(&agent).cloned() else {
                    return;
                };
                if self
                    .connection
                    .hooks
                    .listens(HookEvent::TurnEnd, &session.allowed)
                {
                    let fired = self.fire(HookEvent::TurnEnd, &session, &[], None, &NOTHING);
                    self.hold()
                        .book
                        .remember(fired.reports.iter().map(report_item));
                }
            }
            Later::PostTool(agent, tool, status) => {
                let Some(session) = self.hold().book.sessions.get(&agent).cloned() else {
                    return;
                };
                if !self
                    .connection
                    .hooks
                    .listens(HookEvent::PostTool, &session.allowed)
                {
                    return;
                }
                let fired = self.fire(
                    HookEvent::PostTool,
                    &session,
                    &[
                        (
                            "tool",
                            json!({"kind": tool.kind, "name": tool.name, "input": tool.input}),
                        ),
                        ("status", json!(status)),
                    ],
                    Some(&tool.kind),
                    &POST_TOOL,
                );
                let mut hold = self.hold();
                hold.book.remember(fired.reports.iter().map(report_item));
                let context = hold.book.took(&session.id, fired.context);
                if !context.is_empty() {
                    hold.book
                        .carry
                        .entry(session.id)
                        .or_default()
                        .extend(context);
                }
            }
        }
    }
}
