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
        Self::start_with(home, origin, &[])
    }

    fn start_with(home: &Path, origin: &str, env: &[(&str, &Path)]) -> Self {
        let mut child = Command::new(host_binary())
            .arg(origin)
            .envs(env.iter().copied())
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
        drop(
            self.stdin
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take(),
        );
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
        self.pieces
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
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
            let wait = until.map_or(WAIT, |until| {
                until.saturating_duration_since(Instant::now())
            });
            let messages = self
                .relay
                .messages
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            match messages.recv_timeout(wait) {
                Ok(message) if message.get("d").is_some() => {
                    let bytes = BASE64
                        .decode(message["d"].as_str().unwrap_or_default().as_bytes())
                        .map_err(|_| Failure::Protocol)?;
                    self.pieces
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .push(bytes.len());
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
    assert!(
        reply.get("home").is_none(),
        "a claim of a folder is not made: {reply}"
    );
}

#[test]
fn hello_names_this_computer_and_its_network_addresses() {
    let host = host();
    let relay = Relay::start(&host.home(), ORIGIN);
    let reply = relay.ask(&json!({ "op": "hello" }));
    let name = reply["hostname"].as_str().expect("a host name");
    assert!(!name.is_empty() && name == name.to_lowercase(), "{reply}");
    let addresses = reply["addresses"].as_array().expect("a list of addresses");
    assert!(addresses.len() <= 16, "{reply}");
    for address in addresses {
        let address: std::net::IpAddr = address
            .as_str()
            .and_then(|text| text.parse().ok())
            .unwrap_or_else(|| panic!("an address: {reply}"));
        assert!(
            !address.is_loopback() && !address.is_unspecified(),
            "{address}"
        );
    }
}

#[test]
fn a_stopped_hello_still_names_this_computer() {
    let scratch = Scratch::new();
    let relay = Relay::start(&scratch.0.join("home"), ORIGIN);
    let reply = relay.ask(&json!({ "op": "hello" }));
    assert_eq!(reply["running"], false, "{reply}");
    let name = reply["hostname"].as_str().expect("a host name");
    assert!(!name.is_empty(), "{reply}");
    assert!(reply["addresses"].is_array(), "{reply}");
    let names = reply["hostnames"]
        .as_array()
        .expect("the names DNS may know it by");
    assert!(names.iter().any(|known| known == name), "{reply}");
}

#[test]
fn no_host_running_answers_that_it_is_not() {
    let scratch = Scratch::new();
    let relay = Relay::start(&scratch.0.join("home"), ORIGIN);
    let reply = relay.ask(&json!({ "op": "hello" }));
    assert_eq!(reply["running"], false, "{reply}");
    assert!(reply.get("entries").is_none(), "{reply}");
    let refused = relay.ask(&json!({ "op": "open", "entry": "fixture" }));
    assert!(
        refused["error"].is_string() && refused["opened"].is_null(),
        "{refused}"
    );
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
        cabuya::auth::Method::WebCrypto => refused.is_err(),
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
        let result = support::with_channel(Spy { link: link_ref }, &client, host.peer, |rpc| {
            turn_spy(rpc, &dir, &big)
        });
        sizes.borrow_mut().extend(link_ref.sizes());
        result
    };
    let text = outcome
        .expect("a paired key is served")
        .expect("the turn ends");
    assert_eq!(
        text.len(),
        "echo: ".len() + big.len(),
        "the record arrived whole"
    );
    let sizes = sizes.into_inner();
    assert!(sizes.iter().all(|size| *size <= MAX_PIECE), "{sizes:?}");
    assert!(
        sizes.len() >= 4,
        "a 3 MiB record came in {} pieces",
        sizes.len()
    );
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
        Command::new("taskkill")
            .args(["/F", "/PID", &pid.to_string()])
            .output()
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
        .args([
            "native",
            "install",
            "--name",
            &format!("dev.boriken.canoa_test_{}", std::process::id()),
        ])
        .args(["--browser", "edge", "--profile"])
        .arg(scratch.0.join("profile"))
        .args([
            "--launch",
            host_binary(),
            "--",
            "serve",
            "--allow-empty",
            "--config",
        ])
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
            assert!(
                Instant::now() < until,
                "the started host never came up: {reply}"
            );
            std::thread::sleep(Duration::from_millis(200));
        }
        // A host holds the home now: a second start does nothing.
        let again = relay.ask(&json!({ "op": "start" }));
        assert!(
            again["error"].is_string() && again["started"].is_null(),
            "{again}"
        );
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
    assert!(
        reply["error"].is_string() && reply["started"].is_null(),
        "{reply}"
    );
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
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).expect("a JSON object")
    };
    let mut installed = Command::new(host_binary());
    installed
        .args([
            "native",
            "install",
            "--name",
            &name,
            "--browser",
            "edge",
            "--profile",
        ])
        .arg(&profile)
        .args(["--extension-id", &"b".repeat(32)])
        .env("COQUI_AGENTIC_PEER_HOME", &home);
    let installed = installed.output().expect("canoa native install runs");
    let outcome = std::panic::catch_unwind(|| {
        assert!(
            installed.status.success(),
            "{}",
            String::from_utf8_lossy(&installed.stderr)
        );
        let wrote = String::from_utf8_lossy(&installed.stdout).into_owned();
        let after = status(run(&["status"]));
        assert_eq!(after["name"], name.as_str());
        assert!(
            !after["registered"].as_array().expect("a list").is_empty(),
            "{after}"
        );
        assert_eq!(after["running"], false);
        assert!(
            after["launch"]["args"]
                .as_array()
                .is_some_and(|args| args.contains(&json!("serve"))),
            "{after}"
        );
        let manifest = home.join("native").join(format!("{name}.json"));
        let profiled = profile
            .join("NativeMessagingHosts")
            .join(format!("{name}.json"));
        let text = std::fs::read_to_string(if cfg!(windows) { &manifest } else { &profiled })
            .unwrap_or_else(|error| panic!("the manifest: {error}; wrote {wrote}"));
        let manifest: Value = serde_json::from_str(&text).expect("a manifest");
        assert_eq!(manifest["name"], name.as_str());
        assert_eq!(manifest["type"], "stdio");
        let origins = manifest["allowed_origins"].as_array().expect("origins");
        assert!(
            origins.contains(&json!(ORIGIN)),
            "the known extension: {origins:?}"
        );
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
    assert!(
        gone["registered"].as_array().expect("a list").is_empty(),
        "{gone}"
    );
    assert!(gone["launch"].is_null(), "{gone}");
    outcome.expect("the registration was as asked");
}

/// `canoa pair --native` on the host's home: its first line is the PIN it shows.
fn pairing_door(home: &Path) -> (Child, Value) {
    use std::io::BufRead as _;
    let mut child = Command::new(host_binary())
        .args([
            "pair",
            "--native",
            "--timeout-seconds",
            "120",
            "--method",
            support::method().name(),
        ])
        .env("COQUI_AGENTIC_PEER_HOME", home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("pair --native starts");
    let mut line = String::new();
    std::io::BufReader::new(child.stdout.as_mut().expect("its output"))
        .read_line(&mut line)
        .expect("it shows a PIN");
    let shown: Value = serde_json::from_str(&line).expect("one JSON object");
    (child, shown)
}

#[test]
fn a_browser_pairs_by_pin_through_the_relay_and_its_key_is_bound_to_its_extension() {
    use cabuya::pairing;
    use coqui_peer_webcrypto::pair::{CodeAlphabet, Pin};

    let host = host();
    let (mut pair, shown) = pairing_door(&host.home());
    assert_eq!(shown["native"], true, "{shown}");
    assert_eq!(shown["peer"], host.peer.to_string().as_str());
    let pin =
        Pin::parse(&CodeAlphabet::pin(), shown["pin"].as_str().expect("a PIN")).expect("a PIN");

    let client = support::key();
    let client_store = canoa_host::Home::at(host.scratch.0.join("client"))
        .store()
        .expect("a store");
    let relay = Relay::start(&host.home(), ORIGIN);
    let link = Link::open(&relay, "@pair").expect("the pairing door opens while a PIN is shown");
    let joined = pairing::join(&link, &client, pin, &client_store, WAIT).expect("the PIN pairs");
    link.close();
    assert_eq!(
        joined.peer.id(),
        host.peer,
        "the browser learned the host's key by the PIN, not from hello"
    );
    assert!(
        pair.wait().expect("pair ends").success(),
        "pairing ends once it paired"
    );

    // Served to the extension that paired it...
    let dir = cwd(&host.scratch);
    let relay = Relay::start(&host.home(), ORIGIN);
    let link = Link::open(&relay, "fixture").expect("the door opens the entry");
    let text = support::with_channel(link, &client, host.peer, |rpc| turn(rpc, &dir, "paired"))
        .expect("the paired key is served")
        .expect("the turn ends");
    assert_eq!(text, "echo: paired");

    // ...and to no other origin.
    let relay = Relay::start(
        &host.home(),
        "chrome-extension://pppppppppppppppppppppppppppppppp/",
    );
    let link = Link::open(&relay, "fixture").expect("the door opens the entry");
    let refused = support::with_channel(link, &client, host.peer, |rpc| rpc.initialize());
    let served = matches!(refused, Ok(Ok(_)));
    assert!(
        !served,
        "a key bound to one extension is turned away from another: {refused:?}"
    );
}

#[test]
fn the_pairing_door_is_closed_unless_a_person_asked_for_a_pin() {
    let host = host();
    let relay = Relay::start(&host.home(), ORIGIN);
    let refused = Link::open(&relay, "@pair")
        .err()
        .expect("nothing waits to pair");
    assert!(
        refused["error"]
            .as_str()
            .is_some_and(|error| error.contains("PIN")),
        "{refused}"
    );
    // And the host's own door does not pair: it has no PIN to give.
    let again = Relay::start(&host.home(), ORIGIN);
    let hello = again.ask(&json!({ "op": "hello" }));
    assert_eq!(hello["running"], true);
}

#[test]
fn a_wrong_pin_pairs_nobody() {
    use cabuya::pairing;
    use coqui_peer_webcrypto::pair::{CodeAlphabet, Pin};

    let host = host();
    let (mut pair, shown) = pairing_door(&host.home());
    let right = shown["pin"].as_str().expect("a PIN");
    let mut wrong = right.to_owned();
    let last = wrong.pop().expect("a PIN has characters");
    wrong.push(if last == '0' { '1' } else { '0' });
    let pin = Pin::parse(&CodeAlphabet::pin(), &wrong).expect("a PIN of the same alphabet");
    let client = support::key();
    let client_store = canoa_host::Home::at(host.scratch.0.join("client"))
        .store()
        .expect("a store");
    let relay = Relay::start(&host.home(), ORIGIN);
    let link = Link::open(&relay, "@pair").expect("the pairing door opens");
    let joined = pairing::join(&link, &client, pin, &client_store, Duration::from_secs(10));
    link.close();
    let _ = pair.kill();
    let _ = pair.wait();
    assert!(joined.is_err(), "a wrong PIN does not pair");
    let store = canoa_host::Home::at(host.home())
        .store()
        .expect("the host's store");
    assert!(!store.is_accepted(&client.peer_id()), "nobody was paired");
}

/// Runs `body` on the control channel through the door, as a key the host has paired.
fn on_control<R>(
    host: &Host,
    client: &cabuya::auth::Identity,
    body: impl FnOnce(&support::Rpc<'_, Link<'_>>) -> R,
) -> Result<R, String> {
    let relay = Relay::start(&host.home(), ORIGIN);
    let link = Link::open(&relay, "@control").map_err(|error| error.to_string())?;
    support::with_channel(link, client, host.peer, body)
}

#[test]
fn control_through_the_door_refuses_what_installs_restarts_or_belongs_to_the_owner() {
    let host = host();
    let client = support::key();
    host.pair(&client);
    let refused = on_control(&host, &client, |rpc| {
        let listed = rpc.request("ledger/list", json!({}));
        let denied: Vec<(&str, Option<i64>)> = [
            ("overlay/apply", json!({})),
            ("overlay/remove", json!({"name": "x"})),
            ("catalog/fetch", json!({})),
            ("host/restart", json!({})),
            ("upload/put", json!({"name": "x", "data": ""})),
            (
                "config/set",
                json!({"section": "permissions", "value": {"protected": []}}),
            ),
            ("config/set", json!({"section": "overlays", "value": {}})),
            ("accounts/list", json!({})),
            ("store/rollback", json!({})),
            ("history/export", json!({})),
        ]
        .into_iter()
        .map(|(method, params)| {
            let said = rpc.request(method, params);
            (method, said.err().and_then(|error| error["code"].as_i64()))
        })
        .collect();
        (listed.is_ok(), denied)
    })
    .expect("a paired key reaches the control channel");
    assert!(refused.0, "its own ledger is the one thing it may read");
    for (method, code) in refused.1 {
        assert!(matches!(code, Some(-32001 | -32002)), "{method}: {code:?}");
    }
}

#[test]
fn a_key_forgets_itself_over_the_door_and_is_refused_after() {
    let host = host();
    let client = support::key();
    host.pair(&client);
    let said = on_control(&host, &client, |rpc| rpc.request("peer/forget", json!({})))
        .expect("a paired key reaches the control channel");
    assert_eq!(said.expect("it is forgotten")["removed"], true);
    let store = canoa_host::Home::at(host.home())
        .store()
        .expect("the host's store");
    assert!(
        !store.is_accepted(&client.peer_id()),
        "the host no longer pairs it"
    );
    let relay = Relay::start(&host.home(), ORIGIN);
    let link = Link::open(&relay, "fixture").expect("the door opens the entry");
    let again = support::with_channel(link, &client, host.peer, |rpc| rpc.initialize());
    assert!(!matches!(again, Ok(Ok(_))), "{again:?}");
}

#[test]
fn an_unreadable_record_of_origins_closes_the_door_to_every_key() {
    let host = host();
    let client = support::key();
    host.pair(&client);
    std::fs::create_dir_all(host.home().join("native")).expect("a folder");
    std::fs::write(
        host.home().join("native").join("origins.json"),
        "{ not json",
    )
    .expect("a corrupt file");
    let relay = Relay::start(&host.home(), ORIGIN);
    let link = Link::open(&relay, "fixture").expect("the door opens the entry");
    let served = support::with_channel(link, &client, host.peer, |rpc| rpc.initialize());
    assert!(
        !matches!(served, Ok(Ok(_))),
        "a key was served with the record of origins unreadable: {served:?}"
    );
}

#[test]
fn pairing_that_cannot_record_its_origin_leaves_no_accepted_key() {
    use cabuya::pairing;
    use coqui_peer_webcrypto::pair::{CodeAlphabet, Pin};

    let host = host();
    // A folder where the record belongs: it can be neither read nor replaced.
    std::fs::create_dir_all(host.home().join("native").join("origins.json"))
        .expect("a folder in its place");
    let (mut pair, shown) = pairing_door(&host.home());
    let pin =
        Pin::parse(&CodeAlphabet::pin(), shown["pin"].as_str().expect("a PIN")).expect("a PIN");
    let client = support::key();
    let client_store = canoa_host::Home::at(host.scratch.0.join("client"))
        .store()
        .expect("a store");
    let relay = Relay::start(&host.home(), ORIGIN);
    let link = Link::open(&relay, "@pair").expect("the pairing door opens");
    let _ = pairing::join(&link, &client, pin, &client_store, WAIT);
    link.close();
    assert!(
        !pair.wait().expect("pair ends").success(),
        "pairing reports what it could not record"
    );
    let store = canoa_host::Home::at(host.home())
        .store()
        .expect("the host's store");
    assert!(
        !store.is_accepted(&client.peer_id()),
        "a key was left accepted with no origin bound to it"
    );
}

#[test]
fn a_door_that_could_not_be_opened_is_said_to_the_browser_and_in_status() {
    let scratch = Scratch::new();
    let home = scratch.0.join("home");
    std::fs::create_dir_all(&home).expect("a home");
    // Something else holds the door's name first.
    let name = canoa_host::native::door_name(&home).expect("a name");
    let _squatter = canoa_host::pipe::Listener::bind(&name).expect("the name is free");
    let host = Host::start_in(scratch, &agent_entry(), 1, &[]);
    let relay = Relay::start(&host.home(), ORIGIN);
    let reply = relay.ask(&json!({ "op": "hello" }));
    assert_eq!(reply["running"], true, "{reply}");
    assert_eq!(reply["door"], false, "{reply}");
    assert!(
        reply["why"]
            .as_str()
            .is_some_and(|why| why.contains("door")),
        "{reply}"
    );
    let status = host.command(&["native", "status"]);
    let status: Value = serde_json::from_slice(&status.stdout).expect("a JSON object");
    assert!(status["door_error"].is_string(), "{status}");
}

const NEARBY_PEER: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

/// A descriptor directory naming an agent that is listening on loopback, as `serve --advertise` leaves one.
struct Nearby {
    dir: Scratch,
    _listener: std::net::TcpListener,
}

impl Nearby {
    fn new() -> Self {
        let dir = Scratch::new();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a loopback listener");
        let port = listener.local_addr().expect("its address").port();
        let descriptor = json!({
            "txtvers": 1,
            "proto": "acp",
            "label": "Nearby fixture",
            "url": format!("ws://127.0.0.1:{port}/acp"),
            "peer": NEARBY_PEER,
            "method": "noise",
        });
        std::fs::write(dir.0.join("nearby.json"), descriptor.to_string()).expect("a descriptor");
        Self {
            dir,
            _listener: listener,
        }
    }

    fn path(&self) -> &Path {
        &self.dir.0
    }

    fn port(&self) -> u16 {
        self._listener.local_addr().expect("its address").port()
    }
}

fn found(result: &Value, nearby: &Nearby) {
    let list = result["candidates"]
        .as_array()
        .expect("a list of candidates");
    let candidate = list
        .iter()
        .find(|candidate| candidate["label"] == "Nearby fixture")
        .unwrap_or_else(|| panic!("the descriptor's agent is listed: {result}"));
    assert_eq!(
        candidate["url"],
        format!("ws://127.0.0.1:{}/acp", nearby.port()).as_str()
    );
    assert_eq!(candidate["peer"], NEARBY_PEER);
    assert_eq!(candidate["method"], "noise");
    assert_eq!(candidate["source"], "local");
    // A folder named by the override is this user's own on Unix, and not shown to be on Windows.
    if cfg!(windows) {
        assert_eq!(result["directory"], "unchecked", "{result}");
    } else {
        assert!(result.get("directory").is_none(), "{result}");
    }
}

#[test]
fn discover_nearby_over_the_door_lists_the_agents_this_machine_advertises() {
    let nearby = Nearby::new();
    let host = Host::start_with(
        &agent_entry(),
        1,
        &[],
        &[("COQUI_ACP_DESCRIPTORS", &nearby.path().to_string_lossy())],
    );
    let client = support::key();
    host.pair(&client);
    let result = on_control(&host, &client, |rpc| {
        rpc.request("discover/nearby", json!({ "seconds": 1 }))
    })
    .expect("a paired key reaches the control channel")
    .expect("the default grants include discovery");
    found(&result, &nearby);
}

/// Gives a directory to another user through `sudo`, and back to this one when dropped so the scratch folder can be removed.
#[cfg(unix)]
struct Given<'a> {
    dir: &'a Path,
    back: String,
}

#[cfg(unix)]
impl<'a> Given<'a> {
    /// `None` when this process cannot change an owner without a password.
    fn to(dir: &'a Path, user: &str) -> Option<Self> {
        let me = Command::new("id").arg("-u").output().ok()?;
        let me = format!(
            "{}:{}",
            String::from_utf8_lossy(&me.stdout).trim(),
            String::from_utf8_lossy(&Command::new("id").arg("-g").output().ok()?.stdout).trim()
        );
        let given = Command::new("sudo")
            .args(["-n", "chown", user])
            .arg(dir)
            .status()
            .ok()?
            .success();
        given.then_some(Self { dir, back: me })
    }
}

#[cfg(unix)]
impl Drop for Given<'_> {
    fn drop(&mut self) {
        let _ = Command::new("sudo")
            .args(["-n", "chown", &self.back])
            .arg(self.dir)
            .status();
    }
}

#[cfg(unix)]
#[test]
#[ignore = "needs passwordless sudo to change a folder's owner (sudo -n chown)"]
fn discover_nearby_reports_a_descriptor_folder_another_user_owns_as_shared() {
    let nearby = Nearby::new();
    let _given = Given::to(nearby.path(), "65534:65534")
        .expect("this process can change a folder's owner with sudo -n chown");
    let host = Host::start_with(
        &agent_entry(),
        1,
        &[],
        &[("COQUI_ACP_DESCRIPTORS", &nearby.path().to_string_lossy())],
    );
    let client = support::key();
    host.pair(&client);
    let result = on_control(&host, &client, |rpc| {
        rpc.request("discover/nearby", json!({ "seconds": 1 }))
    })
    .expect("a paired key reaches the control channel")
    .expect("the default grants include discovery");
    assert_eq!(result["directory"], "shared", "{result}");
    let listed = result["candidates"].as_array().expect("a list");
    assert!(
        listed.iter().all(|c| c["label"] != "Nearby fixture"),
        "no descriptor of a folder another user owns is read: {result}"
    );
}

#[test]
fn discover_nearby_is_refused_to_a_key_without_the_grant_and_a_seconds_that_is_not_a_number() {
    let nearby = Nearby::new();
    let config = format!(
        "{}\n[host.native]\ncontrol = [\"ledger.read\"]\n",
        agent_entry()
    );
    let host = Host::start_with(
        &config,
        1,
        &[],
        &[("COQUI_ACP_DESCRIPTORS", &nearby.path().to_string_lossy())],
    );
    let client = support::key();
    host.pair(&client);
    let refused = on_control(&host, &client, |rpc| {
        rpc.request("discover/nearby", json!({ "seconds": 1 }))
    })
    .expect("a paired key reaches the control channel")
    .expect_err("no discover grant, no search");
    assert_eq!(refused["code"], -32001, "{refused}");

    let allowed = Host::start_with(
        &agent_entry(),
        1,
        &[],
        &[("COQUI_ACP_DESCRIPTORS", &nearby.path().to_string_lossy())],
    );
    allowed.pair(&client);
    let bad = on_control(&allowed, &client, |rpc| {
        rpc.request("discover/nearby", json!({ "seconds": "soon" }))
    })
    .expect("a paired key reaches the control channel")
    .expect_err("seconds is a whole number");
    assert_eq!(bad["code"], -32602, "{bad}");
}

#[test]
fn the_relay_discovers_with_no_host_running() {
    let nearby = Nearby::new();
    let scratch = Scratch::new();
    let relay = Relay::start_with(
        &scratch.0.join("home"),
        ORIGIN,
        &[("COQUI_ACP_DESCRIPTORS", nearby.path())],
    );
    let hello = relay.ask(&json!({ "op": "hello" }));
    assert_eq!(hello["running"], false, "no host runs: {hello}");
    let reply = relay.ask(&json!({ "op": "discover", "seconds": 1 }));
    assert_eq!(reply["reply"], "discover", "{reply}");
    found(&reply, &nearby);
}

const SITE: &str = "dev.boriken.canoa/site";

/// What a provider is for: answers one stream's MCP messages as the extension does, from a grant it keeps for that stream.
fn provide_until(
    rpc: &support::Rpc<'_, Link<'_>>,
    done: &std::sync::atomic::AtomicBool,
    seen: &mut Vec<Value>,
) {
    use std::sync::atomic::Ordering;
    let mut granted = std::collections::HashSet::new();
    loop {
        let Ok(message) = rpc.next_message(Duration::from_millis(200)) else {
            if done.load(Ordering::SeqCst) {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
            continue;
        };
        seen.push(message.clone());
        let stream = message["stream"].as_str().unwrap_or_default().to_owned();
        if message["type"] != "mcp" {
            continue;
        }
        let (id, method) = (&message["message"]["id"], &message["message"]["method"]);
        let result = match method.as_str() {
            Some("tools/list") => {
                let mut tools = vec![
                    json!({"name": "browser_request_access", "inputSchema": {"type": "object"}}),
                ];
                if granted.contains(&stream) {
                    tools.push(
                        json!({"name": "browser_read_page", "inputSchema": {"type": "object"}}),
                    );
                }
                json!({"tools": tools})
            }
            Some("tools/call")
                if message["message"]["params"]["name"] == "browser_request_access" =>
            {
                granted.insert(stream.clone());
                json!({"isError": false, "content": [{"type": "text", "text": "allowed"}]})
            }
            _ => json!({"isError": true, "content": [{"type": "text", "text": "denied"}]}),
        };
        let _ = rpc.send(&json!({"type": "mcp", "stream": stream,
            "message": {"jsonrpc": "2.0", "id": id, "result": result}}));
    }
}

/// Pairs a new key the way the extension does: by the PIN `canoa pair --native` shows, which binds the key to `ORIGIN`.
fn pair_by_pin(host: &Host) -> cabuya::auth::Identity {
    use cabuya::pairing;
    use coqui_peer_webcrypto::pair::{CodeAlphabet, Pin};

    let (mut pair, shown) = pairing_door(&host.home());
    let pin =
        Pin::parse(&CodeAlphabet::pin(), shown["pin"].as_str().expect("a PIN")).expect("a PIN");
    let client = support::key();
    let client_store = canoa_host::Home::at(host.scratch.0.join("client"))
        .store()
        .expect("a store");
    let relay = Relay::start(&host.home(), ORIGIN);
    let link = Link::open(&relay, "@pair").expect("the pairing door opens while a PIN is shown");
    pairing::join(&link, &client, pin, &client_store, WAIT).expect("the PIN pairs");
    link.close();
    assert!(pair.wait().expect("pair ends").success());
    client
}

#[test]
fn a_browser_offers_its_tools_through_the_door_to_every_session_but_a_pages() {
    use std::sync::atomic::{AtomicBool, Ordering};

    let scratch = Scratch::new();
    let documents = scratch.0.join("documents");
    let config = format!("{}args = [\"--scripted\", \"--mcp-http\"]\n", agent_entry());
    let host = Host::start_in(
        scratch,
        &config,
        1,
        &[("XDG_DOCUMENTS_DIR", documents.into_os_string())],
    );
    let client = pair_by_pin(&host);
    let (home, peer) = (host.home(), host.peer);
    let dir = cwd(&host.scratch);

    let relay = Relay::start(&home, ORIGIN);
    let link = Link::open(&relay, "@tools").expect("the door opens @tools");
    let done = AtomicBool::new(false);
    let mut seen = Vec::new();
    let told = support::with_channel(link, &client, peer, |rpc| {
        rpc.send(&json!({"type": "register", "provider": "browser"}))
            .expect("sent");
        let registered = rpc.next_message(WAIT).expect("the host answers");
        assert_eq!(registered["type"], "registered", "{registered}");
        std::thread::scope(|scope| {
            let session = scope.spawn(|| {
                let relay = Relay::start(&home, ORIGIN);
                let link = Link::open(&relay, "fixture").expect("the door opens the entry");
                let told = support::with_channel(link, &client.clone(), peer, |rpc| {
                    rpc.initialize().expect("initialize");
                    let new = |meta: Value| {
                        let mut params = json!({"cwd": dir, "mcpServers": []});
                        if !meta.is_null() {
                            params["_meta"] = json!({ SITE: meta });
                        }
                        rpc.request("session/new", params).expect("a session")["sessionId"]
                            .as_str()
                            .expect("an id")
                            .to_owned()
                    };
                    let (own, page, extension) = (
                        new(Value::Null),
                        new(json!({"site": "example.com"})),
                        new(json!({"site": "canoa-extension", "own": true})),
                    );
                    let listed = rpc.prompt(&own, "ListOn canoa-browser").expect("a turn");
                    let page = rpc
                        .prompt(&page, "ServerBearer canoa-browser")
                        .expect("a turn");
                    let extension = rpc
                        .prompt(&extension, "ListOn canoa-browser")
                        .expect("a turn");
                    (listed, page, extension)
                })
                .expect("the paired key is served");
                done.store(true, Ordering::SeqCst);
                told
            });
            provide_until(rpc, &done, &mut seen);
            session.join().expect("the session ran")
        })
    })
    .expect("the browser's key is served");
    let (listed, page, extension) = told;
    assert_eq!(
        listed, "browser_request_access",
        "a paired key needs no grant to use them"
    );
    assert_eq!(
        page, "no bearer",
        "a session a page started is given nothing"
    );
    assert_eq!(
        extension, "browser_request_access",
        "the extension's own pages are the person's own"
    );
    let opened: Vec<&Value> = seen
        .iter()
        .filter(|message| message["type"] == "open")
        .collect();
    assert_eq!(
        opened.len(),
        2,
        "one stream for each session that was given the server: {seen:?}"
    );
    for open in opened {
        assert_eq!(
            open["session"],
            json!({"agent": "fixture", "via": "paired", "peer": client.peer_id().to_string()})
        );
    }
}

#[test]
fn a_browser_key_whose_door_gives_it_no_tools_provide_is_refused() {
    let config = format!(
        "{}[host.native]\ncontrol = [\"ledger.read\"]\n",
        agent_entry()
    );
    let host = Host::start(&config, 1);
    let client = pair_by_pin(&host);
    let relay = Relay::start(&host.home(), ORIGIN);
    let link = Link::open(&relay, "@tools").expect("the door opens @tools");
    let refused = support::with_channel(link, &client, host.peer, |rpc| {
        rpc.send(&json!({"type": "register", "provider": "browser"}))
            .expect("sent");
        rpc.next_message(WAIT).expect("the host answers")
    })
    .expect("the key is served");
    assert_eq!(refused["type"], "refused", "{refused}");
}

#[test]
fn a_key_paired_another_way_cannot_pose_as_the_browser_at_the_tools_door() {
    let host = host();
    let client = support::key();
    host.pair(&client);
    let relay = Relay::start(&host.home(), ORIGIN);
    let link = Link::open(&relay, "@tools").expect("the door opens @tools");
    let answered = support::with_channel(link, &client, host.peer, |rpc| {
        rpc.send(&json!({"type": "register", "provider": "browser"}))
            .ok()?;
        rpc.next_message(WAIT).ok()
    });
    assert!(
        !matches!(&answered, Ok(Some(message)) if message["type"] == "registered"),
        "a key no extension's PIN bound is turned away: {answered:?}"
    );
    // The same key is still served an entry, as `canoa accept` meant.
    let relay = Relay::start(&host.home(), ORIGIN);
    let link = Link::open(&relay, "fixture").expect("the door opens the entry");
    let dir = cwd(&host.scratch);
    let text = support::with_channel(link, &client, host.peer, |rpc| turn(rpc, &dir, "still"))
        .expect("served")
        .expect("a turn");
    assert_eq!(text, "echo: still");
}
