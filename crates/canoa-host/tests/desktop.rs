//! What Canoa's desktop app relies on when it runs the host as its child: the host ends with its input, and every agent it
//! started with it, and a native key pairs by a PIN the app shows.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod support;

use std::io::{BufRead as _, BufReader};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use coqui_agentic_peer::{auth, pairing};
use coqui_peer_webcrypto::pair::{CodeAlphabet, Pin};
use coqui_peer_webcrypto::{Identity, TrustStore};
use coqui_web::Dialer as _;
use coqui_web::websocket::{self, Frames};
use coqui_web::{Channel as _, Kind, Limits};
use serde_json::{Value, json};
use support::{Host, Scratch, WAIT, example, host_binary, quoted, running, with_link};

fn agent_entry() -> String {
    format!(
        "[host.agents.fixture]\nprogram = {}\n",
        quoted(&example("stdio_agent", false).to_string_lossy())
    )
}

type Link = std::thread::JoinHandle<Result<Result<Value, String>, String>>;

/// A host serving the fixture with `args`, a paired link to it held open, and the pid of the agent serving that link. The
/// agent stays after its own input closes, so only something else ending it can end it.
fn agent_running(args: &[&str]) -> (Host, u32, Link) {
    let host = Host::start_with(&agent_entry(), 1, args, &[("CANOA_TEST_AGENT_STAYS", "1")]);
    let client = Identity::generate().expect("a key");
    host.pair(&client);
    let (pids, pid) = mpsc::channel();
    let (url, peer) = (host.url("fixture").to_owned(), host.peer);
    let link = std::thread::spawn(move || {
        with_link(&url, &client, peer, |rpc| {
            rpc.initialize().expect("initialize is answered");
            let said = rpc
                .request("_fixture/pid", json!({}))
                .expect("the agent answers");
            pids.send(said["pid"].as_u64().expect("a pid"))
                .expect("the test waits");
            // Held until the host ends it.
            rpc.next_message(Duration::from_secs(30))
        })
    });
    let pid = u32::try_from(pid.recv_timeout(WAIT).expect("the agent's pid")).expect("a pid");
    assert!(running(pid), "the agent runs while the host does");
    (host, pid, link)
}

fn ends_within_five_seconds(pid: u32) -> bool {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while running(pid) && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    !running(pid)
}

#[test]
fn closing_its_input_ends_the_host_and_every_agent_it_started() {
    let (mut host, pid, link) = agent_running(&["--until-stdin-closes"]);
    host.close_input();
    let status = host
        .ended_within(Duration::from_secs(10))
        .expect("the host ends when its input closes");
    assert!(status.success(), "{status:?}");
    let ended = link.join().expect("the link's thread");
    assert!(
        matches!(ended, Ok(Err(_))),
        "the link ends with the host: {ended:?}"
    );
    assert!(
        ends_within_five_seconds(pid),
        "the agent ended with the host"
    );
}

/// On Windows each agent is in a job object the host holds, which the system closes with the host however it ends.
#[cfg(windows)]
#[test]
fn killing_the_host_on_windows_ends_its_agents() {
    let (mut host, pid, link) = agent_running(&[]);
    host.kill();
    let _ = link.join();
    assert!(
        ends_within_five_seconds(pid),
        "the agent ended with the host"
    );
}

#[test]
fn without_the_flag_closing_its_input_leaves_the_host_serving() {
    let mut host = Host::start(&agent_entry(), 1);
    host.close_input();
    assert!(
        host.ended_within(Duration::from_secs(2)).is_none(),
        "only the desktop app's flag ties the host to its input"
    );
}

/// `canoa-host pair` on a home of its own: what it printed first, and the process.
fn start_pairing(
    home: &std::path::Path,
    args: &[&str],
) -> (Value, std::process::Child, mpsc::Receiver<String>) {
    let mut child = Command::new(host_binary())
        .arg("pair")
        .args(args)
        .env("COQUI_AGENTIC_PEER_HOME", home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("pair starts");
    let stdout = child.stdout.take().expect("its output");
    let (lines, received) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { return };
            if lines.send(line).is_err() {
                return;
            }
        }
    });
    let first = received.recv_timeout(WAIT).expect("pair says its PIN");
    (
        serde_json::from_str(&first).expect("one JSON object"),
        child,
        received,
    )
}

/// Joins a pairing at `url` with `pin`, as `coqui-acp-bridge pair-dial` does.
fn join(url: &str, identity: &Identity, pin: &str, store: &TrustStore) -> Result<String, String> {
    let channel = websocket::Dialer::new()
        .with_frames(Frames::Binary)
        .with_limits(Limits::default().with_max_message(auth::MAX_PROOF_MESSAGE))
        .dial(Kind::WebSocket, &url::Url::parse(url).expect("a URL"), WAIT)
        .map_err(|failure| failure.to_string())?;
    let pin = Pin::parse(&CodeAlphabet::pin(), pin).map_err(|_| "not a PIN".to_owned())?;
    let paired =
        pairing::join(&channel, identity, pin, store, WAIT).map_err(|error| error.to_string());
    channel.close();
    paired.map(|paired| paired.peer.id().to_string())
}

fn accepted(home: &std::path::Path) -> Vec<String> {
    let listed = Command::new(host_binary())
        .args(["peers", "--format", "json"])
        .env("COQUI_AGENTIC_PEER_HOME", home)
        .output()
        .expect("peers runs");
    String::from_utf8_lossy(&listed.stdout)
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("JSON"))
        .filter(|peer| peer["state"] == "accepted")
        .map(|peer| peer["peer"].as_str().expect("an id").to_owned())
        .collect()
}

#[test]
fn pair_shows_a_pin_and_accepts_the_key_that_uses_it() {
    let scratch = Scratch::new();
    let home = scratch.0.join("home");
    let (shown, mut child, lines) = start_pairing(&home, &["--timeout-seconds", "60"]);
    let pin = shown["pin"].as_str().expect("a PIN");
    assert_eq!(pin.len(), 6, "{shown}");
    let url = shown["url"].as_str().expect("a URL");
    assert!(url.starts_with("ws://127.0.0.1:"), "{shown}");

    let joiner = Identity::generate().expect("a key");
    let joiner_store = TrustStore::open(&scratch.0.join("joiner")).expect("a store");
    let host = join(url, &joiner, pin, &joiner_store).expect("the PIN pairs");
    assert_eq!(host, shown["peer"].as_str().expect("the host's id"));
    let said: Value =
        serde_json::from_str(&lines.recv_timeout(WAIT).expect("pair says who paired"))
            .expect("JSON");
    assert_eq!(said, json!({"paired": joiner.peer_id().to_string()}));
    assert!(child.wait().expect("pair ends").success());
    assert_eq!(accepted(&home), [joiner.peer_id().to_string()]);
}

#[test]
fn one_wrong_pin_ends_the_pairing_and_accepts_nobody() {
    let scratch = Scratch::new();
    let home = scratch.0.join("home");
    let (shown, mut child, _lines) = start_pairing(&home, &["--timeout-seconds", "60"]);
    let pin = shown["pin"].as_str().expect("a PIN");
    let wrong = if pin == "000000" { "111111" } else { "000000" };
    let joiner = Identity::generate().expect("a key");
    let joiner_store = TrustStore::open(&scratch.0.join("joiner")).expect("a store");
    let url = shown["url"].as_str().expect("a URL");
    assert!(join(url, &joiner, wrong, &joiner_store).is_err());
    let status = child.wait().expect("pair ends");
    assert_eq!(status.code(), Some(1), "{status:?}");
    assert!(accepted(&home).is_empty());
    assert!(
        join(url, &joiner, pin, &joiner_store).is_err(),
        "the right PIN after a wrong one finds nobody"
    );
}

/// The status line a WebSocket upgrade to `url` gets, sent with `origin` as a browser's page would send it.
fn upgrade_status(url: &str, origin: Option<&str>) -> String {
    use std::io::{Read as _, Write as _};
    let address = url.trim_start_matches("ws://");
    let mut stream = std::net::TcpStream::connect(address).expect("pair listens");
    stream.set_read_timeout(Some(WAIT)).expect("a timeout");
    let origin = origin.map_or(String::new(), |origin| format!("Origin: {origin}\r\n"));
    write!(
        stream,
        "GET / HTTP/1.1\r\nHost: {address}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n{origin}\r\n"
    )
    .expect("the request is sent");
    let mut answer = [0_u8; 64];
    let read = stream.read(&mut answer).unwrap_or(0);
    String::from_utf8_lossy(&answer[..read])
        .lines()
        .next()
        .unwrap_or("")
        .to_owned()
}

#[test]
fn a_web_page_cannot_reach_pair_and_costs_the_pin_nothing() {
    let scratch = Scratch::new();
    let home = scratch.0.join("home");
    let (shown, mut child, _lines) = start_pairing(&home, &["--timeout-seconds", "60"]);
    let url = shown["url"].as_str().expect("a URL");
    let page = upgrade_status(url, Some("https://evil.example"));
    assert!(
        page.contains(" 403 "),
        "a page's upgrade is refused: {page}"
    );
    let opaque = upgrade_status(url, Some("null"));
    assert!(opaque.contains(" 403 "), "{opaque}");

    let joiner = Identity::generate().expect("a key");
    let joiner_store = TrustStore::open(&scratch.0.join("joiner")).expect("a store");
    let pin = shown["pin"].as_str().expect("a PIN");
    join(url, &joiner, pin, &joiner_store).expect("the PIN still pairs a native client");
    assert!(child.wait().expect("pair ends").success());
}

#[test]
fn pair_tied_to_its_input_ends_when_it_closes() {
    let scratch = Scratch::new();
    let mut child = Command::new(host_binary())
        .args(["pair", "--until-stdin-closes"])
        .env("COQUI_AGENTIC_PEER_HOME", scratch.0.join("home"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("pair starts");
    let mut first = String::new();
    BufReader::new(child.stdout.take().expect("its output"))
        .read_line(&mut first)
        .expect("pair says its PIN");
    assert!(first.contains("\"pin\""), "{first}");
    drop(child.stdin.take());
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while child.try_wait().expect("its status").is_none() {
        assert!(
            std::time::Instant::now() < deadline,
            "pair outlived its input"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn pair_listens_only_on_loopback() {
    let scratch = Scratch::new();
    let refused = Command::new(host_binary())
        .args(["pair", "--listen", "0.0.0.0:0"])
        .env("COQUI_AGENTIC_PEER_HOME", scratch.0.join("home"))
        .output()
        .expect("pair runs");
    assert_eq!(refused.status.code(), Some(2), "{refused:?}");
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("loopback"),
        "{refused:?}"
    );
}
