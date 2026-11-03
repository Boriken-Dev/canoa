//! The built `canoa-host` serving a `[host.tools]` entry: Pi's entry from `examples/config/pi.cfg`, unchanged but for its
//! program, which is a fixture shaped like Pi's command line, reached by a client that proves a key as Canoa's chat does.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod support;

use std::path::Path;
use std::time::{Duration, Instant};

use coqui_agentic_peer::auth::Identity;
use coqui_web::websocket::Channel;
use serde_json::{Value, json};
use support::{Host, Rpc, Scratch, WAIT, cwd, example, quoted, running, with_link};

/// `examples/config/pi.cfg` with its program and arguments replaced, and `extra` lines added to the entry.
fn pi_config(args: &[&str], extra: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/config/pi.cfg");
    let shipped = std::fs::read_to_string(path).expect("examples/config/pi.cfg");
    let program = quoted(&example("jsonl_tool", false).to_string_lossy());
    let args = format!(
        "[{}]",
        args.iter()
            .map(|arg| quoted(arg))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let mut lines = Vec::new();
    for line in shipped.lines() {
        if line.starts_with("program = ") {
            lines.push(format!("program = {program}"));
        } else if line.starts_with("args = ") {
            lines.push(format!("args = {args}"));
            lines.push(extra.to_owned());
        } else {
            lines.push(line.to_owned());
        }
    }
    lines.join("\n")
}

/// A tool entry that runs the fixture once per prompt, as Pi's JSON mode would.
fn message_entry(name: &str, args: &[&str], extra: &str) -> String {
    format!(
        r#"[host.tools.{name}]
program = {}
args = [{}]
runs = "message"
unasked = "everything it does"
{extra}

[[host.tools.{name}.events]]
when = {{ type = "session" }}
send = "session"
id = "{{id}}"

[[host.tools.{name}.events]]
when = {{ type = "message_update", "assistantMessageEvent.type" = "text_delta" }}
send = "text"
text = "{{assistantMessageEvent.delta}}"

[[host.tools.{name}.events]]
when = {{ type = "agent_settled" }}
send = "end"
"#,
        quoted(&example("jsonl_tool", false).to_string_lossy()),
        args.iter()
            .map(|arg| quoted(arg))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// What one prompt brought: its answer, or the error, and every update and permission request on the way.
struct Turn {
    answer: Result<Value, Value>,
    updates: Vec<Value>,
    asked: Vec<Value>,
}

impl Turn {
    fn text(&self, kind: &str) -> String {
        self.updates
            .iter()
            .filter(|update| update["sessionUpdate"] == kind)
            .filter_map(|update| update["content"]["text"].as_str())
            .collect()
    }

    fn of(&self, kind: &str) -> Vec<&Value> {
        self.updates
            .iter()
            .filter(|update| update["sessionUpdate"] == kind)
            .collect()
    }

    fn stop(&self) -> &Value {
        &self.answer.as_ref().expect("the turn ended")["stopReason"]
    }
}

/// Sends `text` as a prompt and reads to its answer, answering any permission request with `option`.
fn prompt(rpc: &Rpc<'_, Channel>, session: &str, text: &str, option: &str) -> Turn {
    rpc.send(&json!({
        "jsonrpc": "2.0", "id": "p", "method": "session/prompt",
        "params": {"sessionId": session, "prompt": [{"type": "text", "text": text}]},
    }))
    .expect("sent");
    read_turn(rpc, option)
}

fn read_turn(rpc: &Rpc<'_, Channel>, option: &str) -> Turn {
    let (mut updates, mut asked) = (Vec::new(), Vec::new());
    loop {
        let message = rpc.next_message(WAIT).expect("the turn goes on");
        match message["method"].as_str() {
            Some("session/update") => updates.push(message["params"]["update"].clone()),
            Some("session/request_permission") => {
                asked.push(message["params"].clone());
                rpc.send(&json!({
                    "jsonrpc": "2.0", "id": message["id"],
                    "result": {"outcome": {"outcome": "selected", "optionId": option}},
                }))
                .expect("answered");
            }
            Some(_) => {}
            None if message["id"] == "p" => {
                let answer = match message.get("error") {
                    Some(error) => Err(error.clone()),
                    None => Ok(message["result"].clone()),
                };
                return Turn {
                    answer,
                    updates,
                    asked,
                };
            }
            None => {}
        }
    }
}

fn open(rpc: &Rpc<'_, Channel>, dir: &str) -> (String, Value) {
    rpc.initialize().expect("initialized");
    let opened = rpc
        .request("session/new", json!({"cwd": dir, "mcpServers": []}))
        .expect("a session");
    (
        opened["sessionId"].as_str().expect("an id").to_owned(),
        opened,
    )
}

fn paired(config: &str) -> (Host, Identity) {
    let host = Host::start(config, 1);
    let client = support::key();
    host.pair(&client);
    (host, client)
}

#[test]
fn pis_entry_serves_a_turn_says_what_runs_unasked_and_keeps_the_conversation() {
    let (host, client) = paired(&pi_config(&["--mode", "rpc"], ""));
    let dir = cwd(&host.scratch);
    with_link(host.url("pi"), &client, host.peer, |rpc| {
        let (session, opened) = open(rpc, &dir);
        let notices: Vec<&str> = opened["_meta"]["dev.boriken.canoa/notices"]
            .as_array()
            .expect("notices")
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(notices.len(), 1, "{notices:?}");
        assert!(notices[0].contains("not an ACP agent"), "{notices:?}");
        assert!(notices[0].contains("rm -rf, sudo"), "{notices:?}");
        let hello = prompt(rpc, &session, "hello", "allow");
        assert_eq!(hello.stop(), "end_turn");
        assert_eq!(hello.text("agent_message_chunk"), "echo: hello");
        assert_eq!(hello.text("agent_thought_chunk"), "thinking about hello");
        let again = prompt(rpc, &session, "remember", "allow");
        assert_eq!(again.text("agent_message_chunk"), "you said before: hello");
    })
    .expect("linked");
}

#[test]
fn a_question_pi_asks_reaches_the_person_and_their_answer_reaches_pi() {
    let (host, client) = paired(&pi_config(&["--mode", "rpc"], ""));
    let dir = cwd(&host.scratch);
    with_link(host.url("pi"), &client, host.peer, |rpc| {
        let (session, _) = open(rpc, &dir);
        let allowed = prompt(rpc, &session, "run rm -rf build", "allow");
        assert_eq!(allowed.asked.len(), 1);
        let asked = &allowed.asked[0];
        assert_eq!(asked["sessionId"], session.as_str());
        assert!(
            asked["toolCall"]["title"]
                .as_str()
                .is_some_and(|title| title.contains("rm -rf build") && !title.contains('\n')),
            "{asked}"
        );
        // The whole question, which a title shown cut short could hide the end of.
        assert_eq!(
            asked["toolCall"]["rawInput"]["text"],
            "\u{26a0}\u{fe0f} Dangerous command:\n\n  rm -rf build\n\nAllow?"
        );
        assert!(
            asked["toolCall"]["toolCallId"]
                .as_str()
                .is_some_and(|id| id.starts_with("canoa-host-ask-")),
            "{asked}"
        );
        let kinds: Vec<&Value> = asked["options"]
            .as_array()
            .expect("options")
            .iter()
            .map(|o| &o["kind"])
            .collect();
        assert_eq!(kinds, [&json!("allow_once"), &json!("reject_once")]);
        let call = allowed.of("tool_call");
        assert_eq!(call.len(), 1);
        assert_eq!(call[0]["kind"], "execute");
        assert_eq!(call[0]["title"], "bash rm -rf build");
        assert_eq!(call[0]["rawInput"], json!({"command": "rm -rf build"}));
        let done = allowed.of("tool_call_update");
        assert_eq!(done[0]["status"], "completed");
        assert_eq!(done[0]["content"][0]["content"]["text"], "ran rm -rf build");
        assert_eq!(allowed.text("agent_message_chunk"), "allowed");

        let refused = prompt(rpc, &session, "run sudo reboot", "reject");
        assert_eq!(refused.asked.len(), 1);
        assert_eq!(refused.of("tool_call_update")[0]["status"], "failed");
        assert_eq!(refused.text("agent_message_chunk"), "refused");
        assert_eq!(refused.stop(), "end_turn");
    })
    .expect("linked");
}

#[test]
fn an_answer_that_is_not_allow_refuses() {
    let (host, client) = paired(&pi_config(&["--mode", "rpc"], ""));
    let dir = cwd(&host.scratch);
    with_link(host.url("pi"), &client, host.peer, |rpc| {
        let (session, _) = open(rpc, &dir);
        rpc.send(&json!({
            "jsonrpc": "2.0", "id": "p", "method": "session/prompt",
            "params": {"sessionId": session, "prompt": [{"type": "text", "text": "run rm -rf /"}]},
        }))
        .expect("sent");
        let asked = loop {
            let message = rpc.next_message(WAIT).expect("a request");
            if message["method"] == "session/request_permission" {
                break message;
            }
        };
        // An error, as a client that cannot ask sends.
        rpc.send(&json!({"jsonrpc": "2.0", "id": asked["id"], "error": {"code": -32603, "message": "no"}}))
            .expect("answered");
        let turn = read_turn(rpc, "allow");
        assert_eq!(turn.text("agent_message_chunk"), "refused");
    })
    .expect("linked");
}

#[test]
fn cancelling_asks_pi_to_abort_and_the_session_goes_on() {
    let (host, client) = paired(&pi_config(&["--mode", "rpc"], ""));
    let dir = cwd(&host.scratch);
    with_link(host.url("pi"), &client, host.peer, |rpc| {
        let (session, _) = open(rpc, &dir);
        rpc.send(&json!({
            "jsonrpc": "2.0", "id": "p", "method": "session/prompt",
            "params": {"sessionId": session, "prompt": [{"type": "text", "text": "wait"}]},
        }))
        .expect("sent");
        std::thread::sleep(Duration::from_millis(300));
        rpc.send(&json!({"jsonrpc": "2.0", "method": "session/cancel", "params": {"sessionId": session}}))
            .expect("cancel sent");
        let cancelled = read_turn(rpc, "allow");
        assert_eq!(cancelled.stop(), "cancelled");
        let after = prompt(rpc, &session, "hello", "allow");
        assert_eq!(after.text("agent_message_chunk"), "echo: hello");
    })
    .expect("linked");
}

#[test]
fn cancelling_refuses_a_question_still_waiting_and_a_late_allow_reaches_nothing() {
    let (host, client) = paired(&pi_config(&["--mode", "rpc"], ""));
    let dir = cwd(&host.scratch);
    with_link(host.url("pi"), &client, host.peer, |rpc| {
        let (session, _) = open(rpc, &dir);
        rpc.send(&json!({
            "jsonrpc": "2.0", "id": "p", "method": "session/prompt",
            "params": {"sessionId": session, "prompt": [{"type": "text", "text": "run rm -rf x"}]},
        }))
        .expect("sent");
        let asked = loop {
            let message = rpc.next_message(WAIT).expect("a request");
            if message["method"] == "session/request_permission" {
                break message;
            }
        };
        rpc.send(&json!({"jsonrpc": "2.0", "method": "session/cancel", "params": {"sessionId": session}}))
            .expect("cancel sent");
        let started = Instant::now();
        let cancelled = read_turn(rpc, "allow");
        assert_eq!(cancelled.stop(), "cancelled");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the tool was told no at once"
        );
        assert_eq!(cancelled.of("tool_call_update")[0]["status"], "failed");
        // The person's answer, after the turn it belonged to.
        rpc.send(&json!({
            "jsonrpc": "2.0", "id": asked["id"],
            "result": {"outcome": {"outcome": "selected", "optionId": "allow"}},
        }))
        .expect("answered late");
        let after = prompt(rpc, &session, "remember", "allow");
        assert_eq!(after.text("agent_message_chunk"), "you said before: run rm -rf x");
    })
    .expect("linked");
}

#[test]
fn a_tool_that_does_not_stop_when_asked_is_ended_after_the_grace() {
    let (host, client) = paired(&pi_config(&["--mode", "rpc"], ""));
    let dir = cwd(&host.scratch);
    with_link(host.url("pi"), &client, host.peer, |rpc| {
        let (session, _) = open(rpc, &dir);
        rpc.send(&json!({
            "jsonrpc": "2.0", "id": "p", "method": "session/prompt",
            "params": {"sessionId": session, "prompt": [{"type": "text", "text": "stubborn"}]},
        }))
        .expect("sent");
        std::thread::sleep(Duration::from_millis(300));
        for _ in 0..3 {
            rpc.send(&json!({"jsonrpc": "2.0", "method": "session/cancel", "params": {"sessionId": session}}))
                .expect("cancel sent");
        }
        let cancelled = read_turn(rpc, "allow");
        assert_eq!(cancelled.stop(), "cancelled");
        let after = prompt(rpc, &session, "hello", "allow");
        let error = after.answer.expect_err("the session ended with its process");
        assert!(
            error["message"]
                .as_str()
                .is_some_and(|text| text.contains("did not stop")),
            "{error}"
        );
    })
    .expect("linked");
}

#[cfg(windows)]
#[test]
fn a_network_path_is_never_a_sessions_directory() {
    let (host, client) = paired(&pi_config(&["--mode", "rpc"], ""));
    with_link(host.url("pi"), &client, host.peer, |rpc| {
        rpc.initialize().expect("initialized");
        for cwd in [
            r"\\127.0.0.1\c$\Windows",
            r"\\?\UNC\127.0.0.1\c$",
            r"\\.\pipe\x",
        ] {
            let refused = rpc
                .request("session/new", json!({"cwd": cwd, "mcpServers": []}))
                .expect_err("refused");
            assert!(
                refused["message"]
                    .as_str()
                    .is_some_and(|text| text.contains("network")),
                "{cwd}: {refused}"
            );
        }
    })
    .expect("linked");
}

/// A batch file that names a program bare is not given one from the session's directory, where a client chose it.
#[cfg(windows)]
#[test]
fn a_batch_tool_finds_no_program_in_the_sessions_directory() {
    let scratch = Scratch::new();
    let tool_dir = scratch.0.join("tool");
    let session_dir = scratch.0.join("session");
    std::fs::create_dir_all(&tool_dir).expect("a directory");
    std::fs::create_dir_all(&session_dir).expect("a directory");
    let wrapper = tool_dir.join("wrapper.cmd");
    std::fs::write(&wrapper, "@helper\r\n").expect("written");
    std::fs::write(
        session_dir.join("helper.cmd"),
        "@echo planted> \"%~dp0planted.txt\"\r\n",
    )
    .expect("written");
    let spec = json!({
        "program": wrapper.to_string_lossy(),
        "runs": "session",
        "unasked": "x",
        "prompt": {"type": "prompt", "message": "{prompt}"},
    });
    let mut child = std::process::Command::new(support::host_binary())
        .args(["tool", "--name", "wrapped", "--spec", &spec.to_string()])
        .env_remove("NoDefaultCurrentDirectoryInExePath")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("the tool command starts");
    let mut stdin = child.stdin.take().expect("its input");
    let mut stdout = std::io::BufReader::new(child.stdout.take().expect("its output"));
    for message in [
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": 1}}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "session/new", "params": {"cwd": session_dir.to_string_lossy(), "mcpServers": []}}),
    ] {
        use std::io::{BufRead as _, Write as _};
        writeln!(stdin, "{message}").expect("written");
        let mut answer = String::new();
        stdout.read_line(&mut answer).expect("an answer");
    }
    std::thread::sleep(Duration::from_secs(2));
    drop(stdin);
    let _ = child.wait();
    assert!(
        !session_dir.join("planted.txt").exists(),
        "a program in the session's directory ran"
    );
}

#[test]
fn a_model_failure_fails_the_turn_in_its_words() {
    let (host, client) = paired(&pi_config(&["--mode", "rpc"], ""));
    let dir = cwd(&host.scratch);
    with_link(host.url("pi"), &client, host.peer, |rpc| {
        let (session, _) = open(rpc, &dir);
        let failed = prompt(rpc, &session, "fail", "allow");
        let error = failed.answer.expect_err("a failure");
        assert_eq!(error["message"], "the model is unavailable", "{error}");
        let after = prompt(rpc, &session, "hello", "allow");
        assert_eq!(after.stop(), "end_turn");
    })
    .expect("linked");
}

#[test]
fn a_tool_that_ends_mid_turn_fails_it_and_ends_the_session() {
    let (host, client) = paired(&pi_config(&["--mode", "rpc"], ""));
    let dir = cwd(&host.scratch);
    with_link(host.url("pi"), &client, host.peer, |rpc| {
        let (session, _) = open(rpc, &dir);
        let ended = prompt(rpc, &session, "exit", "allow");
        let error = ended.answer.expect_err("a failure");
        assert!(
            error["message"]
                .as_str()
                .is_some_and(|text| text.contains("pi ended")),
            "{error}"
        );
        let after = prompt(rpc, &session, "hello", "allow");
        let error = after.answer.expect_err("no more turns");
        assert!(
            error["message"]
                .as_str()
                .is_some_and(|text| text.contains("open a new session")),
            "{error}"
        );
    })
    .expect("linked");
}

#[test]
fn a_tool_run_per_message_says_so_and_resumes_the_session_it_reported() {
    let config = message_entry(
        "once",
        &["--mode", "json", "--", "{prompt}"],
        "resume_args = [\"--session-id\", \"{session}\"]",
    ) + &message_entry("forgetful", &["--mode", "json"], "");
    let host = Host::start(&config, 2);
    let client = support::key();
    host.pair(&client);
    let dir = cwd(&host.scratch);
    with_link(host.url("once"), &client, host.peer, |rpc| {
        let (session, opened) = open(rpc, &dir);
        let notices = &opened["_meta"]["dev.boriken.canoa/notices"];
        assert!(
            notices[1]
                .as_str()
                .is_some_and(|text| text.contains("anew")),
            "{notices}"
        );
        assert_eq!(
            prompt(rpc, &session, "one", "allow").text("agent_message_chunk"),
            "echo: one"
        );
        assert_eq!(
            prompt(rpc, &session, "two", "allow").text("agent_message_chunk"),
            "echo: two (in sess-1)"
        );
        let failed = prompt(rpc, &session, "exit", "allow");
        assert!(failed.answer.is_err(), "a run that exits 2 fails its turn");
    })
    .expect("linked");
    with_link(host.url("forgetful"), &client, host.peer, |rpc| {
        let (session, opened) = open(rpc, &dir);
        let notices = &opened["_meta"]["dev.boriken.canoa/notices"];
        assert!(
            notices[1]
                .as_str()
                .is_some_and(|text| text.contains("does not remember")),
            "{notices}"
        );
        // No {prompt} in the arguments: the prompt goes on standard input.
        let turn = prompt(rpc, &session, "-rf --from stdin", "allow");
        assert_eq!(turn.text("agent_message_chunk"), "echo: -rf --from stdin");
    })
    .expect("linked");
}

#[test]
fn the_tools_processes_end_with_the_link() {
    let (host, client) = paired(&pi_config(&["--mode", "rpc"], ""));
    let dir = cwd(&host.scratch);
    with_link(host.url("pi"), &client, host.peer, |rpc| {
        let (first, _) = open(rpc, &dir);
        let (second, _) = open(rpc, &dir);
        // Each has answered once, so each has written its id.
        assert_eq!(prompt(rpc, &first, "hello", "allow").stop(), "end_turn");
        assert_eq!(prompt(rpc, &second, "hello", "allow").stop(), "end_turn");
        rpc.send(&json!({
            "jsonrpc": "2.0", "id": "p", "method": "session/prompt",
            "params": {"sessionId": second, "prompt": [{"type": "text", "text": "wait"}]},
        }))
        .expect("a turn left running");
    })
    .expect("linked");
    let pids: Vec<u32> = std::fs::read_to_string(Path::new(&dir).join("tool.pid"))
        .expect("the fixture wrote its id")
        .lines()
        .map(|line| line.parse().expect("a process id"))
        .collect();
    assert_eq!(pids.len(), 2);
    let until = Instant::now() + Duration::from_secs(10);
    while pids.iter().any(|pid| running(*pid)) {
        assert!(
            Instant::now() < until,
            "a tool process outlived its link: {pids:?}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn a_session_takes_no_mcp_server_and_a_tool_that_cannot_start_says_so() {
    let config = pi_config(&["--mode", "rpc"], "")
        + "\n[host.tools.missing]\nprogram = \"canoa-host-test-no-such-program\"\nruns = \"session\"\nunasked = \"x\"\nprompt = { type = \"prompt\", message = \"{prompt}\" }\n";
    let host = Host::start(&config, 2);
    let client = support::key();
    host.pair(&client);
    let dir = cwd(&host.scratch);
    with_link(host.url("pi"), &client, host.peer, |rpc| {
        rpc.initialize().expect("initialized");
        let refused = rpc
            .request(
                "session/new",
                json!({"cwd": dir, "mcpServers": [{"type": "http", "name": "x", "url": "https://example.com/mcp", "headers": []}]}),
            )
            .expect_err("refused");
        assert_eq!(refused["code"], -32602, "{refused}");
        let relative = rpc
            .request("session/new", json!({"cwd": "work", "mcpServers": []}))
            .expect_err("refused");
        assert_eq!(relative["code"], -32602, "{relative}");
    })
    .expect("linked");
    with_link(host.url("missing"), &client, host.peer, |rpc| {
        rpc.initialize().expect("initialized");
        let failed = rpc
            .request("session/new", json!({"cwd": dir, "mcpServers": []}))
            .expect_err("it cannot start");
        assert!(
            failed["message"]
                .as_str()
                .is_some_and(|text| text.contains("missing did not start")),
            "{failed}"
        );
    })
    .expect("linked");
}

#[test]
fn check_lists_a_tool_entry() {
    let scratch = Scratch::new();
    let path = scratch.0.join("Canoa.cfg");
    std::fs::write(&path, pi_config(&["--mode", "rpc"], "")).expect("written");
    let output = std::process::Command::new(support::host_binary())
        .args(["check", "--format", "json", "--config"])
        .arg(&path)
        .output()
        .expect("check runs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let line: Value = serde_json::from_slice(&output.stdout).expect("one JSON line");
    assert_eq!(line["name"], "pi");
    assert_eq!(line["kind"], "tool");
}
