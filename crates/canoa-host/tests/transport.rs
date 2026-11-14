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

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use coqui_transport::Channel;
use coqui_transport_guatiao::Driver;
use coqui_transport_guatiao::pipe::{PipeDriver, PipeSettings};
use coqui_transport_guatiao::websocket::{WebSocketDriver, WebSocketSettings};
use coqui_transport_web::websocket::Frames;
use serde_json::{Value, json};
use support::{Host, Scratch, cwd, example, host_binary, quoted, with_channel};

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
    let output = Command::new(host_binary())
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
