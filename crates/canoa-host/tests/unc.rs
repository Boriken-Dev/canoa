//! A session whose folder is a Windows share (`\\server\share\...`) starts like any other, and the agent works in that
//! folder. The share is this machine's own administrative share, so nothing leaves it.
#![cfg(windows)]
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod support;

use std::path::Path;

use serde_json::{Value, json};
use support::{Host, Scratch, example, on_share, quoted, shared_cwd, with_link};

fn new_session(
    rpc: &support::Rpc<'_, coqui_transport_web::websocket::Channel>,
    cwd: &str,
) -> Result<Value, Value> {
    rpc.request("session/new", json!({"cwd": cwd, "mcpServers": []}))
}

#[test]
fn a_scripted_agent_starts_a_session_in_a_share_folder() {
    let scratch = Scratch::new();
    let work = shared_cwd(&scratch);
    let config = format!(
        "[host.agents.Agent]\nprogram = {}\nargs = [\"--scripted\"]\nworking_roots = [{}]\n",
        quoted(&example("stdio_agent", false).to_string_lossy()),
        quoted(&work)
    );
    let host = Host::start_in(scratch, &config, 1, &[]);
    let client = support::key();
    host.pair(&client);
    with_link(host.url("Agent"), &client, host.peer, |rpc| {
        rpc.initialize().expect("initialize is answered");
        for written in [work.clone(), work.replacen(r"\\", r"\\?\UNC\", 1)] {
            let opened = new_session(rpc, &written).expect("a session opens in the share folder");
            let id = opened["sessionId"].as_str().expect("an id");
            // The agent is told the folder in its plain form, whichever form the client wrote.
            let said = rpc.prompt(id, "Cwd").expect("the agent answers");
            assert_eq!(said, format!("cwd: {work}"), "{written}");
        }
        new_session(rpc, &format!(r"{work}\sub")).expect("and in a folder under it");
        new_session(rpc, &format!(r"{work}\..")).expect_err("a .. segment is refused");
        new_session(rpc, &on_share(&host.scratch.0).to_string_lossy())
            .expect_err("outside the roots");
    })
    .expect("a paired key is served");
}

/// Claude Code and Codex are run by coqui-acp, which refuses any path written `\\host\share` before this host's own
/// checks are reached: "a network or device path is not taken".
#[test]
#[ignore = "coqui-acp's harnesses take no network path yet"]
fn a_command_file_harness_works_in_a_share_folder() {
    let scratch = Scratch::new();
    let work = shared_cwd(&scratch);
    let exe = scratch.0.join("codex.exe");
    std::fs::copy(example("harness_standin", false), &exe).expect("copied");
    let shim = scratch.0.join("codex.cmd");
    std::fs::write(&shim, "@\"%~dp0codex.exe\" %*\r\n").expect("written");
    let config = format!(
        "[host.agents.Agent]\nharness = \"codex\"\nexecutable = {}\nworking_roots = [{}]\n",
        quoted(&shim.to_string_lossy()),
        quoted(&work)
    );
    let host = Host::start_in(scratch, &config, 1, &[]);
    let client = support::key();
    host.pair(&client);
    with_link(host.url("Agent"), &client, host.peer, |rpc| {
        rpc.initialize().expect("initialize is answered");
        let opened = new_session(rpc, &work).expect("a session opens in the share folder");
        let session = opened["sessionId"].as_str().expect("an id").to_owned();
        rpc.send(
            &json!({"jsonrpc": "2.0", "id": 900, "method": "session/prompt",
            "params": {"sessionId": session, "prompt": [{"type": "text", "text": "write it"}]}}),
        )
        .expect("sent");
        loop {
            let message = rpc.next_message(support::WAIT).expect("the turn goes on");
            if message["method"] == "session/request_permission" {
                let allow = message["params"]["options"]
                    .as_array()
                    .and_then(|options| {
                        options.iter().find(|option| option["kind"] == "allow_once")
                    })
                    .expect("an allow option");
                rpc.send(
                    &json!({"jsonrpc": "2.0", "id": message["id"], "result": {"outcome": {
                    "outcome": "selected", "optionId": allow["optionId"]}}}),
                )
                .expect("answered");
            } else if message["id"] == 900 {
                assert!(message.get("error").is_none(), "{message}");
                break;
            }
        }
    })
    .expect("a paired key is served");
    assert!(
        Path::new(&work).join("standin.txt").is_file(),
        "the agent did not work in the share folder"
    );
}
