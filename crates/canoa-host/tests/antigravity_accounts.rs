//! Antigravity's and Grok's accounts over the control channel, as the desktop app uses them: the account providers and the sources
//! they make listed, an Antigravity account added by starting the program, and what its row says of who is signed in and of its
//! usage. The program is a stand-in everywhere; every folder an agent keeps its login in is this test's own, so no test reads the
//! person's.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod linked;
mod sealed;
mod support;

use std::path::{Path, PathBuf};
use std::sync::Once;

use canoa_host::Link;
use linked::{Hosted, next, send};
use sealed::{App, Typed};
use serde_json::{Value, json};
use support::{example, quoted};

/// Points every variable an agent's login folder is found by at a folder of this test's, before any host starts.
fn isolate() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let folder = std::env::temp_dir().join(format!("canoa-agy-logins-{}", std::process::id()));
        std::fs::create_dir_all(&folder).expect("a folder");
        for name in [
            "CLAUDE_CONFIG_DIR",
            "CODEX_HOME",
            "PI_CODING_AGENT_DIR",
            "GROK_HOME",
        ] {
            // SAFETY: set once, before this test's hosts start, and every test of this file calls `isolate` first.
            unsafe { std::env::set_var(name, folder.join(name)) };
        }
    });
}

impl App {
    /// Sends `method` and plays the person until its answer: the progress seen, and the answer or the error.
    fn drive(
        &mut self,
        link: &Link,
        id: u64,
        method: &str,
        params: &Value,
        typed: &Typed,
    ) -> (Vec<Value>, Result<Value, Value>) {
        let line = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        self.traffic.push(line.to_string());
        send(link, &line);
        let mut progress = Vec::new();
        loop {
            let message = match next(link) {
                Ok(message) => message,
                Err(why) => return (progress, Err(json!(why))),
            };
            self.traffic.push(message.to_string());
            if message["method"] == "_coqui_acp/secret_request" {
                let Some(result) = self.answer(&message["params"], typed) else {
                    return (progress, Err(json!("the person left")));
                };
                let reply = json!({"jsonrpc": "2.0", "id": message["id"], "result": result});
                self.traffic.push(reply.to_string());
                send(link, &reply);
            } else if message["method"] == "accounts/progress" {
                progress.push(message["params"].clone());
            } else if message["id"] == id && message.get("method").is_none() {
                return match message.get("error") {
                    Some(error) => (progress, Err(error.clone())),
                    None => (progress, Ok(message["result"].clone())),
                };
            }
        }
    }
}

fn plain_call(link: &Link, id: u64, method: &str, params: Value) -> Result<Value, String> {
    linked::ask(link, id, method, params).map(|(result, _)| result)
}

/// A program of the test's that stands for `agy`: `body` is what its script does.
fn program(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(if cfg!(windows) {
        format!("{name}.cmd")
    } else {
        name.to_owned()
    });
    if cfg!(windows) {
        std::fs::write(&path, format!("@echo off\r\n{body}\r\n")).expect("script written");
    } else {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("script written");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("script executable");
        }
    }
    path
}

/// The stand-in agent playing Antigravity's status surface from the recorded answer.
fn agy_standin(dir: &Path) -> PathBuf {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("agy")
        .join("getuserstatus.json");
    let agent = example("stdio_agent", false);
    let body = if cfg!(windows) {
        format!("\"{}\" --agy \"{}\"", agent.display(), fixture.display())
    } else {
        format!(
            "exec \"{}\" --agy \"{}\"",
            agent.display(),
            fixture.display()
        )
    };
    program(dir, "agy", &body)
}

fn app_of(host: &Hosted) -> App {
    let secret = host.local.link_secret().expect("the host minted a secret");
    App::new(&secret[..])
}

/// A host with an Antigravity entry that runs `program` and signs in and out through it.
fn host_running(build: impl FnOnce(&Path) -> PathBuf) -> Hosted {
    isolate();
    Hosted::start_with_setup(
        |dir| {
            let program = build(dir);
            let program = quoted(&program.to_string_lossy());
            format!(
                "[secrets]\naccounts = \"file\"\n\n[host.agents.Antigravity]\nharness = \"antigravity\"\nexecutable = {program}\naccount_settings = {{ executable = {program} }}\n"
            )
        },
        &[],
    )
}

#[test]
fn grok_and_antigravity_account_providers_and_sources_are_listed() {
    isolate();
    let agent = example("stdio_agent", false);
    let host = Hosted::start_with_setup(
        |dir| {
            let grok = dir.join(format!("grok{}", std::env::consts::EXE_SUFFIX));
            let agy = dir.join(format!("agy{}", std::env::consts::EXE_SUFFIX));
            std::fs::copy(&agent, &grok).expect("grok stand-in copied");
            std::fs::copy(&agent, &agy).expect("agy stand-in copied");
            format!(
                "[secrets]\naccounts = \"file\"\n\n\
                 [host.agents.Grok]\nharness = \"grok\"\nexecutable = {}\n\n\
                 [host.agents.Antigravity]\nharness = \"antigravity\"\nexecutable = {}\n",
                quoted(&grok.to_string_lossy()),
                quoted(&agy.to_string_lossy()),
            )
        },
        &[],
    );
    let link = host.local.open("@control").expect("a link");
    let listed = plain_call(&link, 1, "providers/list", json!({})).expect("providers");
    let agents = listed["providers"].as_array().expect("providers array");
    for (id, accounts_id) in [
        ("grok", "grok_accounts"),
        ("antigravity", "antigravity_accounts"),
    ] {
        let entry = agents
            .iter()
            .find(|agent| agent["id"] == id)
            .unwrap_or_else(|| panic!("{id} missing from {listed}"));
        assert_eq!(entry["harness"], true, "{entry}");
        assert_eq!(entry["accounts"]["provider"], accounts_id, "{entry}");
        let catalog =
            plain_call(&link, 2, "accounts/catalog", json!({"harness": id})).expect("catalog");
        assert!(catalog["kinds"].is_array(), "{id}: {catalog}");
        assert!(catalog["logins"].is_array(), "{id}: {catalog}");
    }
    let accounts = plain_call(&link, 3, "accounts/list", json!({})).expect("accounts");
    let rows = accounts["accounts"].as_array().expect("accounts array");
    for harness in ["grok", "antigravity"] {
        assert!(
            rows.iter()
                .any(|row| row["harness"] == harness && row["name"] == "local"),
            "missing local account for {harness} in {accounts}"
        );
    }
    let entries = plain_call(&link, 4, "accounts/entries", json!({})).expect("entries");
    let entry_rows = entries["entries"].as_array().expect("entries array");
    for name in ["Grok", "Antigravity"] {
        assert!(
            entry_rows.iter().any(|entry| entry["name"] == name),
            "{entries}"
        );
    }
    host.stop();
}

#[test]
fn antigravity_is_added_by_starting_it_and_its_row_says_who_is_signed_in_and_what_is_left() {
    let host = host_running(agy_standin);
    let link = host.local.open("@control").expect("a link");
    let catalog = plain_call(
        &link,
        1,
        "accounts/catalog",
        json!({"harness": "antigravity"}),
    )
    .expect("catalog");
    let logins = catalog["logins"].as_array().expect("logins");
    assert_eq!(logins.len(), 1, "{catalog}");
    assert_eq!(logins[0]["id"], "start", "{catalog}");
    assert_eq!(
        logins[0]["title"], "Start Antigravity to sign in",
        "{catalog}"
    );

    let mut app = app_of(&host);
    let (_, added) = app.drive(
        &link,
        2,
        "accounts/add",
        &json!({"name": "agy", "harness": "antigravity", "method": "start"}),
        &Typed::Leaves,
    );
    let added = added.expect("the stand-in opens a session, so the account is added");
    assert_eq!(added["account"]["name"], "agy", "{added}");
    assert_eq!(added["account"]["kind"], "harness-login", "{added}");
    assert_eq!(added["account"]["checked"], true, "{added}");

    let read = plain_call(&link, 3, "usage/read", json!({"account": "agy"})).expect("usage/read");
    let row = &read["account"];
    assert_eq!(row["supported"], true, "{read}");
    assert!(
        row["details"]["label"]
            .as_str()
            .is_some_and(|label| !label.is_empty()),
        "the row says who is signed in: {read}"
    );
    assert_eq!(row["details"]["plan"], "Google AI Pro", "{read}");
    assert!(
        row["buckets"]
            .as_array()
            .is_some_and(|buckets| buckets.iter().any(|bucket| bucket["id"]
                .as_str()
                .is_some_and(|id| id.starts_with("agy:")))),
        "the row has the quota: {read}"
    );
    host.stop();
}

#[test]
fn antigravity_that_opens_no_session_asks_the_person_to_start_it_and_adds_nothing_when_they_leave()
{
    let host = host_running(|dir| {
        program(
            dir,
            "agy",
            if cfg!(windows) { "exit /b 1" } else { "exit 1" },
        )
    });
    let link = host.local.open("@control").expect("a link");
    let mut app = app_of(&host);
    let (_, outcome) = app.drive(
        &link,
        1,
        "accounts/add",
        &json!({"name": "agy", "harness": "antigravity", "method": "start"}),
        &Typed::Leaves,
    );
    assert!(outcome.is_err(), "{outcome:?}");
    assert!(
        app.traffic
            .iter()
            .any(|line| line.contains("Start Antigravity to sign in")
                && line.contains("Start Antigravity, sign in there, then continue")),
        "the person is asked to start it: {:?}",
        app.traffic
    );
    let accounts = plain_call(&link, 2, "accounts/list", json!({})).expect("accounts");
    assert!(
        !accounts["accounts"]
            .as_array()
            .expect("accounts array")
            .iter()
            .any(|row| row["name"] == "agy"),
        "{accounts}"
    );
    host.stop();
}
