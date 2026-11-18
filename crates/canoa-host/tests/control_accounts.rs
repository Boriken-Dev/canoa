//! Accounts over the control channel, as the desktop app uses them: the catalog, signing in with the form a login asks put to the
//! person sealed (played here with the extension's own sealing and the in-process link's binding, as the app does), a sign-in
//! given up by closing the link, an answer bound to nothing, signing in again, and removing. A planted key is looked for, in
//! every form an accident could print, in everything the host wrote outside its sealed store and in every line the link carried.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod linked;
mod sealed;
mod support;

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use canoa_host::Link;
use linked::{Hosted, next, send};
use sealed::{App, Typed, forms, found_in};
use serde_json::{Value, json};
use support::{WAIT, example, host_binary, quoted};

const KEY: &str = "sk-ant-CANARY/+=q\"\\z-0123456789";

/// The host on a home of its own, serving a Claude entry that runs the stand-in, with the file store.
fn host() -> Hosted {
    let claude = example("harness_standin", false);
    Hosted::start_with_setup(
        |dir| {
            let program = dir.join(format!("claude{}", std::env::consts::EXE_SUFFIX));
            std::fs::copy(&claude, &program).expect("the stand-in is copied");
            format!(
                "[host]\naccount_store = \"file\"\n\n[host.agents.Claude]\nharness = \"claude\"\nexecutable = {}\naccount_settings = {{ renew_poll_ms = 100 }}\n",
                quoted(&program.to_string_lossy())
            )
        },
        &[],
    )
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
                assert_eq!(message["params"]["id"], id);
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

fn app_of(host: &Hosted) -> App {
    let secret = host.local.link_secret().expect("the host minted a secret");
    App::new(&secret[..])
}

fn plain_call(link: &Link, id: u64, method: &str, params: Value) -> Result<Value, String> {
    linked::ask(link, id, method, params).map(|(result, _)| result)
}

fn files_under(dir: &Path, skip: &[PathBuf], out: &mut Vec<(String, Vec<u8>)>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let path = entry.path();
        if skip.iter().any(|skipped| path.starts_with(skipped)) {
            continue;
        }
        if path.is_dir() {
            files_under(&path, skip, out);
        } else if let Ok(bytes) = std::fs::read(&path) {
            out.push((path.display().to_string(), bytes));
        }
    }
}

/// Everything the host wrote besides its sealed store, and the lines the link carried.
fn written(host: &Hosted, app: &App) -> Vec<(String, Vec<u8>)> {
    let mut places = Vec::new();
    let home = host.scratch.0.join("home");
    let skip = [home.join("accounts")];
    files_under(&host.scratch.0, &skip, &mut places);
    for (index, line) in app.traffic.iter().enumerate() {
        places.push((format!("link line {index}"), line.as_bytes().to_vec()));
    }
    places
}

fn config_text(host: &Hosted) -> String {
    std::fs::read_to_string(host.scratch.0.join("canoa.cfg")).expect("canoa.cfg")
}

#[test]
fn a_planted_key_is_found_where_it_is_and_in_every_form() {
    let places = vec![("a file".to_owned(), forms(KEY)[2].1.clone())];
    assert!(
        found_in(KEY, &places)
            .iter()
            .any(|hit| hit.ends_with("as base64")),
        "base64 is found"
    );
    for (name, bytes) in forms(KEY) {
        let hits = found_in(KEY, &[("p".to_owned(), bytes.clone())]);
        assert!(!hits.is_empty(), "{name} is found");
    }
    assert!(found_in(KEY, &[("p".to_owned(), b"nothing here".to_vec())]).is_empty());
}

#[test]
fn the_catalog_names_the_kinds_and_the_ways_to_sign_in() {
    let host = host();
    let link = host.local.open("@control").expect("a link");
    let catalog =
        plain_call(&link, 1, "accounts/catalog", json!({"harness": "claude"})).expect("catalog");
    assert_eq!(catalog["harness"], "claude");
    let kinds: Vec<&str> = catalog["kinds"]
        .as_array()
        .expect("kinds")
        .iter()
        .filter_map(|kind| kind["id"].as_str())
        .collect();
    assert!(
        kinds.contains(&"anthropic-api-key") && kinds.contains(&"claude-login"),
        "{kinds:?}"
    );
    let api_key = catalog["kinds"]
        .as_array()
        .expect("kinds")
        .iter()
        .find(|kind| kind["id"] == "anthropic-api-key")
        .expect("the key kind");
    for field in [
        "title",
        "description",
        "settings",
        "endpoints",
        "rotates",
        "history_portable",
        "state_dir",
        "renews",
        "notices",
    ] {
        assert!(api_key.get(field).is_some(), "{field} in {api_key}");
    }
    let methods: Vec<&str> = catalog["logins"]
        .as_array()
        .expect("logins")
        .iter()
        .filter_map(|method| method["id"].as_str())
        .collect();
    assert!(
        methods.contains(&"api-key") && methods.contains(&"subscription"),
        "{methods:?}"
    );
    for field in [
        "title",
        "description",
        "produces",
        "inputs",
        "steps",
        "remote",
    ] {
        assert!(catalog["logins"][0].get(field).is_some(), "{field}");
    }
    let unknown = plain_call(&link, 2, "accounts/catalog", json!({"harness": "nobody"}))
        .expect_err("unknown");
    assert!(unknown.contains("-32003"), "{unknown}");
    host.stop();
}

#[test]
fn opencodes_kinds_and_its_one_sign_in_are_listed_and_its_source_can_be_offered_accounts() {
    let host = host();
    let link = host.local.open("@control").expect("a link");
    let catalog =
        plain_call(&link, 1, "accounts/catalog", json!({"harness": "opencode"})).expect("catalog");
    let kinds: Vec<&str> = catalog["kinds"]
        .as_array()
        .expect("kinds")
        .iter()
        .filter_map(|kind| kind["id"].as_str())
        .collect();
    for kind in ["opencode-login", "opencode-provider-key"] {
        assert!(kinds.contains(&kind), "{kind} in {kinds:?}");
    }
    let methods: Vec<&str> = catalog["logins"]
        .as_array()
        .expect("logins")
        .iter()
        .filter_map(|method| method["id"].as_str())
        .collect();
    assert_eq!(methods, ["api-key"]);
    let key = &catalog["logins"][0];
    assert_eq!(key["produces"], "opencode-provider-key");
    assert!(
        key["inputs"]["properties"]["variable"]["enum"]
            .as_array()
            .is_some_and(|variables| variables.iter().any(|name| name == "OPENAI_API_KEY")),
        "{key}"
    );
    // The source is an entry that runs the provider, so an account can be offered to it.
    let entries = plain_call(&link, 2, "accounts/entries", json!({})).expect("entries");
    let opencode = entries["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .find(|entry| entry["name"] == "OpenCode")
        .expect("OpenCode is listed");
    assert_eq!(opencode["harness"], "opencode");
    assert!(
        opencode["accepts"]
            .as_array()
            .is_some_and(|accepted| accepted.iter().any(|standard| standard == "openai-api")),
        "{opencode}"
    );
    host.stop();
}

#[test]
fn an_account_is_added_by_a_key_asked_sealed_and_the_key_is_kept_nowhere_in_the_clear() {
    let host = host();
    let link = host.local.open("@control").expect("a link");
    let mut app = app_of(&host);
    let (_, added) = app.drive(
        &link,
        1,
        "accounts/add",
        &json!({"name": "work", "harness": "claude", "method": "api-key"}),
        &Typed::Fills(&[("api_key", KEY)]),
    );
    let added = added.expect("the account is added");
    assert_eq!(added["account"]["name"], "work");
    assert_eq!(added["account"]["kind"], "anthropic-api-key");
    assert!(
        added["account"]["id"]
            .as_str()
            .is_some_and(|id| id.len() >= 32),
        "{added}"
    );

    let listed = plain_call(&link, 2, "accounts/list", json!({})).expect("listed");
    assert_eq!(listed["accounts"][0]["name"], "work");
    assert_eq!(listed["accounts"][0]["state"], "signed in");
    assert_eq!(listed["accounts"][0]["id"], added["account"]["id"]);

    let cfg = config_text(&host);
    assert!(cfg.contains("[host.accounts.work]"), "{cfg}");
    // The key went to the host as sealed values: the request the host made carried the form, the answer only ciphertext.
    assert!(
        app.traffic
            .iter()
            .any(|line| line.contains("_coqui_acp/secret_request")),
        "the form was put to the app"
    );
    let hits = found_in(KEY, &written(&host, &app));
    assert!(hits.is_empty(), "{hits:?}");
    host.stop();
}

#[test]
fn a_sign_in_the_person_walks_away_from_makes_no_account_and_leaves_no_directory() {
    let host = host();
    let link = host.local.open("@control").expect("a link");
    let mut app = app_of(&host);
    let (progress, outcome) = app.drive(
        &link,
        1,
        "accounts/add",
        &json!({"name": "work", "harness": "claude", "method": "subscription"}),
        &Typed::Leaves,
    );
    assert!(outcome.is_err());
    assert!(
        progress.iter().any(|each| each["open_url"]["url"]
            .as_str()
            .is_some_and(|url| url.starts_with("https://"))),
        "{progress:?}"
    );
    drop(link);
    let login = host.scratch.0.join("home").join("accounts").join("login");
    let until = Instant::now() + WAIT;
    while login.exists() && std::fs::read_dir(&login).is_ok_and(|dir| dir.count() > 0) {
        assert!(Instant::now() < until, "the sign-in's directory stays");
        std::thread::sleep(Duration::from_millis(50));
    }
    let second = host.local.open("@control").expect("another link");
    let listed = plain_call(&second, 1, "accounts/list", json!({})).expect("listed");
    assert_eq!(
        listed["accounts"].as_array().map(Vec::len),
        Some(0),
        "{listed}"
    );
    assert!(
        !config_text(&host).contains("[host.accounts"),
        "no block was written"
    );
    host.stop();
}

#[test]
fn a_declined_form_and_an_answer_bound_to_another_secret_make_no_account() {
    let host = host();
    let link = host.local.open("@control").expect("a link");
    let mut app = app_of(&host);
    let params = json!({"name": "work", "harness": "claude", "method": "api-key"});
    let (_, declined) = app.drive(&link, 1, "accounts/add", &params, &Typed::Declines);
    let declined = declined.expect_err("declined");
    assert_eq!(declined["code"], -32002, "{declined}");
    let (_, forged) = app.drive(
        &link,
        2,
        "accounts/add",
        &params,
        &Typed::Forged(&[("api_key", KEY)]),
    );
    let forged = forged.expect_err("refused");
    assert_eq!(forged["code"], -32002, "{forged}");
    assert!(!forged.to_string().contains(KEY));
    let listed = plain_call(&link, 3, "accounts/list", json!({})).expect("listed");
    assert_eq!(listed["accounts"].as_array().map(Vec::len), Some(0));
    assert!(!config_text(&host).contains("[host.accounts"));
    let hits = found_in(KEY, &written(&host, &app));
    // The forged answer sealed the key to the host's key; nothing but ciphertext crossed.
    assert!(hits.is_empty(), "{hits:?}");
    host.stop();
}

#[test]
fn an_account_signs_in_again_and_is_removed() {
    let host = host();
    let link = host.local.open("@control").expect("a link");
    let mut app = app_of(&host);
    let (_, added) = app.drive(
        &link,
        1,
        "accounts/add",
        &json!({"name": "work", "harness": "claude", "method": "subscription"}),
        &Typed::Fills(&[("code", "123456")]),
    );
    let added = added.expect("added by a pasted code");
    assert_eq!(added["account"]["kind"], "claude-login");
    let (_, again) = app.drive(
        &link,
        2,
        "accounts/login",
        &json!({"name": "work"}),
        &Typed::Fills(&[("code", "654321")]),
    );
    let again = again.expect("signed in again");
    assert_eq!(again["account"]["id"], added["account"]["id"]);
    assert_eq!(again["account"]["name"], "work");
    let removed =
        plain_call(&link, 3, "accounts/remove", json!({"name": "work"})).expect("removed");
    assert_eq!(removed, json!({"removed": true}));
    let listed = plain_call(&link, 4, "accounts/list", json!({})).expect("listed");
    assert_eq!(listed["accounts"].as_array().map(Vec::len), Some(0));
    assert!(!config_text(&host).contains("[host.accounts"));
    let missing =
        plain_call(&link, 5, "accounts/remove", json!({"name": "work"})).expect_err("gone");
    assert!(missing.contains("-32602"), "{missing}");
    host.stop();
}

#[test]
fn the_list_reads_canoa_cfg_again_so_an_account_the_command_line_added_shows() {
    let host = host();
    let link = host.local.open("@control").expect("a link");
    let listed = plain_call(&link, 1, "accounts/list", json!({})).expect("listed");
    assert_eq!(listed["accounts"].as_array().map(Vec::len), Some(0));
    let mut child = Command::new(host_binary())
        .args([
            "accounts",
            "add",
            "late",
            "--harness",
            "claude",
            "--method",
            "subscription",
            "--config",
        ])
        .arg(host.scratch.0.join("canoa.cfg"))
        .env("COQUI_AGENTIC_PEER_HOME", host.scratch.0.join("home"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("canoa runs");
    child
        .stdin
        .take()
        .expect("input")
        .write_all(b"123456\n")
        .expect("typed");
    let output = child.wait_with_output().expect("ends");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let listed = plain_call(&link, 2, "accounts/list", json!({})).expect("listed");
    assert_eq!(listed["accounts"][0]["name"], "late", "{listed}");
    host.stop();
}

#[test]
fn an_entry_is_offered_the_accounts_the_person_picks_and_the_host_says_it_must_start_again() {
    let host = host();
    let link = host.local.open("@control").expect("a link");
    let mut app = app_of(&host);
    let (_, added) = app.drive(
        &link,
        1,
        "accounts/add",
        &json!({"name": "work", "harness": "claude", "method": "api-key"}),
        &Typed::Fills(&[("api_key", KEY)]),
    );
    added.expect("the account is added");
    let entries = plain_call(&link, 2, "accounts/entries", json!({})).expect("entries");
    let names: Vec<(&str, &str)> = entries["entries"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|each| {
            (
                each["name"].as_str().expect("a name"),
                each["harness"].as_str().expect("a harness"),
            )
        })
        .collect();
    assert_eq!(
        names,
        [
            ("Claude", "claude"),
            ("Codex", "codex"),
            ("OpenCode", "opencode")
        ],
        "the entry canoa.cfg has, then the built-in sources it has not named"
    );
    assert_eq!(entries["entries"][0]["accounts"], json!([]));

    let offered = plain_call(
        &link,
        3,
        "accounts/offer",
        json!({"entry": "Claude", "accounts": ["work"]}),
    )
    .expect("offered");
    assert_eq!(offered["restart"], true);
    let cfg = config_text(&host);
    assert!(cfg.contains("accounts = [\"work\"]"), "{cfg}");
    let entries = plain_call(&link, 4, "accounts/entries", json!({})).expect("entries");
    assert_eq!(entries["entries"][0]["accounts"], json!(["work"]));

    // A built-in source takes the name once it offers an account, and canoa.cfg then has an entry of that name.
    let codex = plain_call(
        &link,
        5,
        "accounts/offer",
        json!({"entry": "Codex", "accounts": ["work"]}),
    )
    .expect_err("an account of another provider");
    assert!(codex.contains("-32002"), "{codex}");
    let unknown = plain_call(
        &link,
        6,
        "accounts/offer",
        json!({"entry": "Claude", "accounts": ["nobody"]}),
    )
    .expect_err("no such account");
    assert!(unknown.contains("-32003"), "{unknown}");
    let nowhere = plain_call(
        &link,
        7,
        "accounts/offer",
        json!({"entry": "Nothing", "accounts": []}),
    )
    .expect_err("no such entry");
    assert!(nowhere.contains("-32003"), "{nowhere}");
    let twice = plain_call(
        &link,
        8,
        "accounts/offer",
        json!({"entry": "Claude", "accounts": ["work", "work"]}),
    )
    .expect_err("listed twice");
    assert!(twice.contains("-32602"), "{twice}");

    // The host will not forget an account an entry offers; taking it off the entry first lets it.
    let held = plain_call(&link, 9, "accounts/remove", json!({"name": "work"})).expect_err("held");
    assert!(held.contains("offers work"), "{held}");
    plain_call(
        &link,
        10,
        "accounts/offer",
        json!({"entry": "Claude", "accounts": []}),
    )
    .expect("taken off");
    plain_call(&link, 11, "accounts/remove", json!({"name": "work"})).expect("removed");
    host.stop();
}

#[test]
fn an_account_says_what_it_provides_an_entry_what_it_accepts_and_an_offer_follows_them() {
    let host = host();
    let link = host.local.open("@control").expect("a link");
    let mut app = app_of(&host);
    let (_, added) = app.drive(
        &link,
        1,
        "accounts/add",
        &json!({"name": "work", "harness": "claude", "method": "api-key"}),
        &Typed::Fills(&[("api_key", KEY)]),
    );
    added.expect("the account is added");
    let listed = plain_call(&link, 2, "accounts/list", json!({})).expect("listed");
    assert_eq!(listed["accounts"][0]["provides"], json!(["anthropic-api"]));

    let entries = plain_call(&link, 3, "accounts/entries", json!({})).expect("entries");
    let of = |name: &str| {
        entries["entries"]
            .as_array()
            .expect("a list")
            .iter()
            .find(|each| each["name"] == name)
            .expect("the entry")
            .clone()
    };
    assert_eq!(
        of("Claude")["accepts"],
        json!(["claude-code", "anthropic-api"])
    );
    assert_eq!(of("Claude")["usable"], json!(["work"]));
    assert_eq!(of("Codex")["accepts"], json!(["codex", "openai-api"]));
    assert_eq!(
        of("Codex")["usable"],
        json!([]),
        "Codex takes no Anthropic key"
    );

    // Codex refuses it, saying what each side says; Pi, which has a variable for an Anthropic key, takes it.
    let codex = plain_call(
        &link,
        4,
        "accounts/offer",
        json!({"entry": "Codex", "accounts": ["work"]}),
    )
    .expect_err("Codex accepts no Anthropic key");
    assert!(
        codex.contains("anthropic-api") && codex.contains("codex, openai-api"),
        "{codex}"
    );
    assert!(!codex.contains(KEY), "{codex}");
    let pi = plain_call(
        &link,
        5,
        "accounts/offer",
        json!({"entry": "Pi", "harness": "pi", "accounts": ["work"]}),
    )
    .expect("Pi takes an Anthropic key");
    assert_eq!(pi["restart"], true);
    let entries = plain_call(&link, 6, "accounts/entries", json!({})).expect("entries");
    let pi = entries["entries"]
        .as_array()
        .expect("a list")
        .iter()
        .find(|each| each["name"] == "Pi")
        .expect("the entry offers it now")
        .clone();
    assert_eq!(pi["accounts"], json!(["work"]));
    assert!(
        pi["accepts"]
            .as_array()
            .expect("a list")
            .contains(&json!("anthropic-api")),
        "{pi}"
    );
    assert_eq!(pi["usable"], json!(["work"]));
    host.stop();
}
