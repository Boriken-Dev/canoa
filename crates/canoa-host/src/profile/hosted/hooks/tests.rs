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
        self.install_as("team", manifest, arms, &[])
    }

    /// Installs overlay `name` with `extra` files besides its hook script, and returns its digest.
    fn install_as(
        &self,
        name: &str,
        manifest: &str,
        arms: &[(&str, Arm)],
        extra: &[(&str, &str)],
    ) -> String {
        let root = self.0.join("overlays").join(name);
        std::fs::create_dir_all(&root).expect("a directory");
        std::fs::write(root.join("overlay.cfg"), manifest).expect("written");
        std::fs::write(
            root.join(script::file_name()),
            script::body(&self.log(), arms),
        )
        .expect("written");
        for (path, text) in extra {
            std::fs::write(root.join(path), text).expect("written");
        }
        let (_, digest) =
            aos::read_tree(&aos::StdFs, &root, &aos::Settings::default()).expect("read");
        let digest = digest.digest.to_string();
        std::fs::write(
            self.0.join("overlays").join(format!("{name}.json")),
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
    let allow: Vec<(&str, &str)> = allowed.map(|digest| ("team", digest)).into_iter().collect();
    connect_overlays(temp, &["team"], &allow, lookup)
}

/// A connection whose profile lists `names` and allows the hooks of each `(overlay, digest)` of `allow`.
fn connect_overlays(
    temp: &Temp,
    names: &[&str],
    allow: &[(&str, &str)],
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Result<Connection, String> {
    let list: Vec<String> = names.iter().map(|name| format!("\"{name}\"")).collect();
    let allow = if allow.is_empty() {
        String::new()
    } else {
        let each: Vec<String> = allow
            .iter()
            .map(|(name, digest)| format!("{name} = \"{digest}\""))
            .collect();
        format!("allow_hooks = {{ {} }}\n", each.join(", "))
    };
    let config = Config::from_str_in(
        &format!(
            "[host.agents.a]\nprogram = \"agent\"\nprofiles = [\"main\"]\n\
             [host.profiles.main]\noverlays = [{}]\n{allow}",
            list.join(", ")
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
    prompt_after(connection, text, 0)
}

/// A prompt for a session that already holds `used` bytes of hook context.
fn prompt_after(connection: &Connection, text: &str, used: usize) -> aos::FiredEvent {
    let mut known = Map::new();
    known.insert("harness".to_owned(), json!("claude-code"));
    known.insert("cwd".to_owned(), json!("/work"));
    known.insert("session".to_owned(), json!("canoa-session-1"));
    known.insert("prompt".to_owned(), json!(text));
    connection.hooks.fire(
        &connection.overlays,
        &Firing {
            event: HookEvent::Prompt,
            known: &known,
            select: None,
            person: &BTreeSet::new(),
            applies: &["context", "block"],
            pre_env: &connection.pre_env.runs,
            context_used: used,
        },
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
            &Firing {
                event: HookEvent::Prompt,
                known: &known,
                select: None,
                person,
                applies: &["context", "block"],
                pre_env: &connection.pre_env.runs,
                context_used: 0,
            },
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

#[cfg(unix)]
#[test]
fn a_process_that_leaves_the_hooks_group_and_holds_its_output_does_not_hold_the_relay() {
    let temp = Temp::new();
    let digest = temp.install("[hooks]\nprompt = {}\n", &[("prompt", Arm::Daemon)]);
    let connection = connect(&temp, Some(&digest), &real).expect("prepared");
    let began = std::time::Instant::now();
    let fired = prompt(&connection, "hello");
    assert!(
        began.elapsed() < Duration::from_secs(10),
        "waited {:?} on a descendant: {:?}",
        began.elapsed(),
        causes(&fired)
    );
}

#[test]
fn context_past_the_sessions_bound_counts_utf8_bytes_is_left_whole_and_is_reported() {
    let temp = Temp::new();
    let digest = temp.install_as(
        "team",
        "[hooks]\nprompt = {}\n",
        &[(
            "prompt",
            Arm::Print("{\"context\":\"\\u00e9\\u00e9\"}".to_owned()),
        )],
        &[],
    );
    let mut connection = connect(&temp, Some(&digest), &real).expect("prepared");
    connection.hooks.settings = Settings::default().context_bytes(aos::Limit::at(5));
    // Two characters, four bytes: 2 + 4 passes 5 though 2 + 2 would not.
    let fired = prompt_after(&connection, "hello", 2);
    assert!(fired.context.is_empty(), "{:?}", fired.context);
    let report = fired
        .reports
        .iter()
        .find(|report| report.id == "hook-not-applied")
        .expect("said");
    assert_eq!(report.about.first().map(String::as_str), Some("team"));
    // What the session holds counts, and a part that reaches the bound exactly fits.
    assert_eq!(prompt_after(&connection, "hello", 1).context.len(), 1);
    assert_eq!(prompt_after(&connection, "hello", 0).context.len(), 1);
}

/// What the script logged of the variable it was asked about, one entry per run.
fn probed(temp: &Temp) -> Vec<String> {
    std::fs::read_to_string(temp.log())
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.trim().strip_prefix("envprobe="))
        .map(str::to_owned)
        .collect()
}

#[test]
fn an_overlay_whose_hooks_are_not_allowed_still_applies_and_runs_no_env_script_and_the_report_says_so()
 {
    let temp = Temp::new();
    let digest = temp.install_as(
        "team",
        "[env]\nTEAM = \"crew\"\n[hooks]\nprompt = {}\n",
        &[],
        &[("env.py", "print('x')\n")],
    );
    let connection = connect(&temp, None, &real).expect("prepared");
    assert_eq!(
        env_of(&connection, "TEAM"),
        Some("crew"),
        "the rest applies"
    );
    let said: Vec<&Item> = connection
        .notes
        .iter()
        .filter(|item| item.how.starts_with("env-script-not-allowed"))
        .collect();
    assert_eq!(said.len(), 1, "{:?}", connection.notes);
    assert_eq!(
        said[0].layer,
        LayerId::new(LayerKind::Overlay, "team").to_string()
    );
    assert_eq!(said[0].state, State::Off);
    // Allowed at another digest is not allowed.
    let other = format!("sha256:{}", "0".repeat(64));
    let connection = connect(&temp, Some(&other), &real).expect("prepared");
    assert!(
        connection
            .notes
            .iter()
            .any(|item| item.how.starts_with("env-script-not-allowed"))
    );
    // At this digest the overlay's env scripts are allowed, so nothing is held back.
    let connection = connect(&temp, Some(&digest), &real).expect("prepared");
    assert!(
        !connection
            .notes
            .iter()
            .any(|item| item.how.starts_with("env-script-not-allowed")),
        "{:?}",
        connection.notes
    );
    // An overlay with no env script has none to hold back.
    let temp = Temp::new();
    temp.install("[hooks]\nprompt = {}\n", &[]);
    let connection = connect(&temp, None, &real).expect("prepared");
    assert!(
        !connection
            .notes
            .iter()
            .any(|item| item.how.starts_with("env-script-not-allowed"))
    );
}

#[test]
fn a_hook_gets_its_overlays_declarations_name_and_root_and_a_pre_env_hook_does_not() {
    for (variable, expected) in [("TEAM", Some("crew")), ("AOS_OVERLAY_NAME", Some("team"))] {
        let temp = Temp::new();
        let digest = temp.install(
            "[env]\nTEAM = \"crew\"\n[hooks]\nprompt = {}\n",
            &[("prompt", Arm::LogEnv(variable.to_owned()))],
        );
        let connection = connect(&temp, Some(&digest), &real).expect("prepared");
        prompt(&connection, "hello");
        assert_eq!(probed(&temp), [expected.unwrap_or_default()], "{variable}");
    }
    let temp = Temp::new();
    let digest = temp.install(
        "[hooks]\nprompt = {}\n",
        &[("prompt", Arm::LogEnv("AOS_OVERLAY_ROOT".to_owned()))],
    );
    let connection = connect(&temp, Some(&digest), &real).expect("prepared");
    prompt(&connection, "hello");
    let root = probed(&temp);
    assert!(
        root.len() == 1 && Path::new(&root[0]).starts_with(std::env::temp_dir()),
        "{root:?}"
    );
    // A pre-env hook runs before any declaration is applied: only the base, the roots and its own.
    let temp = Temp::new();
    let digest = temp.install(
        "[env]\nTEAM = \"crew\"\n[hooks]\npre-env = {}\n",
        &[("pre-env", Arm::LogEnv("TEAM".to_owned()))],
    );
    connect(&temp, Some(&digest), &real).expect("prepared");
    assert_eq!(probed(&temp), [""]);
}

#[test]
fn a_hook_gets_what_pre_env_printed_but_not_what_another_overlay_declared() {
    let temp = Temp::new();
    let digest = temp.install(
        "[hooks]\npre-env = {}\nprompt = {}\n",
        &[
            (
                "pre-env",
                Arm::Print(r#"{"PRE_VAR":"from-hook"}"#.to_owned()),
            ),
            ("prompt", Arm::LogEnv("PRE_VAR".to_owned())),
        ],
    );
    let connection = connect(&temp, Some(&digest), &real).expect("prepared");
    prompt(&connection, "hello");
    assert_eq!(probed(&temp), ["from-hook"]);

    for (variable, expected) in [
        ("OTHER_VAR", ""),
        (aos::root_variable("other").as_str(), "*"),
    ] {
        let temp = Temp::new();
        let team = temp.install(
            "[hooks]\nprompt = {}\n",
            &[("prompt", Arm::LogEnv(variable.to_owned()))],
        );
        temp.install_as("other", "[env]\nOTHER_VAR = \"theirs\"\n", &[], &[]);
        let connection = connect_overlays(&temp, &["team", "other"], &[("team", &team)], &real)
            .expect("prepared");
        prompt(&connection, "hello");
        let seen = probed(&temp);
        assert_eq!(seen.len(), 1, "{variable}");
        if expected == "*" {
            assert!(
                !seen[0].is_empty(),
                "every overlay's root variable is given"
            );
        } else {
            assert_eq!(seen[0], expected, "another overlay's declaration is not");
        }
    }
}

#[test]
fn hooks_run_in_composition_order_whatever_order_the_profile_lists_the_overlays_in() {
    let temp = Temp::new();
    let mut allow = Vec::new();
    for (name, priority) in [("late", 600), ("early", 100)] {
        let digest = temp.install_as(
            name,
            &format!("priority = {priority}\n[hooks]\nprompt = {{}}\n"),
            &[(
                "prompt",
                Arm::Print(format!(r#"{{"context":"from {name}"}}"#)),
            )],
            &[],
        );
        allow.push((name, digest));
    }
    let allow: Vec<(&str, &str)> = allow
        .iter()
        .map(|(name, digest)| (*name, digest.as_str()))
        .collect();
    let connection = connect_overlays(&temp, &["late", "early"], &allow, &real).expect("prepared");
    let fired = prompt(&connection, "hello");
    let order: Vec<&str> = fired.context.iter().map(|(who, _)| who.as_str()).collect();
    assert_eq!(order, ["early", "late"]);
}

/// Gives a directory an old modification time.
fn age(dir: &Path, old: std::time::SystemTime) {
    let mut open = std::fs::OpenOptions::new();
    open.read(true);
    #[cfg(windows)]
    {
        open.write(true);
        std::os::windows::fs::OpenOptionsExt::custom_flags(&mut open, 0x0200_0000);
    }
    open.open(dir)
        .and_then(|handle| handle.set_modified(old))
        .expect("aged");
}

#[test]
fn copies_a_killed_host_left_in_the_temporary_directory_are_removed_when_old_and_only_those() {
    let temp = Temp::new();
    for name in ["canoa-hooks-old", "canoa-hooks-new", "other-old"] {
        std::fs::create_dir_all(temp.0.join(name)).expect("a directory");
    }
    let old = std::time::SystemTime::now() - Duration::from_secs(2 * 3600);
    for name in ["canoa-hooks-old", "other-old"] {
        age(&temp.0.join(name), old);
    }
    remove_stale(&temp.0);
    assert!(!temp.0.join("canoa-hooks-old").exists());
    assert!(temp.0.join("canoa-hooks-new").exists());
    assert!(temp.0.join("other-old").exists(), "only copies are removed");
}

#[test]
fn a_connections_copies_go_with_it() {
    let temp = Temp::new();
    let digest = temp.install(
        "[hooks]\nprompt = {}\n",
        &[("prompt", Arm::LogEnv(aos::root_variable("team")))],
    );
    let connection = connect(&temp, Some(&digest), &real).expect("prepared");
    prompt(&connection, "hello");
    let raw = std::fs::read_to_string(temp.log()).expect("logged");
    let root = raw
        .lines()
        .rev()
        .find_map(|line| line.trim().strip_prefix("envprobe="))
        .expect("the root variable")
        .to_owned();
    assert!(Path::new(&root).exists(), "kept while the connection lasts");
    drop(connection);
    assert!(!Path::new(&root).exists(), "removed with it");
}

fn outcome(unreadable: bool, timed_out: bool) -> run::Outcome {
    run::Outcome {
        started: true,
        stdout: Vec::new(),
        exit: -1,
        timed_out,
        unreadable,
        seconds: 1,
    }
}

fn fired(outcome: run::Outcome) -> FiredEvent {
    let mut hook = HookRun::new("team", Vec::new());
    hook.on_failure = Some(OnFailure::Report);
    apply(&mut hook, outcome);
    aos::fire_event(
        HookEvent::PreTool,
        &[hook],
        None,
        &Settings::default(),
        false,
    )
}

#[test]
fn a_run_whose_status_cannot_be_read_refuses_whatever_the_hook_says_on_failure() {
    // A deny the hook may have given is not to be dropped: the tool call is refused, though the hook only reports.
    assert_eq!(
        fired(outcome(true, false)).decision,
        Some(aos::Decision::Deny)
    );
    // Control: a hook that only reports, and ran too long, passes.
    assert_eq!(fired(outcome(false, true)).decision, None);
}

/// A hook the process around the host collects before the host asks, as Dart's process handling does, is a run whose status is
/// unreadable. It is ended at once rather than waited on to its time limit, and it is no timeout.
#[cfg(target_os = "linux")]
#[test]
fn a_hook_collected_by_someone_else_is_unreadable_and_not_a_timeout() {
    let scratch = Temp::new();
    let pid_file = scratch.0.join("pid");
    let collector_file = pid_file.clone();
    let collector = std::thread::spawn(move || {
        let until = std::time::Instant::now() + Duration::from_secs(20);
        while std::time::Instant::now() < until {
            let pid = std::fs::read_to_string(&collector_file)
                .ok()
                .and_then(|text| text.trim().parse::<i32>().ok())
                .and_then(rustix::process::Pid::from_raw);
            if let Some(pid) = pid {
                // The collection an application's exit handler does, of this process's child that has ended.
                while std::time::Instant::now() < until {
                    match rustix::process::waitpid(Some(pid), rustix::process::WaitOptions::NOHANG)
                    {
                        Ok(None) => std::thread::sleep(Duration::from_millis(1)),
                        _ => return,
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    });
    let env = [("PATH".to_owned(), std::env::var("PATH").unwrap_or_default())];
    let ran = run::run(&run::Job {
        program: Path::new("/bin/sh"),
        args: vec![
            "-c".into(),
            "echo $$ > \"$1\"; sleep 1; exit 2".into(),
            "sh".into(),
            pid_file.clone().into(),
        ],
        cwd: &scratch.0,
        env: &env,
        input: b"{}",
        timeout: Duration::from_secs(30),
        max_output: 1024,
    });
    collector.join().expect("the collector ended");
    assert!(ran.started);
    assert!(
        !ran.timed_out,
        "no timeout: it was never waited on to its limit"
    );
    assert!(ran.seconds < 15, "{}", ran.seconds);
    // The collector wins the race almost always; when the host read the status first it is the hook's own.
    assert!(
        ran.unreadable || ran.exit == 2,
        "unreadable {} exit {}",
        ran.unreadable,
        ran.exit
    );
}
