//! The built `canoa`, serving entries through a `transport` provider an entry names by id: the built-in `pipe` and `websocket`
//! ones. The client dials with the same provider's driver, which frames messages as the provider does, and proves a key as it
//! does over a socket: the same pairing, the same refusals.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod linked;
mod support;

use std::sync::atomic::{AtomicU64, Ordering};

use coqui_transport::Channel;
use coqui_transport_guatiao::Driver;
use coqui_transport_guatiao::pipe::{PipeDriver, PipeSettings};
use coqui_transport_guatiao::websocket::{WebSocketDriver, WebSocketSettings};
use coqui_transport_web::websocket::Frames;
use serde_json::{Value, json};
use support::{Host, Scratch, cwd, example, host_command, quoted, with_channel};

/// A pipe or socket name no other test run uses: a pipe name on Windows is global, and a socket's path on Unix is under the
/// test's own directory.
fn pipe_in(scratch: &Scratch) -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    if cfg!(windows) {
        format!(
            "pipe:canoa-transport-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        )
    } else {
        format!("unix:{}", scratch.0.join("host.sock").display())
    }
}

fn agent(extra: &str) -> String {
    format!(
        "program = {}\n{extra}\n",
        quoted(&example("stdio_agent", false).to_string_lossy())
    )
}

fn turn<C: Channel>(rpc: &support::Rpc<'_, C>, dir: &str) -> Result<String, Value> {
    rpc.initialize()?;
    let session = rpc.request("session/new", json!({"cwd": dir, "mcpServers": []}))?;
    rpc.prompt(session["sessionId"].as_str().expect("an id"), "hello")
}

fn pipe_channel(endpoint: &str) -> Box<dyn Channel> {
    PipeDriver::dial(&PipeSettings::default(), endpoint, support::WAIT)
        .expect("the pipe is dialled")
}

fn websocket_channel(endpoint: &str) -> Box<dyn Channel> {
    let mut settings = WebSocketSettings::default();
    settings.dial.frames = Frames::Binary;
    WebSocketDriver::dial(&settings, endpoint, support::WAIT).expect("the socket is dialled")
}

/// What `canoa serve` says on its standard error for `config`, which it must refuse to start.
fn bind_refused(config: &str) -> String {
    let scratch = Scratch::new();
    let file = scratch.0.join("canoa.cfg");
    std::fs::write(&file, config).expect("written");
    let output = host_command()
        .args(["serve", "--config"])
        .arg(&file)
        .env("COQUI_AGENTIC_PEER_HOME", scratch.0.join("home"))
        .stdin(std::process::Stdio::null())
        .output()
        .expect("it runs");
    assert!(!output.status.success(), "it started: {output:?}");
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn a_paired_key_runs_a_turn_through_the_pipe_transport_and_the_next_connection_too() {
    let scratch = Scratch::new();
    let listen = pipe_in(&scratch);
    let config = format!(
        "[host.agents.fixture]\n{}transport = \"pipe\"\nlisten = {}\n",
        agent(""),
        quoted(&listen)
    );
    let host = Host::start_in(scratch, &config, 1, &[]);
    assert_eq!(host.served["fixture"]["transport"], "pipe");
    let client = support::key();
    host.pair(&client);
    let dir = cwd(&host.scratch);
    let endpoint = host.url("fixture").to_owned();
    let text = with_channel(pipe_channel(&endpoint), &client, host.peer, |rpc| {
        turn(rpc, &dir)
    })
    .expect("a paired key is served")
    .expect("the turn ends");
    assert_eq!(text, "echo: hello");
    let again = with_channel(pipe_channel(&endpoint), &client, host.peer, |rpc| {
        rpc.initialize()
    })
    .expect("a paired key is served again");
    assert!(again.is_ok(), "{again:?}");
}

#[test]
fn an_unpaired_key_gets_nothing_through_a_transport_either() {
    let scratch = Scratch::new();
    let listen = pipe_in(&scratch);
    let config = format!(
        "[host.agents.fixture]\n{}transport = \"pipe\"\nlisten = {}\n",
        agent(""),
        quoted(&listen)
    );
    let host = Host::start_in(scratch, &config, 1, &[]);
    let client = support::key();
    let endpoint = host.url("fixture").to_owned();
    let refused = with_channel(pipe_channel(&endpoint), &client, host.peer, |rpc| {
        rpc.initialize()
    });
    let served = matches!(refused, Ok(Ok(_)));
    assert!(!served, "an unpaired key gets nothing: {refused:?}");
}

#[test]
fn a_paired_key_runs_a_turn_through_the_websocket_transport() {
    let config = format!(
        "[host.agents.fixture]\n{}transport = \"websocket\"\nlisten = \"ws://127.0.0.1:0\"\n\
         transport_settings = {{ listen = {{ frames = \"binary\" }} }}\n",
        agent("")
    );
    let host = Host::start(&config, 1);
    let endpoint = host.url("fixture").to_owned();
    assert!(
        endpoint.starts_with("ws://127.0.0.1:"),
        "serve says where the provider listens: {endpoint}"
    );
    assert_ne!(endpoint, "ws://127.0.0.1:0", "the port the system chose");
    let client = support::key();
    host.pair(&client);
    let dir = cwd(&host.scratch);
    let text = with_channel(websocket_channel(&endpoint), &client, host.peer, |rpc| {
        turn(rpc, &dir)
    })
    .expect("a paired key is served")
    .expect("the turn ends");
    assert_eq!(text, "echo: hello");
}

/// The transport's own rule that plain WebSocket stays on loopback is not lifted by the entry's address: it takes the
/// provider's own setting, which the host then logs as non-default.
#[test]
fn plain_websocket_beyond_loopback_through_a_transport_needs_the_providers_setting() {
    let refused = bind_refused(&format!(
        "[host.agents.fixture]\n{}transport = \"websocket\"\nlisten = \"ws://0.0.0.0:0\"\n",
        agent("")
    ));
    assert!(refused.contains("host entry fixture"), "{refused}");
    assert!(refused.contains("transport provider"), "{refused}");
}

#[test]
fn a_setting_the_provider_does_not_declare_is_refused_naming_the_entry_and_quoting_nothing() {
    let refused = bind_refused(&format!(
        "[host.agents.fixture]\n{}transport = \"websocket\"\nlisten = \"ws://127.0.0.1:0\"\n\
         transport_settings = {{ no_such_setting = \"hunter2\" }}\n",
        agent("")
    ));
    assert!(refused.contains("host entry fixture"), "{refused}");
    assert!(!refused.contains("hunter2"), "{refused}");
}

#[test]
fn an_endpoint_the_provider_cannot_read_is_refused_naming_the_entry() {
    let refused = bind_refused(&format!(
        "[host.agents.fixture]\n{}transport = \"pipe\"\nlisten = \"ws://127.0.0.1:0\"\n",
        agent("")
    ));
    assert!(refused.contains("host entry fixture"), "{refused}");
}

/// A host run in this process: stopping it ends the transport's listener, whose endpoint is then free for another host.
#[test]
fn stopping_the_host_closes_a_transports_listener_and_frees_its_endpoint() {
    let scratch = Scratch::new();
    let listen = pipe_in(&scratch);
    let config = format!(
        "[host.agents.fixture]\n{}transport = \"pipe\"\nlisten = {}\n",
        agent(""),
        quoted(&listen)
    );
    let host = linked::Hosted::start_in(scratch, &config, &[]);
    let endpoint = host
        .line_of("fixture")
        .expect("serve says where it listens")["url"]
        .as_str()
        .expect("a url")
        .to_owned();
    // A peer is accepted by the transport, before any proof.
    let peer = pipe_channel(&endpoint);
    peer.close();
    host.stop();
    assert!(host.ended.join().expect("no panic").is_ok());
    let gone = PipeDriver::dial(
        &PipeSettings::default(),
        &endpoint,
        std::time::Duration::from_millis(500),
    );
    assert!(gone.is_err(), "nobody listens after the host stopped");
    let again = linked::Hosted::start(&config);
    again.stop();
    assert!(again.ended.join().expect("no panic").is_ok());
}

/// What a raw WebSocket upgrade to `address` with `origin` is answered: the status line.
fn upgrade_status(address: &str, origin: Option<&str>) -> String {
    use std::io::{Read as _, Write as _};
    let authority = address.trim_start_matches("ws://");
    let mut stream = std::net::TcpStream::connect(authority).expect("connects");
    stream
        .set_read_timeout(Some(support::WAIT))
        .expect("a timeout");
    let origin = origin.map_or_else(String::new, |origin| format!("Origin: {origin}\r\n"));
    write!(
        stream,
        "GET / HTTP/1.1\r\nHost: {authority}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n{origin}\r\n"
    )
    .expect("written");
    let mut answer = [0_u8; 64];
    let read = stream.read(&mut answer).unwrap_or(0);
    String::from_utf8_lossy(&answer[..read])
        .lines()
        .next()
        .unwrap_or_default()
        .to_owned()
}

/// `[host] origins` is the browser-origin check of every entry, an address listener's and a websocket transport's alike, unless
/// the entry lists its own.
#[test]
fn the_hosts_origins_reach_a_websocket_transport_that_lists_none() {
    let entry = |name: &str, extra: &str| {
        format!(
            "[host.agents.{name}]\n{}transport = \"websocket\"\nlisten = \"ws://127.0.0.1:0\"\n{extra}",
            agent("")
        )
    };
    let config = format!(
        "[host]\norigins = [\"https://canoa.example\"]\n{}{}",
        entry("host_origins", ""),
        entry(
            "own_origins",
            "transport_settings = { listen = { origins = [\"https://other.example\"] } }\n"
        )
    );
    let (host, log) = Host::start_logged_with(Scratch::new(), &config, 2, &[]);
    let inherited = host.url("host_origins").to_owned();
    assert!(
        upgrade_status(&inherited, Some("https://canoa.example")).contains("101"),
        "the listed origin is let in"
    );
    assert!(
        !upgrade_status(&inherited, Some("https://evil.example")).contains("101"),
        "another origin is refused"
    );
    assert!(
        upgrade_status(&inherited, None).contains("101"),
        "a client that is not a browser sends none"
    );
    // The entry's own list replaces the host's.
    let own = host.url("own_origins").to_owned();
    assert!(!upgrade_status(&own, Some("https://canoa.example")).contains("101"));
    assert!(upgrade_status(&own, Some("https://other.example")).contains("101"));
    let said = support::log_holding(&log, &["listen.origins from [host] origins"]);
    assert!(
        said.contains("host.agents.host_origins.transport"),
        "{said}"
    );
    assert!(!said.contains("host.agents.own_origins.transport has listen.origins from"));
}

/// A peer that connects to a websocket transport and says nothing holds one handshake of its own: another peer is served at once.
#[test]
fn an_idle_connection_to_a_websocket_transport_delays_no_other_peer() {
    let config = format!(
        "[host.agents.fixture]\n{}transport = \"websocket\"\nlisten = \"ws://127.0.0.1:0\"\n\
         transport_settings = {{ listen = {{ frames = \"binary\" }} }}\n",
        agent("")
    );
    let host = Host::start(&config, 1);
    let endpoint = host.url("fixture").to_owned();
    let _idle =
        std::net::TcpStream::connect(endpoint.trim_start_matches("ws://")).expect("connects");
    let client = support::key();
    host.pair(&client);
    let dir = cwd(&host.scratch);
    let started = std::time::Instant::now();
    let text = with_channel(websocket_channel(&endpoint), &client, host.peer, |rpc| {
        turn(rpc, &dir)
    })
    .expect("a paired key is served")
    .expect("the turn ends");
    assert_eq!(text, "echo: hello");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "a turn took {:?} behind an idle connection",
        started.elapsed()
    );
}
