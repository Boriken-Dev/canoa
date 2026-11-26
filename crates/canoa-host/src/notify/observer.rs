//! Watches the ACP messages of one connection and fires the events of its sessions: an approval or a question put to the client, a
//! turn that finished, a turn that failed or whose agent ended. It stands on the channel to the client, so it sees what reaches the
//! client and nothing a hook answered, and it works with or without any overlay hooks.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "the connection's state is std's"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use canoa::notifier::SessionRef;
use coqui_transport::{Channel, Failure};
use serde_json::Value;

use super::hub::{Hub, Occurrence};

#[derive(Default)]
struct Book {
    /// Prompts waiting for their answer, by the request's id as text: the agent's session.
    turns: BTreeMap<String, String>,
    /// The sessions the client asked to cancel.
    cancelled: BTreeSet<String>,
    /// What the agent says each session is called.
    titles: BTreeMap<String, String>,
}

/// A connection's channel to its client, watched.
pub(crate) struct Observed<'a, C: ?Sized> {
    inner: &'a C,
    hub: Arc<Hub>,
    /// The agent's name for people.
    agent: String,
    book: Mutex<Book>,
}

impl<C: ?Sized> core::fmt::Debug for Observed<'_, C> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Observed").finish_non_exhaustive()
    }
}

impl<'a, C: ?Sized> Observed<'a, C> {
    /// Watches `inner`, the channel of a connection to the agent `agent`.
    pub(crate) fn new(inner: &'a C, hub: Arc<Hub>, agent: impl Into<String>) -> Self {
        Self {
            inner,
            hub,
            agent: agent.into(),
            book: Mutex::new(Book::default()),
        }
    }

    fn book(&self) -> std::sync::MutexGuard<'_, Book> {
        self.book.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn session(&self, id: &str) -> SessionRef {
        let mut reference = SessionRef::new(id);
        reference.title = self.book().titles.get(id).cloned();
        reference.agent = Some(self.agent.clone());
        reference
    }

    fn fire(&self, event: &str, session: &str, request: Option<String>) {
        // The hub's thread is the caller's to drop: nothing here waits on a notifier.
        drop(self.hub.fire(
            event,
            Occurrence {
                session: Some(self.session(session)),
                request,
                figure: None,
            },
        ));
    }

    /// The agent's process ended or stopped reading: every turn that was running failed.
    pub(crate) fn agent_ended(&self) {
        let turns = core::mem::take(&mut self.book().turns);
        for (request, session) in turns {
            self.fire("failed", &session, Some(request));
        }
    }

    /// A message the agent sent the client.
    fn see_agent(&self, message: &[u8]) {
        let Ok(value) = serde_json::from_slice::<Value>(message) else {
            return;
        };
        let id = value
            .get("id")
            .filter(|id| !id.is_null())
            .map(Value::to_string);
        match (value.get("method").and_then(Value::as_str), id) {
            (Some("session/request_permission"), Some(id)) => {
                if let Some(session) = session_of(&value) {
                    self.fire("permission", &session, Some(id));
                }
            }
            (Some("elicitation/create"), Some(id)) => {
                if let Some(session) = session_of(&value) {
                    self.fire("question", &session, Some(id));
                }
            }
            (Some("session/update"), None) => {
                let update = value.pointer("/params/update");
                if update
                    .and_then(|update| update.get("sessionUpdate"))
                    .and_then(Value::as_str)
                    == Some("session_info_update")
                    && let (Some(session), Some(title)) = (
                        session_of(&value),
                        update
                            .and_then(|update| update.get("title"))
                            .and_then(Value::as_str),
                    )
                {
                    self.book().titles.insert(session, title.to_owned());
                }
            }
            (None, Some(id)) => {
                let session = self.book().turns.remove(&id);
                let Some(session) = session else { return };
                let cancelled = self.book().cancelled.remove(&session);
                if value.get("error").is_some() {
                    self.fire("failed", &session, Some(id));
                } else if value.pointer("/result/stopReason").and_then(Value::as_str)
                    != Some("cancelled")
                    && !cancelled
                {
                    self.fire("finished", &session, Some(id));
                }
            }
            _ => {}
        }
    }

    /// A message the client sent the agent.
    fn see_client(&self, message: &[u8]) {
        let Ok(value) = serde_json::from_slice::<Value>(message) else {
            return;
        };
        match value.get("method").and_then(Value::as_str) {
            Some("session/prompt") => {
                if let (Some(id), Some(session)) = (
                    value
                        .get("id")
                        .filter(|id| !id.is_null())
                        .map(Value::to_string),
                    session_of(&value),
                ) {
                    self.book().turns.insert(id, session);
                }
            }
            Some("session/cancel") => {
                if let Some(session) = session_of(&value) {
                    self.book().cancelled.insert(session);
                }
            }
            _ => {}
        }
    }
}

fn session_of(value: &Value) -> Option<String> {
    value
        .pointer("/params/sessionId")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

impl<C: Channel + ?Sized> Channel for Observed<'_, C> {
    fn send(&self, message: &[u8]) -> Result<(), Failure> {
        let sent = self.inner.send(message);
        if sent.is_ok() {
            self.see_agent(message);
        }
        sent
    }

    fn receive(&self, timeout: Option<Duration>) -> Result<Option<Vec<u8>>, Failure> {
        let received = self.inner.receive(timeout)?;
        if let Some(message) = &received {
            self.see_client(message);
        }
        Ok(received)
    }

    fn close(&self) {
        self.inner.close();
    }

    fn set_max_message(&self, bytes: usize) {
        self.inner.set_max_message(bytes);
    }
}
