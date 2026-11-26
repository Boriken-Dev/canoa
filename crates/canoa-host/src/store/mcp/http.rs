//! The store tools' server: the bearers of a [`Hub`]'s connections on the host's shared loopback server.

use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Weak};

use serde_json::Value;

use super::{Access, Hub, SERVER_NAME, Tokens, tools};
use crate::loopback::{self, Answer, Caller, Service};

pub(super) use crate::loopback::Listener;

impl Caller for Access {
    fn calls(&self) -> &AtomicUsize {
        &self.calls
    }
}

/// What the server holds of the hub: not the hub itself, which holds the server.
struct Served {
    hub: Weak<Hub>,
    tokens: Arc<Tokens>,
}

impl Service for Served {
    type Caller = Access;
    const NAME: &'static str = canoa::brand::STORE_SERVER;
    const MAX_PER_CALLER: usize = 4;

    fn caller(&self, token: &str) -> Option<Arc<Access>> {
        self.tokens.find(&loopback::digest(token))
    }

    /// A tool call's few short arguments.
    fn body_limit(&self) -> usize {
        16 * 1024
    }

    fn answer(self: &Arc<Self>, access: &Arc<Access>, message: &Value) -> Answer {
        // The host went away while the request was read.
        let Some(hub) = self.hub.upgrade() else {
            return Answer::Unavailable;
        };
        if let Some(extra) = &access.extra {
            return loopback::rpc(
                extra.server(),
                message,
                || extra.list(),
                |name, arguments| Ok(extra.call(access, name, arguments)),
            )
            .map_or(Answer::Accepted, Answer::Json);
        }
        loopback::rpc(SERVER_NAME, message, tools::list, |name, arguments| {
            let backend = hub.backend().ok_or("the store is not ready")?;
            Ok(tools::call(&*backend, access, name, arguments))
        })
        .map_or(Answer::Accepted, Answer::Json)
    }
}

/// Starts the server for `hub`'s connections.
pub(super) fn start(hub: &Arc<Hub>) -> std::io::Result<Listener> {
    Listener::start(&Arc::new(Served {
        hub: Arc::downgrade(hub),
        tokens: Arc::clone(&hub.tokens),
    }))
}
