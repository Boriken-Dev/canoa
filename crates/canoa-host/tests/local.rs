//! The way into a running host from its own process: a link served as a paired key's connection is, with no proof, ended by the
//! host's stop.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod support;

use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use canoa_host::cli::{self, Cli, Front, Program};
use canoa_host::{Link, LinkError, Local, LocalError};
use clap::Parser as _;
use serde_json::{Value, json};
use support::{Scratch, WAIT, cwd, example, host_binary, quoted, running};

type Then = Box<dyn FnOnce() + Send>;

struct TestFront {
    home: PathBuf,
    local: Mutex<mpsc::Sender<Local>>,
    then: Arc<Mutex<Option<Then>>>,
}

impl Front for TestFront {
    fn print(&self, _line: &str) -> std::io::Result<()> {
        Ok(())
    }

    fn say(&self, _line: &str) {}

    fn when_input_closes(&self, _status: u8, then: Then) -> std::io::Result<()> {
        *self.then.lock().expect("not poisoned") = Some(then);
        Ok(())
    }

    fn program(&self) -> std::io::Result<Program> {
        Ok(Program::new(PathBuf::from(host_binary()), Vec::new()))
    }

    fn home(&self) -> Option<PathBuf> {
        Some(self.home.clone())
    }

    fn holder(&self) -> &str {
        "a test"
    }

    fn ask(&self, _prompt: &str, _secret: bool) -> std::io::Result<Option<String>> {
        Ok(None)
    }

    fn local(&self, local: Local) {
        let _ = self.local.lock().expect("not poisoned").send(local);
    }
}

/// A host on a new home serving `config`, and the way into it.
struct Hosted {
    local: Local,
    then: Arc<Mutex<Option<Then>>>,
    ended: std::thread::JoinHandle<Result<(), cli::Failed>>,
    scratch: Scratch,
}

impl Hosted {
    fn start(entries: &str) -> Self {
        let scratch = Scratch::new();
        let config = scratch.0.join("canoa.cfg");
        std::fs::write(&config, entries).expect("written");
        let (sender, receiver) = mpsc::channel();
        let then = Arc::new(Mutex::new(None));
        let front = TestFront {
            home: scratch.0.join("home"),
            local: Mutex::new(sender),
            then: Arc::clone(&then),
        };
        let method = support::method().name();
        let parsed = Cli::try_parse_from([
            "canoa",
            "serve",
            "--until-stdin-closes",
            "--method",
            method,
            "--config",
            &config.to_string_lossy(),
        ])
        .expect("parses");
        let ended = std::thread::spawn(move || cli::run(parsed, &front));
        let local = receiver
            .recv_timeout(WAIT)
            .expect("the way in is handed over");
        Self {
            local,
            then,
            ended,
            scratch,
        }
    }

    /// Stops the host as an application does and waits for it to return.
    fn stop(&self) {
        // The way in is handed over just before the host registers what its input closing runs.
        let until = Instant::now() + WAIT;
        let then = loop {
            if let Some(then) = self.then.lock().expect("not poisoned").take() {
                break then;
            }
            assert!(Instant::now() < until, "it waits on its input");
            std::thread::sleep(Duration::from_millis(10));
        };
        std::thread::spawn(then).join().expect("it stopped");
    }
}

fn entry(name: &str, extra: &str) -> String {
    format!(
        "[host.agents.{name}]\nprogram = {}\nlisten = \"127.0.0.1:0\"\n{extra}\n",
        quoted(&example("stdio_agent", false).to_string_lossy())
    )
}

fn send(link: &Link, message: &Value) {
    link.send(message.to_string().as_bytes()).expect("sent");
}

/// The next message the agent sent, or why there is none.
fn next(link: &Link) -> Result<Value, String> {
    let until = Instant::now() + WAIT;
    loop {
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err("nothing arrived in time".to_owned());
        }
        match link.receive(Some(left)) {
            Ok(Some(message)) => {
                return serde_json::from_slice(&message).map_err(|e| e.to_string());
            }
            Ok(None) => return Err("the link ended".to_owned()),
            Err(LinkError::Timeout) => {}
            Err(failure) => return Err(failure.to_string()),
        }
    }
}

/// Sends a request and waits for its answer, keeping what came before it.
fn ask(link: &Link, id: u64, method: &str, params: Value) -> Result<(Value, Vec<Value>), String> {
    send(
        link,
        &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
    );
    let mut updates = Vec::new();
    loop {
        let message = next(link)?;
        if message["id"] == id && message.get("method").is_none() {
            return match message.get("error") {
                Some(error) => Err(error.to_string()),
                None => Ok((message["result"].clone(), updates)),
            };
        }
        updates.push(message);
    }
}

fn open_session(link: &Link, dir: &str) -> String {
    ask(
        link,
        1,
        "initialize",
        json!({"protocolVersion": 1, "clientCapabilities": {}}),
    )
    .expect("initialize is answered");
    let (session, _) = ask(
        link,
        2,
        "session/new",
        json!({"cwd": dir, "mcpServers": []}),
    )
    .expect("a session opens");
    session["sessionId"].as_str().expect("an id").to_owned()
}

fn said(updates: &[Value]) -> String {
    updates
        .iter()
        .filter(|update| update["params"]["update"]["sessionUpdate"] == "agent_message_chunk")
        .filter_map(|update| update["params"]["update"]["content"]["text"].as_str())
        .collect()
}

#[test]
fn a_link_from_inside_runs_a_turn_with_no_proof_and_no_paired_key() {
    let host = Hosted::start(&entry("fixture", ""));
    assert_eq!(host.local.entries(), ["fixture"]);
    let link = host.local.open("fixture").expect("a link");
    let session = open_session(&link, &cwd(&host.scratch));
    let (answer, updates) = ask(
        &link,
        3,
        "session/prompt",
        json!({"sessionId": session, "prompt": [{"type": "text", "text": "hello"}]}),
    )
    .expect("the turn is answered");
    assert_eq!(answer["stopReason"], "end_turn");
    assert_eq!(said(&updates), "echo: hello");
    drop(link);
    host.stop();
    assert!(host.ended.join().expect("no panic").is_ok());
}

#[test]
fn an_entry_that_lists_other_keys_is_still_reached_from_inside() {
    let other = format!("sha256:{}", "0".repeat(64));
    let host = Hosted::start(&entry("fixture", &format!("peers = [\"{other}\"]")));
    let link = host.local.open("fixture").expect("a link");
    let session = open_session(&link, &cwd(&host.scratch));
    assert!(!session.is_empty());
    drop(link);
    host.stop();
}

#[test]
fn an_entry_the_host_does_not_serve_is_no_entry() {
    let host = Hosted::start(&entry("fixture", ""));
    assert_eq!(host.local.open("other").err(), Some(LocalError::NoEntry));
    host.stop();
}

#[test]
fn closing_the_link_ends_its_agent() {
    let host = Hosted::start(&entry("fixture", ""));
    let link = host.local.open("fixture").expect("a link");
    open_session(&link, &cwd(&host.scratch));
    let (said, _) = ask(&link, 3, "_fixture/pid", json!({})).expect("it answers");
    let pid = u32::try_from(said["pid"].as_u64().expect("a pid")).expect("a pid");
    assert!(running(pid));
    link.close();
    let until = Instant::now() + WAIT;
    while running(pid) && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!running(pid), "the agent outlived its link");
    host.stop();
}

#[test]
fn stopping_the_host_ends_a_link_and_its_agent_and_later_links_are_refused() {
    let host = Hosted::start(&entry("fixture", ""));
    let link = host.local.open("fixture").expect("a link");
    open_session(&link, &cwd(&host.scratch));
    let (said, _) = ask(&link, 3, "_fixture/pid", json!({})).expect("it answers");
    let pid = u32::try_from(said["pid"].as_u64().expect("a pid")).expect("a pid");
    host.stop();
    assert!(
        !running(pid),
        "the agent was ended before the host returned"
    );
    assert!(next(&link).is_err(), "the link ended");
    assert_eq!(host.local.open("fixture").err(), Some(LocalError::Stopped));
    assert!(host.ended.join().expect("no panic").is_ok());
}

#[test]
fn entries_with_max_peers_take_no_more_links_than_they_allow() {
    let host = Hosted::start(&entry("fixture", "max_peers = 1"));
    let first = host.local.open("fixture").expect("a link");
    open_session(&first, &cwd(&host.scratch));
    let second = host
        .local
        .open("fixture")
        .expect("opens, then is turned away");
    send(
        &second,
        &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": 1}}),
    );
    assert!(next(&second).is_err(), "the second is closed");
    drop(first);
    host.stop();
}
