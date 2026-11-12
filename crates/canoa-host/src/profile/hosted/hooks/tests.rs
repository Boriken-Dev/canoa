use core::sync::atomic::{AtomicU64, Ordering};

use alloc::sync::Arc;

use super::*;
use crate::config::Config;
use crate::profile::hosted::{Connection, offer};

#[path = "../../../../tests/hooked/script.rs"]
mod script;

use script::Arm;

/// A directory of the test's own, removed afterwards.
struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "canoa-hooks-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a directory");
        Self(dir)
    }

    fn log(&self) -> PathBuf {
        self.0.join("hook.log")
    }

    /// Installs overlay `team` as the app does, and returns its digest.
    fn install(&self, manifest: &str, arms: &[(&str, Arm)]) -> String {
        let root = self.0.join("overlays/team");
        std::fs::create_dir_all(&root).expect("a directory");
        std::fs::write(root.join("overlay.cfg"), manifest).expect("written");
        std::fs::write(
            root.join(script::file_name()),
            script::body(&self.log(), arms),
        )
        .expect("written");
        let (_, digest) =
            aos::read_tree(&aos::StdFs, &root, &aos::Settings::default()).expect("read");
        let digest = digest.digest.to_string();
        std::fs::write(
            self.0.join("overlays/team.json"),
            json!({"source": "x", "commit": "y", "digest": digest, "files": {}}).to_string(),
        )
        .expect("written");
        digest
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn real(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

fn without_path(name: &str) -> Option<String> {
    (name != "PATH").then(|| real(name)).flatten()
}

fn connect(
    temp: &Temp,
    allowed: Option<&str>,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Result<Connection, String> {
    let allow = allowed
        .map(|digest| format!("allow_hooks = {{ team = \"{digest}\" }}\n"))
        .unwrap_or_default();
    let config = Config::from_str_in(
        &format!(
            "[host.agents.a]\nprogram = \"agent\"\nprofiles = [\"main\"]\n\
             [host.profiles.main]\noverlays = [\"team\"]\n{allow}"
        ),
        &temp.0,
    )
    .expect("parses");
    let crate::config::EntryKind::Agent { profiles, .. } = &config.entries[0].kind else {
        panic!("an agent");
    };
    let offer = Arc::into_inner(
        offer(profiles, &config.profiles, None, "launcher".into()).expect("an offer"),
    )
    .expect("one owner");
    Connection::prepare(&offer, 0, "a", lookup)
}

fn prompt(connection: &Connection, text: &str) -> aos::FiredEvent {
    let mut known = Map::new();
    known.insert("harness".to_owned(), json!("claude-code"));
    known.insert("cwd".to_owned(), json!("/work"));
    known.insert("session".to_owned(), json!("canoa-session-1"));
    known.insert("prompt".to_owned(), json!(text));
    connection.hooks.fire(
        &connection.overlays,
        HookEvent::Prompt,
        &known,
        None,
        &BTreeSet::new(),
        &["context", "block"],
    )
}

fn causes(fired: &aos::FiredEvent) -> Vec<String> {
    fired
        .reports
        .iter()
        .filter(|report| report.id == "hook-failed")
        .map(|report| report.about[2].clone())
        .collect()
}

fn env_of<'a>(connection: &'a Connection, name: &str) -> Option<&'a str> {
    connection
        .env()
        .iter()
        .find(|(each, _)| each == name)
        .map(|(_, value)| value.expose())
}

#[test]
fn a_pre_env_hook_sets_variables_the_agent_is_given_and_not_those_an_overlay_may_not_set() {
    let temp = Temp::new();
    let digest = temp.install(
        "",
        &[(
            "pre-env",
            Arm::Print(
                r#"{"PRE_VAR":"from-hook","HTTPS_PROXY":"http://proxy.example"}"#.to_owned(),
            ),
        )],
    );
    let connection = connect(&temp, Some(&digest), &real).expect("prepared");
    assert_eq!(env_of(&connection, "PRE_VAR"), Some("from-hook"));
    assert_eq!(env_of(&connection, "HTTPS_PROXY"), None);
    let refused = connection
        .notes
        .iter()
        .find(|item| item.key == "hooks.team.pre-env.HTTPS_PROXY")
        .expect("said");
    assert_eq!(refused.state, State::Refused);
    let told = script::logged(&temp.log());
    assert_eq!(told.len(), 1, "{told:?}");
    let keys: BTreeSet<&str> = told[0]
        .1
        .as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, BTreeSet::from(["event", "harness", "cwd"]));
}

#[test]
fn a_pre_env_hook_that_is_not_allowed_at_this_digest_does_not_run() {
    let temp = Temp::new();
    temp.install(
        "",
        &[(
            "pre-env",
            Arm::Print(r#"{"PRE_VAR":"from-hook"}"#.to_owned()),
        )],
    );
    for allowed in [None, Some(format!("sha256:{}", "0".repeat(64)))] {
        let connection = connect(&temp, allowed.as_deref(), &real).expect("prepared");
        assert_eq!(env_of(&connection, "PRE_VAR"), None);
        assert!(script::logged(&temp.log()).is_empty());
    }
}

#[test]
fn a_pre_env_guard_that_fails_starts_no_agent() {
    let temp = Temp::new();
    let digest = temp.install(
        "[hooks]\npre-env = { on_failure = \"refuse\" }\n",
        &[("pre-env", Arm::Exit(1))],
    );
    let error = connect(&temp, Some(&digest), &real).expect_err("no agent starts");
    assert!(error.contains("overlay:team"), "{error}");
    // The same hook failing without being a guard only reports.
    let temp = Temp::new();
    let digest = temp.install("", &[("pre-env", Arm::Exit(1))]);
    let connection = connect(&temp, Some(&digest), &real).expect("prepared");
    let note = connection
        .notes
        .iter()
        .find(|item| item.key == "hooks.team.pre-env")
        .expect("said");
    assert_eq!(note.state, State::Refused);
    assert!(
        note.how.contains("hook-failed") && note.how.contains("exit"),
        "{}",
        note.how
    );
}

#[test]
fn a_hook_is_given_its_event_and_its_roots_copy_and_nothing_of_the_hosts_environment() {
    let temp = Temp::new();
    let outside = std::env::vars()
        .find(|(name, value)| {
            !value.is_empty()
                && name.chars().all(|c| c.is_ascii_alphabetic() || c == '_')
                && !PASSED
                    .iter()
                    .any(|passed| passed.eq_ignore_ascii_case(name))
                && !name.eq_ignore_ascii_case("PATH")
        })
        .map(|(name, _)| name);
    let Some(outside) = outside else {
        return;
    };
    let digest = temp.install("", &[("pre-env", Arm::LogEnv(outside))]);
    connect(&temp, Some(&digest), &real).expect("prepared");
    let raw = std::fs::read_to_string(temp.log()).expect("logged");
    assert!(raw.lines().any(|line| line.trim() == "envprobe="), "{raw}");

    let temp = Temp::new();
    let digest = temp.install("", &[("pre-env", Arm::LogEnv("AOS_HOOK_EVENT".to_owned()))]);
    connect(&temp, Some(&digest), &real).expect("prepared");
    let raw = std::fs::read_to_string(temp.log()).expect("logged");
    assert!(
        raw.lines().any(|line| line.trim() == "envprobe=pre-env"),
        "{raw}"
    );

    let temp = Temp::new();
    let digest = temp.install("", &[("pre-env", Arm::LogEnv(aos::root_variable("team")))]);
    connect(&temp, Some(&digest), &real).expect("prepared");
    let raw = std::fs::read_to_string(temp.log()).expect("logged");
    let root = raw
        .lines()
        .find_map(|line| line.trim().strip_prefix("envprobe="))
        .expect("the root variable");
    assert!(
        Path::new(root).starts_with(std::env::temp_dir()) && !Path::new(root).starts_with(&temp.0),
        "a copy of the verified files, not the installed directory: {root}"
    );
}

#[test]
fn a_hook_runs_the_bytes_the_digest_covered_whatever_the_directory_holds_later() {
    let temp = Temp::new();
    let digest = temp.install(
        "[hooks]\nprompt = {}\n",
        &[("prompt", Arm::Print(r#"{"context":"ORIGINAL"}"#.to_owned()))],
    );
    let connection = connect(&temp, Some(&digest), &real).expect("prepared");
    std::fs::write(
        temp.0.join("overlays/team").join(script::file_name()),
        script::body(
            &temp.log(),
            &[("prompt", Arm::Print(r#"{"context":"TAMPERED"}"#.to_owned()))],
        ),
    )
    .expect("written");
    let fired = prompt(&connection, "hello");
    assert_eq!(
        fired.context,
        [("team".to_owned(), "ORIGINAL".to_owned())],
        "{:?}",
        causes(&fired)
    );
}

#[test]
fn every_way_a_run_fails_is_named_and_a_guard_blocks_on_each() {
    let cases: Vec<(&str, Arm, &str)> = vec![
        ("exit", Arm::Exit(3), "prompt = { on_failure = \"refuse\" }"),
        (
            "timeout",
            Arm::Sleep(30),
            "prompt = { on_failure = \"refuse\", timeout = 1 }",
        ),
        (
            "too-large",
            Arm::Big(70_000),
            "prompt = { on_failure = \"refuse\" }",
        ),
        (
            "grant",
            Arm::Print(r#"{"decision":"allow"}"#.to_owned()),
            "prompt = { on_failure = \"refuse\" }",
        ),
        (
            "grant",
            Arm::Print(r#"{"decision":""}"#.to_owned()),
            "prompt = { on_failure = \"refuse\" }",
        ),
        (
            "output",
            Arm::Print(r#"{"context":"x","surprise":1}"#.to_owned()),
            "prompt = { on_failure = \"refuse\" }",
        ),
        (
            "output",
            Arm::Print("not json".to_owned()),
            "prompt = { on_failure = \"refuse\" }",
        ),
    ];
    for (cause, arm, table) in cases {
        let temp = Temp::new();
        let digest = temp.install(&format!("[hooks]\n{table}\n"), &[("prompt", arm)]);
        let connection = connect(&temp, Some(&digest), &real).expect("prepared");
        let fired = prompt(&connection, "hello");
        assert_eq!(causes(&fired), [cause], "{table}");
        assert_eq!(
            fired.decision,
            Some(aos::Decision::Block),
            "{cause}: a guard blocks"
        );
        assert!(
            fired.context.is_empty(),
            "{cause}: nothing of a failed run applies"
        );
    }
}

#[test]
fn a_run_that_goes_over_a_bound_is_stopped_and_what_it_started_ends_with_it() {
    let temp = Temp::new();
    let digest = temp.install(
        "[hooks]\nprompt = { timeout = 1 }\n",
        &[("prompt", Arm::Sleep(30))],
    );
    let connection = connect(&temp, Some(&digest), &real).expect("prepared");
    let began = std::time::Instant::now();
    let fired = prompt(&connection, "hello");
    assert_eq!(causes(&fired), ["timeout"]);
    assert!(
        began.elapsed() < Duration::from_secs(20),
        "stopped, not waited for"
    );
    assert_eq!(
        fired.decision, None,
        "not a guard: reported, and the prompt goes on"
    );
}

#[test]
fn a_hook_with_no_interpreter_to_run_it_fails_and_a_payload_past_its_bound_is_not_run() {
    let temp = Temp::new();
    let digest = temp.install(
        "[hooks]\nprompt = { on_failure = \"refuse\" }\n",
        &[("prompt", Arm::Print("{}".to_owned()))],
    );
    let connection = connect(&temp, Some(&digest), &without_path).expect("prepared");
    let fired = prompt(&connection, "hello");
    assert_eq!(causes(&fired), ["not-run"]);
    assert_eq!(fired.decision, Some(aos::Decision::Block));

    let connection = connect(&temp, Some(&digest), &real).expect("prepared");
    let fired = prompt(&connection, &"x".repeat(2 * 1024 * 1024));
    assert_eq!(causes(&fired), ["payload-too-large"]);
    assert!(
        script::logged(&temp.log())
            .iter()
            .all(|(event, _)| event != "prompt"),
        "not run"
    );
}

#[test]
fn the_hooks_of_an_overlay_the_person_allows_for_a_session_run_and_those_of_one_nobody_allowed_do_not()
 {
    let temp = Temp::new();
    temp.install(
        "[hooks]\nprompt = {}\n",
        &[("prompt", Arm::Print(r#"{"context":"HI"}"#.to_owned()))],
    );
    let connection = connect(&temp, None, &real).expect("prepared");
    let mut known = Map::new();
    known.insert("prompt".to_owned(), json!("x"));
    let fire = |person: &BTreeSet<String>| {
        connection.hooks.fire(
            &connection.overlays,
            HookEvent::Prompt,
            &known,
            None,
            person,
            &["context", "block"],
        )
    };
    assert!(fire(&BTreeSet::new()).context.is_empty());
    let allowed = BTreeSet::from(["team".to_owned()]);
    assert_eq!(fire(&allowed).context.len(), 1);
    assert_eq!(
        connection
            .hooks
            .offered(&["team".to_owned(), "other".to_owned()]),
        allowed
    );
    assert_eq!(connection.hooks.approvable().len(), 1);
}

#[test]
fn context_a_hook_prints_is_kept_to_the_spec_s_characters() {
    let temp = Temp::new();
    let digest = temp.install(
        "[hooks]\nprompt = {}\n",
        &[(
            "prompt",
            Arm::Print("{\"context\":\"\\u001b[31mred\"}".to_owned()),
        )],
    );
    let connection = connect(&temp, Some(&digest), &real).expect("prepared");
    let fired = prompt(&connection, "hello");
    assert_eq!(causes(&fired), ["output"], "a terminal escape is refused");
    assert!(fired.context.is_empty());
}
