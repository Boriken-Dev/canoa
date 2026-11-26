//! Telemetry export through a whole host: a stand-in agent (`stdio_agent --scripted`) runs a turn whose prompt, working folder,
//! account name, file paths and tool inputs all hold a canary, and what the `file` exporter wrote is searched for it. The host
//! exports nothing unless the person turned it on and accepted the list, and says so in what it exports when it does.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod linked;
mod support;

use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};

use canoa_host::Link;
use linked::{Hosted, entry, next, open_session, send};
use serde_json::{Value, json};
use support::{cwd, quoted};

/// The host of one test installs the process's telemetry, so the tests take turns.
static ONE_HOST: Mutex<()> = Mutex::new(());

fn one_host() -> MutexGuard<'static, ()> {
    ONE_HOST.lock().unwrap_or_else(PoisonError::into_inner)
}

const ACCOUNT: &str = "canary_account_5521";

fn config(dir: &Path, telemetry: &str) -> String {
    format!("{}\n{telemetry}", entry(ACCOUNT, "args = [\"--scripted\"]"),).replace(
        "{OUT}",
        &dir.join("out").to_string_lossy().replace('\\', "/"),
    )
}

fn accepted() -> String {
    format!(
        "[host.telemetry]\nenabled = true\nacknowledged = \"{}\"\n\n[host.telemetry.exporters.local]\nprovider = \"file\"\nbatch_seconds = 1\nsettings = {{ folder = {} }}\n",
        canoa_host::telemetry_digest(),
        quoted("{OUT}"),
    )
}

/// Runs a prompt and answers each request for permission by allowing it, as a person who always says yes does.
fn turn(link: &Link, id: u64, session: &str, text: &str) -> Value {
    send(
        link,
        &json!({"jsonrpc": "2.0", "id": id, "method": "session/prompt", "params": {
            "sessionId": session, "prompt": [{"type": "text", "text": text}],
        }}),
    );
    loop {
        let message = next(link).expect("the turn goes on");
        if message["method"] == "session/request_permission" {
            let allow = message["params"]["options"]
                .as_array()
                .and_then(|options| {
                    options.iter().find(|option| {
                        option["kind"]
                            .as_str()
                            .is_some_and(|kind| kind.starts_with("allow"))
                    })
                })
                .and_then(|option| option["optionId"].as_str())
                .expect("an option that allows")
                .to_owned();
            send(
                link,
                &json!({"jsonrpc": "2.0", "id": message["id"], "result": {"outcome": {"outcome": "selected", "optionId": allow}}}),
            );
        } else if message["id"] == id && message.get("method").is_none() {
            return message["result"].clone();
        }
    }
}

/// Everything the exporter wrote, as text, and the lines parsed.
fn written(out: &Path) -> (String, Vec<(String, Value)>) {
    let mut text = String::new();
    let mut lines = Vec::new();
    let Ok(entries) = std::fs::read_dir(out) else {
        return (text, lines);
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        for line in std::fs::read_to_string(entry.path()).expect("read").lines() {
            text.push_str(line);
            text.push('\n');
            lines.push((
                name.clone(),
                serde_json::from_str(line).expect("a line is JSON"),
            ));
        }
    }
    (text, lines)
}

fn names_in(lines: &[(String, Value)], group: &str, path: &[&str]) -> Vec<String> {
    lines
        .iter()
        .filter(|(name, _)| name.starts_with(group))
        .flat_map(|(_, line)| {
            let mut at = &line[path[0]][0][path[1]][0][path[2]];
            if at.is_null() {
                at = &Value::Null;
            }
            at.as_array().cloned().unwrap_or_default()
        })
        .filter_map(|record| {
            record["name"]
                .as_str()
                .or_else(|| record["eventName"].as_str())
                .map(str::to_owned)
        })
        .collect()
}

#[test]
fn a_turn_through_a_stand_in_agent_exports_its_shape_and_nothing_it_said() {
    let _one = one_host();
    let host = Hosted::start_with_setup(|dir| config(dir, &accepted()), &[]);
    let out = host.scratch.0.join("out");
    let folder = cwd(&host.scratch);
    let link = host.local.open(ACCOUNT).expect("a link");
    let session = open_session(&link, &folder);
    // The working folder the agent was given, and a name that no signal may carry.
    let result = turn(
        &link,
        3,
        &session,
        "Rename CANARY_PROMPT_7731 in src/calc.js with sk-CANARY_SECRET_9902",
    );
    assert_eq!(result["stopReason"], "end_turn");
    drop(link);
    host.stop();
    assert!(host.ended.join().expect("no panic").is_ok());

    let (text, lines) = written(&out);
    assert!(
        !text.is_empty(),
        "something was exported to {}",
        out.display()
    );
    let scratch = host.scratch.0.to_string_lossy().replace('\\', "/");
    let folder_name = Path::new(&folder)
        .file_name()
        .expect("a name")
        .to_string_lossy()
        .into_owned();
    for canary in [
        "CANARY",
        "calc.js",
        "src/",
        ACCOUNT,
        scratch.as_str(),
        folder_name.as_str(),
        "echo:",
    ] {
        assert!(
            !text.contains(canary),
            "{canary} reached what was exported:\n{text}"
        );
    }
    // The control: the search finds a canary that is there.
    assert!(format!("{text}CANARY").contains("CANARY"));

    let spans = names_in(&lines, "spans-", &["resourceSpans", "scopeSpans", "spans"]);
    for span in ["canoa.session", "canoa.turn", "canoa.permission"] {
        assert!(spans.iter().any(|name| name == span), "{span} in {spans:?}");
    }
    let metrics = names_in(
        &lines,
        "metrics-",
        &["resourceMetrics", "scopeMetrics", "metrics"],
    );
    assert!(
        metrics.iter().any(|name| name == "canoa.turns"),
        "{metrics:?}"
    );
    let logs = names_in(
        &lines,
        "logs-",
        &["resourceLogs", "scopeLogs", "logRecords"],
    );
    assert!(logs.iter().any(|name| name == "host.started"), "{logs:?}");
    assert!(logs.iter().any(|name| name == "host.stopped"), "{logs:?}");
    let outcome = text
        .lines()
        .find(|line| line.contains("canoa.permission.outcome"))
        .expect("a permission span");
    assert!(outcome.contains("\"allow\""), "{outcome}");
}

#[test]
fn a_host_with_no_telemetry_table_exports_and_writes_nothing() {
    let _one = one_host();
    let host = Hosted::start_with_setup(|dir| config(dir, ""), &[]);
    let link = host.local.open(ACCOUNT).expect("a link");
    let session = open_session(&link, &cwd(&host.scratch));
    turn(&link, 3, &session, "Say hello");
    drop(link);
    host.stop();
    assert!(host.ended.join().expect("no panic").is_ok());
    assert!(!host.scratch.0.join("out").exists());
    assert!(
        !host.scratch.0.join("home").join("telemetry").exists(),
        "not even the folder the exporter would use"
    );
}

#[test]
fn a_list_that_was_accepted_before_it_changed_pauses_export_until_it_is_accepted_again() {
    let _one = one_host();
    let stale = accepted().replace(&canoa_host::telemetry_digest(), "0000000000000000");
    let host = Hosted::start_with_setup(|dir| config(dir, &stale), &[]);
    let link = host.local.open(ACCOUNT).expect("a link");
    let session = open_session(&link, &cwd(&host.scratch));
    turn(&link, 3, &session, "Say hello");
    drop(link);
    host.stop();
    assert!(host.ended.join().expect("no panic").is_ok());
    assert!(!host.scratch.0.join("out").exists(), "nothing was exported");
}

#[test]
fn an_exporter_that_names_no_folder_writes_inside_the_hosts_home() {
    let _one = one_host();
    let on = "[host.telemetry]\nenabled = true\nacknowledged = \"DIGEST\"\n\n[host.telemetry.exporters.here]\nprovider = \"file\"\nbatch_seconds = 1\n"
        .replace("DIGEST", &canoa_host::telemetry_digest());
    let host = Hosted::start_with_setup(|dir| config(dir, &on), &[]);
    let link = host.local.open(ACCOUNT).expect("a link");
    let session = open_session(&link, &cwd(&host.scratch));
    turn(&link, 3, &session, "Say hello");
    drop(link);
    host.stop();
    assert!(host.ended.join().expect("no panic").is_ok());
    let (text, _) = written(&host.scratch.0.join("home").join("telemetry"));
    assert!(text.contains("canoa.turn"), "{text}");
}
