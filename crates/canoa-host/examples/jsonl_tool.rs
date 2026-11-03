//! A test fixture shaped like Pi's command line: `--mode rpc` reads JSON commands on standard input and writes JSON events,
//! as Pi's RPC mode does; `--mode json` answers one prompt, from after `--` or from standard input, and exits. The prompt
//! picks what it does: `run COMMAND` asks leave as Pi's permission-gate extension does, `wait` waits for `abort`,
//! `stubborn` takes no notice of it, `remember` repeats the prompt before, `fail` reports a model failure, `exit` ends the
//! process mid-turn.
//!
//! It appends its process id to `tool.pid` in its working directory, so a test can see whether it still runs.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    missing_docs,
    reason = "a fixture"
)]

use std::io::{BufRead as _, Read as _, Write as _};

use serde_json::{Value, json};

fn emit(event: &Value) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{event}");
    let _ = out.flush();
}

fn text(delta: &str) {
    emit(
        &json!({"type": "message_update", "assistantMessageEvent": {"type": "text_delta", "contentIndex": 0, "delta": delta}}),
    );
}

fn settled(aborted: bool) {
    emit(&json!({"type": "agent_end", "messages": [], "willRetry": false}));
    emit(&json!({"type": "agent_settled", "aborted": aborted}));
}

/// The next command on standard input.
fn next(lines: &mut impl Iterator<Item = std::io::Result<String>>) -> Option<Value> {
    loop {
        let line = lines.next()?.ok()?;
        if let Ok(value) = serde_json::from_str::<Value>(&line) {
            return Some(value);
        }
    }
}

fn rpc() {
    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();
    let mut before = String::new();
    while let Some(command) = next(&mut lines) {
        if command["type"] != "prompt" {
            emit(&json!({"type": "response", "command": command["type"], "success": true}));
            continue;
        }
        let message = command["message"].as_str().unwrap_or_default().to_owned();
        emit(
            &json!({"id": command["id"], "type": "response", "command": "prompt", "success": true, "data": {"disposition": "started"}}),
        );
        emit(&json!({"type": "agent_start"}));
        if let Some(command_text) = message.strip_prefix("run ") {
            emit(
                &json!({"type": "tool_execution_start", "toolCallId": "call_1", "toolName": "bash", "args": {"command": command_text}}),
            );
            emit(&json!({
                "type": "extension_ui_request", "id": "u1", "method": "select",
                "title": format!("\u{26a0}\u{fe0f} Dangerous command:\n\n  {command_text}\n\nAllow?"),
                "options": ["Yes", "No"],
            }));
            let answer = loop {
                match next(&mut lines) {
                    Some(reply)
                        if reply["type"] == "extension_ui_response" && reply["id"] == "u1" =>
                    {
                        break reply;
                    }
                    Some(_) => {}
                    None => return,
                }
            };
            if answer["value"] == "Yes" {
                emit(
                    &json!({"type": "tool_execution_end", "toolCallId": "call_1", "toolName": "bash", "result": {"content": [{"type": "text", "text": format!("ran {command_text}")}]}, "isError": false}),
                );
                text("allowed");
            } else {
                emit(
                    &json!({"type": "tool_execution_end", "toolCallId": "call_1", "toolName": "bash", "result": {"content": [{"type": "text", "text": "Blocked by user"}]}, "isError": true}),
                );
                text("refused");
            }
            settled(false);
        } else if message == "wait" {
            loop {
                match next(&mut lines) {
                    Some(abort) if abort["type"] == "abort" => {
                        emit(&json!({"type": "response", "command": "abort", "success": true}));
                        settled(true);
                        break;
                    }
                    Some(_) => {}
                    // Busy, as a tool running a long command is: its input closing does not end it.
                    None => {
                        std::thread::sleep(std::time::Duration::from_secs(60));
                        return;
                    }
                }
            }
        } else if message == "stubborn" {
            // Takes no notice of abort, as a tool stuck in a command would.
            while next(&mut lines).is_some() {}
            std::thread::sleep(std::time::Duration::from_secs(60));
            return;
        } else if message == "remember" {
            text(&format!("you said before: {before}"));
            settled(false);
        } else if message == "fail" {
            emit(
                &json!({"type": "message_end", "message": {"role": "assistant", "stopReason": "error", "errorMessage": "the model is unavailable"}}),
            );
            settled(false);
        } else if message == "exit" {
            std::process::exit(3);
        } else {
            println!("this line is not JSON");
            emit(
                &json!({"type": "message_update", "assistantMessageEvent": {"type": "thinking_delta", "contentIndex": 0, "delta": format!("thinking about {message}")}}),
            );
            text("echo: ");
            text(&message);
            settled(false);
        }
        before = message;
    }
}

fn once(args: &[String]) {
    let session = args
        .iter()
        .position(|arg| arg == "--session-id")
        .and_then(|at| args.get(at + 1))
        .cloned();
    let message = match args.iter().position(|arg| arg == "--") {
        Some(at) => args[at + 1..].join(" "),
        None => {
            let mut input = String::new();
            let _ = std::io::stdin().read_to_string(&mut input);
            input
        }
    };
    if message == "exit" {
        std::process::exit(2);
    }
    emit(
        &json!({"type": "session", "version": 3, "id": session.clone().unwrap_or_else(|| "sess-1".to_owned())}),
    );
    emit(&json!({"type": "agent_start"}));
    match session {
        Some(session) => text(&format!("echo: {message} (in {session})")),
        None => text(&format!("echo: {message}")),
    }
    settled(false);
}

fn main() {
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("tool.pid")
    {
        let _ = writeln!(file, "{}", std::process::id());
    }
    eprintln!("jsonl_tool: started");
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args
        .windows(2)
        .find(|pair| pair[0] == "--mode")
        .map(|pair| pair[1].as_str())
    {
        Some("rpc") => rpc(),
        Some("json") => once(&args),
        _ => {
            eprintln!("jsonl_tool: --mode rpc or --mode json");
            std::process::exit(2);
        }
    }
}
