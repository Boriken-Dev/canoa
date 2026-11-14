//! A host hooking a real Claude Code, through the adapter `canoa agent claude`, against a stand-in model on loopback: what
//! the overlay's hooks are run for that ACP does not show (a working directory that changed, a compaction, a session that
//! starts again after one), and what a hook may do about it.
//!
//! Nothing is spent and nothing of the person's is read: the program's environment is built from nothing under a directory of
//! the test's own, its base URL is a loopback port, its key is a dummy, and its traffic otherwise goes to a closed port.
//! The tests need a Claude Code and a stand-in for its model, so they are ignored without these variables, and a run counts
//! them as not run:
//!
//! - `CANOA_TEST_CLAUDE`: the Claude Code executable.
//! - `CANOA_TEST_STANDIN_MODEL`: a script that serves the Anthropic Messages API on 127.0.0.1, as `SCRIPT PORT_FILE LOG_FILE
//!   STEPS_JSON` (it writes its port to `PORT_FILE`; a step is `{"tools": [[name, input]]}` or `{"text": ...}`).
//! - `CANOA_TEST_PYTHON`: a Python 3 that runs it.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod hooked;
mod linked;
mod support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use hooked::{Allow, Arm, Setup, hooks, start_program};
use serde_json::{Value, json};

use crate::support::host_binary;

const NEEDS: &str =
    "NOT RUN: set CANOA_TEST_CLAUDE, CANOA_TEST_STANDIN_MODEL and CANOA_TEST_PYTHON";

fn variable(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{NEEDS}"))
}

/// A directory of the run's own that is not under the temporary directory: Claude Code run under it loses its shell's `cd`s.
fn run_root() -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "native-hooks-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("a directory");
    root
}

/// The stand-in model and everything the program is given, removed with it.
struct Run {
    root: PathBuf,
    model: Child,
    setup: Setup,
    session: Option<String>,
    next: u64,
}

impl Drop for Run {
    fn drop(&mut self) {
        self.setup.host.stop();
        let _ = self.model.kill();
        let _ = self.model.wait();
        // The program may still hold a file for a moment after the host has gone.
        for _ in 0..50 {
            if std::fs::remove_dir_all(&self.root).is_ok() || !self.root.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

/// A Claude Code on the stand-in, hooked by overlay `team` (`manifest`, `arms`), allowed by canoa.cfg.
fn claude(manifest: &str, arms: &[(&str, Arm)], steps: &Value) -> Run {
    let (claude, standin, python) = (
        variable("CANOA_TEST_CLAUDE"),
        variable("CANOA_TEST_STANDIN_MODEL"),
        variable("CANOA_TEST_PYTHON"),
    );
    let root = run_root();
    for dir in ["w/sub", "w/.git", "c", "h", "t", "l"] {
        std::fs::create_dir_all(root.join(dir)).expect("a directory");
    }
    std::fs::write(root.join("w/sub/x.txt"), "x").expect("written");
    std::fs::write(root.join("l/steps.json"), steps.to_string()).expect("written");
    let (port_file, log) = (root.join("l/port"), root.join("l/model.jsonl"));
    let model = Command::new(&python)
        .args(["-I", &standin])
        .args([&port_file, &log, &root.join("l/steps.json")])
        .stdin(Stdio::null())
        .spawn()
        .expect("the stand-in model starts");
    let until = Instant::now() + Duration::from_secs(20);
    let port = loop {
        if let Ok(text) = std::fs::read_to_string(&port_file)
            && !text.trim().is_empty()
        {
            break text.trim().to_owned();
        }
        assert!(Instant::now() < until, "the stand-in model gives its port");
        std::thread::sleep(Duration::from_millis(50));
    };
    std::fs::write(
        root.join("c/settings.json"),
        json!({"skipWebFetchPreflight": true, "permissions": {"allow": ["Bash", "PowerShell"]}})
            .to_string(),
    )
    .expect("written");
    let home = root.join("h");
    let mut env: BTreeMap<&str, String> = BTreeMap::new();
    let text = |path: &Path| path.to_string_lossy().into_owned();
    for (name, value) in [
        ("HOME", text(&home)),
        ("USERPROFILE", text(&home)),
        ("APPDATA", text(&home.join("AppData/Roaming"))),
        ("LOCALAPPDATA", text(&home.join("AppData/Local"))),
        ("TEMP", text(&root.join("t"))),
        ("TMP", text(&root.join("t"))),
        ("TMPDIR", text(&root.join("t"))),
        ("TZ", "UTC".to_owned()),
        ("ANTHROPIC_BASE_URL", format!("http://127.0.0.1:{port}")),
        ("ANTHROPIC_API_KEY", "dummy-stand-in-key".to_owned()),
        ("CLAUDE_CONFIG_DIR", text(&root.join("c"))),
        ("DISABLE_TELEMETRY", "1".to_owned()),
        ("DISABLE_AUTOUPDATER", "1".to_owned()),
        ("DISABLE_ERROR_REPORTING", "1".to_owned()),
        ("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1".to_owned()),
        ("CLAUDE_CODE_DISABLE_CLAUDE_MDS", "1".to_owned()),
        ("HTTPS_PROXY", "http://127.0.0.1:9".to_owned()),
        ("NO_PROXY", "127.0.0.1,localhost,::1".to_owned()),
    ] {
        env.insert(name, value);
    }
    if cfg!(windows) {
        env.insert(
            "PATH",
            "C:/Windows/System32;C:/Windows;C:/Windows/System32/WindowsPowerShell/v1.0".to_owned(),
        );
        env.insert(
            "CLAUDE_CODE_GIT_BASH_PATH",
            "C:/Program Files/Git/bin/bash.exe".to_owned(),
        );
        env.insert("CLAUDE_CODE_USE_POWERSHELL_TOOL", "1".to_owned());
        for name in [
            "SYSTEMROOT",
            "WINDIR",
            "COMSPEC",
            "PATHEXT",
            "SYSTEMDRIVE",
            "PROCESSOR_ARCHITECTURE",
            "NUMBER_OF_PROCESSORS",
            "OS",
        ] {
            if let Ok(value) = std::env::var(name) {
                env.insert(name, value);
            }
        }
    } else {
        env.insert("PATH", "/usr/bin:/bin".to_owned());
    }
    // The agent is `canoa agent claude`, started with exactly this environment, through a script that sets it.
    let argv = json!([
        host_binary(),
        "agent",
        "claude",
        "--executable",
        claude,
        "--setting-sources",
        "user,project",
        "--allow-client-plugins"
    ]);
    std::fs::write(
        root.join("agent.json"),
        json!({"argv": argv, "env": env}).to_string(),
    )
    .expect("written");
    std::fs::write(
        root.join("agent.py"),
        "import json, subprocess, sys\n\
         config = json.load(open(sys.argv[1], encoding='utf-8'))\n\
         sys.exit(subprocess.call(config['argv'], env=config['env']))\n",
    )
    .expect("written");
    let mut setup = start_program(
        manifest,
        arms,
        &Allow::Digest,
        &python,
        &[
            root.join("agent.py").to_string_lossy().into_owned(),
            root.join("agent.json").to_string_lossy().into_owned(),
        ],
        false,
    );
    setup.cwd = root.join("w").to_string_lossy().into_owned();
    Run {
        root,
        model,
        setup,
        session: None,
        next: 10,
    }
}

impl Run {
    /// Opens the session, once the agent has said what it is.
    fn open(&mut self) {
        self.setup.initialize(false);
        let opened = self.setup.new_session(2).expect("a session opens");
        self.session = opened["sessionId"].as_str().map(str::to_owned);
        assert!(self.session.is_some(), "{opened}");
    }

    /// A prompt, with every permission it asks for allowed: how it ended.
    fn say(&mut self, text: &str) -> Result<Value, Value> {
        let id = self.next;
        self.next += 1;
        let session = self.session.clone().expect("a session");
        self.setup.turn(id, &session, text, "allow").0
    }

    /// The payloads the overlay's hook logged for `event`.
    fn logged(&self, event: &str) -> Vec<Value> {
        hooked::logged(&self.setup.log)
            .into_iter()
            .filter(|(each, _)| each == event)
            .map(|(_, payload)| payload)
            .collect()
    }

    /// Waits for `count` payloads of `event`, and returns them.
    fn wait_for(&self, event: &str, count: usize) -> Vec<Value> {
        self.setup.events(event, count)
    }
}

fn bash(command: &str) -> Value {
    json!(["Bash", {"command": command, "description": "stand-in"}])
}

fn same_place(path: &Value, tail: &str) -> bool {
    path.as_str().is_some_and(|path| {
        path.replace('\\', "/")
            .trim_end_matches('/')
            .ends_with(tail)
    })
}

#[test]
#[ignore = "NOT RUN: set CANOA_TEST_CLAUDE, CANOA_TEST_STANDIN_MODEL and CANOA_TEST_PYTHON"]
fn a_cd_in_a_real_claude_runs_the_cd_hook_with_where_it_was() {
    let mut run = claude(
        &hooks("cd = {}"),
        &[],
        &json!([{"tools": [bash("cd sub && pwd")]}, {"text": "over"}]),
    );
    run.open();
    run.say("go").expect("the turn ends");
    let moves = run.wait_for("cd", 1);
    assert_eq!(moves.len(), 1, "{:?}", hooked::logged(&run.setup.log));
    assert!(same_place(&moves[0]["previous"], "/w"), "{}", moves[0]);
    assert!(same_place(&moves[0]["cwd"], "/w/sub"), "{}", moves[0]);
    assert_eq!(moves[0]["event"], "cd");
    assert!(
        moves[0]["session"]
            .as_str()
            .is_some_and(|id| id.starts_with("canoa-session-"))
    );
}

#[test]
#[ignore = "NOT RUN: set CANOA_TEST_CLAUDE, CANOA_TEST_STANDIN_MODEL and CANOA_TEST_PYTHON"]
fn a_compact_in_a_real_claude_runs_pre_compact_then_session_start_for_compact() {
    let mut run = claude(
        &hooks("pre-compact = {}\nsession-start = { match = [\"compact\"] }"),
        &[(
            "session-start",
            Arm::Print(r#"{"context":"AFTER-COMPACT"}"#.to_owned()),
        )],
        &json!([{"text": "first turn over"}]),
    );
    run.open();
    run.say("go").expect("the turn ends");
    assert!(
        run.logged("pre-compact").is_empty(),
        "nothing compacts by itself"
    );
    run.say("/compact").expect("the compaction ends");
    let before = run.wait_for("pre-compact", 1);
    assert_eq!(before.len(), 1, "{:?}", hooked::logged(&run.setup.log));
    assert_eq!(before[0]["trigger"], "manual");
    let after = run.wait_for("session-start", 1);
    assert_eq!(after.len(), 1, "{:?}", hooked::logged(&run.setup.log));
    assert_eq!(after[0]["reason"], "compact");
    // The hook's output is the host's to apply: it went to the person's next prompt, never to the harness's own hook.
    let events: Vec<String> = hooked::logged(&run.setup.log)
        .into_iter()
        .map(|(event, _)| event)
        .collect();
    let (pre, start) = (
        events.iter().position(|event| event == "pre-compact"),
        events.iter().position(|event| event == "session-start"),
    );
    assert!(pre < start, "{events:?}");
}

#[test]
#[ignore = "NOT RUN: set CANOA_TEST_CLAUDE, CANOA_TEST_STANDIN_MODEL and CANOA_TEST_PYTHON"]
fn a_hook_that_blocks_a_compaction_stops_it() {
    let mut run = claude(
        &hooks("pre-compact = {}\nsession-start = { match = [\"compact\"] }"),
        &[(
            "pre-compact",
            Arm::Print(r#"{"decision":"block","reason":"not now"}"#.to_owned()),
        )],
        &json!([{"text": "first turn over"}]),
    );
    run.open();
    run.say("go").expect("the turn ends");
    let _ = run.say("/compact");
    assert_eq!(run.wait_for("pre-compact", 1).len(), 1);
    // The harness's own hook ran before the answer to the prompt came back: a compaction that went on would have started a
    // session again by now.
    std::thread::sleep(Duration::from_secs(2));
    assert!(
        run.logged("session-start")
            .iter()
            .all(|payload| payload["reason"] != "compact")
    );
}

#[test]
#[ignore = "NOT RUN: set CANOA_TEST_CLAUDE, CANOA_TEST_STANDIN_MODEL and CANOA_TEST_PYTHON"]
fn a_clear_in_a_real_claude_never_reaches_a_session_start_hook() {
    let mut run = claude(
        &hooks("session-start = { match = [\"clear\"] }"),
        &[],
        &json!([{"text": "first turn over"}]),
    );
    run.open();
    run.say("go").expect("the turn ends");
    // Over ACP the adapter takes the new session for a stranger's and ends: the prompt fails, and no start follows.
    let _ = run.say("/clear");
    std::thread::sleep(Duration::from_secs(2));
    assert!(
        run.logged("session-start").is_empty(),
        "{:?}",
        hooked::logged(&run.setup.log)
    );
}
