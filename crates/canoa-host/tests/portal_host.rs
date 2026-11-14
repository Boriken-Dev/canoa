//! The built `canoa serve --portal`: a host whose every entry and whose control channel are on sockets, reached only by the
//! portal's user, with a socket that introduces a browser's key.
#![cfg(unix)]
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod support;

use std::io::{Read, Write};
use std::os::unix::fs::MetadataExt as _;
use std::process::{Command, Stdio};
use std::time::Duration;

use canoa_host::pipe::{PipeName, Stream};
use coqui_transport_pipe::ReadTimeout as _;
use serde_json::{Value, json};
use support::{Host, Scratch, cwd, example, host_binary, quoted, with_pipe_link};

fn my_uid() -> u32 {
    std::fs::metadata("/proc/self").expect("proc").uid()
}

/// A host for a portal run by user `uid`, with an entry whose own file asks for a TCP address, and the directory of its sockets.
fn portal_host(uid: u32) -> (Host, std::path::PathBuf) {
    let scratch = Scratch::new();
    let dir = scratch.0.join("run");
    let config = format!(
        "[host.agents.fixture]\nprogram = {}\nlisten = \"127.0.0.1:0\"\n",
        quoted(&example("stdio_agent", false).to_string_lossy()),
    );
    let uid = uid.to_string();
    let args = [
        "--portal",
        dir.to_str().expect("utf-8"),
        "--portal-user",
        &uid,
    ];
    // One line for the entry and one for the control channel.
    let host = Host::start_in_with(scratch, &config, 2, &args, &[]);
    (host, dir)
}

/// One request to the introduction socket: a line of JSON, answered with a line of JSON.
fn ask(dir: &std::path::Path, request: &Value) -> Result<Value, String> {
    let name = PipeName::at(dir.join("portal.sock")).map_err(|error| error.to_string())?;
    let stream = Stream::connect(&name).map_err(|error| error.to_string())?;
    let mut stream = &stream;
    stream
        .write_all(format!("{request}\n").as_bytes())
        .map_err(|error| error.to_string())?;
    let mut answer = String::new();
    stream
        .read_to_string(&mut answer)
        .map_err(|error| error.to_string())?;
    serde_json::from_str(answer.trim()).map_err(|error| format!("{error}: {answer:?}"))
}

#[test]
fn every_entry_and_the_control_channel_are_on_sockets_whatever_the_file_asks() {
    let (host, dir) = portal_host(my_uid());
    let socket = dir.join("entries").join("fixture.sock");
    assert_eq!(host.url("fixture"), format!("unix:{}", socket.display()));
    assert!(
        host.urls.values().all(|url| url.starts_with("unix:")),
        "{:?}",
        host.urls
    );
    assert!(socket.exists());
    assert!(dir.join("control.sock").exists());
    assert!(dir.join("portal.sock").exists());
    let index = ask(&dir, &json!({"op": "index"})).expect("the introduction socket answers");
    assert_eq!(index["peer"], host.peer.to_string().as_str());
    let entries = index["entries"].as_array().expect("entries");
    assert!(entries.iter().any(|entry| entry["name"] == "fixture"));
    assert!(
        entries.iter().all(|entry| entry.get("url").is_none()),
        "{index}"
    );
    assert!(entries.iter().any(|entry| entry["kind"] == "control"));
}

#[test]
fn a_key_that_waits_is_accepted_through_the_introduction_socket_and_then_runs_a_turn_and_the_control_channel()
 {
    let (host, dir) = portal_host(my_uid());
    let listen = host.url("fixture").to_owned();
    let control = format!("unix:{}", dir.join("control.sock").display());
    let key = support::key();
    // Introduced before it has connected: nothing is waiting, so nothing is accepted.
    let early = ask(
        &dir,
        &json!({"op": "accept", "peer": key.peer_id().to_string()}),
    )
    .expect("answered");
    assert!(early.get("error").is_some(), "{early}");
    let refused = with_pipe_link(&listen, &key, host.peer, |rpc| rpc.initialize());
    assert!(!matches!(refused, Ok(Ok(_))), "{refused:?}");
    let accepted = ask(
        &dir,
        &json!({"op": "accept", "peer": key.peer_id().to_string()}),
    )
    .expect("answered");
    assert_eq!(accepted["ok"], true, "{accepted}");
    let work = cwd(&host.scratch);
    let text = with_pipe_link(&listen, &key, host.peer, |rpc| {
        rpc.initialize()?;
        let session = rpc.request("session/new", json!({"cwd": work, "mcpServers": []}))?;
        rpc.prompt(session["sessionId"].as_str().expect("an id"), "hello")
    })
    .expect("an introduced key is served")
    .expect("the turn ends");
    assert_eq!(text, "echo: hello");
    // On the control channel every paired key is the user's own browser: it holds the page grants, with no entry for it in canoa.cfg.
    let ledger = with_pipe_link(&control, &key, host.peer, |rpc| {
        rpc.request("ledger/list", json!({}))
    })
    .expect("the key reaches the control channel");
    assert!(ledger.is_ok(), "{ledger:?}");
    let folders = with_pipe_link(&control, &key, host.peer, |rpc| {
        rpc.request("fs/list", json!({}))
    })
    .expect("the key reaches the control channel");
    assert!(folders.is_ok(), "{folders:?}");
    let revoked = ask(
        &dir,
        &json!({"op": "revoke", "peer": key.peer_id().to_string()}),
    )
    .expect("answered");
    assert_eq!(revoked["ok"], true);
    let after = with_pipe_link(&listen, &key, host.peer, |rpc| rpc.initialize());
    assert!(!matches!(after, Ok(Ok(_))), "{after:?}");
}

#[test]
fn a_connection_from_any_user_but_the_portals_is_closed_before_a_byte_is_read() {
    // The host takes the portal to be user 0, or user 1 when this test is root: the connection here is from another user.
    let other = if my_uid() == 0 { 1 } else { 0 };
    let (host, dir) = portal_host(other);
    for path in [
        dir.join("portal.sock"),
        dir.join("entries").join("fixture.sock"),
        dir.join("control.sock"),
    ] {
        let name = PipeName::at(&path).expect("a name");
        let stream = Stream::connect(&name).expect("the socket's mode lets any user connect");
        let mut stream = &stream;
        let _ = stream.write_all(b"{\"op\":\"index\"}\n");
        let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
        let mut answer = Vec::new();
        let _ = stream.read_to_end(&mut answer);
        assert!(
            answer.is_empty(),
            "{path:?} answered another user: {answer:?}"
        );
    }
    let key = support::key();
    let listen = host.url("fixture").to_owned();
    let refused = with_pipe_link(&listen, &key, host.peer, |rpc| rpc.initialize());
    assert!(!matches!(refused, Ok(Ok(_))), "{refused:?}");
    // And what that user did not ask for is not done: the key never became waiting, so it could not have been accepted.
    let listed = host.command(&["peers", "--format", "json"]);
    assert!(!String::from_utf8_lossy(&listed.stdout).contains(&key.peer_id().to_string()));
}

#[test]
fn a_portal_user_that_is_not_an_account_is_a_usage_error_and_binds_nothing() {
    let scratch = Scratch::new();
    let config = scratch.0.join("canoa.cfg");
    std::fs::write(&config, "").expect("a file");
    let output = Command::new(host_binary())
        .args(["serve", "--allow-empty", "--config"])
        .arg(&config)
        .args(["--portal"])
        .arg(scratch.0.join("run"))
        .args(["--portal-user", "canoa-no-such-account"])
        .env("COQUI_AGENTIC_PEER_HOME", scratch.0.join("home"))
        .stdin(Stdio::null())
        .output()
        .expect("runs");
    assert_eq!(
        output.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!scratch.0.join("run").join("portal.sock").exists());
}
