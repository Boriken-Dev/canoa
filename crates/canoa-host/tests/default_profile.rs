//! The default profile (canoa.cfg's top level: `[profile]`, `[overlays]`, `[profiles.default]`) applies to every agent entry that
//! names no profile, written by hand with no `profiles` key: a Claude Code and a Codex stand-in and a program on its standard
//! streams get the overlay's root and skills and the profile's access; an entry naming another profile gets that one, and one
//! naming an empty profile gets nothing of the default.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod support;

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use canoa_host::profile::PROFILE_META;
use serde_json::{Value, json};
use support::{Host, Scratch, cwd, example, quoted, with_link};

/// The variables a stand-in reports the value of.
const ASKED: &str = "AGENTS_PROFILE,TEAM_OVERLAY_ROOT,TEAM_TOOL";

/// canoa.cfg's top level: overlay `team` and a default profile that reads the workspace only.
const TOP: &str =
    "[overlays]\nteam = \"fixture\"\n[profiles.default]\naccess = { sandbox = \"read-only\" }\n";

/// Overlay `team` in `scratch`'s `overlays/`, as the app installs one: it declares `TEAM_TOOL` and carries the skill `review`.
fn install_overlay(scratch: &Scratch) {
    let root = scratch.0.join("overlays").join("team");
    std::fs::create_dir_all(root.join("skills").join("review")).expect("a directory");
    std::fs::write(
        root.join("skills").join("review").join("SKILL.md"),
        "---\nname: review\ndescription: Review it.\n---\nReview it.\n",
    )
    .expect("written");
    std::fs::write(root.join("tool"), "x").expect("written");
    std::fs::write(
        root.join("overlay.cfg"),
        "[env]\nTEAM_TOOL = \"${AOS_OVERLAY_ROOT}/tool\"\n",
    )
    .expect("written");
    let (_, digest) =
        coqui_aos::read_tree(&coqui_aos::StdFs, &root, &coqui_aos::Settings::default())
            .expect("read");
    std::fs::write(
        scratch.0.join("overlays").join("team.json"),
        json!({"source": "fixture", "commit": "0", "digest": digest.digest.to_string(), "files": {}})
            .to_string(),
    )
    .expect("written");
}

fn standin(scratch: &Scratch, name: &str) -> PathBuf {
    let path = scratch
        .0
        .join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(example("harness_standin", false), &path).expect("the stand-in is copied");
    path
}

fn records(file: &Path) -> Vec<Value> {
    std::fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).expect("one JSON line"))
        .collect()
}

/// Everything a harness or the stand-in reads is the test's own folder, never the person's.
fn homes(scratch: &Scratch) -> Vec<(&'static str, OsString)> {
    let home = scratch.0.join("home");
    std::fs::create_dir_all(&home).expect("a directory");
    [
        "HOME",
        "USERPROFILE",
        "CLAUDE_CONFIG_DIR",
        "CODEX_HOME",
        "PI_CODING_AGENT_DIR",
    ]
    .into_iter()
    .map(|name| (name, home.clone().into_os_string()))
    .collect()
}

/// A host with one entry `Agent` (`entry` is the rest of its table, then any other tables), the stand-ins' record, and the
/// answers of its first `initialize` and `session/new`.
struct Started {
    initialized: Value,
    opened: Value,
    record: Vec<Value>,
    _host: Host,
}

fn start(entry: &dyn Fn(&Scratch) -> String, extra_env: &[(&'static str, &str)]) -> Started {
    let scratch = Scratch::new();
    install_overlay(&scratch);
    let record = scratch.0.join("record.jsonl");
    let mut env = homes(&scratch);
    env.push(("STANDIN_RECORD", record.clone().into_os_string()));
    env.push(("STANDIN_ENV", OsString::from(ASKED)));
    for (name, value) in extra_env {
        env.push((name, OsString::from(value)));
    }
    let config = format!("{TOP}{}", entry(&scratch));
    let host = Host::start_in(scratch, &config, 1, &env);
    let client = support::key();
    host.pair(&client);
    let dir = cwd(&host.scratch);
    let (initialized, opened) = with_link(host.url("Agent"), &client, host.peer, |rpc| {
        let initialized = rpc.initialize().expect("initialize is answered");
        let opened = rpc
            .request("session/new", json!({"cwd": dir, "mcpServers": []}))
            .expect("a session opens");
        (initialized, opened)
    })
    .expect("a paired key is served");
    Started {
        initialized,
        opened,
        record: records(&record),
        _host: host,
    }
}

fn harness_entry(harness: &'static str, extra: &'static str) -> impl Fn(&Scratch) -> String {
    move |scratch| {
        let program = standin(scratch, harness);
        format!(
            "[host.agents.Agent]\nharness = \"{harness}\"\nexecutable = {}\n{extra}",
            quoted(&program.to_string_lossy())
        )
    }
}

fn program_entry(name_args: &'static str, extra: &'static str) -> impl Fn(&Scratch) -> String {
    move |_| {
        format!(
            "[host.agents.Agent]\nprogram = {}\n{name_args}{extra}",
            quoted(&example("profile_agent", false).to_string_lossy())
        )
    }
}

fn item<'a>(report: &'a Value, key: &str) -> Option<&'a Value> {
    report["items"]
        .as_array()?
        .iter()
        .find(|item| item["key"] == key)
}

fn args_of(start: &Value) -> Vec<String> {
    start["args"]
        .as_array()
        .expect("its arguments")
        .iter()
        .map(|arg| arg.as_str().expect("text").to_owned())
        .collect()
}

fn thread_of(started: &Started) -> &Value {
    started
        .record
        .iter()
        .find(|entry| entry["method"] == "thread/start")
        .expect("a thread started")
}

fn assert_default_reported(started: &Started) {
    let report = &started.initialized["_meta"][PROFILE_META];
    assert_eq!(report["name"], "default", "{report}");
    let said = item(report, "profile").unwrap_or_else(|| panic!("no profile item in {report}"));
    assert_eq!(said["state"], "applied", "{said}");
    assert!(
        said["how"]
            .as_str()
            .is_some_and(|how| how.contains("names no profile")),
        "{said}"
    );
}

#[test]
fn a_hand_written_claude_entry_with_no_profile_gets_the_default_profile() {
    let started = start(&harness_entry("claude", ""), &[]);
    let first = &started.record[0];
    assert_eq!(first["env"]["AGENTS_PROFILE"], "default", "{first}");
    assert!(
        first["env"]["TEAM_OVERLAY_ROOT"]
            .as_str()
            .is_some_and(|root| root.ends_with("team")),
        "the overlay's root: {first}"
    );
    assert_eq!(first["plugin_skills"], json!(["review"]), "{first}");
    assert!(
        args_of(first).contains(&"--permission-mode=plan".to_owned()),
        "the default profile's read-only access: {first}"
    );
    assert_default_reported(&started);
}

#[test]
fn a_hand_written_codex_entry_with_no_profile_gets_the_default_profile() {
    let started = start(&harness_entry("codex", ""), &[]);
    let first = &started.record[0];
    assert_eq!(first["env"]["AGENTS_PROFILE"], "default", "{first}");
    assert!(
        first["env"]["TEAM_OVERLAY_ROOT"]
            .as_str()
            .is_some_and(|root| root.ends_with("team")),
        "the overlay's root: {first}"
    );
    assert_eq!(
        thread_of(&started)["params"]["sandbox"],
        "read-only",
        "the default profile's access"
    );
    assert_default_reported(&started);
}

#[test]
fn a_hand_written_program_entry_with_no_profile_gets_the_default_profile() {
    // An entry named Grok is given the access in Grok's own words.
    let started = start(
        &|scratch| {
            let _ = scratch;
            format!(
                "[host.agents.Agent]\nprogram = {}\nargs = [\"agent\", \"stdio\"]\n",
                quoted(&example("profile_agent", false).to_string_lossy())
            )
        },
        &[("PROFILE_AGENT_ENV", "TEAM_TOOL")],
    );
    let session = &started.opened["session_env"];
    assert_eq!(session["AGENTS_PROFILE"], "default", "{session}");
    assert!(
        session["TEAM_OVERLAY_ROOT"]
            .as_str()
            .is_some_and(|root| root.ends_with("team")),
        "the overlay's root: {session}"
    );
    assert!(
        session["TEAM_TOOL"]
            .as_str()
            .is_some_and(|tool| tool.ends_with("tool")),
        "an overlay's own variable: {session}"
    );
    assert_default_reported(&started);
}

#[test]
fn an_entry_naming_another_profile_gets_that_one() {
    let main = "profiles = [\"main\"]\n[host.profiles.main]\ncontext = { text = \"The named profile.\" }\n";
    let started = start(&program_entry("", main), &[]);
    assert_eq!(started.opened["systemPrompt"], "The named profile.");
    assert_eq!(started.opened["session_env"]["AGENTS_PROFILE"], "main");
    assert_eq!(
        started.opened["session_env"]["TEAM_OVERLAY_ROOT"],
        Value::Null
    );
    let report = &started.initialized["_meta"][PROFILE_META];
    assert!(item(report, "profile").is_none(), "{report}");
    let started = start(&harness_entry("codex", main), &[]);
    assert_eq!(started.record[0]["env"]["AGENTS_PROFILE"], "main");
    assert_eq!(
        thread_of(&started)["params"]["sandbox"],
        "danger-full-access",
        "the default profile's access does not reach an entry that names its own"
    );
}

#[test]
fn an_entry_naming_an_empty_profile_runs_bare() {
    let bare = "profiles = [\"bare\"]\n[host.profiles.bare]\n";
    let started = start(&program_entry("", bare), &[]);
    let session = &started.opened["session_env"];
    assert_eq!(session["AGENTS_PROFILE"], "bare", "{session}");
    assert_eq!(session["TEAM_OVERLAY_ROOT"], Value::Null, "{session}");
    let started = start(&harness_entry("claude", bare), &[]);
    let first = &started.record[0];
    assert_eq!(first["env"]["TEAM_OVERLAY_ROOT"], Value::Null, "{first}");
    assert_eq!(first["plugin_skills"], json!([]), "{first}");
    assert!(
        !args_of(first).contains(&"--permission-mode=plan".to_owned()),
        "{first}"
    );
}

#[test]
fn check_names_the_profile_each_entry_starts_with() {
    let scratch = Scratch::new();
    let path = scratch.0.join("canoa.cfg");
    std::fs::write(
        &path,
        format!(
            "{TOP}[host.agents.Plain]
program = \"x\"
[host.agents.Named]
program = \"x\"
profiles = [\"p\"]
             [host.profiles.p]
[host.agents.Mine]
harness = \"claude\"
"
        ),
    )
    .expect("written");
    let run = |path: &Path| {
        let checked = std::process::Command::new(support::host_binary())
            .args([
                "check",
                "--format",
                "json",
                "--config",
                &path.to_string_lossy(),
            ])
            .env("COQUI_AGENTIC_PEER_HOME", scratch.0.join("home"))
            .output()
            .expect("runs");
        assert!(checked.status.success(), "{checked:?}");
        String::from_utf8_lossy(&checked.stdout)
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).expect("JSON"))
            .map(|line| {
                (
                    line["name"].as_str().unwrap_or_default().to_owned(),
                    line["profile"].as_str().unwrap_or_default().to_owned(),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    let said = run(&path);
    assert!(said["Plain"].starts_with("default"), "{said:?}");
    assert!(said["Mine"].starts_with("default"), "{said:?}");
    assert_eq!(said["Named"], "p (named by the entry)", "{said:?}");
    std::fs::write(
        &path,
        "[host.agents.Plain]
program = \"x\"
",
    )
    .expect("written");
    assert!(run(&path)["Plain"].starts_with("none"));
}

#[test]
fn an_entry_that_sets_own_profile_is_refused_naming_the_key() {
    let parsed = canoa_host::Config::from_str_in(
        "[host.agents.a]
program = \"x\"
own_profile = true
",
        Path::new("."),
    );
    let error = parsed.expect_err("an unknown key").to_string();
    assert!(error.contains("own_profile"), "{error}");
}
