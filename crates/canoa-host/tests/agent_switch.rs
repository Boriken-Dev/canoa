//! A conversation changes agent: the first agent's own session is exported, imported into the other at a new id, and carried
//! back the same way, so each holds every turn. Claude Code and Codex run as stand-ins for their command lines, in throwaway
//! configuration folders; nothing of the person's is read. One test in this file, since it sets the environment the providers
//! read their session files from.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    unsafe_code,
    reason = "tests name what they use from std, and a test sets the environment its host reads"
)]

mod linked;
mod support;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use linked::{Hosted, ask, next, send};
use serde_json::{Value, json};
use support::{Scratch, cwd, example, quoted};

const SESSION: &str = "6f2c1b0e-0000-4000-8000-00000000c0de";
const ASKED: &str = "Fix the failing build";
const ANSWERED: &str = "the-answer-that-must-follow-the-conversation";

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

/// Where Claude Code keeps the sessions of `work`: every character but an ASCII letter or digit as a dash.
fn project_of(work: &str) -> String {
    work.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

fn transcript(work: &str) -> String {
    let user = json!({"parentUuid": null, "isSidechain": false, "type": "user",
        "message": {"role": "user", "content": ASKED},
        "uuid": "11111111-1111-4111-8111-111111111111", "timestamp": "2026-11-05T09:45:11.335Z",
        "cwd": work, "sessionId": SESSION, "version": "2.1.318"});
    let assistant = json!({"parentUuid": "11111111-1111-4111-8111-111111111111", "isSidechain": false,
        "type": "assistant", "uuid": "22222222-2222-4222-8222-222222222222", "timestamp": "2026-11-05T09:45:12.000Z",
        "message": {"id": "msg_1", "type": "message", "role": "assistant", "model": "claude-x",
            "content": [{"type": "text", "text": ANSWERED}], "stop_reason": "end_turn"}});
    format!("{user}\n{assistant}\n")
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
            return said;
        }
    }
}

/// The history of `session` once the host has kept the turn that said `said`.
fn exported(link: &canoa_host::Link, entry: &str, session: &str, said: &str) -> String {
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        match call(
            link,
            100,
            "history/export",
            json!({"entry": entry, "session": session}),
        ) {
            Ok(result) => {
                let text = result["text"].as_str().unwrap_or("").to_owned();
                if text.contains(said) || Instant::now() >= until {
                    return text;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err((-32003, _)) if Instant::now() < until => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(other) => panic!("{other:?}"),
        }
    }
}

/// Every session file under `folder` whose name holds `id`, with its text.
fn file_of(folder: &Path, id: &str) -> Option<String> {
    let mut pending = vec![folder.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for each in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path: PathBuf = each.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.to_string_lossy().contains(id) {
                return std::fs::read_to_string(&path).ok();
            }
        }
    }
    None
}

#[test]
fn a_conversation_goes_from_claude_to_codex_and_back_with_every_turn_in_each() {
    let root = Scratch::new();
    let (claude_home, codex_home) = (root.0.join("claude-home"), root.0.join("codex-home"));
    std::fs::create_dir_all(&claude_home).expect("dir");
    std::fs::create_dir_all(&codex_home).expect("dir");
    env("CLAUDE_CONFIG_DIR", &claude_home);
    env("CODEX_HOME", &codex_home);
    let pi_home = root.0.join("pi-home");
    std::fs::create_dir_all(&pi_home).expect("dir");
    env("PI_CODING_AGENT_DIR", &pi_home);
    let config = format!(
        "[host.agents.Claude]\nharness = \"claude\"\nexecutable = {}\nlisten = \"127.0.0.1:0\"\n\
         [host.agents.Codex]\nharness = \"codex\"\nexecutable = {}\nlisten = \"127.0.0.1:0\"\n\
         [host.agents.Pi]\nharness = \"pi\"\nexecutable = {}\nlisten = \"127.0.0.1:0\"\n",
        quoted(&standin(&root, "claude")),
        quoted(&standin(&root, "codex")),
        quoted(&standin(&root, "pi")),
    );
    let host = Hosted::start_in(root, &config, &[]);
    let dir = cwd(&host.scratch);
    // A session Claude Code has in its own folder, which this host never served, so it has no record of it.
    let file = claude_home
        .join("projects")
        .join(project_of(&dir))
        .join(format!("{SESSION}.jsonl"));
    std::fs::create_dir_all(file.parent().expect("a parent")).expect("a folder");
    std::fs::write(&file, transcript(&dir)).expect("the session file");
    let control = host.local.open("@control").expect("the control channel");

    // The first agent's own export is what moves it.
    let to_codex = call(
        &control,
        1,
        "history/move",
        json!({"entry": "Claude", "session": SESSION, "to": "Codex", "cwd": dir}),
    )
    .expect("moved to Codex");
    assert_eq!(to_codex["via"], "provider", "{to_codex}");
    let on_codex = to_codex["session"].as_str().expect("a session");
    let rollout = file_of(&codex_home, on_codex).expect("Codex has a rollout of it");
    assert!(
        rollout.contains(ASKED) && rollout.contains(ANSWERED),
        "{rollout}"
    );

    // A turn on Codex, which the host records as it goes by.
    let codex = host.local.open("Codex").expect("a link");
    ask(
        &codex,
        1,
        "initialize",
        json!({"protocolVersion": 1, "clientCapabilities": {}}),
    )
    .expect("initialized");
    ask(
        &codex,
        2,
        "session/resume",
        json!({"sessionId": on_codex, "cwd": dir, "mcpServers": []}),
    )
    .or_else(|_| {
        ask(
            &codex,
            3,
            "session/load",
            json!({"sessionId": on_codex, "cwd": dir, "mcpServers": []}),
        )
    })
    .expect("the moved session opens");
    assert_eq!(turn(&codex, on_codex, "now on codex"), "Done: now on codex");
    let recorded = exported(&control, "Codex", on_codex, "Done: now on codex");
    assert!(recorded.contains("now on codex"), "{recorded}");

    // Back: what Codex added is in Claude's own session, at another new id.
    let back = call(
        &control,
        2,
        "history/move",
        json!({"entry": "Codex", "session": on_codex, "to": "Claude", "cwd": dir}),
    )
    .expect("moved back to Claude");
    let on_claude = back["session"].as_str().expect("a session");
    assert_ne!(on_claude, SESSION);
    let written = file_of(&claude_home, on_claude).expect("Claude has a transcript of it");
    for said in [ASKED, ANSWERED, "now on codex", "Done: now on codex"] {
        assert!(written.contains(said), "{said} is in Claude's: {written}");
    }
    assert!(back["losses"].is_array(), "{back}");

    // What each agent can take, as the picker says it.
    let portable = call(&control, 3, "history/portable", json!({})).expect("portable");
    for name in ["Claude", "Codex", "Pi"] {
        assert_eq!(portable["entries"][name]["export"], true, "{portable}");
        assert_eq!(portable["entries"][name]["import"], true, "{portable}");
    }
    host.stop();

    // A host that records nothing carries a conversation by what each agent itself exports: Claude to Pi, Pi to Codex.
    let host = Hosted::start_in(
        Scratch::new(),
        &format!(
            "{config}[host.history]
record = false
"
        ),
        &[],
    );
    let dir = cwd(&host.scratch);
    let file = claude_home
        .join("projects")
        .join(project_of(&dir))
        .join(format!("{SESSION}.jsonl"));
    std::fs::create_dir_all(file.parent().expect("a parent")).expect("a folder");
    std::fs::write(&file, transcript(&dir)).expect("the session file");
    let control = host.local.open("@control").expect("the control channel");
    let to_pi = call(
        &control,
        1,
        "history/move",
        json!({"entry": "Claude", "session": SESSION, "to": "Pi", "cwd": dir}),
    )
    .expect("moved to Pi");
    assert_eq!(to_pi["via"], "provider", "{to_pi}");
    let on_pi = to_pi["session"].as_str().expect("a session");
    let kept = file_of(&pi_home, on_pi).expect("Pi has a session file of it");
    assert!(kept.contains(ASKED) && kept.contains(ANSWERED), "{kept}");
    let pi_to_codex = call(
        &control,
        2,
        "history/move",
        json!({"entry": "Pi", "session": on_pi, "to": "Codex", "cwd": dir}),
    )
    .expect("moved from Pi to Codex");
    assert_eq!(pi_to_codex["via"], "provider", "{pi_to_codex}");
    let rollout = file_of(
        &codex_home,
        pi_to_codex["session"].as_str().expect("a session"),
    )
    .expect("Codex has a rollout of it");
    assert!(
        rollout.contains(ASKED) && rollout.contains(ANSWERED),
        "{rollout}"
    );
    host.stop();
}
