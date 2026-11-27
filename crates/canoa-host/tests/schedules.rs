//! Scheduled tasks (`[schedules.ID]`): the host runs a task's prompt as a session of an entry, with no application connected, records
//! it, says what became of it, and leaves what needs the person unanswered. Each test starts the real `serve` on a home of its own,
//! with the fixture agent, which echoes a prompt and asks permission when the prompt starts `ask`.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod linked;
mod support;

use std::time::{Duration, Instant};

use canoa_host::Link;
use linked::{Hosted, ask, entry};
use serde_json::{Value, json};
use support::{Scratch, cwd, quoted};

/// The configuration: the fixture entry (`extra` lines in it) and a task `id` that runs `prompt` on it every day at 03:00.
fn host(extra: &str, tasks: &[(&str, &str)]) -> Hosted {
    let scratch = Scratch::new();
    let folder = quoted(&cwd(&scratch));
    Hosted::start_in_setup(
        scratch,
        |_| {
            let mut text = entry("fixture", extra);
            for (id, prompt) in tasks {
                text.push_str(&format!(
                    "[schedules.{id}]\nentry = \"fixture\"\nfolder = {folder}\nprompt = {}\nat = \"03:00\"\n",
                    quoted(prompt)
                ));
            }
            text
        },
        &[],
    )
}

fn call(link: &Link, id: u64, method: &str, params: Value) -> Result<Value, (i64, String)> {
    match ask(link, id, method, params) {
        Ok((result, _)) => Ok(result),
        Err(text) => {
            // A refusal arrives as the error object, which `ask` reports in its text.
            let code = text
                .split("\"code\":")
                .nth(1)
                .and_then(|rest| rest.split([',', '}']).next())
                .and_then(|code| code.trim().parse().ok())
                .unwrap_or(0);
            Err((code, text))
        }
    }
}

/// The task `id` once its first run has been noted.
fn finished(link: &Link, id: &str) -> Value {
    let until = Instant::now() + Duration::from_secs(60);
    let mut call_id = 100;
    loop {
        call_id += 1;
        let listed = call(link, call_id, "schedules/list", json!({})).expect("listed");
        let task = listed["tasks"]
            .as_array()
            .and_then(|tasks| tasks.iter().find(|task| task["id"] == id))
            .expect("the task is listed")
            .clone();
        if !task["last"].is_null() {
            return task;
        }
        assert!(Instant::now() < until, "the run did not finish: {task}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn a_task_runs_with_no_application_and_says_what_became_of_it() {
    let host = host("", &[("nightly", "hello")]);
    let link = host.local.open("@control").expect("the control channel");

    let listed = call(&link, 1, "schedules/list", json!({})).expect("listed");
    let task = &listed["tasks"][0];
    assert_eq!(task["id"], "nightly");
    assert_eq!(task["repeat"], "daily");
    assert_eq!(task["at"], "03:00");
    assert!(
        task["next"]
            .as_u64()
            .is_some_and(|next| next > listed["now"].as_u64().unwrap_or(0)),
        "{task}"
    );
    assert!(task["last"].is_null());
    assert!(
        listed["entries"]
            .as_array()
            .is_some_and(|entries| entries.contains(&json!("fixture"))),
        "{listed}"
    );

    assert_eq!(
        call(&link, 2, "schedules/run", json!({"id": "nightly"})).expect("started")["started"],
        true
    );
    let done = finished(&link, "nightly");
    assert_eq!(done["last"]["result"], "completed", "{done}");
    assert_eq!(done["last"]["note"], "echo: hello");
    assert!(
        done["last"]["session"]
            .as_str()
            .is_some_and(|session| !session.is_empty()),
        "{done}"
    );

    // What became of it is kept in the host's home, so a restart still says it.
    let kept =
        std::fs::read_to_string(host.scratch.0.join("home").join("schedules.json")).expect("kept");
    assert!(
        kept.contains("\"nightly\"") && kept.contains("completed"),
        "{kept}"
    );
    host.stop();
}

#[test]
fn a_request_for_leave_is_left_unanswered_and_waits_in_a_shared_entry() {
    let host = host("shared = true", &[("careful", "ask may I")]);
    let link = host.local.open("@control").expect("the control channel");
    call(&link, 1, "schedules/run", json!({"id": "careful"})).expect("started");
    let done = finished(&link, "careful");
    assert_eq!(done["last"]["result"], "needs_you", "{done}");
    assert!(
        done["last"]["note"]
            .as_str()
            .is_some_and(|note| note.contains("waits in the session")),
        "{done}"
    );

    // The session is still open, with the question in it, for whoever attaches next.
    let running = call(&link, 2, "sessions/running", json!({})).expect("listed");
    let row = &running["sessions"][0];
    assert_eq!(row["entry"], "fixture", "{running}");
    assert_eq!(row["session"], done["last"]["session"], "{running}");
    assert_eq!(row["permissionsWaiting"], 1, "{running}");
    host.stop();
}

#[test]
fn on_an_entry_that_is_not_shared_the_question_is_said_to_be_only_in_the_record() {
    let host = host("", &[("careful", "ask may I")]);
    let link = host.local.open("@control").expect("the control channel");
    call(&link, 1, "schedules/run", json!({"id": "careful"})).expect("started");
    let done = finished(&link, "careful");
    assert_eq!(done["last"]["result"], "needs_you", "{done}");
    assert!(
        done["last"]["note"]
            .as_str()
            .is_some_and(|note| note.contains("not shared")),
        "{done}"
    );
    host.stop();
}

#[test]
fn tasks_are_set_and_removed_in_canoa_cfg_and_a_task_the_reader_would_refuse_is_not_written() {
    let host = host("", &[]);
    let link = host.local.open("@control").expect("the control channel");
    let task = json!({"entry": "fixture", "folder": "/work", "prompt": "check", "repeat": "weekly", "at": "08:15", "weekdays": ["mon", "thu"], "title": "Checks"});
    call(
        &link,
        1,
        "schedules/set",
        json!({"id": "checks", "task": task}),
    )
    .expect("set");
    let listed = call(&link, 2, "schedules/list", json!({})).expect("listed");
    assert_eq!(listed["tasks"][0]["title"], "Checks", "{listed}");
    assert_eq!(listed["tasks"][0]["weekdays"], json!([1, 4]));
    let file = std::fs::read_to_string(host.scratch.0.join("canoa.cfg")).expect("canoa.cfg");
    assert!(
        file.contains("[schedules.checks]") && file.contains("08:15"),
        "{file}"
    );
    assert!(
        file.contains("[host.agents.fixture]"),
        "what else the person wrote stays: {file}"
    );

    let bad = json!({"entry": "fixture", "folder": "/work", "prompt": "check", "at": "later"});
    let (code, _) =
        call(&link, 3, "schedules/set", json!({"id": "bad", "task": bad})).expect_err("refused");
    assert_eq!(code, -32602);
    let file = std::fs::read_to_string(host.scratch.0.join("canoa.cfg")).expect("canoa.cfg");
    assert!(!file.contains("[schedules.bad]"), "{file}");

    call(&link, 4, "schedules/remove", json!({"id": "checks"})).expect("removed");
    let listed = call(&link, 5, "schedules/list", json!({})).expect("listed");
    assert_eq!(
        listed["tasks"].as_array().map(Vec::len),
        Some(0),
        "{listed}"
    );
    let (code, _) = call(&link, 6, "schedules/run", json!({"id": "checks"})).expect_err("none");
    assert_eq!(code, -32003);
    host.stop();
}
