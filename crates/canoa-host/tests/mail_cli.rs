//! Mail accounts in the built `canoa`: a password read from standard input rests in the account store and in no file of canoa.cfg's,
//! `canoa mail list` never prints it, and `canoa mail remove` forgets it. The store is the file store, in a directory of the test's.
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
        std::fs::write(&config, "[host]\naccount_store = \"file\"\n").expect("canoa.cfg");
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

    fn add(&self, name: &str, input: &str) -> Output {
        self.canoa(
            &[
                "mail",
                "add",
                name,
                "--backend",
                "ews",
                "--server",
                "https://mail.example.test/EWS/Exchange.asmx",
                "--user",
                "EXAMPLE\\ana",
            ],
            input,
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
    let added = fixture.add("work", &format!("{PASSWORD}\n"));
    assert!(added.status.success(), "{}", text(&added.stderr));
    assert_eq!(text(&added.stdout).lines().next(), Some("added work (ews)"));

    let cfg = std::fs::read_to_string(&fixture.config).expect("canoa.cfg");
    assert!(
        cfg.contains("[host.mail.accounts.work]") && cfg.contains("id = \""),
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

    let listed = fixture.canoa(&["mail", "list", "--format", "json"], "");
    assert!(listed.status.success(), "{}", text(&listed.stderr));
    let rows: Vec<Value> = text(&listed.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).expect("one JSON object per line"))
        .collect();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["name"], "work");
    assert_eq!(rows[0]["backend"], "ews");
    assert_eq!(rows[0]["state"], "password stored");
    assert!(!text(&listed.stdout).contains(PASSWORD));

    let removed = fixture.canoa(&["mail", "remove", "work"], "");
    assert!(removed.status.success(), "{}", text(&removed.stderr));
    let cfg = std::fs::read_to_string(&fixture.config).expect("canoa.cfg");
    assert!(!cfg.contains("work"), "{cfg}");
    let listed = fixture.canoa(&["mail", "list"], "");
    assert!(listed.status.success());
    assert_eq!(text(&listed.stdout).trim(), "", "no account is left");
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
    let empty = fixture.add("work", "\n");
    assert!(!empty.status.success());
    assert!(
        text(&empty.stderr).contains("no password"),
        "{}",
        text(&empty.stderr)
    );

    let clear = fixture.canoa(
        &[
            "mail",
            "add",
            "work",
            "--backend",
            "ews",
            "--server",
            "http://mail.example.com/EWS/Exchange.asmx",
            "--user",
            "u",
        ],
        &format!("{PASSWORD}\n"),
    );
    assert_eq!(clear.status.code(), Some(1));
    assert!(
        text(&clear.stderr).contains("in the clear"),
        "{}",
        text(&clear.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(&fixture.config).expect("canoa.cfg"),
        before
    );
    assert!(!contains(&fixture.everything(), PASSWORD));
}
