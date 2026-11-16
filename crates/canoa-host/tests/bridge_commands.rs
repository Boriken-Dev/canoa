//! The commands `canoa` runs from the bridge library, as a person types them: `id --identity`, `grant`, `discover`, `dial`,
//! `forward`, `pair --origin` and `--join`, `mcp-config` and `serve --program`. The library's own suite proves what each does;
//! these show that the program reaches it, on its own home, with its flags.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod support;

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::Value;
use support::{Scratch, WAIT, example, host_binary};

fn canoa(home: &Path) -> Command {
    let mut command = Command::new(host_binary());
    command
        .env("COQUI_AGENTIC_PEER_HOME", home)
        .env("COQUI_ACP_DESCRIPTORS", home.join("descriptors"))
        .env_remove("CANOA_CONFIG");
    command
}

fn run(command: &mut Command) -> Output {
    command.output().expect("canoa runs")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim().to_owned()
}

/// A running program whose standard error is read line by line.
struct Running {
    child: Child,
    lines: mpsc::Receiver<String>,
    said: Vec<String>,
}

impl Running {
    fn start(mut command: Command) -> Self {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("canoa starts");
        let (send, lines) = mpsc::channel();
        for stream in [
            child
                .stderr
                .take()
                .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
            child
                .stdout
                .take()
                .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
        ]
        .into_iter()
        .flatten()
        {
            let send = send.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(stream).lines().map_while(Result::ok) {
                    if send.send(line).is_err() {
                        return;
                    }
                }
            });
        }
        Self {
            child,
            lines,
            said: Vec::new(),
        }
    }

    /// The first line containing `needle`, within [`WAIT`].
    fn until(&mut self, needle: &str) -> String {
        let end = Instant::now() + WAIT;
        loop {
            if let Some(line) = self.said.iter().find(|line| line.contains(needle)) {
                return line.clone();
            }
            let left = end.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(left) {
                Ok(line) => self.said.push(line),
                Err(_) => panic!("no line with {needle:?}; saw {:?}", self.said),
            }
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("a port")
        .local_addr()
        .expect("its address")
        .port()
}

#[test]
fn id_with_an_identity_file_makes_the_key_once_and_names_it_after() {
    let scratch = Scratch::new();
    let key = scratch.0.join("agent.key");
    let first = run(canoa(&scratch.0).arg("id").arg("--identity").arg(&key));
    assert!(first.status.success(), "{}", text(&first.stderr));
    let peer = text(&first.stdout);
    assert!(peer.starts_with("sha256:"), "{peer}");
    assert!(key.exists());
    let again = run(canoa(&scratch.0).arg("id").arg("--identity").arg(&key));
    assert!(again.status.success(), "{}", text(&again.stderr));
    assert_eq!(text(&again.stdout), peer);
    // The home's own key is another one.
    let own = run(canoa(&scratch.0).arg("id"));
    assert_ne!(text(&own.stdout), peer);
}

#[test]
fn grant_adds_and_takes_away_a_protocol_in_this_homes_grants() {
    let scratch = Scratch::new();
    let peer = format!("sha256:{}", "ab".repeat(32));
    let given = run(canoa(&scratch.0).args(["grant", &peer, "tcp/1:127.0.0.1:9"]));
    assert!(given.status.success(), "{}", text(&given.stderr));
    let grants =
        std::fs::read_to_string(scratch.0.join("grants.json")).expect("this home's grants");
    assert!(grants.contains("tcp/1:127.0.0.1:9"), "{grants}");
    let taken = run(canoa(&scratch.0).args(["grant", &peer, "tcp/1:127.0.0.1:9", "--revoke"]));
    assert!(taken.status.success(), "{}", text(&taken.stderr));
    let grants = std::fs::read_to_string(scratch.0.join("grants.json")).expect("grants");
    assert!(!grants.contains("tcp/1:127.0.0.1:9"), "{grants}");
    // Being paired gives acp/1, so taking it away would say it worked and change nothing.
    let refused = run(canoa(&scratch.0).args(["grant", &peer, "acp/1", "--revoke"]));
    assert_eq!(refused.status.code(), Some(2), "{}", text(&refused.stderr));
}

#[test]
fn serve_program_advertises_what_discover_finds_and_dial_keeps_its_agent() {
    let scratch = Scratch::new();
    let agent = example("stdio_agent", false);
    let key = scratch.0.join("agent.key");
    let server_key = scratch.0.join("server.key");
    let client = text(&run(canoa(&scratch.0).arg("id").arg("--identity").arg(&key)).stdout);
    let port = free_port();
    let mut command = canoa(&scratch.0);
    command
        .args(["serve", "--program", "--identity"])
        .arg(&server_key)
        .args(["--allow", &client, "--listen"])
        .arg(format!("127.0.0.1:{port}"))
        .args(["--advertise", "Bridge command", "--"])
        .arg(&agent);
    // The server's key is made first, as `keygen` did.
    let made = run(canoa(&scratch.0)
        .arg("id")
        .arg("--identity")
        .arg(&server_key));
    let server_peer = text(&made.stdout);
    let mut server = Running::start(command);
    server.until("listening on");

    let found = run(canoa(&scratch.0).args(["discover", "--seconds", "0"]));
    assert!(found.status.success(), "{}", text(&found.stderr));
    let candidates: Vec<Value> = text(&found.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).expect("a JSON line"))
        .collect();
    let mine = candidates
        .iter()
        .find(|candidate| candidate["label"] == "Bridge command")
        .unwrap_or_else(|| panic!("{candidates:?}"));
    assert_eq!(mine["peer"], server_peer.as_str());

    let mut dial = canoa(&scratch.0);
    dial.arg("dial")
        .arg(format!("ws://127.0.0.1:{port}"))
        .arg("--identity")
        .arg(&key)
        .args(["--peer", &server_peer, "--"])
        .arg(&agent);
    let mut dialing = Running::start(dial);
    dialing.until("connected");
}

#[test]
fn serve_flags_that_belong_to_program_are_refused_without_it() {
    let scratch = Scratch::new();
    let refused = run(canoa(&scratch.0).args(["serve", "--listen", "127.0.0.1:1"]));
    assert_eq!(refused.status.code(), Some(2), "{}", text(&refused.stderr));
    assert!(
        text(&refused.stderr).contains("--program"),
        "{}",
        text(&refused.stderr)
    );
    let alone = run(canoa(&scratch.0).args(["serve", "--program"]));
    assert_eq!(alone.status.code(), Some(2), "{}", text(&alone.stderr));
}

#[test]
fn forward_refuses_a_listener_that_is_not_loopback() {
    let scratch = Scratch::new();
    let refused = run(canoa(&scratch.0).args([
        "forward",
        "ws://127.0.0.1:1",
        "--paired",
        "--local",
        "0.0.0.0:7=127.0.0.1:8",
    ]));
    assert_eq!(refused.status.code(), Some(2), "{}", text(&refused.stderr));
    assert!(
        text(&refused.stderr).contains("not loopback"),
        "{}",
        text(&refused.stderr)
    );
}

/// What a browser sends to open a WebSocket, with its origin.
fn upgrade(port: u16, origin: Option<&str>) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("the listener");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("a timeout");
    let origin = origin.map_or_else(String::new, |origin| format!("Origin: {origin}\r\n"));
    write!(
        stream,
        "GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{origin}Upgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
    )
    .expect("written");
    let mut head = [0_u8; 64];
    let read = stream.read(&mut head).unwrap_or(0);
    String::from_utf8_lossy(&head[..read]).into_owned()
}

#[test]
fn pair_takes_a_browser_only_from_the_origin_it_was_given() {
    let scratch = Scratch::new();
    let allowed = "chrome-extension://abcdefghijklmnopabcdefghijklmnop";
    let mut waiting = Running::start({
        let mut command = canoa(&scratch.0);
        command
            .args(["pair", "--listen", "127.0.0.1:0", "--origin", allowed])
            .args(["--timeout-seconds", "60"]);
        command
    });
    let line = waiting.until("\"pin\"");
    let shown: Value = serde_json::from_str(&line).expect("the PIN line");
    let url = shown["url"].as_str().expect("its url");
    let port: u16 = url
        .rsplit(':')
        .next()
        .and_then(|port| port.parse().ok())
        .expect("a port");
    assert!(
        upgrade(port, Some(allowed)).starts_with("HTTP/1.1 101"),
        "the named origin is let in"
    );
    assert!(
        !upgrade(port, Some("https://example.com")).starts_with("HTTP/1.1 101"),
        "another origin is not"
    );
    assert!(
        upgrade(port, None).starts_with("HTTP/1.1 101"),
        "a native client sends none"
    );

    // Without --origin a browser stays refused.
    let other = Scratch::new();
    let mut plain = Running::start({
        let mut command = canoa(&other.0);
        command.args(["pair", "--listen", "127.0.0.1:0", "--timeout-seconds", "60"]);
        command
    });
    let line = plain.until("\"pin\"");
    let shown: Value = serde_json::from_str(&line).expect("the PIN line");
    let url = shown["url"].as_str().expect("its url");
    let port: u16 = url
        .rsplit(':')
        .next()
        .and_then(|port| port.parse().ok())
        .expect("a port");
    assert!(
        !upgrade(port, Some(allowed)).starts_with("HTTP/1.1 101"),
        "a browser is refused with no --origin"
    );
}

#[test]
fn pair_join_pairs_with_a_waiting_pair_and_names_it() {
    let host = Scratch::new();
    let joiner = Scratch::new();
    let mut waiting = Running::start({
        let mut command = canoa(&host.0);
        command.args(["pair", "--listen", "127.0.0.1:0", "--timeout-seconds", "60"]);
        command
    });
    let line = waiting.until("\"pin\"");
    let shown: Value = serde_json::from_str(&line).expect("the PIN line");
    let (pin, url) = (
        shown["pin"].as_str().expect("a PIN"),
        shown["url"].as_str().expect("a url"),
    );
    let file = joiner.0.join("pin.txt");
    std::fs::write(&file, format!("{pin}\n")).expect("the PIN file");
    let joined = run(canoa(&joiner.0)
        .args(["pair", "--join", url, "--pin-file"])
        .arg(&file));
    assert!(joined.status.success(), "{}", text(&joined.stderr));
    assert_eq!(
        text(&joined.stdout),
        shown["peer"].as_str().expect("its peer")
    );
    waiting.until("paired");
}

#[test]
fn mcp_config_names_this_program_this_home_and_the_peer() {
    let scratch = Scratch::new();
    let peer = format!("sha256:{}", "cd".repeat(32));
    let printed = run(canoa(&scratch.0).args(["mcp-config", "--peer", &peer]));
    assert!(printed.status.success(), "{}", text(&printed.stderr));
    let config: Value = serde_json::from_str(&text(&printed.stdout)).expect("JSON");
    let server = &config["mcpServers"]["canoa-browser"];
    assert_eq!(
        server["args"],
        serde_json::json!(["mcp-shim", "--peer", peer])
    );
    assert_eq!(
        server["env"]["COQUI_AGENTIC_PEER_HOME"].as_str(),
        scratch.0.to_str()
    );
    let program = server["command"].as_str().expect("a command");
    assert!(
        Path::new(program)
            .file_stem()
            .is_some_and(|stem| stem.eq_ignore_ascii_case("canoa")),
        "{program}"
    );
}
