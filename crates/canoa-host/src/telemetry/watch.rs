//! What one connection tells the telemetry hub: the sessions, turns, tool calls and permission requests of the ACP messages that pass
//! through it, as typed signals.
//!
//! The observer reads a message only to find ids, closed words and numbers in it: a session id, the kind of a tool, a stop reason, the
//! tokens an agent reported. No text of the message is kept or sent: the text a prompt, a reply, a tool's input or output, a title
//! or a path holds never reaches a signal, because no signal has a place for it. The two names it does take from an agent (a model
//! id, a tool's name) go through [`Text`].
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "locks are std's"
)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use coqui_acp_api::{Spend, USAGE_NOTIFICATION, UsageReport};
use serde_json::Value;

use super::hub::{Hub, now_ns};
use super::signal::{
    Ctx, DecidedBy, Outcome, SessionEnd, Signal, StopReason, Text, TokenType, ToolKind, ToolStatus,
    TurnTokens,
};
use crate::usage_splice::tap::{
    config_option_value, has_spend, merge_acp_spend, parse_acp_usage, parse_cost_micro_usd,
    parse_state_spend,
};

/// How many open requests, sessions and calls one connection keeps track of; past it a new one is not followed.
const MAX_OPEN: usize = 256;

pub(super) fn random() -> u64 {
    let mut bytes = [0_u8; 8];
    if getrandom::fill(&mut bytes).is_err() {
        // A clock is a poor source, but ids only tell spans apart.
        return u64::try_from(now_ns()).unwrap_or(1);
    }
    u64::from_le_bytes(bytes)
}

struct Call {
    start: i64,
    kind: ToolKind,
    name: Option<Text>,
}

struct Turn {
    start: i64,
    spend: Option<Spend>,
}

struct Session {
    key: u64,
    trace: u128,
    span: u64,
    start: i64,
    model: Option<String>,
    turn: Option<Turn>,
    calls: BTreeMap<String, Call>,
}

struct Ask {
    session: String,
    start: i64,
    kind: ToolKind,
    /// Each option's id and whether choosing it allows.
    allows: Vec<(String, bool)>,
}

#[derive(Default)]
struct Inner {
    /// `session/new` and `session/load` requests waiting for their answer, with the session a load names.
    opening: BTreeMap<String, Option<String>>,
    /// `session/prompt` requests waiting for theirs, with the session each is for.
    prompts: BTreeMap<String, String>,
    /// `session/close` requests waiting for theirs.
    closing: BTreeMap<String, String>,
    /// Requests for permission waiting for the client's answer.
    asks: BTreeMap<String, Ask>,
    sessions: BTreeMap<String, Session>,
}

/// A connection's side of the telemetry.
pub(crate) struct Watch {
    hub: Arc<Hub>,
    harness: Text,
    inner: Mutex<Inner>,
}

impl core::fmt::Debug for Watch {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Watch").finish_non_exhaustive()
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn id_of(value: &Value) -> Option<String> {
    value.get("id").map(ToString::to_string)
}

fn stop_of(word: &str) -> StopReason {
    match word {
        "end_turn" => StopReason::EndTurn,
        "max_tokens" => StopReason::MaxTokens,
        "max_turn_requests" => StopReason::MaxTurnRequests,
        "cancelled" => StopReason::Cancelled,
        "refusal" => StopReason::Refusal,
        _ => StopReason::Error,
    }
}

fn kind_of(value: Option<&Value>) -> ToolKind {
    value
        .and_then(Value::as_str)
        .and_then(ToolKind::parse)
        .unwrap_or(ToolKind::Other)
}

/// The name an adapter gives a tool in the update's `_meta` (`toolName`), when it does.
fn tool_name(update: &Value) -> Option<Text> {
    update
        .get("_meta")?
        .as_object()?
        .values()
        .find_map(|vendor| vendor.get("toolName")?.as_str())
        .and_then(Text::maybe)
}

fn count(value: u64) -> Option<u64> {
    (value > 0).then_some(value)
}

impl Watch {
    /// A watch on a connection of `harness`, when the host exports telemetry.
    pub(crate) fn new(harness: &str) -> Option<Self> {
        super::current().map(|hub| Self::with_hub(hub, harness))
    }

    pub(crate) fn with_hub(hub: Arc<Hub>, harness: &str) -> Self {
        Self {
            hub,
            harness: Text::new(harness),
            inner: Mutex::new(Inner::default()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Reads a message from the client: a request that opens, prompts or closes a session, or the answer to a request for permission.
    pub(crate) fn heard_client(&self, message: &[u8]) {
        let session = contains(message, b"session/");
        if !(session || contains(message, b"\"outcome\"")) {
            return;
        }
        let Ok(value) = serde_json::from_slice::<Value>(message) else {
            return;
        };
        let now = now_ns();
        let mut inner = self.lock();
        if let Some(method) = value.get("method").and_then(Value::as_str) {
            let id = id_of(&value);
            let named = value
                .pointer("/params/sessionId")
                .and_then(Value::as_str)
                .map(str::to_owned);
            match (method, id, named) {
                ("session/new", Some(id), _) if inner.opening.len() < MAX_OPEN => {
                    inner.opening.insert(id, None);
                }
                ("session/load", Some(id), named) if inner.opening.len() < MAX_OPEN => {
                    inner.opening.insert(id, named);
                }
                ("session/prompt", Some(id), Some(named)) => {
                    if inner.prompts.len() < MAX_OPEN {
                        inner.prompts.insert(id, named.clone());
                    }
                    if let Some(open) = inner.sessions.get_mut(&named) {
                        open.turn = Some(Turn {
                            start: now,
                            spend: None,
                        });
                    }
                }
                ("session/close", Some(id), Some(named)) if inner.closing.len() < MAX_OPEN => {
                    inner.closing.insert(id, named);
                }
                ("session/set_model", _, Some(named)) => {
                    if let (Some(open), Some(model)) = (
                        inner.sessions.get_mut(&named),
                        value.pointer("/params/modelId").and_then(Value::as_str),
                    ) {
                        open.model = Some(model.to_owned());
                    }
                }
                ("session/set_config_option", _, Some(named))
                    if value.pointer("/params/configId").and_then(Value::as_str)
                        == Some("model") =>
                {
                    if let (Some(open), Some(model)) = (
                        inner.sessions.get_mut(&named),
                        value.pointer("/params/value").and_then(Value::as_str),
                    ) {
                        open.model = Some(model.to_owned());
                    }
                }
                _ => {}
            }
        } else if let Some(id) = id_of(&value)
            && let Some(ask) = inner.asks.remove(&id)
        {
            let outcome = match value
                .pointer("/result/outcome/outcome")
                .and_then(Value::as_str)
            {
                Some("selected") => {
                    let option = value
                        .pointer("/result/outcome/optionId")
                        .and_then(Value::as_str);
                    match ask
                        .allows
                        .iter()
                        .find(|(each, _)| Some(each.as_str()) == option)
                    {
                        Some((_, true)) => Outcome::Allow,
                        Some((_, false)) => Outcome::Reject,
                        None => Outcome::Cancelled,
                    }
                }
                _ => Outcome::Cancelled,
            };
            let ctx = inner
                .sessions
                .get(&ask.session)
                .map(|open| (open.trace, open.span, open.key));
            drop(inner);
            self.permission(ctx, outcome, ask.kind, ask.start, now);
        }
    }

    fn permission(
        &self,
        ctx: Option<(u128, u64, u64)>,
        outcome: Outcome,
        kind: ToolKind,
        start: i64,
        end: i64,
    ) {
        // The host does not see who decided: the connected client answered, so it is the person.
        let by = DecidedBy::Person;
        if let Some((trace, parent, key)) = ctx {
            self.hub.emit(Signal::Permission {
                ctx: Ctx {
                    trace,
                    span: random(),
                    parent: Some(parent),
                    session: key,
                },
                outcome,
                by,
                kind,
                start_ns: start,
                end_ns: end,
            });
        }
        self.hub.emit(Signal::Permissions {
            outcome,
            by,
            at: end,
        });
    }

    /// Reads a message from the agent: an update, a request for permission, or the answer to a request of the client.
    pub(crate) fn heard_agent(&self, message: &[u8]) {
        let interesting = contains(message, b"session/update")
            || contains(message, b"session/request_permission")
            || contains(message, USAGE_NOTIFICATION.as_bytes())
            || contains(message, b"\"id\"");
        if !interesting {
            return;
        }
        let Ok(value) = serde_json::from_slice::<Value>(message) else {
            return;
        };
        let now = now_ns();
        match value.get("method").and_then(Value::as_str) {
            Some("session/update") => self.update(&value, now),
            Some("session/request_permission") => self.ask(&value, now),
            Some(USAGE_NOTIFICATION) => self.report(&value),
            Some(_) => {}
            None => self.answer(&value, now),
        }
    }

    fn ask(&self, value: &Value, now: i64) {
        let (Some(id), Some(session)) = (
            id_of(value),
            value.pointer("/params/sessionId").and_then(Value::as_str),
        ) else {
            return;
        };
        let allows = value
            .pointer("/params/options")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|option| {
                Some((
                    option.get("optionId")?.as_str()?.to_owned(),
                    option
                        .get("kind")
                        .and_then(Value::as_str)
                        .is_some_and(|kind| kind.starts_with("allow")),
                ))
            })
            .collect();
        let mut inner = self.lock();
        if inner.asks.len() < MAX_OPEN {
            inner.asks.insert(
                id,
                Ask {
                    session: session.to_owned(),
                    start: now,
                    kind: kind_of(value.pointer("/params/toolCall/kind")),
                    allows,
                },
            );
        }
    }

    fn report(&self, value: &Value) {
        let Some(report) = value
            .pointer("/params/report")
            .and_then(|report| serde_json::from_value::<UsageReport>(report.clone()).ok())
        else {
            return;
        };
        let Some(spend) = report.spend else { return };
        let session = if spend.session_id.is_empty() {
            value
                .pointer("/params/sessionId")
                .and_then(Value::as_str)
                .map(str::to_owned)
        } else {
            Some(spend.session_id.clone())
        };
        let mut inner = self.lock();
        if let Some(turn) = session
            .and_then(|session| inner.sessions.get_mut(&session))
            .and_then(|open| open.turn.as_mut())
        {
            turn.spend = Some(spend);
        }
    }

    fn update(&self, value: &Value, now: i64) {
        let (Some(session), Some(update)) = (
            value.pointer("/params/sessionId").and_then(Value::as_str),
            value.pointer("/params/update"),
        ) else {
            return;
        };
        let mut inner = self.lock();
        let Some(open) = inner.sessions.get_mut(session) else {
            return;
        };
        match update.get("sessionUpdate").and_then(Value::as_str) {
            Some("current_model_update") => {
                if let Some(model) = update.get("currentModelId").and_then(Value::as_str) {
                    open.model = Some(model.to_owned());
                }
            }
            Some("config_option_update") => {
                if let Some(model) = config_option_value(update, "model") {
                    open.model = Some(model.to_owned());
                }
            }
            Some("state_update") => {
                if let (Some(turn), Some(spend)) =
                    (open.turn.as_mut(), parse_state_spend(session, update))
                {
                    match &mut turn.spend {
                        Some(held) => merge_acp_spend(held, &spend),
                        none => *none = Some(spend),
                    }
                }
            }
            Some("usage_update") => {
                if let (Some(turn), Some(cost)) =
                    (open.turn.as_mut(), parse_cost_micro_usd(update.get("cost")))
                {
                    turn.spend
                        .get_or_insert_with(|| Spend::new(session))
                        .cost_micro_usd = Some(cost);
                }
            }
            Some("tool_call") => {
                let Some(call) = update.get("toolCallId").and_then(Value::as_str) else {
                    return;
                };
                if open.calls.len() < MAX_OPEN {
                    open.calls.insert(
                        call.to_owned(),
                        Call {
                            start: now,
                            kind: kind_of(update.get("kind")),
                            name: tool_name(update),
                        },
                    );
                }
                self.maybe_end_call(inner, session, update, now);
            }
            Some("tool_call_update") => {
                let Some(call) = update.get("toolCallId").and_then(Value::as_str) else {
                    return;
                };
                if !open.calls.contains_key(call) {
                    if open.calls.len() >= MAX_OPEN {
                        return;
                    }
                    open.calls.insert(
                        call.to_owned(),
                        Call {
                            start: now,
                            kind: kind_of(update.get("kind")),
                            name: None,
                        },
                    );
                }
                if let Some(held) = open.calls.get_mut(call) {
                    if let Some(name) = tool_name(update) {
                        held.name = Some(name);
                    }
                    if let Some(kind) = update.get("kind") {
                        held.kind = kind_of(Some(kind));
                    }
                }
                self.maybe_end_call(inner, session, update, now);
            }
            _ => {}
        }
    }

    /// Ends the tool call `update` names when it says the call completed or failed.
    fn maybe_end_call(
        &self,
        mut inner: std::sync::MutexGuard<'_, Inner>,
        session: &str,
        update: &Value,
        now: i64,
    ) {
        let status = match update.get("status").and_then(Value::as_str) {
            Some("completed") => ToolStatus::Completed,
            Some("failed") => ToolStatus::Failed,
            _ => return,
        };
        let Some(call_id) = update.get("toolCallId").and_then(Value::as_str) else {
            return;
        };
        let Some(open) = inner.sessions.get_mut(session) else {
            return;
        };
        let Some(call) = open.calls.remove(call_id) else {
            return;
        };
        let (trace, parent, key) = (open.trace, open.span, open.key);
        drop(inner);
        self.hub.emit(Signal::ToolCall {
            ctx: Ctx {
                trace,
                span: random(),
                parent: Some(parent),
                session: key,
            },
            kind: call.kind,
            name: call.name,
            status,
            start_ns: call.start,
            end_ns: now,
        });
        self.hub.emit(Signal::ToolCalls {
            kind: call.kind,
            status,
            at: now,
        });
    }

    fn answer(&self, value: &Value, now: i64) {
        let Some(id) = id_of(value) else { return };
        let mut inner = self.lock();
        if let Some(session) = inner.prompts.remove(&id) {
            self.end_turn(inner, &session, value, now);
        } else if let Some(named) = inner.opening.remove(&id) {
            self.opened(inner, value, named, now);
        } else if let Some(session) = inner.closing.remove(&id) {
            let ended = inner.sessions.remove(&session);
            drop(inner);
            if let Some(ended) = ended {
                self.session_ended(&ended, SessionEnd::Closed, now);
            }
        }
    }

    fn opened(
        &self,
        mut inner: std::sync::MutexGuard<'_, Inner>,
        value: &Value,
        named: Option<String>,
        now: i64,
    ) {
        let Some(result) = value.get("result") else {
            drop(inner);
            self.hub.emit(Signal::SessionFailed {
                reason: super::signal::FailReason::Other,
                at: now,
            });
            return;
        };
        let Some(session) = named.or_else(|| {
            result
                .get("sessionId")
                .and_then(Value::as_str)
                .map(str::to_owned)
        }) else {
            return;
        };
        if inner.sessions.len() >= MAX_OPEN || inner.sessions.contains_key(&session) {
            return;
        }
        let model = result
            .pointer("/models/currentModelId")
            .and_then(Value::as_str)
            .or_else(|| config_option_value(result, "model"))
            .map(str::to_owned);
        inner.sessions.insert(
            session,
            Session {
                key: random(),
                trace: (u128::from(random()) << 64) | u128::from(random()),
                span: random(),
                start: now,
                model,
                turn: None,
                calls: BTreeMap::new(),
            },
        );
        drop(inner);
        let open = self.hub.sessions_changed(self.harness.as_str(), true);
        self.hub.emit(Signal::SessionsActive {
            harness: self.harness.clone(),
            count: open,
            at: now,
        });
    }

    fn end_turn(
        &self,
        mut inner: std::sync::MutexGuard<'_, Inner>,
        session: &str,
        value: &Value,
        now: i64,
    ) {
        let Some(open) = inner.sessions.get_mut(session) else {
            return;
        };
        let turn = open.turn.take();
        let (trace, parent, key) = (open.trace, open.span, open.key);
        let held_model = open.model.clone();
        drop(inner);
        let Some(turn) = turn else { return };
        let stop = if value.get("error").is_some() {
            StopReason::Error
        } else {
            stop_of(
                value
                    .pointer("/result/stopReason")
                    .and_then(Value::as_str)
                    .unwrap_or("end_turn"),
            )
        };
        let mut spend = value
            .pointer("/result/usage")
            .and_then(|usage| parse_acp_usage(session, usage));
        if let Some(reported) = turn.spend {
            match &mut spend {
                None => spend = Some(reported),
                Some(held) => {
                    if held.cost_micro_usd.is_none() {
                        held.cost_micro_usd = reported.cost_micro_usd;
                    }
                    if held.by_model.is_empty() {
                        held.by_model = reported.by_model;
                    }
                }
            }
        }
        let spend = spend.filter(has_spend);
        let model = held_model.as_deref().and_then(Text::maybe).or_else(|| {
            spend
                .as_ref()
                .and_then(|spend| spend.by_model.first())
                .and_then(|each| Text::maybe(&each.model))
        });
        let tokens = spend
            .as_ref()
            .map_or_else(TurnTokens::default, |spend| TurnTokens {
                input: count(spend.input),
                output: count(spend.output),
                cache_read: count(spend.cache_read),
                cache_write: count(spend.cache_write),
            });
        let cost = spend.as_ref().and_then(|spend| spend.cost_micro_usd);
        self.hub.emit(Signal::Turn {
            ctx: Ctx {
                trace,
                span: random(),
                parent: Some(parent),
                session: key,
            },
            stop,
            model: model.clone(),
            tokens,
            cost_micro_usd: cost,
            start_ns: turn.start,
            end_ns: now,
        });
        self.hub.emit(Signal::Turns {
            harness: self.harness.clone(),
            stop,
            at: now,
        });
        #[allow(
            clippy::cast_precision_loss,
            reason = "nanoseconds of a turn, shown in seconds"
        )]
        let seconds = (now - turn.start).max(0) as f64 / 1e9;
        self.hub.emit(Signal::TurnDuration {
            harness: self.harness.clone(),
            seconds,
            at: now,
        });
        let model = model.unwrap_or_else(|| Text::new("unknown"));
        for (token, value) in [
            (TokenType::Input, tokens.input),
            (TokenType::Output, tokens.output),
            (TokenType::CacheRead, tokens.cache_read),
            (TokenType::CacheWrite, tokens.cache_write),
        ] {
            if let Some(count) = value {
                self.hub.emit(Signal::Tokens {
                    harness: self.harness.clone(),
                    model: model.clone(),
                    token,
                    count,
                    at: now,
                });
            }
        }
        if let Some(micro_usd) = cost {
            self.hub.emit(Signal::Cost {
                harness: self.harness.clone(),
                model,
                micro_usd,
                at: now,
            });
        }
    }

    fn session_ended(&self, ended: &Session, end: SessionEnd, now: i64) {
        self.hub.emit(Signal::Session {
            ctx: Ctx {
                trace: ended.trace,
                span: ended.span,
                parent: None,
                session: ended.key,
            },
            harness: self.harness.clone(),
            end,
            start_ns: ended.start,
            end_ns: now,
        });
        let open = self.hub.sessions_changed(self.harness.as_str(), false);
        self.hub.emit(Signal::SessionsActive {
            harness: self.harness.clone(),
            count: open,
            at: now,
        });
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        let now = now_ns();
        let end = if self.hub.is_stopping() {
            SessionEnd::HostStopped
        } else {
            SessionEnd::Closed
        };
        let sessions = core::mem::take(&mut self.lock().sessions);
        for ended in sessions.values() {
            self.session_ended(ended, end, now);
        }
    }
}
