//! A session moved to a source that keeps no sessions of its own: the history is replayed as context, and only where that
//! entry's `replay_max_bytes` is set. The sources are coqui-acp's stdio provider in its library over its stand-in for `agy`.
//!
//!   CANOA_TEST_STDIO_PROVIDER=<libcoqui_acp_harness-stdio-provider> CANOA_TEST_FAKE_STDIO=<fake_stdio_cli> cargo test -p canoa-host --test history_replay
//!
//! (`cargo build -p coqui-acp-harness --features kind --examples` in coqui-acp builds both.) Without them it does nothing.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod linked;
mod support;

use std::time::{Duration, Instant};

use linked::{Hosted, ask, next, open_session, send};
use serde_json::{Value, json};
use support::{cwd, quoted};

fn variable(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

fn call(
    link: &canoa_host::Link,
    id: u64,
    method: &str,
    params: Value,
) -> Result<Value, (i64, String)> {
    send(
        link,
        &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
    );
    let answer = next(link).expect("answered");
    match answer.get("error") {
        Some(error) => Err((
            error["code"].as_i64().unwrap_or(0),
            error["message"].as_str().unwrap_or("").to_owned(),
        )),
        None => Ok(answer["result"].clone()),
    }
}

#[test]
fn a_session_is_replayed_into_a_source_only_where_it_is_allowed() {
    let (Some(library), Some(fake)) = (
        variable("CANOA_TEST_STDIO_PROVIDER"),
        variable("CANOA_TEST_FAKE_STDIO"),
    ) else {
        eprintln!(
            "skipped: CANOA_TEST_STDIO_PROVIDER and CANOA_TEST_FAKE_STDIO name the built stdio provider and its stand-in"
        );
        return;
    };
    let entry = |name: &str, replay: u64| {
        format!(
            "[host.agents.{name}]\nharness = \"stdio\"\nlibrary = {}\nlisten = \"127.0.0.1:0\"\n\
             [host.agents.{name}.settings]\nharness = \"antigravity\"\nexecutable = {}\nreplay_max_bytes = {replay}\n",
            quoted(&library),
            quoted(&fake),
        )
    };
    let config = [
        entry("Source", 0),
        entry("Replay", 100_000),
        entry("Never", 0),
    ]
    .join("");
    let host = Hosted::start(&config);
    let dir = cwd(&host.scratch);

    let source = host.local.open("Source").expect("a link");
    let session = open_session(&source, &dir);
    let (answer, _) = ask(
        &source,
        3,
        "session/prompt",
        json!({"sessionId": session, "prompt": [{"type": "text", "text": "hello"}]}),
    )
    .expect("a turn");
    assert_eq!(answer["stopReason"], "end_turn");

    let control = host.local.open("@control").expect("the control channel");
    let until = Instant::now() + Duration::from_secs(20);
    while call(
        &control,
        1,
        "history/export",
        json!({"entry": "Source", "session": session}),
    )
    .is_err()
    {
        assert!(Instant::now() < until, "the host kept no history");
        std::thread::sleep(Duration::from_millis(50));
    }

    // Where replay is not allowed the move is refused, with the reason, and nothing is made.
    let (code, why) = call(
        &control,
        2,
        "history/move",
        json!({"entry": "Source", "session": session, "to": "Never"}),
    )
    .expect_err("refused");
    assert_eq!(code, -32002, "{why}");
    assert!(why.contains("replay_max_bytes"), "{why}");

    // Where it is, the history is replayed and the report says so.
    let moved = call(
        &control,
        3,
        "history/move",
        json!({"entry": "Source", "session": session, "to": "Replay"}),
    )
    .expect("moved");
    assert_eq!(moved["replayed"], true, "{moved}");
    let kinds: Vec<&str> = moved["losses"]
        .as_array()
        .expect("losses")
        .iter()
        .filter_map(|loss| loss["kind"].as_str())
        .collect();
    assert!(kinds.contains(&"replayed_as_prompt"), "{kinds:?}");

    // The new session opens by its id and its first prompt carries the context.
    let target = host.local.open("Replay").expect("a link");
    ask(
        &target,
        1,
        "initialize",
        json!({"protocolVersion": 1, "clientCapabilities": {}}),
    )
    .expect("initialized");
    let new = moved["session"].as_str().expect("a session");
    ask(
        &target,
        2,
        "session/resume",
        json!({"sessionId": new, "cwd": dir, "mcpServers": []}),
    )
    .expect("the replayed session opens");
    let (turn, _) = ask(
        &target,
        3,
        "session/prompt",
        json!({"sessionId": new, "prompt": [{"type": "text", "text": "again"}]}),
    )
    .expect("a turn");
    assert_eq!(turn["stopReason"], "end_turn");
    host.stop();
}
