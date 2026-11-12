//! A connection's channel on an account: the account named in the answer to `initialize`, and, for an account whose
//! credentials rotate, one session at a time, counted from what the provider answers and not from what the client asks.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a state shared between threads is std's"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use coqui_transport::{Channel, Failure};
use serde_json::{Value, json};

use super::ACCOUNT_META;

/// ACP's invalid-params code, which a refused session answers.
const INVALID_PARAMS: i64 = -32602;
/// How long an answer to the client waits for room before the client is taken to have stopped reading.
const REPLY_LIMIT: Duration = Duration::from_secs(10);
const REPLY_POLL: Duration = Duration::from_millis(20);

/// A request that was forwarded and has not been answered, and what its answer means.
enum Pending {
    /// `session/new` or `session/fork`: the answer names the session.
    Opens,
    /// `session/load` or `session/resume`: the session is the one the request names.
    Loads(String),
    /// `session/close`: the session is the one the request names.
    Closes(String),
}

#[derive(Default)]
struct Sessions {
    pending: BTreeMap<String, Pending>,
    /// The sessions the provider has opened and the client has not closed.
    open: BTreeSet<String>,
}

impl Sessions {
    /// Sessions that are open or being opened.
    fn held(&self) -> usize {
        self.open.len()
            + self
                .pending
                .values()
                .filter(|pending| matches!(pending, Pending::Opens | Pending::Loads(_)))
                .count()
    }
}

/// The messages of one connection, with the account's part applied.
pub(crate) struct Gated<'a, C: Channel + ?Sized> {
    inner: &'a C,
    /// What to put under [`ACCOUNT_META`] in the answer to `initialize`: the account's name and kind.
    report: Option<Value>,
    /// Whether the account's credentials rotate, so that a second session is refused.
    one_session: bool,
    reported: AtomicBool,
    sessions: Mutex<Sessions>,
}

impl<'a, C: Channel + ?Sized> Gated<'a, C> {
    /// `inner`, naming `report` in its `initialize` answer; refusing a second session when `one_session`.
    pub(crate) fn new(inner: &'a C, report: Option<Value>, one_session: bool) -> Self {
        Self {
            inner,
            report,
            one_session,
            reported: AtomicBool::new(false),
            sessions: Mutex::new(Sessions::default()),
        }
    }

    fn sessions(&self) -> std::sync::MutexGuard<'_, Sessions> {
        self.sessions.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn reply(&self, message: &[u8]) -> Result<(), Failure> {
        let until = Instant::now() + REPLY_LIMIT;
        loop {
            match self.inner.send(message) {
                Err(Failure::NoProgress) if Instant::now() < until => {
                    std::thread::sleep(REPLY_POLL);
                }
                other => return other,
            }
        }
    }

    /// What the provider's answer to a request this gate forwarded does to the sessions held.
    fn answered(&self, value: &Value) {
        let Some(id) = value.get("id").map(Value::to_string) else {
            return;
        };
        let mut sessions = self.sessions();
        let Some(pending) = sessions.pending.remove(&id) else {
            return;
        };
        if value.get("error").is_some() || value.get("result").is_none() {
            return;
        }
        match pending {
            Pending::Opens => {
                if let Some(session) = value["result"]["sessionId"].as_str() {
                    sessions.open.insert(session.to_owned());
                }
            }
            Pending::Loads(session) => {
                sessions.open.insert(session);
            }
            Pending::Closes(session) => {
                sessions.open.remove(&session);
            }
        }
    }
}

/// A request's method, id and `sessionId`.
fn request_of(message: &[u8]) -> Option<(String, Value, Option<String>)> {
    let value: Value = serde_json::from_slice(message).ok()?;
    let method = value.get("method")?.as_str()?.to_owned();
    let id = value.get("id")?.clone();
    let session = value["params"]["sessionId"].as_str().map(str::to_owned);
    Some((method, id, session))
}

/// Whether `message` is the answer to `initialize`: a response whose result names a protocol version.
fn is_initialized(value: &Value) -> bool {
    value.get("method").is_none()
        && value
            .get("result")
            .is_some_and(|result| result.get("protocolVersion").is_some())
}

impl<C: Channel + ?Sized> Channel for Gated<'_, C> {
    fn send(&self, message: &[u8]) -> Result<(), Failure> {
        if !message.starts_with(b"{") {
            return self.inner.send(message);
        }
        let Ok(mut value) = serde_json::from_slice::<Value>(message) else {
            return self.inner.send(message);
        };
        if self.one_session && value.get("method").is_none() {
            self.answered(&value);
        }
        let Some(report) = &self.report else {
            return self.inner.send(message);
        };
        if self.reported.load(Ordering::SeqCst)
            || !is_initialized(&value)
            || self.reported.swap(true, Ordering::SeqCst)
        {
            return self.inner.send(message);
        }
        let Some(result) = value.get_mut("result").and_then(Value::as_object_mut) else {
            return self.inner.send(message);
        };
        let meta = result.entry("_meta").or_insert_with(|| json!({}));
        if !meta.is_object() {
            *meta = json!({});
        }
        meta[ACCOUNT_META] = report.clone();
        match serde_json::to_vec(&value) {
            Ok(bytes) => self.inner.send(&bytes),
            Err(_) => self.inner.send(message),
        }
    }

    /// A refused session reads as a timeout, after which the channel is still usable.
    fn receive(&self, timeout: Option<Duration>) -> Result<Option<Vec<u8>>, Failure> {
        let Some(message) = self.inner.receive(timeout)? else {
            return Ok(None);
        };
        if !self.one_session {
            return Ok(Some(message));
        }
        let Some((method, id, session)) = request_of(&message) else {
            return Ok(Some(message));
        };
        let pending = match method.as_str() {
            "session/new" | "session/fork" => Pending::Opens,
            "session/load" | "session/resume" => match session {
                Some(session) => Pending::Loads(session),
                None => return Ok(Some(message)),
            },
            "session/close" => match session {
                Some(session) => Pending::Closes(session),
                None => return Ok(Some(message)),
            },
            _ => return Ok(Some(message)),
        };
        let opening = !matches!(pending, Pending::Closes(_));
        {
            let mut sessions = self.sessions();
            let again =
                matches!(&pending, Pending::Loads(session) if sessions.open.contains(session));
            if opening && !again && sessions.held() >= 1 {
                drop(sessions);
                let answer = json!({"jsonrpc": "2.0", "id": id, "error": {
                    "code": INVALID_PARAMS,
                    "message": "this account rotates its credentials, so one session at a time may use it: close the first before opening another",
                }});
                self.reply(answer.to_string().as_bytes())?;
                tracing::info!("a second session was refused on an account that rotates");
                return Err(Failure::Timeout);
            }
            sessions.pending.insert(id.to_string(), pending);
        }
        Ok(Some(message))
    }

    fn close(&self) {
        self.inner.close();
    }

    fn set_max_message(&self, bytes: usize) {
        self.inner.set_max_message(bytes);
    }

    fn set_max_receive(&self, bytes: usize) {
        self.inner.set_max_receive(bytes);
    }
}

#[cfg(test)]
mod tests;
