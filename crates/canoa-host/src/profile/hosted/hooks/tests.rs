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
        offer(
            profiles,
            &config.profiles,
            None,
            "launcher".into(),
            Vec::new(),
        )
        .expect("an offer"),
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
    known.insert("cwd".to_owned(), json!(std::env::temp_dir()));
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

/// A `pre-tool` event for a call of `kind` with `input`, in the folder `cwd`, as the host answers one over ACP.
fn pre_tool(connection: &Connection, kind: &str, input: Value, cwd: &Path) -> aos::FiredEvent {
    let mut known = Map::new();
    known.insert("harness".to_owned(), json!("claude-code"));
    known.insert("cwd".to_owned(), json!(cwd));
    known.insert("session".to_owned(), json!("canoa-session-1"));
    known.insert(
        "tool".to_owned(),
        json!({"kind": kind, "name": "Tool", "input": input}),
    );
    connection.hooks.fire(
        &connection.overlays,
        &Firing {
            event: HookEvent::PreTool,
            known: &known,
            select: Some(kind),
            person: &BTreeSet::new(),
            applies: &events::PRE_TOOL,
            pre_env: &connection.pre_env.runs,
            context_used: 0,
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
fn a_hook_is_given_its_event_its_roots_copy_and_the_hosts_environment() {
    let temp = Temp::new();
    let outside = std::env::vars().find(|(name, value)| {
        !value.trim().is_empty()
            && value
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
            && name.chars().all(|c| c.is_ascii_alphabetic() || c == '_')
            && !name.eq_ignore_ascii_case("PATH")
    });
    let Some((outside, value)) = outside else {
        return;
    };
    let digest = temp.install("", &[("pre-env", Arm::LogEnv(outside))]);
    connect(&temp, Some(&digest), &real).expect("prepared");
    let raw = std::fs::read_to_string(temp.log()).expect("logged");
    let expected = format!("envprobe={}", value.trim());
    assert!(raw.lines().any(|line| line.trim() == expected), "{raw}");

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
        Path::new(root).starts_with(super::super::private::base())
            && !Path::new(root).starts_with(&temp.0),
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
            "output",
            Arm::Print(r#"{"decision":"allow"}"#.to_owned()),
            "prompt = { on_failure = \"refuse\" }",
        ),
        (
            "output",
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
fn a_run_that_goes_over_a_bound_is_stopped() {
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
fn a_process_a_hook_leaves_behind_and_that_holds_its_output_does_not_hold_the_relay() {
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
        root.len() == 1 && Path::new(&root[0]).starts_with(super::super::private::base()),
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
fn a_hook_gets_what_pre_env_printed_and_what_another_overlay_declared() {
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
        ("OTHER_VAR", "theirs"),
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
            assert_eq!(seen[0], expected, "another overlay's declaration is given");
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

/// What the script logged of the directory it ran in, one entry per run.
fn cwds(temp: &Temp) -> Vec<String> {
    std::fs::read_to_string(temp.log())
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.trim().strip_prefix("cwd="))
        .map(str::to_owned)
        .collect()
}

#[test]
fn a_hook_of_a_later_overlay_reads_a_variable_an_earlier_overlay_declared_and_the_reverse() {
    // Overlays stack: every hook runs on the finished stack, whichever overlay declared the variable and whichever order
    // the profile lists them in.
    for (hook_priority, other_priority) in [(600, 100), (100, 600)] {
        let temp = Temp::new();
        temp.install_as(
            "first",
            &format!("priority = {other_priority}\n[env]\nFIRST_VAR = \"one\"\n"),
            &[],
            &[],
        );
        let digest = temp.install_as(
            "second",
            &format!("priority = {hook_priority}\n[hooks]\nprompt = {{}}\n"),
            &[("prompt", Arm::LogEnv("FIRST_VAR".to_owned()))],
            &[],
        );
        let connection =
            connect_overlays(&temp, &["first", "second"], &[("second", &digest)], &real)
                .expect("prepared");
        prompt(&connection, "hello");
        assert_eq!(
            probed(&temp),
            ["one"],
            "priorities {hook_priority}/{other_priority}"
        );
    }
}

#[test]
fn a_pre_env_hook_reads_what_an_earlier_overlays_pre_env_hook_set_and_not_a_later_ones() {
    let temp = Temp::new();
    let first = temp.install_as(
        "first",
        "priority = 100\n[hooks]\npre-env = {}\n",
        &[(
            "pre-env",
            Arm::Print(r#"{"FIRST_PRE":"from-first"}"#.to_owned()),
        )],
        &[],
    );
    let second = temp.install_as(
        "second",
        "priority = 600\n[hooks]\npre-env = {}\n",
        &[("pre-env", Arm::LogEnv("FIRST_PRE".to_owned()))],
        &[],
    );
    connect_overlays(
        &temp,
        &["second", "first"],
        &[("first", &first), ("second", &second)],
        &real,
    )
    .expect("prepared");
    // The earlier overlay's own run read nothing of the later one's, and the later one's read the earlier one's.
    assert_eq!(probed(&temp), ["from-first"]);
}

#[test]
fn a_hook_runs_in_the_sessions_folder_not_its_overlays() {
    let temp = Temp::new();
    let digest = temp.install("[hooks]\nprompt = {}\n", &[("prompt", Arm::LogCwd)]);
    let connection = connect(&temp, Some(&digest), &real).expect("prepared");
    prompt(&connection, "hello");
    let ran_in = cwds(&temp);
    assert_eq!(ran_in.len(), 1, "{ran_in:?}");
    assert_eq!(
        Path::new(&ran_in[0]).canonicalize().expect("a folder"),
        std::env::temp_dir().canonicalize().expect("a folder"),
        "{ran_in:?}"
    );
}

#[test]
fn a_hook_of_the_event_reads_what_an_earlier_hook_of_it_set() {
    let temp = Temp::new();
    let first = temp.install_as(
        "first",
        "priority = 100\n[hooks]\npre-tool = { sets = [\"SET_BY_FIRST\"] }\n",
        &[(
            "pre-tool",
            Arm::Print(r#"{"env":{"SET_BY_FIRST":"yes"}}"#.to_owned()),
        )],
        &[],
    );
    let second = temp.install_as(
        "second",
        "priority = 600\n[hooks]\npre-tool = {}\n",
        &[("pre-tool", Arm::LogEnv("SET_BY_FIRST".to_owned()))],
        &[],
    );
    let connection = connect_overlays(
        &temp,
        &["first", "second"],
        &[("first", &first), ("second", &second)],
        &real,
    )
    .expect("prepared");
    pre_tool(
        &connection,
        "execute",
        json!({"command": "ls"}),
        &std::env::temp_dir(),
    );
    assert_eq!(probed(&temp), ["yes"]);
}

/// Two overlays' `pre-tool` hooks, the first printing `first` and the second `second` (nothing for an empty text), and the
/// event they fire for an edit.
fn two_pre_tool_hooks(first: &str, second: &str) -> (Temp, aos::FiredEvent) {
    let temp = Temp::new();
    let mut allow = Vec::new();
    for (name, priority, output) in [("first", 100, first), ("second", 600, second)] {
        let arms = if output.is_empty() {
            vec![]
        } else {
            vec![("pre-tool", Arm::Print(output.to_owned()))]
        };
        let digest = temp.install_as(
            name,
            &format!("priority = {priority}\n[hooks]\npre-tool = {{}}\n"),
            &arms,
            &[],
        );
        allow.push((name, digest));
    }
    let allow: Vec<(&str, &str)> = allow
        .iter()
        .map(|(name, digest)| (*name, digest.as_str()))
        .collect();
    let connection =
        connect_overlays(&temp, &["first", "second"], &allow, &real).expect("prepared");
    let fired = pre_tool(
        &connection,
        "edit",
        json!({"file_path": "a.txt"}),
        &std::env::temp_dir(),
    );
    (temp, fired)
}

const ALLOW: &str = r#"{"decision":"allow","reason":"fine"}"#;
const DENY: &str = r#"{"decision":"deny","reason":"no"}"#;
const ASK: &str = r#"{"decision":"ask","reason":"look"}"#;

#[test]
fn hooks_combine_to_the_tightest_a_deny_beats_an_ask_and_an_ask_beats_an_allow() {
    for (first, second, expected) in [
        (ALLOW, "", Some(aos::Decision::Allow)),
        (ALLOW, ALLOW, Some(aos::Decision::Allow)),
        (ALLOW, DENY, Some(aos::Decision::Deny)),
        (DENY, ALLOW, Some(aos::Decision::Deny)),
        (ALLOW, ASK, Some(aos::Decision::Ask)),
        (ASK, ALLOW, Some(aos::Decision::Ask)),
        (ASK, DENY, Some(aos::Decision::Deny)),
        ("", "", None),
    ] {
        let (_temp, fired) = two_pre_tool_hooks(first, second);
        assert_eq!(fired.decision, expected, "{first} then {second}");
    }
}

#[test]
fn a_deny_ends_the_event_and_an_allow_does_not() {
    let (temp, fired) = two_pre_tool_hooks(DENY, ALLOW);
    assert_eq!(fired.ran, ["first"], "{:?}", fired.ran);
    drop(temp);
    let (_temp, fired) = two_pre_tool_hooks(ALLOW, ASK);
    assert_eq!(
        fired.ran,
        ["first", "second"],
        "an allow lets the next hook decide"
    );
}

#[test]
fn an_allow_that_brings_an_input_is_not_applied_over_acp_and_is_reported() {
    // ACP's answer to a permission request names an option and carries no input, so the call cannot run as rewritten.
    let (_temp, fired) =
        two_pre_tool_hooks(r#"{"decision":"allow","input":{"file_path":"b.txt"}}"#, "");
    assert_eq!(fired.decision, None, "the allow is not applied either");
    assert!(
        fired
            .reports
            .iter()
            .any(|report| report.id == "hook-not-applied"
                && report.about == ["first", "pre-tool", "input"]),
        "{:?}",
        fired.reports
    );
    assert_eq!(fired.input, None);
}

#[test]
fn a_rewrite_an_ask_beats_is_not_applied() {
    let (_temp, fired) =
        two_pre_tool_hooks(r#"{"decision":"allow","input":{"file_path":"b.txt"}}"#, ASK);
    assert_eq!(fired.decision, Some(aos::Decision::Ask));
    assert_eq!(fired.input, None, "the call is asked about as it was given");
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
fn what_a_killed_host_left_in_the_temporary_directory_is_removed_when_old_and_only_that() {
    let temp = Temp::new();
    let me = std::process::id();
    let names = [
        "canoa-hooks-old",
        "canoa-hooks-new",
        "canoa-profile-old",
        "canoa-profile-4194999-ab",
        &format!("canoa-profile-{me}-ab"),
        "other-old",
    ];
    for name in names {
        std::fs::create_dir_all(temp.0.join(name)).expect("a directory");
    }
    let old = std::time::SystemTime::now() - Duration::from_secs(48 * 3600);
    let live = format!("canoa-profile-{me}-ab");
    for name in names
        .into_iter()
        .filter(|name| *name != "canoa-hooks-new" && *name != live)
    {
        age(&temp.0.join(name), old);
    }
    age(
        &temp.0.join(&live),
        std::time::SystemTime::now() - Duration::from_secs(2 * 3600),
    );
    // Dead, but not old: a host in another process namespace has no process here either.
    #[cfg(unix)]
    std::fs::create_dir_all(temp.0.join("canoa-profile-4194998-fresh")).expect("a directory");
    remove_stale(&temp.0);
    assert!(!temp.0.join("canoa-hooks-old").exists());
    assert!(
        !temp.0.join("canoa-profile-old").exists(),
        "plugins and skills too"
    );
    assert!(!temp.0.join("canoa-profile-4194999-ab").exists());
    assert!(temp.0.join("canoa-hooks-new").exists());
    assert!(
        temp.0.join("other-old").exists(),
        "only a host's own are removed"
    );
    // A host that runs is not swept: by its process id where the system can say, by the file it holds open on Windows.
    #[cfg(unix)]
    {
        assert!(temp.0.join(&live).exists());
        assert!(temp.0.join("canoa-profile-4194998-fresh").exists());
    }
}

#[test]
fn the_directory_of_a_host_that_runs_is_not_swept_when_it_is_old_but_not_a_day_old() {
    let temp = Temp::new();
    let live = super::super::private::Private::under(&temp.0, "canoa-profile");
    live.ensure().expect("made");
    std::fs::write(live.path().join("channel.json"), b"x").expect("written");
    age(
        live.path(),
        std::time::SystemTime::now() - Duration::from_secs(3 * 3600),
    );
    remove_stale(&temp.0);
    assert!(
        live.path().join("channel.json").exists(),
        "a live host's files stay"
    );
    let path = live.path().to_owned();
    drop(live);
    assert!(!path.exists());
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

/// The harness's own hooks, through a host whose agent is a stand-in that only answers `initialize` and session requests:
/// what the host writes for the session, what it does with a report from the forwarder, and what the forwarder is told.
mod native {
    use alloc::collections::VecDeque;
    use std::io::Cursor;
    use std::sync::Mutex;
    use std::time::Instant;

    use coqui_transport::{Channel, Failure};

    use super::*;
    use crate::profile::hosted::Hosted;
    use crate::scope::Scope;

    /// The client's end: what it sends, and what the host sent it.
    #[derive(Default)]
    struct Client {
        from: Mutex<VecDeque<Vec<u8>>>,
        to: Mutex<Vec<Vec<u8>>>,
    }

    impl Channel for Client {
        fn send(&self, message: &[u8]) -> Result<(), Failure> {
            self.to.lock().expect("lock").push(message.to_vec());
            Ok(())
        }

        fn receive(&self, _timeout: Option<Duration>) -> Result<Option<Vec<u8>>, Failure> {
            Ok(self.from.lock().expect("lock").pop_front())
        }

        fn close(&self) {}
    }

    /// A session of a host, open on a stand-in agent.
    struct Session<'a> {
        hosted: Hosted<'a, Client>,
        client: &'a Client,
        next: u64,
        /// The plugin directory the session request carried, when it carried one.
        plugin: Option<PathBuf>,
        id: String,
    }

    fn send(client: &Client, id: u64, method: &str, params: Value) {
        let request = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        client
            .from
            .lock()
            .expect("lock")
            .push_back(serde_json::to_vec(&request).expect("JSON"));
    }

    fn session<'a>(
        hosted: Hosted<'a, Client>,
        client: &'a Client,
        harness: &str,
        cwd: &str,
    ) -> Session<'a> {
        send(
            client,
            1,
            "initialize",
            json!({"protocolVersion": 1, "clientCapabilities": {}}),
        );
        hosted
            .receive(Some(Duration::from_millis(10)))
            .expect("read");
        let answer = json!({"jsonrpc": "2.0", "id": 1, "result": {"protocolVersion": 1,
            "agentCapabilities": {}, "agentInfo": {"name": harness}}});
        hosted
            .send(&serde_json::to_vec(&answer).expect("JSON"))
            .expect("sent");
        send(
            client,
            2,
            "session/new",
            json!({"cwd": cwd, "mcpServers": []}),
        );
        let request = hosted
            .receive(Some(Duration::from_millis(10)))
            .expect("read")
            .expect("the request goes on");
        let request: Value = serde_json::from_slice(&request).expect("JSON");
        let plugin = request
            .pointer("/params/_meta/claudeCode/options/plugins")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|plugin| plugin["path"].as_str())
            .map(PathBuf::from)
            .find(|path| path.join("hooks").join("hooks.json").exists());
        let answer = json!({"jsonrpc": "2.0", "id": 2, "result": {"sessionId": "s1"}});
        hosted
            .send(&serde_json::to_vec(&answer).expect("JSON"))
            .expect("sent");
        Session {
            hosted,
            client,
            next: 3,
            plugin,
            id: "s1".to_owned(),
        }
    }

    impl Session<'_> {
        /// The plugin's hooks.json.
        fn hooks(&self) -> Value {
            let plugin = self.plugin.as_ref().expect("a plugin");
            serde_json::from_slice(
                &std::fs::read(plugin.join("hooks").join("hooks.json")).expect("read"),
            )
            .expect("JSON")
        }

        fn channel(&self) -> PathBuf {
            self.plugin.as_ref().expect("a plugin").join("channel.json")
        }

        /// What the harness's hook does: the forwarder with `payload` as its input, while the host reads between messages.
        fn report(&self, kind: &'static str, payload: Value) -> Result<bool, String> {
            let channel = self.channel();
            let run = std::thread::spawn(move || {
                forward(
                    kind,
                    &channel,
                    false,
                    Cursor::new(payload.to_string().into_bytes()),
                )
            });
            let until = Instant::now() + Duration::from_secs(60);
            while !run.is_finished() && Instant::now() < until {
                let _ = self.hosted.receive(Some(Duration::from_millis(10)));
                std::thread::sleep(Duration::from_millis(10));
            }
            run.join().expect("the forwarder ended")
        }

        /// A prompt the client sends: the request that goes to the agent.
        fn prompt(&mut self, text: &str) -> Value {
            let id = self.next;
            self.next += 1;
            send(
                self.client,
                id,
                "session/prompt",
                json!({"sessionId": self.id, "prompt": [{"type": "text", "text": text}]}),
            );
            let request = self
                .hosted
                .receive(Some(Duration::from_millis(10)))
                .expect("read")
                .expect("the request goes on");
            serde_json::from_slice(&request).expect("JSON")
        }
    }

    fn payload_of(temp: &Temp, event: &str) -> Vec<Value> {
        script::logged(&temp.log())
            .into_iter()
            .filter(|(each, _)| each == event)
            .map(|(_, payload)| payload)
            .collect()
    }

    /// A host with overlay `team` allowed for `manifest` and `arms`, and a session on `harness` working in `cwd`, a folder
    /// (with `sub` and `a` in it) made under the test's own directory.
    fn run(
        manifest: &str,
        arms: &[(&str, Arm)],
        harness: &str,
        cwd: &str,
        then: impl FnOnce(&Temp, Session<'_>),
    ) {
        let temp = Temp::new();
        let digest = temp.install(manifest, arms);
        let connection = connect(&temp, Some(&digest), &real).expect("prepared");
        let client = Client::default();
        let scope = Scope::new(Vec::new());
        let hosted = Hosted::in_process(&client, &scope, &connection);
        let dir = temp.0.join(cwd);
        for sub in ["sub", "a"] {
            std::fs::create_dir_all(dir.join(sub)).expect("a folder");
        }
        let opened = session(hosted, &client, harness, &dir.to_string_lossy());
        then(&temp, opened);
    }

    const CLAUDE: &str = "coqui-acp-claude";

    #[test]
    fn claude_is_given_a_plugin_whose_hooks_run_the_forwarder_for_the_events_an_overlay_listens_to()
    {
        run(
            "[hooks]\npre-compact = {}\ncd = {}\n",
            &[],
            CLAUDE,
            "work",
            |_, session| {
                let hooks = session.hooks();
                let mut events: Vec<&str> = hooks["hooks"]
                    .as_object()
                    .expect("an object")
                    .keys()
                    .map(String::as_str)
                    .collect();
                events.sort_unstable();
                assert_eq!(events, ["PostToolBatch", "PreCompact", "SessionStart"]);
                let command = hooks["hooks"]["PreCompact"][0]["hooks"][0]["command"]
                    .as_str()
                    .expect("a command");
                assert!(command.contains("hook-forward --channel"), "{command}");
                assert!(command.ends_with(" pre-compact"), "{command}");
                // The channel is a file of the session's own directory, and nothing a process list shows holds its secret.
                let file: Value =
                    serde_json::from_slice(&std::fs::read(session.channel()).expect("read"))
                        .expect("JSON");
                let secret = file["secret"].as_str().expect("a secret");
                assert!(secret.len() >= 32);
                assert!(!hooks.to_string().contains(secret));
                assert!(
                    file["addr"]
                        .as_str()
                        .is_some_and(|addr| addr.starts_with("127.0.0.1:"))
                );
            },
        );
    }

    #[test]
    fn nothing_is_written_when_no_overlay_listens_or_the_harness_is_not_claude() {
        let none = "[hooks]\nprompt = {}\ncd = false\npre-compact = false\nsession-start = false\n";
        run(none, &[], CLAUDE, "work", |_, session| {
            assert!(
                session.plugin.is_none(),
                "an overlay with no hook for these events gets no plugin"
            );
        });
        for harness in ["coqui-acp-codex", "gemini-cli", "pi-acp"] {
            run(
                "[hooks]\npre-compact = {}\ncd = {}\n",
                &[],
                harness,
                "work",
                |_, session| {
                    assert!(session.plugin.is_none(), "{harness}");
                },
            );
        }
    }

    #[test]
    fn a_compaction_runs_the_hook_with_its_trigger_and_a_block_stops_it() {
        run(
            "[hooks]\npre-compact = {}\n",
            &[(
                "pre-compact",
                Arm::Print(r#"{"decision":"block","reason":"not now"}"#.to_owned()),
            )],
            CLAUDE,
            "work",
            |temp, session| {
                let work = temp.0.join("work");
                let stopped = session.report(
                    "pre-compact",
                    json!({"trigger": "manual", "cwd": work, "session_id": "x"}),
                );
                assert_eq!(stopped, Ok(true));
                let payloads = payload_of(temp, "pre-compact");
                assert_eq!(payloads.len(), 1);
                assert_eq!(payloads[0]["trigger"], "manual");
                assert_eq!(payloads[0]["event"], "pre-compact");
                assert!(
                    payloads[0]["session"]
                        .as_str()
                        .is_some_and(|id| id.starts_with("canoa-session-"))
                );
            },
        );
        run(
            "[hooks]\npre-compact = {}\n",
            &[],
            CLAUDE,
            "work",
            |temp, session| {
                assert_eq!(
                    session.report("pre-compact", json!({"trigger": "auto"})),
                    Ok(false)
                );
                assert_eq!(payload_of(temp, "pre-compact")[0]["trigger"], "auto");
            },
        );
    }

    #[test]
    fn a_guard_on_compaction_that_fails_stops_it() {
        run(
            "[hooks]\npre-compact = { on_failure = \"refuse\" }\n",
            &[("pre-compact", Arm::Exit(3))],
            CLAUDE,
            "work",
            |_, session| {
                assert_eq!(
                    session.report("pre-compact", json!({"trigger": "manual"})),
                    Ok(true)
                );
            },
        );
    }

    #[test]
    fn a_working_directory_that_changed_runs_cd_with_where_it_was_and_the_same_place_does_not() {
        run(
            "[hooks]\ncd = {}\n",
            &[],
            CLAUDE,
            "work",
            |temp, session| {
                let work = temp.0.join("work");
                let sub = work.join("sub");
                let (work, sub) = (work.to_string_lossy(), sub.to_string_lossy());
                // The harness says where it starts, spelled as it spells it: that is where the session is, not a change.
                session
                    .report(
                        "start",
                        json!({"source": "startup", "cwd": format!("{work}{}", std::path::MAIN_SEPARATOR)}),
                    )
                    .expect("answered");
                session
                    .report("batch", json!({"cwd": work}))
                    .expect("answered");
                assert!(payload_of(temp, "cd").is_empty());
                session
                    .report("batch", json!({"cwd": sub}))
                    .expect("answered");
                session
                    .report("batch", json!({"cwd": sub}))
                    .expect("answered");
                session
                    .report("batch", json!({"cwd": work}))
                    .expect("answered");
                let moves = payload_of(temp, "cd");
                assert_eq!(moves.len(), 2, "{moves:?}");
                assert_eq!(
                    (moves[0]["previous"].as_str(), moves[0]["cwd"].as_str()),
                    (Some(&*work), Some(&*sub))
                );
                assert_eq!(
                    (moves[1]["previous"].as_str(), moves[1]["cwd"].as_str()),
                    (Some(&*sub), Some(&*work))
                );
            },
        );
    }

    #[test]
    fn a_session_start_after_compaction_runs_its_hooks_and_its_context_goes_in_the_next_prompt() {
        run(
            "[hooks]\nsession-start = { match = [\"compact\"] }\n",
            &[(
                "session-start",
                Arm::Print(r#"{"context":"AFTER-COMPACT"}"#.to_owned()),
            )],
            CLAUDE,
            "work",
            |temp, mut session| {
                let work = temp.0.join("work");
                // A start the host opened itself is not run again from the harness's own report.
                session
                    .report("start", json!({"source": "startup", "cwd": work}))
                    .expect("answered");
                assert!(payload_of(temp, "session-start").is_empty());
                session
                    .report("start", json!({"source": "compact", "cwd": work}))
                    .expect("answered");
                let starts = payload_of(temp, "session-start");
                assert_eq!(starts.len(), 1);
                assert_eq!(starts[0]["reason"], "compact");
                let prompt = session.prompt("go on");
                assert!(prompt.to_string().contains("AFTER-COMPACT"), "{prompt}");
            },
        );
    }

    #[test]
    fn a_clear_cannot_be_reached_and_the_report_says_so_for_claude_and_every_cannot_for_codex() {
        let temp = Temp::new();
        let digest = temp.install(
            "[hooks]\nsession-start = { match = [\"clear\"] }\ncd = {}\npre-compact = {}\n",
            &[],
        );
        let connection = connect(&temp, Some(&digest), &real).expect("prepared");
        let say = |harness: &str, key: &str| {
            connection
                .hooks
                .items(&BTreeSet::new(), Some(harness))
                .into_iter()
                .map(|item| item.json())
                .find(|item| item["key"] == key)
                .map(|item| (item["state"].to_string(), item["how"].to_string()))
                .expect("an item")
        };
        let (state, how) = say("claude-code", "hooks.team.session-start");
        assert!(
            state.contains("not supported") && how.contains("/clear"),
            "{state} {how}"
        );
        assert!(say("claude-code", "hooks.team.cd").0.contains("applied"));
        assert!(
            say("claude-code", "hooks.team.pre-compact")
                .0
                .contains("applied")
        );
        for key in [
            "hooks.team.cd",
            "hooks.team.pre-compact",
            "hooks.team.session-start",
        ] {
            let (state, how) = say("codex", key);
            assert!(
                state.contains("not supported") && how.contains("Codex") || how.contains("harness"),
                "{key}: {state} {how}"
            );
        }
    }

    #[test]
    fn a_session_that_ended_is_not_heard() {
        run(
            "[hooks]\ncd = {}\n",
            &[],
            CLAUDE,
            "work",
            |temp, session| {
                let channel = session.channel();
                session
                    .report("batch", json!({"cwd": temp.0.join("work").join("a")}))
                    .expect("answered");
                assert_eq!(payload_of(temp, "cd").len(), 1);
                send(
                    session.client,
                    9,
                    "session/close",
                    json!({"sessionId": "s1"}),
                );
                session
                    .hosted
                    .receive(Some(Duration::from_millis(10)))
                    .expect("read");
                // The secret ended with the session: the forwarder is not answered, and nothing runs.
                let started = Instant::now();
                let heard = forward(
                    "batch",
                    &channel,
                    false,
                    Cursor::new(br#"{"cwd":"/elsewhere"}"#.to_vec()),
                );
                assert_eq!(heard, Ok(false));
                assert!(started.elapsed() < Duration::from_secs(10));
                assert_eq!(payload_of(temp, "cd").len(), 1);
            },
        );
    }

    /// A session request as far as the agent: what the host sends on, and the plugin it carries.
    fn begin<'a>(
        hosted: &Hosted<'a, Client>,
        client: &Client,
        harness: &str,
    ) -> (Value, Option<PathBuf>) {
        send(
            client,
            1,
            "initialize",
            json!({"protocolVersion": 1, "clientCapabilities": {}}),
        );
        hosted
            .receive(Some(Duration::from_millis(10)))
            .expect("read");
        let answer = json!({"jsonrpc": "2.0", "id": 1, "result": {"protocolVersion": 1,
            "agentCapabilities": {}, "agentInfo": {"name": harness}}});
        hosted
            .send(&serde_json::to_vec(&answer).expect("JSON"))
            .expect("sent");
        send(
            client,
            2,
            "session/new",
            json!({"cwd": "/work", "mcpServers": []}),
        );
        // A request the host refuses is read as a timeout, as the relay reads it.
        let request = hosted
            .receive(Some(Duration::from_millis(10)))
            .ok()
            .flatten();
        let request: Value = request.map_or(Value::Null, |bytes| {
            serde_json::from_slice(&bytes).expect("JSON")
        });
        let plugin = request
            .pointer("/params/_meta/claudeCode/options/plugins")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|plugin| plugin["path"].as_str())
            .map(PathBuf::from)
            .find(|path| path.join("hooks").join("hooks.json").exists());
        (request, plugin)
    }

    /// What the client was last told.
    fn last_to(client: &Client) -> Value {
        let to = client.to.lock().expect("lock");
        serde_json::from_slice(to.last().expect("a message")).expect("JSON")
    }

    #[test]
    fn a_connection_directory_someone_made_first_is_never_used_and_a_guard_refuses_the_session() {
        for (manifest, guarded) in [
            ("[hooks]\ncd = {}\n", false),
            ("[hooks]\npre-compact = { on_failure = \"refuse\" }\n", true),
        ] {
            let temp = Temp::new();
            let digest = temp.install(manifest, &[]);
            let connection = connect(&temp, Some(&digest), &real).expect("prepared");
            // Another user made the path first (it could not be guessed, but it could be read from a listing).
            std::fs::create_dir_all(connection.skills()).expect("made first");
            let client = Client::default();
            let scope = Scope::new(Vec::new());
            let hosted = Hosted::in_process(&client, &scope, &connection);
            let (request, plugin) = begin(&hosted, &client, CLAUDE);
            assert!(
                plugin.is_none(),
                "nothing is written under a directory that was not made here"
            );
            assert!(
                std::fs::read_dir(connection.skills())
                    .expect("read")
                    .next()
                    .is_none(),
                "nothing was written into it"
            );
            if guarded {
                assert_eq!(request, Value::Null, "the session request goes no further");
                assert_eq!(
                    last_to(&client)["error"]["code"],
                    -32000,
                    "{}",
                    last_to(&client)
                );
            } else {
                let answer = json!({"jsonrpc": "2.0", "id": 2, "result": {"sessionId": "s1"}});
                hosted
                    .send(&serde_json::to_vec(&answer).expect("JSON"))
                    .expect("sent");
                let told = last_to(&client);
                let cd = told["result"]["_meta"]["dev.boriken.canoa/profile"]["items"]
                    .as_array()
                    .and_then(|items| items.iter().find(|item| item["key"] == "hooks.team.cd"))
                    .expect("cd is reported")
                    .clone();
                assert_eq!(cd["state"], "refused", "{cd}");
                assert!(
                    cd["how"]
                        .as_str()
                        .is_some_and(|how| how.contains("hook-failed")),
                    "{cd}"
                );
            }
        }
    }

    #[test]
    fn a_guard_that_needs_a_plugin_the_host_cannot_write_refuses_for_a_path_it_cannot_quote() {
        let temp = Temp::new();
        let digest = temp.install("[hooks]\npre-compact = { on_failure = \"refuse\" }\n", &[]);
        let mut connection = connect(&temp, Some(&digest), &real).expect("prepared");
        connection.launcher = "/opt/$HOME/canoa".into();
        let client = Client::default();
        let scope = Scope::new(Vec::new());
        let hosted = Hosted::in_process(&client, &scope, &connection);
        let (request, plugin) = begin(&hosted, &client, CLAUDE);
        assert!(plugin.is_none());
        assert_eq!(request, Value::Null);
        assert_eq!(last_to(&client)["error"]["code"], -32000);
    }

    #[test]
    fn a_request_the_agent_answers_with_an_error_ends_its_channel_and_its_plugin() {
        let temp = Temp::new();
        let digest = temp.install("[hooks]\ncd = {}\n", &[]);
        let connection = connect(&temp, Some(&digest), &real).expect("prepared");
        let client = Client::default();
        let scope = Scope::new(Vec::new());
        let hosted = Hosted::in_process(&client, &scope, &connection);
        let (_, plugin) = begin(&hosted, &client, CLAUDE);
        let plugin = plugin.expect("a plugin");
        let copy = temp.0.join("channel-copy.json");
        std::fs::copy(plugin.join("channel.json"), &copy).expect("copied");
        let failed = json!({"jsonrpc": "2.0", "id": 2, "error": {"code": -32000, "message": "no"}});
        hosted
            .send(&serde_json::to_vec(&failed).expect("JSON"))
            .expect("sent");
        assert!(!plugin.exists(), "the plugin went with the request");
        // Its secret is not good any more: the forwarder is not answered.
        assert!(forward("batch", &copy, true, Cursor::new(Vec::new())).is_err());
    }

    #[test]
    fn a_request_that_is_answered_keeps_its_plugin_until_the_session_ends() {
        run("[hooks]\ncd = {}\n", &[], CLAUDE, "work", |_, session| {
            let plugin = session.plugin.clone().expect("a plugin");
            assert!(plugin.exists());
            send(
                session.client,
                9,
                "session/close",
                json!({"sessionId": "s1"}),
            );
            session
                .hosted
                .receive(Some(Duration::from_millis(10)))
                .expect("read");
            assert!(!plugin.exists());
        });
    }

    #[test]
    fn the_connection_directory_is_made_once_private_and_goes_with_the_connection() {
        let temp = Temp::new();
        let digest = temp.install("[hooks]\ncd = {}\n", &[]);
        let connection = connect(&temp, Some(&digest), &real).expect("prepared");
        let dir = connection.skills().to_owned();
        assert!(!dir.exists(), "nothing is made until something is written");
        assert_eq!(connection.private().expect("made"), dir);
        assert_eq!(connection.private().expect("again"), dir);
        let client = Client::default();
        let scope = Scope::new(Vec::new());
        {
            let hosted = Hosted::in_process(&client, &scope, &connection);
            let (_, plugin) = begin(&hosted, &client, CLAUDE);
            assert!(plugin.expect("a plugin").starts_with(&dir));
            let answer = json!({"jsonrpc": "2.0", "id": 2, "result": {"sessionId": "s1"}});
            hosted
                .send(&serde_json::to_vec(&answer).expect("JSON"))
                .expect("sent");
        }
        assert!(
            dir.exists(),
            "a session's end does not take the connection's directory"
        );
        drop(connection);
        assert!(!dir.exists());
    }

    #[test]
    fn a_session_loaded_again_is_given_the_plugin_and_secret_it_has_and_no_new_ones() {
        let temp = Temp::new();
        let digest = temp.install("[hooks]\ncd = {}\n", &[]);
        let connection = connect(&temp, Some(&digest), &real).expect("prepared");
        let client = Client::default();
        let scope = Scope::new(Vec::new());
        let hosted = Hosted::in_process(&client, &scope, &connection);
        let (_, first) = begin(&hosted, &client, CLAUDE);
        let first = first.expect("a plugin");
        let answer = json!({"jsonrpc": "2.0", "id": 2, "result": {"sessionId": "s1"}});
        hosted
            .send(&serde_json::to_vec(&answer).expect("JSON"))
            .expect("sent");
        send(
            &client,
            3,
            "session/load",
            json!({"sessionId": "s1", "cwd": "/work", "mcpServers": []}),
        );
        let request = hosted
            .receive(Some(Duration::from_millis(10)))
            .ok()
            .flatten()
            .expect("the request goes on");
        let request: Value = serde_json::from_slice(&request).expect("JSON");
        let plugins: Vec<&str> = request["params"]["_meta"]["claudeCode"]["options"]["plugins"]
            .as_array()
            .expect("plugins")
            .iter()
            .filter_map(|plugin| plugin["path"].as_str())
            .collect();
        assert_eq!(plugins, [first.to_string_lossy()], "the plugin it has");
        assert_eq!(
            dirs_in(connection.skills()),
            1,
            "no second plugin was written"
        );
        // An error for the load does not end the session's own channel.
        let failed = json!({"jsonrpc": "2.0", "id": 3, "error": {"code": -32000, "message": "no"}});
        hosted
            .send(&serde_json::to_vec(&failed).expect("JSON"))
            .expect("sent");
        assert!(first.exists());
        assert!(heard(&hosted, &first), "its secret is still good");
    }

    /// Whether the host answers a forwarder with the secret in the channel file of the plugin at `plugin`.
    fn heard(hosted: &Hosted<'_, Client>, plugin: &Path) -> bool {
        let channel = plugin.join("channel.json");
        let run =
            std::thread::spawn(move || forward("batch", &channel, true, Cursor::new(Vec::new())));
        let until = Instant::now() + Duration::from_secs(20);
        while !run.is_finished() && Instant::now() < until {
            let _ = hosted.receive(Some(Duration::from_millis(10)));
            std::thread::sleep(Duration::from_millis(10));
        }
        run.join().expect("ran").is_ok()
    }

    /// The directories directly in `dir`.
    fn dirs_in(dir: &Path) -> usize {
        std::fs::read_dir(dir)
            .expect("read")
            .flatten()
            .filter(|entry| entry.path().is_dir())
            .count()
    }

    #[test]
    fn a_request_the_host_stops_before_the_agent_has_it_ends_its_channel_and_plugin() {
        let temp = Temp::new();
        let digest = temp.install("[hooks]\ncd = {}\n", &[]);
        let connection = connect(&temp, Some(&digest), &real).expect("prepared");
        let client = Client::default();
        let scope = Scope::new(Vec::new());
        let hosted = Hosted::in_process(&client, &scope, &connection);
        let (request, plugin) = begin(&hosted, &client, CLAUDE);
        let plugin = plugin.expect("a plugin");
        let copy = temp.0.join("channel-copy.json");
        std::fs::copy(plugin.join("channel.json"), &copy).expect("copied");
        // What the host does with a request it stops after the hooks said yes (a part that cannot be added, the usage
        // splice answering it itself).
        hosted.abandon_opening(&serde_json::to_vec(&request).expect("JSON"));
        assert_eq!(
            dirs_in(connection.skills()),
            0,
            "the plugin written for it is gone"
        );
        assert!(
            forward("batch", &copy, true, Cursor::new(Vec::new())).is_err(),
            "its secret is not good"
        );
    }

    const ALLOW_ALL: &str = r#"{"decision":"allow"}"#;

    /// What the host answers a `session/request_permission` with `params` when one overlay's `pre-tool` hook allows every
    /// call: the option it selected, or none when the question went on to the client.
    fn allowed_option(params: Value) -> Option<String> {
        let temp = Temp::new();
        let digest = temp.install(
            "[hooks]\npre-tool = {}\n",
            &[("pre-tool", Arm::Print(ALLOW_ALL.to_owned()))],
        );
        let connection = connect(&temp, Some(&digest), &real).expect("prepared");
        let client = Client::default();
        let scope = Scope::new(Vec::new());
        let hosted = Hosted::in_process(&client, &scope, &connection);
        let message = json!({"jsonrpc": "2.0", "id": 7, "method": "session/request_permission", "params": params});
        match hosted.on_agent(&serde_json::to_vec(&message).expect("JSON")) {
            Seen::Dropped => {
                let sent = hosted.hold().book.next_injected()?;
                let sent: Value = serde_json::from_slice(&sent).expect("JSON");
                sent["result"]["outcome"]["optionId"]
                    .as_str()
                    .map(str::to_owned)
            }
            Seen::Passed(_) => None,
        }
    }

    fn options() -> Value {
        json!([
            {"optionId": "yes", "name": "Allow", "kind": "allow_once"},
            {"optionId": "yes-always", "name": "Always", "kind": "allow_always"},
            {"optionId": "no", "name": "Reject", "kind": "reject_once"}
        ])
    }

    fn edit_of(path: &str) -> Value {
        json!({"toolCallId": "c1", "title": "Edit", "kind": "edit", "rawInput": {"file_path": path}})
    }

    #[test]
    fn an_allow_answers_a_call_asked_over_v1_and_over_v2_alike() {
        let v1 = json!({"sessionId": "s", "toolCall": edit_of("src/lib.rs"), "options": options()});
        let v2 = json!({"sessionId": "s", "subject": {"toolCall": edit_of("src/lib.rs")}, "options": options()});
        assert_eq!(allowed_option(v1).as_deref(), Some("yes"));
        assert_eq!(allowed_option(v2).as_deref(), Some("yes"));
    }

    #[test]
    fn a_protected_path_is_asked_about_over_v2_as_over_v1() {
        let v1 = json!({"sessionId": "s", "toolCall": edit_of(".env"), "options": options()});
        let v2 = json!({"sessionId": "s", "subject": {"toolCall": edit_of(".env")}, "options": options()});
        assert_eq!(allowed_option(v1), None);
        assert_eq!(allowed_option(v2), None);
    }

    #[test]
    fn a_question_with_no_tool_call_in_it_is_never_allowed() {
        let command =
            json!({"sessionId": "s", "subject": {"command": "rm -rf ."}, "options": options()});
        let unknown = json!({"sessionId": "s", "subject": {"toolCall": "x"}, "options": options()});
        let none = json!({"sessionId": "s", "options": options()});
        for params in [command, unknown, none] {
            assert_eq!(allowed_option(params.clone()), None, "{params}");
        }
    }

    #[test]
    fn a_request_to_widen_the_sandbox_is_never_answered_by_an_allow() {
        let grant = json!({"toolCallId": "c1", "title": "Grant more permissions", "kind": "other",
            "rawInput": {"fileSystem": {"write": ["/home/me"], "entries": [{"path": "/home/me"}]}}});
        let v1 = json!({"sessionId": "s", "toolCall": grant, "options": options()});
        let v2 = json!({"sessionId": "s", "subject": {"toolCall": grant}, "options": options()});
        assert_eq!(allowed_option(v1), None);
        assert_eq!(allowed_option(v2), None);
    }

    #[test]
    fn an_allow_once_option_whose_id_another_option_shares_is_not_taken() {
        let shared = json!([
            {"optionId": "x", "name": "Allow", "kind": "allow_once"},
            {"optionId": "x", "name": "Reject", "kind": "reject_once"}
        ]);
        let params =
            json!({"sessionId": "s", "toolCall": edit_of("src/lib.rs"), "options": shared});
        assert_eq!(allowed_option(params), None);
    }
}
