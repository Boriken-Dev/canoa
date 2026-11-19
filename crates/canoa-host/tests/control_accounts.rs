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
                "[secrets]\naccounts = \"file\"\n\n[host.agents.Claude]\nharness = \"claude\"\nexecutable = {}\naccount_settings = {{ renew_poll_ms = 100 }}\n",
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
fn opencodes_kinds_and_its_sign_ins_are_listed_and_its_source_can_be_offered_accounts() {
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
    assert_eq!(methods, ["provider-key", "provider-code", "api-key"]);
    for own in &catalog["logins"].as_array().expect("logins")[..2] {
        assert_eq!(own["produces"], "opencode-login");
        assert_eq!(own["inputs"]["required"], json!(["provider"]));
    }
    let key = &catalog["logins"][2];
    assert_eq!(key["produces"], "opencode-provider-key");
    // The providers' variables are suggestions, not the only names the form takes.
    let variable = &key["inputs"]["properties"]["variable"];
    assert!(
        variable["examples"]
            .as_array()
            .is_some_and(|variables| variables.iter().any(|name| name == "OPENAI_API_KEY")),
        "{key}"
    );
    assert!(variable.get("enum").is_none(), "{key}");
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
fn an_opencode_key_account_names_any_variable_but_the_hosts_own() {
    let host = host();
    let link = host.local.open("@control").expect("a link");
    let mut app = app_of(&host);
    let (_, added) = app.drive(
        &link,
        1,
        "accounts/add",
        &json!({"name": "gateway", "harness": "opencode", "method": "api-key",
                "inputs": {"variable": "ACME_GATEWAY_KEY"}}),
        &Typed::Fills(&[("api_key", KEY)]),
    );
    let added = added.expect("a variable no provider lists is taken");
    assert_eq!(added["account"]["kind"], "opencode-provider-key");
    let cfg = config_text(&host);
    assert!(cfg.contains("ACME_GATEWAY_KEY"), "{cfg}");
    let (_, refused) = app.drive(
        &link,
        2,
        "accounts/add",
        &json!({"name": "owned", "harness": "opencode", "method": "api-key",
                "inputs": {"variable": "AOS_OVERLAY_NAME"}}),
        &Typed::Fills(&[("api_key", KEY)]),
    );
    assert!(refused.is_err(), "a name the host owns is refused");
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
            ("OpenCode", "opencode"),
            ("Pi", "pi")
        ],
        "the entry canoa.cfg has, then every built-in source it has not named that runs a harness"
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
fn a_pool_is_made_edited_and_removed_over_the_link_and_an_entry_offers_it() {
    let host = host();
    let link = host.local.open("@control").expect("a link");
    let mut app = app_of(&host);
    for name in ["one", "two"] {
        let (_, added) = app.drive(
            &link,
            1,
            "accounts/add",
            &json!({"name": name, "harness": "claude", "method": "api-key"}),
            &Typed::Fills(&[("api_key", KEY)]),
        );
        added.expect("a key account");
    }
    let (_, login) = app.drive(
        &link,
        2,
        "accounts/add",
        &json!({"name": "login", "harness": "claude", "method": "subscription"}),
        &Typed::Fills(&[("code", "123456")]),
    );
    login.expect("a login");

    // The host lists its own pool of each standard before the person has made one: nothing is written for them.
    let before = plain_call(&link, 3, "pools/list", json!({})).expect("listed");
    let pools = before["pools"].as_array().expect("pools");
    let any = pools
        .iter()
        .find(|pool| pool["name"] == "any-anthropic-api")
        .expect("the host's own pool");
    assert_eq!(any["automatic"], true, "{any}");
    assert_eq!(any["accounts"], json!(["one", "two"]), "{any}");
    assert!(
        pools.iter().all(|pool| pool["automatic"] == true),
        "{before}"
    );
    assert!(
        !config_text(&host).contains("[host.pools"),
        "{}",
        config_text(&host)
    );

    // Made: the accounts are the pool's in the order given, and the file says so.
    let made = plain_call(
        &link,
        4,
        "pools/set",
        json!({"name": "keys", "standard": "anthropic-api", "accounts": ["two", "one"]}),
    )
    .expect("made");
    assert_eq!(made["restart"], true);
    let cfg = config_text(&host);
    assert!(
        cfg.contains(
            "[host.pools.keys]\nstandard = \"anthropic-api\"\naccounts = [\"two\", \"one\"]\n"
        ),
        "{cfg}"
    );
    let listed = plain_call(&link, 5, "pools/list", json!({})).expect("listed");
    assert_eq!(listed["pools"][0]["name"], "keys", "{listed}");
    assert_eq!(
        listed["pools"][0]["accounts"],
        json!(["two", "one"]),
        "{listed}"
    );
    assert_eq!(listed["pools"][0]["automatic"], false, "{listed}");

    // An account that does not provide the standard is refused, naming the account, its kind's standards and the pool's.
    let bad = plain_call(
        &link,
        6,
        "pools/set",
        json!({"name": "keys", "standard": "anthropic-api", "accounts": ["one", "login"]}),
    )
    .expect_err("a login is not an API key");
    assert!(
        bad.contains("login")
            && bad.contains("claude-login")
            && bad.contains("claude-code")
            && bad.contains("anthropic-api"),
        "{bad}"
    );
    assert!(
        config_text(&host).contains("accounts = [\"two\", \"one\"]"),
        "a refused edit changes nothing"
    );
    // A pool takes no account's name, nor the host's own pools'; an account the host lacks is not found.
    for (name, why) in [("one", "account"), ("any-codex", "taken")] {
        let refused = plain_call(
            &link,
            7,
            "pools/set",
            json!({"name": name, "standard": "anthropic-api", "accounts": []}),
        )
        .expect_err("taken");
        assert!(refused.contains(why), "{refused}");
    }
    let missing = plain_call(
        &link,
        8,
        "pools/set",
        json!({"name": "ghosts", "standard": "anthropic-api", "accounts": ["nobody"]}),
    )
    .expect_err("no such account");
    assert!(missing.contains("-32003"), "{missing}");

    // Edited: the same name takes the new accounts and the file holds one table for it.
    plain_call(
        &link,
        9,
        "pools/set",
        json!({"name": "keys", "standard": "anthropic-api", "accounts": ["one"]}),
    )
    .expect("edited");
    let cfg = config_text(&host);
    assert_eq!(cfg.matches("[host.pools.keys]").count(), 1, "{cfg}");
    assert!(cfg.contains("accounts = [\"one\"]\n"), "{cfg}");

    // An entry offers the pool by name, beside accounts, and the host says which pools each entry offers.
    plain_call(
        &link,
        10,
        "accounts/offer",
        json!({"entry": "Claude", "accounts": ["keys", "login"]}),
    )
    .expect("a pool is offered like an account");
    let entries = plain_call(&link, 11, "accounts/entries", json!({})).expect("entries");
    let claude = &entries["entries"][0];
    assert_eq!(claude["accounts"], json!(["keys", "login"]), "{claude}");
    let offered = claude["pools"].as_array().expect("pools");
    let keys = offered
        .iter()
        .find(|pool| pool["name"] == "keys")
        .expect("keys");
    assert_eq!(keys["listed"], true, "{keys}");
    assert_eq!(
        keys["accounts"],
        json!([{"name": "one", "listed": true}]),
        "{keys}"
    );
    // A pool of another harness's standard is no choice for this entry.
    let other = plain_call(
        &link,
        12,
        "pools/set",
        json!({"name": "chatgpt", "standard": "codex", "accounts": ["local"]}),
    )
    .expect("made");
    assert_eq!(other["pool"]["standard"], "codex");
    let refused = plain_call(
        &link,
        13,
        "accounts/offer",
        json!({"entry": "Claude", "accounts": ["chatgpt"]}),
    )
    .expect_err("Claude takes no Codex login");
    assert!(
        refused.contains("codex") && refused.contains("claude"),
        "{refused}"
    );

    // An entry that offers a pool, or a pool that holds an account, keeps them from being removed.
    let held = plain_call(&link, 14, "pools/remove", json!({"name": "keys"})).expect_err("offered");
    assert!(held.contains("offers keys"), "{held}");
    let member =
        plain_call(&link, 15, "accounts/remove", json!({"name": "one"})).expect_err("held");
    assert!(member.contains("pool keys holds one"), "{member}");
    plain_call(
        &link,
        16,
        "accounts/offer",
        json!({"entry": "Claude", "accounts": []}),
    )
    .expect("taken off");
    plain_call(&link, 17, "pools/remove", json!({"name": "keys"})).expect("removed");
    plain_call(&link, 18, "pools/remove", json!({"name": "chatgpt"})).expect("removed");
    let cfg = config_text(&host);
    assert!(!cfg.contains("[host.pools"), "{cfg}");
    let gone = plain_call(&link, 19, "pools/remove", json!({"name": "keys"})).expect_err("gone");
    assert!(gone.contains("-32003"), "{gone}");
    let own = plain_call(
        &link,
        20,
        "pools/remove",
        json!({"name": "any-anthropic-api"}),
    )
    .expect_err("not a file's");
    assert!(own.contains("host's own"), "{own}");
    plain_call(&link, 21, "accounts/remove", json!({"name": "one"}))
        .expect("free once the pool is gone");
    host.stop();
}

/// As [`host`], with every provider's usage read from a port nothing listens on, so no usage is ever asked of the real services.
fn host_without_usage() -> Hosted {
    let claude = example("harness_standin", false);
    Hosted::start_with_setup(
        |dir| {
            let program = dir.join(format!("claude{}", std::env::consts::EXE_SUFFIX));
            std::fs::copy(&claude, &program).expect("the stand-in is copied");
            format!(
                "[secrets]\naccounts = \"file\"\n\n[host.agents.Claude]\nharness = \"claude\"\nexecutable = {}\naccount_settings = {{ renew_poll_ms = 100, usage_endpoint = \"http://127.0.0.1:9\" }}\n",
                quoted(&program.to_string_lossy())
            )
        },
        &[],
    )
}

#[test]
fn the_next_turn_of_a_pool_session_is_asked_of_the_host_and_a_single_account_is_never_moved() {
    let host = host_without_usage();
    let link = host.local.open("@control").expect("a link");
    let mut app = app_of(&host);
    for name in ["one", "two"] {
        let (_, added) = app.drive(
            &link,
            1,
            "accounts/add",
            &json!({"name": name, "harness": "claude", "method": "api-key"}),
            &Typed::Fills(&[("api_key", KEY)]),
        );
        added.expect("a key account");
    }
    plain_call(
        &link,
        2,
        "pools/set",
        json!({"name": "turns", "standard": "anthropic-api", "accounts": ["one", "two"]}),
    )
    .expect("made");
    plain_call(
        &link,
        3,
        "accounts/offer",
        json!({"entry": "Claude", "accounts": ["turns"]}),
    )
    .expect("offered");
    let turn = |id: u64, params: Value| plain_call(&link, id, "pools/turn", params);
    let ask = |extra: Value| {
        let mut params = json!({"entry": "Claude", "account": "turns"});
        for (key, value) in extra.as_object().expect("an object") {
            params[key] = value.clone();
        }
        params
    };
    // A session that starts is given the pool's accounts in turn.
    let first = turn(4, ask(json!({}))).expect("a start");
    assert_eq!(
        (
            first["account"].as_str(),
            first["why"].as_str(),
            first["pool"].as_str()
        ),
        (Some("one"), Some("start"), Some("turns")),
        "{first}"
    );
    let second = turn(5, ask(json!({}))).expect("another start");
    assert_eq!(second["account"], "two", "{second}");
    // A warm cache keeps the account the session is on, however the others have been given out.
    let warm = turn(
        6,
        ask(json!({"current": "one", "idle_seconds": 30, "lifetime": "long"})),
    )
    .expect("kept");
    assert_eq!(
        (
            warm["account"].as_str(),
            warm["moved"].as_bool(),
            warm["why"].as_str()
        ),
        (Some("one"), Some(false), Some("kept")),
        "{warm}"
    );
    // A cache that has lapsed gives the turn to the next account, which moves the session.
    let cold = turn(
        7,
        ask(json!({"current": "two", "idle_seconds": 4000, "lifetime": "long"})),
    )
    .expect("in turn");
    assert_eq!(
        (
            cold["account"].as_str(),
            cold["moved"].as_bool(),
            cold["why"].as_str()
        ),
        (Some("one"), Some(true), Some("in_turn")),
        "{cold}"
    );
    // A limit the client heard of moves it even while the cache is warm, and the account it left is named.
    let hit = turn(
        8,
        ask(
            json!({"current": "one", "idle_seconds": 5, "lifetime": "long",
        "limited": [{"account": "one", "resets_at": 4_102_444_800_i64}]}),
        ),
    )
    .expect("moved");
    assert_eq!(
        (
            hit["account"].as_str(),
            hit["moved"].as_bool(),
            hit["why"].as_str()
        ),
        (Some("two"), Some(true), Some("exhausted")),
        "{hit}"
    );
    assert_eq!(hit["skipped"][0]["account"], "one", "{hit}");
    // With every account out the turn is refused, saying when the soonest allowance refills.
    let none = turn(9, ask(json!({"current": "two", "idle_seconds": 5,
        "limited": [{"account": "one", "resets_at": 4_102_444_800_i64}, {"account": "two", "resets_at": 4_102_358_400_i64}]})))
        .expect_err("none has room");
    assert!(
        none.contains("refills") && none.contains("2099-12-31 00:00 UTC"),
        "{none}"
    );
    // An account that is not one the entry offers, and a pool it does not, are not found.
    let missing = turn(10, json!({"entry": "Claude", "account": "nobody"})).expect_err("none");
    assert!(missing.contains("-32003"), "{missing}");

    // One account is never moved: when it runs out, the answer says so and lists what could take the session in its place.
    let single = turn(
        11,
        json!({"entry": "Claude", "account": "one", "current": "one"}),
    )
    .expect("a single account");
    assert_eq!(
        (
            single["moved"].as_bool(),
            single["exhausted"].as_bool(),
            single["pool"].is_null()
        ),
        (Some(false), Some(false), true),
        "{single}"
    );
    let out = turn(
        12,
        json!({"entry": "Claude", "account": "one", "current": "one", "idle_seconds": 9999,
            "limited": [{"account": "one", "resets_at": 4_102_444_800_i64}]}),
    )
    .expect("a single account that ran out");
    assert_eq!(out["moved"], false, "{out}");
    assert_eq!(out["exhausted"], true, "{out}");
    assert_eq!(out["account"], "one", "{out}");
    assert_eq!(out["alternatives"], json!(["two"]), "{out}");
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
