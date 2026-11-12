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
    ListTasksResponse, Role, SendMessageRequest, SendMessageResponse, SubscribeToTaskRequest, Task,
};
use coqui_a2a::{Agent, Caller, Events};
use serde_json::{Map, Value, json};

use super::child::{self, Child, Job};
use super::{Engine, Session, privilege};

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

    /// The engine and the session a caller is, or why it is neither.
    fn of(&self, caller: &Caller<'_>) -> Result<(Arc<Engine>, Arc<Session>), A2aError> {
        let engine = self
            .engine
            .upgrade()
            .ok_or_else(|| A2aError::new(ErrorKind::Internal, "the host is stopping"))?;
        if !caller.is_local() {
            return Err(A2aError::new(
                ErrorKind::PermissionDenied,
                "this host delegates to the sessions it runs only",
            ));
        }
        let session = engine
            .session_of_peer(caller.peer)
            .ok_or_else(|| A2aError::new(ErrorKind::Unauthenticated, "no such session"))?;
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
                description: format!("A session of {}", source.name),
                id: source.name,
                ..AgentSkill::default()
            })
            .collect()
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
        let roots = session
            .conn()
            .map(|conn| conn.roots().to_vec())
            .unwrap_or_default();
        let folder = privilege::folder(&session.cwd, asked, &roots)
            .map_err(|why| A2aError::new(ErrorKind::PermissionDenied, why))?;
        let title = title_of(
            request
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.get("title"))
                .and_then(Value::as_str),
            &text,
        );
        let reservation = engine.reserve(&session)?;
        let context = session.session_id().ok_or_else(|| {
            A2aError::new(ErrorKind::Unauthenticated, "the session is not open yet")
        })?;
        // The host's words, not the caller's: where the task runs and what it is called are what the host settled.
        let mut metadata = Map::new();
        metadata.insert("title".to_owned(), json!(title));
        metadata.insert("source".to_owned(), json!(source));
        metadata.insert("folder".to_owned(), json!(folder.to_string_lossy()));
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
        let child = Child::new(task.id.clone(), title, source);
        let job = Job {
            engine: Arc::clone(&engine),
            session: Arc::clone(&session),
            child,
            folder,
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
        let (_, events) = self.start(caller, request, true)?;
        events.ok_or_else(|| A2aError::new(ErrorKind::Internal, "no stream"))
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
