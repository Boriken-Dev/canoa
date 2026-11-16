//! What one connection tells the usage log: the spend and allowances an agent reports, and the turn each belongs to.
//!
//! The agent's report reaches here before it goes on to the client, whoever speaks for usage. A spend that arrives while a
//! turn of its session is open waits for that turn's answer and is recorded then; one that arrives with no turn open is
//! recorded at once. The log keeps the difference from what it already knew, so a report seen twice adds nothing.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "locks and clocks are std's"
)]

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::sync::Arc;
use std::sync::{Mutex, PoisonError};

use coqui_acp_api::{Spend, USAGE_NOTIFICATION, UsageReport};
use serde_json::Value;

use super::{MAX_OPEN, contains, unix_now};
use crate::usage_log::{Turn, UsageLog};

#[derive(Debug, Default)]
struct Open {
    /// Ids of `session/prompt` requests waiting for their answer, with the session each is for.
    prompts: BTreeMap<String, String>,
    /// Ids of `session/new` and `session/load` requests waiting for theirs, with the session a load names.
    opens: BTreeMap<String, Option<String>>,
    /// The model each session runs, as far as the connection has seen.
    models: BTreeMap<String, String>,
    /// The latest spend reported for a session whose turn is open.
    spent: BTreeMap<String, Spend>,
}

/// A connection's side of the usage log.
#[derive(Debug)]
pub(crate) struct Tap {
    log: Arc<UsageLog>,
    account: String,
    harness: String,
    open: Mutex<Open>,
}

impl Tap {
    pub(crate) fn new(log: Arc<UsageLog>, account: String, harness: String) -> Self {
        Self {
            log,
            account,
            harness,
            open: Mutex::new(Open::default()),
        }
    }

    /// Reads a message from the client: which session a prompt is for, which model a session was given.
    pub(crate) fn heard_client(&self, message: &[u8]) {
        if !contains(message, b"session/") {
            return;
        }
        let Ok(value) = serde_json::from_slice::<Value>(message) else {
            return;
        };
        let (Some(method), Some(params)) = (
            value.get("method").and_then(Value::as_str),
            value.get("params"),
        ) else {
            return;
        };
        let session = params.get("sessionId").and_then(Value::as_str);
        let id = value.get("id").map(ToString::to_string);
        let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        match (method, id, session) {
            ("session/prompt", Some(id), Some(session)) if open.prompts.len() < MAX_OPEN => {
                open.prompts.insert(id, session.to_owned());
            }
            ("session/new" | "session/load", Some(id), _) if open.opens.len() < MAX_OPEN => {
                open.opens.insert(id, session.map(str::to_owned));
            }
            ("session/set_model", _, Some(session)) => {
                if let Some(model) = params.get("modelId").and_then(Value::as_str) {
                    remember(&mut open.models, session, model);
                }
            }
            ("session/set_config_option", _, Some(session))
                if params.get("configId").and_then(Value::as_str) == Some("model") =>
            {
                if let Some(model) = params.get("value").and_then(Value::as_str) {
                    remember(&mut open.models, session, model);
                }
            }
            _ => {}
        }
    }

    /// Reads a message from the agent: a usage report, or the answer that ends a turn or opens a session.
    pub(crate) fn heard_agent(&self, message: &[u8]) {
        let report =
            contains(message, USAGE_NOTIFICATION.as_bytes()) || contains(message, b"\"supported\"");
        let answer = contains(message, b"\"id\"") && {
            let open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
            !(open.prompts.is_empty() && open.opens.is_empty())
        };
        if !(report || answer) {
            return;
        }
        let Ok(value) = serde_json::from_slice::<Value>(message) else {
            return;
        };
        match value.get("method").and_then(Value::as_str) {
            Some(USAGE_NOTIFICATION) => {
                if let Some(report) = value.pointer("/params/report") {
                    self.report(
                        report,
                        value.pointer("/params/sessionId").and_then(Value::as_str),
                    );
                }
            }
            Some(_) => {}
            None => {
                let Some(id) = value.get("id").map(ToString::to_string) else {
                    return;
                };
                if let Some(session) = self.take_prompt(&id) {
                    self.finish(&session);
                } else if let Some(named) = self.take_open(&id) {
                    self.opened(&value, named);
                } else if let Some(result) = value.get("result")
                    && result.get("supported").is_some()
                {
                    self.report(result, None);
                }
            }
        }
    }

    fn take_prompt(&self, id: &str) -> Option<String> {
        let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        open.prompts.remove(id)
    }

    fn take_open(&self, id: &str) -> Option<Option<String>> {
        let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        open.opens.remove(id)
    }

    /// The answer to `session/new` or `session/load`: the model the session starts on.
    fn opened(&self, answer: &Value, named: Option<String>) {
        let Some(result) = answer.get("result") else {
            return;
        };
        let session = named.or_else(|| {
            result
                .get("sessionId")
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
        let model = result
            .pointer("/models/currentModelId")
            .and_then(Value::as_str);
        if let (Some(session), Some(model)) = (session, model) {
            let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
            remember(&mut open.models, &session, model);
        }
    }

    /// A usage report: its allowances are kept, and its spend is the turn's once the turn ends.
    fn report(&self, report: &Value, session: Option<&str>) {
        let Ok(report) = serde_json::from_value::<UsageReport>(report.clone()) else {
            return;
        };
        self.log.note_report(&self.account, &report, unix_now());
        let Some(spend) = report.spend else { return };
        let session = if spend.session_id.is_empty() {
            session.map(str::to_owned)
        } else {
            Some(spend.session_id.clone())
        };
        let Some(session) = session else { return };
        let turn_open = {
            let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
            let turn_open = open.prompts.values().any(|prompt| *prompt == session);
            if turn_open && (open.spent.len() < MAX_OPEN || open.spent.contains_key(&session)) {
                open.spent.insert(session.clone(), spend.clone());
            }
            turn_open
        };
        if !turn_open {
            self.record(&session, &spend);
        }
    }

    /// The turn of `session` ended: its spend is recorded.
    fn finish(&self, session: &str) {
        let spend = {
            let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
            open.spent.remove(session)
        };
        if let Some(spend) = spend {
            self.record(session, &spend);
        }
    }

    fn record(&self, session: &str, spend: &Spend) {
        let model = self
            .open
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .models
            .get(session)
            .cloned();
        self.log.record(
            &Turn {
                account: &self.account,
                harness: &self.harness,
                session,
                parent: None,
                model: model.as_deref(),
            },
            spend,
            unix_now(),
        );
    }
}

fn remember(models: &mut BTreeMap<String, String>, session: &str, model: &str) {
    if models.len() < MAX_OPEN || models.contains_key(session) {
        models.insert(session.to_owned(), model.to_owned());
    }
}
