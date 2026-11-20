//! A client's channel, with the longest message the host takes named in the answer to `initialize`, so a client that frames
//! its own messages can open its limit to the host's.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a state shared between threads is std's"
)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use coqui_transport::{Channel, Failure};
use serde_json::{Value, json};

/// The `_meta` key of the answer to `initialize` that holds `[host] max_message_bytes`, a number.
pub const MAX_MESSAGE_META: &str = "dev.boriken.canoa/maxMessageBytes";

/// `inner`, naming `limit` in the first answer to `initialize` that passes through it.
pub(super) struct Naming<'a, C: Channel + ?Sized> {
    inner: &'a C,
    limit: usize,
    named: AtomicBool,
}

impl<'a, C: Channel + ?Sized> Naming<'a, C> {
    pub(super) fn new(inner: &'a C, limit: usize) -> Self {
        Self {
            inner,
            limit,
            named: AtomicBool::new(false),
        }
    }
}

/// Whether `value` is the answer to `initialize`: a response whose result names a protocol version.
fn is_initialized(value: &Value) -> bool {
    value.get("method").is_none()
        && value
            .get("result")
            .is_some_and(|result| result.get("protocolVersion").is_some())
}

impl<C: Channel + ?Sized> Channel for Naming<'_, C> {
    fn send(&self, message: &[u8]) -> Result<(), Failure> {
        const MARK: &[u8] = b"protocolVersion";
        if self.named.load(Ordering::SeqCst)
            || !message.starts_with(b"{")
            || !message.windows(MARK.len()).any(|window| window == MARK)
        {
            return self.inner.send(message);
        }
        let Ok(mut value) = serde_json::from_slice::<Value>(message) else {
            return self.inner.send(message);
        };
        if !is_initialized(&value) || self.named.swap(true, Ordering::SeqCst) {
            return self.inner.send(message);
        }
        let Some(result) = value.get_mut("result").and_then(Value::as_object_mut) else {
            return self.inner.send(message);
        };
        let meta = result.entry("_meta").or_insert_with(|| json!({}));
        if !meta.is_object() {
            *meta = json!({});
        }
        meta[MAX_MESSAGE_META] = json!(self.limit);
        match serde_json::to_vec(&value) {
            Ok(bytes) => self.inner.send(&bytes),
            Err(_) => self.inner.send(message),
        }
    }

    fn receive(&self, timeout: Option<Duration>) -> Result<Option<Vec<u8>>, Failure> {
        self.inner.receive(timeout)
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
