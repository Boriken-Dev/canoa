//! A connection's channel on an account: the account named in the answer to `initialize`, and, for an account whose
//! credentials rotate, one session at a time.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a flag shared between threads is std's"
)]

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use coqui_web_channel::{Channel, Failure};
use serde_json::{Value, json};

use super::ACCOUNT_META;

/// ACP's invalid-params code, which a refused session answers.
const INVALID_PARAMS: i64 = -32602;
/// How long an answer to the client waits for room before the client is taken to have stopped reading.
const REPLY_LIMIT: Duration = Duration::from_secs(10);
const REPLY_POLL: Duration = Duration::from_millis(20);

/// The messages of one connection, with the account's part applied.
pub(crate) struct Gated<'a, C: Channel + ?Sized> {
    inner: &'a C,
    /// What to put under [`ACCOUNT_META`] in the answer to `initialize`: the account's name and kind.
    report: Option<Value>,
    /// Whether the account's credentials rotate, so that a second session is refused.
    one_session: bool,
    reported: AtomicBool,
    opened: AtomicUsize,
}

impl<'a, C: Channel + ?Sized> Gated<'a, C> {
    /// `inner`, naming `report` in its `initialize` answer; refusing a second session when `one_session`.
    pub(crate) fn new(inner: &'a C, report: Option<Value>, one_session: bool) -> Self {
        Self {
            inner,
            report,
            one_session,
            reported: AtomicBool::new(false),
            opened: AtomicUsize::new(0),
        }
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
}

/// The method of a request, and its id.
fn request_of(message: &[u8]) -> Option<(String, Value)> {
    let value: Value = serde_json::from_slice(message).ok()?;
    let method = value.get("method")?.as_str()?.to_owned();
    let id = value.get("id")?.clone();
    Some((method, id))
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
        let Some(report) = &self.report else {
            return self.inner.send(message);
        };
        if self.reported.load(Ordering::SeqCst) || !message.starts_with(b"{") {
            return self.inner.send(message);
        }
        let Ok(mut value) = serde_json::from_slice::<Value>(message) else {
            return self.inner.send(message);
        };
        if !is_initialized(&value) || self.reported.swap(true, Ordering::SeqCst) {
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
        let Some((method, id)) = request_of(&message) else {
            return Ok(Some(message));
        };
        match method.as_str() {
            "session/new" | "session/load" | "session/resume" => {
                if self.opened.fetch_add(1, Ordering::SeqCst) == 0 {
                    return Ok(Some(message));
                }
                self.opened.fetch_sub(1, Ordering::SeqCst);
                let answer = json!({"jsonrpc": "2.0", "id": id, "error": {
                    "code": INVALID_PARAMS,
                    "message": "this account rotates its credentials, so one session at a time may use it: close the first before opening another",
                }});
                self.reply(answer.to_string().as_bytes())?;
                tracing::info!("a second session was refused on an account that rotates");
                Err(Failure::Timeout)
            }
            "session/close" => {
                let _ = self
                    .opened
                    .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1));
                Ok(Some(message))
            }
            _ => Ok(Some(message)),
        }
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
