//! The account-side limits reads through `usage_at`, against a stand-in for the provider's usage endpoint.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use coqui_acp_api::Credentials;
use serde_json::{Value, json};

use super::Endpoints;
use crate::accounts::Accounts;
use crate::accounts::store::FileStore;
use crate::config::Config;

/// A directory of its own that goes with the test.
struct Dir(PathBuf);

impl Dir {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos());
        let dir = std::env::temp_dir().join(format!(
            "canoa-limits-{}-{stamp}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("a directory");
        Self(dir)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A usage endpoint on loopback that answers `status` and `body` to each request and keeps what it was sent.
struct Standin {
    address: String,
    heard: Arc<Mutex<Vec<String>>>,
}

fn standin(status: u16, body: Value) -> Standin {
    let listener = TcpListener::bind("127.0.0.1:0").expect("binds");
    let address = format!("http://{}", listener.local_addr().expect("an address"));
    let heard = Arc::new(Mutex::new(Vec::new()));
    let kept = Arc::clone(&heard);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let kept = Arc::clone(&kept);
            let text = body.to_string();
            // One thread a connection, and a client that speaks TLS is let go: held, it would stall every other probe of
            // this process's listeners (the usage tests' reader tries each by TLS too) until that reader's deadline.
            std::thread::spawn(move || {
                let mut head = Vec::new();
                let mut byte = [0_u8; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    if stream.read(&mut byte).unwrap_or(0) == 0 {
                        return;
                    }
                    if head.is_empty() && !byte[0].is_ascii_alphabetic() {
                        return;
                    }
                    head.push(byte[0]);
                }
                kept.lock()
                    .expect("lock")
                    .push(String::from_utf8_lossy(&head).into_owned());
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                    text.len()
                );
            });
        }
    });
    Standin { address, heard }
}

fn config(accounts: &str) -> Config {
    Config::from_str_in(accounts, std::path::Path::new("")).expect("parses")
}

fn accounts(dir: &Dir) -> Arc<Accounts> {
    let root = dir.0.join("accounts");
    Accounts::with_store(root.clone(), Box::new(FileStore::new(root))).expect("opens")
}

fn login_file(expires_at_ms: i64) -> String {
    json!({"claudeAiOauth": {
        "accessToken": "sk-ant-oat-canary",
        "refreshToken": "sk-ant-ort-canary",
        "expiresAt": expires_at_ms,
        "scopes": ["user:inference", "user:profile"],
    }})
    .to_string()
}

const FAR: i64 = 4_102_444_800_000;

fn claude_config() -> Config {
    config(
        "[host.accounts.work]\nkind = \"claude-login\"\nid = \"id-work\"\n\
         [host.accounts.key]\nkind = \"anthropic-api-key\"\nid = \"id-key\"\n",
    )
}

#[test]
fn a_claude_login_reads_its_windows_with_no_session_and_sends_its_token_only_to_the_endpoint() {
    let dir = Dir::new();
    let store = accounts(&dir);
    store
        .store_login(
            "id-work",
            &Credentials::from_iter([("credentials_file", login_file(FAR))]),
            None,
        )
        .expect("stored");
    let server = standin(
        200,
        json!({
            "five_hour": {"utilization": 12.5, "resets_at": "2099-01-01T05:00:00Z"},
            "seven_day": {"utilization": 40.0, "resets_at": "2099-01-05T00:00:00Z"},
        }),
    );
    let endpoints = Endpoints::default().with("claude", server.address.clone());
    let row = super::super::usage_at(&claude_config(), "work", &store, &endpoints).expect("read");
    assert_eq!(row["supported"], true, "{row}");
    assert_eq!(row["account"], "id-work", "{row}");
    let ids: Vec<&str> = row["buckets"]
        .as_array()
        .expect("buckets")
        .iter()
        .map(|bucket| bucket["id"].as_str().expect("an id"))
        .collect();
    assert_eq!(ids, ["five_hour", "seven_day"], "{row}");
    assert_eq!(row["buckets"][0]["used"], 125, "{row}");
    assert!(row["observed_at"].as_i64().unwrap_or(0) > 0, "{row}");
    let heard = server.heard.lock().expect("lock");
    assert_eq!(heard.len(), 1, "{heard:?}");
    assert!(heard[0].starts_with("GET /api/oauth/usage "), "{heard:?}");
    assert!(
        heard[0].contains("Bearer sk-ant-oat-canary"),
        "the access token is the one credential sent: {heard:?}"
    );
    assert!(
        !heard[0].contains("sk-ant-ort-canary"),
        "the refresh token never leaves: {heard:?}"
    );
    assert!(
        !row.to_string().contains("canary"),
        "the answer holds no credential: {row}"
    );
}

#[test]
fn an_api_key_behind_a_gateway_reports_the_gateways_budget_and_the_key_goes_only_there() {
    let dir = Dir::new();
    let store = accounts(&dir);
    store
        .store_login(
            "id-key",
            &Credentials::from_iter([("api_key", "sk-ant-api-canary")]),
            None,
        )
        .expect("stored");
    let server = standin(
        200,
        json!({"info": {"spend": 4.0, "max_budget": 5.0, "rpm_limit": 20}}),
    );
    let config = config(&format!(
        "[host.accounts.key]
kind = \"anthropic-api-key\"
id = \"id-key\"
         settings = {{ base_url = \"{}\" }}
",
        server.address
    ));
    let row = super::super::usage_at(&config, "key", &store, &Endpoints::default()).expect("read");
    assert_eq!(row["supported"], true, "{row}");
    assert_eq!(row["account"], "id-key", "{row}");
    let ids: Vec<&str> = row["buckets"]
        .as_array()
        .expect("buckets")
        .iter()
        .filter_map(|bucket| bucket["id"].as_str())
        .collect();
    assert_eq!(ids, ["budget", "rpm"], "{row}");
    assert_eq!(row["buckets"][0]["source"], "gateway", "{row}");
    assert_eq!(row["buckets"][0]["unit"], "micro_usd", "{row}");
    assert_eq!(row["buckets"][0]["remaining"], 1_000_000, "{row}");
    assert!(
        !row.to_string().contains("canary"),
        "the answer holds no credential: {row}"
    );
    let heard = server.heard.lock().expect("lock");
    assert!(heard.iter().any(|head| head.starts_with("GET /v1/models")));
    assert!(heard.iter().any(|head| head.starts_with("GET /key/info")));
}

#[test]
fn a_refused_login_is_told_to_renew_and_a_failed_renewal_is_said() {
    let dir = Dir::new();
    let store = accounts(&dir);
    store
        .store_login(
            "id-work",
            &Credentials::from_iter([("credentials_file", login_file(FAR))]),
            None,
        )
        .expect("stored");
    let server = standin(401, json!({"error": "no"}));
    let endpoints = Endpoints::default().with("claude", server.address.clone());
    // A `claude` program that does not exist: the renewal cannot run, which the answer says after the refusal.
    let config = config(
        "[host.agents]\n[host.accounts.work]\nkind = \"claude-login\"\nid = \"id-work\"\n\
         [host.agents.Claude]\nharness = \"claude\"\nexecutable = \"canoa-no-such-claude\"\naccounts = [\"work\"]\n",
    );
    let why = super::super::usage_at(&config, "work", &store, &endpoints).expect_err("refused");
    assert!(why.contains("renewing it failed"), "{why}");
    assert!(!why.contains("canary"), "{why}");
    assert_eq!(server.heard.lock().expect("lock").len(), 1);
}

#[test]
fn a_codex_chatgpt_login_reads_and_pi_says_it_offers_none() {
    let dir = Dir::new();
    let store = accounts(&dir);
    let auth = json!({"tokens": {"access_token": "codex-access-canary", "account_id": "acct-1"}})
        .to_string();
    store
        .store_login(
            "id-codex",
            &Credentials::from_iter([("auth_file", auth)]),
            None,
        )
        .expect("stored");
    store
        .store_login(
            "id-pi",
            &Credentials::from_iter([("auth_json", "{\"pi\": \"pi-login-canary\"}")]),
            None,
        )
        .expect("stored");
    let server = standin(
        200,
        json!({"plan_type": "plus", "rate_limit": {"allowed": true, "limit_reached": false,
            "primary_window": {"used_percent": 17, "limit_window_seconds": 18000, "reset_at": 4_102_444_800_i64},
            "secondary_window": {"used_percent": 42, "limit_window_seconds": 604_800, "reset_at": 4_102_444_800_i64}}}),
    );
    let config = config(
        "[host.accounts.chat]\nkind = \"codex-chatgpt\"\nid = \"id-codex\"\n\
         [host.accounts.pi]\nkind = \"pi-oauth\"\nid = \"id-pi\"\n",
    );
    let endpoints = Endpoints::default().with("codex", server.address.clone());
    let row = super::super::usage_at(&config, "chat", &store, &endpoints).expect("read");
    assert_eq!(row["supported"], true, "{row}");
    let ids: Vec<&str> = row["buckets"]
        .as_array()
        .expect("buckets")
        .iter()
        .map(|bucket| bucket["id"].as_str().expect("an id"))
        .collect();
    assert_eq!(ids, ["codex:five_hour", "codex:weekly"], "{row}");
    let heard = server.heard.lock().expect("lock");
    assert!(heard[0].contains("Bearer codex-access-canary"), "{heard:?}");
    assert!(heard[0].contains("acct-1"), "{heard:?}");
    drop(heard);
    let pi = super::super::usage_at(&config, "pi", &store, &endpoints).expect("pi");
    assert_eq!(pi["supported"], false, "{pi}");
    assert!(pi["unsupported_because"].is_string(), "{pi}");
}

#[test]
fn a_refused_login_is_renewed_from_the_directory_the_stored_copy_was_written_to() {
    let dir = Dir::new();
    let store = accounts(&dir);
    store
        .store_login(
            "id-work",
            &Credentials::from_iter([("credentials_file", login_file(1_000))]),
            None,
        )
        .expect("stored");
    let server = standin(401, json!({"error": "no"}));
    let endpoints = Endpoints::default().with("claude", server.address.clone());
    let config = config(
        "[host.agents.Claude]\nharness = \"claude\"\nexecutable = \"canoa-no-such-claude\"\naccounts = [\"work\"]\n\
         [host.accounts.work]\nkind = \"claude-login\"\nid = \"id-work\"\n",
    );
    let why = super::super::usage_at(&config, "work", &store, &endpoints).expect_err("refused");
    // The login was there for the account provider to look at, so what stopped it was the program, not a missing login.
    assert!(why.contains("renewing it failed"), "{why}");
    assert!(why.contains("could not be started"), "{why}");
    assert!(!why.contains("canary"), "{why}");
}

#[test]
fn antigravity_usage_read_without_session_uses_agy_usage_reader_and_records_buckets() {
    let dir = Dir::new();
    let store = accounts(&dir);
    let exe = std::env::current_exe().expect("test path");
    let debug = exe.parent().expect("deps").parent().expect("debug");
    let stdio_agent = debug
        .join("examples")
        .join(format!("stdio_agent{}", std::env::consts::EXE_SUFFIX));
    assert!(
        stdio_agent.is_file(),
        "build the examples first: cargo build -p canoa-host --examples"
    );
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("agy")
        .join("getuserstatus.json");
    let wrapper = if cfg!(windows) {
        let path = dir.0.join("agy.cmd");
        std::fs::write(
            &path,
            format!(
                "@echo off\r\n\"{}\" --agy \"{}\"\r\n",
                stdio_agent.display(),
                fixture.display()
            ),
        )
        .expect("wrapper written");
        path
    } else {
        #[cfg(unix)]
        use std::os::unix::fs::PermissionsExt as _;
        let path = dir.0.join("agy");
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\nexec \"{}\" --agy \"{}\"\n",
                stdio_agent.display(),
                fixture.display()
            ),
        )
        .expect("wrapper written");
        #[cfg(unix)]
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("wrapper executable");
        path
    };
    let quoted_exe = serde_json::to_string(&wrapper.to_string_lossy().into_owned()).expect("json");
    let cfg = config(&format!(
        "[host.agents.Antigravity]\nharness = \"antigravity\"\nexecutable = {quoted_exe}\n",
    ));
    let account = crate::config::AccountConfig::local("antigravity");
    let read =
        crate::options::usage_report_for(&cfg, ("local", &account), &store, &Endpoints::default())
            .expect("read");
    let who = read.account.expect("the agent says who is signed in");
    assert!(who.label.is_some_and(|label| !label.is_empty()));
    assert_eq!(who.plan.as_deref(), Some("Google AI Pro"));
    let report = read.report;
    assert!(report.supported, "{report:?}");
    assert!(
        report
            .buckets
            .iter()
            .any(|bucket| bucket.id.starts_with("agy:")),
        "expected agy:* quota buckets in {report:?}"
    );
}
