//! The secret tools' server: the bearers of [`Tokens`] on the host's shared loopback server.

use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

use serde_json::Value;

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
        let Some(tools) = tools_of(grant) else {
            return Answer::Unavailable;
        };
        tools
            .handle_message(message)
            .map_or(Answer::Accepted, Answer::Json)
    }
}
