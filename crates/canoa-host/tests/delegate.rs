//! An agent session handing a task to another source: the tools over loopback HTTP behind a per-session bearer, the privilege
//! every task is held to before its process starts, and what a task asks of the person. Scripted stand-ins only: no real model
//! runs.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod linked;
mod support;

use std::io::{Read as _, Write as _};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use canoa_host::{Link, LinkError};
use linked::Hosted;
use serde_json::{Value, json};
use support::{Host, Scratch, WAIT, example, host_binary, quoted, running};

const META: &str = "dev.boriken.canoa/delegate";

type Socket = coqui_transport_web::websocket::Channel;

/// One scripted agent entry. `modes` is what its mode option lists, the first being the mode it starts in.
fn agent(name: &str, modes: &str, extra: &str) -> String {
    format!(
        "[host.agents.{name}]\nprogram = {}\nargs = [\"--scripted\", \"--mcp-http\", \"--modes={modes}\"{}]\nlisten = \"127.0.0.1:0\"\n",
        quoted(&example("stdio_agent", false).to_string_lossy()),
        extra
    )
}

/// An entry that records each process it starts in `marker`.
fn marked(name: &str, modes: &str, marker: &Path) -> String {
    agent(
        name,
        modes,
        &format!(
            ", \"--marker={}\"",
            marker.to_string_lossy().replace('\\', "/")
        ),
    )
}

fn directory(parent: &Path, name: &str) -> PathBuf {
    let dir = parent.join(name);
    std::fs::create_dir_all(&dir).expect("a directory");
    dir
}

/// A client of one link: it asks, and answers what the host asks of it as the test says.
struct Convo<'a> {
    link: &'a Link,
    next: u64,
    /// Every request of the host's own the client has seen: permission requests of a task.
    requests: Vec<Value>,
    updates: Vec<Value>,
    /// The host's `_canoa/delegate/event` notifications, in the order they came.
    events: Vec<Value>,
}

impl<'a> Convo<'a> {
    fn new(link: &'a Link) -> Self {
        Self {
            link,
            next: 0,
            requests: Vec::new(),
            updates: Vec::new(),
            events: Vec::new(),
        }
    }

    fn message(&self, wait: Duration) -> Option<Value> {
        let until = Instant::now() + wait;
        loop {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            match self
                .link
                .receive(Some(left.min(Duration::from_millis(200))))
            {
                Ok(Some(bytes)) => return serde_json::from_slice(&bytes).ok(),
                Ok(None) => return None,
                Err(LinkError::Timeout) => {}
                Err(_) => return None,
            }
        }
    }

    fn send(&self, message: &Value) {
        self.link
            .send(message.to_string().as_bytes())
            .expect("sent");
    }

    /// A request, answered; `answer` gives the result for a request of the host's, or `None` to leave it unanswered.
    fn request(
        &mut self,
        method: &str,
        params: Value,
        answer: &dyn Fn(&Value) -> Option<Value>,
    ) -> Result<Value, Value> {
        self.next += 1;
        let id = self.next;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        let until = Instant::now() + WAIT;
        loop {
            let message = self
                .message(until.saturating_duration_since(Instant::now()))
                .unwrap_or_else(|| panic!("no answer to {method}"));
            if message.get("method").is_some() {
                if message.get("id").is_some() {
                    self.requests.push(message.clone());
                    if let Some(result) = answer(&message) {
                        self.send(
                            &json!({"jsonrpc": "2.0", "id": message["id"], "result": result}),
                        );
                    }
                } else if message["method"] == "_canoa/delegate/event" {
                    self.events.push(message["params"].clone());
                } else {
                    self.updates.push(message);
                }
            } else if message["id"] == id {
                return match message.get("error") {
                    Some(error) => Err(error.clone()),
                    None => Ok(message["result"].clone()),
                };
            }
        }
    }

    fn open(&mut self, meta: bool, cwd: &Path) -> String {
        let mut params = json!({"protocolVersion": 1, "clientCapabilities": {}});
        if meta {
            params["_meta"] = json!({ META: true });
        }
        self.request("initialize", params, &|_| None)
            .expect("initialize");
        let session = self
            .request(
                "session/new",
                json!({"cwd": cwd.to_string_lossy(), "mcpServers": []}),
                &|_| None,
            )
            .expect("a session");
        session["sessionId"].as_str().expect("an id").to_owned()
    }

    /// One prompt and the text the agent said, leaving the host's questions unanswered.
    fn prompt(&mut self, session: &str, text: &str) -> String {
        self.prompt_answering(session, text, &|_| None)
    }

    fn prompt_answering(
        &mut self,
        session: &str,
        text: &str,
        answer: &dyn Fn(&Value) -> Option<Value>,
    ) -> String {
        self.updates.clear();
        let result = self
            .request(
                "session/prompt",
                json!({"sessionId": session, "prompt": [{"type": "text", "text": text}]}),
                answer,
            )
            .expect("the turn is answered");
        assert_eq!(result["stopReason"], "end_turn", "{text}");
        self.updates
            .iter()
            .filter(|update| update["params"]["update"]["sessionUpdate"] == "agent_message_chunk")
            .filter_map(|update| update["params"]["update"]["content"]["text"].as_str())
            .collect()
    }

    /// The task id a `Delegate` prompt answered.
    fn delegate(&mut self, session: &str, source: &str, task: &str) -> String {
        let said = self.prompt(session, &format!("Delegate {source} {task}"));
        let answer: Value =
            serde_json::from_str(&said).unwrap_or_else(|_| panic!("a task id: {said}"));
        answer["taskId"].as_str().expect("a task id").to_owned()
    }

    /// `task_status` as the parent's agent reads it, waiting up to `wait_ms` for the task to change. A wait of 20 seconds or more
    /// asks for the end: it repeats the call until the task is in a state it will not leave, as an agent would.
    fn status(&mut self, session: &str, task: &str, wait_ms: u64) -> Value {
        let limit = Instant::now() + WAIT;
        loop {
            let said = self.prompt(session, &format!("Status {task} {wait_ms}"));
            let state: Value =
                serde_json::from_str(&said).unwrap_or_else(|_| json!({"error": said}));
            let ended = ["completed", "failed", "canceled", "rejected"]
                .iter()
                .any(|name| state["state"] == *name);
            if wait_ms < 20_000 || ended || state.get("error").is_some() || Instant::now() > limit {
                return state;
            }
        }
    }

    /// Waits until the host sends a request of its own.
    fn host_request(&mut self) -> Value {
        let until = Instant::now() + WAIT;
        loop {
            if let Some(found) = self.requests.pop() {
                return found;
            }
            let message = self
                .message(until.saturating_duration_since(Instant::now()))
                .expect("the host asks the client something");
            if message.get("method").is_some() && message.get("id").is_some() {
                return message;
            }
        }
    }

    fn mode(&mut self, session: &str, mode: &str) {
        self.request(
            "session/set_config_option",
            json!({"sessionId": session, "configId": "mode", "value": mode}),
            &|_| None,
        )
        .expect("the mode is set");
    }
}

/// The address and bearer the tool server gave a session.
fn bearer(convo: &mut Convo<'_>, session: &str) -> (String, String) {
    let said = convo.prompt(session, "Bearer");
    let (url, token) = said
        .split_once(' ')
        .unwrap_or_else(|| panic!("a bearer: {said}"));
    (url.to_owned(), token.to_owned())
}

/// One HTTP request to the tool server: the status and the body.
fn post(url: &str, headers: &[(&str, &str)], body: &str) -> (u16, String) {
    let rest = url.strip_prefix("http://").expect("an http address");
    let (authority, path) = rest.split_once('/').expect("a path");
    let mut sent = format!("POST /{path} HTTP/1.1\r\n");
    if !headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("host"))
    {
        sent.push_str(&format!("Host: {authority}\r\n"));
    }
    for (name, value) in headers {
        sent.push_str(&format!("{name}: {value}\r\n"));
    }
    sent.push_str(&format!(
        "Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    ));
    let mut stream = TcpStream::connect(authority).expect("the tool server listens");
    stream.write_all(sent.as_bytes()).expect("sent");
    let mut answer = String::new();
    let _ = stream.read_to_string(&mut answer);
    let status = answer
        .split(' ')
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    let body = answer.split_once("\r\n\r\n").map_or("", |(_, body)| body);
    (status, body.to_owned())
}

const LIST: &str = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;

fn bearer_header(token: &str) -> String {
    format!("Bearer {token}")
}

fn until(what: &str, mut done: impl FnMut() -> bool) {
    let limit = Instant::now() + WAIT;
    while !done() {
        assert!(Instant::now() < limit, "{what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The process ids a marker file lists.
fn pids(marker: &Path) -> Vec<u32> {
    std::fs::read_to_string(marker)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.trim().parse().ok())
        .collect()
}

fn gone(pid: u32) {
    until("the process ended", || !running(pid));
}

/// A directory link: a symbolic link, or a junction where Windows allows no other.
fn link(from: &Path, to: &Path) {
    if cfg!(windows) {
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(from)
            .arg(to)
            .output()
            .expect("cmd runs");
        assert!(made.status.success(), "{made:?}");
    } else {
        #[cfg(unix)]
        std::os::unix::fs::symlink(to, from).expect("a link");
    }
}

#[test]
fn a_parent_hands_a_task_to_a_second_source_and_reads_the_childs_reply_as_the_result() {
    let dir = Scratch::new();
    let work = directory(&dir.0, "work");
    let config = format!(
        "{}{}",
        agent("parent", "default,manual,bypass", ""),
        agent("child", "default,manual,bypass", "")
    );
    let host = Hosted::start(&config);
    let link = host.local.open("parent").expect("a link");
    let mut convo = Convo::new(&link);
    let session = convo.open(true, &work);
    let task = convo.delegate(&session, "child", "Verbatim hello  world");
    assert!(task.starts_with("task-"), "{task}");
    let status = convo.status(&session, &task, 20_000);
    assert_eq!(status["state"], "completed", "{status}");
    assert_eq!(status["source"], "child");
    // The child's prompt is the task's text and nothing else: one block, with every space as it was written.
    assert_eq!(status["result"], "blocks=1 text=Verbatim hello  world");
    drop(link);
    host.stop();
}

#[test]
fn no_opt_in_or_a_host_that_turns_it_off_is_given_no_tool() {
    let dir = Scratch::new();
    let work = directory(&dir.0, "work");
    let both = format!(
        "{}{}",
        agent("parent", "default", ""),
        agent("child", "default", "")
    );
    let host = Hosted::start(&both);
    let link = host.local.open("parent").expect("a link");
    let mut convo = Convo::new(&link);
    let session = convo.open(false, &work);
    assert_eq!(convo.prompt(&session, "Bearer"), "no bearer");
    assert_eq!(
        convo.prompt(&session, "Delegate child hi"),
        "this session has no delegation tools"
    );
    drop(link);
    host.stop();

    let off = format!("{both}[host.delegate]\nenabled = false\n");
    let host = Hosted::start(&off);
    let link = host.local.open("parent").expect("a link");
    let mut convo = Convo::new(&link);
    let session = convo.open(true, &work);
    assert_eq!(convo.prompt(&session, "Bearer"), "no bearer");
    drop(link);
    host.stop();
}

#[test]
fn a_request_without_this_sessions_bearer_is_refused_and_so_is_one_that_comes_from_a_page() {
    let dir = Scratch::new();
    let work = directory(&dir.0, "work");
    let config = format!(
        "{}{}",
        agent("parent", "default", ""),
        agent("child", "default", "")
    );
    let host = Hosted::start(&config);
    let first = host.local.open("parent").expect("a link");
    let mut one = Convo::new(&first);
    let one_session = one.open(true, &work);
    let second = host.local.open("parent").expect("a link");
    let mut two = Convo::new(&second);
    let two_session = two.open(true, &work);
    let (url, mine) = bearer(&mut one, &one_session);
    let (_, theirs) = bearer(&mut two, &two_session);
    assert_ne!(mine, theirs, "each session has a bearer of its own");
    assert_eq!(mine.len(), 64);

    assert_eq!(post(&url, &[], LIST).0, 401, "no bearer");
    assert_eq!(
        post(
            &url,
            &[("Authorization", &bearer_header("0".repeat(64).as_str()))],
            LIST
        )
        .0,
        401,
        "a bearer nobody holds"
    );
    assert_eq!(
        post(&url, &[("Authorization", &format!("Basic {mine}"))], LIST).0,
        401,
        "another scheme"
    );
    let auth = bearer_header(&mine);
    let with = |extra: &[(&str, &str)]| {
        let mut headers: Vec<(&str, &str)> = vec![("Authorization", auth.as_str())];
        headers.extend_from_slice(extra);
        post(&url, &headers, LIST)
    };
    let (status, body) = with(&[]);
    assert_eq!(status, 200, "{body}");
    let listed: Value = serde_json::from_str(&body).expect("JSON");
    let names: Vec<&str> = listed["result"]["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert_eq!(
        names,
        ["delegate_targets", "delegate_task", "task_status", "cancel"]
    );
    assert!(
        listed["result"]["tools"][1]["inputSchema"]["properties"]["source"]["description"]
            .as_str()
            .is_some_and(|text| text.contains("child") && text.contains("parent"))
    );
    assert_eq!(
        with(&[("Origin", "http://example.com")]).0,
        403,
        "an Origin"
    );
    assert_eq!(with(&[("Sec-Fetch-Site", "cross-site")]).0, 403, "a page");
    assert_eq!(
        with(&[("Host", "example.com")]).0,
        403,
        "a Host that is not this socket"
    );
    let bad_path = url.replace("/mcp", "/other");
    assert_eq!(
        post(&bad_path, &[("Authorization", auth.as_str())], LIST).0,
        404
    );

    // A task of another session is not found by this one.
    let task = two.delegate(&two_session, "child", "Say there");
    let found = one.prompt(&one_session, &format!("Status {task} 0"));
    assert!(found.starts_with("error:"), "{found}");
    assert!(!found.contains("there"), "{found}");

    // Ending the connection revokes the bearer.
    drop(first);
    until("the bearer was revoked", || {
        post(&url, &[("Authorization", auth.as_str())], LIST).0 == 401
    });
    assert_eq!(
        post(&url, &[("Authorization", &bearer_header(&theirs))], LIST).0,
        200,
        "another session's bearer is still good"
    );
    drop(second);
    host.stop();
    // Stopping the host closes the server.
    assert!(
        TcpStream::connect(
            url.trim_start_matches("http://")
                .split('/')
                .next()
                .expect("an address")
        )
        .is_err()
    );
}

#[test]
fn a_task_names_only_a_source_the_session_may_use_and_a_folder_inside_its_own() {
    let dir = Scratch::new();
    let work = directory(&dir.0, "work");
    directory(&work, "inside");
    let outside = directory(&dir.0, "outside");
    link(&work.join("door"), &outside);
    let marker = dir.0.join("child.pids");
    let config = format!(
        "{}{}{}[host.delegate]\nsources = [\"child\"]\n",
        agent("parent", "default", ""),
        marked("child", "default", &marker),
        agent("other", "default", "")
    );
    let host = Hosted::start(&config);
    let link = host.local.open("parent").expect("a link");
    let mut convo = Convo::new(&link);
    let session = convo.open(true, &work);

    // A source `[host.delegate] sources` leaves out.
    let said = convo.prompt(&session, "Delegate other hi");
    assert!(
        said.starts_with("error:") && said.contains("child"),
        "{said}"
    );
    let said = convo.prompt(&session, "Delegate nobody hi");
    assert!(said.starts_with("error:"), "{said}");
    // A folder outside, by `..` and by a link, and one that is not there.
    for folder in ["..", "../outside", "inside/../..", "door", "missing"] {
        let said = convo.prompt(&session, &format!("DelegateIn {folder} child hi"));
        assert!(said.starts_with("error:"), "{folder}: {said}");
    }
    let absolute = outside.to_string_lossy().into_owned();
    let said = convo.prompt(&session, &format!("DelegateIn {absolute} child hi"));
    assert!(said.starts_with("error:"), "{said}");
    assert!(pids(&marker).is_empty(), "a refused task started nothing");

    // A folder inside is the child's working directory, settled.
    let task = {
        let said = convo.prompt(&session, "DelegateIn inside/. child Cwd");
        let answer: Value =
            serde_json::from_str(&said).unwrap_or_else(|_| panic!("a task: {said}"));
        answer["taskId"].as_str().expect("an id").to_owned()
    };
    let status = convo.status(&session, &task, 20_000);
    let cwd = status["result"]
        .as_str()
        .expect("a result")
        .trim_start_matches("cwd: ");
    assert!(
        Path::new(cwd).ends_with("work/inside") || Path::new(cwd).ends_with("work\\inside"),
        "{cwd}"
    );
    assert_eq!(pids(&marker).len(), 1);
    drop(link);
    host.stop();
}

#[test]
fn a_key_delegates_only_to_entries_that_list_it_and_inside_its_working_roots() {
    let dir = Scratch::new();
    let roots = directory(&dir.0, "roots");
    let work = directory(&roots, "work");
    let elsewhere = directory(&dir.0, "elsewhere");
    link(&roots.join("door"), &elsewhere);
    let me = support::key();
    let someone = support::key();
    let marker = dir.0.join("secret.pids");
    let config = format!(
        "{}{}{}",
        agent("parent", "default", "").replace(
            "listen = \"127.0.0.1:0\"\n",
            &format!(
                "listen = \"127.0.0.1:0\"\nworking_roots = [{}]\n",
                quoted(&roots.to_string_lossy())
            )
        ),
        agent("open", "default", ""),
        marked("secret", "default", &marker).replace(
            "listen = \"127.0.0.1:0\"\n",
            &format!(
                "listen = \"127.0.0.1:0\"\npeers = [\"{}\"]\n",
                someone.peer_id()
            )
        ),
    );
    let (host, log) = Host::start_logged(Scratch::new(), &config, 3, &[]);
    host.pair(&me);
    let opted = |rpc: &support::Rpc<'_, Socket>| {
        rpc.request(
            "initialize",
            json!({"protocolVersion": 1, "clientCapabilities": {}, "_meta": {META: true}}),
        )
        .expect("initialize");
    };
    let session_in = |rpc: &support::Rpc<'_, Socket>, cwd: &Path| {
        rpc.request(
            "session/new",
            json!({"cwd": cwd.to_string_lossy(), "mcpServers": []}),
        )
        .expect("a session")["sessionId"]
            .as_str()
            .expect("an id")
            .to_owned()
    };
    let (token, said) = support::with_link(host.url("parent"), &me, host.peer, |rpc| {
        opted(rpc);
        let session = session_in(rpc, &work);
        let token = say(rpc, &session, "Bearer")
            .split_once(' ')
            .expect("a bearer")
            .1
            .to_owned();
        // An entry that lists other keys is not this key's to use, and the list does not name it.
        let refused = say(rpc, &session, "Delegate secret hi");
        // One it may use is.
        let task: Value =
            serde_json::from_str(&say(rpc, &session, "Delegate open Say fine")).expect("a task");
        let id = task["taskId"].as_str().expect("an id");
        let done: Value = serde_json::from_str(&say(rpc, &session, &format!("Status {id} 20000")))
            .expect("a status");
        // The session's folder is inside the roots, a link in it that leads out is not.
        let behind = session_in(rpc, &roots.join("door"));
        let outside = say(rpc, &behind, "Delegate open hi");
        (token, (refused, done, outside))
    })
    .expect("a link");
    let (refused, done, outside) = said;
    assert!(refused.starts_with("error:"), "{refused}");
    assert!(
        refused.contains("parent, open") && !refused.contains("parent, open, secret"),
        "{refused}"
    );
    assert_eq!(done["state"], "completed", "{done}");
    assert_eq!(done["result"], "fine");
    assert!(
        outside.starts_with("error:") && outside.contains("working roots"),
        "{outside}"
    );
    assert!(
        pids(&marker).is_empty(),
        "the entry that does not list the key never started"
    );

    // Nothing the host logged holds a bearer.
    std::thread::sleep(Duration::from_millis(500));
    let written = support::log_holding(&log, &[]);
    assert!(!written.is_empty(), "the host logged nothing at debug");
    assert!(!written.contains(&token), "the bearer is in the host's log");
}

#[test]
fn a_task_runs_in_the_parents_mode_and_never_a_wider_one() {
    let dir = Scratch::new();
    let work = directory(&dir.0, "work");
    let config = format!(
        "{}{}{}{}",
        agent("parent", "default,manual,bypass", ""),
        agent("full", "default,manual,bypass", ""),
        agent("narrow", "default,acceptEdits", ""),
        agent("none", "acceptEdits,bypass", "")
    );
    let host = Hosted::start(&config);
    let link = host.local.open("parent").expect("a link");
    let mut convo = Convo::new(&link);
    let session = convo.open(true, &work);
    let run = |convo: &mut Convo<'_>, source: &str, task: &str| {
        let id = convo.delegate(&session, source, task);
        convo.status(&session, &id, 20_000)
    };

    // The parent asks for everything (default): so does the child.
    assert_eq!(run(&mut convo, "full", "Mode")["result"], "mode: default");
    // The parent in bypass; an agent that lists it takes it, an agent without it is asked for everything instead.
    convo.mode(&session, "bypass");
    assert_eq!(run(&mut convo, "full", "Mode")["result"], "mode: bypass");
    assert_eq!(run(&mut convo, "narrow", "Mode")["result"], "mode: default");
    // The parent in manual: the child is in manual, and no argument raises it.
    convo.mode(&session, "manual");
    assert_eq!(run(&mut convo, "full", "Mode")["result"], "mode: manual");
    let said = convo.prompt(
        &session,
        r#"Call delegate_task {"source":"full","task":"Mode","mode":"bypass","permission_mode":"bypass"}"#,
    );
    let id = serde_json::from_str::<Value>(&said).expect("a task")["taskId"]
        .as_str()
        .expect("an id")
        .to_owned();
    assert_eq!(
        convo.status(&session, &id, 20_000)["result"],
        "mode: manual"
    );
    // An agent with neither the parent's mode nor one that asks is refused, and the task says why.
    let refused = run(&mut convo, "none", "Mode");
    assert_eq!(refused["state"], "rejected", "{refused}");
    assert!(
        refused["message"]
            .as_str()
            .is_some_and(|why| why.contains("mode")),
        "{refused}"
    );
    drop(link);
    host.stop();
}

#[test]
fn a_childs_permission_request_reaches_the_parents_person_with_its_task() {
    let dir = Scratch::new();
    let work = directory(&dir.0, "work");
    let marker = dir.0.join("child.pids");
    let config = format!(
        "{}{}",
        agent("parent", "default", ""),
        marked("child", "default", &marker)
    );
    let host = Hosted::start(&config);
    let link = host.local.open("parent").expect("a link");
    let mut convo = Convo::new(&link);
    let session = convo.open(true, &work);
    let task = convo.delegate(&session, "child", "Please edit src/report.js");
    let asked = convo.host_request();
    assert_eq!(asked["method"], "session/request_permission");
    assert_eq!(
        asked["params"]["sessionId"], session,
        "it is asked on the parent's session"
    );
    let call = asked["params"]["toolCall"]["toolCallId"]
        .as_str()
        .expect("a call id");
    assert!(call.starts_with(&format!("{task}:")), "{call}");
    let meta = &asked["params"]["_meta"][META];
    assert_eq!(meta["taskId"], task.as_str());
    assert_eq!(meta["source"], "child");
    assert!(
        meta["title"]
            .as_str()
            .is_some_and(|title| title.contains("edit src"))
    );
    assert!(
        asked["params"]["options"]
            .as_array()
            .is_some_and(|options| options.len() == 4)
    );
    // Until the person answers, the task is waiting on them.
    assert_eq!(convo.status(&session, &task, 0)["state"], "input_required");
    convo.send(&json!({"jsonrpc": "2.0", "id": asked["id"], "result": {"outcome": {"outcome": "selected", "optionId": "yes"}}}));
    let done = convo.status(&session, &task, 20_000);
    assert_eq!(done["state"], "completed", "{done}");
    assert_eq!(
        done["result"], "Renamed everywhere.",
        "an allow reached the child"
    );

    // A refusal reaches it too, and a connection that closes with the question open ends the task's process.
    let refused = convo.delegate(&session, "child", "Please edit again");
    let asked = convo.host_request();
    convo.send(&json!({"jsonrpc": "2.0", "id": asked["id"], "result": {"outcome": {"outcome": "selected", "optionId": "no"}}}));
    let said = convo.status(&session, &refused, 20_000);
    assert_eq!(said["state"], "completed", "{said}");
    let _ = convo.delegate(&session, "child", "Please edit once more");
    let _ = convo.host_request();
    let pid = *pids(&marker).last().expect("the child started");
    assert!(running(pid));
    drop(link);
    gone(pid);
    host.stop();
}

/// The states a task's events named, in order, without the `TASK_STATE_` prefix.
fn states(events: &[Value], task: &str) -> Vec<String> {
    events
        .iter()
        .filter(|event| event["taskId"] == task)
        .filter_map(|event| {
            let event = event.get("event")?;
            let status = event
                .pointer("/task/status/state")
                .or_else(|| event.pointer("/statusUpdate/status/state"))?;
            Some(
                status
                    .as_str()?
                    .trim_start_matches("TASK_STATE_")
                    .to_owned(),
            )
        })
        .collect()
}

#[test]
fn a_client_that_opted_in_is_told_each_state_of_a_task_and_what_its_agent_said_and_can_ask_and_stop()
 {
    let dir = Scratch::new();
    let work = directory(&dir.0, "work");
    let marker = dir.0.join("child.pids");
    let config = format!(
        "{}{}",
        agent("parent", "default", ""),
        marked("child", "default", &marker)
    );
    let host = Hosted::start(&config);
    let link = host.local.open("parent").expect("a link");
    let mut convo = Convo::new(&link);
    let session = convo.open(true, &work);

    let task = convo.delegate(&session, "child", "Verbatim hello");
    assert_eq!(convo.status(&session, &task, 20_000)["state"], "completed");
    until("the last event came", || {
        convo.status(&session, &task, 0);
        states(&convo.events, &task).last().map(String::as_str) == Some("COMPLETED")
    });
    let seen = states(&convo.events, &task);
    assert_eq!(
        seen.first().map(String::as_str),
        Some("SUBMITTED"),
        "{seen:?}"
    );
    assert!(seen.contains(&"WORKING".to_owned()), "{seen:?}");
    let first = convo
        .events
        .iter()
        .find(|event| event["taskId"] == task.as_str())
        .unwrap();
    assert_eq!(first["sessionId"], session);
    assert_eq!(first["event"]["task"]["metadata"]["source"], "child");
    assert!(
        convo
            .events
            .iter()
            .any(|event| event["update"]["sessionUpdate"] == "agent_message_chunk"),
        "the child's own updates reach the client"
    );

    // What the host kept of the task is asked for by its id, ended or not; another id is refused.
    let kept = convo
        .request(
            "_canoa/delegate/history",
            json!({"sessionId": session, "taskId": task}),
            &|_| None,
        )
        .expect("a history");
    assert!(
        kept["updates"]
            .as_array()
            .is_some_and(|updates| !updates.is_empty()),
        "{kept}"
    );
    let refused = convo
        .request(
            "_canoa/delegate/history",
            json!({"sessionId": session, "taskId": "task-nope"}),
            &|_| None,
        )
        .expect_err("no such task");
    assert_eq!(refused["code"], -32602);

    // Stop ends a task's process and its last event says so.
    let waiting = convo.delegate(&session, "child", "Wait");
    until("the child started", || pids(&marker).len() == 2);
    let pid = pids(&marker)[1];
    convo
        .request(
            "_canoa/delegate/stop",
            json!({"sessionId": session}),
            &|_| None,
        )
        .expect("stopped");
    gone(pid);
    until("the canceled event came", || {
        convo.status(&session, &waiting, 0);
        states(&convo.events, &waiting).last().map(String::as_str) == Some("CANCELED")
    });
    drop(link);
    host.stop();
}

/// The lines of the host's usage log, in the order they were written.
fn usage_rows(home: &Path) -> Vec<Value> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(home.join("usage"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
        .collect();
    files.sort();
    files
        .iter()
        .flat_map(|file| {
            std::fs::read_to_string(file)
                .unwrap_or_default()
                .lines()
                .filter_map(|line| serde_json::from_str(line).ok())
                .collect::<Vec<Value>>()
        })
        .collect()
}

fn usage_of(events: &[Value], task: &str) -> Option<Value> {
    events
        .iter()
        .filter(|event| event["taskId"] == task)
        .find_map(|event| event.pointer("/event/task/metadata/usage").cloned())
}

#[test]
fn each_child_is_recorded_under_its_parent_once_and_its_figures_reach_the_client() {
    let dir = Scratch::new();
    let work = directory(&dir.0, "work");
    let config = format!(
        "{}{}{}",
        agent("parent", "default", ", \"--usage\""),
        agent("first", "default", ", \"--usage\""),
        agent("second", "default", ", \"--usage\"")
    );
    let host = Hosted::start(&config);
    let link = host.local.open("parent").expect("a link");
    let mut convo = Convo::new(&link);
    let session = convo.open(true, &work);

    convo.prompt(&session, "spend 1000 100");
    let a = convo.delegate(&session, "first", "spend 120 30");
    let b = convo.delegate(&session, "second", "spend 50 10 7");
    for task in [&a, &b] {
        assert_eq!(convo.status(&session, task, 20_000)["state"], "completed");
    }
    until("both children's figures arrived", || {
        convo.status(&session, &a, 0);
        usage_of(&convo.events, &a).is_some() && usage_of(&convo.events, &b).is_some()
    });
    let first = usage_of(&convo.events, &a).expect("figures");
    assert_eq!(first, json!({"input": 120, "output": 30}), "{first}");
    // A counter the agent never moved is absent, never 0.
    let second = usage_of(&convo.events, &b).expect("figures");
    assert_eq!(
        second,
        json!({"input": 50, "output": 10, "cacheRead": 7}),
        "{second}"
    );

    let rows = usage_rows(&host.scratch.0.join("home"));
    let of = |account: &str| -> Vec<&Value> {
        rows.iter()
            .filter(|row| row["account"] == account)
            .collect()
    };
    let [own] = of("parent")[..] else {
        panic!("one line for the parent: {rows:?}");
    };
    assert!(own.get("parent_session_id").is_none(), "{own}");
    assert_eq!(own["session_id"], session.as_str());
    for (account, input) in [("first", 120), ("second", 50)] {
        let [row] = of(account)[..] else {
            panic!("one line for {account}: {rows:?}");
        };
        assert_eq!(row["parent_session_id"], session.as_str(), "{row}");
        assert_eq!(row["input"], input, "{row}");
    }
    // Every figure is in one line: the whole is the parent's and its children's, each once.
    let total: u64 = rows.iter().filter_map(|row| row["input"].as_u64()).sum();
    assert_eq!(total, 1170);
    drop(link);
    host.stop();
}

#[test]
fn history_and_stop_reach_only_the_tasks_of_the_session_and_connection_that_ask() {
    let dir = Scratch::new();
    let work = directory(&dir.0, "work");
    let marker = dir.0.join("child.pids");
    let config = format!(
        "{}{}",
        agent("parent", "default", ""),
        marked("child", "default", &marker)
    );
    let host = Hosted::start(&config);
    let (first, second) = (
        host.local.open("parent").expect("a link"),
        host.local.open("parent").expect("another link"),
    );
    let (mut one, mut two) = (Convo::new(&first), Convo::new(&second));
    let a = one.open(true, &work);
    let a2 = one
        .request(
            "session/new",
            json!({"cwd": work.to_string_lossy(), "mcpServers": []}),
            &|_| None,
        )
        .expect("a second session")["sessionId"]
        .as_str()
        .expect("an id")
        .to_owned();
    let b = two.open(true, &work);
    let mut started = 0;
    let mut wait = |convo: &mut Convo<'_>, session: &str| {
        let task = convo.delegate(session, "child", "Wait");
        started += 1;
        until("the child started", || pids(&marker).len() == started);
        (task, pids(&marker)[started - 1])
    };
    let (a_task, a_pid) = wait(&mut one, &a);
    let (a_other, a_other_pid) = wait(&mut one, &a);
    let (a2_task, a2_pid) = wait(&mut one, &a2);
    let (_, b_pid) = wait(&mut two, &b);

    // The stand-in numbers its sessions per process, so `a` is also the second connection's own id; `a2` is only the first's.
    // Another connection's session id is no session here; another session's task id is no task.
    for method in ["_canoa/delegate/history", "_canoa/delegate/stop"] {
        let foreign = two
            .request(method, json!({"sessionId": a2, "taskId": a2_task}), &|_| {
                None
            })
            .expect_err("another connection's session");
        assert_eq!(foreign["code"], -32602, "{method}");
        let crossed = one
            .request(method, json!({"sessionId": a, "taskId": a2_task}), &|_| {
                None
            })
            .expect_err("another session's task");
        assert_eq!(crossed["code"], -32602, "{method}");
    }
    let bare = two
        .request("_canoa/delegate/stop", json!({"sessionId": a2}), &|_| None)
        .expect_err("another connection's session, with no task named");
    assert_eq!(bare["code"], -32602);
    // A task id that is not text is refused, not read as "every task".
    for id in [json!(5), json!(["x"]), json!({})] {
        let refused = one
            .request(
                "_canoa/delegate/stop",
                json!({"sessionId": a, "taskId": id}),
                &|_| None,
            )
            .expect_err("a task id that is not text");
        assert_eq!(refused["code"], -32602, "{id}");
    }
    for pid in [a_pid, a_other_pid, a2_pid, b_pid] {
        assert!(running(pid), "nothing was stopped by a refused request");
    }

    // One task's stop ends that task only.
    one.request(
        "_canoa/delegate/stop",
        json!({"sessionId": a, "taskId": a_task}),
        &|_| None,
    )
    .expect("stopped");
    gone(a_pid);
    for pid in [a_other_pid, a2_pid, b_pid] {
        assert!(running(pid));
    }
    assert_eq!(one.status(&a, &a_task, 20_000)["state"], "canceled");
    assert_eq!(one.status(&a, &a_other, 0)["state"], "working");
    drop((first, second));
    host.stop();
}

#[test]
fn the_parents_cancel_and_the_cancel_tool_end_the_childs_process_and_a_closed_connection_ends_all()
{
    let dir = Scratch::new();
    let work = directory(&dir.0, "work");
    let marker = dir.0.join("child.pids");
    let config = format!(
        "{}{}",
        agent("parent", "default", ""),
        marked("child", "default", &marker)
    );
    let host = Hosted::start(&config);
    let link = host.local.open("parent").expect("a link");
    let mut convo = Convo::new(&link);
    let session = convo.open(true, &work);

    // The parent's own cancel.
    let first = convo.delegate(&session, "child", "Wait");
    until("the child started", || pids(&marker).len() == 1);
    let pid = pids(&marker)[0];
    assert_eq!(convo.status(&session, &first, 0)["state"], "working");
    convo.send(
        &json!({"jsonrpc": "2.0", "method": "session/cancel", "params": {"sessionId": session}}),
    );
    gone(pid);
    let ended = convo.status(&session, &first, 20_000);
    assert_eq!(ended["state"], "canceled", "{ended}");

    // The tool.
    let second = convo.delegate(&session, "child", "Wait");
    until("the second child started", || pids(&marker).len() == 2);
    let pid = pids(&marker)[1];
    let said = convo.prompt(&session, &format!("Cancel {second}"));
    assert!(said.contains("canceled"), "{said}");
    gone(pid);

    // A closed connection ends every task.
    let _ = convo.delegate(&session, "child", "Wait");
    let _ = convo.delegate(&session, "child", "Wait");
    until("both started", || pids(&marker).len() == 4);
    let all: Vec<u32> = pids(&marker)[2..].to_vec();
    drop(link);
    for pid in all {
        gone(pid);
    }
    host.stop();
}

#[test]
fn a_limit_refuses_the_task_it_would_pass_and_touches_no_other() {
    let dir = Scratch::new();
    let work = directory(&dir.0, "work");
    let config = format!(
        "{}{}[host.delegate]\nmax_children = 1\nmax_running = 2\ntask_bytes = 40\nresult_bytes = 5\n",
        agent("parent", "default", ""),
        agent("child", "default", "")
    );
    let host = Hosted::start(&config);
    let first = host.local.open("parent").expect("a link");
    let mut one = Convo::new(&first);
    let one_session = one.open(true, &work);
    let kept = one.delegate(&one_session, "child", "Wait");
    // One session's own limit.
    let said = one.prompt(&one_session, "Delegate child second");
    assert!(
        said.starts_with("error:") && said.contains("max_children"),
        "{said}"
    );
    // The first task is as it was.
    assert_eq!(one.status(&one_session, &kept, 0)["state"], "working");
    // A task text past task_bytes.
    let said = one.prompt(&one_session, &format!("Delegate child {}", "x".repeat(60)));
    assert!(
        said.starts_with("error:") && said.contains("task_bytes"),
        "{said}"
    );

    // The host's own limit, across sessions.
    let second = host.local.open("parent").expect("a link");
    let mut two = Convo::new(&second);
    let two_session = two.open(true, &work);
    let other = two.delegate(&two_session, "child", "Wait");
    let third = host.local.open("parent").expect("a link");
    let mut three = Convo::new(&third);
    let three_session = three.open(true, &work);
    let said = three.prompt(&three_session, "Delegate child Say hi");
    assert!(
        said.starts_with("error:") && said.contains("max_running"),
        "{said}"
    );
    // Ending a task gives its place back.
    let said = two.prompt(&two_session, &format!("Cancel {other}"));
    assert!(said.contains("canceled"), "{said}");
    let mut hello = None;
    until("the place came back", || {
        let said = three.prompt(&three_session, "Delegate child Say hello");
        hello = serde_json::from_str::<Value>(&said)
            .ok()
            .and_then(|task| task["taskId"].as_str().map(str::to_owned));
        hello.is_some()
    });
    assert_eq!(
        three.status(&three_session, &hello.expect("a task"), 20_000)["state"],
        "completed"
    );
    // A reply past result_bytes is cut and says so.
    let id = {
        let said = three.prompt(&three_session, "Delegate child Say 0123456789");
        serde_json::from_str::<Value>(&said).expect("a task")["taskId"]
            .as_str()
            .expect("an id")
            .to_owned()
    };
    let cut = three.status(&three_session, &id, 20_000);
    assert_eq!(cut["result"], "01234", "{cut}");
    assert_eq!(cut["truncated"], true);
    assert_eq!(one.status(&one_session, &kept, 0)["state"], "working");
    drop((first, second, third));
    host.stop();
}

#[test]
fn a_task_may_delegate_further_only_as_deep_as_max_depth_allows() {
    let dir = Scratch::new();
    let work = directory(&dir.0, "work");
    let agents = format!(
        "{}{}",
        agent("parent", "default", ""),
        agent("child", "default", "")
    );
    // The default depth is 1: the task has no tool.
    let host = Hosted::start(&agents);
    let link = host.local.open("parent").expect("a link");
    let mut convo = Convo::new(&link);
    let session = convo.open(true, &work);
    let task = convo.delegate(&session, "child", "Delegate child Say deeper");
    let status = convo.status(&session, &task, 20_000);
    assert_eq!(
        status["result"], "this session has no delegation tools",
        "{status}"
    );
    drop(link);
    host.stop();

    // At 2 it has, and its own task has none.
    let host = Hosted::start(&format!("{agents}[host.delegate]\nmax_depth = 2\n"));
    let link = host.local.open("parent").expect("a link");
    let mut convo = Convo::new(&link);
    let session = convo.open(true, &work);
    let task = convo.delegate(
        &session,
        "child",
        "Delegate child Delegate child Say bottom",
    );
    let status = convo.status(&session, &task, 20_000);
    assert_eq!(status["state"], "completed", "{status}");
    let inner: Value = serde_json::from_str(status["result"].as_str().expect("a result"))
        .expect("a task of the task");
    assert!(inner["taskId"].is_string(), "{status}");
    drop(link);
    host.stop();
}

#[test]
fn the_bearer_is_in_no_log_report_or_error() {
    let dir = Scratch::new();
    let work = directory(&dir.0, "work");
    let config = format!(
        "{}{}",
        agent("parent", "default", ""),
        agent("child", "default", "")
    );
    let (host, log) = Host::start_logged(Scratch::new(), &config, 2, &[]);
    let me = support::key();
    host.pair(&me);
    let (token, seen) = support::with_link(host.url("parent"), &me, host.peer, |rpc| {
        rpc.request(
            "initialize",
            json!({"protocolVersion": 1, "clientCapabilities": {}, "_meta": {META: true}}),
        )
        .expect("initialize");
        let session = rpc
            .request("session/new", json!({"cwd": work.to_string_lossy(), "mcpServers": []}))
            .expect("a session")["sessionId"]
            .as_str()
            .expect("an id")
            .to_owned();
        let said = say(rpc, &session, "Bearer");
        let (url, token) = said.split_once(' ').expect("a bearer");
        let mut seen = String::new();
        // What the tool server answers a request that fails, and one that does not.
        for headers in [
            vec![("Authorization", bearer_header(token))],
            vec![("Authorization", bearer_header(token)), ("Origin", "http://example.com".to_owned())],
            vec![("Authorization", bearer_header(&format!("{token}x")))],
        ] {
            let pairs: Vec<(&str, &str)> = headers.iter().map(|(n, v)| (*n, v.as_str())).collect();
            let (status, body) = post(url, &pairs, r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"task_status","arguments":{"taskId":"nope"}}}"#);
            seen.push_str(&format!("{status} {body}\n"));
        }
        seen.push_str(&say(rpc, &session, "Call task_status {\"taskId\":5}"));
        seen.push_str(&say(rpc, &session, "Call nothing {}"));
        (token.to_owned(), seen)
    })
    .expect("a link");
    assert!(!seen.contains(&token), "an answer holds the bearer: {seen}");
    let written = support::log_holding(&log, &[]);
    assert!(!written.contains(&token), "the host's log holds the bearer");
}

/// One turn of a session over the network, and what the agent said.
fn say(rpc: &support::Rpc<'_, Socket>, session: &str, text: &str) -> String {
    rpc.prompt(session, text).expect("a turn")
}

#[test]
fn a_mode_id_is_kept_only_for_the_same_kind_of_agent() {
    let dir = Scratch::new();
    let work = directory(&dir.0, "work");
    // The same scripted program under another name stands for another provider: its `bypass` is not the parent's.
    let other = dir
        .0
        .join(format!("other_agent{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(example("stdio_agent", false), &other).expect("copied");
    let foreign = |name: &str, modes: &str| {
        agent(name, modes, "").replace(
            &quoted(&example("stdio_agent", false).to_string_lossy()),
            &quoted(&other.to_string_lossy()),
        )
    };
    let config = format!(
        "{}{}{}{}",
        agent("parent", "default,manual,bypass", ""),
        agent("same", "default,manual,bypass", ""),
        foreign("foreign", "default,manual,bypass"),
        foreign("foreign_none", "acceptEdits,bypass")
    );
    let host = Hosted::start(&config);
    let link = host.local.open("parent").expect("a link");
    let mut convo = Convo::new(&link);
    let session = convo.open(true, &work);
    let run = |convo: &mut Convo<'_>, source: &str| {
        let id = convo.delegate(&session, source, "Mode");
        convo.status(&session, &id, 20_000)
    };
    convo.mode(&session, "bypass");
    assert_eq!(run(&mut convo, "same")["result"], "mode: bypass");
    assert_eq!(
        run(&mut convo, "foreign")["result"],
        "mode: manual",
        "never the parent's id in another agent"
    );
    let none = run(&mut convo, "foreign_none");
    assert_eq!(none["state"], "rejected", "{none}");
    drop(link);
    host.stop();
}

/// Sockets that connect and say nothing.
fn idle(url: &str, count: usize) -> Vec<TcpStream> {
    let authority = url
        .trim_start_matches("http://")
        .split('/')
        .next()
        .expect("an address");
    (0..count)
        .map(|_| TcpStream::connect(authority).expect("connects"))
        .collect()
}

#[test]
fn connections_that_never_send_a_request_do_not_hold_the_server() {
    let dir = Scratch::new();
    let work = directory(&dir.0, "work");
    let host = Hosted::start(&agent("parent", "default", ""));
    let link = host.local.open("parent").expect("a link");
    let mut convo = Convo::new(&link);
    let session = convo.open(true, &work);
    let (url, token) = bearer(&mut convo, &session);
    let held = idle(&url, 40);
    let started = Instant::now();
    let auth = bearer_header(&token);
    loop {
        let answered =
            std::panic::catch_unwind(|| post(&url, &[("Authorization", auth.as_str())], LIST).0)
                .unwrap_or(0);
        if answered == 200 {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(8),
            "the silent connections still hold the server after {:?}",
            started.elapsed()
        );
        std::thread::sleep(Duration::from_millis(200));
    }
    drop(held);
    drop(link);
    host.stop();
}

#[test]
fn one_session_waiting_cannot_starve_another() {
    let dir = Scratch::new();
    let work = directory(&dir.0, "work");
    let host = Hosted::start(&format!(
        "{}{}",
        agent("parent", "default", ""),
        agent("child", "default", "")
    ));
    let first = host.local.open("parent").expect("a link");
    let mut one = Convo::new(&first);
    let one_session = one.open(true, &work);
    let second = host.local.open("parent").expect("a link");
    let mut two = Convo::new(&second);
    let two_session = two.open(true, &work);
    let (url, mine) = bearer(&mut one, &one_session);
    let (_, theirs) = bearer(&mut two, &two_session);
    let task = one.delegate(&one_session, "child", "Wait");
    let call = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"task_status","arguments":{{"taskId":"{task}","wait_ms":6000}}}}}}"#
    );
    let waiters: Vec<_> = (0..40)
        .map(|_| {
            let (url, call, auth) = (url.clone(), call.clone(), bearer_header(&mine));
            std::thread::spawn(move || post(&url, &[("Authorization", auth.as_str())], &call).0)
        })
        .collect();
    std::thread::sleep(Duration::from_millis(1000));
    // The other session is answered while the first one's waits are still running.
    let (status, _) = post(
        &url,
        &[("Authorization", bearer_header(&theirs).as_str())],
        LIST,
    );
    assert_eq!(status, 200, "another session was starved by waits");
    let statuses: Vec<u16> = waiters
        .into_iter()
        .map(|waiter| waiter.join().expect("no panic"))
        .collect();
    assert!(
        statuses.contains(&429),
        "one session may hold only so many calls: {statuses:?}"
    );
    assert!(statuses.contains(&200), "{statuses:?}");
    drop((first, second));
    host.stop();
}

/// A scripted agent entry that takes no HTTP MCP server, so the host gives it the stdio bridge.
fn plain(name: &str, modes: &str) -> String {
    agent(name, modes, "").replace(", \"--mcp-http\"", "")
}

/// `canoa mcp-bridge` started as an agent's harness would start it, with the environment it names.
struct Bridge {
    child: std::process::Child,
    input: Option<std::process::ChildStdin>,
    lines: std::sync::mpsc::Receiver<String>,
    errors: std::sync::mpsc::Receiver<String>,
}

impl Bridge {
    fn start(url: Option<&str>, token: Option<&str>) -> Self {
        let mut command = Command::new(host_binary());
        command
            .arg("mcp-bridge")
            .env_remove("CANOA_DELEGATE_URL")
            .env_remove("CANOA_DELEGATE_TOKEN")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(url) = url {
            command.env("CANOA_DELEGATE_URL", url);
        }
        if let Some(token) = token {
            command.env("CANOA_DELEGATE_TOKEN", token);
        }
        let mut child = command.spawn().expect("the bridge starts");
        let input = child.stdin.take();
        let forward = |stream: Box<dyn std::io::Read + Send>| {
            let (sender, receiver) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                use std::io::BufRead as _;
                for line in std::io::BufReader::new(stream).lines() {
                    let Ok(line) = line else { return };
                    if sender.send(line).is_err() {
                        return;
                    }
                }
            });
            receiver
        };
        let lines = forward(Box::new(child.stdout.take().expect("piped")));
        let errors = forward(Box::new(child.stderr.take().expect("piped")));
        Self {
            child,
            input,
            lines,
            errors,
        }
    }

    fn send(&mut self, line: &str) {
        let input = self.input.as_mut().expect("its input is open");
        writeln!(input, "{line}").expect("written");
        input.flush().expect("flushed");
    }

    /// One request line and the line that answers it.
    fn ask(&mut self, request: &str) -> Value {
        self.send(request);
        let line = self.lines.recv_timeout(WAIT).expect("an answer");
        serde_json::from_str(&line).expect("one JSON line")
    }

    /// How it ended, once it has.
    fn ended(&mut self) -> std::process::ExitStatus {
        let limit = Instant::now() + WAIT;
        loop {
            if let Some(status) = self.child.try_wait().expect("waited") {
                return status;
            }
            assert!(Instant::now() < limit, "the bridge did not end");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn said(&self) -> String {
        std::thread::sleep(Duration::from_millis(200));
        self.errors.try_iter().collect::<Vec<_>>().join("\n")
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The command line a running process was started with, as the operating system lists it to any user.
fn command_line(pid: u32) -> String {
    if cfg!(windows) {
        let listed = Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!("(Get-CimInstance Win32_Process -Filter 'ProcessId={pid}').CommandLine"),
            ])
            .output()
            .expect("powershell runs");
        String::from_utf8_lossy(&listed.stdout).trim().to_owned()
    } else {
        std::fs::read(format!("/proc/{pid}/cmdline"))
            .map(|bytes| {
                String::from_utf8_lossy(&bytes)
                    .replace('\0', " ")
                    .trim()
                    .to_owned()
            })
            .unwrap_or_default()
    }
}

#[test]
fn through_the_bridge_an_agent_without_http_hands_a_task_on_and_its_token_is_not_on_a_command_line()
{
    let dir = Scratch::new();
    let work = directory(&dir.0, "work");
    let config = format!(
        "{}{}",
        plain("parent", "default,manual"),
        plain("child", "default,manual")
    );
    let host = Hosted::start(&config);
    let link = host.local.open("parent").expect("a link");
    let mut convo = Convo::new(&link);
    let session = convo.open(true, &work);

    let said = convo.prompt(&session, "Bridge");
    let (_, token) = bearer(&mut convo, &session);
    let (pid, line) = said
        .strip_prefix("pid=")
        .and_then(|rest| rest.split_once(" command="))
        .unwrap_or_else(|| panic!("a bridge: {said}"));
    let pid: u32 = pid.parse().expect("a process id");
    assert!(line.ends_with("mcp-bridge"), "{line}");
    assert!(
        !line.contains(&token),
        "the entry's arguments name no token"
    );
    let listed = command_line(pid);
    assert!(
        listed.contains("mcp-bridge"),
        "the process is the bridge: {listed}"
    );
    assert!(
        !listed.contains(&token),
        "the bearer is on a command line: {listed}"
    );

    // A task is handed on and its reply read as the result, as over HTTP.
    let task = convo.delegate(&session, "child", "Verbatim hello  world");
    assert!(task.starts_with("task-"), "{task}");
    let status = convo.status(&session, &task, 20_000);
    assert_eq!(status["state"], "completed", "{status}");
    assert_eq!(status["source"], "child");
    assert_eq!(status["result"], "blocks=1 text=Verbatim hello  world");

    // The task's own session, at the depth limit, is given no tools.
    let nested = convo.delegate(&session, "child", "Bearer");
    let answered = convo.status(&session, &nested, 20_000);
    assert_eq!(answered["result"], "no bearer", "{answered}");

    // Closing its input ends the bridge.
    let ended = convo.prompt(&session, "BridgeClose");
    assert!(ended.starts_with("bridge ended: "), "{ended}");
    until("the bridge process is gone", || !running(pid));
    drop(link);
    host.stop();
}

#[test]
fn through_the_bridge_no_bearer_another_sessions_or_a_revoked_one_opens_nothing() {
    let dir = Scratch::new();
    let work = directory(&dir.0, "work");
    let config = format!(
        "{}{}",
        plain("parent", "default"),
        plain("child", "default")
    );
    let host = Hosted::start(&config);
    let first = host.local.open("parent").expect("a link");
    let mut one = Convo::new(&first);
    let one_session = one.open(true, &work);
    let second = host.local.open("parent").expect("a link");
    let mut two = Convo::new(&second);
    let two_session = two.open(true, &work);
    let (url, mine) = bearer(&mut one, &one_session);
    let (_, theirs) = bearer(&mut two, &two_session);
    assert_ne!(mine, theirs);

    // Another session's task is not found through this session's bridge.
    let task = two.delegate(&two_session, "child", "Say there");
    let found = one.prompt(&one_session, &format!("Status {task} 0"));
    assert!(found.starts_with("error:"), "{found}");
    assert!(!found.contains("there"), "{found}");

    // A bearer nobody holds, and the right one.
    let mut nobody = Bridge::start(Some(&url), Some(&"0".repeat(64)));
    let refused = nobody.ask(LIST);
    assert_eq!(refused["id"], 1);
    assert!(
        refused["error"]["message"]
            .as_str()
            .is_some_and(|text| text.contains("bearer")),
        "{refused}"
    );
    let mut held = Bridge::start(Some(&url), Some(&mine));
    let listed = held.ask(LIST);
    let names: Vec<&str> = listed["result"]["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert_eq!(
        names,
        ["delegate_targets", "delegate_task", "task_status", "cancel"]
    );

    // Ending the connection revokes the bearer, bridge or not.
    drop(first);
    until("the bearer was revoked", || {
        held.ask(LIST)["error"]["message"]
            .as_str()
            .is_some_and(|text| text.contains("bearer"))
    });
    let mut other = Bridge::start(Some(&url), Some(&theirs));
    assert!(
        other.ask(LIST).get("result").is_some(),
        "another session's bearer is still good"
    );
    drop((second, nobody, held, other));
    host.stop();
}

#[test]
fn the_bridge_refuses_a_missing_token_a_missing_address_and_one_that_is_not_loopback() {
    let token = "f".repeat(64);
    for (url, bearer, why) in [
        (Some("http://127.0.0.1:9/mcp"), None, "CANOA_DELEGATE_TOKEN"),
        (None, Some(token.as_str()), "CANOA_DELEGATE_URL"),
        (
            Some("http://10.20.30.40:9/mcp"),
            Some(token.as_str()),
            "loopback",
        ),
        (
            Some("http://example.com/mcp"),
            Some(token.as_str()),
            "loopback",
        ),
        (
            Some("https://127.0.0.1:9/mcp"),
            Some(token.as_str()),
            "http://",
        ),
    ] {
        let mut bridge = Bridge::start(url, bearer);
        let status = bridge.ended();
        assert_eq!(status.code(), Some(2), "{url:?} {why}");
        let said = bridge.said();
        assert!(said.contains(why), "{url:?}: {said}");
        assert!(!said.contains(&token));
        assert!(
            bridge.lines.try_recv().is_err(),
            "nothing is said on its output"
        );
    }
}

#[test]
fn the_bridge_ends_when_its_input_closes_and_keeps_nothing_between_requests() {
    let dir = Scratch::new();
    let work = directory(&dir.0, "work");
    let host = Hosted::start(&plain("parent", "default"));
    let link = host.local.open("parent").expect("a link");
    let mut convo = Convo::new(&link);
    let session = convo.open(true, &work);
    let (url, token) = bearer(&mut convo, &session);

    let mut bridge = Bridge::start(Some(&url), Some(&token));
    let initialized = bridge.ask(
        r#"{"jsonrpc":"2.0","id":"a","method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#,
    );
    assert_eq!(initialized["id"], "a");
    assert_eq!(
        initialized["result"]["serverInfo"]["name"],
        "canoa-delegate"
    );
    // A notification has no answer; the next request still gets its own.
    bridge.send(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
    assert_eq!(
        bridge.ask(r#"{"jsonrpc":"2.0","id":9,"method":"ping"}"#)["id"],
        9
    );
    // A line that is not JSON is answered, and the bridge goes on.
    bridge.send("not json");
    let line = bridge.lines.recv_timeout(WAIT).expect("an answer");
    assert!(line.contains("-32700"), "{line}");
    assert!(bridge.ask(LIST)["result"]["tools"].is_array());
    bridge.input = None;
    assert_eq!(bridge.ended().code(), Some(0));
    drop(link);
    host.stop();
}

/// A server that answers every request with `status` and `body`, and sends the head of each request it read down the channel.
fn answering(status: &str, body: &str) -> (String, std::sync::mpsc::Receiver<String>) {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("bound");
    let port = listener.local_addr().expect("an address").port();
    let (seen, receiver) = std::sync::mpsc::channel();
    let answer = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut head = Vec::new();
            let mut chunk = [0_u8; 1024];
            while !head.windows(4).any(|window| window == b"\r\n\r\n") {
                match stream.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => head.extend_from_slice(&chunk[..read]),
                }
            }
            let _ = seen.send(String::from_utf8_lossy(&head).into_owned());
            let _ = stream.write_all(answer.as_bytes());
        }
    });
    (format!("http://127.0.0.1:{port}/mcp"), receiver)
}

#[test]
fn the_bridge_relays_a_request_with_the_bearer_and_answers_what_the_server_cannot() {
    let token = "a1".repeat(32);
    let (url, seen) = answering(
        "200 OK",
        r#"{"jsonrpc":"2.0","id":7,"result":{"tools":[]}}"#,
    );
    let mut bridge = Bridge::start(Some(&url), Some(&token));
    let answered = bridge.ask(r#"{"jsonrpc":"2.0","id":7,"method":"tools/list"}"#);
    assert_eq!(
        answered,
        json!({"jsonrpc": "2.0", "id": 7, "result": {"tools": []}})
    );
    let head = seen.recv_timeout(WAIT).expect("the server was asked");
    assert!(head.starts_with("POST /mcp HTTP/1.1\r\n"), "{head}");
    assert!(
        head.contains(&format!("Authorization: Bearer {token}\r\n")),
        "{head}"
    );
    let authority = url.trim_start_matches("http://").trim_end_matches("/mcp");
    assert!(head.contains(&format!("Host: {authority}\r\n")), "{head}");

    // A server that takes a notification and answers it with nothing: no line, and the next request is answered in order.
    let (url, _seen) = answering("202 Accepted", "");
    let mut bridge = Bridge::start(Some(&url), Some(&token));
    bridge.send(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
    bridge.send(r#"{"jsonrpc":"2.0","id":"a","method":"ping"}"#);
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        bridge.lines.try_recv().is_err(),
        "a 202 is no answer to give"
    );

    // A server that refuses the bearer, and one that is not listening: an error for the request, never the bearer.
    let (url, _seen) = answering("401 Unauthorized", "");
    let mut bridge = Bridge::start(Some(&url), Some(&token));
    let refused = bridge.ask(r#"{"jsonrpc":"2.0","id":"b","method":"tools/list"}"#);
    assert_eq!(refused["id"], "b");
    assert_eq!(refused["error"]["code"], -32000);
    assert!(
        refused["error"]["message"]
            .as_str()
            .is_some_and(|text| text.contains("bearer")),
        "{refused}"
    );
    let gone = {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("bound");
        format!(
            "http://127.0.0.1:{}/mcp",
            listener.local_addr().expect("an address").port()
        )
    };
    let mut bridge = Bridge::start(Some(&gone), Some(&token));
    let silent = bridge.ask(r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#);
    assert_eq!(silent["id"], 3);
    assert!(
        silent["error"]["message"]
            .as_str()
            .is_some_and(|text| text.contains("not listening")),
        "{silent}"
    );
    assert!(!refused.to_string().contains(&token) && !silent.to_string().contains(&token));
    assert!(!bridge.said().contains(&token));
}

/// What `Call TOOL JSON` answered, as JSON, or the text of an error.
fn called(
    convo: &mut Convo<'_>,
    session: &str,
    tool: &str,
    arguments: &Value,
) -> Result<Value, String> {
    let said = convo.prompt(session, &format!("Call {tool} {arguments}"));
    serde_json::from_str(&said).map_err(|_| said)
}

#[test]
fn a_parent_lists_its_targets_and_hands_a_task_on_with_a_chosen_model_and_effort() {
    let dir = Scratch::new();
    let work = directory(&dir.0, "work");
    let config = format!(
        "{}{}{}",
        agent("parent", "default,manual", ""),
        agent("child", "default,manual", ""),
        agent("hidden", "default,manual", "")
    );
    let host = Hosted::start(&format!(
        "{config}[host.delegate]\nsources = [\"child\", \"hidden\"]\n"
    ));
    let link = host.local.open("parent").expect("a link");
    let mut convo = Convo::new(&link);
    let session = convo.open(true, &work);

    let listed = called(&mut convo, &session, "delegate_targets", &json!({})).expect("targets");
    let names: Vec<&str> = listed["targets"]
        .as_array()
        .expect("targets")
        .iter()
        .filter_map(|target| target["source"].as_str())
        .collect();
    assert_eq!(names, ["child", "hidden"], "{listed}");
    let child = &listed["targets"][0];
    assert_eq!(child["host"], "this host");
    assert_eq!(child["ready"], true);
    assert_eq!(child["accounts"], json!([]));
    assert_eq!(child["workspace"]["kind"], "folder");
    // No credential is listed, whatever the entries hold.
    assert!(
        !listed.to_string().to_lowercase().contains("secret"),
        "{listed}"
    );
    // A source named gets its models too.
    let one = called(
        &mut convo,
        &session,
        "delegate_targets",
        &json!({"source": "child"}),
    )
    .expect("one target");
    assert_eq!(one["targets"].as_array().map(Vec::len), Some(1), "{one}");
    assert!(one["targets"][0].get("models").is_some(), "{one}");
    let refused = called(
        &mut convo,
        &session,
        "delegate_targets",
        &json!({"source": "parent"}),
    )
    .expect_err("not a source the session may use");
    assert!(refused.contains("parent"), "{refused}");

    let handed = called(
        &mut convo,
        &session,
        "delegate_task",
        &json!({"source": "child", "task": "Chosen", "model": "fast", "effort": "high"}),
    )
    .expect("a task");
    let task = handed["taskId"].as_str().expect("an id").to_owned();
    let status = convo.status(&session, &task, 20_000);
    assert_eq!(status["state"], "completed", "{status}");
    assert_eq!(status["result"], "model=fast effort=high");
    drop(link);
    host.stop();
}

#[test]
fn a_choice_the_source_does_not_offer_is_refused_by_name_and_never_replaced() {
    let dir = Scratch::new();
    let work = directory(&dir.0, "work");
    let config = format!(
        "{}{}",
        agent("parent", "default,manual", ""),
        agent("child", "default,manual", "")
    );
    let host = Hosted::start(&config);
    let link = host.local.open("parent").expect("a link");
    let mut convo = Convo::new(&link);
    let session = convo.open(true, &work);

    for (key, named) in [
        ("model", "no-such-model"),
        ("effort", "ludicrous"),
        ("account", "somebody-elses"),
        ("context", "messages"),
    ] {
        let mut asked = json!({"source": "child", "task": "Chosen"});
        asked[key] = json!(named);
        // Either the host refuses it when it is asked, or the task ends rejected naming it: never a task on another model.
        match called(&mut convo, &session, "delegate_task", &asked) {
            Err(refusal) => assert!(refusal.contains(named), "{refusal}"),
            Ok(handed) => {
                let task = handed["taskId"].as_str().expect("an id").to_owned();
                let status = convo.status(&session, &task, 20_000);
                assert_eq!(status["state"], "rejected", "{status}");
                assert!(
                    status["message"].as_str().unwrap_or("").contains(named),
                    "{status}"
                );
            }
        }
    }
    drop(link);
    host.stop();
}

#[test]
fn the_persons_own_task_is_the_sessions_task_and_one_sent_twice_is_made_once() {
    let dir = Scratch::new();
    let work = directory(&dir.0, "work");
    let marker = dir.0.join("child.pids");
    let config = format!(
        "{}{}{}",
        agent("parent", "default,manual", ""),
        marked("child", "default,manual", &marker),
        agent("other", "default,manual", "")
    );
    let host = Hosted::start(&config);
    let link = host.local.open("parent").expect("a link");
    let mut convo = Convo::new(&link);
    let session = convo.open(true, &work);

    let listed = convo
        .request(
            "_canoa/delegate/targets",
            json!({"sessionId": session}),
            &|_| None,
        )
        .expect("targets");
    assert_eq!(
        listed["targets"].as_array().map(Vec::len),
        Some(3),
        "{listed}"
    );
    let start = json!({"sessionId": session, "requestId": "r-1", "source": "child",
        "task": "Chosen", "model": "fast", "title": "By the person"});
    let made = convo
        .request("_canoa/delegate/start", start.clone(), &|_| None)
        .expect("started");
    let task = made["taskId"].as_str().expect("an id").to_owned();
    // The parent's agent reads it with its own task_status, as it reads its own.
    let status = convo.status(&session, &task, 20_000);
    assert_eq!(status["state"], "completed", "{status}");
    assert_eq!(status["title"], "By the person");
    assert_eq!(status["result"], "model=fast effort=low");
    // Sent again after a reconnect, it finds the task and starts no other process (the first start asked what the source lists).
    let started = pids(&marker).len();
    let again = convo
        .request("_canoa/delegate/start", start, &|_| None)
        .expect("found");
    assert_eq!(again["taskId"], task.as_str());
    assert_eq!(again["repeated"], true);
    assert_eq!(pids(&marker).len(), started, "{:?}", pids(&marker));
    // A refusal reaches the person as an error, naming the choice.
    let refused = convo
        .request(
            "_canoa/delegate/start",
            json!({"sessionId": session, "source": "child", "task": "x", "account": "nope"}),
            &|_| None,
        )
        .expect_err("refused");
    assert!(
        refused["message"].as_str().unwrap_or("").contains("nope"),
        "{refused}"
    );
    let missing = convo
        .request(
            "_canoa/delegate/start",
            json!({"sessionId": "no-such", "source": "child", "task": "x"}),
            &|_| None,
        )
        .expect_err("no session");
    assert_eq!(missing["code"], -32602);
    drop(link);
    host.stop();
}

#[test]
fn what_the_person_sets_narrows_the_targets_and_sets_a_default_and_a_limit() {
    let dir = Scratch::new();
    let work = directory(&dir.0, "work");
    let config = format!(
        "{}{}{}",
        agent("parent", "default,manual", ""),
        agent("child", "default,manual", ""),
        agent("other", "default,manual", "")
    );
    let host = Hosted::start(&config);
    let link = host.local.open("parent").expect("a link");
    let mut convo = Convo::new(&link);
    let session = convo.open(true, &work);
    convo
        .request(
            "_canoa/delegate/setup",
            json!({"sessionId": session, "sources": ["child"], "default": "child", "maxRunning": 1}),
            &|_| None,
        )
        .expect("set");
    let listed = called(&mut convo, &session, "delegate_targets", &json!({})).expect("targets");
    assert_eq!(
        listed["targets"].as_array().map(Vec::len),
        Some(1),
        "{listed}"
    );
    assert_eq!(listed["default"], "child");
    // A source the person left out is not one it may name; one with no source goes to the default.
    let hidden = called(
        &mut convo,
        &session,
        "delegate_task",
        &json!({"source": "other", "task": "x"}),
    )
    .expect_err("not offered");
    assert!(hidden.contains("other"), "{hidden}");
    let first = called(
        &mut convo,
        &session,
        "delegate_task",
        &json!({"task": "Wait"}),
    )
    .expect("default");
    assert!(first["taskId"].is_string(), "{first}");
    let second = called(
        &mut convo,
        &session,
        "delegate_task",
        &json!({"task": "Wait"}),
    )
    .expect_err("past the limit");
    assert!(second.contains("1 tasks"), "{second}");
    convo
        .request(
            "_canoa/delegate/stop",
            json!({"sessionId": session}),
            &|_| None,
        )
        .expect("stopped");
    drop(link);
    host.stop();
}

/// A parent entry that offers one profile, `main`, whose text is `profile`.
fn profiled_parent(profile: &str, extra_config: &str) -> String {
    format!(
        "[host.agents.parent]\nprogram = {}\nargs = [\"--scripted\", \"--mcp-http\", \"--modes=default,manual\"]\nlisten = \"127.0.0.1:0\"\nprofiles = [\"main\"]\n[host.profiles.main.delegate]\n{profile}\n{}{}{extra_config}",
        quoted(&example("stdio_agent", false).to_string_lossy()),
        agent("child", "default,manual", ""),
        agent("other", "default,manual", ""),
    )
}

#[test]
fn a_profiles_delegation_suggestions_apply_where_the_connection_allows_them() {
    let dir = Scratch::new();
    let work = directory(&dir.0, "work");
    let config = profiled_parent(
        "default = \"child\"\nsources = [\"child\", \"ghost\"]\nmax_running = 1",
        "",
    );
    let host = Hosted::start(&config);
    let link = host.local.open("parent").expect("a link");
    let mut convo = Convo::new(&link);
    let session = convo.open(true, &work);
    let listed = called(&mut convo, &session, "delegate_targets", &json!({})).expect("targets");
    // The suggested sources narrow what the connection may use, and a name it has no source for adds nothing.
    assert_eq!(
        listed["targets"].as_array().map(Vec::len),
        Some(1),
        "{listed}"
    );
    assert_eq!(listed["targets"][0]["source"], "child", "{listed}");
    assert_eq!(listed["default"], "child", "{listed}");
    assert_eq!(listed["maxRunning"], 1, "{listed}");
    assert_eq!(listed["from"]["default"], "user", "{listed}");
    assert_eq!(listed["from"]["sources"], "user", "{listed}");
    assert_eq!(listed["from"]["maxRunning"], "user", "{listed}");
    let hidden = called(
        &mut convo,
        &session,
        "delegate_task",
        &json!({"source": "other", "task": "x"}),
    )
    .expect_err("not suggested");
    assert!(hidden.contains("other"), "{hidden}");
    // A task with no source goes to the suggested default, and the suggested limit holds.
    let first = called(
        &mut convo,
        &session,
        "delegate_task",
        &json!({"task": "Wait"}),
    )
    .expect("default");
    assert!(first["taskId"].is_string(), "{first}");
    let second = called(
        &mut convo,
        &session,
        "delegate_task",
        &json!({"task": "Wait"}),
    )
    .expect_err("past the limit");
    assert!(second.contains("1 tasks"), "{second}");
    // What the person sets replaces the suggestion key by key, and clearing it brings the suggestion back.
    convo
        .request(
            "_canoa/delegate/setup",
            json!({"sessionId": session, "sources": ["child", "other"], "default": "other", "maxRunning": 5}),
            &|_| None,
        )
        .expect("set");
    let own = called(&mut convo, &session, "delegate_targets", &json!({})).expect("targets");
    assert_eq!(own["default"], "other", "{own}");
    assert_eq!(own["maxRunning"], 5, "{own}");
    assert_eq!(own["from"]["default"], "session", "{own}");
    assert_eq!(own["from"]["sources"], "session", "{own}");
    assert_eq!(own["targets"].as_array().map(Vec::len), Some(2), "{own}");
    convo
        .request(
            "_canoa/delegate/setup",
            json!({"sessionId": session, "sources": null, "default": null, "maxRunning": null}),
            &|_| None,
        )
        .expect("cleared");
    let back = called(&mut convo, &session, "delegate_targets", &json!({})).expect("targets");
    assert_eq!(back["default"], "child", "{back}");
    assert_eq!(back["from"]["default"], "user", "{back}");
    convo
        .request(
            "_canoa/delegate/stop",
            json!({"sessionId": session}),
            &|_| None,
        )
        .expect("stopped");
    drop(link);
    host.stop();
}

#[test]
fn a_suggested_default_the_connection_may_not_use_and_a_limit_above_the_hosts_change_nothing() {
    let dir = Scratch::new();
    let work = directory(&dir.0, "work");
    // `other` is suggested as the default but left out of the suggested sources; the suggested limit is above the host's.
    let config = profiled_parent(
        "default = \"other\"\nsources = [\"child\"]\nmax_running = 9",
        "[host.delegate]\nmax_children = 1\n",
    );
    let host = Hosted::start(&config);
    let link = host.local.open("parent").expect("a link");
    let mut convo = Convo::new(&link);
    let session = convo.open(true, &work);
    let listed = called(&mut convo, &session, "delegate_targets", &json!({})).expect("targets");
    assert!(listed.get("default").is_none(), "{listed}");
    assert_eq!(listed["unusedDefault"]["value"], "other", "{listed}");
    assert_eq!(listed["unusedDefault"]["layer"], "user", "{listed}");
    let none = called(
        &mut convo,
        &session,
        "delegate_task",
        &json!({"task": "Wait"}),
    )
    .expect_err("no default in effect");
    assert!(none.contains("source is required"), "{none}");
    let kept = called(
        &mut convo,
        &session,
        "delegate_task",
        &json!({"source": "child", "task": "Wait"}),
    )
    .expect("a task");
    assert!(kept["taskId"].is_string(), "{kept}");
    let over = called(
        &mut convo,
        &session,
        "delegate_task",
        &json!({"source": "child", "task": "Wait"}),
    )
    .expect_err("the host's limit");
    assert!(over.contains("max_children"), "{over}");
    convo
        .request(
            "_canoa/delegate/stop",
            json!({"sessionId": session}),
            &|_| None,
        )
        .expect("stopped");
    drop(link);
    host.stop();
}

/// `_canoa/delegate/context` as the person's client asks it.
fn preview(convo: &mut Convo<'_>, session: &str, kind: &str) -> Result<Value, Value> {
    convo.request(
        "_canoa/delegate/context",
        json!({"sessionId": session, "kind": kind}),
        &|_| None,
    )
}

#[test]
fn a_task_is_handed_the_conversation_only_as_the_person_allowed_or_was_shown() {
    let dir = Scratch::new();
    let work = directory(&dir.0, "work");
    let config = format!(
        "{}{}",
        agent("parent", "default,manual", ""),
        agent("child", "default,manual", "")
    );
    let host = Hosted::start(&config);
    let link = host.local.open("parent").expect("a link");
    let mut convo = Convo::new(&link);
    let session = convo.open(true, &work);
    convo.prompt(&session, "Chosen");

    // Nothing is handed on by the agent's own call until the person lets the session do it, and the refusal names the kind.
    let asked = json!({"source": "child", "task": "Verbatim now", "context": "messages"});
    let refused = called(&mut convo, &session, "delegate_task", &asked).expect_err("not allowed");
    assert!(
        refused.contains("messages") && refused.contains("setup"),
        "{refused}"
    );

    // The person is shown the text, and a task they start with that preview is given exactly it.
    let shown = preview(&mut convo, &session, "messages").expect("a preview");
    let text = shown["text"].as_str().expect("its text").to_owned();
    assert!(text.contains("[person]\nChosen"), "{text}");
    assert!(text.contains("[agent]\nmodel=default effort=low"), "{text}");
    // The refused call above is a turn of the conversation too.
    assert_eq!(shown["items"].as_array().map(Vec::len), Some(4), "{shown}");
    let started = convo
        .request(
            "_canoa/delegate/start",
            json!({"sessionId": session, "requestId": "c-1", "source": "child", "task": "Verbatim now",
                "context": "messages", "contextId": shown["contextId"]}),
            &|_| None,
        )
        .expect("started");
    let task = started["taskId"].as_str().expect("an id").to_owned();
    let status = convo.status(&session, &task, 20_000);
    assert_eq!(status["state"], "completed", "{status}");
    let received = status["result"].as_str().expect("what the child got");
    assert!(received.contains("text=Verbatim now"), "{received}");
    assert!(received.contains(&text), "{received}");

    // A preview that was never shown is no preview, and a kind the host does not build is refused by name.
    let unknown = convo
        .request(
            "_canoa/delegate/start",
            json!({"sessionId": session, "source": "child", "task": "x", "context": "messages", "contextId": "nope"}),
            &|_| None,
        )
        .expect_err("no such preview");
    assert!(
        unknown["message"]
            .as_str()
            .unwrap_or("")
            .contains("preview"),
        "{unknown}"
    );
    let history = called(
        &mut convo,
        &session,
        "delegate_task",
        &json!({"source": "child", "task": "x", "context": "history"}),
    )
    .expect_err("not a kind");
    assert!(history.contains("history"), "{history}");

    // Once the person lets the session, its own call takes the turns so far, and the setup reads back as they set it.
    convo
        .request(
            "_canoa/delegate/setup",
            json!({"sessionId": session, "context": ["messages"]}),
            &|_| None,
        )
        .expect("set");
    let listed = convo
        .request(
            "_canoa/delegate/targets",
            json!({"sessionId": session}),
            &|_| None,
        )
        .expect("targets");
    assert_eq!(listed["context"], json!(["messages"]), "{listed}");
    assert!(
        listed["offered"]
            .as_array()
            .is_some_and(|offered| offered.iter().any(|each| each["source"] == "child")),
        "{listed}"
    );
    let handed = called(&mut convo, &session, "delegate_task", &asked).expect("allowed");
    let task = handed["taskId"].as_str().expect("an id").to_owned();
    let status = convo.status(&session, &task, 20_000);
    assert_eq!(status["state"], "completed", "{status}");
    assert!(
        status["result"]
            .as_str()
            .is_some_and(|result| result.contains("[person]\nChosen")),
        "{status}"
    );
    // The kind the person did not allow stays refused.
    let files = called(
        &mut convo,
        &session,
        "delegate_task",
        &json!({"source": "child", "task": "Verbatim now", "context": "files"}),
    )
    .expect_err("files are not allowed");
    assert!(files.contains("files"), "{files}");
    drop(link);
    host.stop();
}

#[test]
fn the_files_the_agents_tools_touched_are_handed_on_from_inside_its_folder_only() {
    let dir = Scratch::new();
    let work = directory(&dir.0, "work");
    std::fs::create_dir_all(work.join("src")).expect("a folder");
    std::fs::write(work.join("src").join("calc.js"), "return a - b;").expect("a file");
    let config = format!(
        "{}{}",
        agent("parent", "default,manual", ""),
        agent("child", "default,manual", "")
    );
    let host = Hosted::start(&config);
    let link = host.local.open("parent").expect("a link");
    let mut convo = Convo::new(&link);
    let session = convo.open(true, &work);

    let none = preview(&mut convo, &session, "files").expect_err("nothing touched yet");
    assert!(
        none["message"].as_str().unwrap_or("").contains("no file"),
        "{none}"
    );
    convo.prompt_answering(&session, "The tests fails", &|_| {
        Some(json!({"outcome": {"outcome": "selected", "optionId": "yes"}}))
    });
    let shown = preview(&mut convo, &session, "files").expect("a preview");
    let text = shown["text"].as_str().expect("its text").to_owned();
    assert!(text.contains("return a - b;"), "{text}");
    let first = shown["items"][0]["name"].as_str().expect("a name");
    assert!(first.ends_with("calc.js"), "{shown}");
    let started = convo
        .request(
            "_canoa/delegate/start",
            json!({"sessionId": session, "source": "child", "task": "Verbatim now",
                "context": "files", "contextId": shown["contextId"]}),
            &|_| None,
        )
        .expect("started");
    let task = started["taskId"].as_str().expect("an id").to_owned();
    let status = convo.status(&session, &task, 20_000);
    assert_eq!(status["state"], "completed", "{status}");
    assert!(
        status["result"]
            .as_str()
            .is_some_and(|result| result.contains("return a - b;")),
        "{status}"
    );
    drop(link);
    host.stop();
}

#[test]
fn a_profile_the_entry_offers_is_taken_and_one_it_does_not_is_refused_by_name() {
    let dir = Scratch::new();
    let work = directory(&dir.0, "work");
    let config = format!(
        "{}[host.agents.child]\nprogram = {}\nargs = [\"--scripted\", \"--mcp-http\", \"--modes=default,manual\"]\nlisten = \"127.0.0.1:0\"\nprofiles = [\"main\"]\n[host.profiles.main]\n",
        agent("parent", "default,manual", ""),
        quoted(&example("stdio_agent", false).to_string_lossy()),
    );
    let host = Hosted::start(&config);
    let link = host.local.open("parent").expect("a link");
    let mut convo = Convo::new(&link);
    let session = convo.open(true, &work);

    let listed = called(&mut convo, &session, "delegate_targets", &json!({})).expect("targets");
    let child = listed["targets"]
        .as_array()
        .and_then(|targets| targets.iter().find(|target| target["source"] == "child"))
        .expect("child");
    assert_eq!(child["profiles"][0]["name"], "main", "{listed}");
    let handed = called(
        &mut convo,
        &session,
        "delegate_task",
        &json!({"source": "child", "task": "Chosen", "profile": "main"}),
    )
    .expect("a task");
    let task = handed["taskId"].as_str().expect("an id").to_owned();
    let status = convo.status(&session, &task, 20_000);
    assert_eq!(status["state"], "completed", "{status}");
    for (key, named) in [
        ("profile", "no-such-profile"),
        ("loadout", "no-such-loadout"),
    ] {
        let mut asked = json!({"source": "child", "task": "Chosen", "profile": "main"});
        asked[key] = json!(named);
        match called(&mut convo, &session, "delegate_task", &asked) {
            Err(refusal) => assert!(refusal.contains(named), "{refusal}"),
            Ok(handed) => {
                let task = handed["taskId"].as_str().expect("an id").to_owned();
                let status = convo.status(&session, &task, 20_000);
                assert_eq!(status["state"], "rejected", "{status}");
                assert!(
                    status["message"].as_str().unwrap_or("").contains(named),
                    "{status}"
                );
            }
        }
    }
    drop(link);
    host.stop();
}
