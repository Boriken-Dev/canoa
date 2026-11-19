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
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use canoa_foundry::{
    ApiError, Cancel, Completion, Engine, EngineError, KEEP_ALIVE, ModelInfo, Translated,
    anthropic, openai, responses,
};
use serde_json::Value;

mod log;

use self::log::{Log, Track};
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
        // No one is waiting for it.
        EngineError::Cancelled => ApiError::upstream("the request was cancelled"),
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
    log: Log,
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

    fn run(&self, made: &Translated, cancel: &Cancel) -> Result<Completion, ApiError> {
        let answer = match self.engine.chat(&made.chat, cancel) {
            Ok(answer) => answer,
            // A tool call the answer's allowance cut off is a stop for length, which the harness knows how to meet; Foundry Local
            // reports it as a failed request.
            Err(EngineError::CutToolCall(why)) => {
                tracing::warn!("the model's tool call was cut off by the token limit: {why}");
                let model = self.engine.model().map(|m| m.id).unwrap_or_default();
                return Ok(Completion::cut_off(model));
            }
            Err(EngineError::Cancelled) => {
                tracing::info!("the client went away, and the model's work for it was stopped");
                return Err(engine_error(&EngineError::Cancelled));
            }
            Err(error) => {
                tracing::warn!("the local model did not answer: {error}");
                return Err(engine_error(&error));
            }
        };
        let mut done = Completion::from_chat(&answer)?;
        if let Ok(model) = self.engine.model() {
            done.model = model.id;
        }
        Ok(done)
    }

    /// An error reply, and the line that says the request ended in it.
    fn fail(&self, api: Api, track: Track, error: &ApiError) -> Reply {
        track.finish(&self.log, error.status, None, Some(&error.message));
        Self::error(api, error)
    }

    fn complete(self: &Arc<Self>, api: Api, request: &Request<'_>, mut track: Track) -> Reply {
        let body = request.body;
        // The client hanging up ends the model's work for it: a model that goes on answering no one holds every request behind it.
        let cancel = Cancel::new();
        let hung_up = cancel.clone();
        request.closed.on_close(move || hung_up.cancel());
        let asked = match serde_json::from_slice::<Value>(body) {
            Ok(asked) if asked.is_object() => asked,
            _ => {
                return self.fail(
                    api,
                    track,
                    &ApiError::invalid("the request body is a JSON object"),
                );
            }
        };
        track.asked(
            asked.get("model").and_then(Value::as_str).unwrap_or(""),
            asked.get("stream").and_then(Value::as_bool) == Some(true),
        );
        let made = match self.translate(api, &asked) {
            Ok(made) => made,
            Err(error) => return self.fail(api, track, &error),
        };
        track.begin(
            &self.log,
            body.len(),
            made.chat.get("max_tokens").and_then(Value::as_u64),
        );
        if !made.stream {
            return match self.run(&made, &cancel) {
                Ok(done) => {
                    track.answered(done.usage.input, done.usage.output);
                    track.finish(&self.log, 200, None, None);
                    Self::json(
                        &match api {
                            Api::Chat => openai::completion(&done),
                            Api::Responses => responses::response(&done),
                            Api::Messages => anthropic::message(&done, made.show_thinking),
                        },
                        &made.dropped,
                    )
                }
                Err(error) => self.fail(api, track, &error),
            };
        }
        let mut headers = vec!["Content-Type: text/event-stream".to_owned()];
        headers
            .extend(openai::dropped_note(&made.dropped).map(|note| format!("{DROPPED}: {note}")));
        let gateway = Arc::clone(self);
        let usage = openai::wants_usage(&asked);
        let input = anthropic::count_tokens(&asked);
        Reply::Stream {
            headers,
            run: Box::new(move |out| {
                gateway.stream(api, &made, (usage, input), &cancel, track, out)
            }),
        }
    }

    /// The head of a stream goes out before the model is asked, and a comment or a ping while it works, so that a client's idle
    /// timer does not run out on a model that takes minutes; then the answer is replayed as events.
    fn stream(
        &self,
        api: Api,
        made: &Translated,
        (usage, input): (bool, u64),
        cancel: &Cancel,
        mut track: Track,
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
        let first = track.elapsed();
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
                        let said = match api {
                            Api::Messages => anthropic::ping(),
                            Api::Responses => responses::ping(id, &model),
                            Api::Chat => KEEP_ALIVE.to_owned(),
                        };
                        let mut out = shared.lock().unwrap_or_else(PoisonError::into_inner);
                        if out
                            .write_all(said.as_bytes())
                            .and_then(|()| out.flush())
                            .is_err()
                        {
                            // The client is not there: nothing else will tell the model.
                            cancel.cancel();
                            return;
                        }
                    }
                }
            });
            let outcome = self.run(made, cancel);
            finished.store(true, Ordering::SeqCst);
            outcome
        });
        let failure = outcome.as_ref().err().map(|error| error.message.clone());
        if let Ok(done) = &outcome {
            track.answered(done.usage.input, done.usage.output);
        }
        let text = match (outcome, api) {
            (Ok(done), Api::Chat) => openai::stream_body(&done, usage),
            (Ok(done), Api::Responses) => responses::stream_body(&done),
            (Ok(done), Api::Messages) => anthropic::stream_body(&done, made.show_thinking),
            (Err(error), Api::Chat) => openai::stream_error(&error),
            (Err(error), Api::Responses) => responses::stream_error(&error),
            (Err(error), Api::Messages) => anthropic::stream_error(&error),
        };
        let out = shared.into_inner().unwrap_or_else(PoisonError::into_inner);
        let written = out.write_all(text.as_bytes()).and_then(|()| out.flush());
        // The status line went out before the model was asked, so it is 200 whatever the model did; the error says the rest.
        let error = match (&failure, &written) {
            (Some(message), _) => Some(message.clone()),
            (None, Err(error)) => Some(format!("the client went away: {error}")),
            (None, Ok(())) => None,
        };
        track.finish(&self.log, 200, Some(first), error.as_deref());
        written
    }

    fn models(&self, which: Option<&str>, track: Track) -> Reply {
        let models = self.engine.models();
        let body = match which {
            None => openai::model_list(&models),
            Some(id) => match models.iter().find(|model| model.id == id) {
                Some(model) => openai::model_entry(model),
                None => {
                    return self.fail(
                        Api::Chat,
                        track,
                        &ApiError::not_found(format!("no model called {id} is served here")),
                    );
                }
            },
        };
        track.finish(&self.log, 200, None, None);
        Self::json(&body, &[])
    }
}

impl Service for Gateway {
    type Caller = Session;
    const NAME: &'static str = "canoa-foundry";
    /// A turn of an agent asks for a title, a count and the answer at once; the model takes them one by one.
    const MAX_PER_CALLER: usize = 8;
    /// An answer takes minutes, and a client that gave up on it ends the model's work for it.
    const WATCHES_CLIENT: bool = true;

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
        let track = Track::new(request.path);
        match request.path {
            "/v1/chat/completions" => self.complete(Api::Chat, request, track),
            "/v1/responses" => self.complete(Api::Responses, request, track),
            "/v1/messages" => self.complete(Api::Messages, request, track),
            "/v1/messages/count_tokens" => match serde_json::from_slice::<Value>(request.body) {
                Ok(body) if body.is_object() => {
                    track.finish(&self.log, 200, None, None);
                    Self::json(
                        &serde_json::json!({"input_tokens": anthropic::count_tokens(&body)}),
                        &[],
                    )
                }
                _ => self.fail(
                    Api::Messages,
                    track,
                    &ApiError::invalid("the request body is a JSON object"),
                ),
            },
            "/v1/models" => self.models(None, track),
            other => self.models(other.strip_prefix("/v1/models/"), track),
        }
    }
}

/// A gateway listening on loopback.
pub(crate) struct Running {
    gateway: Arc<Gateway>,
    listener: Listener,
}

impl Running {
    /// Listens on a loopback port of the system's choosing, for `engine`, and records each request in `log` when it is given.
    ///
    /// # Errors
    ///
    /// The port could not be bound.
    pub(crate) fn start(
        engine: Arc<dyn Engine>,
        log: Option<PathBuf>,
    ) -> std::io::Result<Arc<Self>> {
        Self::start_with(engine, KEEP_ALIVE_EVERY, log)
    }

    fn start_with(
        engine: Arc<dyn Engine>,
        keep_alive: Duration,
        log: Option<PathBuf>,
    ) -> std::io::Result<Arc<Self>> {
        let gateway = Arc::new(Gateway {
            engine,
            sessions: Mutex::new(BTreeMap::new()),
            keep_alive,
            log: Log::new(log),
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
