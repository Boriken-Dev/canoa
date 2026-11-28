//! The built `canoa agent ID`: a bundled agent's harness provider served on standard input and output, whichever agent the id
//! names. No agent program is started: `initialize` is the provider's own answer.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod support;

use std::io::{BufRead as _, BufReader, Write as _};
use std::process::Stdio;

use serde_json::{Value, json};

/// Runs `canoa agent ARGS`, sends it `initialize` and returns what it answered, and how it ended.
fn initialize(args: &[&str]) -> (Option<Value>, std::process::ExitStatus, String) {
    let mut child = support::host_command()
        .arg("agent")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the host starts");
    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = BufReader::new(child.stdout.take().expect("stdout"));
    let request = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": 1, "clientCapabilities": {}}});
    let sent = writeln!(stdin, "{request}").is_ok();
    let mut line = String::new();
    let answered = sent && stdout.read_line(&mut line).is_ok_and(|read| read > 0);
    drop(stdin);
    let output = child.wait_with_output().expect("it ends");
    let answer = answered.then(|| serde_json::from_str(&line).expect("one JSON line"));
    (
        answer,
        output.status,
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn a_bundled_agent_answers_initialize_on_standard_streams_and_ends_when_they_close() {
    for id in ["claude", "codex"] {
        let (answer, status, why) = initialize(&[id, "--executable", "none"]);
        let answer = answer.unwrap_or_else(|| panic!("{id} answered nothing: {why}"));
        assert_eq!(answer["id"], 1, "{id}: {answer}");
        assert!(answer.get("result").is_some(), "{id}: {answer}");
        assert!(status.success(), "{id}: {status} {why}");
    }
}

#[test]
fn an_id_the_host_does_not_carry_is_a_usage_error() {
    let (answer, status, why) = initialize(&["nobody"]);
    assert!(answer.is_none(), "{answer:?}");
    assert_eq!(status.code(), Some(2), "{why}");
    assert!(why.contains("nobody"), "{why}");
}

#[test]
fn a_setting_the_provider_does_not_declare_is_a_usage_error() {
    let (answer, status, why) = initialize(&["claude", "--setting", "no_such_setting=1"]);
    assert!(answer.is_none(), "{answer:?}");
    assert_eq!(status.code(), Some(2), "{why}");
}
