//! A LiteLLM account in the built `canoa`, against a loopback stand-in for the proxy: made by the proxy's SSO (the test plays the
//! browser at the page `canoa accounts add` shows) and by a key typed into the form, and one account running a stand-in of Claude Code
//! and a stand-in of Codex, which list the proxy's models. No browser is opened and no credential of the person's is read.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

#[path = "../../../extensions/canoa-litellm/tests/standin/mod.rs"]
mod standin;
mod support;

use std::io::{BufRead as _, BufReader, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use canoa_host::ACCOUNT_META;
use serde_json::{Value, json};
use standin::{Proxy, VIRTUAL_KEY};
use support::{Host, Scratch, cwd, example, host_binary, quoted, with_link};

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn records(file: &Path) -> Vec<Value> {
    std::fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

fn waits_for(what: &str, mut done: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(60);
    while !done() {
        assert!(Instant::now() < until, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A scratch directory with a stand-in named `claude`, one named `codex`, and a canoa.cfg whose entries run them.
struct Fixture {
    scratch: Scratch,
    config: PathBuf,
    record: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let scratch = Scratch::new();
        let mut entries = String::new();
        for (entry, harness) in [("Claude", "claude"), ("Codex", "codex")] {
            let program = scratch
                .0
                .join(format!("{harness}{}", std::env::consts::EXE_SUFFIX));
            std::fs::copy(example("harness_standin", false), &program).expect("the stand-in");
            entries.push_str(&format!(
                "[host.agents.{entry}]\nharness = \"{harness}\"\nexecutable = {}\n",
                quoted(&program.to_string_lossy())
            ));
        }
        let config = scratch.0.join("canoa.cfg");
        std::fs::write(
            &config,
            format!("[secrets]\naccounts = \"file\"\n\n{entries}"),
        )
        .expect("canoa.cfg");
        let record = scratch.0.join("record.jsonl");
        Self {
            scratch,
            config,
            record,
        }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(host_binary());
        command
            .args(args)
            .arg("--config")
            .arg(&self.config)
            .env("COQUI_AGENTIC_PEER_HOME", self.scratch.0.join("home"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }

    fn canoa(&self, args: &[&str], input: Option<&str>) -> Output {
        let mut child = self.command(args).spawn().expect("canoa runs");
        let mut stdin = child.stdin.take().expect("its input");
        if let Some(input) = input {
            let _ = stdin.write_all(input.as_bytes());
        }
        drop(stdin);
        child.wait_with_output().expect("canoa ends")
    }

    /// `canoa accounts add gw --provider litellm_accounts --method M`, where the person is the test: each page the sign-in asks to open is
    /// "visited" by `browse`. Answers what it printed and whether it succeeded.
    fn add_as(&self, method: &str, address: &str, browse: impl Fn(&str)) -> (bool, String, bool) {
        let input = format!("base_url={address}");
        let mut child = self
            .command(&[
                "accounts",
                "add",
                "gw",
                "--provider",
                "litellm_accounts",
                "--method",
                method,
                "--input",
                &input,
            ])
            .spawn()
            .expect("canoa runs");
        let mut stdin = child.stdin.take().expect("its input");
        let (lines, heard) = mpsc::channel::<String>();
        let mut readers = Vec::new();
        for stream in [
            Box::new(child.stdout.take().expect("stdout")) as Box<dyn std::io::Read + Send>,
            Box::new(child.stderr.take().expect("stderr")),
        ] {
            let lines = lines.clone();
            readers.push(std::thread::spawn(move || {
                for line in BufReader::new(stream).lines().map_while(Result::ok) {
                    let _ = lines.send(line);
                }
            }));
        }
        drop(lines);
        let mut said = String::new();
        let mut opened = false;
        let until = Instant::now() + Duration::from_secs(60);
        while let Ok(line) = heard.recv_timeout(until.saturating_duration_since(Instant::now())) {
            said.push_str(&line);
            said.push('\n');
            if let Some(page) = line.strip_prefix("Open this page: ") {
                opened = true;
                browse(page.trim());
            }
        }
        let _ = stdin.write_all(b"");
        drop(stdin);
        let status = child.wait().expect("canoa ends");
        for reader in readers {
            let _ = reader.join();
        }
        (status.success(), said, opened)
    }

    fn offer_to_both(&self) {
        let cfg = std::fs::read_to_string(&self.config).expect("canoa.cfg");
        std::fs::write(
            &self.config,
            cfg.replace(
                "[host.agents.Claude]\n",
                "[host.agents.Claude]\naccounts = [\"gw\"]\n",
            )
            .replace(
                "[host.agents.Codex]\n",
                "[host.agents.Codex]\naccounts = [\"gw\"]\n",
            ),
        )
        .expect("the entries offer the account");
    }

    fn listed(&self) -> Value {
        let listed = self.canoa(&["accounts", "list", "--format", "json"], None);
        assert!(listed.status.success(), "{}", text(&listed.stderr));
        serde_json::from_str(text(&listed.stdout).lines().next().expect("a row")).expect("JSON")
    }
}

#[test]
fn signing_in_through_the_proxys_sso_makes_the_account_and_the_key_rests_in_the_store_alone() {
    let proxy = Proxy::start();
    let fixture = Fixture::new();
    let (made, said, opened) = fixture.add_as("litellm-sso", &proxy.address, |page| {
        // The page is the proxy's, and the person's browser would go on to the address it redirects to.
        let page = page.to_owned();
        let proxy_address = proxy.address.clone();
        assert!(
            page.starts_with(&format!("{proxy_address}/authorize?")),
            "{page}"
        );
        assert_eq!(proxy.browse(&page), 200);
    });
    assert!(made && opened, "{said}");
    assert!(said.contains("added gw (litellm)"), "{said}");
    // A key the proxy issued is nowhere the host writes: not canoa.cfg, not what it printed.
    let cfg = std::fs::read_to_string(&fixture.config).expect("canoa.cfg");
    assert!(
        !cfg.contains("sk-issued") && !cfg.contains("refresh-"),
        "{cfg}"
    );
    assert!(
        cfg.contains(&proxy.address) && cfg.contains("litellm"),
        "{cfg}"
    );
    assert!(
        !said.contains("sk-issued") && !said.contains("refresh-"),
        "{said}"
    );
    let row = fixture.listed();
    assert_eq!(row["kind"], "litellm", "{row}");
    assert_eq!(row["provider"], "litellm_accounts", "{row}");
    assert_eq!(row["fits"], json!(["claude", "codex"]), "{row}");
    assert_eq!(
        row["types"],
        json!(["openai-api", "anthropic-api"]),
        "{row}"
    );
    assert!(!row.to_string().contains("sk-issued"), "{row}");
    // The key it holds is the one the proxy issued: it reads the proxy's models and nothing else made a request with another.
    fixture.offer_to_both();
    let listed = fixture.canoa(
        &[
            "options",
            "Claude",
            "--entry",
            "--account",
            "gw",
            "--format",
            "json",
        ],
        None,
    );
    assert!(listed.status.success(), "{}", text(&listed.stderr));
    let answer: Value = serde_json::from_slice(&listed.stdout).expect("JSON");
    assert_eq!(answer["known"], true, "{answer}");
    assert!(
        proxy
            .to("/v1/models")
            .iter()
            .all(|each| each.credential == "sk-issued-1"),
        "{:?}",
        proxy.seen()
    );
    // A proxy with no such sign-in refuses it and no account is made.
    let old = Proxy::start_with(false);
    let other = Fixture::new();
    let (made, said, opened) = other.add_as("litellm-sso", &old.address, |_| {});
    assert!(
        !made && !opened && said.contains("offers no sign-in"),
        "{said}"
    );
    assert!(
        !std::fs::read_to_string(&other.config)
            .expect("canoa.cfg")
            .contains("litellm")
    );
}

#[test]
fn one_litellm_account_set_by_a_key_runs_a_stand_in_of_each_harness_and_both_list_the_proxys_models()
 {
    let proxy = Proxy::start();
    let fixture = Fixture::new();
    let added = fixture.canoa(
        &[
            "accounts",
            "add",
            "gw",
            "--provider",
            "litellm_accounts",
            "--method",
            "litellm-key",
            "--input",
            &format!("base_url={}/v1", proxy.address),
        ],
        Some(&format!("{VIRTUAL_KEY}\n")),
    );
    assert!(added.status.success(), "{}", text(&added.stderr));
    assert_eq!(text(&added.stdout).trim(), "added gw (litellm)");
    let cfg = std::fs::read_to_string(&fixture.config).expect("canoa.cfg");
    assert!(!cfg.contains(VIRTUAL_KEY), "{cfg}");
    assert!(
        cfg.contains(&format!("\"{}\"", proxy.address)) && !cfg.contains("/v1"),
        "the proxy's root, not /v1: {cfg}"
    );
    fixture.offer_to_both();
    for entry in ["Claude", "Codex"] {
        let listed = fixture.canoa(
            &[
                "options",
                entry,
                "--entry",
                "--account",
                "gw",
                "--format",
                "json",
            ],
            None,
        );
        assert!(listed.status.success(), "{}", text(&listed.stderr));
        let answer: Value = serde_json::from_slice(&listed.stdout).expect("JSON");
        assert_eq!(answer["known"], true, "{entry}: {answer}");
        let ids: Vec<&str> = answer["models"]
            .as_array()
            .expect("models")
            .iter()
            .filter_map(|model| model["id"].as_str())
            .collect();
        assert_eq!(ids, ["claude-y", "gpt-x"], "{entry}");
    }
    let record = fixture.record.clone();
    let env = vec![
        ("STANDIN_RECORD", record.clone().into_os_string()),
        ("STANDIN_ENV", "ANTHROPIC_BASE_URL".into()),
    ];
    let config = std::fs::read_to_string(&fixture.config).expect("canoa.cfg");
    let (host, log) = Host::start_logged(fixture.scratch, &config, 2, &env);
    let client = support::key();
    host.pair(&client);
    let dir = cwd(&host.scratch);
    for entry in ["Claude", "Codex"] {
        let answer = with_link(host.url(entry), &client, host.peer, |rpc| {
            let answer = rpc
                .request(
                    "initialize",
                    json!({"protocolVersion": 1, "clientCapabilities": {}, "_meta": {ACCOUNT_META: "gw"}}),
                )
                .expect("initialized on the account");
            rpc.request("session/new", json!({"cwd": dir, "mcpServers": []}))
                .expect("a session opens");
            answer
        })
        .expect("served");
        assert_eq!(
            answer["_meta"][ACCOUNT_META],
            json!({"name": "gw", "kind": "litellm"}),
            "{entry}: {answer}"
        );
    }
    waits_for("both stand-ins started", || records(&record).len() >= 2);
    let started = records(&record);
    let claude = started
        .iter()
        .find(|each| each["env"]["ANTHROPIC_BASE_URL"] == json!(proxy.address))
        .unwrap_or_else(|| panic!("Claude Code was given the proxy's address: {started:?}"));
    assert!(
        claude["env_names"]
            .as_array()
            .is_some_and(|names| names.iter().any(|name| name == "ANTHROPIC_API_KEY")),
        "{claude}"
    );
    let codex = started
        .iter()
        .find(|each| {
            each["args"]
                .as_array()
                .is_some_and(|args| args.iter().any(|arg| arg == "app-server"))
        })
        .unwrap_or_else(|| panic!("Codex started: {started:?}"));
    assert!(
        codex["args"].to_string().contains(&proxy.address),
        "Codex was given it too: {codex}"
    );
    let logged = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(!logged.contains(VIRTUAL_KEY), "{logged}");
}
