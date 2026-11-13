//! The host's delegation agent: an A2A `Agent` whose card lists the sources the caller may name, and whose tasks run on those
//! sources. Everything a task may be given is decided here, before a process starts, whoever asked: an agent's tool call, and
//! later a peer over a link.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "locks, the clock and paths are std's"
)]

use std::sync::{Arc, Weak};
use std::time::Duration;

use coqui_a2a::error::{A2aError, ErrorKind};
use coqui_a2a::model::{
    AgentCapabilities, AgentCard, AgentSkill, CancelTaskRequest, GetTaskRequest, ListTasksRequest,
    ListTasksResponse, Role, SendMessageRequest, SendMessageResponse, StreamResponse,
    SubscribeToTaskRequest, Task,
};
use coqui_a2a::{Agent, Caller, Events};
use serde_json::{Map, Value, json};

use super::child::{self, Child, Job};
use super::{Engine, Session, Who, lock, privilege};

/// The longest title a task is shown by, in characters.
const TITLE_CHARS: usize = 80;

/// The agent behind the host's card.
pub(crate) struct DelegationAgent {
    engine: Weak<Engine>,
}

impl DelegationAgent {
    pub(crate) fn new(engine: Weak<Engine>) -> Self {
        Self { engine }
    }

    /// The engine and the session a caller is, or why it is neither: a session of this host's own, or a key granted `a2a`
    /// that a link proved.
    fn of(&self, caller: &Caller<'_>) -> Result<(Arc<Engine>, Arc<Session>), A2aError> {
        let engine = self
            .engine
            .upgrade()
            .ok_or_else(|| A2aError::new(ErrorKind::Internal, "the host is stopping"))?;
        let session = if caller.is_local() {
            engine
                .session_of_peer(caller.peer)
                .ok_or_else(|| A2aError::new(ErrorKind::Unauthenticated, "no such session"))?
        } else {
            engine.remote_session(caller.peer)?
        };
        Ok((engine, session))
    }

    /// The sources `session` may name.
    pub(super) fn skills(engine: &Engine, session: &Session) -> Vec<AgentSkill> {
        let (Some(host), Some(conn)) = (engine.host(), session.conn()) else {
            return Vec::new();
        };
        privilege::sources(host.as_ref(), &engine.config, conn.principal())
            .into_iter()
            .map(|source| AgentSkill {
                name: source.title.clone().unwrap_or_else(|| source.name.clone()),
                description: if source.remote.is_some() {
                    format!(
                        "An agent on another machine, {}: the task's text is sent there",
                        source.name
                    )
                } else {
                    format!("A session of {}", source.name)
                },
                id: source.name,
                ..AgentSkill::default()
            })
            .collect()
    }

    /// A key's answer to what one of its own tasks asked of it (`INPUT_REQUIRED`): the data of its message goes to the task's
    /// agent as the person's answer. Only a key from another machine has such a task: a session of this host's own is asked
    /// on its own connection.
    fn answer(&self, caller: &Caller<'_>, request: SendMessageRequest) -> Result<Task, A2aError> {
        let (_, session) = self.of(caller)?;
        if !session.is_remote() {
            return Err(A2aError::new(
                ErrorKind::UnsupportedOperation,
                "a task takes no further message: it is watched and stopped, not answered",
            ));
        }
        request.validate()?;
        let id = request.message.task_id.clone().unwrap_or_default();
        // The caller's own task only: another's is `TaskNotFound`, as it is to every other call.
        let task = session.store.get(
            caller,
            &GetTaskRequest {
                id: id.clone(),
                ..GetTaskRequest::default()
            },
        )?;
        let answer = request
            .message
            .parts
            .iter()
            .find_map(|part| part.data.clone())
            .ok_or_else(|| A2aError::new(ErrorKind::InvalidParams, "an answer is a data part"))?;
        let child = lock(&session.children).get(&id).cloned();
        if child.is_some_and(|child| child.answer(answer)) {
            Ok(task)
        } else {
            Err(A2aError::new(
                ErrorKind::InvalidRequest,
                "the task asks nothing now",
            ))
        }
    }

    /// Checks a request against everything the session allows and starts its task; the task as it was made.
    fn start(
        &self,
        caller: &Caller<'_>,
        mut request: SendMessageRequest,
        follow: bool,
    ) -> Result<(Task, Option<Events>), A2aError> {
        let (engine, session) = self.of(caller)?;
        request.validate()?;
        if request.message.task_id.is_some() {
            return Err(A2aError::new(
                ErrorKind::UnsupportedOperation,
                "a task takes no further message: it is watched and stopped, not answered",
            ));
        }
        let text = request.message.joined_text();
        if text.trim().is_empty() {
            return Err(A2aError::new(
                ErrorKind::InvalidParams,
                "the task has no text",
            ));
        }
        if text.len() > engine.config.task_bytes {
            return Err(A2aError::new(
                ErrorKind::InvalidParams,
                format!(
                    "the task is {} bytes, past {} (task_bytes)",
                    text.len(),
                    engine.config.task_bytes
                ),
            )
            .with_metadata("limit", "task_bytes"));
        }
        let source = self
            .agent_card(caller)
            .select_skill(request.tenant.as_deref())?
            .id
            .clone();
        let asked = request
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.get("folder"))
            .map(|folder| {
                folder
                    .as_str()
                    .ok_or_else(|| A2aError::new(ErrorKind::InvalidParams, "folder is not text"))
            })
            .transpose()?;
        let listed = engine.source(&source);
        let to_remote = listed
            .as_ref()
            .is_some_and(|listed| listed.remote.is_some());
        // Neither a caller from another machine nor a task for one has a folder to name: the first is run where this host
        // puts it, and nothing of the second's leaves the machine.
        let folder = if session.is_remote() || to_remote {
            if asked.is_some() {
                return Err(A2aError::new(
                    ErrorKind::InvalidParams,
                    "a task that crosses machines takes no folder",
                ));
            }
            if session.is_remote() {
                let roots = listed.map(|listed| listed.roots).unwrap_or_default();
                Some(
                    privilege::remote_folder(&roots)
                        .map_err(|why| A2aError::new(ErrorKind::PermissionDenied, why))?,
                )
            } else {
                None
            }
        } else {
            let roots = session
                .conn()
                .map(|conn| conn.roots().to_vec())
                .unwrap_or_default();
            Some(
                privilege::folder(&session.cwd, asked, &roots)
                    .map_err(|why| A2aError::new(ErrorKind::PermissionDenied, why))?,
            )
        };
        let title = title_of(
            request
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.get("title"))
                .and_then(Value::as_str),
            &text,
        );
        let reservation = engine.reserve(&session)?;
        let context = session
            .session_id()
            .or_else(|| session.peer())
            .ok_or_else(|| {
                A2aError::new(ErrorKind::Unauthenticated, "the session is not open yet")
            })?;
        // The host's words, not the caller's: where the task runs and what it is called are what the host settled.
        let mut metadata = Map::new();
        metadata.insert("title".to_owned(), json!(title));
        metadata.insert("source".to_owned(), json!(source));
        if let Some(folder) = &folder {
            metadata.insert("folder".to_owned(), json!(folder.to_string_lossy()));
        }
        request.metadata = Some(metadata);
        request.tenant = Some(source.clone());
        request.message.context_id = Some(context);
        request.message.role = Role::User;
        let task = session.store.create(caller, &request)?;
        let events = if follow {
            Some(session.store.subscribe(
                caller,
                &SubscribeToTaskRequest {
                    id: task.id.clone(),
                    ..SubscribeToTaskRequest::default()
                },
            )?)
        } else {
            None
        };
        // The person's client is told of the task from its first state, before any work starts.
        relay_events(&session, &task.id);
        let child = Child::new(task.id.clone(), title, source);
        let job = Job {
            engine: Arc::clone(&engine),
            session: Arc::clone(&session),
            child,
            folder: folder.unwrap_or_else(|| session.cwd.clone()),
            text,
            reservation,
        };
        if let Err(error) = child::start(job) {
            let _ = session.store.update_status(
                &task.id,
                coqui_a2a::model::TaskState::Failed,
                Some(coqui_a2a::model::Message::text(
                    Role::Agent,
                    "note",
                    format!("the task's thread could not start: {error}"),
                )),
            );
            return Err(A2aError::new(
                ErrorKind::Internal,
                "the task could not start",
            ));
        }
        Ok((task, events))
    }
}

/// The events of a task that a key from another machine started, which cancels the task when the key's stream ends before the
/// task does: with its link gone, nobody is left to answer what the task asks or to read what it makes.
struct Following {
    events: Events,
    session: Arc<Session>,
    task: String,
}

impl Iterator for Following {
    type Item = Result<StreamResponse, A2aError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.events.next()
    }
}

impl Drop for Following {
    fn drop(&mut self) {
        let (Some(peer), Some(running)) =
            (self.session.peer(), self.session.store.snapshot(&self.task))
        else {
            return;
        };
        if running.status.state.is_terminal() {
            return;
        }
        let (session, task) = (Arc::clone(&self.session), self.task.clone());
        let spawned = std::thread::Builder::new()
            .name("canoa-delegate-orphan".to_owned())
            .spawn(move || {
                let who = Who::new(peer);
                let _ = session.store.cancel(
                    &who.caller(),
                    &CancelTaskRequest {
                        id: task,
                        ..CancelTaskRequest::default()
                    },
                );
            });
        if let Err(error) = spawned {
            tracing::warn!("delegation: an orphaned task could not be stopped: {error}");
        }
    }
}

/// Sends each state of the task `task` of `session` to the client of its connection, ending with the task's last.
fn relay_events(session: &Arc<Session>, task: &str) {
    let (Some(peer), Some(context)) = (session.peer(), session.session_id()) else {
        return;
    };
    let who = Who::new(peer);
    let request = SubscribeToTaskRequest {
        id: task.to_owned(),
        ..SubscribeToTaskRequest::default()
    };
    let Ok(events) = session.store.subscribe(&who.caller(), &request) else {
        return;
    };
    let (session, task) = (Arc::clone(session), task.to_owned());
    let spawned = std::thread::Builder::new()
        .name(format!("canoa-delegate-events-{task}"))
        .spawn(move || {
            let _who = who;
            for event in events.map_while(Result::ok) {
                let (Some(conn), Ok(event)) = (session.conn(), serde_json::to_value(&event)) else {
                    break;
                };
                conn.notify(
                    json!({"sessionId": context, "taskId": task, "event": event}),
                    false,
                );
            }
        });
    if let Err(error) = spawned {
        tracing::warn!("delegation: a task's events could not start their thread: {error}");
    }
}

/// A task's title: the one it was given, else the first line of its text; one line, cut.
fn title_of(given: Option<&str>, text: &str) -> String {
    let source = given
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| {
            text.lines()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("")
        });
    let flat: String = source
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let trimmed = flat.trim();
    if trimmed.chars().count() <= TITLE_CHARS {
        trimmed.to_owned()
    } else {
        let cut: String = trimmed.chars().take(TITLE_CHARS - 1).collect();
        format!("{cut}\u{2026}")
    }
}

impl Agent for DelegationAgent {
    fn agent_card(&self, caller: &Caller<'_>) -> AgentCard {
        let skills = self
            .of(caller)
            .map(|(engine, session)| Self::skills(&engine, &session))
            .unwrap_or_default();
        AgentCard {
            name: "Canoa".to_owned(),
            description: "Tasks that run as sessions of this host's agents".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            capabilities: AgentCapabilities {
                streaming: Some(true),
                push_notifications: Some(false),
                ..AgentCapabilities::default()
            },
            default_input_modes: vec!["text/plain".to_owned()],
            default_output_modes: vec!["text/plain".to_owned()],
            skills,
            ..AgentCard::default()
        }
    }

    fn send_message(
        &self,
        caller: &Caller<'_>,
        request: SendMessageRequest,
    ) -> Result<SendMessageResponse, A2aError> {
        if request.message.task_id.is_some() {
            return self.answer(caller, request).map(SendMessageResponse::Task);
        }
        let immediately = request
            .configuration
            .as_ref()
            .and_then(|configuration| configuration.return_immediately)
            == Some(true);
        let (task, _) = self.start(caller, request, false)?;
        if immediately {
            return Ok(SendMessageResponse::Task(task));
        }
        // Without `returnImmediately` the answer is the task once it ends or waits for the person.
        let (_, session) = self.of(caller)?;
        let mut latest = task;
        while !latest.status.state.is_terminal() && !latest.status.state.is_interrupted() {
            if !caller.is_open() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
            if let Some(task) = session.store.snapshot(&latest.id) {
                latest = task;
            }
        }
        Ok(SendMessageResponse::Task(latest))
    }

    fn send_streaming_message(
        &self,
        caller: &Caller<'_>,
        request: SendMessageRequest,
    ) -> Result<Events, A2aError> {
        let (task, events) = self.start(caller, request, true)?;
        let events = events.ok_or_else(|| A2aError::new(ErrorKind::Internal, "no stream"))?;
        let (_, session) = self.of(caller)?;
        Ok(if session.is_remote() {
            Box::new(Following {
                events,
                session,
                task: task.id,
            })
        } else {
            events
        })
    }

    fn get_task(&self, caller: &Caller<'_>, request: GetTaskRequest) -> Result<Task, A2aError> {
        self.of(caller)?.1.store.get(caller, &request)
    }

    fn list_tasks(
        &self,
        caller: &Caller<'_>,
        request: ListTasksRequest,
    ) -> Result<ListTasksResponse, A2aError> {
        self.of(caller)?.1.store.list(caller, &request)
    }

    fn cancel_task(
        &self,
        caller: &Caller<'_>,
        request: CancelTaskRequest,
    ) -> Result<Task, A2aError> {
        self.of(caller)?.1.store.cancel(caller, &request)
    }

    fn subscribe_to_task(
        &self,
        caller: &Caller<'_>,
        request: SubscribeToTaskRequest,
    ) -> Result<Events, A2aError> {
        self.of(caller)?.1.store.subscribe(caller, &request)
    }
}
