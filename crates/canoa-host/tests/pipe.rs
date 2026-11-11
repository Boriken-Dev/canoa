//! The built `canoa`, serving entries on a Windows named pipe or a Unix socket, reached by a client that proves a key as it
//! does over a socket: the same pairing, the same refusals.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod support;

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use support::{Host, Scratch, cwd, example, host_binary, quoted, with_link, with_pipe_link};

/// A name for an entry's pipe that no other test run uses: a pipe name on Windows is global, and a socket's path on Unix is
/// under the test's own directory.
fn listen_in(scratch: &Scratch) -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    if cfg!(windows) {
        format!(
            "pipe:canoa-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        )
    } else {
        format!("unix:{}", scratch.0.join("host.sock").display())
    }
}

fn agent_entry(name: &str, listen: &str, extra: &str) -> String {
    format!(
        "[host.agents.{name}]\nprogram = {}\nlisten = {}\n{extra}\n",
        quoted(&example("stdio_agent", false).to_string_lossy()),
        quoted(listen)
    )
}

/// The host serving one entry named `fixture` on a pipe, and the text it listens on.
fn host_on_a_pipe(extra: &str) -> (Host, String) {
    let scratch = Scratch::new();
    let listen = listen_in(&scratch);
    let host = Host::start_in(scratch, &agent_entry("fixture", &listen, extra), 1, &[]);
    (host, listen)
}

/// Whether a link to a host that does not admit the key gave nothing. WebCrypto's initiator hears the refusal in the proof;
/// Noise's finishes on its last message and hears it as the link ending before anything is answered.
fn refused_link<T: core::fmt::Debug>(link: &Result<Result<T, Value>, String>) -> bool {
    match support::method() {
        coqui_agentic_peer::auth::Method::WebCrypto => link.is_err(),
        _ => !matches!(link, Ok(Ok(_))),
    }
}

fn turn(rpc: &support::Rpc<'_, canoa_host::pipe::PipeChannel>, dir: &str) -> Result<String, Value> {
    rpc.initialize()?;
    let session = rpc.request("session/new", json!({"cwd": dir, "mcpServers": []}))?;
    rpc.prompt(session["sessionId"].as_str().expect("an id"), "hello")
}

#[test]
fn a_paired_key_runs_a_turn_over_a_pipe() {
    let (host, listen) = host_on_a_pipe("");
    assert_eq!(host.url("fixture"), listen, "serve says where it listens");
    let client = support::key();
    host.pair(&client);
    let dir = cwd(&host.scratch);
    let text = with_pipe_link(&listen, &client, host.peer, |rpc| turn(rpc, &dir))
        .expect("a paired key is served")
        .expect("the turn ends");
    assert_eq!(text, "echo: hello");
    // The pipe serves the next connection too.
    let again = with_pipe_link(&listen, &client, host.peer, |rpc| rpc.initialize())
        .expect("a paired key is served again");
    assert!(again.is_ok(), "{again:?}");
}

#[test]
fn an_unpaired_key_is_refused_over_a_pipe_until_a_person_accepts_it() {
    let (host, listen) = host_on_a_pipe("");
    let client = support::key();
    let refused = with_pipe_link(&listen, &client, host.peer, |rpc| rpc.initialize());
    assert!(
        refused_link(&refused),
        "an unpaired key gets nothing: {refused:?}"
    );

    let listed = host.command(&["peers", "--format", "json"]);
    assert!(listed.status.success());
    let id = client.peer_id().to_string();
    let waiting = String::from_utf8_lossy(&listed.stdout).lines().any(|line| {
        let peer: Value = serde_json::from_str(line).expect("JSON");
        peer["peer"] == id.as_str() && peer["state"] == "waiting"
    });
    assert!(waiting, "the refused key waits for `canoa accept`");
    let accepted = host.command(&["accept", &id]);
    assert!(
        accepted.status.success(),
        "{}",
        String::from_utf8_lossy(&accepted.stderr)
    );

    let dir = cwd(&host.scratch);
    let text = with_pipe_link(&listen, &client, host.peer, |rpc| turn(rpc, &dir))
        .expect("an accepted key is served")
        .expect("the turn ends");
    assert_eq!(text, "echo: hello");
}

#[test]
fn a_revoked_key_loses_its_link_and_is_refused_again_over_a_pipe() {
    let (host, listen) = host_on_a_pipe("");
    let client = support::key();
    host.pair(&client);
    with_pipe_link(&listen, &client, host.peer, |rpc| {
        rpc.initialize().expect("initialize is answered");
        let removed = host.command(&["revoke", &client.peer_id().to_string()]);
        assert!(removed.status.success());
        let started = Instant::now();
        let ended = rpc.next_message(Duration::from_secs(10));
        assert!(ended.is_err(), "the link ends: {ended:?}");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
    })
    .expect("the key was paired when it connected");
    let after = with_pipe_link(&listen, &client, host.peer, |rpc| rpc.initialize());
    assert!(refused_link(&after), "a removed key is refused: {after:?}");
}

#[test]
fn a_client_pinning_another_host_key_is_refused_over_a_pipe() {
    let (host, listen) = host_on_a_pipe("");
    let client = support::key();
    host.pair(&client);
    let impostor = support::key().peer_id();
    assert!(with_pipe_link(&listen, &client, impostor, |_| ()).is_err());
}

#[test]
fn an_entry_that_lists_its_peers_turns_away_other_paired_keys_over_a_pipe() {
    let listed = support::key();
    let other = support::key();
    let (host, listen) = host_on_a_pipe(&format!("peers = [\"{}\"]", listed.peer_id()));
    host.pair(&listed);
    host.pair(&other);
    let turned_away = with_pipe_link(&listen, &other, host.peer, |rpc| rpc.initialize())
        .expect("the proof itself succeeds");
    assert!(turned_away.is_err(), "{turned_away:?}");
    with_pipe_link(&listen, &listed, host.peer, |rpc| {
        rpc.initialize().expect("the listed key is served");
    })
    .expect("a paired key is served");
}

#[test]
fn a_pipe_and_an_address_serve_side_by_side_and_each_keeps_its_own_kind_of_url() {
    let scratch = Scratch::new();
    let listen = listen_in(&scratch);
    let config = [
        agent_entry("piped", &listen, ""),
        format!(
            "[host.agents.socket]\nprogram = {}\n",
            quoted(&example("stdio_agent", false).to_string_lossy())
        ),
    ]
    .concat();
    let host = Host::start_in(scratch, &config, 2, &[]);
    assert_eq!(host.url("piped"), listen);
    assert!(host.url("socket").starts_with("ws://127.0.0.1:"));
    let client = support::key();
    host.pair(&client);
    with_link(host.url("socket"), &client, host.peer, |rpc| {
        rpc.initialize().expect("the socket still serves");
    })
    .expect("a paired key is served");
    with_pipe_link(&listen, &client, host.peer, |rpc| {
        rpc.initialize().expect("the pipe serves");
    })
    .expect("a paired key is served");
}

#[test]
fn check_prints_a_pipe_entry_and_refuses_a_pipe_it_cannot_use() {
    let scratch = Scratch::new();
    let path = scratch.0.join("canoa.cfg");
    let run = |args: &[&str]| {
        std::process::Command::new(host_binary())
            .args(args)
            .env("COQUI_AGENTIC_PEER_HOME", scratch.0.join("home"))
            .output()
            .expect("runs")
    };
    let config_arg = path.to_string_lossy().into_owned();
    let listen = listen_in(&scratch);
    std::fs::write(&path, agent_entry("fixture", &listen, "")).expect("written");
    let checked = run(&["check", "--config", &config_arg]);
    assert!(
        checked.status.success(),
        "{}",
        String::from_utf8_lossy(&checked.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&checked.stdout).trim(),
        format!("fixture agent {listen}")
    );
    let json = run(&["check", "--config", &config_arg, "--format", "json"]);
    let line: Value =
        serde_json::from_str(String::from_utf8_lossy(&json.stdout).trim()).expect("JSON");
    assert_eq!(line["listen"], listen.as_str());

    std::fs::write(&path, agent_entry("fixture", "pipe:a/b", "")).expect("written");
    assert_eq!(
        run(&["check", "--config", &config_arg]).status.code(),
        Some(2)
    );
}

#[test]
fn a_name_that_is_taken_stops_the_host_before_it_serves() {
    let (_first, listen) = host_on_a_pipe("");
    let scratch = Scratch::new();
    let path = scratch.0.join("canoa.cfg");
    std::fs::write(&path, agent_entry("fixture", &listen, "")).expect("written");
    let second = std::process::Command::new(host_binary())
        .arg("serve")
        .arg("--config")
        .arg(&path)
        .env("COQUI_AGENTIC_PEER_HOME", scratch.0.join("home"))
        .stdin(std::process::Stdio::null())
        .output()
        .expect("runs");
    assert!(!second.status.success(), "{second:?}");
    let said = String::from_utf8_lossy(&second.stderr);
    assert!(said.contains("fixture"), "the entry is named: {said}");
}

/// The cache variables Claude Code's stand-in was started with for a session over a pipe whose client asked for `lifetime`.
fn cache_ttl_over_a_pipe(lifetime: &str) -> serde_json::Value {
    const TTL: &str = "CLAUDE_CODE_PROMPT_CACHE_TTL";
    let scratch = Scratch::new();
    let claude = scratch
        .0
        .join(format!("claude{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(example("harness_standin", false), &claude).expect("the stand-in is copied");
    let record = scratch.0.join("record.jsonl");
    let listen = listen_in(&scratch);
    let config = format!(
        "[host.agents.Claude]\nharness = \"claude\"\nexecutable = {}\nlisten = {}\n",
        quoted(&claude.to_string_lossy()),
        quoted(&listen)
    );
    let env = vec![
        ("STANDIN_RECORD", record.clone().into_os_string()),
        ("STANDIN_ENV", std::ffi::OsString::from(TTL)),
    ];
    let host = Host::start_in(scratch, &config, 1, &env);
    let client = support::key();
    host.pair(&client);
    let dir = cwd(&host.scratch);
    with_pipe_link(&listen, &client, host.peer, |rpc| {
        let meta = json!({canoa_host::CACHE_META: {"lifetime": lifetime}});
        rpc.request(
            "initialize",
            json!({"protocolVersion": 1, "clientCapabilities": {}, "_meta": meta}),
        )?;
        rpc.request("session/new", json!({"cwd": dir, "mcpServers": []}))?;
        Ok::<(), Value>(())
    })
    .expect("a paired key is served")
    .expect("the session opens");
    let first = std::fs::read_to_string(&record)
        .unwrap_or_default()
        .lines()
        .next()
        .map(|line| serde_json::from_str::<Value>(line).expect("one JSON line"))
        .expect("the stand-in started");
    first["env"][TTL].clone()
}

#[test]
fn a_session_over_a_pipe_carries_the_cache_lifetime_its_client_chose() {
    assert_eq!(cache_ttl_over_a_pipe("short"), json!("5m"));
    assert_eq!(cache_ttl_over_a_pipe("long"), json!("1h"));
}
