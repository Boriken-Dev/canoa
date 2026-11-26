//! A conversation goes on from before one of its turns: `history/move` with `before` carries the turns that came first into a new
//! session of the same agent, and the session it leaves is as it was. Claude Code runs as a stand-in for its command line, in a
//! throwaway configuration folder; nothing of the person's is read. One test in this file, since it sets the environment the
//! provider reads its session files from.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    unsafe_code,
    reason = "tests name what they use from std, and a test sets the environment its host reads"
)]

mod linked;
mod support;

use std::path::{Path, PathBuf};

use linked::{Hosted, next, send};
use serde_json::{Value, json};
use support::{Scratch, cwd, example, quoted};

const SESSION: &str = "6f2c1b0e-0000-4000-8000-00000000c0de";
const TURNS: [(&str, &str); 3] = [
    ("Fix the failing build", "answer-to-the-build"),
    ("Now add a test for it", "answer-about-the-test"),
    ("Rename the helper", "answer-about-the-name"),
];

fn env(name: &str, value: &Path) {
    // SAFETY: the only test in this process, set before any thread of the host starts.
    unsafe { std::env::set_var(name, value) };
}

fn project_of(work: &str) -> String {
    work.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// A Claude Code session file with one user line and one assistant line for each of [`TURNS`].
fn transcript(work: &str) -> String {
    let mut out = String::new();
    let mut parent = Value::Null;
    for (n, (asked, answered)) in TURNS.iter().enumerate() {
        let user_id = format!("11111111-1111-4111-8111-11111111111{n}");
        let assistant_id = format!("22222222-2222-4222-8222-22222222222{n}");
        let user = json!({"parentUuid": parent, "isSidechain": false, "type": "user",
            "message": {"role": "user", "content": asked},
            "uuid": user_id, "timestamp": format!("2026-11-05T09:4{n}:11.335Z"),
            "cwd": work, "sessionId": SESSION, "version": "2.1.318"});
        let assistant = json!({"parentUuid": user_id, "isSidechain": false,
            "type": "assistant", "uuid": assistant_id, "timestamp": format!("2026-11-05T09:4{n}:12.000Z"),
            "message": {"id": format!("msg_{n}"), "type": "message", "role": "assistant", "model": "claude-x",
                "content": [{"type": "text", "text": answered}], "stop_reason": "end_turn"}});
        out.push_str(&format!("{user}\n{assistant}\n"));
        parent = json!(assistant_id);
    }
    out
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
fn a_session_goes_on_from_before_a_turn_in_a_new_session_and_the_old_one_stays() {
    let root = Scratch::new();
    let (claude_home, codex_home, pi_home) = (
        root.0.join("claude-home"),
        root.0.join("codex-home"),
        root.0.join("pi-home"),
    );
    for home in [&claude_home, &codex_home, &pi_home] {
        std::fs::create_dir_all(home).expect("dir");
    }
    std::fs::write(claude_home.join(".credentials.json"), "{\"fixture\":true}").expect("a login");
    env("CLAUDE_CONFIG_DIR", &claude_home);
    env("CODEX_HOME", &codex_home);
    env("PI_CODING_AGENT_DIR", &pi_home);
    let standin = root
        .0
        .join(format!("claude{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(example("harness_standin", false), &standin).expect("the stand-in is copied");
    let config = format!(
        "[host.agents.Claude]\nharness = \"claude\"\nexecutable = {}\nlisten = \"127.0.0.1:0\"\n\
         [host.history]\nrecord = false\n",
        quoted(&standin.to_string_lossy()),
    );
    let host = Hosted::start_in(root, &config, &[]);
    let dir = cwd(&host.scratch);
    let file = claude_home
        .join("projects")
        .join(project_of(&dir))
        .join(format!("{SESSION}.jsonl"));
    std::fs::create_dir_all(file.parent().expect("a parent")).expect("a folder");
    let original = transcript(&dir);
    std::fs::write(&file, &original).expect("the session file");
    let control = host.local.open("@control").expect("the control channel");

    // Before the third turn: the first two are in the new session, the third is not.
    let rewound = call(
        &control,
        1,
        "history/move",
        json!({"entry": "Claude", "session": SESSION, "to": "Claude", "cwd": dir,
               "before": 2, "expect": TURNS[2].0}),
    )
    .expect("rewound");
    let new = rewound["session"].as_str().expect("a session");
    assert_ne!(new, SESSION);
    let written = file_of(&claude_home, new).expect("a transcript of it");
    for (asked, answered) in &TURNS[..2] {
        assert!(
            written.contains(asked) && written.contains(answered),
            "{written}"
        );
    }
    for (asked, answered) in &TURNS[2..] {
        assert!(
            !written.contains(asked) && !written.contains(answered),
            "{written}"
        );
    }
    assert_eq!(
        std::fs::read_to_string(&file).expect("the old one"),
        original,
        "the session it leaves is as it was"
    );

    // Before the second turn: only the first.
    let earlier = call(
        &control,
        2,
        "history/move",
        json!({"entry": "Claude", "session": SESSION, "to": "Claude", "cwd": dir, "before": 1}),
    )
    .expect("rewound");
    let written = file_of(
        &claude_home,
        earlier["session"].as_str().expect("a session"),
    )
    .expect("a transcript of it");
    assert!(
        written.contains(TURNS[0].1) && !written.contains(TURNS[1].0),
        "{written}"
    );

    // A turn the history does not have, the first (nothing before it), and one that is not what the caller meant are refused.
    for (n, params) in [
        json!({"before": 3}),
        json!({"before": 0}),
        json!({"before": 1, "expect": TURNS[2].0}),
        json!({"before": "one"}),
    ]
    .into_iter()
    .enumerate()
    {
        let mut ask = json!({"entry": "Claude", "session": SESSION, "to": "Claude", "cwd": dir});
        ask.as_object_mut()
            .expect("an object")
            .extend(params.as_object().expect("an object").clone());
        let refused = call(&control, 10 + n as u64, "history/move", ask)
            .expect_err("a turn that is not there or not meant is refused");
        assert_eq!(refused.0, -32602, "{refused:?}");
    }
    host.stop();
}
