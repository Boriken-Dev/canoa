//! One account of the generic API-gateway kind, set to two standards, in the built `canoa`: it is made by signing in (the key typed
//! into the form, never in canoa.cfg), it runs a stand-in of Claude Code and a stand-in of Codex, and it lists the models of a
//! loopback stand-in for the gateway. A canary key is followed through everything the host writes.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod support;

use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use canoa_host::ACCOUNT_META;
use serde_json::{Value, json};
use support::{Host, Scratch, cwd, example, host_binary, quoted, with_link};

const CANARY: &str = "CANARY-gateway-key-0001";

/// A gateway on loopback that lists `claude-y` and `gpt-x` for a request that carries the canary as a bearer.
fn gateway() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("binds");
    let address = format!("http://{}", listener.local_addr().expect("an address"));
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            std::thread::spawn(move || {
                let mut head = Vec::new();
                let mut byte = [0_u8; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    if stream.read(&mut byte).unwrap_or(0) == 0 {
                        return;
                    }
                    head.push(byte[0]);
                }
                let head = String::from_utf8_lossy(&head).to_lowercase();
                let allowed = head.contains(&format!("bearer {}", CANARY.to_lowercase()));
                let (status, body) = if head.starts_with("get /v1/models ") && allowed {
                    (
                        "200 OK",
                        json!({"object": "list", "data": [{"id": "claude-y"}, {"id": "gpt-x"}]}),
                    )
                } else {
                    ("401 Unauthorized", json!({"error": "no"}))
                };
                let body = body.to_string();
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            });
        }
    });
    address
}

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

/// A scratch directory with a stand-in named `claude`, one named `codex`, and a canoa.cfg whose entries `Claude` and `Codex` run
/// them and offer no account yet.
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

    fn canoa(&self, args: &[&str], input: Option<&str>) -> Output {
        let mut child = Command::new(host_binary())
            .args(args)
            .arg("--config")
            .arg(&self.config)
            .env("COQUI_AGENTIC_PEER_HOME", self.scratch.0.join("home"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("canoa runs");
        let mut stdin = child.stdin.take().expect("its input");
        if let Some(input) = input {
            let _ = stdin.write_all(input.as_bytes());
        }
        drop(stdin);
        child.wait_with_output().expect("canoa ends")
    }

    /// Signs in to the account `gw` through `harness`, naming `address` and `standards`, and has both entries offer it.
    fn signed_in(self, harness: &str, address: &str, standards: &str) -> Self {
        self.signed_in_with(harness, address, standards, &[])
    }

    /// As [`Fixture::signed_in`], with more inputs for the sign-in (`NAME=VALUE`).
    fn signed_in_with(
        self,
        harness: &str,
        address: &str,
        standards: &str,
        more: &[(&str, &str)],
    ) -> Self {
        let mut args = vec![
            "accounts".to_owned(),
            "add".to_owned(),
            "gw".to_owned(),
            "--harness".to_owned(),
            harness.to_owned(),
            "--method".to_owned(),
            "api-gateway".to_owned(),
            "--input".to_owned(),
            format!("base_url={address}"),
            "--input".to_owned(),
            format!("standards={standards}"),
        ];
        for (name, value) in more {
            args.push("--input".to_owned());
            args.push(format!("{name}={value}"));
        }
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let added = self.canoa(&args, Some(&format!("{CANARY}\n")));
        assert!(added.status.success(), "{}", text(&added.stderr));
        assert_eq!(text(&added.stdout).trim(), "added gw (api-gateway)");
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
        self
    }
}

#[test]
fn one_account_of_two_standards_is_made_by_signing_in_and_its_key_is_not_in_canoa_cfg() {
    let address = gateway();
    let fixture = Fixture::new().signed_in("claude", &address, "openai-api, anthropic-api");
    let cfg = std::fs::read_to_string(&fixture.config).expect("canoa.cfg");
    assert!(!cfg.contains(CANARY), "{cfg}");
    assert!(
        cfg.contains(&address) && cfg.contains("anthropic-api"),
        "{cfg}"
    );
    let listed = fixture.canoa(&["accounts", "list", "--format", "json"], None);
    assert!(listed.status.success(), "{}", text(&listed.stderr));
    let out = text(&listed.stdout);
    assert!(!out.contains(CANARY), "{out}");
    let row: Value = serde_json::from_str(out.lines().next().expect("a row")).expect("JSON");
    assert_eq!(row["kind"], "api-gateway", "{row}");
    assert_eq!(
        row["provides"],
        json!(["openai-api", "anthropic-api"]),
        "{row}"
    );
    // A key is refused in canoa.cfg, naming the setting.
    let refused = text(
        &fixture
            .canoa(
                &[
                    "accounts",
                    "add",
                    "bad",
                    "--harness",
                    "codex",
                    "--method",
                    "api-gateway",
                    "--input",
                    "base_url=http://llm.example.com",
                    "--input",
                    "standards=openai-api",
                ],
                Some("k\n"),
            )
            .stderr,
    );
    assert!(refused.contains("https"), "{refused}");
}

#[test]
fn it_runs_a_stand_in_of_each_harness_on_the_same_account_and_lists_the_gateways_models_for_both() {
    let address = gateway();
    let fixture = Fixture::new().signed_in("claude", &address, "openai-api, anthropic-api");
    // The models, as `canoa options` asks for each entry on the account.
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
        let answer: Value = serde_json::from_slice(&listed.stdout).expect("one JSON object");
        assert_eq!(answer["known"], true, "{entry}: {answer}");
        let ids: Vec<&str> = answer["models"]
            .as_array()
            .expect("models")
            .iter()
            .filter_map(|model| model["id"].as_str())
            .collect();
        assert_eq!(ids, ["claude-y", "gpt-x"], "{entry}");
        assert!(!answer.to_string().contains(CANARY));
    }
    // Each stand-in is started on it, with the address in the place its program takes one.
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
            json!({"name": "gw", "kind": "api-gateway"}),
            "{entry}: {answer}"
        );
    }
    waits_for("both stand-ins started", || records(&record).len() >= 2);
    let started = records(&record);
    let claude = started
        .iter()
        .find(|each| each["env"]["ANTHROPIC_BASE_URL"] == json!(address))
        .unwrap_or_else(|| panic!("Claude Code was given the address: {started:?}"));
    assert!(
        claude["env_names"]
            .as_array()
            .is_some_and(|names| names.iter().any(|name| name == "ANTHROPIC_API_KEY")),
        "and the key in the variable it reads: {claude}"
    );
    assert!(
        !claude.to_string().contains(CANARY),
        "the record names no value of it"
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
        codex["args"].to_string().contains(&address),
        "Codex was given the address: {codex}"
    );
    // The key is in the log, in no answer and in canoa.cfg: nowhere the host writes.
    let logged = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(!logged.contains(CANARY), "{logged}");
    let cfg = std::fs::read_to_string(host.scratch.0.join("canoa.cfg")).expect("canoa.cfg");
    assert!(!cfg.contains(CANARY));
}

#[test]
fn an_account_that_names_an_address_and_a_key_form_for_each_standard_runs_each_harness_at_its_own()
{
    let default = "http://127.0.0.1:3";
    let openai = "http://127.0.0.1:1/openai/v1";
    let anthropic = "http://127.0.0.1:2/anthropic";
    let fixture = Fixture::new().signed_in_with(
        "codex",
        default,
        "openai-api, anthropic-api",
        &[
            (
                "addresses",
                &format!("openai-api={openai},anthropic-api={anthropic}"),
            ),
            ("credentials", "anthropic-api=auth_token"),
        ],
    );
    // The addresses and the key forms are the account's settings, and the key is still in no file.
    let cfg = std::fs::read_to_string(&fixture.config).expect("canoa.cfg");
    assert!(
        cfg.contains(openai) && cfg.contains(anthropic) && cfg.contains("auth_token"),
        "{cfg}"
    );
    assert!(!cfg.contains(CANARY), "{cfg}");
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
        with_link(host.url(entry), &client, host.peer, |rpc| {
            rpc.request(
                "initialize",
                json!({"protocolVersion": 1, "clientCapabilities": {}, "_meta": {ACCOUNT_META: "gw"}}),
            )
            .expect("initialized on the account");
            rpc.request("session/new", json!({"cwd": dir, "mcpServers": []}))
                .expect("a session opens");
        })
        .expect("served");
    }
    waits_for("both stand-ins started", || records(&record).len() >= 2);
    let started = records(&record);
    // Claude Code is sent to the address of the Anthropic API, with the key as the bearer token the account says it takes there.
    let claude = started
        .iter()
        .find(|each| each["env"]["ANTHROPIC_BASE_URL"] == json!(anthropic))
        .unwrap_or_else(|| panic!("Claude Code was given the Anthropic address: {started:?}"));
    let names: Vec<&str> = claude["env_names"]
        .as_array()
        .expect("names")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(names.contains(&"ANTHROPIC_AUTH_TOKEN"), "{names:?}");
    assert!(!names.contains(&"ANTHROPIC_API_KEY"), "{names:?}");
    // Codex is sent to the address of the OpenAI API, and never to the other two.
    let codex = started
        .iter()
        .find(|each| {
            each["args"]
                .as_array()
                .is_some_and(|args| args.iter().any(|arg| arg == "app-server"))
        })
        .unwrap_or_else(|| panic!("Codex started: {started:?}"));
    let args = codex["args"].to_string();
    assert!(args.contains(openai), "{codex}");
    assert!(
        !args.contains(anthropic) && !args.contains(default),
        "{codex}"
    );
    let logged = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(!logged.contains(CANARY), "{logged}");
}
