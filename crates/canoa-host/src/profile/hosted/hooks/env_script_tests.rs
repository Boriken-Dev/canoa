//! An overlay's env scripts through a real connection: they run only at the allowed digest, under their interpreter and time
//! limit, in the order of application, and what they print joins the environment the agent is given.

use core::time::Duration;
use std::path::Path;
use std::time::Instant;

use serde_json::json;

use super::super::{Connection, Item, State, Takes, parts_for};
use super::tests::script::{self, Arm};
use super::tests::{Temp, connect_overlays, env_of, probed, prompt, real};
use super::*;

/// The default env script's name on this platform: PowerShell on Windows and a shell elsewhere, the interpreters every
/// machine has.
const fn env_file() -> &'static str {
    if cfg!(windows) { "env.ps1" } else { "env.sh" }
}

/// The interpreter's AOS name for the platform's script.
const fn interpreter() -> &'static str {
    if cfg!(windows) { "powershell" } else { "sh" }
}

/// A script that appends a line to `log` and prints `output`.
fn printing(log: &Path, output: &str) -> String {
    let log = log.to_string_lossy();
    if cfg!(windows) {
        format!(
            "Add-Content -LiteralPath '{}' -Value 'ran'\n[Console]::Out.Write('{}')\n",
            log.replace('\'', "''"),
            output.replace('\'', "''")
        )
    } else {
        format!(
            "printf 'ran\\n' >> '{}'\nprintf '%s' '{}'\n",
            log.replace('\'', "'\\''"),
            output.replace('\'', "'\\''")
        )
    }
}

/// A script that prints one variable it was given, as `{"<to>": "<value>"}`, and `own` besides: `(name, value)` pairs.
fn reading(from: &str, to: &str, own: &[(&str, &str)]) -> String {
    let more: String = own
        .iter()
        .map(|(name, value)| format!(",\"{name}\":\"{value}\""))
        .collect();
    if cfg!(windows) {
        format!(
            "[Console]::Out.Write('{{\"{to}\":\"' + $env:{from} + '\"{}}}')\n",
            more.replace('\'', "''")
        )
    } else {
        format!(
            "printf '{{\"{to}\":\"%s\"{}}}' \"${from}\"\n",
            more.replace('%', "%%")
        )
    }
}

/// A script that exits with `status` after writing `words` to standard error.
fn failing(words: &str, status: i32) -> String {
    if cfg!(windows) {
        format!("[Console]::Error.Write('{words}')\nexit {status}\n")
    } else {
        format!("printf '%s' '{words}' >&2\nexit {status}\n")
    }
}

/// A script that waits `seconds`.
fn waiting(seconds: u64) -> String {
    if cfg!(windows) {
        format!("Start-Sleep -Seconds {seconds}\n")
    } else {
        format!("sleep {seconds}\n")
    }
}

impl Temp {
    /// Installs overlay `name` with the files given and no hook, and returns its digest.
    fn install_files(&self, name: &str, manifest: &str, files: &[(&str, String)]) -> String {
        let root = self.0.join("overlays").join(name);
        std::fs::create_dir_all(&root).expect("a directory");
        std::fs::write(root.join("overlay.cfg"), manifest).expect("written");
        for (path, text) in files {
            let file = root.join(path);
            std::fs::create_dir_all(file.parent().expect("a parent")).expect("a directory");
            std::fs::write(file, text).expect("written");
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

    fn runs(&self) -> usize {
        std::fs::read_to_string(self.log())
            .unwrap_or_default()
            .lines()
            .filter(|line| line.trim() == "ran")
            .count()
    }
}

fn note<'a>(connection: &'a Connection, key: &str) -> Option<&'a Item> {
    connection.notes.iter().find(|item| item.key == key)
}

#[test]
fn an_allowed_env_script_s_variables_reach_the_agent_and_one_that_is_not_allowed_runs_nothing() {
    let temp = Temp::new();
    let digest = temp.install_files(
        "team",
        "[env]\nTEAM = \"crew\"\n",
        &[(
            env_file(),
            printing(&temp.log(), r#"{"FROM_SCRIPT":"yes","TEAM":"script"}"#),
        )],
    );
    let connection =
        connect_overlays(&temp, &["team"], &[("team", &digest)], &real).expect("prepared");
    assert_eq!(env_of(&connection, "FROM_SCRIPT"), Some("yes"));
    // The script's output goes on top of what the overlay's own declarations set.
    assert_eq!(env_of(&connection, "TEAM"), Some("script"));
    let said = note(&connection, &format!("env_scripts.team.{}", env_file())).expect("said");
    assert_eq!(said.state, State::Applied);
    assert!(
        said.how
            .starts_with("env-script: ran; it set FROM_SCRIPT, TEAM"),
        "{}",
        said.how
    );
    assert!(
        !connection
            .notes
            .iter()
            .any(|item| item.how.starts_with("env-script-not-allowed"))
    );
    assert_eq!(temp.runs(), 1);

    // Not allowed, or allowed at another digest: reported, applied otherwise, and the script never starts.
    let other = format!("sha256:{}", "0".repeat(64));
    for allowed in [None, Some(other.as_str())] {
        let temp = Temp::new();
        temp.install_files(
            "team",
            "[env]\nTEAM = \"crew\"\n",
            &[(
                env_file(),
                printing(&temp.log(), r#"{"FROM_SCRIPT":"yes"}"#),
            )],
        );
        let allow: Vec<(&str, &str)> = allowed.map(|digest| ("team", digest)).into_iter().collect();
        let connection = connect_overlays(&temp, &["team"], &allow, &real).expect("prepared");
        assert_eq!(
            env_of(&connection, "TEAM"),
            Some("crew"),
            "the rest applies"
        );
        assert_eq!(env_of(&connection, "FROM_SCRIPT"), None);
        assert_eq!(temp.runs(), 0, "nothing ran");
        assert!(
            connection
                .notes
                .iter()
                .any(|item| item.how.starts_with("env-script-not-allowed")),
            "{:?}",
            connection.notes
        );
    }
}

#[test]
fn a_script_runs_once_however_many_environments_are_built_from_it() {
    let temp = Temp::new();
    let digest = temp.install_files(
        "team",
        "",
        &[(env_file(), printing(&temp.log(), r#"{"ONCE":"1"}"#))],
    );
    let connection =
        connect_overlays(&temp, &["team"], &[("team", &digest)], &real).expect("prepared");
    // The report builds the servers' environments after each overlay, from the same outputs.
    let parts = parts_for(&connection, None, &Takes::default(), &real);
    assert!(parts.report["items"].is_array());
    assert_eq!(temp.runs(), 1);
}

#[test]
fn the_layers_apply_in_composition_order_and_each_script_sees_what_the_earlier_ones_left() {
    let temp = Temp::new();
    let one = temp.install_files(
        "one",
        "priority = 100\n[env]\nOWN_ONE = \"declared-one\"\n",
        &[(
            env_file(),
            printing(&temp.log(), r#"{"FROM_ONE":"first","LEVEL":"one"}"#),
        )],
    );
    let two = temp.install_files(
        "two",
        "priority = 600\n[env]\nCOPY = \"${FROM_ONE}\"\n",
        &[(
            env_file(),
            reading(
                "FROM_ONE",
                "SEEN_ONE",
                &[("LEVEL", "two"), ("FROM_TWO", "second")],
            ),
        )],
    );
    // Whatever order the profile lists them in.
    for names in [["one", "two"], ["two", "one"]] {
        let connection = connect_overlays(&temp, &names, &[("one", &one), ("two", &two)], &real)
            .expect("prepared");
        assert_eq!(env_of(&connection, "FROM_ONE"), Some("first"), "{names:?}");
        // The later overlay's script saw the earlier one's output, and the later overlay's own declaration read it too.
        assert_eq!(env_of(&connection, "SEEN_ONE"), Some("first"), "{names:?}");
        assert_eq!(env_of(&connection, "COPY"), Some("first"), "{names:?}");
        // The later layer's value sits on top.
        assert_eq!(env_of(&connection, "LEVEL"), Some("two"), "{names:?}");
        assert_eq!(env_of(&connection, "FROM_TWO"), Some("second"), "{names:?}");
    }
}

#[test]
fn a_script_gets_its_overlay_s_own_name_and_a_value_is_never_substituted() {
    let temp = Temp::new();
    let digest = temp.install_files(
        "team",
        "",
        &[(
            env_file(),
            reading("AOS_OVERLAY_NAME", "NAMED", &[("LITERAL", "${HOME}")]),
        )],
    );
    let connection =
        connect_overlays(&temp, &["team"], &[("team", &digest)], &real).expect("prepared");
    assert_eq!(env_of(&connection, "NAMED"), Some("team"));
    assert_eq!(env_of(&connection, "LITERAL"), Some("${HOME}"));
}

#[test]
fn a_script_that_passes_its_time_limit_is_stopped_and_reported_and_the_session_still_starts() {
    let temp = Temp::new();
    let slow = if cfg!(windows) { "slow.ps1" } else { "slow.sh" };
    let digest = temp.install_files(
        "team",
        &format!(
            "[env]\nTEAM = \"crew\"\n[[env_scripts]]\ninterpreter = \"{}\"\npath = \"{slow}\"\ntimeout = 1\n",
            interpreter()
        ),
        &[(slow, waiting(60))],
    );
    let began = Instant::now();
    let connection =
        connect_overlays(&temp, &["team"], &[("team", &digest)], &real).expect("prepared");
    assert!(
        began.elapsed() < Duration::from_secs(40),
        "stopped at its limit, not run to its end"
    );
    assert_eq!(
        env_of(&connection, "TEAM"),
        Some("crew"),
        "the session starts"
    );
    let said = note(&connection, &format!("env_scripts.team.{slow}")).expect("said");
    assert_eq!(said.state, State::Refused);
    assert!(
        said.how
            .starts_with("env-script-failed: stopped at its time limit"),
        "{}",
        said.how
    );
}

#[test]
fn a_script_that_fails_is_reported_with_the_end_of_its_standard_error_and_applies_nothing() {
    let temp = Temp::new();
    let digest = temp.install_files(
        "team",
        "[env]\nTEAM = \"crew\"\n",
        &[(env_file(), failing("boom from the script", 3))],
    );
    let connection =
        connect_overlays(&temp, &["team"], &[("team", &digest)], &real).expect("prepared");
    assert_eq!(env_of(&connection, "TEAM"), Some("crew"));
    let said = note(&connection, &format!("env_scripts.team.{}", env_file())).expect("said");
    assert_eq!(said.state, State::Refused);
    assert!(
        said.how
            .starts_with("env-script-failed: it exited with status 3"),
        "{}",
        said.how
    );
    assert!(said.how.contains("boom from the script"), "{}", said.how);

    // An output that is not the specification's applies nothing, and its text is not quoted.
    let temp = Temp::new();
    let digest = temp.install_files(
        "team",
        "",
        &[(env_file(), printing(&temp.log(), "SECRET_TEXT_NOT_JSON"))],
    );
    let connection =
        connect_overlays(&temp, &["team"], &[("team", &digest)], &real).expect("prepared");
    let said = note(&connection, &format!("env_scripts.team.{}", env_file())).expect("said");
    assert_eq!(said.state, State::Refused);
    assert!(
        said.how.starts_with("env-script-failed: its output is not"),
        "{}",
        said.how
    );
    assert!(!said.how.contains("SECRET_TEXT_NOT_JSON"));
}

#[test]
fn a_script_that_cannot_start_is_reported_not_run() {
    let temp = Temp::new();
    let digest = temp.install_files(
        "team",
        "[[env_scripts]]\ninterpreter = \"node\"\npath = \"tools/x.js\"\n",
        &[
            ("env.js", "console.log('{}');".to_owned()),
            ("tools/x.js", "console.log('{}');".to_owned()),
        ],
    );
    // A PATH with no node on it.
    let lookup = |name: &str| {
        if name == "PATH" {
            Some(String::new())
        } else {
            real(name)
        }
    };
    let connection =
        connect_overlays(&temp, &["team"], &[("team", &digest)], &lookup).expect("prepared");
    // The default script has no interpreter to run it.
    let default = note(&connection, "env_scripts.team.env.js").expect("said");
    assert_eq!(default.state, State::Unsupported);
    assert!(default.how.starts_with("script-not-run"), "{}", default.how);
    let said = note(&connection, "env_scripts.team.tools/x.js").expect("said");
    assert_eq!(said.state, State::Refused);
    assert!(
        said.how
            .starts_with("env-script-failed: not run (node is not on the host's PATH)"),
        "{}",
        said.how
    );
}

#[test]
fn a_sensitive_variable_a_script_sets_is_applied_and_listed() {
    let temp = Temp::new();
    let digest = temp.install_files(
        "team",
        "",
        &[(
            env_file(),
            printing(&temp.log(), r#"{"HTTPS_PROXY":"http://proxy.example"}"#),
        )],
    );
    let connection =
        connect_overlays(&temp, &["team"], &[("team", &digest)], &real).expect("prepared");
    assert_eq!(
        env_of(&connection, "HTTPS_PROXY"),
        Some("http://proxy.example")
    );
    let listed = note(
        &connection,
        &format!("env_scripts.team.{}.HTTPS_PROXY", env_file()),
    )
    .expect("said");
    assert!(listed.how.starts_with("env-sensitive"), "{}", listed.how);
}

#[test]
fn a_hook_runs_in_the_environment_the_scripts_left() {
    let temp = Temp::new();
    let text = printing(&temp.log(), r#"{"FROM_SCRIPT":"seen-by-hook"}"#);
    let digest = temp.install_as(
        "team",
        "[hooks]\nprompt = {}\n",
        &[("prompt", Arm::LogEnv("FROM_SCRIPT".to_owned()))],
        &[(env_file(), text.as_str())],
    );
    let connection =
        connect_overlays(&temp, &["team"], &[("team", &digest)], &real).expect("prepared");
    prompt(&connection, "hello");
    assert_eq!(probed(&temp), ["seen-by-hook"]);
    assert!(
        script::logged(&temp.log())
            .iter()
            .any(|(event, _)| event == "prompt")
    );
}
