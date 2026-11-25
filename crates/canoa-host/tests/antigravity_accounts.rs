//! Antigravity's and Grok's accounts over the control channel, as the desktop app uses them: the account providers and the sources
//! they make listed, and Antigravity's automatic `local` account read from the login Antigravity stores, with no process
//! started: who is signed in, the plan and the quota when a login is stored, "not signed in" and its own sign-in when none is.
//! Every login is a fixture in a home of the test's own, the keyring is never read, and Google's service is a loopback
//! stand-in; every folder another agent keeps its login in is the test's own too, so no test reads the person's.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod linked;
mod sealed;
mod support;

use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::thread::JoinHandle;

use canoa_host::Link;
use linked::{Hosted, next, send};
use sealed::{App, Typed};
use serde_json::{Value, json};
use support::{example, quoted};

const EMAIL: &str = "person@example.test";
const ACCESS: &str = "fixture-access-token";
/// The page the stand-in sign-in prints, on the host Antigravity's own sign-in uses.
const SIGN_IN_PAGE: &str = "https://accounts.google.com/o/oauth2/auth?client_id=fixture";

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
        // Grok's own provider finds its login there.
        let grok = folder.join("GROK_HOME");
        std::fs::create_dir_all(&grok).expect("a folder");
        std::fs::write(grok.join("auth.json"), "{\"fixture\":true}").expect("a login");
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

/// A stand-in for `agy` that only leaves a mark: reading the account must never start it.
fn marking(dir: &Path) -> PathBuf {
    let mark = dir.join("agy-ran");
    let body = if cfg!(windows) {
        format!("echo ran> \"{}\"\r\nexit /b 1", mark.display())
    } else {
        format!("echo ran > \"{}\"\nexit 1", mark.display())
    };
    program(dir, "agy", &body)
}

/// A stand-in for `agy acp --login`: it prints the page to open, stores the fixture login a moment later, and stays running.
fn signing_in(dir: &Path) -> PathBuf {
    let fixture = dir.join("stored-login.json");
    std::fs::write(&fixture, login_json().to_string()).expect("the login to store");
    let stored = gemini(dir).join("jetski-standalone-oauth-token");
    let body = if cfg!(windows) {
        format!(
            "echo Open the URL below in your browser:\r\necho {SIGN_IN_PAGE}\r\n\
             ping -n 2 127.0.0.1 >nul\r\ncopy /y \"{}\" \"{}\" >nul\r\nping -n 60 127.0.0.1 >nul",
            fixture.display(),
            stored.display()
        )
    } else {
        format!(
            "echo 'Open the URL below in your browser:'\necho '{SIGN_IN_PAGE}'\nsleep 1\ncp \"{}\" \"{}\"\nsleep 60",
            fixture.display(),
            stored.display()
        )
    };
    program(dir, "agy", &body)
}

/// The test's own Antigravity home.
fn home(dir: &Path) -> PathBuf {
    dir.join("agy-home")
}

fn gemini(dir: &Path) -> PathBuf {
    home(dir).join(".gemini")
}

/// agy's stored login, as it keeps it: a token that never expires (Go's zero time), the email and the plan it signed in to.
fn login_json() -> Value {
    json!({
        "token": {"access_token": ACCESS, "token_type": "Bearer", "refresh_token": "fixture-refresh-token",
                  "expiry": "0001-01-01T00:00:00Z"},
        "auth_method": "consumer",
        "id_token_email": EMAIL,
        "project_id": "fixture-project",
        "user_tier": "g1-pro-tier",
        "tier_display_name": "Stored Plan"
    })
}

/// A host with an Antigravity entry that runs `build`'s program and reads its login from the test's home, from Google's stand-in
/// at `google`, never from the keyring.
fn host_running(google: &Google, login: bool, build: impl FnOnce(&Path) -> PathBuf) -> Hosted {
    isolate();
    let base = google.base();
    Hosted::start_with_setup(
        |dir| {
            std::fs::create_dir_all(gemini(dir)).expect("a home");
            if login {
                std::fs::write(
                    gemini(dir).join("jetski-standalone-oauth-token"),
                    login_json().to_string(),
                )
                .expect("the stored login");
            }
            let program = quoted(&build(dir).to_string_lossy());
            let home = quoted(&home(dir).to_string_lossy());
            format!(
                "[secrets]\naccounts = \"file\"\n\n[host.agents.Antigravity]\nharness = \"antigravity\"\nexecutable = {program}\n\
                 account_settings = {{ executable = {program}, home = {home}, keyring = false, login_poll_ms = 100, \
                 token_url = \"{base}/token\", userinfo_url = \"{base}/userinfo\", service = \"{base}\" }}\n"
            )
        },
        &[],
    )
}

fn app_of(host: &Hosted) -> App {
    let secret = host.local.link_secret().expect("the host minted a secret");
    App::new(&secret[..])
}

// ---------------------------------------------------------------------------------------------
// The stand-in for Google's userinfo and Cloud Code service.

struct Google {
    address: SocketAddr,
    paths: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Google {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
        let address = listener.local_addr().expect("its address");
        let stop = Arc::new(AtomicBool::new(false));
        let paths = Arc::new(Mutex::new(Vec::new()));
        let (stopping, seen) = (Arc::clone(&stop), Arc::clone(&paths));
        let thread = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if stopping.load(Ordering::SeqCst) {
                    return;
                }
                let Ok(mut stream) = stream else { continue };
                let Some((method, path, authorization)) = request(&mut stream) else {
                    continue;
                };
                seen.lock().expect("not poisoned").push(path.clone());
                let (status, body) = answer(&method, &path, authorization.as_deref());
                let body = body.to_string();
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.flush();
            }
        });
        Self {
            address,
            paths,
            stop,
            thread: Some(thread),
        }
    }

    fn base(&self) -> String {
        format!("http://{}", self.address)
    }

    fn paths(&self) -> Vec<String> {
        self.paths.lock().expect("not poisoned").clone()
    }
}

impl Drop for Google {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.address);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The request's method, path and `Authorization`, its body read and dropped.
fn request(stream: &mut TcpStream) -> Option<(String, String, Option<String>)> {
    let mut head = Vec::new();
    let mut byte = [0_u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte).ok()? == 0 {
            return None;
        }
        head.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&head).into_owned();
    let mut lines = head.lines();
    let mut first = lines.next()?.split(' ');
    let (method, path) = (first.next()?.to_owned(), first.next()?.to_owned());
    let (mut length, mut authorization) = (0, None);
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("content-length") {
            length = value.trim().parse().ok()?;
        } else if name.eq_ignore_ascii_case("authorization") {
            authorization = Some(value.trim().to_owned());
        }
    }
    let mut body = vec![0_u8; length];
    stream.read_exact(&mut body).ok()?;
    Some((method, path, authorization))
}

fn answer(method: &str, path: &str, authorization: Option<&str>) -> (u16, Value) {
    if authorization != Some(&format!("Bearer {ACCESS}")) {
        return (401, json!({"error": {"code": 401}}));
    }
    match (method, path) {
        ("GET", "/userinfo") => (200, json!({"id": "1", "email": EMAIL, "name": "A Person"})),
        ("POST", "/v1internal:loadCodeAssist") => (
            200,
            json!({
                "currentTier": {"id": "free-tier", "name": "Free"},
                "paidTier": {"id": "g1-pro-tier", "name": "Google AI Pro"},
                "cloudaicompanionProject": "fixture-project"
            }),
        ),
        ("POST", "/v1internal:fetchAvailableModels") => (
            200,
            json!({"models": {
                "gemini-fixture-high": {"displayName": "Gemini Fixture", "quotaInfo": {"remainingFraction": 0.5, "resetTime": "2099-01-01T00:00:00Z"}},
                "gemini-fixture-low": {"displayName": "Gemini Fixture Low", "quotaInfo": {"remainingFraction": 1, "resetTime": "2099-01-08T00:00:00Z"}}
            }}),
        ),
        _ => (404, json!({})),
    }
}

// ---------------------------------------------------------------------------------------------

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
            // A stored login in the test's own home, read with no keyring: the operating system's is never asked.
            std::fs::create_dir_all(gemini(dir)).expect("a home");
            std::fs::write(
                gemini(dir).join("jetski-standalone-oauth-token"),
                login_json().to_string(),
            )
            .expect("the stored login");
            format!(
                "[secrets]\naccounts = \"file\"\n\n\
                 [host.agents.Grok]\nharness = \"grok\"\nexecutable = {}\n\n\
                 [host.agents.Antigravity]\nharness = \"antigravity\"\nexecutable = {}\n\
                 account_settings = {{ home = {}, keyring = false }}\n",
                quoted(&grok.to_string_lossy()),
                quoted(&agy.to_string_lossy()),
                quoted(&home(dir).to_string_lossy()),
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
        let catalog = plain_call(
            &link,
            2,
            "accounts/catalog",
            json!({"provider": format!("{id}_accounts")}),
        )
        .expect("catalog");
        assert!(catalog["kinds"].is_array(), "{id}: {catalog}");
        assert!(catalog["logins"].is_array(), "{id}: {catalog}");
    }
    let accounts = plain_call(&link, 3, "accounts/list", json!({})).expect("accounts");
    let rows = accounts["accounts"].as_array().expect("accounts array");
    for harness in ["grok", "antigravity"] {
        assert!(
            rows.iter()
                .any(|row| row["agent"] == harness && row["name"] == "local"),
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
fn a_stored_antigravity_login_is_the_local_account_with_its_email_plan_and_quota_and_nothing_is_started()
 {
    let google = Google::start();
    let host = host_running(&google, true, marking);
    let ran = host.scratch.0.join("agy-ran");
    let link = host.local.open("@control").expect("a link");

    let accounts = plain_call(&link, 1, "accounts/list", json!({})).expect("accounts");
    assert!(
        accounts["accounts"]
            .as_array()
            .expect("accounts array")
            .iter()
            .any(|row| row["agent"] == "antigravity" && row["name"] == "local"),
        "{accounts}"
    );

    let read = plain_call(
        &link,
        2,
        "usage/read",
        json!({"account": "local", "harness": "antigravity"}),
    )
    .expect("usage/read");
    let row = &read["account"];
    assert_eq!(row["supported"], true, "{read}");
    assert_eq!(row["details"]["label"], EMAIL, "{read}");
    assert_eq!(row["details"]["plan"], "Google AI Pro", "{read}");
    let buckets = row["buckets"].as_array().expect("buckets");
    assert!(
        buckets.iter().any(|bucket| bucket["id"]
            .as_str()
            .is_some_and(|id| id.starts_with("agy:"))),
        "the row has the quota: {read}"
    );
    assert!(
        google
            .paths()
            .iter()
            .any(|path| path == "/v1internal:fetchAvailableModels"),
        "{:?}",
        google.paths()
    );

    // Signing in with a login already stored runs nothing: the login is the account.
    let catalog = plain_call(
        &link,
        3,
        "accounts/catalog",
        json!({"provider": "antigravity_accounts"}),
    )
    .expect("catalog");
    let logins = catalog["logins"].as_array().expect("logins");
    assert_eq!(logins.len(), 1, "{catalog}");
    assert_eq!(logins[0]["id"], "sign_in", "{catalog}");
    assert_eq!(logins[0]["title"], "Sign in to Antigravity", "{catalog}");
    let mut app = app_of(&host);
    let (_, added) = app.drive(
        &link,
        4,
        "accounts/add",
        &json!({"name": "agy", "provider": "antigravity_accounts", "method": "sign_in"}),
        &Typed::Leaves,
    );
    let added = added.expect("a stored login signs in at once");
    assert_eq!(added["account"]["name"], "local", "{added}");
    assert_eq!(added["account"]["kind"], "harness-login", "{added}");
    assert_eq!(added["account"]["label"], EMAIL, "{added}");
    assert_eq!(added["account"]["checked"], true, "{added}");

    assert!(!ran.exists(), "agy was started to read or sign in");
    host.stop();
}

#[test]
fn with_no_antigravity_login_the_account_says_so_and_its_own_sign_in_stores_one() {
    let google = Google::start();
    let host = host_running(&google, false, signing_in);
    let link = host.local.open("@control").expect("a link");

    // The provider says no sign-in is stored, so there is no account `local` to read, nor one listed.
    let refused = plain_call(
        &link,
        1,
        "usage/read",
        json!({"account": "local", "harness": "antigravity"}),
    )
    .expect_err("no login to read");
    assert!(
        refused.contains("nobody is signed in to antigravity") && !refused.contains("fixture"),
        "{refused}"
    );
    let listed = plain_call(&link, 4, "accounts/list", json!({})).expect("accounts");
    assert!(
        !listed["accounts"]
            .as_array()
            .expect("accounts array")
            .iter()
            .any(|row| row["agent"] == "antigravity" && row["name"] == "local"),
        "{listed}"
    );
    assert!(google.paths().is_empty(), "{:?}", google.paths());
    let stored = gemini(&host.scratch.0).join("jetski-standalone-oauth-token");
    assert!(!stored.exists(), "agy was started to read the account");

    let mut app = app_of(&host);
    let (progress, added) = app.drive(
        &link,
        2,
        "accounts/add",
        &json!({"name": "agy", "provider": "antigravity_accounts", "method": "sign_in"}),
        &Typed::Leaves,
    );
    let added = added.expect("the stand-in stores a login, so the sign-in succeeds");
    assert_eq!(added["account"]["name"], "local", "{added}");
    assert_eq!(added["account"]["label"], EMAIL, "{added}");
    assert!(
        progress
            .iter()
            .any(|each| each["open_url"]["url"] == SIGN_IN_PAGE),
        "the person is offered the sign-in's page: {progress:?}"
    );
    assert!(stored.exists(), "the sign-in ran the program");

    // The failed reading was not kept: the account is read again, now signed in.
    let read = plain_call(
        &link,
        3,
        "usage/read",
        json!({"account": "local", "harness": "antigravity"}),
    )
    .expect("usage/read");
    assert_eq!(read["account"]["supported"], true, "{read}");
    assert_eq!(read["account"]["details"]["label"], EMAIL, "{read}");
    host.stop();
}

#[test]
fn an_antigravity_sign_in_that_ends_with_no_login_stored_adds_nothing() {
    let google = Google::start();
    let host = host_running(&google, false, |dir| {
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
        &json!({"name": "agy", "provider": "antigravity_accounts", "method": "sign_in"}),
        &Typed::Leaves,
    );
    let error = outcome.expect_err("no login was stored");
    assert!(error.to_string().contains("no login is stored"), "{error}");
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

/// A host with an agent entry made as the Antigravity source preset makes one (`usage = "antigravity"`), whose program is the
/// `stdio_agent` example playing `agy` and logging what it is asked to `log`, reading its stored login from the test's home and
/// Google's stand-in at `base`, never from the keyring. `login` stores the fixture login there.
fn entry_host(base: &str, login: bool, log: &Path) -> Hosted {
    isolate();
    // SAFETY: the agent stand-in reads the variable from the host's environment; the tests that set it hold `LOG` for their whole run.
    unsafe { std::env::set_var("CANOA_TEST_AGY_LOG", log) };
    Hosted::start_with_setup(
        |dir| {
            std::fs::create_dir_all(gemini(dir)).expect("a home");
            if login {
                std::fs::write(
                    gemini(dir).join("jetski-standalone-oauth-token"),
                    login_json().to_string(),
                )
                .expect("the stored login");
            }
            let program = quoted(&example("stdio_agent", false).to_string_lossy());
            let fixture = quoted(
                &Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/agy/getuserstatus.json")
                    .to_string_lossy(),
            );
            let home = quoted(&home(dir).to_string_lossy());
            format!(
                "[host.agents.Agy]\nprogram = {program}\nargs = [\"--agy\", {fixture}]\nlisten = \"127.0.0.1:0\"\nusage = \"antigravity\"\n\
                 usage_settings = {{ home = {home}, keyring = false, token_url = \"{base}/token\", \
                 userinfo_url = \"{base}/userinfo\", service = \"{base}\" }}\n"
            )
        },
        &[],
    )
}

/// Serializes the tests that set the stand-in's log variable.
static LOG: Mutex<()> = Mutex::new(());

/// What the agent stand-ins logged: one line per event, led by the process id.
fn logged(log: &Path) -> String {
    std::fs::read_to_string(log).unwrap_or_default()
}

/// Whether any agent opened a session: the connection's own agent never does here, so one that did was started to read.
fn opened_a_session(log: &str) -> bool {
    log.lines()
        .any(|line| line.ends_with(" new") || line.contains(" cwd "))
}

/// Opens a link to the `Agy` entry and initializes it, with no session.
fn initialized(host: &Hosted) -> Link {
    let link = host.local.open("Agy").expect("a link");
    linked::ask(
        &link,
        1,
        "initialize",
        json!({"protocolVersion": 1, "clientCapabilities": {}}),
    )
    .expect("initialize is answered");
    link
}

#[test]
fn an_agent_entrys_usage_is_read_from_the_stored_login_with_no_session_and_no_private_agent() {
    let _serial = LOG
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let google = Google::start();
    let log = std::env::temp_dir().join(format!("canoa-agy-entry-log-{}", std::process::id()));
    let _ = std::fs::remove_file(&log);
    let host = entry_host(&google.base(), true, &log);
    let link = initialized(&host);
    // No session is open: the host answers from the login, and starts nothing to read it.
    let (report, _) = linked::ask(&link, 2, "_coqui_acp/usage", json!({})).expect("answered");
    assert_eq!(report["supported"], true, "{report}");
    assert_eq!(report["account"], "Agy", "{report}");
    let buckets = report["buckets"].as_array().expect("buckets");
    assert_eq!(buckets.len(), 2, "{report}");
    for bucket in buckets {
        assert!(
            bucket["id"]
                .as_str()
                .is_some_and(|id| id.starts_with("agy:")),
            "{bucket}"
        );
        assert_eq!(bucket["source"], "reported", "{bucket}");
    }
    assert!(
        google
            .paths()
            .iter()
            .any(|path| path == "/v1internal:fetchAvailableModels"),
        "{:?}",
        google.paths()
    );
    let text = report.to_string();
    assert!(
        !text.contains(EMAIL) && !text.contains(ACCESS),
        "the login's details reached the client"
    );
    let seen = logged(&log);
    assert!(!opened_a_session(&seen), "an agent read the quota: {seen}");
    host.stop();
    let _ = std::fs::remove_file(&log);
}

#[test]
fn an_agent_entrys_usage_with_no_login_stored_is_empty_and_starts_no_private_agent() {
    let _serial = LOG
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let google = Google::start();
    let log = std::env::temp_dir().join(format!("canoa-agy-entry-none-{}", std::process::id()));
    let _ = std::fs::remove_file(&log);
    let host = entry_host(&google.base(), false, &log);
    let link = initialized(&host);
    let (report, _) = linked::ask(&link, 2, "_coqui_acp/usage", json!({})).expect("answered");
    assert_eq!(report["buckets"], json!([]), "{report}");
    assert!(google.paths().is_empty(), "{:?}", google.paths());
    let seen = logged(&log);
    assert!(!opened_a_session(&seen), "an agent read the quota: {seen}");
    host.stop();
    let _ = std::fs::remove_file(&log);
}
