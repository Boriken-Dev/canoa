//! A host run through the command line in this process with its way in kept, and a client of its links: what the tests of
//! `Local` share.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    dead_code,
    reason = "tests name what they use from std, and not every test uses all of it"
)]

use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use canoa_host::cli::{self, Cli, Front, Program};
use canoa_host::{Link, LinkError, Local};
use clap::Parser as _;
use serde_json::{Value, json};

use crate::support::{Scratch, WAIT, example, host_binary, quoted};

type Then = Box<dyn FnOnce() + Send>;

pub struct TestFront {
    home: PathBuf,
    printed: Arc<Mutex<Vec<Value>>>,
    pub local: Mutex<mpsc::Sender<Local>>,
    pub then: Arc<Mutex<Option<Then>>>,
}

impl Front for TestFront {
    fn print(&self, line: &str) -> std::io::Result<()> {
        if let Ok(value) = serde_json::from_str(line) {
            self.printed.lock().expect("not poisoned").push(value);
        }
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
pub struct Hosted {
    pub local: Local,
    pub then: Arc<Mutex<Option<Then>>>,
    pub ended: std::thread::JoinHandle<Result<(), cli::Failed>>,
    pub scratch: Scratch,
    /// The JSON lines `serve` printed, up to when the way in was handed over: one per entry.
    pub printed: Arc<Mutex<Vec<Value>>>,
}

impl Hosted {
    pub fn start(entries: &str) -> Self {
        Self::start_with(entries, &[])
    }

    /// As [`Hosted::start`], with more arguments to `serve`.
    pub fn start_with(entries: &str, more: &[&str]) -> Self {
        Self::start_in(Scratch::new(), entries, more)
    }

    /// As [`Hosted::start_with`], in a `scratch` the test has already put things in, such as the host's identity.
    pub fn start_in(scratch: Scratch, entries: &str, more: &[&str]) -> Self {
        let config = scratch.0.join("canoa.cfg");
        std::fs::write(&config, entries).expect("written");
        let (sender, receiver) = mpsc::channel();
        let then = Arc::new(Mutex::new(None));
        let printed = Arc::new(Mutex::new(Vec::new()));
        let front = TestFront {
            home: scratch.0.join("home"),
            printed: Arc::clone(&printed),
            local: Mutex::new(sender),
            then: Arc::clone(&then),
        };
        let method = crate::support::method().name();
        let config = config.to_string_lossy().into_owned();
        let parsed = Cli::try_parse_from(
            [
                "canoa",
                "serve",
                "--until-stdin-closes",
                "--method",
                method,
                "--config",
                &config,
            ]
            .into_iter()
            .chain(more.iter().copied()),
        )
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
            printed,
        }
    }

    /// What `serve` printed for the entry `name`, once it has printed it.
    pub fn line_of(&self, name: &str) -> Option<Value> {
        let until = Instant::now() + WAIT;
        loop {
            let found = self
                .printed
                .lock()
                .expect("not poisoned")
                .iter()
                .find(|line| line["name"] == name)
                .cloned();
            if found.is_some() || Instant::now() >= until {
                return found;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Stops the host as an application does and waits for it to return.
    pub fn stop(&self) {
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

pub fn entry(name: &str, extra: &str) -> String {
    format!(
        "[host.agents.{name}]\nprogram = {}\nlisten = \"127.0.0.1:0\"\n{extra}\n",
        quoted(&example("stdio_agent", false).to_string_lossy())
    )
}

pub fn send(link: &Link, message: &Value) {
    link.send(message.to_string().as_bytes()).expect("sent");
}

/// The next message the agent sent, or why there is none.
pub fn next(link: &Link) -> Result<Value, String> {
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
pub fn ask(
    link: &Link,
    id: u64,
    method: &str,
    params: Value,
) -> Result<(Value, Vec<Value>), String> {
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

pub fn open_session(link: &Link, dir: &str) -> String {
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

pub fn said(updates: &[Value]) -> String {
    updates
        .iter()
        .filter(|update| update["method"] == "session/update")
        .filter(|update| update["params"]["update"]["sessionUpdate"] == "agent_message_chunk")
        .filter_map(|update| update["params"]["update"]["content"]["text"].as_str())
        .collect()
}
