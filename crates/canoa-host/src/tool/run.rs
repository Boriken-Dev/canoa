//! The ACP side of a tool entry: requests from the client in, the tool's processes run, and their events turned into
//! updates by the entry's rules. One connection; its sessions each hold a process (`session`) or start one per prompt
//! (`message`).

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use core::time::Duration;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Instant;

use serde_json::{Value, json};

use super::template::{lookup, matches, render, render_json};
use super::{
    Action, CANCEL_GRACE, MAX_ASKS, MAX_LABEL, MAX_TEXT, NOTICES_META_KEY, Rule, Runs, Stop,
    TOOL_KINDS, ToolSpec, ToolStatus,
};
use crate::child::OUTPUT_LIMIT;
use crate::serve::MAX_LINE;

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const INTERNAL_ERROR: i64 = -32603;
/// Lines waiting to be written to one process; past it the process counts as stuck and is ended.
const LINES_IN: usize = 64;
/// How long a process that has closed its output has to exit before it is said to have ended without a status.
const EXIT_WAIT: Duration = Duration::from_secs(2);
/// How long a process whose input closed at the end of a connection has to exit before it is ended.
const QUIT_WAIT: Duration = Duration::from_secs(1);

/// Serves one connection until `input` ends, then ends every process.
pub(super) fn serve(
    spec: Arc<ToolSpec>,
    name: &str,
    input: impl Read,
    output: impl Write + Send + 'static,
) -> io::Result<()> {
    let link = Arc::new(Link {
        spec,
        name: name.to_owned(),
        out: Out(Arc::new(Mutex::new(Box::new(output)))),
        sessions: Mutex::new(BTreeMap::new()),
        next_session: AtomicU64::new(1),
        next_request: AtomicU64::new(1),
        asks: Mutex::new(BTreeMap::new()),
    });
    let mut reader = BufReader::new(input);
    let ended = loop {
        match read_line(&mut reader, MAX_LINE) {
            Ok(Line::Text(line)) => link.receive(&line),
            Ok(Line::End) => break Ok(()),
            Ok(Line::TooLong) => {
                tracing::warn!(entry = link.name, "a client message was over the limit");
                break Ok(());
            }
            Err(error) => break Err(error),
        }
    };
    link.end();
    ended
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

enum Line {
    Text(Vec<u8>),
    End,
    TooLong,
}

/// One line of at most `max` bytes, without its line break.
fn read_line(reader: &mut impl BufRead, max: usize) -> io::Result<Line> {
    loop {
        let mut line = Vec::new();
        let limit = u64::try_from(max).unwrap_or(u64::MAX).saturating_add(1);
        let read = reader.take(limit).read_until(b'\n', &mut line)?;
        if read == 0 {
            return Ok(Line::End);
        }
        if line.last() != Some(&b'\n') && line.len() > max {
            return Ok(Line::TooLong);
        }
        while matches!(line.last(), Some(b'\n' | b'\r')) {
            line.pop();
        }
        if !line.is_empty() {
            return Ok(Line::Text(line));
        }
    }
}

/// Where messages to the client go, one line each.
#[derive(Clone)]
struct Out(Arc<Mutex<Box<dyn Write + Send>>>);

impl Out {
    fn send(&self, message: &Value) {
        let line = line_of(message);
        let mut out = lock(&self.0);
        // A client that has gone ends the connection by its input; nothing here waits on it.
        let _ = out.write_all(&line).and_then(|()| out.flush());
    }

    fn reply(&self, id: &Value, result: Value) {
        self.send(&json!({"jsonrpc": "2.0", "id": id, "result": result}));
    }

    fn fail(&self, id: &Value, code: i64, message: &str) {
        self.send(
            &json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}),
        );
    }

    fn update(&self, session: &str, update: Value) {
        self.send(&json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {"sessionId": session, "update": update},
        }));
    }
}

/// A permission request waiting for the client's answer.
struct Ask {
    session: String,
    generation: u64,
    turn: u64,
    allow: Value,
    reject: Value,
}

/// One connection.
struct Link {
    spec: Arc<ToolSpec>,
    name: String,
    out: Out,
    sessions: Mutex<BTreeMap<String, Arc<Session>>>,
    next_session: AtomicU64,
    next_request: AtomicU64,
    asks: Mutex<BTreeMap<u64, Ask>>,
}

struct Session {
    id: String,
    cwd: PathBuf,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// The process running for this session: its only one in `session` mode, the latest prompt's in `message` mode.
    process: Option<Arc<Process>>,
    /// The latest process's number; events and answers for an older one are dropped.
    generation: u64,
    /// The turn running, if any.
    turn: Option<Turn>,
    /// Turns started, so a late timer can tell its turn from a later one.
    turns: u64,
    /// The session the tool reported, for `{session}`.
    tool_session: Option<String>,
    /// Why this session takes no more prompts.
    ended: Option<String>,
    /// Permission requests of the latest process still waiting.
    asks: usize,
    /// A cancelled turn's number and when its process is ended unless the turn has ended by then.
    deadline: Option<(u64, Instant)>,
    /// Whether a thread is waiting on `deadline`: one per session, however often a client cancels.
    timer: bool,
}

struct Turn {
    request: Value,
    number: u64,
    failure: Option<String>,
    cancelled: bool,
}

/// How a turn ends.
enum Outcome {
    Stop(Stop),
    Error(String),
}

impl Link {
    fn receive(self: &Arc<Self>, line: &[u8]) {
        let Ok(message) = serde_json::from_slice::<Value>(line) else {
            return self.out.fail(&Value::Null, PARSE_ERROR, "not JSON");
        };
        let Some(object) = message.as_object() else {
            return self.out.fail(
                &Value::Null,
                INVALID_REQUEST,
                "one JSON-RPC object per message",
            );
        };
        let params = object.get("params").cloned().unwrap_or(Value::Null);
        match (
            object.get("method").and_then(Value::as_str),
            object.get("id"),
        ) {
            (Some(method), Some(id)) => self.request(method, id, &params),
            (Some(method), None) => self.notification(method, &params),
            (None, Some(id)) => self.answer(id, object),
            (None, None) => {}
        }
    }

    fn request(self: &Arc<Self>, method: &str, id: &Value, params: &Value) {
        let answered = match method {
            "initialize" => Ok(json!({
                "protocolVersion": 1,
                "agentCapabilities": {
                    "loadSession": false,
                    "promptCapabilities": {"image": false, "audio": false, "embeddedContext": false},
                },
                "authMethods": [],
                "agentInfo": {"name": "canoa-host", "version": env!("CARGO_PKG_VERSION")},
            })),
            "session/new" => self.new_session(params),
            "session/prompt" => match self.prompt(id, params) {
                // The answer comes when the turn ends.
                Ok(()) => return,
                Err(error) => Err(error),
            },
            _ => Err((
                METHOD_NOT_FOUND,
                format!("{method} is not offered by this agent"),
            )),
        };
        match answered {
            Ok(result) => self.out.reply(id, result),
            Err((code, message)) => self.out.fail(id, code, &message),
        }
    }

    fn notification(self: &Arc<Self>, method: &str, params: &Value) {
        if method == "session/cancel"
            && let Some(session) = params
                .get("sessionId")
                .and_then(Value::as_str)
                .and_then(|id| self.session(id))
        {
            self.cancel(&session);
        }
    }

    fn session(&self, id: &str) -> Option<Arc<Session>> {
        lock(&self.sessions).get(id).cloned()
    }

    fn new_session(self: &Arc<Self>, params: &Value) -> Result<Value, (i64, String)> {
        let invalid = |what: &str| (INVALID_PARAMS, what.to_owned());
        let cwd = params
            .get("cwd")
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .filter(|cwd| cwd.is_absolute())
            .ok_or_else(|| invalid("cwd must be an absolute path"))?;
        match params.get("mcpServers") {
            None | Some(Value::Null) => {}
            Some(Value::Array(servers)) if servers.is_empty() => {}
            Some(_) => return Err(invalid("this agent takes no MCP servers")),
        }
        // Looking at a share would reach that machine with this user's credentials, before any check.
        if !local(&cwd) {
            return Err(invalid(
                "cwd on a network share or device namespace is refused: it would reach another machine",
            ));
        }
        if !cwd.is_dir() {
            return Err(invalid("cwd is not a directory"));
        }
        let id = format!("s{}", self.next_session.fetch_add(1, Ordering::SeqCst));
        let session = Arc::new(Session {
            id: id.clone(),
            cwd,
            state: Mutex::new(State::default()),
        });
        {
            let mut sessions = lock(&self.sessions);
            if sessions.len() >= self.spec.max_sessions {
                return Err((
                    INVALID_REQUEST,
                    "this connection holds as many sessions as it may".to_owned(),
                ));
            }
            sessions.insert(id.clone(), Arc::clone(&session));
        }
        if self.spec.runs == Runs::Session {
            let args: Vec<String> = self.spec.args.clone();
            let mut state = lock(&session.state);
            if let Err(error) = self.start(&session, &mut state, &args, None) {
                drop(state);
                lock(&self.sessions).remove(&id);
                return Err((
                    INTERNAL_ERROR,
                    format!("{} did not start: {error}", self.name),
                ));
            }
        }
        Ok(json!({
            "sessionId": id,
            "_meta": {NOTICES_META_KEY: self.spec.notices(&self.name)},
        }))
    }

    /// Starts a process for `session` with `args`, writing `input` to it and closing its input when given.
    fn start(
        self: &Arc<Self>,
        session: &Arc<Session>,
        state: &mut State,
        args: &[String],
        input: Option<Vec<u8>>,
    ) -> io::Result<()> {
        state.generation += 1;
        state.asks = 0;
        let (process, stdout, stderr) =
            Process::spawn(&self.spec.program, args, &session.cwd, state.generation)?;
        let process = Arc::new(process);
        if let Some(input) = input {
            let _ = process.write(input);
            process.close_input();
        }
        forward(&self.name, stderr);
        let (link, reading, read) = (Arc::clone(self), Arc::clone(session), Arc::clone(&process));
        let spawned = thread::Builder::new()
            .name("canoa-host-tool".to_owned())
            .spawn(move || link.read_tool(&reading, &read, stdout));
        if let Err(error) = spawned {
            process.stop();
            return Err(error);
        }
        state.process = Some(process);
        Ok(())
    }

    fn prompt(self: &Arc<Self>, id: &Value, params: &Value) -> Result<(), (i64, String)> {
        let session = params
            .get("sessionId")
            .and_then(Value::as_str)
            .and_then(|id| self.session(id))
            .ok_or((INVALID_PARAMS, "no such session".to_owned()))?;
        let text =
            prompt_text(params.get("prompt")).map_err(|why| (INVALID_PARAMS, why.to_owned()))?;
        let mut state = lock(&session.state);
        if state.turn.is_some() {
            return Err((
                INVALID_REQUEST,
                "a turn is already running in this session".to_owned(),
            ));
        }
        if let Some(ended) = &state.ended {
            return Err((INTERNAL_ERROR, ended.clone()));
        }
        state.turns += 1;
        state.turn = Some(Turn {
            request: id.clone(),
            number: state.turns,
            failure: None,
            cancelled: false,
        });
        let context = json!({
            "prompt": text,
            "session": state.tool_session.clone().unwrap_or_default(),
        });
        match self.spec.runs {
            Runs::Session => {
                let line = render_json(self.spec.prompt.as_ref().unwrap_or(&Value::Null), &context);
                let written = state
                    .process
                    .as_ref()
                    .is_some_and(|process| process.write(line_of(&line)).is_ok());
                if !written {
                    let why = format!("{} did not take the prompt", self.name);
                    self.finish(&session, &mut state, Outcome::Error(why));
                }
            }
            Runs::Message => {
                if let Some(old) = state.process.take() {
                    // An earlier prompt's process still running is ended, with what it started, off this thread.
                    thread::spawn(move || old.stop());
                }
                let mut args: Vec<String> = Vec::new();
                if state.tool_session.is_some() {
                    args.extend(
                        self.spec
                            .resume_args
                            .iter()
                            .map(|arg| render(arg, &context)),
                    );
                }
                args.extend(self.spec.args.iter().map(|arg| render(arg, &context)));
                // Its input closes either way, so a tool that reads it finds the end.
                let input = if self.spec.prompt_in_args() {
                    Vec::new()
                } else {
                    text.into_bytes()
                };
                if let Err(error) = self.start(&session, &mut state, &args, Some(input)) {
                    let why = format!("{} did not start: {error}", self.name);
                    self.finish(&session, &mut state, Outcome::Error(why));
                }
            }
        }
        Ok(())
    }

    fn cancel(self: &Arc<Self>, session: &Arc<Session>) {
        let mut state = lock(&session.state);
        let Some(turn) = state.turn.as_mut() else {
            return;
        };
        if turn.cancelled {
            return;
        }
        turn.cancelled = true;
        let number = turn.number;
        // A question this turn still has waiting is answered no now: the person can no longer be asked it.
        self.reject_waiting(&session.id, &mut state);
        let Some(process) = state.process.clone() else {
            return self.finish(session, &mut state, Outcome::Stop(Stop::Cancelled));
        };
        if let (Runs::Session, Some(cancel)) = (self.spec.runs, &self.spec.cancel)
            && process.write(line_of(cancel)).is_ok()
        {
            state.deadline = Some((number, Instant::now() + CANCEL_GRACE));
            if !state.timer {
                state.timer = true;
                let (link, session) = (Arc::clone(self), Arc::clone(session));
                thread::spawn(move || link.cancel_timer(&session));
            }
            return;
        }
        // Ended, its output closes and the turn ends as cancelled from there.
        if self.spec.runs == Runs::Session {
            state.ended = Some(format!(
                "{} was ended to cancel a turn: open a new session",
                self.name
            ));
        }
        drop(state);
        process.stop();
    }

    /// Waits on the session's deadline: a turn asked to stop that has not after [`CANCEL_GRACE`] has its process ended,
    /// and the session with it.
    fn cancel_timer(self: &Arc<Self>, session: &Arc<Session>) {
        let mut state = lock(&session.state);
        let number = loop {
            match state.deadline {
                None => {
                    state.timer = false;
                    return;
                }
                Some((number, at)) if Instant::now() >= at => {
                    state.deadline = None;
                    state.timer = false;
                    break number;
                }
                Some((_, at)) => {
                    drop(state);
                    thread::sleep(at.saturating_duration_since(Instant::now()));
                    state = lock(&session.state);
                }
            }
        };
        if state.turn.as_ref().is_none_or(|turn| turn.number != number) {
            return;
        }
        state.ended = Some(format!(
            "{} did not stop its turn when asked and was ended: open a new session",
            self.name
        ));
        let process = state.process.take();
        self.finish(session, &mut state, Outcome::Stop(Stop::Cancelled));
        drop(state);
        if let Some(process) = process {
            process.stop();
        }
    }

    /// Ends the turn running, answering its prompt. A cancelled turn is answered `cancelled`, however it ended.
    fn finish(&self, session: &Session, state: &mut State, outcome: Outcome) {
        let Some(turn) = state.turn.take() else {
            return;
        };
        state.deadline = None;
        self.reject_waiting(&session.id, state);
        let outcome = match (turn.cancelled, outcome, turn.failure) {
            (true, _, _) => Outcome::Stop(Stop::Cancelled),
            (false, Outcome::Stop(Stop::EndTurn), Some(failure)) => Outcome::Error(failure),
            (false, outcome, _) => outcome,
        };
        match outcome {
            Outcome::Stop(stop) => self
                .out
                .reply(&turn.request, json!({"stopReason": stop.as_str()})),
            Outcome::Error(why) => {
                let why = label(&why);
                tracing::info!(entry = self.name, session = session.id, "a turn failed");
                self.out.fail(&turn.request, INTERNAL_ERROR, &why);
            }
        }
    }

    /// Reads one process's output to its end, applying the rules to each JSON line.
    fn read_tool(
        self: &Arc<Self>,
        session: &Arc<Session>,
        process: &Arc<Process>,
        stdout: ChildStdout,
    ) {
        let mut reader = BufReader::new(stdout);
        let mut unread = 0_usize;
        loop {
            match read_line(&mut reader, MAX_LINE) {
                Ok(Line::Text(line)) => match serde_json::from_slice::<Value>(&line) {
                    Ok(event) => self.event(session, process, &event),
                    Err(_) => unread += 1,
                },
                Ok(Line::TooLong) => {
                    tracing::warn!(
                        entry = self.name,
                        "the tool wrote a line over the limit, so it was ended"
                    );
                    process.stop();
                    break;
                }
                Ok(Line::End) | Err(_) => break,
            }
        }
        if unread > 0 {
            tracing::debug!(
                entry = self.name,
                unread,
                "lines that were not JSON were passed over"
            );
        }
        let status = process.exit_status();
        self.ended(session, process, status);
    }

    /// A process's output has closed: its turn ends, and in `session` mode its session with it.
    fn ended(&self, session: &Session, process: &Process, status: Option<ExitStatus>) {
        lock(&self.asks)
            .retain(|_, ask| !(ask.session == session.id && ask.generation == process.generation));
        let mut state = lock(&session.state);
        if state.generation != process.generation {
            return;
        }
        state.process = None;
        let how = match status {
            Some(status) if status.success() => None,
            Some(status) => Some(format!("{} ended: {status}", self.name)),
            None => Some(format!("{} closed its output", self.name)),
        };
        if self.spec.runs == Runs::Session && state.ended.is_none() {
            state.ended = Some(format!("{} has ended: open a new session", self.name));
        }
        let outcome = match (self.spec.runs, how) {
            (Runs::Message, None) => Outcome::Stop(Stop::EndTurn),
            (Runs::Session, None) => Outcome::Error(format!("{} ended during the turn", self.name)),
            (_, Some(how)) => Outcome::Error(how),
        };
        self.finish(session, &mut state, outcome);
    }

    fn event(self: &Arc<Self>, session: &Arc<Session>, process: &Arc<Process>, event: &Value) {
        // An earlier prompt's process, being ended, says nothing more.
        if lock(&session.state).generation != process.generation {
            return;
        }
        let Some(rule) = self
            .spec
            .events
            .iter()
            .find(|rule| matches(&rule.when, event))
        else {
            return;
        };
        let text = |field: &Option<String>| {
            field
                .as_deref()
                .map(|template| render(template, event))
                .unwrap_or_default()
        };
        match rule.send {
            Action::Text => self.chunks(&session.id, "agent_message_chunk", &text(&rule.text)),
            Action::Thought => self.chunks(&session.id, "agent_thought_chunk", &text(&rule.text)),
            Action::Tool => {
                let mut update = json!({
                    "sessionUpdate": "tool_call",
                    "toolCallId": label(&text(&rule.id)),
                    "title": label(&text(&rule.title)),
                    "kind": self.kind(&text(&rule.kind)),
                    "status": "in_progress",
                });
                if let Some(input) = rule
                    .input
                    .as_deref()
                    .and_then(|path| lookup(event, path))
                    .filter(|input| input.to_string().len() <= MAX_TEXT)
                {
                    update["rawInput"] = input.clone();
                }
                self.out.update(&session.id, update);
            }
            Action::ToolDone => {
                let status = match rule.status.unwrap_or(ToolStatus::Completed) {
                    ToolStatus::Completed => "completed",
                    ToolStatus::Failed => "failed",
                };
                let mut update = json!({
                    "sessionUpdate": "tool_call_update",
                    "toolCallId": label(&text(&rule.id)),
                    "status": status,
                });
                let output = text(&rule.output);
                if !output.is_empty() {
                    update["content"] = json!([{
                        "type": "content",
                        "content": {"type": "text", "text": cut(&output, MAX_TEXT)},
                    }]);
                }
                self.out.update(&session.id, update);
            }
            Action::Failure => self.with_turn(session, process, |_, state| {
                if let Some(turn) = state.turn.as_mut() {
                    turn.failure = Some(text(&rule.text));
                }
            }),
            Action::End => {
                let outcome = match (&rule.error, rule.stop) {
                    (Some(_), _) => Outcome::Error(text(&rule.error)),
                    (None, stop) => Outcome::Stop(stop.unwrap_or(Stop::EndTurn)),
                };
                self.with_turn(session, process, |link, state| {
                    link.finish(session, state, outcome)
                });
            }
            Action::Session => {
                let id = label(&text(&rule.id));
                if !id.is_empty() {
                    self.with_turn(session, process, |_, state| state.tool_session = Some(id));
                }
            }
            Action::Ask => self.ask(session, process, rule, event),
            Action::Reply => {
                if let Some(line) = &rule.line {
                    let _ = process.write(line_of(&render_json(line, event)));
                }
            }
            Action::Ignore => {}
        }
    }

    /// Runs `apply` on the session's state when `process` is still its latest.
    fn with_turn(
        &self,
        session: &Session,
        process: &Process,
        apply: impl FnOnce(&Self, &mut State),
    ) {
        let mut state = lock(&session.state);
        if state.generation == process.generation {
            apply(self, &mut state);
        }
    }

    fn ask(&self, session: &Session, process: &Arc<Process>, rule: &Rule, event: &Value) {
        let allow = render_json(rule.allow.as_ref().unwrap_or(&Value::Null), event);
        let reject = render_json(rule.reject.as_ref().unwrap_or(&Value::Null), event);
        let mut state = lock(&session.state);
        let askable = state.generation == process.generation
            && state.turn.as_ref().is_some_and(|turn| !turn.cancelled)
            && state.asks < MAX_ASKS;
        if !askable {
            drop(state);
            tracing::info!(
                entry = self.name,
                "a permission request was refused: no turn could ask it"
            );
            let _ = process.write(line_of(&reject));
            return;
        }
        state.asks += 1;
        let id = self.next_request.fetch_add(1, Ordering::SeqCst);
        lock(&self.asks).insert(
            id,
            Ask {
                session: session.id.clone(),
                generation: process.generation,
                turn: state.turn.as_ref().map_or(0, |turn| turn.number),
                allow,
                reject,
            },
        );
        drop(state);
        let question = rule
            .title
            .as_deref()
            .map(|title| render(title, event))
            .unwrap_or_default();
        self.out.send(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "session/request_permission",
            "params": {
                "sessionId": session.id,
                // The title is shown cut short, so the whole question goes as the call's input too.
                "toolCall": {
                    "toolCallId": format!("canoa-host-ask-{id}"),
                    "title": label(&question),
                    "status": "pending",
                    "rawInput": {"text": cut(&question, MAX_TEXT)},
                },
                "options": [
                    {"optionId": "allow", "name": "Allow", "kind": "allow_once"},
                    {"optionId": "reject", "name": "Refuse", "kind": "reject_once"},
                ],
            },
        }));
    }

    /// Writes `reject` for every question of `session` still waiting, and forgets them: their turn has ended or is
    /// cancelled, so a later answer reaches nothing.
    fn reject_waiting(&self, session: &str, state: &mut State) {
        let waiting: Vec<Ask> = {
            let mut asks = lock(&self.asks);
            let ids: Vec<u64> = asks
                .iter()
                .filter(|(_, ask)| ask.session == session)
                .map(|(id, _)| *id)
                .collect();
            ids.iter().filter_map(|id| asks.remove(id)).collect()
        };
        state.asks = 0;
        let Some(process) = state.process.clone() else {
            return;
        };
        for ask in waiting {
            if ask.generation == process.generation {
                let _ = process.write(line_of(&ask.reject));
            }
        }
    }

    /// The client's answer to a permission request: `allow` only for the allow option selected, `reject` for anything else.
    fn answer(&self, id: &Value, message: &serde_json::Map<String, Value>) {
        let Some(ask) = id.as_u64().and_then(|id| lock(&self.asks).remove(&id)) else {
            return;
        };
        let allowed = message
            .get("result")
            .and_then(|result| result.get("outcome"))
            .is_some_and(|outcome| {
                outcome.get("outcome").and_then(Value::as_str) == Some("selected")
                    && outcome.get("optionId").and_then(Value::as_str) == Some("allow")
            });
        let Some(session) = self.session(&ask.session) else {
            return;
        };
        let mut state = lock(&session.state);
        // Only to the turn that asked, still running: an ended or cancelled one had its answer written already.
        let current = state.generation == ask.generation
            && state
                .turn
                .as_ref()
                .is_some_and(|turn| turn.number == ask.turn && !turn.cancelled);
        if !current {
            return;
        }
        state.asks = state.asks.saturating_sub(1);
        if let Some(process) = state.process.clone() {
            drop(state);
            let _ = process.write(line_of(if allowed { &ask.allow } else { &ask.reject }));
        }
    }

    /// `text` as updates of `kind`, each at most [`MAX_TEXT`] bytes.
    fn chunks(&self, session: &str, kind: &str, text: &str) {
        let mut rest = text;
        while !rest.is_empty() {
            let at = boundary(rest, MAX_TEXT);
            let (chunk, after) = rest.split_at(at);
            self.out.update(
                session,
                json!({"sessionUpdate": kind, "content": {"type": "text", "text": chunk}}),
            );
            rest = after;
        }
    }

    fn kind(&self, named: &str) -> &str {
        if let Some(kind) = TOOL_KINDS.iter().find(|kind| **kind == named) {
            return kind;
        }
        self.spec
            .kinds
            .get(named)
            .and_then(|kind| TOOL_KINDS.iter().find(|known| *known == kind))
            .copied()
            .unwrap_or("other")
    }

    /// The connection has ended: every process is asked to end by its input closing, then ended.
    fn end(&self) {
        let sessions: Vec<Arc<Session>> = lock(&self.sessions).values().cloned().collect();
        let processes: Vec<Arc<Process>> = sessions
            .iter()
            .filter_map(|session| lock(&session.state).process.take())
            .collect();
        for process in &processes {
            process.close_input();
        }
        let until = Instant::now() + QUIT_WAIT;
        for process in &processes {
            while Instant::now() < until && !process.has_exited() {
                thread::sleep(Duration::from_millis(20));
            }
            process.stop();
        }
    }
}

/// A prompt's text: its text blocks, and a resource link as its address. Anything else is refused.
fn prompt_text(prompt: Option<&Value>) -> Result<String, &'static str> {
    let blocks = prompt
        .and_then(Value::as_array)
        .ok_or("prompt is a list of content blocks")?;
    let mut parts = Vec::new();
    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => parts.push(
                block
                    .get("text")
                    .and_then(Value::as_str)
                    .ok_or("a text block has no text")?,
            ),
            Some("resource_link") => {
                parts.push(
                    block
                        .get("uri")
                        .and_then(Value::as_str)
                        .ok_or("a resource link has no uri")?,
                );
            }
            _ => return Err("this agent takes text and resource links only"),
        }
    }
    Ok(parts.join("\n"))
}

/// `value` as one line, with its line break. U+2028 and U+2029, which JSON allows raw in a string, are escaped: some
/// line readers, Node's `readline` among them, split a line at either.
pub(super) fn line_of(value: &Value) -> Vec<u8> {
    let text = serde_json::to_string(value).unwrap_or_default();
    let mut line = if text.contains(['\u{2028}', '\u{2029}']) {
        text.replace('\u{2028}', "\\u2028")
            .replace('\u{2029}', "\\u2029")
            .into_bytes()
    } else {
        text.into_bytes()
    };
    line.push(b'\n');
    line
}

/// Whether `path` names this machine's file system: on Windows a drive, not a share (`\\server\share`, `\\?\UNC\`) or a
/// device namespace (`\\.\`, `\\?\Volume{…}`). Elsewhere every path does.
fn local(path: &Path) -> bool {
    match path.components().next() {
        Some(std::path::Component::Prefix(prefix)) => matches!(
            prefix.kind(),
            std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)
        ),
        _ => true,
    }
}

/// The largest char boundary in `text` at or below `max`.
fn boundary(text: &str, max: usize) -> usize {
    if text.len() <= max {
        return text.len();
    }
    let mut at = max;
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// `text` cut to at most `max` bytes, marked when cut.
fn cut(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_owned();
    }
    format!("{}…", &text[..boundary(text, max.saturating_sub(3))])
}

/// Characters that can reorder or hide what is shown around them.
fn is_format(c: char) -> bool {
    matches!(c, '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2060}'..='\u{2069}' | '\u{feff}')
}

/// A short text from the tool for a title, an id or a failure: control characters become spaces, format characters go,
/// and it is cut to [`MAX_LABEL`].
fn label(text: &str) -> String {
    let clean: String = text
        .chars()
        .filter(|c| !is_format(*c))
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    cut(clean.trim(), MAX_LABEL)
}

/// Copies what a process writes to its standard error to this process's, up to [`OUTPUT_LIMIT`] bytes, then reads and drops
/// the rest, so a tool writing without end is never blocked by it.
fn forward(entry: &str, mut stderr: ChildStderr) {
    let entry = entry.to_owned();
    let spawned = thread::Builder::new()
        .name("canoa-host-tool-stderr".to_owned())
        .spawn(move || {
            let mut left = OUTPUT_LIMIT;
            let mut buffer = vec![0_u8; 8192];
            loop {
                let read = match stderr.read(&mut buffer) {
                    Ok(0) | Err(_) => return,
                    Ok(read) => read,
                };
                let before = left;
                let kept = read.min(before);
                left -= kept;
                if kept > 0 {
                    let _ = std::io::stderr().lock().write_all(&buffer[..kept]);
                    if kept == before {
                        tracing::warn!(entry, "the tool wrote more than {OUTPUT_LIMIT} bytes of errors; the rest is dropped");
                    }
                }
            }
        });
    if let Err(error) = spawned {
        tracing::warn!("the tool's errors cannot be read: {error}");
    }
}

/// One process of a tool, and what it started.
struct Process {
    child: Mutex<Child>,
    input: Mutex<Option<SyncSender<Vec<u8>>>>,
    #[cfg(windows)]
    job: Mutex<Option<win32job::Job>>,
    generation: u64,
}

impl Process {
    /// Starts `program args` in `cwd`. On Windows it is in a job object of its own before this returns; on Unix it stays in
    /// this process's group, which the host ends with the connection.
    fn spawn(
        program: &str,
        args: &[String],
        cwd: &Path,
        generation: u64,
    ) -> io::Result<(Self, ChildStdout, ChildStderr)> {
        let mut command = Command::new(program);
        command
            .args(args)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // The session's directory is the client's choice: cmd.exe, which runs a .cmd tool such as npm's shims, would
        // otherwise look there first for every program the script names bare.
        #[cfg(windows)]
        command.env("NoDefaultCurrentDirectoryInExePath", "1");
        let mut child = command.spawn()?;
        let streams = (child.stdin.take(), child.stdout.take(), child.stderr.take());
        let (Some(mut stdin), Some(stdout), Some(stderr)) = streams else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::other("the tool's standard streams are missing"));
        };
        #[cfg(windows)]
        let job = {
            use std::os::windows::io::AsRawHandle as _;
            let mut limits = win32job::ExtendedLimitInfo::new();
            limits.limit_kill_on_job_close();
            let job = win32job::Job::create_with_limit_info(&limits).and_then(|job| {
                job.assign_process(child.as_raw_handle() as isize)
                    .map(|()| job)
            });
            match job {
                Ok(job) => job,
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(io::Error::other(error));
                }
            }
        };
        // Writes go through a thread of their own, since a write to a process that has stopped reading never returns.
        let (input, lines) = mpsc::sync_channel::<Vec<u8>>(LINES_IN);
        thread::Builder::new()
            .name("canoa-host-tool-input".to_owned())
            .spawn(move || {
                while let Ok(line) = lines.recv() {
                    if stdin.write_all(&line).and_then(|()| stdin.flush()).is_err() {
                        return;
                    }
                }
            })?;
        Ok((
            Self {
                child: Mutex::new(child),
                input: Mutex::new(Some(input)),
                #[cfg(windows)]
                job: Mutex::new(Some(job)),
                generation,
            },
            stdout,
            stderr,
        ))
    }

    /// Queues `line` for the process's input. A full queue means it has stopped reading: it is ended.
    fn write(&self, line: Vec<u8>) -> io::Result<()> {
        let sent = match lock(&self.input).as_ref() {
            Some(input) => input.try_send(line),
            None => return Err(io::Error::other("the tool's input is closed")),
        };
        match sent {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => {
                self.stop();
                Err(io::Error::other("the tool stopped reading its input"))
            }
            Err(TrySendError::Disconnected(_)) => {
                Err(io::Error::other("the tool's input is closed"))
            }
        }
    }

    /// Closes the process's input once what is queued has been written.
    fn close_input(&self) {
        drop(lock(&self.input).take());
    }

    fn has_exited(&self) -> bool {
        !matches!(lock(&self.child).try_wait(), Ok(None))
    }

    /// How it ended, once it has within [`EXIT_WAIT`]; `None` if it has not.
    fn exit_status(&self) -> Option<ExitStatus> {
        let until = Instant::now() + EXIT_WAIT;
        loop {
            if let Ok(Some(status)) = lock(&self.child).try_wait() {
                return Some(status);
            }
            if Instant::now() >= until {
                return None;
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// Ends the process and collects it: on Windows with everything it started, by its job object.
    fn stop(&self) {
        self.close_input();
        #[cfg(windows)]
        drop(lock(&self.job).take());
        let mut child = lock(&self.child);
        let _ = child.kill();
        let _ = child.wait();
    }
}
