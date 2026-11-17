//! Accounts in the built `canoa`: an account made by signing in to a stand-in for Claude Code, held by one connection at a
//! time, its renewals stored and its losses marked, with a canary credential followed through everything the host writes.
//! The stores are the file store here, in a directory of the test's; the operating system's has its own test beside the code.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod support;

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use canoa_host::ACCOUNT_META;
use serde_json::{Value, json};
use support::{Host, Scratch, WAIT, cwd, example, host_binary, quoted, with_link};

const CANARY: &str = "CANARY";

/// A scratch directory holding a stand-in named `claude`, a canoa.cfg with one entry and an account made by signing in.
struct Fixture {
    scratch: Scratch,
    config: PathBuf,
    record: PathBuf,
}

impl Fixture {
    /// A canoa.cfg with the entry `Claude` and no account yet.
    fn bare() -> Self {
        let scratch = Scratch::new();
        let claude = scratch
            .0
            .join(format!("claude{}", std::env::consts::EXE_SUFFIX));
        std::fs::copy(example("harness_standin", false), &claude).expect("the stand-in is copied");
        let config = scratch.0.join("canoa.cfg");
        std::fs::write(
            &config,
            format!(
                "[host]\naccount_store = \"file\"\n\n[host.agents.Claude]\nharness = \"claude\"\nexecutable = {}\naccount_settings = {{ renew_poll_ms = 100 }}\n",
                quoted(&claude.to_string_lossy())
            ),
        )
        .expect("canoa.cfg is written");
        let record = scratch.0.join("record.jsonl");
        Self {
            scratch,
            config,
            record,
        }
    }

    /// As [`Fixture::bare`], with the account `work` signed in by the stand-in and offered by the entry.
    fn signed_in() -> Self {
        let fixture = Self::unlisted();
        let cfg = std::fs::read_to_string(&fixture.config).expect("canoa.cfg");
        std::fs::write(
            &fixture.config,
            cfg.replace(
                "account_settings = { renew_poll_ms = 100 }",
                "accounts = [\"work\"]\naccount_settings = { renew_poll_ms = 100 }",
            ),
        )
        .expect("the entry offers the account");
        fixture
    }

    /// As [`Fixture::signed_in`], with the entry listing no account.
    fn unlisted() -> Self {
        let fixture = Self::bare();
        let added = fixture.canoa(
            &[
                "accounts",
                "add",
                "work",
                "--harness",
                "claude",
                "--method",
                "subscription",
            ],
            Some("123456\n"),
        );
        assert!(added.status.success(), "{}", text(&added.stderr));
        assert_eq!(text(&added.stdout).trim(), "added work (claude-login)");
        fixture
    }

    fn home(&self) -> PathBuf {
        self.scratch.0.join("home")
    }

    /// `canoa ARGS --config canoa.cfg` on this fixture's home, with `input` on its standard input.
    fn canoa(&self, args: &[&str], input: Option<&str>) -> Output {
        self.canoa_on(args, input, false)
    }

    /// As [`Fixture::canoa`]; `busless` leaves the process no session bus, as a service has none.
    fn canoa_on(&self, args: &[&str], input: Option<&str>, busless: bool) -> Output {
        let mut command = Command::new(host_binary());
        if busless {
            command
                .env_remove("DBUS_SESSION_BUS_ADDRESS")
                .env("XDG_RUNTIME_DIR", &self.scratch.0);
        }
        let mut child = command
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
        if let Some(input) = input {
            // A process that ends before it reads its input (it refused something first) closes the pipe: that is
            // its answer, whose status and standard error the caller's assertion prints, not a failure to write.
            match stdin.write_all(input.as_bytes()) {
                Ok(()) => {}
                Err(failure) if failure.kind() == std::io::ErrorKind::BrokenPipe => {}
                Err(failure) => {
                    let _ = child.kill();
                    let ended = child.wait_with_output().expect("canoa ends");
                    panic!(
                        "input not written: {failure}; canoa ended {:?}: {}",
                        ended.status,
                        text(&ended.stderr)
                    );
                }
            }
        }
        drop(stdin);
        child.wait_with_output().expect("canoa ends")
    }

    /// The accounts, as `accounts list --format json` prints them.
    fn listed(&self) -> Vec<Value> {
        let listed = self.canoa(&["accounts", "list", "--format", "json"], None);
        assert!(listed.status.success(), "{}", text(&listed.stderr));
        text(&listed.stdout)
            .lines()
            .map(|line| serde_json::from_str(line).expect("one JSON object per line"))
            .collect()
    }

    fn account(&self) -> Value {
        self.listed().remove(0)
    }

    /// Serves the entry, logging to `host.log` in the scratch directory.
    fn serve(self) -> (Host, PathBuf, PathBuf, PathBuf) {
        let config = std::fs::read_to_string(&self.config).expect("canoa.cfg");
        let record = self.record.clone();
        let env = vec![
            ("STANDIN_RECORD", record.clone().into_os_string()),
            ("STANDIN_ENV", "ANTHROPIC_API_KEY".into()),
            ("ANTHROPIC_API_KEY", "a-key-the-account-must-clear".into()),
            (
                "CLAUDE_CODE_OAUTH_TOKEN",
                "a-token-the-account-must-clear".into(),
            ),
        ];
        let (host, log) = Host::start_logged(self.scratch, &config, 1, &env);
        let cfg = host.scratch.0.join("canoa.cfg");
        (host, log, record, cfg)
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn records(file: &Path) -> Vec<Value> {
    std::fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).expect("one JSON line"))
        .collect()
}

/// The login file a connection's provider placed for the account, wherever its directory is under the home.
fn login_file(home: &Path) -> Option<PathBuf> {
    std::fs::read_dir(home.join("accounts"))
        .ok()?
        .flatten()
        .map(|entry| entry.path().join("state").join(".credentials.json"))
        .find(|path| path.exists())
}

fn rotated(number: u32, lasts_hours: u64) -> String {
    let ends = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("a time")
        .as_millis()
        + u128::from(lasts_hours) * 3_600_000;
    json!({"claudeAiOauth": {
        "accessToken": format!("CANARY-access-{number}"),
        "refreshToken": format!("CANARY-refresh-{number}"),
        "expiresAt": u64::try_from(ends).expect("fits"),
        "scopes": ["user:inference"],
        "subscriptionType": "max",
    }})
    .to_string()
}

fn waits_for(what: &str, mut done: impl FnMut() -> bool) {
    let until = Instant::now() + WAIT;
    while !done() {
        assert!(Instant::now() < until, "{what}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn initialize_as_work(
    rpc: &support::Rpc<'_, coqui_transport_web::websocket::Channel>,
) -> Result<Value, Value> {
    rpc.request(
        "initialize",
        json!({"protocolVersion": 1, "clientCapabilities": {}, "_meta": {ACCOUNT_META: "work"}}),
    )
}

#[test]
fn signing_in_makes_an_account_whose_credential_is_in_the_store_and_not_in_canoa_cfg() {
    let fixture = Fixture::bare();
    let added = fixture.canoa(
        &[
            "accounts",
            "add",
            "work",
            "--harness",
            "claude",
            "--method",
            "subscription",
        ],
        Some("123456\n"),
    );
    let (out, err) = (text(&added.stdout), text(&added.stderr));
    assert!(added.status.success(), "{err}");
    assert_eq!(out.trim(), "added work (claude-login)");
    // The page the CLI printed is shown to the person, and the code they typed is not.
    assert!(
        err.contains("https://claude.com/cai/oauth/authorize"),
        "{err}"
    );
    assert!(!err.contains("123456"), "{err}");

    let cfg = std::fs::read_to_string(&fixture.config).expect("canoa.cfg");
    assert!(
        cfg.contains(
            "[host.accounts.work]\nharness = \"claude\"\nkind = \"claude-login\"\nid = \""
        ),
        "{cfg}"
    );
    assert!(!cfg.contains(CANARY) && !cfg.contains("123456"), "{cfg}");

    let account = fixture.account();
    assert_eq!(account["name"], "work");
    assert_eq!(account["state"], "signed in");
    assert_eq!(account["revision"], 1);
    // Nothing of the sign-in's directory is left, and the stored record holds no plaintext.
    let accounts = fixture.home().join("accounts");
    assert!(
        !accounts.join("login").exists()
            || std::fs::read_dir(accounts.join("login"))
                .expect("lists")
                .next()
                .is_none()
    );
    for file in std::fs::read_dir(&accounts).expect("lists").flatten() {
        if file.path().is_file() {
            let bytes = std::fs::read(file.path()).expect("reads");
            assert!(
                !String::from_utf8_lossy(&bytes).contains(CANARY),
                "{}",
                file.path().display()
            );
        }
    }

    // A name that exists, and a second sign-in's refusal, change nothing.
    let again = fixture.canoa(
        &[
            "accounts",
            "add",
            "work",
            "--harness",
            "claude",
            "--method",
            "subscription",
        ],
        Some("1\n"),
    );
    assert!(!again.status.success());
    assert_eq!(fixture.listed().len(), 1);
}

#[test]
fn a_code_the_cli_refuses_makes_no_account() {
    let fixture = Fixture::bare();
    let claude_dir = fixture.scratch.0.clone();
    let _ = claude_dir;
    // An empty code is refused by the stand-in as the real command line refuses one.
    let added = fixture.canoa(
        &[
            "accounts",
            "add",
            "work",
            "--harness",
            "claude",
            "--method",
            "subscription",
        ],
        Some("\n"),
    );
    assert!(!added.status.success());
    assert!(fixture.listed().is_empty());
    let cfg = std::fs::read_to_string(&fixture.config).expect("canoa.cfg");
    assert!(!cfg.contains("accounts."), "{cfg}");
}

#[test]
fn the_method_is_named_from_the_providers_list() {
    let fixture = Fixture::bare();
    let added = fixture.canoa(&["accounts", "add", "work", "--harness", "claude"], None);
    assert_eq!(added.status.code(), Some(2));
    let err = text(&added.stderr);
    assert!(
        err.contains("subscription") && err.contains("api-key"),
        "{err}"
    );
}

#[test]
fn a_rotating_account_is_held_by_one_connection_and_free_again_when_it_ends() {
    let fixture = Fixture::signed_in();
    let (host, _log, _record, _cfg) = fixture.serve();
    let client = support::key();
    host.pair(&client);
    let dir = cwd(&host.scratch);
    let (ready, ready_rx) = mpsc::channel();
    let (release, release_rx) = mpsc::channel::<()>();
    let (url, peer) = (host.url("Claude").to_owned(), host.peer);
    std::thread::scope(|scope| {
        let (client, dir, url) = (&client, &dir, &url);
        let first = scope.spawn(move || {
            with_link(url, client, peer, |rpc| {
                let initialized = initialize_as_work(rpc).expect("the first connection holds it");
                assert_eq!(
                    initialized["_meta"][ACCOUNT_META],
                    json!({"name": "work", "kind": "claude-login"}),
                    "{initialized}"
                );
                rpc.request("session/new", json!({"cwd": dir, "mcpServers": []}))
                    .expect("a session opens");
                // A second session on a connection that holds a rotating account is refused, naming why.
                let second = rpc
                    .request("session/new", json!({"cwd": dir, "mcpServers": []}))
                    .expect_err("a second session");
                assert_eq!(second["code"], -32602, "{second}");
                assert!(
                    second["message"]
                        .as_str()
                        .is_some_and(|m| m.contains("one session at a time")),
                    "{second}"
                );
                ready.send(()).expect("told");
                release_rx.recv_timeout(WAIT * 2).expect("released");
            })
        });
        ready_rx
            .recv_timeout(WAIT)
            .expect("the first connection is up");
        // Another connection choosing it is refused before anything starts.
        let refused = with_link(host.url("Claude"), client, host.peer, |rpc| {
            initialize_as_work(rpc).expect_err("the account is held")
        })
        .expect("served");
        assert_eq!(refused["code"], -32602, "{refused}");
        let why = refused["message"].as_str().expect("a reason");
        assert!(why.contains("holds it") && !why.contains(CANARY), "{why}");
        release.send(()).expect("release");
        first.join().expect("the first ended").expect("served");
    });
    // Once it has ended the account is free: the provider is given a moment to say it has let go of it.
    waits_for(
        "the account is free after the first connection ends",
        || {
            with_link(host.url("Claude"), &client, host.peer, |rpc| {
                initialize_as_work(rpc).is_ok()
            })
            .unwrap_or(false)
        },
    );
}

#[test]
fn a_rotated_login_is_stored_and_a_deleted_one_marks_the_account_for_a_sign_in() {
    let fixture = Fixture::signed_in();
    let probe = fixture.canoa(&["accounts", "list"], None);
    assert!(text(&probe.stdout).contains("signed in"));
    let (host, _log, record, cfg) = fixture.serve();
    let client = support::key();
    host.pair(&client);
    let dir = cwd(&host.scratch);
    let list = || -> Value {
        let out = Command::new(host_binary())
            .args(["accounts", "list", "--format", "json", "--config"])
            .arg(&cfg)
            .env("COQUI_AGENTIC_PEER_HOME", host.home())
            .output()
            .expect("canoa runs");
        serde_json::from_str(text(&out.stdout).lines().next().expect("an account")).expect("JSON")
    };
    with_link(host.url("Claude"), &client, host.peer, |rpc| {
        initialize_as_work(rpc).expect("held");
        rpc.request("session/new", json!({"cwd": dir, "mcpServers": []}))
            .expect("a session opens");
        let started = records(&record);
        assert_eq!(
            started[0]["login"], "1",
            "the harness started on the stored login: {started:?}"
        );
        let names = started[0]["env_names"].as_array().expect("names");
        for credential in [
            "ANTHROPIC_API_KEY",
            "CLAUDE_CODE_OAUTH_TOKEN",
            "ANTHROPIC_AUTH_TOKEN",
        ] {
            assert!(
                !names.contains(&json!(credential)),
                "{credential} reached the harness"
            );
        }
        let config = started[0]["config"]
            .as_str()
            .expect("its configuration directory");
        assert!(Path::new(config).starts_with(host.home()), "{config}");

        // The harness refreshes its login while the connection runs: the newer copy is stored.
        let file = login_file(&host.home()).expect("the login file the provider placed");
        std::fs::write(&file, rotated(2, 16)).expect("rotated");
        waits_for("the rotated login is stored", || list()["revision"] == 2);
        assert_eq!(list()["state"], "in use");

        // A login older than the stored one, as an older copy coming back would be, replaces nothing.
        std::fs::write(&file, rotated(1, 1)).expect("an older login");
        std::thread::sleep(Duration::from_millis(1500));
        assert_eq!(list()["revision"], 2, "{}", list());

        // The harness loses its login: the account needs a sign-in, and the copy the host holds stays.
        std::fs::remove_file(&file).expect("deleted");
        waits_for("the loss is marked", || list()["state"] == "needs sign-in");
    })
    .expect("served");
    waits_for("the next connection can hold the account", || {
        with_link(host.url("Claude"), &client, host.peer, |rpc| {
            if initialize_as_work(rpc).is_err() {
                return false;
            }
            rpc.request("session/new", json!({"cwd": dir, "mcpServers": []}))
                .is_ok()
        })
        .unwrap_or(false)
    });
    let starts: Vec<String> = records(&record)
        .iter()
        .filter_map(|line| line["login"].as_str().map(str::to_owned))
        .collect();
    assert_eq!(
        starts.last().map(String::as_str),
        Some("2"),
        "the next start used the stored copy, not the lost one: {starts:?}"
    );
    assert_eq!(list()["state"], "needs sign-in");
    // The files the provider placed went with the connections.
    assert!(
        login_file(&host.home()).is_none() || {
            let file = login_file(&host.home()).expect("a file");
            std::fs::read_to_string(file).is_ok_and(|t| t.contains("CANARY-access-2"))
        }
    );
}

#[test]
fn a_canary_credential_is_in_no_configuration_log_argument_or_message() {
    let fixture = Fixture::signed_in();
    let (host, log, record, cfg) = fixture.serve();
    let client = support::key();
    host.pair(&client);
    let dir = cwd(&host.scratch);
    let mut seen = Vec::new();
    with_link(host.url("Claude"), &client, host.peer, |rpc| {
        seen.push(initialize_as_work(rpc).expect("held").to_string());
        seen.push(
            rpc.request("session/new", json!({"cwd": dir, "mcpServers": []}))
                .expect("a session opens")
                .to_string(),
        );
        let file = login_file(&host.home()).expect("the login file");
        assert!(
            std::fs::read_to_string(&file)
                .expect("reads")
                .contains(CANARY)
        );
        std::fs::write(&file, rotated(2, 16)).expect("rotated");
        std::thread::sleep(Duration::from_millis(1500));
        seen.extend(rpc.updates.borrow().iter().map(Value::to_string));
    })
    .expect("served");
    waits_for("the connection is over", || {
        !login_file(&host.home()).is_some_and(|f| f.exists())
    });
    let log = std::fs::read_to_string(&log).expect("the host's log");
    let listed = Command::new(host_binary())
        .args(["accounts", "list", "--config"])
        .arg(&cfg)
        .env("COQUI_AGENTIC_PEER_HOME", host.home())
        .output()
        .expect("runs");
    let record = std::fs::read_to_string(&record).expect("the stand-in's record");
    let config = std::fs::read_to_string(&cfg).expect("canoa.cfg");
    // Everything the host left on disk, the account store and its directories included (the host has no ledger of its own:
    // the session ledger is the app's, and takes only what the app is shown, which the messages above are).
    let mut on_disk = String::new();
    let mut found = vec![host.scratch.0.clone()];
    while let Some(path) = found.pop() {
        for entry in std::fs::read_dir(&path).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                found.push(path);
            } else if path.extension().is_some_and(|e| e == "exe")
                || path.file_name().is_some_and(|n| n == "claude")
            {
                // The stand-in's own program holds the canary as text, to write it into a login.
            } else if let Ok(bytes) = std::fs::read(&path) {
                assert!(
                    !String::from_utf8_lossy(&bytes).contains(CANARY),
                    "the canary is in {}",
                    path.display()
                );
                on_disk.push_str(&path.to_string_lossy());
            }
        }
    }
    assert!(
        on_disk.contains("accounts"),
        "the walk found the accounts directory: {on_disk}"
    );
    for (place, content) in [
        ("canoa.cfg", config),
        ("the host's log", log),
        ("the harness's arguments and environment names", record),
        ("what the client saw", seen.join("\n")),
        (
            "accounts list",
            text(&listed.stdout) + &text(&listed.stderr),
        ),
    ] {
        assert!(!content.contains(CANARY), "the canary is in {place}");
    }
}

#[test]
fn a_credential_written_in_canoa_cfg_is_refused_naming_its_key() {
    // A key of the account table itself.
    let fixture = Fixture::bare();
    let cfg = std::fs::read_to_string(&fixture.config).expect("canoa.cfg");
    std::fs::write(
        &fixture.config,
        format!("{cfg}\n[host.accounts.work]\nharness = \"claude\"\nkind = \"anthropic-api-key\"\nid = \"x\"\napi_key = \"CANARY-in-clear\"\n"),
    )
    .expect("written");
    let refused = fixture.canoa(&["accounts", "list"], None);
    let err = text(&refused.stderr);
    assert_eq!(refused.status.code(), Some(2), "{err}");
    assert!(err.contains("api_key") && !err.contains(CANARY), "{err}");

    // A credential among the settings, which the provider's own schema marks as one.
    let fixture = Fixture::bare();
    let cfg = std::fs::read_to_string(&fixture.config).expect("canoa.cfg");
    let cfg = cfg.replace(
        "account_settings = { renew_poll_ms = 100 }",
        "accounts = [\"work\"]\naccount_settings = { renew_poll_ms = 100 }",
    );
    std::fs::write(
        &fixture.config,
        format!("{cfg}\n[host.accounts.work]\nharness = \"claude\"\nkind = \"anthropic-api-key\"\nid = \"x\"\nsettings = {{ api_key = \"CANARY-in-clear\" }}\n"),
    )
    .expect("written");
    let served = Command::new(host_binary())
        .args(["serve", "--config"])
        .arg(&fixture.config)
        .env("COQUI_AGENTIC_PEER_HOME", fixture.home())
        .stdin(Stdio::null())
        .output()
        .expect("canoa runs");
    let err = text(&served.stderr);
    assert!(!served.status.success(), "{err}");
    assert!(
        err.contains("api_key") && err.contains("credential") && !err.contains(CANARY),
        "{err}"
    );
}

#[test]
fn an_account_is_removed_with_its_credential_and_its_block() {
    let fixture = Fixture::signed_in();
    // An entry that offers it keeps it.
    let kept = fixture.canoa(&["accounts", "remove", "work"], None);
    assert!(!kept.status.success());
    assert!(
        text(&kept.stderr).contains("offers work"),
        "{}",
        text(&kept.stderr)
    );
    let cfg = std::fs::read_to_string(&fixture.config).expect("canoa.cfg");
    std::fs::write(&fixture.config, cfg.replace("accounts = [\"work\"]\n", ""))
        .expect("not offered");

    let removed = fixture.canoa(&["accounts", "remove", "work"], None);
    assert!(removed.status.success(), "{}", text(&removed.stderr));
    assert!(fixture.listed().is_empty());
    let cfg = std::fs::read_to_string(&fixture.config).expect("canoa.cfg");
    assert!(
        !cfg.contains("accounts.work") && cfg.contains("[host.agents.Claude]"),
        "{cfg}"
    );
    let accounts = fixture.home().join("accounts");
    let leftovers: Vec<_> = std::fs::read_dir(&accounts)
        .map(|entries| entries.flatten().map(|e| e.file_name()).collect())
        .unwrap_or_default();
    assert!(
        leftovers
            .iter()
            .all(|name| name.to_string_lossy() == "store.key" || name.to_string_lossy() == "login"),
        "{leftovers:?}"
    );
}

#[test]
fn signing_in_again_replaces_the_stored_login() {
    let fixture = Fixture::signed_in();
    let again = fixture.canoa(&["accounts", "login", "work"], Some("654321\n"));
    assert!(again.status.success(), "{}", text(&again.stderr));
    assert_eq!(fixture.account()["revision"], 2);
}

/// A fixture whose entry offers one account made by `method` (a pasted code for `subscription`, a typed key for `api-key`),
/// with `extra` lines in the entry.
fn offering(method: &str, input: &str, extra: &str) -> Fixture {
    let fixture = Fixture::bare();
    let added = fixture.canoa(
        &[
            "accounts",
            "add",
            "one",
            "--harness",
            "claude",
            "--method",
            method,
        ],
        Some(input),
    );
    assert!(added.status.success(), "{}", text(&added.stderr));
    let cfg = std::fs::read_to_string(&fixture.config).expect("canoa.cfg");
    std::fs::write(
        &fixture.config,
        cfg.replace(
            "account_settings = { renew_poll_ms = 100 }",
            &format!("accounts = [\"one\"]\n{extra}account_settings = {{ renew_poll_ms = 100 }}"),
        ),
    )
    .expect("the entry offers the account");
    fixture
}

/// The `--setting-sources` argument each start of the stand-in was given, after one session on the account `one`.
fn setting_sources_of(fixture: Fixture) -> Vec<String> {
    let (host, _log, record, _cfg) = fixture.serve();
    let client = support::key();
    host.pair(&client);
    let dir = cwd(&host.scratch);
    with_link(host.url("Claude"), &client, host.peer, |rpc| {
        rpc.request(
            "initialize",
            json!({"protocolVersion": 1, "clientCapabilities": {}, "_meta": {ACCOUNT_META: "one"}}),
        )
        .expect("initialized on the account");
        rpc.request("session/new", json!({"cwd": dir, "mcpServers": []}))
            .expect("a session opens");
    })
    .expect("served");
    waits_for("the stand-in started", || !records(&record).is_empty());
    records(&record)
        .iter()
        .filter_map(|each| each["args"].as_array())
        .flatten()
        .filter_map(Value::as_str)
        .filter(|arg| arg.starts_with("--setting-sources"))
        .map(str::to_owned)
        .collect()
}

#[test]
fn an_entry_offering_an_api_key_loads_no_settings_file_so_a_repository_cannot_move_the_key() {
    let sources = setting_sources_of(offering("api-key", "sk-test-key-0000\n", ""));
    assert!(!sources.is_empty());
    assert!(
        sources.iter().all(|arg| arg == "--setting-sources="),
        "{sources:?}"
    );
}

#[test]
fn an_entry_offering_a_login_keeps_the_providers_own_choice_of_settings() {
    let sources = setting_sources_of(offering("subscription", "123456\n", ""));
    assert!(
        sources.iter().all(|arg| arg != "--setting-sources="),
        "{sources:?}"
    );
}

#[test]
fn an_entrys_own_setting_sources_win_over_that_default() {
    let sources = setting_sources_of(offering(
        "api-key",
        "sk-test-key-0000\n",
        "setting_sources = [\"project\"]\n",
    ));
    assert!(
        sources.iter().all(|arg| arg == "--setting-sources=project"),
        "{sources:?}"
    );
    assert!(!sources.is_empty());
}

/// A host with no session bus keeps an account in sealed files by itself, with `account_store` left at its default.
#[cfg(target_os = "linux")]
#[test]
fn a_host_with_no_session_bus_signs_an_account_in_and_keeps_it_in_files() {
    let fixture = Fixture::bare();
    let cfg = std::fs::read_to_string(&fixture.config).expect("canoa.cfg");
    std::fs::write(
        &fixture.config,
        cfg.replace(
            "[host]
account_store = \"file\"
",
            "[host]
",
        ),
    )
    .expect("canoa.cfg without account_store");
    let added = fixture.canoa_on(
        &[
            "accounts",
            "add",
            "work",
            "--harness",
            "claude",
            "--method",
            "subscription",
        ],
        Some("123456\n"),
        true,
    );
    assert!(added.status.success(), "{}", text(&added.stderr));
    let listed = fixture.canoa_on(&["accounts", "list"], None, true);
    let out = text(&listed.stdout);
    assert!(out.contains("work claude claude-login signed in"), "{out}");
    assert!(
        out.contains("non-default: host.account_store is auto and "),
        "{out}"
    );
    let sealed = std::fs::read_dir(fixture.home().join("accounts"))
        .expect("accounts directory")
        .flatten()
        .any(|entry| entry.file_name().to_string_lossy().ends_with(".sealed"));
    assert!(sealed, "the account is in a sealed file");
}

#[test]
fn a_key_trusted_with_accounts_runs_a_session_on_an_account_the_entry_does_not_list() {
    let (trusted, other) = (support::key(), support::key());
    let fixture = Fixture::unlisted();
    let mut cfg = std::fs::read_to_string(&fixture.config).expect("canoa.cfg");
    cfg.push_str(&format!(
        "
[host.control]
grants = {{ \"{}\" = [\"accounts\"] }}
",
        trusted.peer_id()
    ));
    std::fs::write(&fixture.config, cfg).expect("canoa.cfg");
    let (host, _log, _record, _cfg) = fixture.serve();
    host.pair(&trusted);
    host.pair(&other);
    // A key with no grant for accounts is turned away, as an entry that lists none has always done.
    let refused = with_link(host.url("Claude"), &other, host.peer, |rpc| {
        initialize_as_work(rpc).expect_err("not listed")
    })
    .expect("served");
    assert!(
        refused["message"]
            .as_str()
            .is_some_and(|why| why.contains("does not offer")),
        "{refused}"
    );
    // The trusted key chooses it, and the connection reports it.
    let chosen = with_link(host.url("Claude"), &trusted, host.peer, |rpc| {
        initialize_as_work(rpc).expect("an account of the provider")
    })
    .expect("served");
    assert_eq!(
        chosen["_meta"][ACCOUNT_META],
        json!({"name": "work", "kind": "claude-login"}),
        "{chosen}"
    );
    // It may also run on the login the program has where it runs, which `null` says.
    let own = with_link(host.url("Claude"), &trusted, host.peer, |rpc| {
        rpc.request(
            "initialize",
            json!({"protocolVersion": 1, "clientCapabilities": {}, "_meta": {ACCOUNT_META: null}}),
        )
        .expect("the machine's login")
    })
    .expect("served");
    assert!(own["_meta"].get(ACCOUNT_META).is_none(), "{own}");
}

/// A fixture whose Claude entry's provider made the account `one` from a key, and whose entry `Pi` runs the Pi stand-in and
/// offers it; `extra` is more of canoa.cfg. The names of what each start of Pi was given are in `pi.log` of the scratch.
fn pi_on_a_claude_key(extra: &str) -> Fixture {
    let fixture = Fixture::bare();
    let added = fixture.canoa(
        &[
            "accounts",
            "add",
            "one",
            "--harness",
            "claude",
            "--method",
            "api-key",
        ],
        Some("sk-test-key-0000\n"),
    );
    assert!(added.status.success(), "{}", text(&added.stderr));
    let pi = fixture
        .scratch
        .0
        .join(format!("pi{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(example("fake_pi", false), &pi).expect("the Pi stand-in is copied");
    let mut cfg = std::fs::read_to_string(&fixture.config).expect("canoa.cfg");
    cfg.push_str(&format!(
        "\n[host.agents.Pi]\nharness = \"pi\"\nexecutable = {}\naccounts = [\"one\"]\n{extra}",
        quoted(&pi.to_string_lossy())
    ));
    std::fs::write(&fixture.config, cfg).expect("canoa.cfg");
    fixture
}

#[test]
fn an_account_says_which_standards_it_provides() {
    let fixture = Fixture::signed_in();
    assert_eq!(fixture.account()["provides"], json!(["claude-code"]));
    let fixture = offering("api-key", "sk-test-key-0000\n", "");
    assert_eq!(fixture.account()["provides"], json!(["anthropic-api"]));
}

#[test]
fn a_pi_session_runs_on_an_anthropic_key_that_claude_codes_provider_made() {
    let fixture = pi_on_a_claude_key("");
    let scratch = fixture.scratch.0.clone();
    let log = scratch.join("pi.log");
    let config = std::fs::read_to_string(&fixture.config).expect("canoa.cfg");
    let env = vec![("COQUI_ACP_FAKE_PI_LOG", log.clone().into_os_string())];
    let (host, _log) = Host::start_logged(fixture.scratch, &config, 2, &env);
    let client = support::key();
    host.pair(&client);
    let dir = cwd(&host.scratch);
    let answer = with_link(host.url("Pi"), &client, host.peer, |rpc| {
        let answer = rpc
            .request(
                "initialize",
                json!({"protocolVersion": 1, "clientCapabilities": {}, "_meta": {ACCOUNT_META: "one"}}),
            )
            .expect("initialized on the account");
        rpc.request("session/new", json!({"cwd": dir, "mcpServers": []}))
            .expect("a session opens");
        answer
    })
    .expect("served");
    assert_eq!(
        answer["_meta"][ACCOUNT_META],
        json!({"name": "one", "kind": "anthropic-api-key"}),
        "{answer}"
    );
    waits_for("Pi started", || !records(&log).is_empty());
    let names: Vec<String> = records(&log)
        .iter()
        .filter_map(|each| each["env_names"].as_array())
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    assert!(
        names.contains(&"ANTHROPIC_API_KEY".to_owned()),
        "Pi was given the key in the variable it reads: {names:?}"
    );
    assert!(
        names.contains(&"PI_CODING_AGENT_DIR".to_owned()),
        "and a directory of its own for the account: {names:?}"
    );
}

#[test]
fn an_entry_that_lists_an_account_its_harness_takes_no_standard_of_does_not_start_and_says_why() {
    // A Claude subscription provides `claude-code`, which only Claude Code takes.
    let fixture = Fixture::unlisted();
    let codex = fixture
        .scratch
        .0
        .join(format!("codex{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(example("harness_standin", false), &codex).expect("copied");
    let mut cfg = std::fs::read_to_string(&fixture.config).expect("canoa.cfg");
    cfg.push_str(&format!(
        "\n[host.agents.Codex]\nharness = \"codex\"\nexecutable = {}\naccounts = [\"work\"]\n",
        quoted(&codex.to_string_lossy())
    ));
    std::fs::write(&fixture.config, cfg).expect("canoa.cfg");
    let output = fixture.canoa(&["serve"], None);
    assert!(!output.status.success());
    let said = text(&output.stderr);
    assert!(
        said.contains("account work cannot run on codex")
            && said.contains("claude-code")
            && said.contains("codex, openai-api"),
        "{said}"
    );
}
