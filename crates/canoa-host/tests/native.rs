//! The browser extension's native messaging host, as the browser runs it: the built `canoa` started with an extension's origin,
//! speaking native messages on standard input and output, relaying to a running host's door. The link through it is proven
//! end to end by a client of the host's own key, as the extension proves.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod support;

use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};


use coqui_agentic_peer::companion::{read_message_up_to, write_message};
use coqui_transport::{Channel, Failure};
use coqui_transport_web::frame;
use data_encoding::BASE64;
use serde_json::{Value, json};
use support::{Host, Scratch, WAIT, cwd, example, host_binary, quoted};

const ORIGIN: &str = "chrome-extension://lpppofcphpjkedfnpbpabhmiamaicaed/";
const MAX: usize = 16 * 1024 * 1024;
/// What a browser takes from a native host is 1 MiB a message; a piece is base64 in one.
const MAX_PIECE: usize = 768_000;

/// A native messaging host as the browser starts it: the program, the origin as its argument, and the browser's end of its streams.
struct Relay {
    child: Child,
    stdin: Mutex<Option<ChildStdin>>,
    messages: Mutex<Receiver<Value>>,
}

impl Relay {
    fn start(home: &Path, origin: &str) -> Self {
        let mut child = Command::new(host_binary())
            .arg(origin)
            .env("COQUI_AGENTIC_PEER_HOME", home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("the native host starts");
        let stdin = child.stdin.take();
        let mut stdout = child.stdout.take().expect("its output");
        let (send, messages) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(Some(bytes)) = read_message_up_to(&mut stdout, 2 * 1024 * 1024) {
                let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
                    return;
                };
                if send.send(value).is_err() {
                    return;
                }
            }
        });
        Self {
            child,
            stdin: Mutex::new(stdin),
            messages: Mutex::new(messages),
        }
    }

    fn write(&self, message: &Value) -> bool {
        let mut stdin = self.stdin.lock().unwrap_or_else(PoisonError::into_inner);
        stdin
            .as_mut()
            .is_some_and(|stdin| write_message(stdin, message.to_string().as_bytes()).is_ok())
    }

    fn next(&self, wait: Duration) -> Option<Value> {
        self.messages
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .recv_timeout(wait)
            .ok()
    }

    /// Asks `request` and takes the reply that is not a piece of a link.
    fn ask(&self, request: &Value) -> Value {
        assert!(self.write(request), "the native host takes a message");
        self.next(WAIT).expect("the native host answers")
    }

    fn close_input(&self) {
        drop(self.stdin.lock().unwrap_or_else(PoisonError::into_inner).take());
    }

    fn ended_within(&mut self, wait: Duration) -> bool {
        let until = Instant::now() + wait;
        loop {
            if self.child.try_wait().expect("its status").is_some() {
                return true;
            }
            if Instant::now() >= until {
                return false;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The link a relay carries, as a channel: what is sent is cut into pieces, and what arrives is put back together.
struct Link<'a> {
    relay: &'a Relay,
    inbox: Mutex<Vec<u8>>,
    pieces: Mutex<Vec<usize>>,
    limit: std::sync::atomic::AtomicUsize,
}

impl<'a> Link<'a> {
    /// Opens `entry` through `relay`; `Err` with what the native host said when it will not.
    fn open(relay: &'a Relay, entry: &str) -> Result<Self, Value> {
        let reply = relay.ask(&json!({ "op": "open", "entry": entry }));
        if reply["opened"] != true {
            return Err(reply);
        }
        Ok(Self {
            relay,
            inbox: Mutex::new(Vec::new()),
            pieces: Mutex::new(Vec::new()),
            limit: std::sync::atomic::AtomicUsize::new(MAX),
        })
    }

    fn sizes(&self) -> Vec<usize> {
        self.pieces.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

impl Channel for Link<'_> {
    fn send(&self, message: &[u8]) -> Result<(), Failure> {
        let framed = frame::encode(message, MAX).map_err(|_| Failure::Protocol)?;
        for piece in framed.chunks(MAX_PIECE) {
            if !self.relay.write(&json!({ "d": BASE64.encode(piece) })) {
                return Err(Failure::PathLost);
            }
        }
        Ok(())
    }

    fn receive(&self, timeout: Option<Duration>) -> Result<Option<Vec<u8>>, Failure> {
        let until = timeout.map(|wait| Instant::now() + wait);
        let mut inbox = self.inbox.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if inbox.len() >= 4 {
                let length = u32::from_le_bytes([inbox[0], inbox[1], inbox[2], inbox[3]]) as usize;
                if length == 0 || length > self.limit.load(std::sync::atomic::Ordering::SeqCst) {
                    return Err(Failure::Protocol);
                }
                if inbox.len() >= 4 + length {
                    let body = inbox[4..4 + length].to_vec();
                    inbox.drain(..4 + length);
                    return Ok(Some(body));
                }
            }
            let wait = until.map_or(WAIT, |until| until.saturating_duration_since(Instant::now()));
            let messages = self.relay.messages.lock().unwrap_or_else(PoisonError::into_inner);
            match messages.recv_timeout(wait) {
                Ok(message) if message.get("d").is_some() => {
                    let bytes = BASE64
                        .decode(message["d"].as_str().unwrap_or_default().as_bytes())
                        .map_err(|_| Failure::Protocol)?;
                    self.pieces.lock().unwrap_or_else(PoisonError::into_inner).push(bytes.len());
                    inbox.extend_from_slice(&bytes);
                }
                Ok(message) if message["closed"] == true => return Ok(None),
                Ok(_) => {}
                Err(RecvTimeoutError::Timeout) => return Err(Failure::Timeout),
                Err(RecvTimeoutError::Disconnected) => return Ok(None),
            }
        }
    }

    fn close(&self) {
        self.relay.close_input();
    }

    fn set_max_message(&self, bytes: usize) {
        self.limit.store(bytes, std::sync::atomic::Ordering::SeqCst);
    }

    fn set_max_receive(&self, bytes: usize) {
        self.limit.store(bytes, std::sync::atomic::Ordering::SeqCst);
    }

    fn set_max_send(&self, _bytes: usize) {}
}

fn agent_entry() -> String {
    format!(
        "[host.agents.fixture]\nprogram = {}\n",
        quoted(&example("stdio_agent", false).to_string_lossy())
    )
}

fn host() -> Host {
    Host::start(&agent_entry(), 1)
}

fn turn(rpc: &support::Rpc<'_, Link<'_>>, dir: &str, text: &str) -> Result<String, Value> {
    rpc.initialize()?;
    let session = rpc.request("session/new", json!({"cwd": dir, "mcpServers": []}))?;
    rpc.prompt(session["sessionId"].as_str().expect("an id"), text)
}

#[test]
fn hello_lists_the_entries_and_says_the_host_runs() {
    let host = host();
    let relay = Relay::start(&host.home(), ORIGIN);
    let reply = relay.ask(&json!({ "op": "hello" }));
    assert_eq!(reply["reply"], "hello", "{reply}");
    assert_eq!(reply["running"], true, "{reply}");
    assert_eq!(reply["name"], "Canoa");
    assert_eq!(reply["peer"], host.peer.to_string().as_str());
    assert_eq!(reply["method"], support::method().name());
    assert_eq!(reply["entries"][0]["name"], "fixture", "{reply}");
    assert_eq!(reply["entries"][0]["kind"], "agent");
    assert_eq!(reply["control"], true);
}

#[test]
fn no_host_running_answers_that_it_is_not() {
    let scratch = Scratch::new();
    let relay = Relay::start(&scratch.0.join("home"), ORIGIN);
    let reply = relay.ask(&json!({ "op": "hello" }));
    assert_eq!(reply["running"], false, "{reply}");
    assert!(reply.get("entries").is_none(), "{reply}");
    let refused = relay.ask(&json!({ "op": "open", "entry": "fixture" }));
    assert!(refused["error"].is_string() && refused["opened"].is_null(), "{refused}");
}

#[test]
fn a_host_of_another_home_is_not_reached() {
    let ours = host();
    let theirs = host();
    assert_ne!(ours.peer, theirs.peer);
    let relay = Relay::start(&theirs.home(), ORIGIN);
    let reply = relay.ask(&json!({ "op": "hello" }));
    assert_eq!(reply["peer"], theirs.peer.to_string().as_str(), "{reply}");
    let other = Relay::start(&ours.home(), ORIGIN);
    assert_eq!(
        other.ask(&json!({ "op": "hello" }))["peer"],
        ours.peer.to_string().as_str()
    );
}

#[test]
fn a_paired_key_runs_a_turn_through_the_relay_and_an_unpaired_one_is_refused() {
    let host = host();
    let client = support::key();

    let relay = Relay::start(&host.home(), ORIGIN);
    let link = Link::open(&relay, "fixture").expect("the door opens the entry");
    let refused = support::with_channel(link, &client, host.peer, |rpc| rpc.initialize());
    let refused = refused.map(|inner| inner.map_err(|error| error.to_string()));
    let none = match support::method() {
        coqui_agentic_peer::auth::Method::WebCrypto => refused.is_err(),
        _ => !matches!(refused, Ok(Ok(_))),
    };
    assert!(none, "an unpaired key gets nothing: {refused:?}");

    host.pair(&client);
    let relay = Relay::start(&host.home(), ORIGIN);
    let link = Link::open(&relay, "fixture").expect("the door opens the entry");
    let dir = cwd(&host.scratch);
    let text = support::with_channel(link, &client, host.peer, |rpc| turn(rpc, &dir, "hello"))
        .expect("a paired key is served")
        .expect("the turn ends");
    assert_eq!(text, "echo: hello");
}

#[test]
fn a_record_of_three_mebibytes_arrives_whole_in_pieces_under_the_limit() {
    let host = host();
    let client = support::key();
    host.pair(&client);
    let relay = Relay::start(&host.home(), ORIGIN);
    let link = Link::open(&relay, "fixture").expect("the door opens the entry");
    let dir = cwd(&host.scratch);
    let big = "y".repeat(3 * 1024 * 1024);
    let sizes = std::cell::RefCell::new(Vec::new());
    let outcome = {
        let link_ref = &link;
        let result = support::with_channel(
            Spy { link: link_ref },
            &client,
            host.peer,
            |rpc| turn_spy(rpc, &dir, &big),
        );
        sizes.borrow_mut().extend(link_ref.sizes());
        result
    };
    let text = outcome.expect("a paired key is served").expect("the turn ends");
    assert_eq!(text.len(), "echo: ".len() + big.len(), "the record arrived whole");
    let sizes = sizes.into_inner();
    assert!(sizes.iter().all(|size| *size <= MAX_PIECE), "{sizes:?}");
    assert!(sizes.len() >= 4, "a 3 MiB record came in {} pieces", sizes.len());
}

/// The same link, lent to a proof that takes its channel by value.
struct Spy<'a, 'b> {
    link: &'a Link<'b>,
}

impl Channel for Spy<'_, '_> {
    fn send(&self, message: &[u8]) -> Result<(), Failure> {
        self.link.send(message)
    }

    fn receive(&self, timeout: Option<Duration>) -> Result<Option<Vec<u8>>, Failure> {
        self.link.receive(timeout)
    }

    fn close(&self) {
        self.link.close();
    }

    fn set_max_message(&self, bytes: usize) {
        self.link.set_max_message(bytes);
    }

    fn set_max_receive(&self, bytes: usize) {
        self.link.set_max_receive(bytes);
    }

    fn set_max_send(&self, bytes: usize) {
        self.link.set_max_send(bytes);
    }
}

fn turn_spy(rpc: &support::Rpc<'_, Spy<'_, '_>>, dir: &str, text: &str) -> Result<String, Value> {
    rpc.initialize()?;
    let session = rpc.request("session/new", json!({"cwd": dir, "mcpServers": []}))?;
    rpc.prompt(session["sessionId"].as_str().expect("an id"), text)
}

#[test]
fn the_relay_ends_when_the_browser_closes_its_input() {
    let host = host();
    let mut relay = Relay::start(&host.home(), ORIGIN);
    assert_eq!(relay.ask(&json!({ "op": "hello" }))["running"], true);
    relay.close_input();
    assert!(relay.ended_within(Duration::from_secs(10)), "it ended");
}

#[test]
fn an_unknown_entry_and_an_unknown_op_are_answered_not_obeyed() {
    let host = host();
    let relay = Relay::start(&host.home(), ORIGIN);
    let reply = relay.ask(&json!({ "op": "open", "entry": "nothing-like-it" }));
    assert!(reply["error"].is_string(), "{reply}");
    let reply = relay.ask(&json!({ "op": "run", "program": "calc.exe" }));
    assert_eq!(reply["error"], "unknown op");
}

#[test]
fn the_door_can_be_turned_off() {
    let config = format!("{}\n[host.native]\nenabled = false\n", agent_entry());
    let host = Host::start(&config, 1);
    let relay = Relay::start(&host.home(), ORIGIN);
    let reply = relay.ask(&json!({ "op": "hello" }));
    assert_eq!(reply["running"], true, "a host holds the home: {reply}");
    assert_eq!(reply["door"], false, "{reply}");
    assert!(reply.get("entries").is_none(), "{reply}");
}

/// The host's output ends when it is killed; this kills by process id, as a test that started it may.
fn kill(pid: u64) {
    let _ = if cfg!(windows) {
        Command::new("taskkill").args(["/F", "/PID", &pid.to_string()]).output()
    } else {
        Command::new("kill").arg(pid.to_string()).output()
    };
}

#[test]
fn start_runs_the_recorded_program_only_when_no_host_holds_the_home() {
    let scratch = Scratch::new();
    let home = scratch.0.join("home");
    let config = scratch.0.join("canoa.cfg");
    std::fs::write(&config, "").expect("a configuration");
    // The program a person registered: this same canoa, serving nothing.
    let install = Command::new(host_binary())
        .args(["native", "install", "--name", &format!("dev.boriken.canoa_test_{}", std::process::id())])
        .args(["--browser", "edge", "--profile"])
        .arg(scratch.0.join("profile"))
        .args(["--launch", host_binary(), "--", "serve", "--allow-empty", "--config"])
        .arg(&config)
        .env("COQUI_AGENTIC_PEER_HOME", &home)
        .output()
        .expect("native install runs");
    let name = format!("dev.boriken.canoa_test_{}", std::process::id());
    let cleanup = || {
        Command::new(host_binary())
            .args(["native", "uninstall", "--name", &name, "--profile"])
            .arg(scratch.0.join("profile"))
            .env("COQUI_AGENTIC_PEER_HOME", &home)
            .output()
            .expect("native uninstall runs")
    };
    if !install.status.success() {
        cleanup();
        panic!("{}", String::from_utf8_lossy(&install.stderr));
    }

    let relay = Relay::start(&home, ORIGIN);
    assert_eq!(relay.ask(&json!({ "op": "hello" }))["running"], false);
    let started = relay.ask(&json!({ "op": "start" }));
    let pid = started["started"].as_u64();
    let outcome = std::panic::catch_unwind(|| {
        let pid = pid.unwrap_or_else(|| panic!("start answered {started}"));
        let until = Instant::now() + WAIT;
        loop {
            let reply = relay.ask(&json!({ "op": "hello" }));
            if reply["running"] == true && reply.get("entries").is_some() {
                break;
            }
            assert!(Instant::now() < until, "the started host never came up: {reply}");
            std::thread::sleep(Duration::from_millis(200));
        }
        // A host holds the home now: a second start does nothing.
        let again = relay.ask(&json!({ "op": "start" }));
        assert!(again["error"].is_string() && again["started"].is_null(), "{again}");
        pid
    });
    if let Ok(pid) = &outcome {
        kill(*pid);
    } else if let Some(pid) = pid {
        kill(pid);
    }
    cleanup();
    outcome.expect("start brought the host up");
}

#[test]
fn start_without_a_recorded_program_starts_nothing() {
    let scratch = Scratch::new();
    let relay = Relay::start(&scratch.0.join("home"), ORIGIN);
    let reply = relay.ask(&json!({ "op": "start" }));
    assert!(reply["error"].is_string() && reply["started"].is_null(), "{reply}");
}

#[test]
fn install_status_and_uninstall_leave_nothing_behind() {
    let scratch = Scratch::new();
    let home = scratch.0.join("home");
    let name = format!("dev.boriken.canoa_test_{}_a", std::process::id());
    let profile = scratch.0.join("profile");
    let run = |args: &[&str]| {
        Command::new(host_binary())
            .arg("native")
            .args(args)
            .args(["--name", &name, "--profile"])
            .arg(&profile)
            .env("COQUI_AGENTIC_PEER_HOME", &home)
            .output()
            .expect("canoa native runs")
    };
    let status = |output: std::process::Output| -> Value {
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        serde_json::from_slice(&output.stdout).expect("a JSON object")
    };
    let mut installed = Command::new(host_binary());
    installed
        .args(["native", "install", "--name", &name, "--browser", "edge", "--profile"])
        .arg(&profile)
        .args(["--extension-id", &"b".repeat(32)])
        .env("COQUI_AGENTIC_PEER_HOME", &home);
    let installed = installed.output().expect("canoa native install runs");
    let outcome = std::panic::catch_unwind(|| {
        assert!(installed.status.success(), "{}", String::from_utf8_lossy(&installed.stderr));
        let wrote = String::from_utf8_lossy(&installed.stdout).into_owned();
        let after = status(run(&["status"]));
        assert_eq!(after["name"], name.as_str());
        assert!(!after["registered"].as_array().expect("a list").is_empty(), "{after}");
        assert_eq!(after["running"], false);
        assert!(after["launch"]["args"].as_array().is_some_and(|args| args.contains(&json!("serve"))), "{after}");
        let manifest = home.join("native").join(format!("{name}.json"));
        let profiled = profile.join("NativeMessagingHosts").join(format!("{name}.json"));
        let text = std::fs::read_to_string(if cfg!(windows) { &manifest } else { &profiled })
            .unwrap_or_else(|error| panic!("the manifest: {error}; wrote {wrote}"));
        let manifest: Value = serde_json::from_str(&text).expect("a manifest");
        assert_eq!(manifest["name"], name.as_str());
        assert_eq!(manifest["type"], "stdio");
        let origins = manifest["allowed_origins"].as_array().expect("origins");
        assert!(origins.contains(&json!(ORIGIN)), "the known extension: {origins:?}");
        assert!(origins.contains(&json!(format!("chrome-extension://{}/", "b".repeat(32)))));
        assert_eq!(origins.len(), 2);
        assert_eq!(
            Path::new(manifest["path"].as_str().expect("a path")).file_name(),
            Path::new(host_binary()).file_name()
        );
    });
    let removed = run(&["uninstall"]);
    assert!(removed.status.success());
    let gone = status(run(&["status"]));
    assert!(gone["registered"].as_array().expect("a list").is_empty(), "{gone}");
    assert!(gone["launch"].is_null(), "{gone}");
    outcome.expect("the registration was as asked");
}
