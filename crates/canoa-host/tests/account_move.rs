//! A conversation goes from one account of an entry to another: where the accounts keep their conversations in one place it only
//! resumes, and where each has a folder of its own the provider exports it from the first's and imports it into the second's.
//! Stand-ins for Claude Code's login, two throwaway config folders and a recorded session file; nothing of the person's.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    unsafe_code,
    reason = "tests name what they use from std, and a test sets the environment its host reads"
)]

mod linked;
mod support;

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use linked::{Hosted, next, send};
use serde_json::{Value, json};
use sha2::Digest as _;
use support::{Scratch, cwd, example, host_binary, quoted};

const SESSION: &str = "6f2c1b0e-0000-4000-8000-00000000c0de";
const ASKED: &str = "Fix the failing build";
const ANSWERED: &str = "the-answer-that-must-follow-the-conversation";

fn canoa(scratch: &Scratch, args: &[&str], input: &str) -> Output {
    let mut child = Command::new(host_binary())
        .args(args)
        .arg("--config")
        .arg(scratch.0.join("canoa.cfg"))
        .env("COQUI_AGENTIC_PEER_HOME", scratch.0.join("home"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("canoa runs");
    let mut stdin = child.stdin.take().expect("its input");
    stdin.write_all(input.as_bytes()).expect("input written");
    drop(stdin);
    child.wait_with_output().expect("canoa ends")
}

fn add(scratch: &Scratch, name: &str, method: &str, input: &str) {
    let added = canoa(
        scratch,
        &[
            "accounts",
            "add",
            name,
            "--harness",
            "claude",
            "--method",
            method,
        ],
        input,
    );
    assert!(
        added.status.success(),
        "{}",
        String::from_utf8_lossy(&added.stderr)
    );
}

/// The account's folder in the host's home, from `accounts list`.
fn folder_of(scratch: &Scratch, name: &str) -> PathBuf {
    let listed = canoa(scratch, &["accounts", "list", "--format", "json"], "");
    for line in String::from_utf8_lossy(&listed.stdout).lines() {
        let account: Value = serde_json::from_str(line).expect("one JSON object a line");
        if account["name"] == name {
            let id = account["id"].as_str().expect("an id");
            // The host names an account's folder by a digest of its id.
            let digest: String = sha2::Sha256::digest(id.as_bytes())
                .iter()
                .take(12)
                .map(|byte| format!("{byte:02x}"))
                .collect();
            return scratch
                .0
                .join("home")
                .join("accounts")
                .join(digest)
                .join("state");
        }
    }
    panic!("no account {name}");
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

fn call(link: &canoa_host::Link, id: u64, params: &Value) -> Result<Value, (i64, String)> {
    send(
        link,
        &json!({"jsonrpc": "2.0", "id": id, "method": "history/account", "params": params}),
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

/// The session files under `folder`, with their text.
fn sessions_in(folder: &Path) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut pending = vec![folder.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for each in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = each.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|ext| ext == "jsonl") {
                found.push((
                    path.file_stem()
                        .expect("a name")
                        .to_string_lossy()
                        .into_owned(),
                    std::fs::read_to_string(&path).unwrap_or_default(),
                ));
            }
        }
    }
    found
}

#[test]
fn a_conversation_follows_the_person_to_another_account_by_the_providers_own_export_and_import() {
    let scratch = Scratch::new();
    // The accounts without a folder of their own keep conversations where the program's own configuration is: here, a throwaway.
    let default_folder = scratch.0.join("default-config");
    std::fs::create_dir_all(&default_folder).expect("a folder");
    // SAFETY: the only test in this process, set before any thread of the host starts.
    unsafe { std::env::set_var("CLAUDE_CONFIG_DIR", &default_folder) };
    let claude = scratch
        .0
        .join(format!("claude{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(example("harness_standin", false), &claude).expect("the stand-in is copied");
    std::fs::write(
        scratch.0.join("canoa.cfg"),
        format!(
            "[host]\naccount_store = \"file\"\n\n[host.agents.Claude]\nharness = \"claude\"\nexecutable = {}\nlisten = \"127.0.0.1:0\"\n",
            quoted(&claude.to_string_lossy())
        ),
    )
    .expect("canoa.cfg is written");
    add(&scratch, "work", "subscription", "123456\n");
    add(&scratch, "other", "subscription", "123456\n");
    add(&scratch, "key1", "api-key", "sk-test-key-0001\n");
    add(&scratch, "key2", "api-key", "sk-test-key-0002\n");
    add(&scratch, "key3", "api-key", "sk-test-key-0003\n");
    let cfg = std::fs::read_to_string(scratch.0.join("canoa.cfg")).expect("canoa.cfg");
    let cfg = cfg.replace(
        "listen = \"127.0.0.1:0\"",
        "listen = \"127.0.0.1:0\"\naccounts = [\"work\", \"key1\", \"key2\"]",
    );
    let dir = cwd(&scratch);
    let (work, other) = (folder_of(&scratch, "work"), folder_of(&scratch, "other"));
    // A recorded session of the first account: where Claude Code keeps it, in that account's own folder.
    let file = work
        .join("projects")
        .join(project_of(&dir))
        .join(format!("{SESSION}.jsonl"));
    std::fs::create_dir_all(file.parent().expect("a parent")).expect("a folder");
    std::fs::write(&file, transcript(&dir)).expect("the session file");

    let host = Hosted::start_in(scratch, &cfg, &[]);
    let control = host.local.open("@control").expect("the control channel");
    let ask = |from: &str, to: &str| {
        call(
            &control,
            1,
            &json!({"entry": "Claude", "session": SESSION, "cwd": dir, "from": from, "to": to}),
        )
    };

    // `other` is not listed by the entry: a move is the person's own, so it may go there all the same.
    // One folder each: exported from the first account's, imported into the second's at a new id.
    let moved = ask("work", "other").expect("moved");
    assert_eq!(moved["moved"], true, "{moved}");
    assert_eq!(
        moved["via"], "provider",
        "the provider's own export, not the host's record: {moved}"
    );
    let new = moved["session"].as_str().expect("a session id").to_owned();
    assert_ne!(new, SESSION);
    let arrived = sessions_in(&other);
    assert_eq!(arrived.len(), 1, "{arrived:?}");
    assert!(
        arrived[0].1.contains(ASKED) && arrived[0].1.contains(ANSWERED),
        "what was asked and answered went with it: {}",
        arrived[0].1
    );
    assert_eq!(
        sessions_in(&work).len(),
        1,
        "the first account still has it"
    );
    assert!(moved["losses"].is_array());

    // Accounts that keep conversations in one place: nothing moves, and the session resumes as it is.
    let same = ask("key1", "key2").expect("answered");
    assert_eq!(same["moved"], false, "{same}");
    assert_eq!(same["session"], SESSION);

    // From a folder to the program's own place: moved into it.
    let home = ask("work", "key1").expect("moved to the shared place");
    assert_eq!(home["moved"], true, "{home}");
    assert_eq!(sessions_in(&default_folder).len(), 1);

    // To the login the program has where it runs, and from it to an unlisted account with a folder of its own.
    let machine = call(
        &control,
        3,
        &json!({"entry": "Claude", "session": SESSION, "cwd": dir, "from": "work", "to": null}),
    )
    .expect("moved to the machine's login");
    assert_eq!(machine["moved"], true, "{machine}");
    let back = machine["session"]
        .as_str()
        .expect("a session id")
        .to_owned();
    let away = call(
        &control,
        4,
        &json!({"entry": "Claude", "session": back, "cwd": dir, "from": null, "to": "other"}),
    )
    .expect("moved from the machine's login");
    assert_eq!(away["moved"], true, "{away}");
    // An unlisted account whose key reaches the agent has to be offered to the entry first.
    let keyed = ask("work", "key3").expect_err("refused");
    assert_eq!(keyed.0, -32003, "{keyed:?}");

    // What cannot be exported is refused, with the account and the reason, never made up.
    let missing = call(
        &control,
        2,
        &json!({"entry": "Claude", "session": "6f2c1b0e-0000-4000-8000-0000000000aa", "cwd": dir, "from": "work", "to": "other"}),
    )
    .expect_err("refused");
    assert_eq!(missing.0, -32002, "{missing:?}");
    assert!(
        missing.1.contains("account work") && missing.1.contains("cannot export"),
        "{missing:?}"
    );
    let unknown = ask("work", "nobody").expect_err("refused");
    assert_eq!(unknown.0, -32003, "{unknown:?}");
    assert_eq!(ask("work", "work").expect_err("refused").0, -32602);

    host.stop();
}
