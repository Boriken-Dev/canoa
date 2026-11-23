//! The mail service through the built `canoa`: a password read from standard input rests in the account store and in no file of canoa.cfg's,
//! `canoa service mail accounts` never prints it, and `canoa service mail remove` forgets it. The store is the file store, in a directory of the test's.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod support;

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::Value;
use support::{Scratch, host_binary};

const PASSWORD: &str = "CANARY-mail-pw-7d3f";

/// A canoa.cfg on the file store, and the home it keeps accounts in.
struct Fixture {
    scratch: Scratch,
    config: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let scratch = Scratch::new();
        let config = scratch.0.join("canoa.cfg");
        std::fs::write(&config, "[secrets]\naccounts = \"file\"\n").expect("canoa.cfg");
        Self { scratch, config }
    }

    fn home(&self) -> PathBuf {
        self.scratch.0.join("home")
    }

    /// `canoa ARGS --config canoa.cfg` on this fixture's home, with `input` on its standard input.
    fn canoa(&self, args: &[&str], input: &str) -> Output {
        let mut child = Command::new(host_binary())
            .args(args)
            .arg("--config")
            .arg(&self.config)
            .env("COQUI_AGENTIC_PEER_HOME", self.home())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("canoa runs");
        let mut stdin = child.stdin.take().expect("its input");
        // A process that ends before it reads (it refused something first) closes the pipe: its status says why.
        let _ = stdin.write_all(input.as_bytes());
        drop(stdin);
        child.wait_with_output().expect("canoa ends")
    }

    /// `canoa service mail add --params -` with the account as the JSON on standard input, the password in it.
    fn add(&self, name: &str, password: &str) -> Output {
        let account = serde_json::json!({
            "name": name, "backend": "ews", "server": "https://mail.example.test/EWS/Exchange.asmx",
            "user": "EXAMPLE\\ana", "password": password,
        });
        self.canoa(
            &["service", "mail", "add", "--params", "-"],
            &account.to_string(),
        )
    }

    /// Every byte under the scratch directory, to look for the password in.
    fn everything(&self) -> Vec<u8> {
        fn walk(path: &Path, into: &mut Vec<u8>) {
            let Ok(entries) = std::fs::read_dir(path) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, into);
                } else if let Ok(bytes) = std::fs::read(&path) {
                    into.extend(bytes);
                }
            }
        }
        let mut bytes = Vec::new();
        walk(&self.scratch.0, &mut bytes);
        bytes
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

#[test]
fn a_password_read_from_input_rests_in_the_store_and_is_forgotten_with_the_account() {
    let fixture = Fixture::new();
    let added = fixture.add("work", PASSWORD);
    assert!(added.status.success(), "{}", text(&added.stderr));
    let answer: Value = serde_json::from_str(text(&added.stdout).trim()).expect("JSON");
    assert_eq!(answer["account"]["name"], "work", "{answer}");

    let cfg = std::fs::read_to_string(&fixture.config).expect("canoa.cfg");
    assert!(
        cfg.contains("[host.services.mail.accounts.work]") && cfg.contains("id = \""),
        "{cfg}"
    );
    assert!(
        !contains(&fixture.everything(), PASSWORD),
        "the password is in no file the host wrote, canoa.cfg included"
    );
    assert!(
        !text(&added.stdout).contains(PASSWORD) && !text(&added.stderr).contains(PASSWORD),
        "and in nothing it said"
    );

    let listed = fixture.canoa(&["service", "mail", "accounts"], "");
    assert!(listed.status.success(), "{}", text(&listed.stderr));
    let answer: Value = serde_json::from_str(text(&listed.stdout).trim()).expect("JSON");
    let rows = answer["accounts"].as_array().expect("accounts");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["name"], "work");
    assert_eq!(rows[0]["backend"], "ews");
    assert_eq!(rows[0]["state"], "password stored");
    assert!(!text(&listed.stdout).contains(PASSWORD));

    let removed = fixture.canoa(
        &[
            "service",
            "mail",
            "remove",
            "--params",
            "{\"name\": \"work\"}",
        ],
        "",
    );
    assert!(removed.status.success(), "{}", text(&removed.stderr));
    let cfg = std::fs::read_to_string(&fixture.config).expect("canoa.cfg");
    assert!(!cfg.contains("work"), "{cfg}");
    let listed = fixture.canoa(&["service", "mail", "accounts"], "");
    assert!(listed.status.success());
    let answer: Value = serde_json::from_str(text(&listed.stdout).trim()).expect("JSON");
    assert_eq!(
        answer["accounts"],
        serde_json::json!([]),
        "no account is left"
    );
    let sealed = std::fs::read_dir(fixture.home().join("accounts"))
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "sealed"))
                .count()
        })
        .unwrap_or(0);
    assert_eq!(sealed, 0, "the stored password is gone");
}

#[test]
fn an_empty_password_and_a_refused_address_leave_nothing_behind() {
    let fixture = Fixture::new();
    let before = std::fs::read_to_string(&fixture.config).expect("canoa.cfg");
    let empty = fixture.add("work", "");
    assert!(!empty.status.success());
    assert!(
        text(&empty.stderr).contains("needs a password"),
        "{}",
        text(&empty.stderr)
    );

    let clear = fixture.canoa(
        &["service", "mail", "add", "--params", "-"],
        &serde_json::json!({
            "name": "work", "backend": "ews", "server": "ftp://mail.example.com/EWS/Exchange.asmx",
            "user": "u", "password": PASSWORD,
        })
        .to_string(),
    );
    assert_eq!(clear.status.code(), Some(1));
    assert!(
        text(&clear.stderr).contains("http or https address"),
        "{}",
        text(&clear.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(&fixture.config).expect("canoa.cfg"),
        before
    );
    assert!(!contains(&fixture.everything(), PASSWORD));
}

/// With `CANOA_CREDENTIAL_STORE=memory`, which `Scratch` sets for every test, a canoa.cfg that leaves `[secrets]` at its default
/// keeps the password in the process's memory: the next process does not have it, and nothing rests in a file or in the operating
/// system's store.
#[test]
fn a_password_kept_while_the_credential_store_is_in_memory_ends_with_the_process() {
    let fixture = Fixture::new();
    std::fs::write(&fixture.config, "").expect("canoa.cfg");
    let added = fixture.add("work", PASSWORD);
    assert!(added.status.success(), "{}", text(&added.stderr));
    assert!(
        !contains(&fixture.everything(), PASSWORD),
        "the password is in no file"
    );
    let listed = fixture.canoa(&["service", "mail", "accounts"], "");
    assert!(listed.status.success(), "{}", text(&listed.stderr));
    let answer: Value = serde_json::from_str(text(&listed.stdout).trim()).expect("JSON");
    assert_ne!(
        answer["accounts"][0]["state"], "password stored",
        "the password outlived the process that was given it"
    );
}
