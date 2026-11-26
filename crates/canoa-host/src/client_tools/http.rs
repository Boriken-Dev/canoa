//! The providers' tools server: the bearers of a [`Hub`]'s sessions on the host's shared loopback server. A `POST` carries one
//! MCP message and is answered by the host or the provider; a `GET` is the session's stream of server messages, which is how an
//! agent hears that its tool list changed.

use std::sync::{Arc, Weak};

use serde_json::Value;

use super::{Access, Hub, Note, Tokens};
use crate::loopback::{self, Answer, Reply, Request, Route, Service};

pub(super) use crate::loopback::Listener;

/// What the server holds of the hub: not the hub itself, which holds the server.
struct Served {
    hub: Weak<Hub>,
    tokens: Arc<Tokens>,
}

impl Service for Served {
    type Caller = Access;
    const NAME: &'static str = canoa::brand::TOOLS_SERVER;
    /// One of them may be the stream of messages, and a call waits on the person for minutes while the agent's pings go on.
    const MAX_PER_CALLER: usize = 8;

    fn caller(&self, token: &str) -> Option<Arc<Access>> {
        self.tokens
            .find(&loopback::digest(token))
            .filter(|access| !access.is_closed())
    }

    /// A tool call's arguments: a selector, some text to fill in.
    fn body_limit(&self) -> usize {
        256 * 1024
    }

    fn route(&self, method: &str, path: &str) -> Route {
        if path != "/mcp" {
            Route::NotFound
        } else if matches!(method, "POST" | "GET") {
            Route::Serve
        } else {
            Route::Method("GET, POST")
        }
    }

    fn respond(self: &Arc<Self>, access: &Arc<Access>, request: &Request<'_>) -> Reply {
        if request.method == "GET" {
            let access = Arc::clone(access);
            return Reply::Stream {
                headers: vec!["Content-Type: text/event-stream".to_owned()],
                run: Box::new(move |out| {
                    loop {
                        match access.next_note() {
                            Note::Message(text) => write!(out, "event: message\ndata: {text}\n\n")?,
                            Note::Idle => write!(out, ": open\n\n")?,
                            Note::Closed => return Ok(()),
                        }
                        out.flush()?;
                    }
                }),
            };
        }
        let Ok(message) = serde_json::from_slice::<Value>(request.body) else {
            return Reply::Answer(Answer::Json(loopback::error(
                &Value::Null,
                -32700,
                "the body is not JSON",
            )));
        };
        // The host went away while the request was read.
        let Some(hub) = self.hub.upgrade() else {
            return Reply::Answer(Answer::Unavailable);
        };
        Reply::Answer(
            hub.answer(access, &message)
                .map_or(Answer::Accepted, Answer::Json),
        )
    }
}

/// Starts the server for `hub`'s sessions.
pub(super) fn start(hub: &Arc<Hub>) -> std::io::Result<Listener> {
    Listener::start(&Arc::new(Served {
        hub: Arc::downgrade(hub),
        tokens: Arc::clone(&hub.tokens),
    }))
}
