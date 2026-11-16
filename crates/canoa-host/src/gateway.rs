//! The Foundry Local gateway: a local model behind the OpenAI and Anthropic APIs, on the host's shared loopback server.
//!
//! An agent (Claude Code, Codex, Pi) is pointed at it as it would be at a cloud endpoint, with a bearer minted for its session.
//! Each request is translated to one chat completion ([`canoa_foundry`]), run by the [`Engine`], and the answer is written back
//! whole or replayed as the server-sent events the client asked for. The model is called once, not streamed: the service's
//! own streaming does not deliver tokens in the versions measured.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "sockets, locks and threads are std's"
)]

use std::collections::BTreeMap;
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use canoa_foundry::{
    ApiError, Completion, Engine, EngineError, KEEP_ALIVE, ModelInfo, Translated, anthropic,
    openai, responses,
};
use serde_json::Value;

use crate::loopback::{self, Caller, Listener, Reply, Request, Route, Service};

/// The header that says what a request carried that the model could not be given.
const DROPPED: &str = "x-canoa-dropped";

/// How long a stream waits on the model before it says it is still there.
const KEEP_ALIVE_EVERY: Duration = Duration::from_secs(10);

/// One session's bearer, and the calls it has under way.
pub(crate) struct Session {
    calls: AtomicUsize,
}

impl Caller for Session {
    fn calls(&self) -> &AtomicUsize {
        &self.calls
    }
}

/// What a client is told of why the model did not answer: what the person can fix, or what changing nothing would repeat, is
/// a refusal that is not retried; the rest is a failure that may pass.
fn engine_error(error: &EngineError) -> ApiError {
    match error {
        EngineError::NotLoaded(_) | EngineError::Refused(_) => ApiError::invalid(error.to_string()),
        EngineError::Failed(_) => ApiError::upstream(error.to_string()),
        _ => ApiError::upstream(error.to_string()),
    }
}

/// The three APIs and what each is for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Api {
    Chat,
    Responses,
    Messages,
}

/// What answers the gateway's requests.
pub(crate) struct Gateway {
    engine: Arc<dyn Engine>,
    sessions: Mutex<BTreeMap<[u8; 32], Arc<Session>>>,
    keep_alive: Duration,
}

impl Gateway {
    fn sessions(&self) -> std::sync::MutexGuard<'_, BTreeMap<[u8; 32], Arc<Session>>> {
        self.sessions.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn error(api: Api, error: &ApiError) -> Reply {
        let body = if api == Api::Messages {
            error.anthropic()
        } else {
            error.openai()
        };
        let reason = match error.status {
            400 => "Bad Request",
            404 => "Not Found",
            _ => "Bad Gateway",
        };
        Reply::Whole {
            code: error.status,
            reason,
            headers: vec!["Content-Type: application/json".to_owned()],
            body: body.to_string(),
        }
    }

    fn json(body: &Value, dropped: &[String]) -> Reply {
        let mut headers = vec!["Content-Type: application/json".to_owned()];
        headers.extend(openai::dropped_note(dropped).map(|note| format!("{DROPPED}: {note}")));
        Reply::Whole {
            code: 200,
            reason: "OK",
            headers,
            body: body.to_string(),
        }
    }

    /// The model, or why it cannot answer: not loaded is the person's to fix, so it is a refusal and not a failure to retry.
    fn model(&self) -> Result<ModelInfo, ApiError> {
        self.engine.model().map_err(|error| engine_error(&error))
    }

    fn translate(&self, api: Api, request: &Value) -> Result<Translated, ApiError> {
        let model = self.model()?;
        match api {
            Api::Chat => openai::to_chat(request, &model),
            Api::Responses => responses::to_chat(request, &model),
            Api::Messages => anthropic::to_chat(request, &model),
        }
    }

    fn run(&self, made: &Translated) -> Result<Completion, ApiError> {
        let answer = self.engine.chat(&made.chat).map_err(|error| {
            tracing::warn!("the local model did not answer: {error}");
            engine_error(&error)
        })?;
        let mut done = Completion::from_chat(&answer)?;
        if let Ok(model) = self.engine.model() {
            done.model = model.id;
        }
        Ok(done)
    }

    fn complete(self: &Arc<Self>, api: Api, body: &[u8]) -> Reply {
        let request = match serde_json::from_slice::<Value>(body) {
            Ok(request) if request.is_object() => request,
            _ => {
                return Self::error(api, &ApiError::invalid("the request body is a JSON object"));
            }
        };
        let made = match self.translate(api, &request) {
            Ok(made) => made,
            Err(error) => return Self::error(api, &error),
        };
        if !made.stream {
            return match self.run(&made) {
                Ok(done) => Self::json(
                    &match api {
                        Api::Chat => openai::completion(&done),
                        Api::Responses => responses::response(&done),
                        Api::Messages => anthropic::message(&done, made.show_thinking),
                    },
                    &made.dropped,
                ),
                Err(error) => Self::error(api, &error),
            };
        }
        let mut headers = vec!["Content-Type: text/event-stream".to_owned()];
        headers
            .extend(openai::dropped_note(&made.dropped).map(|note| format!("{DROPPED}: {note}")));
        let gateway = Arc::clone(self);
        let usage = openai::wants_usage(&request);
        let input = anthropic::count_tokens(&request);
        Reply::Stream {
            headers,
            run: Box::new(move |out| gateway.stream(api, &made, (usage, input), out)),
        }
    }

    /// The head of a stream goes out before the model is asked, and a comment or a ping while it works, so that a client's idle
    /// timer does not run out on a model that takes minutes; then the answer is replayed as events.
    fn stream(
        &self,
        api: Api,
        made: &Translated,
        (usage, input): (bool, u64),
        out: &mut (dyn Write + Send),
    ) -> std::io::Result<()> {
        let model = self.engine.model().map(|m| m.id).unwrap_or_default();
        let id = "local";
        out.write_all(
            match api {
                Api::Chat => openai::stream_head(id, &model),
                Api::Responses => responses::stream_head(id, &model),
                Api::Messages => anthropic::stream_head(id, &model, input),
            }
            .as_bytes(),
        )?;
        out.flush()?;
        let shared = Mutex::new(out);
        let finished = AtomicBool::new(false);
        let tick = (self.keep_alive / 4).max(Duration::from_millis(5));
        let outcome = std::thread::scope(|scope| {
            scope.spawn(|| {
                let mut waited = Duration::ZERO;
                while !finished.load(Ordering::SeqCst) {
                    std::thread::sleep(tick);
                    waited += tick;
                    if waited >= self.keep_alive {
                        waited = Duration::ZERO;
                        let said = if api == Api::Messages {
                            anthropic::ping()
                        } else {
                            KEEP_ALIVE.to_owned()
                        };
                        let mut out = shared.lock().unwrap_or_else(PoisonError::into_inner);
                        if out
                            .write_all(said.as_bytes())
                            .and_then(|()| out.flush())
                            .is_err()
                        {
                            return;
                        }
                    }
                }
            });
            let outcome = self.run(made);
            finished.store(true, Ordering::SeqCst);
            outcome
        });
        let text = match (outcome, api) {
            (Ok(done), Api::Chat) => openai::stream_body(&done, usage),
            (Ok(done), Api::Responses) => responses::stream_body(&done),
            (Ok(done), Api::Messages) => anthropic::stream_body(&done, made.show_thinking),
            (Err(error), Api::Chat) => openai::stream_error(&error),
            (Err(error), Api::Responses) => responses::stream_error(&error),
            (Err(error), Api::Messages) => anthropic::stream_error(&error),
        };
        let out = shared.into_inner().unwrap_or_else(PoisonError::into_inner);
        out.write_all(text.as_bytes())?;
        out.flush()
    }

    fn models(&self, which: Option<&str>) -> Reply {
        let models = self.engine.models();
        let body = match which {
            None => openai::model_list(&models),
            Some(id) => match models.iter().find(|model| model.id == id) {
                Some(model) => openai::model_entry(model),
                None => {
                    return Self::error(
                        Api::Chat,
                        &ApiError::not_found(format!("no model called {id} is served here")),
                    );
                }
            },
        };
        Self::json(&body, &[])
    }
}

impl Service for Gateway {
    type Caller = Session;
    const NAME: &'static str = "canoa-foundry";
    /// A turn of an agent asks for a title, a count and the answer at once; the model takes them one by one.
    const MAX_PER_CALLER: usize = 8;

    fn caller(&self, token: &str) -> Option<Arc<Session>> {
        self.sessions().get(&loopback::digest(token)).cloned()
    }

    /// A conversation with its tools and, for a model that reads them, its images.
    fn body_limit(&self) -> usize {
        64 * 1024 * 1024
    }

    fn route(&self, method: &str, path: &str) -> Route {
        let (allowed, found) = match path {
            "/v1/chat/completions"
            | "/v1/responses"
            | "/v1/messages"
            | "/v1/messages/count_tokens" => ("POST", true),
            "/v1/models" => ("GET", true),
            _ if path.starts_with("/v1/models/") => ("GET", true),
            _ => ("", false),
        };
        if !found {
            Route::NotFound
        } else if method == allowed {
            Route::Serve
        } else {
            Route::Method(allowed)
        }
    }

    fn respond(self: &Arc<Self>, _caller: &Arc<Session>, request: &Request<'_>) -> Reply {
        match request.path {
            "/v1/chat/completions" => self.complete(Api::Chat, request.body),
            "/v1/responses" => self.complete(Api::Responses, request.body),
            "/v1/messages" => self.complete(Api::Messages, request.body),
            "/v1/messages/count_tokens" => match serde_json::from_slice::<Value>(request.body) {
                Ok(body) if body.is_object() => Self::json(
                    &serde_json::json!({"input_tokens": anthropic::count_tokens(&body)}),
                    &[],
                ),
                _ => Self::error(
                    Api::Messages,
                    &ApiError::invalid("the request body is a JSON object"),
                ),
            },
            "/v1/models" => self.models(None),
            other => self.models(other.strip_prefix("/v1/models/")),
        }
    }
}

/// A gateway listening on loopback.
pub(crate) struct Running {
    gateway: Arc<Gateway>,
    listener: Listener,
}

impl Running {
    /// Listens on a loopback port of the system's choosing, for `engine`.
    ///
    /// # Errors
    ///
    /// The port could not be bound.
    pub(crate) fn start(engine: Arc<dyn Engine>) -> std::io::Result<Arc<Self>> {
        Self::start_with(engine, KEEP_ALIVE_EVERY)
    }

    fn start_with(engine: Arc<dyn Engine>, keep_alive: Duration) -> std::io::Result<Arc<Self>> {
        let gateway = Arc::new(Gateway {
            engine,
            sessions: Mutex::new(BTreeMap::new()),
            keep_alive,
        });
        let listener = Listener::start(&gateway)?;
        Ok(Arc::new(Self { gateway, listener }))
    }

    /// The address agents are pointed at, without a path.
    pub(crate) fn address(&self) -> String {
        format!("http://127.0.0.1:{}", self.listener.addr().port())
    }

    /// A bearer for one session, good until the ticket is dropped.
    ///
    /// # Errors
    ///
    /// The system has no randomness.
    pub(crate) fn mint(self: &Arc<Self>) -> Result<Ticket, getrandom::Error> {
        let mut random = [0_u8; 32];
        getrandom::fill(&mut random)?;
        let bearer: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let key = loopback::digest(&bearer);
        self.gateway.sessions().insert(
            key,
            Arc::new(Session {
                calls: AtomicUsize::new(0),
            }),
        );
        Ok(Ticket {
            running: Arc::clone(self),
            key,
            bearer,
        })
    }
}

/// A session's bearer; dropping it ends the session's access, and the last one ends the gateway.
pub(crate) struct Ticket {
    running: Arc<Running>,
    key: [u8; 32],
    bearer: String,
}

impl Ticket {
    /// The bearer an agent shows.
    pub(crate) fn bearer(&self) -> &str {
        &self.bearer
    }

    /// The address its agent is pointed at.
    pub(crate) fn address(&self) -> String {
        self.running.address()
    }
}

impl core::fmt::Debug for Ticket {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // The bearer is not shown.
        f.debug_struct("Ticket").finish_non_exhaustive()
    }
}

impl Drop for Ticket {
    fn drop(&mut self) {
        self.running.gateway.sessions().remove(&self.key);
    }
}

#[cfg(test)]
mod tests;
