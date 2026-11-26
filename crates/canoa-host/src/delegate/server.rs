//! The delegate tools' server: an [`Engine`]'s sessions on the host's shared loopback server.

use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

use serde_json::Value;

use super::{Engine, SERVER_NAME, Session, tools};
use crate::loopback::{self, Answer, Caller, MAX_HEAD, Service};

pub(crate) use crate::loopback::Listener;

impl Caller for Session {
    fn calls(&self) -> &AtomicUsize {
        &self.calls
    }
}

impl Service for Engine {
    type Caller = Session;
    const NAME: &'static str = canoa::brand::DELEGATE_SERVER;
    const MAX_PER_CALLER: usize = 8;

    fn caller(&self, token: &str) -> Option<Arc<Session>> {
        self.session_of_token(token)
    }

    fn body_limit(&self) -> usize {
        self.config
            .task_bytes
            .saturating_mul(6)
            .saturating_add(MAX_HEAD)
    }

    fn answer(self: &Arc<Self>, session: &Arc<Session>, message: &Value) -> Answer {
        let answer = loopback::rpc(
            SERVER_NAME,
            message,
            || tools::list(self, session),
            |name, arguments| Ok(tools::call(self, session, name, arguments)),
        );
        answer.map_or(Answer::Accepted, Answer::Json)
    }
}
