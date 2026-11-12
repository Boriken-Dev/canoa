//! The events Canoa sees over ACP, and the hooks it fires for them. The client's `session/new`, `session/load` and
//! `session/resume` start a session, its `session/prompt` is a prompt and its `session/close` the end of one; the agent's
//! answer to a prompt ends a turn, its `session/request_permission` asks about a tool call, and the `tool_call_update` that
//! ends one reports it done. `cd` and `pre-compact` are not shown over ACP, so nothing here fires them.

use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use coqui_aos::{Decision, FiredEvent, HookEvent};
use coqui_transport::Channel;
use serde_json::{Map, Value, json};

use super::report_item;
use crate::profile::hosted::{Hosted, Item, SESSION_METHODS, error};

/// The outputs Canoa applies for each event over ACP.
const SESSION_START: [&str; 2] = ["context", "refuse"];
const PROMPT: [&str; 2] = ["context", "block"];
const PRE_TOOL: [&str; 2] = ["deny", "ask"];
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
}

/// A session request on its way to the agent.
#[derive(Debug)]
struct Opening {
    hooks: SessionHooks,
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
    /// Bytes of hook context each session (by Canoa's id for it) has been given.
    used: BTreeMap<String, usize>,
    /// Tool calls shown and not yet ended, by session and call.
    tools: BTreeMap<(String, String), Tool>,
    /// Messages Canoa answers the agent with, read before the client's.
    inject: VecDeque<Vec<u8>>,
    /// What went wrong with hooks outside a session request, for the reports to come.
    items: Vec<Item>,
}

impl Book {
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

    /// The part of `context` the session's bound admits; the rest is reported.
    fn admit(
        &mut self,
        hooks: &super::Hooks,
        session: &str,
        context: Vec<(String, String)>,
        event: &str,
    ) -> Vec<(String, String)> {
        let used = self.used.entry(session.to_owned()).or_default();
        let (kept, items) = hooks.admit(used, context, event);
        self.remember(items);
        kept
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

fn tool_of(call: &Value) -> Tool {
    let name = call
        .pointer("/_meta/claudeCode/toolName")
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
            id: format!(
                "canoa-session-{}",
                random
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            ),
            allowed: BTreeSet::new(),
            cwd: self
                .connection
                .host
                .directory
                .to_string_lossy()
                .into_owned(),
        };
        if hold.book.sessions.len() < MAX_TRACKED {
            hold.book.sessions.insert(agent.to_owned(), session.clone());
        }
        session
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
        let fired = self.connection.hooks.fire(
            &self.connection.overlays,
            event,
            &known,
            select,
            &session.allowed,
            applies,
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
        if !self.connection.hooks.any() {
            return Ok(bytes);
        }
        let Ok(mut value) = serde_json::from_slice::<Value>(&bytes) else {
            return Ok(bytes);
        };
        let method = value
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let id = value.get("id").cloned();
        let rewritten = if SESSION_METHODS.contains(&method.as_str()) {
            self.session_start(&mut value, &method, id.as_ref(), offered)?
        } else if method == "session/prompt" {
            self.prompt(&mut value, id.as_ref())?
        } else if method == "session/close" {
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
        let session = known.unwrap_or_else(|| SessionHooks {
            id: format!(
                "canoa-session-{}",
                random
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            ),
            allowed: allowed.clone(),
            cwd: params
                .get("cwd")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
        });
        let allowed = &session.allowed;
        let reason = if method == "session/new" {
            "new"
        } else {
            "resume"
        };
        let mut notes = Vec::new();
        let mut changed = false;
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
            let context = self.hold().book.admit(
                &self.connection.hooks,
                &session.id,
                fired.context,
                "session-start",
            );
            if !context.is_empty() {
                let takes = self
                    .hold()
                    .parts
                    .as_ref()
                    .is_some_and(|parts| parts.system_prompt_taken);
                if method == "session/new" && takes {
                    let blocks = blocks_of(&context, "session-start");
                    match params
                        .get_mut("systemPrompt")
                        .and_then(Value::as_object_mut)
                    {
                        Some(prompt) => {
                            match prompt.get_mut("content").and_then(Value::as_array_mut) {
                                Some(content) => content.extend(blocks),
                                None => {
                                    prompt.insert("content".to_owned(), Value::Array(blocks));
                                }
                            }
                        }
                        None => {
                            params.insert(
                                "systemPrompt".to_owned(),
                                json!({"mode": "append", "content": blocks}),
                            );
                        }
                    }
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
        notes.extend(self.connection.hooks.items(allowed));
        let mut hold = self.hold();
        // A request the agent never answers is not kept for ever.
        if hold.book.opening.len() >= MAX_TRACKED {
            hold.book.opening.clear();
            hold.book.notes.clear();
        }
        hold.book.opening.insert(
            id.to_string(),
            Opening {
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
            fresh = self.hold().book.admit(
                &self.connection.hooks,
                &session.id,
                fired.context,
                "prompt",
            );
        }
        let mut context = self
            .hold()
            .book
            .carry
            .remove(&session.id)
            .unwrap_or_default();
        context.extend(fresh);
        let mut changed = false;
        if !context.is_empty() {
            // After the user's own blocks, so the prompt still begins as the user wrote it.
            let blocks = blocks_of(&context, "hook");
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
        let session = self.hold().book.sessions.remove(&agent);
        if let Some(session) = session {
            self.end(&session);
        }
    }

    fn end(&self, session: &SessionHooks) {
        self.hold().book.carry.remove(&session.id);
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
        if !self.connection.hooks.any() {
            return;
        }
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
        if !self.connection.hooks.any() {
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
            if value.get("error").is_none()
                && let Some(agent) = agent
            {
                // A session already held keeps its record: loading it again does not replace it.
                if hold.book.sessions.len() < MAX_TRACKED || hold.book.sessions.contains_key(&agent)
                {
                    hold.book.sessions.entry(agent).or_insert(opening.hooks);
                }
            }
            hold.book.notes.insert(key, opening.notes);
            return None;
        }
        let agent = hold.book.turns.remove(&key)?;
        Some(Later::TurnEnd(agent))
    }

    /// The agent asks permission for a tool call: `pre-tool`.
    fn asked(&self, value: &Value, id: Option<&Value>) -> Seen {
        let id = id.cloned().unwrap_or(Value::Null);
        let params = value.get("params").unwrap_or(&Value::Null);
        let Some(agent) = params.get("sessionId").and_then(Value::as_str) else {
            return Seen::Passed(None);
        };
        let session = self.session_for(agent);
        if !self
            .connection
            .hooks
            .listens(HookEvent::PreTool, &session.allowed)
        {
            return Seen::Passed(None);
        }
        let tool = tool_of(params.get("toolCall").unwrap_or(&Value::Null));
        let fired = self.fire(
            HookEvent::PreTool,
            &session,
            &[(
                "tool",
                json!({"kind": tool.kind, "name": tool.name, "input": tool.input}),
            )],
            Some(&tool.kind),
            &PRE_TOOL,
        );
        let mut hold = self.hold();
        hold.book.remember(fired.reports.iter().map(report_item));
        // No decision is not an approval: the question goes to the client as it would have without the hooks, and `ask` is
        // that same question.
        if !matches!(fired.decision, Some(Decision::Deny)) {
            return Seen::Passed(None);
        }
        let rejecting = params
            .get("options")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|option| option.get("kind").and_then(Value::as_str) == Some("reject_once"))
            .or_else(|| {
                params
                    .get("options")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .find(|option| {
                        option.get("kind").and_then(Value::as_str) == Some("reject_always")
                    })
            })
            .and_then(|option| option.get("optionId").cloned());
        let outcome = match rejecting {
            Some(option) => json!({"outcome": "selected", "optionId": option}),
            None => json!({"outcome": "cancelled"}),
        };
        let answer = json!({"jsonrpc": "2.0", "id": id, "result": {"outcome": outcome}});
        if let Ok(answer) = serde_json::to_vec(&answer) {
            hold.book.inject.push_back(answer);
        }
        tracing::info!(
            entry = self.connection.entry,
            "a tool call was denied by an overlay's hook"
        );
        Seen::Dropped
    }

    /// A session update: the start of a tool call is remembered, its end is `post-tool`.
    fn updated(&self, value: &Value) -> Option<Later> {
        let params = value.get("params")?;
        let agent = params.get("sessionId").and_then(Value::as_str)?.to_owned();
        let update = params.get("update")?;
        let call = update.get("toolCallId").and_then(Value::as_str)?.to_owned();
        let kind = update.get("sessionUpdate").and_then(Value::as_str)?;
        let key = (agent.clone(), call);
        let mut hold = self.hold();
        match kind {
            "tool_call" => {
                if hold.book.tools.len() >= MAX_TRACKED {
                    hold.book.tools.clear();
                }
                hold.book.tools.insert(key, tool_of(update));
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
                    .unwrap_or_else(|| tool_of(update));
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
                let context = hold.book.admit(
                    &self.connection.hooks,
                    &session.id,
                    fired.context,
                    "post-tool",
                );
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
