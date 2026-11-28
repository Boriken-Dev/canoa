//! What `canoa options --entry` says of an entry that runs a program speaking ACP: its models and efforts, read from a short
//! session of the program's own that sends no prompt, once per version of the program.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod support;

use std::path::Path;

use serde_json::Value;
use support::{Scratch, example, host_binary, host_command, quoted};

fn config(scratch: &Scratch, program: &Path, args: &[&str]) -> String {
    let list = args
        .iter()
        .map(|arg| quoted(arg))
        .collect::<Vec<_>>()
        .join(", ");
    let file = scratch.0.join("canoa.cfg");
    std::fs::write(
        &file,
        format!(
            "[host.agents.Probe]\nprogram = {}\nargs = [{list}]\nlisten = \"127.0.0.1:0\"\n",
            quoted(&program.to_string_lossy())
        ),
    )
    .expect("a config");
    file.to_string_lossy().into_owned()
}

fn options(scratch: &Scratch, config: &str, more: &[&str]) -> Value {
    let output = host_command()
        .args([
            "options", "Probe", "--entry", "--config", config, "--format", "json",
        ])
        .args(more)
        .env("COQUI_AGENTIC_PEER_HOME", scratch.0.join("home"))
        .output()
        .expect("canoa runs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("one JSON object")
}

#[test]
fn an_agent_that_lists_in_its_session_lists_before_one_starts() {
    let scratch = Scratch::new();
    let record = scratch.0.join("methods.txt");
    let record_arg = format!("--record={}", record.to_string_lossy());
    let config = config(
        &scratch,
        &example("stdio_agent", false),
        &["--scripted", "--closeable", &record_arg],
    );
    let answer = options(&scratch, &config, &[]);
    assert_eq!(answer["known"], true, "{answer}");
    assert!(answer.get("unknown_because").is_none(), "{answer}");
    let ids: Vec<&str> = answer["models"]
        .as_array()
        .expect("models")
        .iter()
        .filter_map(|model| model["id"].as_str())
        .collect();
    assert_eq!(ids, ["default", "fast", "refuse-me"], "{answer}");
    assert_eq!(
        answer["models"][0]["efforts"],
        serde_json::json!(["low", "high"])
    );
    assert_eq!(answer["models"][0]["default_effort"], "low");
    assert_eq!(answer["models"][0]["is_default"], true);
    let seen = std::fs::read_to_string(&record).expect("the agent recorded its methods");
    let methods: Vec<&str> = seen.lines().collect();
    assert_eq!(
        methods,
        ["initialize", "session/new", "session/close"],
        "a probe opens one session, closes it, and sends no prompt"
    );
}

#[test]
fn an_agent_that_cannot_close_is_still_ended_without_a_prompt() {
    let scratch = Scratch::new();
    let record = scratch.0.join("methods.txt");
    let record_arg = format!("--record={}", record.to_string_lossy());
    let config = config(
        &scratch,
        &example("stdio_agent", false),
        &["--scripted", &record_arg],
    );
    let answer = options(&scratch, &config, &[]);
    assert_eq!(answer["known"], true, "{answer}");
    let seen = std::fs::read_to_string(&record).expect("recorded");
    assert!(
        !seen.contains("session/prompt") && !seen.contains("session/close"),
        "{seen}"
    );
}

#[test]
fn an_agent_whose_session_lists_nothing_says_so_plainly() {
    let scratch = Scratch::new();
    let config = config(&scratch, &example("stdio_agent", false), &[]);
    let answer = options(&scratch, &config, &[]);
    assert_eq!(answer["known"], false, "{answer}");
    assert!(answer.get("unknown_kind").is_none(), "{answer}");
    let why = answer["unknown_because"].as_str().expect("a reason");
    assert!(why.contains("lists no models"), "{why}");
}

#[test]
fn a_program_that_is_not_there_is_not_installed() {
    let scratch = Scratch::new();
    let config = config(&scratch, &scratch.0.join("no-such-agent"), &[]);
    let answer = options(&scratch, &config, &[]);
    assert_eq!(answer["known"], false, "{answer}");
    assert_eq!(answer["unknown_kind"], "not_installed", "{answer}");
}

#[test]
fn a_program_that_ends_without_answering_fails() {
    let scratch = Scratch::new();
    // A program that ends without answering `initialize`.
    let config = config(&scratch, Path::new(host_binary()), &["--version"]);
    let started = std::time::Instant::now();
    let answer = options(&scratch, &config, &["--wait", "2"]);
    assert_eq!(answer["known"], false, "{answer}");
    assert_eq!(answer["unknown_kind"], "failed", "{answer}");
    assert!(answer["unknown_because"].is_string(), "{answer}");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(30),
        "bounded by --wait"
    );
}

#[test]
fn a_probe_is_asked_once_per_version_of_the_program() {
    let scratch = Scratch::new();
    let record = scratch.0.join("methods.txt");
    let record_arg = format!("--record={}", record.to_string_lossy());
    let file = config(
        &scratch,
        &example("stdio_agent", false),
        &["--scripted", &record_arg],
    );
    let text = std::fs::read_to_string(&file).expect("the config");
    let config = canoa_host::Config::from_str_in(&text, &scratch.0).expect("a config");
    let first = canoa_host::options::entry(&config, "Probe").expect("an entry");
    let again = canoa_host::options::entry(&config, "Probe").expect("an entry");
    assert!(first.known, "{first:?}");
    assert_eq!(first, again);
    assert_eq!(first.program_version.as_deref(), Some("stdio_agent 1.0"));
    let seen = std::fs::read_to_string(&record).expect("recorded");
    assert_eq!(seen.matches("session/new").count(), 1, "{seen}");
}
