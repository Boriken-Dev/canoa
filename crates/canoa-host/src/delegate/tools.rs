//! The three MCP tools an agent is given, mapped onto A2A calls of the delegation agent with the caller of the session whose
//! bearer was presented: `delegate_task` is `SendMessage`, `task_status` is `GetTask` or `SubscribeToTask`, `cancel` is
//! `CancelTask`.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "channels, threads and the clock are std's"
)]

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use coqui_a2a::error::A2aError;
use coqui_a2a::model::{
    Message, Part, Role, SendMessageConfiguration, SendMessageRequest, SendMessageResponse,
    StreamResponse, Task, TaskState,
};
use coqui_a2a::server::Reply;
use coqui_a2a::wire::Method;
use serde_json::{Map, Value, json};

use super::agent::DelegationAgent;
use super::{Engine, MAX_WAIT, Session, Who};

/// The tools and their input, as `tools/list` answers.
pub(crate) fn list(engine: &Engine, session: &Session) -> Value {
    let sources: Vec<String> = DelegationAgent::skills(engine, session)
        .into_iter()
        .map(|skill| skill.id)
        .collect();
    let named = if sources.is_empty() {
        "No source is available to this session.".to_owned()
    } else {
        format!("One of: {}.", sources.join(", "))
    };
    json!({"tools": [
        {
            "name": "delegate_task",
            "description": "Hand a task to another agent. It runs as its own session, sees only the task text you give it and \
                            nothing of this conversation, and works in a folder at or below this session's. Answers a task id \
                            at once; read the result with task_status.",
            "inputSchema": {"type": "object", "properties": {
                "source": {"type": "string", "description": named},
                "task": {"type": "string", "description": "Everything the other agent needs, as one self-contained instruction."},
                "folder": {"type": "string", "description": "Where it works: this session's folder or one below it. Default: this session's folder."},
                "title": {"type": "string", "description": "A short name the person sees for the task."},
            }, "required": ["source", "task"], "additionalProperties": false},
        },
        {
            "name": "task_status",
            "description": "The state of a task you handed out, and its result once it has completed. With wait_ms it waits up to \
                            that long for the task to change state or end; a wait that runs out cancels nothing.",
            "inputSchema": {"type": "object", "properties": {
                "taskId": {"type": "string"},
                "wait_ms": {"type": "integer", "minimum": 0, "description": "Milliseconds to wait for a change. Default: do not wait."},
            }, "required": ["taskId"], "additionalProperties": false},
        },
        {
            "name": "cancel",
            "description": "Stop a task you handed out, and the agent working on it.",
            "inputSchema": {"type": "object", "properties": {"taskId": {"type": "string"}},
                "required": ["taskId"], "additionalProperties": false},
        },
    ]})
}

/// Runs the tool `name` and answers as `tools/call` does: the text, and whether it is an error.
pub(crate) fn call(
    engine: &Arc<Engine>,
    session: &Arc<Session>,
    name: &str,
    args: &Value,
) -> Value {
    let answer = match name {
        "delegate_task" => delegate(engine, session, args),
        "task_status" => status(engine, session, args),
        "cancel" => cancel(engine, session, args),
        other => Err(format!("no tool is named {other}")),
    };
    match answer {
        Ok(value) => {
            json!({"content": [{"type": "text", "text": value.to_string()}], "isError": false})
        }
        Err(why) => json!({"content": [{"type": "text", "text": why}], "isError": true}),
    }
}

fn a2a(engine: &Engine, who: &Who, method: Method, params: Value) -> Result<Reply, String> {
    engine
        .a2a
        .call(&who.caller(), method, params)
        .map_err(|error| reason(&error))
}

/// What an A2A error says, for an agent to read.
fn reason(error: &A2aError) -> String {
    error.message.clone()
}

fn who(session: &Session) -> Result<Who, String> {
    session
        .peer()
        .map(Who::new)
        .ok_or_else(|| "this session is not open yet".to_owned())
}

fn text_arg<'a>(args: &'a Value, name: &str) -> Result<Option<&'a str>, String> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text)),
        Some(_) => Err(format!("{name} is text")),
    }
}

fn delegate(engine: &Arc<Engine>, session: &Arc<Session>, args: &Value) -> Result<Value, String> {
    let source = text_arg(args, "source")?.ok_or("source is required")?;
    let task = text_arg(args, "task")?.ok_or("task is required")?;
    let who = who(session)?;
    let mut metadata = Map::new();
    for key in ["folder", "title"] {
        if let Some(text) = text_arg(args, key)? {
            metadata.insert(key.to_owned(), json!(text));
        }
    }
    let number = engine.next.fetch_add(1, Ordering::SeqCst);
    let mut request =
        SendMessageRequest::new(Message::text(Role::User, format!("m-{number}"), task));
    request.tenant = Some(source.to_owned());
    request.metadata = Some(metadata);
    request.configuration = Some(SendMessageConfiguration {
        return_immediately: Some(true),
        ..SendMessageConfiguration::default()
    });
    let params = serde_json::to_value(&request).map_err(|error| error.to_string())?;
    match a2a(engine, &who, Method::SendMessage, params)? {
        Reply::Message(SendMessageResponse::Task(task)) => Ok(json!({
            "taskId": task.id,
            "state": state_name(&task.status.state),
        })),
        _ => Err("the host answered no task".to_owned()),
    }
}

fn task_id(args: &Value) -> Result<&str, String> {
    text_arg(args, "taskId")?.ok_or_else(|| "taskId is required".to_owned())
}

fn status(engine: &Arc<Engine>, session: &Arc<Session>, args: &Value) -> Result<Value, String> {
    let id = task_id(args)?;
    let wait = match args.get("wait_ms") {
        None | Some(Value::Null) => 0,
        Some(value) => value
            .as_u64()
            .ok_or("wait_ms is a number of milliseconds")?,
    };
    let who = who(session)?;
    let mut task = get(engine, &who, id)?;
    if wait > 0 && !task.status.state.is_terminal() {
        let limit = Duration::from_millis(wait).min(MAX_WAIT);
        wait_for_change(engine, &who, id, &task.status.state, limit);
        task = get(engine, &who, id)?;
    }
    Ok(describe(&task, engine.config.result_bytes))
}

fn get(engine: &Engine, who: &Who, id: &str) -> Result<Task, String> {
    match a2a(engine, who, Method::GetTask, json!({"id": id}))? {
        Reply::Task(task) => Ok(task),
        _ => Err("the host answered no task".to_owned()),
    }
}

/// Returns when the task leaves `state`, ends, or `limit` runs out, through a subscription so that nothing is polled.
fn wait_for_change(engine: &Engine, who: &Who, id: &str, state: &TaskState, limit: Duration) {
    let Ok(events) = engine
        .a2a
        .stream(&who.caller(), Method::SubscribeToTask, json!({"id": id}))
    else {
        return;
    };
    let (sender, receiver) = mpsc::channel();
    // The watch of `who` closes when the call is over, which ends the thread.
    let forwarded = std::thread::Builder::new()
        .name("canoa-delegate-wait".to_owned())
        .spawn(move || {
            for item in events {
                if sender.send(item).is_err() {
                    break;
                }
            }
        });
    if forwarded.is_err() {
        return;
    }
    let until = Instant::now() + limit;
    loop {
        let left = until.saturating_duration_since(Instant::now());
        match receiver.recv_timeout(left) {
            Ok(Ok(StreamResponse::StatusUpdate(update))) if update.status.state != *state => return,
            Ok(Ok(item)) if item.ends_task() => return,
            Ok(Ok(_)) => {}
            Ok(Err(_)) | Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn cancel(engine: &Arc<Engine>, session: &Arc<Session>, args: &Value) -> Result<Value, String> {
    let id = task_id(args)?;
    let who = who(session)?;
    match a2a(engine, &who, Method::CancelTask, json!({"id": id}))? {
        Reply::Task(task) => Ok(describe(&task, engine.config.result_bytes)),
        _ => Err("the host answered no task".to_owned()),
    }
}

/// A state as an agent reads it: `working`, `input_required`, `completed`.
fn state_name(state: &TaskState) -> String {
    state
        .as_str()
        .trim_start_matches("TASK_STATE_")
        .to_ascii_lowercase()
}

/// A task as `task_status` answers it: its state, what the host settled about it, and once it has completed its reply, cut at
/// `result_bytes` and saying so.
fn describe(task: &Task, result_bytes: usize) -> Value {
    let mut out = Map::new();
    out.insert("taskId".to_owned(), json!(task.id));
    out.insert("state".to_owned(), json!(state_name(&task.status.state)));
    for key in ["title", "source"] {
        if let Some(value) = task
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.get(key))
        {
            out.insert(key.to_owned(), value.clone());
        }
    }
    if let Some(message) = &task.status.message {
        let said = message.joined_text();
        if !said.is_empty() {
            out.insert("message".to_owned(), json!(said));
        }
    }
    let result = task
        .artifacts
        .iter()
        .flatten()
        .find(|artifact| artifact.artifact_id == "result")
        .map(|artifact| {
            artifact
                .parts
                .iter()
                .filter_map(|part: &Part| part.text.as_deref())
                .collect::<String>()
        });
    if let Some(result) = result {
        if result.len() > result_bytes {
            let mut end = result_bytes;
            while !result.is_char_boundary(end) {
                end -= 1;
            }
            out.insert("result".to_owned(), json!(&result[..end]));
            out.insert("truncated".to_owned(), json!(true));
            out.insert("resultBytes".to_owned(), json!(result.len()));
        } else {
            out.insert("result".to_owned(), json!(result));
        }
    }
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use coqui_a2a::model::{Artifact, TaskStatus};

    use super::*;

    fn done(text: &str) -> Task {
        Task {
            id: "task-1".to_owned(),
            status: TaskStatus::now(TaskState::Completed),
            artifacts: Some(vec![Artifact {
                artifact_id: "result".to_owned(),
                parts: vec![Part::text(text)],
                ..Artifact::default()
            }]),
            ..Task::default()
        }
    }

    #[test]
    fn a_completed_task_answers_its_result_and_a_long_one_is_cut_and_says_so() {
        let short = describe(&done("fine"), 100);
        assert_eq!(short["state"], "completed");
        assert_eq!(short["result"], "fine");
        assert!(short.get("truncated").is_none());
        let long = describe(&done("0123456789"), 4);
        assert_eq!(long["result"], "0123");
        assert_eq!(long["truncated"], true);
        assert_eq!(long["resultBytes"], 10);
        // A cut never lands inside a character.
        assert_eq!(describe(&done("ab\u{e9}\u{e9}"), 3)["result"], "ab");
    }

    #[test]
    fn states_read_in_lower_case_words() {
        assert_eq!(state_name(&TaskState::InputRequired), "input_required");
        assert_eq!(state_name(&TaskState::Canceled), "canceled");
    }
}
