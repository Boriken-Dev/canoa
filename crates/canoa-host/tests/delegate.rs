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
use std::time::{Duration, Instant};

use canoa_host::{Link, LinkError};
use linked::Hosted;
use serde_json::{Value, json};
use support::{Host, Scratch, WAIT, example, quoted, running};

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
}

impl<'a> Convo<'a> {
    fn new(link: &'a Link) -> Self {
        Self {
            link,
            next: 0,
            requests: Vec::new(),
            updates: Vec::new(),
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
fn no_opt_in_or_a_host_that_turns_it_off_or_an_agent_without_http_is_given_no_tool() {
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

    // An agent that takes no HTTP MCP server is offered none.
    let plain = format!(
        "[host.agents.parent]\nprogram = {}\nargs = [\"--scripted\"]\nlisten = \"127.0.0.1:0\"\n",
        quoted(&example("stdio_agent", false).to_string_lossy())
    );
    let host = Hosted::start(&plain);
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
    assert_eq!(names, ["delegate_task", "task_status", "cancel"]);
    assert!(
        listed["result"]["tools"][0]["inputSchema"]["properties"]["source"]["description"]
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
