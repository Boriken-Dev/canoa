//! The secret tools' server: the bearers of [`Tokens`] on the host's shared loopback server.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};

use super::link::tools_of;
use super::{Grant, Tokens};
use crate::loopback::{self, Answer, Caller, Service};

pub(super) use crate::loopback::Listener;

impl Caller for Grant {
    fn calls(&self) -> &AtomicUsize {
        &self.calls
    }
}

impl Service for Tokens {
    type Caller = Grant;
    const NAME: &'static str = "canoa-secrets";
    const MAX_PER_CALLER: usize = 4;

    fn caller(&self, token: &str) -> Option<Arc<Grant>> {
        self.find(&loopback::digest(token))
    }

    /// A form's schema and a command, which the tools limit further.
    fn body_limit(&self) -> usize {
        256 * 1024
    }

    fn answer(self: &Arc<Self>, grant: &Arc<Grant>, message: &Value) -> Answer {
        if grant.detached.load(Ordering::SeqCst) {
            return unattended(message);
        }
        let Some(tools) = tools_of(grant) else {
            return Answer::Unavailable;
        };
        tools
            .handle_message(message)
            .map_or(Answer::Accepted, Answer::Json)
    }
}

/// What an agent is told when it calls a tool after the connection of its person ended: for a request, the status that says no one
/// is there; any other tool says so as an error. Anything else of the protocol is refused as for any bearer that is gone.
fn unattended(message: &Value) -> Answer {
    if message.get("method").and_then(Value::as_str) != Some("tools/call") {
        return Answer::Unavailable;
    }
    let Some(id) = message.get("id") else {
        return Answer::Unavailable;
    };
    let said = "No one is at the screen to answer: the person's app has gone. Go on without it, or ask again when they are back.";
    let (text, is_error) = match message.pointer("/params/name").and_then(Value::as_str) {
        Some("secret_request") => (
            json!({"status": "no_one_present", "message": said}).to_string(),
            false,
        ),
        _ => (json!({"error": said}).to_string(), true),
    };
    Answer::Json(json!({
        "jsonrpc": "2.0", "id": id,
        "result": {"content": [{"type": "text", "text": text}], "isError": is_error},
    }))
}
