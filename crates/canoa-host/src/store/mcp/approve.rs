//! What lets a store tool ask the person before it changes anything: the question goes to the client the session belongs to as
//! an ordinary `session/request_permission`, and the answer is read off that client's stream, never off the agent's.
//!
//! A change is approved only by the person's own answer to that question, or by the session's mode when the person put the
//! session in one that asks for nothing. What a catalog or an agent says never approves anything.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "locks, threads and the clock are std's"
)]

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::lock;
use crate::delegate::privilege::Modes;

/// The prefix of the id of every question the host puts to the client, which tells an answer to one from any other message.
pub(super) const ID_PREFIX: &str = "canoa-store-ask-";
/// How long the host waits for the person.
const WAIT: Duration = Duration::from_secs(300);
/// The option the person picks to allow the change; every other answer, an error and silence refuse it.
const ALLOW: &str = "allow-once";
/// What the question is told apart by on the client.
const META: &str = "dev.boriken.canoa/store";

/// How a change came to be allowed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Approval {
    /// The person said yes to it.
    Allowed,
    /// Nobody was asked: the person put the session in a mode that asks for nothing.
    Silent,
    /// The person said no.
    Declined,
}

/// What the person is asked: the change in plain words, and the digest it is bound to.
pub(crate) struct Card {
    pub(crate) title: String,
    pub(crate) text: String,
    pub(crate) digest: String,
}

/// A session that was given the tools.
pub(super) struct Held {
    pub(super) digest: [u8; 32],
    /// The modes the agent listed when the session opened: a mode outside them is never taken.
    pub(super) modes: Modes,
    /// Whether the person put the session in a mode that asks for nothing. Only the client's own `session/set_mode` or
    /// `session/set_config_option`, answered OK by the agent, raises it; what the agent says of itself can only lower it.
    pub(super) bypass: AtomicBool,
}

impl Held {
    pub(super) fn lists(&self, mode: &str) -> bool {
        self.modes.available.iter().any(|listed| listed == mode)
    }

    /// The agent says the session is in `mode`: a Bypass it was not put in by the person is not believed.
    pub(super) fn reported(&self, mode: &str) {
        if !crate::secrets::is_bypass(mode) {
            self.bypass.store(false, Ordering::SeqCst);
        }
    }
}

struct Waiting {
    answer: Mutex<Option<Value>>,
    ready: Condvar,
}

/// The questions of one client connection: what is to be sent, and who waits for what.
#[derive(Default)]
pub(super) struct Questions {
    outbox: Mutex<VecDeque<Vec<u8>>>,
    waiting: Mutex<HashMap<String, Arc<Waiting>>>,
    next: AtomicU64,
}

impl Questions {
    /// Adds a message of the host's own to the stream to the client.
    pub(super) fn push(&self, message: Vec<u8>) {
        lock(&self.outbox).push_back(message);
    }

    /// What the host added to the stream to the client.
    pub(super) fn drain(&self) -> Vec<Vec<u8>> {
        lock(&self.outbox).drain(..).collect()
    }

    /// Lets every waiter look again, as the connection has ended.
    pub(super) fn wake(&self) {
        for waiting in lock(&self.waiting).values() {
            waiting.ready.notify_all();
        }
    }

    /// An answer of the client's to a question this connection asked: whether it was one, which then goes no further. An id
    /// nobody waits for any more is still swallowed, so it never reaches the agent.
    pub(super) fn answered(&self, value: &Value) -> bool {
        if value.get("method").is_some() {
            return false;
        }
        let Some(id) = value
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| id.starts_with(ID_PREFIX))
        else {
            return false;
        };
        if let Some(waiting) = lock(&self.waiting).remove(id) {
            *lock(&waiting.answer) = Some(value.clone());
            waiting.ready.notify_all();
        }
        true
    }

    /// Puts `card` to the person as a permission request for `session` and waits. Whether they allowed it.
    pub(super) fn ask(
        &self,
        session: &str,
        card: &Card,
        closed: &dyn Fn() -> bool,
    ) -> Result<bool, String> {
        if closed() {
            return Err("the person's app is gone".to_owned());
        }
        let mut salt = [0_u8; 8];
        getrandom::fill(&mut salt).map_err(|_| "no randomness".to_owned())?;
        let number = self.next.fetch_add(1, Ordering::SeqCst);
        let id = format!(
            "{ID_PREFIX}{number}-{}",
            salt.iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        let waiting = Arc::new(Waiting {
            answer: Mutex::new(None),
            ready: Condvar::new(),
        });
        lock(&self.waiting).insert(id.clone(), Arc::clone(&waiting));
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "session/request_permission",
            "params": permission(session, number, card),
        });
        lock(&self.outbox).push_back(request.to_string().into_bytes());
        let until = Instant::now() + WAIT;
        let mut answer = lock(&waiting.answer);
        while answer.is_none() && !closed() {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            // A short slice, so an ended connection is noticed even if nobody wakes this thread.
            answer = waiting
                .ready
                .wait_timeout(answer, left.min(Duration::from_millis(250)))
                .map_or_else(|error| error.into_inner().0, |(guard, _)| guard);
        }
        let taken = answer.take();
        drop(answer);
        lock(&self.waiting).remove(&id);
        let Some(reply) = taken else {
            return Err("the person did not answer".to_owned());
        };
        if reply.get("error").is_some() {
            return Err("the person's app could not take the question".to_owned());
        }
        let outcome = &reply["result"]["outcome"];
        Ok(outcome["outcome"] == "selected" && outcome["optionId"] == ALLOW)
    }
}

/// The permission request: the change as plain text, with the choices Allow and Reject.
fn permission(session: &str, number: u64, card: &Card) -> Value {
    json!({
        "sessionId": session,
        "toolCall": {
            // No colon: a colon names a delegated task's call to the app.
            "toolCallId": format!("store-change-{number}"),
            "title": card.title,
            "kind": "other",
            "status": "pending",
            "content": [{"type": "content", "content": {"type": "text", "text": card.text}}],
        },
        "options": [
            {"optionId": ALLOW, "name": "Allow", "kind": "allow_once"},
            {"optionId": "reject", "name": "Reject", "kind": "reject_once"},
        ],
        "_meta": {META: {"digest": card.digest}},
    })
}
