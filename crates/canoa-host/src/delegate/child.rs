//! A task's work: a normal host session of the task's source, opened with [`crate::Local`], driven by the host as an ACP client.
//! It is given the folder as its working directory, no files or terminal of the host's, and one prompt: the task's text. Its
//! reply becomes the task's `result` artifact. Everything it asks of the person goes to the delegating session's connection.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "locks, threads and the clock are std's"
)]

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use coqui_a2a::error::A2aError;
use coqui_a2a::model::{Artifact, Message, Part, Role, TaskState};
use serde_json::{Value, json};

use super::privilege::{self, Modes};
use super::{ChildContext, Engine, Reservation, Session, lock};
use crate::local::{Link, LinkError};

/// How long a task's agent has to end its turn after it was told to cancel, before its process is ended.
pub(crate) const GRACE: Duration = Duration::from_secs(3);
/// How long a read waits before it looks up to see whether the task was cancelled.
const POLL: Duration = Duration::from_millis(100);

/// What a task's work came to.
pub(crate) enum Outcome {
    /// The agent ended its turn, with this reply (cut at the history limit when `cut`).
    Reply {
        text: String,
        stop: String,
        cut: bool,
    },
    /// The task was cancelled; the store has said so.
    Cancelled,
    /// The work failed.
    Failed(String),
    /// The task was refused after its session opened: no mode could be given it.
    Rejected(String),
}

/// The updates of a task, newest last, held to a size.
#[derive(Default)]
struct History {
    bytes: usize,
    items: VecDeque<String>,
}

impl History {
    fn push(&mut self, item: String, limit: usize) {
        self.bytes += item.len();
        self.items.push_back(item);
        while self.bytes > limit {
            let Some(old) = self.items.pop_front() else {
                break;
            };
            self.bytes -= old.len();
        }
    }
}

/// One running task, as the delegating session and its cancel see it.
pub(crate) struct Child {
    pub(crate) task: String,
    pub(crate) title: String,
    pub(crate) source: String,
    cancelling: AtomicBool,
    link: Mutex<Option<Arc<Link>>>,
    session: Mutex<Option<String>>,
    done: Mutex<bool>,
    ended: Condvar,
    history: Mutex<History>,
}

impl Child {
    pub(crate) fn new(task: String, title: String, source: String) -> Arc<Self> {
        Arc::new(Self {
            task,
            title,
            source,
            cancelling: AtomicBool::new(false),
            link: Mutex::new(None),
            session: Mutex::new(None),
            done: Mutex::new(false),
            ended: Condvar::new(),
            history: Mutex::new(History::default()),
        })
    }

    fn cancelled(&self) -> bool {
        self.cancelling.load(Ordering::SeqCst)
    }

    /// The updates the task's agent has sent, oldest first, as far as `history_bytes` holds them.
    #[allow(
        dead_code,
        reason = "read by whatever shows a task's thread to its client"
    )]
    pub(crate) fn history(&self) -> Vec<String> {
        lock(&self.history).items.iter().cloned().collect()
    }

    /// Keeps one update, dropping the oldest while the updates pass `limit` bytes.
    pub(super) fn keep(&self, update: String, limit: usize) {
        lock(&self.history).push(update, limit);
    }

    fn finish(&self) {
        *lock(&self.done) = true;
        self.ended.notify_all();
    }

    fn wait_done(&self, limit: Duration) -> bool {
        let until = Instant::now() + limit;
        let mut done = lock(&self.done);
        while !*done {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            done = self
                .ended
                .wait_timeout(done, left)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
        true
    }

    /// Tells the task's agent to cancel its turn and gives it [`GRACE`] to do so, then ends its process.
    pub(crate) fn cancel(&self) {
        self.cancelling.store(true, Ordering::SeqCst);
        let link = lock(&self.link).clone();
        let session = lock(&self.session).clone();
        if let (Some(link), Some(session)) = (link, session) {
            let _ = send(
                &link,
                &json!({"jsonrpc": "2.0", "method": "session/cancel", "params": {"sessionId": session}}),
            );
        }
        if !self.wait_done(GRACE + GRACE) {
            self.stop_now();
            self.wait_done(GRACE);
        }
    }

    /// Ends the task's agent's process at once.
    pub(crate) fn stop_now(&self) {
        self.cancelling.store(true, Ordering::SeqCst);
        if let Some(link) = lock(&self.link).as_ref() {
            link.close();
        }
    }
}

/// Cancels the task `task` of `session`: the cancel hook of its store. A task that already ended has nothing to cancel.
pub(crate) fn cancel(session: &Session, task: &str) -> Result<(), A2aError> {
    let child = lock(&session.children).get(task).cloned();
    if let Some(child) = child {
        child.cancel();
    }
    Ok(())
}

fn send(link: &Link, message: &Value) -> bool {
    link.send(message.to_string().as_bytes()).is_ok()
}

/// Everything a task's thread needs.
pub(crate) struct Job {
    pub(crate) engine: Arc<Engine>,
    pub(crate) session: Arc<Session>,
    pub(crate) child: Arc<Child>,
    pub(crate) folder: PathBuf,
    pub(crate) text: String,
    pub(crate) reservation: Reservation,
}

/// Starts the task's thread.
///
/// # Errors
///
/// The reason the thread could not start.
pub(crate) fn start(job: Job) -> Result<(), std::io::Error> {
    let name = format!("canoa-delegate-{}", job.child.task);
    lock(&job.session.children).insert(job.child.task.clone(), Arc::clone(&job.child));
    let session = Arc::clone(&job.session);
    let task = job.child.task.clone();
    let spawned = std::thread::Builder::new()
        .name(name)
        .spawn(move || run(job));
    if spawned.is_err() {
        lock(&session.children).remove(&task);
    }
    spawned.map(drop)
}

fn run(job: Job) {
    let Job {
        engine,
        session,
        child,
        folder,
        text,
        reservation,
    } = job;
    let outcome = drive(&engine, &session, &child, &folder, &text);
    // The agent ends before the task is called done, so a task that reads as ended has no process.
    if let Some(link) = lock(&child.link).as_ref() {
        link.close();
    }
    settle(&session, &child, outcome);
    lock(&session.children).remove(&child.task);
    drop(reservation);
    child.finish();
}

fn note(text: &str) -> Message {
    Message::text(Role::Agent, format!("note-{}", text.len()), text)
}

fn settle(session: &Session, child: &Child, outcome: Outcome) {
    let store = &session.store;
    let task = &child.task;
    let ended = match outcome {
        Outcome::Cancelled => return,
        Outcome::Reply { text, stop, cut } => {
            if !text.is_empty() {
                let artifact = Artifact {
                    artifact_id: "result".to_owned(),
                    name: Some("result".to_owned()),
                    parts: vec![Part::text(text)],
                    ..Artifact::default()
                };
                let _ = store.add_artifact(task, artifact, false, true);
            }
            let mut said = Vec::new();
            if stop != "end_turn" {
                said.push(format!("the agent stopped: {stop}"));
            }
            if cut {
                said.push("the reply was cut at history_bytes".to_owned());
            }
            let message = (!said.is_empty()).then(|| note(&said.join("; ")));
            store.update_status(task, TaskState::Completed, message)
        }
        Outcome::Failed(why) => store.update_status(task, TaskState::Failed, Some(note(&why))),
        Outcome::Rejected(why) => store.update_status(task, TaskState::Rejected, Some(note(&why))),
    };
    // A task cancelled meanwhile is already over, and says so.
    drop(ended);
}

fn drive(
    engine: &Arc<Engine>,
    session: &Arc<Session>,
    child: &Arc<Child>,
    folder: &Path,
    text: &str,
) -> Outcome {
    // A cancel that came before the thread did has already ended the task.
    if session
        .store
        .snapshot(&child.task)
        .is_none_or(|task| task.status.state.is_terminal())
    {
        return Outcome::Cancelled;
    }
    let _ = session
        .store
        .update_status(&child.task, TaskState::Working, None);
    let (Some(conn), Some(host)) = (session.conn(), engine.host()) else {
        return Outcome::Failed("the delegating connection has ended".to_owned());
    };
    let context = ChildContext {
        principal: conn.principal().clone(),
        roots: conn.roots().to_vec(),
        depth: conn.depth() + 1,
    };
    let link = match host.open(&child.source, context) {
        Ok(link) => Arc::new(link),
        Err(error) => {
            return Outcome::Failed(format!("{} could not be opened: {error}", child.source));
        }
    };
    *lock(&child.link) = Some(Arc::clone(&link));
    if child.cancelled() {
        return Outcome::Cancelled;
    }
    let mut work = Work {
        engine,
        session,
        child,
        link,
        next: 0,
        reply: String::new(),
        cut: false,
        cancelled_at: None,
    };
    match work.exchange(folder, text) {
        Ok(outcome) | Err(outcome) => outcome,
    }
}

/// The host as the ACP client of a task's agent.
struct Work<'a> {
    engine: &'a Arc<Engine>,
    session: &'a Arc<Session>,
    child: &'a Arc<Child>,
    link: Arc<Link>,
    next: u64,
    reply: String,
    cut: bool,
    cancelled_at: Option<Instant>,
}

impl Work<'_> {
    fn exchange(&mut self, folder: &Path, text: &str) -> Result<Outcome, Outcome> {
        self.call(
            "initialize",
            json!({
                "protocolVersion": 1,
                "clientCapabilities": {},
                "clientInfo": {"name": "canoa-delegate", "version": env!("CARGO_PKG_VERSION")},
            }),
        )?;
        let opened = self.call(
            "session/new",
            json!({"cwd": folder.to_string_lossy(), "mcpServers": []}),
        )?;
        let Some(id) = opened
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            return Err(Outcome::Failed("the agent opened no session".to_owned()));
        };
        *lock(&self.child.session) = Some(id.clone());
        let modes = privilege::modes_of(&opened);
        let parent = self.session.mode();
        let wanted = privilege::choose_mode(parent.as_deref(), &modes.available)
            .map_err(Outcome::Rejected)?;
        if modes.current.as_deref() != Some(wanted.as_str()) {
            self.set_mode(&modes, &id, &wanted)?;
        }
        let turn = self.call(
            "session/prompt",
            json!({"sessionId": id, "prompt": [{"type": "text", "text": text}]}),
        )?;
        let stop = turn
            .get("stopReason")
            .and_then(Value::as_str)
            .unwrap_or("end_turn")
            .to_owned();
        if self.child.cancelled() {
            return Ok(Outcome::Cancelled);
        }
        if stop == "cancelled" {
            return Ok(Outcome::Failed("the agent cancelled its turn".to_owned()));
        }
        Ok(Outcome::Reply {
            text: core::mem::take(&mut self.reply),
            stop,
            cut: self.cut,
        })
    }

    fn set_mode(&mut self, modes: &Modes, session: &str, wanted: &str) -> Result<(), Outcome> {
        let (method, params) = if modes.set_mode {
            (
                "session/set_mode",
                json!({"sessionId": session, "modeId": wanted}),
            )
        } else if let Some(option) = &modes.option {
            (
                "session/set_config_option",
                json!({"sessionId": session, "configId": option, "value": wanted}),
            )
        } else {
            return Err(Outcome::Rejected(
                "the agent offers no way to set its mode".to_owned(),
            ));
        };
        match self.call(method, params) {
            Ok(_) => Ok(()),
            Err(Outcome::Failed(why)) => Err(Outcome::Rejected(format!(
                "the agent would not take the mode {wanted}: {why}"
            ))),
            Err(other) => Err(other),
        }
    }

    /// Sends a request and reads until its answer, serving what the agent asks meanwhile.
    fn call(&mut self, method: &str, params: Value) -> Result<Value, Outcome> {
        self.next += 1;
        let id = self.next;
        let request = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        if !send(&self.link, &request) {
            return Err(self.lost());
        }
        loop {
            if self.child.cancelled() {
                let since = *self.cancelled_at.get_or_insert_with(Instant::now);
                if since.elapsed() > GRACE {
                    return Err(Outcome::Cancelled);
                }
            }
            match self.link.receive(Some(POLL)) {
                Ok(Some(bytes)) => {
                    let Ok(message) = serde_json::from_slice::<Value>(&bytes) else {
                        continue;
                    };
                    if message.get("method").is_some() {
                        self.incoming(&message);
                    } else if message.get("id") == Some(&json!(id)) {
                        return match message.get("error") {
                            Some(error) => Err(Outcome::Failed(format!(
                                "{method} failed: {}",
                                error
                                    .get("message")
                                    .and_then(Value::as_str)
                                    .unwrap_or("an error")
                            ))),
                            None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
                        };
                    }
                }
                Ok(None) | Err(LinkError::Ended | LinkError::TooLong) => return Err(self.lost()),
                Err(LinkError::Timeout) => {}
            }
        }
    }

    fn lost(&self) -> Outcome {
        if self.child.cancelled() {
            Outcome::Cancelled
        } else {
            Outcome::Failed("the agent's connection ended".to_owned())
        }
    }

    /// What the agent sent on its own: an update to keep, or a request to answer.
    fn incoming(&mut self, message: &Value) {
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let id = message.get("id").cloned();
        match (method, id) {
            ("session/update", None) => self.record(&message["params"]["update"]),
            ("session/request_permission", Some(id)) => self.permission(&id, &message["params"]),
            (_, Some(id)) => {
                let refusal = json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601,
                    "message": "this host gives a delegated task no files, terminal or other request of its own"}});
                send(&self.link, &refusal);
            }
            _ => {}
        }
    }

    fn record(&mut self, update: &Value) {
        let limit = self.engine.config.history_bytes;
        self.child.keep(update.to_string(), limit);
        if update.get("sessionUpdate").and_then(Value::as_str) == Some("agent_message_chunk")
            && let Some(text) = update
                .pointer("/content/text")
                .and_then(Value::as_str)
                .filter(|_| update.pointer("/content/type") == Some(&json!("text")))
        {
            let room = limit.saturating_sub(self.reply.len());
            if text.len() <= room {
                self.reply.push_str(text);
            } else {
                let mut end = room;
                while !text.is_char_boundary(end) {
                    end -= 1;
                }
                self.reply.push_str(&text[..end]);
                self.cut = true;
            }
        }
    }

    /// Passes a question of the task's agent to the person on the delegating session's connection. The task reads as waiting on
    /// the person until they answer; no answer, or a connection that ends first, cancels the question.
    fn permission(&mut self, id: &Value, params: &Value) {
        let task = json!({"taskId": self.child.task, "title": self.child.title, "source": self.child.source});
        let store = &self.session.store;
        let _ = store.update_status(
            &self.child.task,
            TaskState::InputRequired,
            Some(note(&format!(
                "waiting for the person: {}",
                self.child.title
            ))),
        );
        let asked = match (self.session.conn(), self.session.session_id()) {
            (Some(conn), Some(session)) => {
                let child = self.child;
                let engine = self.engine;
                conn.ask(&session, &task, params.clone(), &|| {
                    child.cancelled() || engine.stopped()
                })
            }
            _ => None,
        };
        let _ = store.update_status(&self.child.task, TaskState::Working, None);
        let result = asked
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({"outcome": {"outcome": "cancelled"}}));
        send(
            &self.link,
            &json!({"jsonrpc": "2.0", "id": id, "result": result}),
        );
    }
}
