//! The host records each session it serves in the session-history format, exports it over `@control`, and moves it to another
//! source: here Claude Code to Codex, each a stand-in for its command line, with no model and no Node. One test in this file,
//! since it sets the environment the providers read their session files from.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    unsafe_code,
    reason = "tests name what they use from std, and a test sets the environment its host reads"
)]

mod linked;
mod support;

use std::path::Path;
use std::time::{Duration, Instant};

use linked::{Hosted, ask, next, open_session, send};
use serde_json::{Value, json};
use support::{Scratch, cwd, example, quoted};

fn env(name: &str, value: &Path) {
    // SAFETY: the only test in this process, set before any thread of the host starts.
    unsafe { std::env::set_var(name, value) };
}

fn standin(scratch: &Scratch, name: &str) -> String {
    let path = scratch
        .0
        .join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(example("harness_standin", false), &path).expect("the stand-in is copied");
    path.to_string_lossy().into_owned()
}

/// One turn, its one permission allowed: what the agent said.
fn turn(link: &canoa_host::Link, session: &str, text: &str) -> String {
    send(
        link,
        &json!({"jsonrpc": "2.0", "id": 9, "method": "session/prompt",
            "params": {"sessionId": session, "prompt": [{"type": "text", "text": text}]}}),
    );
    let mut said = String::new();
    loop {
        let message = next(link).expect("the turn goes on");
        if message["method"] == "session/request_permission" {
            let allow = message["params"]["options"]
                .as_array()
                .and_then(|options| options.iter().find(|option| option["kind"] == "allow_once"))
                .expect("an allow option")
                .clone();
            send(
                link,
                &json!({"jsonrpc": "2.0", "id": message["id"],
                    "result": {"outcome": {"outcome": "selected", "optionId": allow["optionId"]}}}),
            );
        } else if message["params"]["update"]["sessionUpdate"] == "agent_message_chunk" {
            said.push_str(
                message["params"]["update"]["content"]["text"]
                    .as_str()
                    .unwrap_or(""),
            );
        } else if message["id"] == 9 {
            assert_eq!(message["result"]["stopReason"], "end_turn", "{message}");
            return said;
        }
    }
}

/// The control call's result, or its error as `(code, message)`.
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

/// The history of `session` once the host has kept its turn.
fn exported(link: &canoa_host::Link, entry: &str, session: &str) -> Value {
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        match call(
            link,
            100,
            "history/export",
            json!({"entry": entry, "session": session}),
        ) {
            Ok(result) => return result,
            Err((-32003, _)) if Instant::now() < until => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(other) => panic!("{other:?}"),
        }
    }
}

fn files_under(dir: &Path) -> usize {
    std::fs::read_dir(dir).map_or(0, |entries| {
        entries
            .flatten()
            .map(|each| {
                if each.path().is_dir() {
                    files_under(&each.path())
                } else {
                    1
                }
            })
            .sum()
    })
}

#[test]
fn a_session_is_recorded_exported_and_moved_from_claude_to_codex() {
    let root = Scratch::new();
    let (claude_home, codex_home) = (root.0.join("claude-home"), root.0.join("codex-home"));
    std::fs::create_dir_all(&claude_home).expect("dir");
    std::fs::create_dir_all(&codex_home).expect("dir");
    env("CLAUDE_CONFIG_DIR", &claude_home);
    env("CODEX_HOME", &codex_home);
    let config = format!(
        "[host.agents.Claude]\nharness = \"claude\"\nexecutable = {}\nlisten = \"127.0.0.1:0\"\n\
         [host.agents.Codex]\nharness = \"codex\"\nexecutable = {}\nlisten = \"127.0.0.1:0\"\n",
        quoted(&standin(&root, "claude")),
        quoted(&standin(&root, "codex")),
    );
    let host = Hosted::start(&config);
    let dir = cwd(&host.scratch);

    // Recorded as it runs.
    let claude = host.local.open("Claude").expect("a link");
    let session = open_session(&claude, &dir);
    assert_eq!(turn(&claude, &session, "write it"), "Done: write it");
    let control = host.local.open("@control").expect("the control channel");
    let history = exported(&control, "Claude", &session);
    let text = history["text"].as_str().expect("text");
    assert!(text.contains("\"coqui-acp-history\""), "{text}");
    assert!(
        text.contains("write it") && text.contains("Done: write it"),
        "{text}"
    );
    assert!(history.get("next").is_none(), "a small history is one page");

    // The recording is in the host's home, in files named by hashes, and nothing of it is in the working directory.
    let kept = host.scratch.0.join("home").join("history");
    assert_eq!(files_under(&kept), 1);
    for each in std::fs::read_dir(&kept).expect("dir").flatten() {
        let name = each.file_name().to_string_lossy().into_owned();
        assert!(
            name.ends_with(".jsonl") && !name.contains(&session),
            "{name}"
        );
    }

    // Moved to Codex: a native import, a new session, the loss report.
    let moved = call(
        &control,
        101,
        "history/move",
        json!({"entry": "Claude", "session": session, "to": "Codex"}),
    )
    .expect("moved");
    let new = moved["session"].as_str().expect("a session");
    assert_ne!(new, session);
    assert_eq!(moved["entry"], "Codex");
    assert_eq!(moved["replayed"], false, "{moved}");
    assert!(moved["losses"].is_array(), "{moved}");
    assert!(files_under(&codex_home) > 0, "Codex has a rollout of it");
    // The moved session has a history of its own, which can be exported and moved again.
    let carried = call(
        &control,
        102,
        "history/export",
        json!({"entry": "Codex", "session": new}),
    )
    .expect("exported");
    assert!(
        carried["text"]
            .as_str()
            .is_some_and(|text| text.contains("write it"))
    );
    assert_eq!(carried["id"], new);

    // Paged: a page holds whole lines and says where the next begins.
    let first = call(
        &control,
        103,
        "history/export",
        json!({"entry": "Claude", "session": session, "max_bytes": 1000}),
    )
    .expect("a page");
    let mut pages = 1;
    let mut joined = first["text"].as_str().expect("text").to_owned();
    let mut after = first.get("next").and_then(Value::as_str).map(str::to_owned);
    while let Some(position) = after {
        let page = call(
            &control,
            104,
            "history/export",
            json!({"entry": "Claude", "session": session, "max_bytes": 1000, "after": position}),
        )
        .expect("the next page");
        joined.push_str(page["text"].as_str().expect("text"));
        after = page.get("next").and_then(Value::as_str).map(str::to_owned);
        pages += 1;
    }
    assert!(pages > 1);
    assert_eq!(joined, text);

    // What is refused, and how.
    let refused =
        |params: Value, method: &str| call(&control, 200, method, params).expect_err("refused").0;
    assert_eq!(
        refused(
            json!({"entry": "Claude", "session": "nope"}),
            "history/export"
        ),
        -32003
    );
    assert_eq!(
        refused(
            json!({"entry": "Claude", "session": session, "to": "Nowhere"}),
            "history/move"
        ),
        -32003
    );
    assert_eq!(
        refused(
            json!({"entry": "Claude", "session": session}),
            "history/move"
        ),
        -32602
    );
    assert_eq!(
        refused(
            json!({"entry": "Claude", "session": session, "max_bytes": 0}),
            "history/export"
        ),
        -32602
    );
    assert_eq!(
        refused(
            json!({"entry": "Claude", "session": session, "to": "Codex", "cwd": "relative/dir"}),
            "history/move"
        ),
        -32602
    );
    // Importing at a new UUID each time: a second move is another session, never an overwrite.
    let again = call(
        &control,
        105,
        "history/move",
        json!({"entry": "Claude", "session": session, "to": "Codex"}),
    )
    .expect("moved again");
    assert_ne!(again["session"], moved["session"]);

    // The moved session opens in Codex by its id and carries what was said.
    let codex = host.local.open("Codex").expect("a link");
    ask(
        &codex,
        1,
        "initialize",
        json!({"protocolVersion": 1, "clientCapabilities": {}}),
    )
    .expect("initialized");
    let (opened, updates) = ask(
        &codex,
        2,
        "session/resume",
        json!({"sessionId": new, "cwd": dir, "mcpServers": []}),
    )
    .or_else(|_| {
        ask(
            &codex,
            3,
            "session/load",
            json!({"sessionId": new, "cwd": dir, "mcpServers": []}),
        )
    })
    .expect("the moved session opens");
    let _ = (opened, updates);
    host.stop();
}
