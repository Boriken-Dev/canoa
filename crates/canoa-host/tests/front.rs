//! The command line run through a `Front` of a test's own, as an application runs it in its own process: there, what a
//! command leaves running when it returns goes on running, so its end has to mean that nothing of it acts any more.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod support;

use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use canoa_host::Home;
use canoa_host::cli::{self, Cli, Front, Program};
use clap::Parser as _;
use coqui_agentic_peer::{auth, pairing};
use coqui_peer_webcrypto::pair::{CodeAlphabet, Pin};
use coqui_web::Dialer as _;
use coqui_web::websocket::{self, Frames};
use coqui_web::{Channel as _, Kind, Limits};
use serde_json::{Value, json};
use support::{Scratch, WAIT, example, host_binary, quoted, running, with_link};

type Then = Box<dyn FnOnce() + Send>;

/// What a command prints, and what closing its input runs, kept for the test.
struct TestFront {
    home: PathBuf,
    out: Mutex<mpsc::Sender<String>>,
    then: Arc<Mutex<Option<Then>>>,
}

impl Front for TestFront {
    fn print(&self, line: &str) -> std::io::Result<()> {
        let _ = self.out.lock().expect("not poisoned").send(line.to_owned());
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
}

struct Running {
    lines: mpsc::Receiver<String>,
    then: Arc<Mutex<Option<Then>>>,
    ended: std::thread::JoinHandle<Result<(), cli::Failed>>,
}

impl Running {
    fn start(home: PathBuf, args: &[&str]) -> Self {
        let (out, lines) = mpsc::channel();
        let then = Arc::new(Mutex::new(None));
        let front = TestFront {
            home,
            out: Mutex::new(out),
            then: Arc::clone(&then),
        };
        let parsed = Cli::try_parse_from(core::iter::once("canoa").chain(args.iter().copied()))
            .expect("parses");
        let ended = std::thread::spawn(move || cli::run(parsed, &front));
        Self { lines, then, ended }
    }

    fn line(&self) -> Value {
        serde_json::from_str(&self.lines.recv_timeout(WAIT).expect("a line")).expect("JSON")
    }

    /// Closes its input as an application does, from a thread of its own, and waits for the command to return.
    fn close_input(self) -> Result<(), cli::Failed> {
        let then = self
            .then
            .lock()
            .expect("not poisoned")
            .take()
            .expect("it waits on its input");
        std::thread::spawn(then);
        self.ended.join().expect("it did not panic")
    }
}

#[test]
fn a_pairing_given_up_accepts_nobody_who_was_still_connecting() {
    let scratch = Scratch::new();
    let home = scratch.0.join("home");
    let method = support::method().name();
    let pairing = Running::start(
        home.clone(),
        &["pair", "--until-stdin-closes", "--method", method],
    );
    let shown = pairing.line();
    let url = url::Url::parse(shown["url"].as_str().expect("a URL")).expect("parses");
    let pin =
        Pin::parse(&CodeAlphabet::pin(), shown["pin"].as_str().expect("a PIN")).expect("a PIN");

    // Connected while the PIN is shown, and proving only once the person has given up.
    let channel = websocket::Dialer::new()
        .with_frames(Frames::Binary)
        .with_limits(Limits::default().with_max_message(auth::MAX_PROOF_MESSAGE))
        .dial(Kind::WebSocket, &url, WAIT)
        .expect("it connects");
    let failed = pairing.close_input().expect_err("given up");
    assert!(failed.why.contains("cancelled"), "{failed:?}");

    let client = support::key();
    let client_store = Home::at(scratch.0.join("client")).store().expect("a store");
    let joined = pairing::join(&channel, &client, pin, &client_store, WAIT);
    channel.close();
    assert!(joined.is_err(), "the PIN is no longer good");
    let store = Home::at(&home).store().expect("the host's store");
    assert!(
        !store.is_accepted(&client.peer_id()),
        "a key paired after the pairing was given up"
    );
}

#[test]
fn a_host_stopped_through_its_input_has_ended_every_agent_by_the_time_it_returns() {
    let scratch = Scratch::new();
    let home = scratch.0.join("home");
    let config = scratch.0.join("canoa.cfg");
    let entries: String = (0..4)
        .map(|n| {
            format!(
                "[host.agents.fixture{n}]\nprogram = {}\nlisten = \"127.0.0.1:0\"\n",
                quoted(&example("stdio_agent", false).to_string_lossy())
            )
        })
        .collect();
    std::fs::write(&config, entries).expect("written");
    let method = support::method().name();
    let host = Running::start(
        home.clone(),
        &[
            "serve",
            "--until-stdin-closes",
            "--method",
            method,
            "--config",
            &config.to_string_lossy(),
        ],
    );
    let urls: Vec<String> = (0..4)
        .map(|_| host.line()["url"].as_str().expect("a URL").to_owned())
        .collect();
    let peer = host.line()["peer"]
        .as_str()
        .expect("the host's id")
        .parse()
        .expect("a peer id");

    let client = support::key();
    Home::at(&home)
        .store()
        .expect("a store")
        .accept_key(&client.key_material())
        .expect("the key is accepted");
    let (pids, pid) = mpsc::channel();
    let links: Vec<_> = urls
        .into_iter()
        .map(|url| {
            let (client, pids) = (client.clone(), pids.clone());
            std::thread::spawn(move || {
                with_link(&url, &client, peer, |rpc| {
                    rpc.initialize().expect("initialize is answered");
                    let said = rpc.request("_fixture/pid", json!({})).expect("it answers");
                    pids.send(said["pid"].as_u64().expect("a pid"))
                        .expect("the test waits");
                    rpc.next_message(Duration::from_secs(30))
                })
            })
        })
        .collect();
    let agents: Vec<u32> = (0..4)
        .map(|_| u32::try_from(pid.recv_timeout(WAIT).expect("a pid")).expect("a pid"))
        .collect();

    host.close_input().expect("it ends in order");
    // At once: an application may end the moment the host says it has.
    let left: Vec<u32> = agents.into_iter().filter(|pid| running(*pid)).collect();
    assert!(
        left.is_empty(),
        "still running after the host returned: {left:?}"
    );
    for link in links {
        let _ = link.join();
    }
}
