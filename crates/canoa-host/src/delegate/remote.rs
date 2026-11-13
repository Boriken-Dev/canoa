//! A task handed to an agent on another machine. The host dials it with its own identity, sends the task text and nothing else
//! (no folder, no history, no session), and mirrors the other host's task as its own: the states, the result, and the questions
//! its agent asks, which go to the person on the parent's card as any task's do. What the other agent may do is bounded by the
//! other host's grant to this host's key and by its own rules; nothing here can widen it.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "locks, threads and the clock are std's"
)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use coqui_a2a::model::{
    Artifact, CancelTaskRequest, Message, Part, Role, SendMessageRequest, StreamResponse,
    TaskState, TaskStatus,
};
use coqui_a2a::{Client, ClientError};
use coqui_agentic_peer::auth::{self, Proof, Sealed};
use coqui_agentic_peer::mux::{Heartbeat, Limits, Mux, Side};
use coqui_transport::Channel;
use coqui_transport_web::websocket::{self, Frames};
use coqui_transport_web::{Dialer as _, Kind};
use serde_json::{Value, json};

use super::child::{Child, Outcome};
use super::{Engine, Remote, Session, lock};

/// How long the dial and the proof may take.
const DIAL_LIMIT: Duration = Duration::from_secs(20);
/// The largest message of the link once the other host has proved itself.
const MAX_MESSAGE: usize = 16 * 1024 * 1024;
/// How often a link that says nothing is probed, and how many probes may go unanswered before it is lost.
const HEARTBEAT: (Duration, u32) = (Duration::from_secs(15), 3);
/// How often a stop is looked for while the other host's events are awaited.
const POLL: Duration = Duration::from_millis(50);
/// The most of the other host's words a status carries: they are another machine's, and shown as they are.
const NOTE_BYTES: usize = 4096;

/// The request that hands a task over: the text, addressed to `source`, and nothing else.
fn request(task: &str, text: &str, source: Option<&str>) -> SendMessageRequest {
    SendMessageRequest {
        tenant: source.map(str::to_owned),
        ..SendMessageRequest::new(Message::text(Role::User, format!("canoa-{task}"), text))
    }
}

/// What a task for another machine's agent came to.
pub(super) fn drive(
    engine: &Arc<Engine>,
    session: &Arc<Session>,
    child: &Arc<Child>,
    text: &str,
) -> Outcome {
    let name = &child.source;
    let Some((host, target)) = engine
        .host()
        .zip(engine.source(name).and_then(|source| source.remote))
    else {
        return Outcome::Failed(format!("{name} is not an agent on another machine"));
    };
    let proof = match host.prover(&target.peer) {
        Ok(proof) => proof,
        Err(why) => return Outcome::Failed(format!("{name} cannot be reached: {why}")),
    };
    let channel = match dial(&target.url, &proof) {
        Ok(channel) => Arc::new(channel),
        Err(why) => return Outcome::Failed(format!("{name} cannot be reached: {why}")),
    };
    let closing = Arc::clone(&channel);
    child.on_stop(move || closing.close());
    let outcome = linked(engine, session, child, text, (&proof, &channel, &target));
    channel.close();
    outcome
}

fn dial(url: &str, proof: &Proof) -> Result<websocket::Channel, String> {
    let url = url::Url::parse(url).map_err(|error| error.to_string())?;
    let mut limits = coqui_transport_web::Limits::default();
    limits.max_message = proof.method().record_length(MAX_MESSAGE);
    websocket::Dialer::builder()
        .frames(Frames::Binary)
        .limits(limits)
        .build()
        .map_err(|error| error.to_string())?
        .dial(Kind::WebSocket, &url, DIAL_LIMIT)
        .map_err(|failure| failure.to_string())
}

fn linked(
    engine: &Arc<Engine>,
    session: &Arc<Session>,
    child: &Arc<Child>,
    text: &str,
    (proof, channel, target): (&Proof, &Arc<websocket::Channel>, &Remote),
) -> Outcome {
    let name = &child.source;
    let established = match auth::prove_out(
        proof,
        &**channel,
        DIAL_LIMIT,
        proof.method().record_length(MAX_MESSAGE),
    ) {
        Ok(established) => established,
        Err(error) => return Outcome::Failed(format!("{name} did not prove its key: {error}")),
    };
    // The trust set names one key, but the key that answered is checked against the configuration, not the dialer.
    if established.peer != target.peer {
        return Outcome::Failed(format!("{name} proved a key other than the one configured"));
    }
    let sealed = Sealed::new(&**channel, established.session);
    let mut limits = Limits::default();
    limits.max_message = MAX_MESSAGE;
    limits.heartbeat = Some(Heartbeat::new(HEARTBEAT.0, HEARTBEAT.1));
    let outcome = Mux::run(&sealed, Side::Dialed, limits, None, |mux| {
        mirror(engine, session, child, text, target, mux)
    });
    sealed.close();
    outcome
}

fn mirror<C: Channel>(
    engine: &Arc<Engine>,
    session: &Arc<Session>,
    child: &Arc<Child>,
    text: &str,
    target: &Remote,
    mux: &Mux<'_, C>,
) -> Outcome {
    let client = Client::new(mux);
    let events = match client.send_streaming_message(&request(
        &child.task,
        text,
        target.source.as_deref(),
    )) {
        Ok(events) => events,
        Err(error) => return failed(child, &error),
    };
    let remote = Mutex::new(None::<String>);
    let over = AtomicBool::new(false);
    std::thread::scope(|scope| {
        // A stop here is a `CancelTask` there: the other host ends its own task, as it would for any caller.
        scope.spawn(|| {
            while !over.load(Ordering::SeqCst) {
                let task = lock(&remote).clone();
                if let Some(id) = task.filter(|_| child.cancelled()) {
                    let _ = client.cancel_task(&CancelTaskRequest {
                        id,
                        ..CancelTaskRequest::default()
                    });
                    return;
                }
                std::thread::sleep(POLL);
            }
        });
        let mut mirrored = Mirrored {
            engine,
            session,
            child,
            client: &client,
            remote: &remote,
            result_id: None,
            asked: 0,
        };
        let outcome = mirrored.follow(events);
        over.store(true, Ordering::SeqCst);
        outcome
    })
}

/// Why the other host did not carry a task through: what became of it, never a success.
fn failed(child: &Child, error: &ClientError) -> Outcome {
    if child.cancelled() {
        return Outcome::Cancelled;
    }
    let name = &child.source;
    Outcome::Failed(match error {
        ClientError::NotGranted(_) => {
            format!("{name} does not serve tasks to this host's key")
        }
        ClientError::Remote(error) => format!("{name} refused the task: {}", cut(&error.message)),
        // A link that ends is not a task that did: the task may still be running there, and nothing says how it ended.
        _ => format!("the link to {name} was lost before its task ended"),
    })
}

/// `text` cut to what a status carries.
fn cut(text: &str) -> String {
    let mut end = text.len().min(NOTE_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

/// The other host's task, followed into this host's.
struct Mirrored<'a, 'm, 'c, C: Channel> {
    engine: &'a Arc<Engine>,
    session: &'a Arc<Session>,
    child: &'a Arc<Child>,
    client: &'a Client<'m, 'c, C>,
    /// The id the other host gave its task, once it said.
    remote: &'a Mutex<Option<String>>,
    /// The id of the artifact that is the result, as the other host named it.
    result_id: Option<String>,
    asked: u64,
}

impl<C: Channel> Mirrored<'_, '_, '_, C> {
    fn follow(
        &mut self,
        events: impl Iterator<Item = Result<StreamResponse, ClientError>>,
    ) -> Outcome {
        for event in events {
            let ended = match event {
                Ok(StreamResponse::Task(task)) => {
                    *lock(self.remote) = Some(task.id);
                    for artifact in task.artifacts.unwrap_or_default() {
                        if let Some(ended) = self.artifact(artifact, false, true) {
                            return ended;
                        }
                    }
                    self.state(task.status)
                }
                Ok(StreamResponse::StatusUpdate(update)) => {
                    *lock(self.remote) = Some(update.task_id);
                    self.state(update.status)
                }
                Ok(StreamResponse::ArtifactUpdate(update)) => self.artifact(
                    update.artifact,
                    update.append.unwrap_or(false),
                    update.last_chunk.unwrap_or(true),
                ),
                // A message in place of a task is the answer.
                Ok(StreamResponse::Message(message)) => {
                    let artifact = Artifact {
                        artifact_id: "result".to_owned(),
                        parts: vec![Part::text(message.joined_text())],
                        ..Artifact::default()
                    };
                    self.artifact(artifact, false, true).or(Some(done()))
                }
                Ok(_) => None,
                Err(error) => return failed(self.child, &error),
            };
            if let Some(ended) = ended {
                return ended;
            }
        }
        // A stream that finished in order but never put the task in a final state says nothing of how it ended.
        failed(self.child, &ClientError::Interrupted)
    }

    /// Adds what the other host's task made to this one's; `Some` when that cannot be done and the task ends.
    fn artifact(&mut self, mut artifact: Artifact, append: bool, last: bool) -> Option<Outcome> {
        let name = &self.child.source;
        // The tools read the result as the artifact `result`, whatever the other agent called its first.
        let first = self
            .result_id
            .get_or_insert_with(|| artifact.artifact_id.clone());
        if *first == artifact.artifact_id {
            "result".clone_into(&mut artifact.artifact_id);
        }
        self.session
            .store
            .add_artifact(&self.child.task, artifact, append, last)
            .err()
            .map(|error| {
                Outcome::Failed(format!(
                    "the result of {name} could not be kept: {}",
                    error.message
                ))
            })
    }

    /// Follows the other host's task into `status`; `Some` when the task has ended.
    fn state(&mut self, status: TaskStatus) -> Option<Outcome> {
        let name = &self.child.source;
        let said = status
            .message
            .as_ref()
            .map(|message| cut(&message.joined_text()))
            .unwrap_or_default();
        match status.state {
            TaskState::Working => {
                let _ =
                    self.session
                        .store
                        .update_status(&self.child.task, TaskState::Working, None);
                None
            }
            TaskState::InputRequired => self.question(status.message.as_ref()),
            TaskState::AuthRequired => Some(Outcome::Failed(format!(
                "{name} needs authentication this host cannot give: {said}"
            ))),
            TaskState::Completed => Some(done()),
            TaskState::Failed => Some(Outcome::Failed(format!("{name} failed the task: {said}"))),
            TaskState::Rejected => Some(Outcome::Rejected(format!(
                "{name} refused the task: {said}"
            ))),
            TaskState::Canceled if self.child.cancelled() => Some(Outcome::Cancelled),
            TaskState::Canceled => Some(Outcome::Failed(format!("{name} canceled the task"))),
            _ => None,
        }
    }

    /// The other agent's question to the person, put to them on the parent's card; their answer goes back as a message of the
    /// task. A question this host cannot put, or that nobody answers, is cancelled: the other agent is told so.
    fn question(&mut self, message: Option<&Message>) -> Option<Outcome> {
        let child = self.child;
        let store = &self.session.store;
        let _ = store.update_status(
            &child.task,
            TaskState::InputRequired,
            Some(Message::text(
                Role::Agent,
                format!("note-{}", child.title.len()),
                format!("waiting for the person: {}", child.title),
            )),
        );
        // Only the two members the card shows are taken from what the other host sent.
        let asked = message
            .and_then(|message| message.parts.iter().find_map(|part| part.data.as_ref()))
            .filter(|data| data["toolCall"].is_object() && data["options"].is_array())
            .map(|data| json!({"toolCall": data["toolCall"], "options": data["options"]}));
        let answer = match (asked, self.session.conn(), self.session.session_id()) {
            (Some(params), Some(conn), Some(session)) => {
                let task =
                    json!({"taskId": child.task, "title": child.title, "source": child.source});
                let (engine, child) = (self.engine, self.child);
                conn.ask(&session, &task, params, &|| {
                    child.cancelled() || engine.stopped()
                })
            }
            _ => None,
        };
        let _ = store.update_status(&child.task, TaskState::Working, None);
        let result = answer
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({"outcome": {"outcome": "cancelled"}}));
        let id = lock(self.remote).clone()?;
        self.asked += 1;
        let mut answer = Message::text(
            Role::User,
            format!("canoa-{}-answer-{}", child.task, self.asked),
            String::new(),
        );
        answer.task_id = Some(id);
        answer.parts = vec![Part::data(result)];
        self.client
            .send_message(&SendMessageRequest::new(answer))
            .err()
            .map(|error| failed(child, &error))
    }
}

/// A task that ended well: its result was added as it came, so nothing is left to say.
fn done() -> Outcome {
    Outcome::Reply {
        text: String::new(),
        stop: "end_turn".to_owned(),
        cut: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_task_for_another_machine_carries_its_text_and_the_source_and_nothing_else() {
        let sent = serde_json::to_value(request("task-1", "do it", Some("far"))).expect("JSON");
        assert_eq!(sent["tenant"], "far");
        assert_eq!(sent["message"]["parts"][0]["text"], "do it");
        let text = sent.to_string();
        for absent in ["folder", "contextId", "taskId", "metadata", "history"] {
            assert!(!text.contains(absent), "{absent} was sent: {text}");
        }
        let none = serde_json::to_value(request("task-1", "x", None)).expect("JSON");
        assert!(none.get("tenant").is_none(), "{none}");
    }

    #[test]
    fn the_other_hosts_words_are_cut_on_a_character() {
        let long = "\u{e9}".repeat(NOTE_BYTES);
        let kept = cut(&long);
        assert!(kept.len() <= NOTE_BYTES && kept.chars().all(|c| c == '\u{e9}'));
    }
}
