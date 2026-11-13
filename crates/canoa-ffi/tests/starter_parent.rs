//! Whose child each process the host starts is. Every one has to be a child of the host's own process starter and never of the
//! application: an application that collects the children of its own process would take its status. The agent below is
//! started by the host through a link, as a session's is; the other is started as every other process the host starts is.
#![cfg(unix)]
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use canoa_ffi::exports::{
    STATUS_OK, canoa_ffi_job_read, canoa_ffi_job_start, canoa_ffi_job_stop, canoa_ffi_link_close,
    canoa_ffi_link_open, canoa_ffi_link_read, canoa_ffi_link_send, canoa_ffi_text_free,
};
use serde_json::{Value, json};

const WAIT: Duration = Duration::from_secs(30);

fn example(name: &str) -> PathBuf {
    let tests = std::env::current_exe().expect("the test knows where it is");
    let profile = tests
        .parent()
        .and_then(Path::parent)
        .expect("a test binary is in target/<profile>/deps");
    profile.join("examples").join(name)
}

fn text_of(call: impl FnOnce(&mut *mut u8, &mut usize) -> i32) -> Value {
    let (mut text, mut len) = (core::ptr::null_mut(), 0_usize);
    assert_eq!(call(&mut text, &mut len), STATUS_OK);
    // SAFETY: the library just gave this text of this length.
    let value = serde_json::from_slice(unsafe { core::slice::from_raw_parts(text, len) })
        .expect("the text is JSON");
    // SAFETY: as it was given, once.
    unsafe { canoa_ffi_text_free(text, len) };
    value
}

fn start(request: &Value) -> u64 {
    let text = request.to_string();
    let mut job = 0_u64;
    // SAFETY: both point at what lives across the call.
    let status = unsafe { canoa_ffi_job_start(text.as_ptr(), text.len(), &raw mut job) };
    assert_eq!(status, STATUS_OK, "{request}");
    job
}

fn send_on(link: u64, message: &Value) {
    let text = message.to_string();
    // SAFETY: the text lives across the call.
    assert_eq!(
        unsafe { canoa_ffi_link_send(link, text.as_ptr(), text.len()) },
        STATUS_OK
    );
}

fn answer_on(link: u64, id: u64) -> Value {
    let until = Instant::now() + WAIT;
    loop {
        // SAFETY: both point at locals that live across the call.
        let read = text_of(|text, len| unsafe { canoa_ffi_link_read(link, text, len) });
        for message in read["messages"].as_array().expect("a list") {
            let message: Value =
                serde_json::from_str(message.as_str().expect("text")).expect("ACP");
            if message["id"] == id && message.get("method").is_none() {
                return message;
            }
        }
        assert!(Instant::now() < until, "no answer to {id}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// `(parent id, its command line)` of process `pid`, as the system's own `ps` says.
fn parent_of(pid: u32) -> (u32, String) {
    let listed = Command::new("ps")
        .args(["-o", "ppid=", "-p", &pid.to_string()])
        .output()
        .expect("ps runs");
    let parent: u32 = String::from_utf8_lossy(&listed.stdout)
        .trim()
        .parse()
        .expect("a parent id");
    let named = Command::new("ps")
        .args(["-o", "args=", "-p", &parent.to_string()])
        .output()
        .expect("ps runs");
    (
        parent,
        String::from_utf8_lossy(&named.stdout).trim().to_owned(),
    )
}

#[test]
fn every_process_the_host_starts_is_a_child_of_its_process_starter_and_never_of_the_application() {
    let me = std::process::id();
    let scratch = std::env::temp_dir().join(format!("canoa-parent-{me}"));
    std::fs::create_dir_all(&scratch).expect("a directory");
    let agent = example("stdio_agent");
    assert!(
        agent.exists(),
        "build the examples first: cargo build -p canoa-ffi --examples"
    );
    let config = scratch.join("canoa.cfg");
    std::fs::write(
        &config,
        format!(
            "[host.agents.fixture]\nprogram = {}\nlisten = \"127.0.0.1:0\"\n",
            serde_json::to_string(&agent.to_string_lossy()).expect("quoted")
        ),
    )
    .expect("written");
    let entry = example("child_entry");
    let job = start(&json!({
        "args": ["serve", "--allow-empty", "--until-stdin-closes", "--config", config],
        "home": scratch.join("home"),
        "holder": "a test",
        "program": entry,
        "program_args": [canoa_ffi::jobs::CHILD_FLAG],
    }));
    let until = Instant::now() + WAIT;
    loop {
        // SAFETY: both point at locals that live across the call.
        let said = text_of(|text, len| unsafe { canoa_ffi_job_read(job, text, len) });
        if said["out"].to_string().contains("ready") {
            break;
        }
        assert!(Instant::now() < until, "the host never was ready: {said}");
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut link = 0_u64;
    let name = "fixture";
    // SAFETY: both point at what lives across the call.
    assert_eq!(
        unsafe { canoa_ffi_link_open(job, name.as_ptr(), name.len(), &raw mut link) },
        STATUS_OK
    );
    send_on(
        link,
        &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": 1, "clientCapabilities": {}}}),
    );
    answer_on(link, 1);
    send_on(
        link,
        &json!({"jsonrpc": "2.0", "id": 2, "method": "session/new",
        "params": {"cwd": scratch.to_string_lossy(), "mcpServers": []}}),
    );
    answer_on(link, 2);
    send_on(
        link,
        &json!({"jsonrpc": "2.0", "id": 3, "method": "_fixture/pid", "params": {}}),
    );
    let agent_pid =
        u32::try_from(answer_on(link, 3)["result"]["pid"].as_u64().expect("a pid")).expect("an id");

    // The agent the host started for a session.
    let (starter, command) = parent_of(agent_pid);
    assert_ne!(starter, me, "the agent is a child of the application");
    assert!(
        command.contains("spawner"),
        "its parent is not the starter: {command}"
    );
    assert_eq!(
        parent_of(starter).0,
        me,
        "the starter is the application's own child"
    );

    // Whatever else the host starts, by the one way it starts them.
    let mut other = canoa_host::spawn::Command::new("sleep")
        .arg("300")
        .spawn()
        .expect("it starts");
    let (its_parent, command) = parent_of(other.id());
    assert_eq!(
        its_parent, starter,
        "a process of the host's is a child of the application: {command}"
    );
    other.kill().expect("ended");
    other.wait().expect("collected");

    assert_eq!(canoa_ffi_link_close(link), STATUS_OK);
    assert_eq!(canoa_ffi_job_stop(job), STATUS_OK);
    let _ = std::fs::remove_dir_all(&scratch);
}
