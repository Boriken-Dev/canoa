//! Foundry Local as an account: its kind in a harness's catalog, the account it makes, and the grant a session is given.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use canoa_foundry::{EngineError, ModelInfo};
use serde_json::json;

use super::*;
use crate::accounts::matching::Runs;
use crate::accounts::store::FileStore;
use crate::accounts::{Accounts, Refusal};
use crate::config::AccountConfig;

/// A directory of its own that goes with the test.
struct Dir(PathBuf);

impl Dir {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "canoa-foundry-accounts-{}-{}",
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

/// The model and the window each engine was opened with.
type Seen = Arc<Mutex<Vec<(String, Option<u64>)>>>;

struct Local;

impl Engine for Local {
    fn model(&self) -> Result<ModelInfo, EngineError> {
        Ok(ModelInfo::new("local-model").tools(true))
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![ModelInfo::new("local-model").tools(true)]
    }

    fn chat(&self, _request: &serde_json::Value) -> Result<serde_json::Value, EngineError> {
        Err(EngineError::Failed("not asked here".to_owned()))
    }
}

fn catalog_of(harness: &str) -> AccountCatalog {
    let mut catalog = AccountCatalog::new();
    extend(&mut catalog, harness);
    catalog
}

fn settings(model: &str) -> Settings {
    let mut map = serde_json::Map::new();
    map.insert("model".to_owned(), json!(model));
    Settings::new(map)
}

fn config(harness: &str) -> AccountConfig {
    AccountConfig {
        harness: harness.to_owned(),
        kind: KIND.to_owned(),
        id: "id-foundry".to_owned(),
        settings: Some(settings("local-model")),
    }
}

fn accounts(dir: &Dir) -> Arc<Accounts> {
    let accounts = Accounts::with_store(
        dir.0.join("accounts"),
        Box::new(FileStore::new(dir.0.join("accounts"))),
    )
    .expect("opens");
    accounts
        .gateways()
        .engines(Arc::new(|_| Ok(Arc::new(Local) as Arc<dyn Engine>)));
    accounts
        .store_login("id-foundry", &Credentials::new(), None)
        .expect("the account is made, and holds no credential");
    accounts
}

fn status(address: &str, bearer: &str) -> Option<u16> {
    let host = address.strip_prefix("http://")?;
    let mut stream = TcpStream::connect(host).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .ok()?;
    let request = format!(
        "GET /v1/models HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {bearer}\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).ok()?;
    let mut said = String::new();
    let _ = stream.read_to_string(&mut said);
    said.split_whitespace().nth(1)?.parse().ok()
}

fn text(grant: &AccountGrant, key: &str) -> String {
    let settings = guatiao_serde::adapt::from_value::<serde_json::Map<String, serde_json::Value>>(
        &grant.settings,
    )
    .expect("settings");
    settings[key].as_str().expect("text").to_owned()
}

#[test]
fn the_kind_and_its_method_join_the_catalogs_of_the_harnesses_that_can_be_given_a_gateway() {
    for harness in ["claude", "codex"] {
        let catalog = catalog_of(harness);
        let kind = catalog.kind(KIND).expect("the kind");
        assert!(kind.state_dir && kind.notices.len() == 2);
        assert_eq!(catalog.logins[0].produces, KIND);
        let mut again = catalog.clone();
        extend(&mut again, harness);
        assert_eq!(again, catalog, "adding it twice changes nothing");
    }
    assert!(catalog_of("grok").kind(KIND).is_none());
    assert!(catalog_of("pi").kind(KIND).is_some());
    assert!(catalog_of("antigravity").logins.is_empty());
}

#[test]
fn the_kinds_settings_are_checked_like_any_others() {
    let catalog = catalog_of("claude");
    let kind = catalog.kind(KIND).expect("the kind");
    assert!(crate::accounts::check_settings(kind, Some(&settings("m"))).is_ok());
    let mut open = serde_json::Map::new();
    open.insert("model".to_owned(), json!("m"));
    open.insert("endpoint".to_owned(), json!("http://example.org:1"));
    let refused = crate::accounts::check_settings(kind, Some(&Settings::new(open)))
        .expect_err("a far address");
    assert!(refused.contains("endpoint"), "{refused}");
    let mut stray = serde_json::Map::new();
    stray.insert("model".to_owned(), json!("m"));
    stray.insert("password".to_owned(), json!("x"));
    assert!(crate::accounts::check_settings(kind, Some(&Settings::new(stray))).is_err());
}

#[test]
fn signing_in_names_a_model_and_keeps_no_credential() {
    let mut inputs = serde_json::Map::new();
    inputs.insert("model".to_owned(), json!("qwen3-14b-generic-gpu:2"));
    let outcome = sign_in(&inputs).expect("signs in");
    assert_eq!(outcome.kind, KIND);
    assert_eq!(outcome.label.as_deref(), Some("qwen3-14b-generic-gpu:2"));
    assert!(outcome.credentials.expose().is_empty());
    assert!(
        sign_in(&serde_json::Map::new()).is_err(),
        "a model is named"
    );
}

#[test]
fn a_claude_session_is_given_a_gateway_token_and_its_address() {
    let dir = Dir::new();
    let accounts = accounts(&dir);
    let info = catalog_of("claude").kind(KIND).expect("the kind").clone();
    let held = accounts
        .begin(
            "local",
            &config("claude"),
            &info,
            Runs::plain("claude"),
            "a session",
        )
        .expect("begins");
    let grant = held.grant();
    assert_eq!(grant.kind, "anthropic-auth-token");
    let address = text(&grant, "base_url");
    assert!(
        address.starts_with("http://127.0.0.1:") && !address.ends_with("/v1"),
        "{address}"
    );
    let bearer = grant
        .credentials
        .expose()
        .get("auth_token")
        .expect("a bearer")
        .to_owned();
    assert_eq!(bearer.len(), 64);
    assert_eq!(status(&address, &bearer), Some(200));
    assert_eq!(status(&address, "another"), Some(401));
    assert_eq!(
        held.report()["kind"],
        KIND,
        "the account is still the Foundry Local one"
    );
    drop(held);
    assert_ne!(
        status(&address, &bearer),
        Some(200),
        "the session's gateway went with it"
    );
}

#[test]
fn a_codex_session_is_given_an_api_key_the_v1_address_and_a_folder_that_turns_off_what_a_gateway_cannot_serve()
 {
    let dir = Dir::new();
    let accounts = accounts(&dir);
    let info = catalog_of("codex").kind(KIND).expect("the kind").clone();
    let held = accounts
        .begin(
            "local",
            &config("codex"),
            &info,
            Runs::plain("codex"),
            "a session",
        )
        .expect("begins");
    let grant = held.grant();
    assert_eq!(grant.kind, "codex-api-key");
    let address = text(&grant, "openai_base_url");
    assert!(address.ends_with("/v1"), "{address}");
    let bearer = grant
        .credentials
        .expose()
        .get("api_key")
        .expect("a bearer")
        .to_owned();
    assert_eq!(status(address.trim_end_matches("/v1"), &bearer), Some(200));
    let folder = PathBuf::from(grant.state_dir.as_deref().expect("a folder"));
    let config = std::fs::read_to_string(folder.join("config.toml")).expect("written");
    assert!(config.contains("multi_agent = false") && config.contains("web_search = \"disabled\""));
}

#[test]
fn a_pi_session_is_given_an_api_key_in_the_openai_variable_and_a_folder_that_names_the_gateway() {
    let dir = Dir::new();
    let accounts = accounts(&dir);
    let info = catalog_of("pi").kind(KIND).expect("the kind").clone();
    let held = accounts
        .begin(
            "local",
            &config("pi"),
            &info,
            Runs::plain("pi"),
            "a session",
        )
        .expect("begins");
    let grant = held.grant();
    assert_eq!(grant.kind, "pi-api-key");
    assert_eq!(text(&grant, "variable"), "OPENAI_API_KEY");
    let bearer = grant
        .credentials
        .expose()
        .get("api_key")
        .expect("a bearer")
        .to_owned();
    let folder = PathBuf::from(grant.state_dir.as_deref().expect("a folder"));
    let models: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(folder.join("models.json")).expect("written"),
    )
    .expect("JSON");
    let address = models["providers"]["openai"]["baseUrl"]
        .as_str()
        .expect("an address")
        .to_owned();
    assert!(address.ends_with("/v1"), "{address}");
    assert_eq!(status(address.trim_end_matches("/v1"), &bearer), Some(200));
}

#[test]
fn an_account_made_for_one_harness_runs_another_in_that_harness_s_own_kind_and_keeps_its_standards()
{
    let dir = Dir::new();
    let accounts = accounts(&dir);
    let info = catalog_of("claude").kind(KIND).expect("the kind").clone();
    // Made for Claude Code, used by Codex: the gateway is addressed the way Codex reads it.
    let held = accounts
        .begin(
            "local",
            &config("claude"),
            &info,
            Runs::plain("codex"),
            "a session",
        )
        .expect("begins");
    let grant = held.grant();
    assert_eq!(grant.kind, "codex-api-key");
    assert!(text(&grant, "openai_base_url").ends_with("/v1"));
    assert!(grant.state_dir.is_some(), "Codex's folder is made");
    assert_eq!(held.report()["kind"], KIND);
    // A grant of another provider's kind carries what it provides, so the harness can apply it by a standard it accepts.
    let key = AccountKindInfo::new("anthropic-api-key", "Key").provides([standard::ANTHROPIC_API]);
    let plain = AccountConfig {
        harness: "claude".to_owned(),
        kind: "anthropic-api-key".to_owned(),
        id: "id-key".to_owned(),
        settings: None,
    };
    accounts
        .store_login(
            "id-key",
            &Credentials::from_iter([("api_key", "sk-test-0000")]),
            None,
        )
        .expect("stored");
    let held = accounts
        .begin("key", &plain, &key, Runs::plain("pi"), "a session")
        .expect("begins");
    assert_eq!(held.grant().provides, [standard::ANTHROPIC_API]);
}

#[test]
fn sessions_on_one_account_share_a_gateway_and_each_has_its_own_bearer() {
    let dir = Dir::new();
    let accounts = accounts(&dir);
    let info = catalog_of("claude").kind(KIND).expect("the kind").clone();
    let one = accounts
        .begin(
            "local",
            &config("claude"),
            &info,
            Runs::plain("claude"),
            "one",
        )
        .expect("begins");
    let two = accounts
        .begin(
            "local",
            &config("claude"),
            &info,
            Runs::plain("claude"),
            "two",
        )
        .expect("begins");
    let (a, b) = (one.grant(), two.grant());
    assert_eq!(text(&a, "base_url"), text(&b, "base_url"));
    let key = |grant: &AccountGrant| {
        grant
            .credentials
            .expose()
            .get("auth_token")
            .expect("a bearer")
            .to_owned()
    };
    assert_ne!(key(&a), key(&b));
    let address = text(&a, "base_url");
    drop(one);
    assert_ne!(
        status(&address, &key(&a)),
        Some(200),
        "the first session's bearer ended"
    );
    assert_eq!(
        status(&address, &key(&b)),
        Some(200),
        "the second's did not"
    );
}

#[test]
fn a_gateway_that_cannot_start_is_a_refusal_the_person_can_read_and_lets_the_account_go() {
    let dir = Dir::new();
    let accounts = accounts(&dir);
    let info = catalog_of("claude").kind(KIND).expect("the kind").clone();
    let broken = Accounts::with_store(
        dir.0.join("other"),
        Box::new(FileStore::new(dir.0.join("other"))),
    )
    .expect("opens");
    broken.gateways().engines(Arc::new(|_| {
        Err("no Foundry Local service is running".to_owned())
    }));
    broken
        .store_login("id-foundry", &Credentials::new(), None)
        .expect("stores");
    let refused = broken
        .begin(
            "local",
            &config("claude"),
            &info,
            Runs::plain("claude"),
            "a session",
        )
        .expect_err("no service");
    assert_eq!(
        refused,
        Refusal::Gateway("no Foundry Local service is running".to_owned())
    );
    assert!(refused.client_reason().contains("no Foundry Local service"));
    assert!(!broken.held("id-foundry"), "the account was let go");
    // A harness no gateway can be given to is refused by name.
    let refused = accounts
        .begin(
            "local",
            &config("grok"),
            &info,
            Runs::plain("grok"),
            "a session",
        )
        .expect_err("no route");
    assert!(
        refused
            .client_reason()
            .contains("grok cannot be given a gateway")
    );
}

/// Gives a Claude, a Codex and a Pi session their grants against the real Foundry Local service and writes them to
/// `CANOA_LIVE_OUT/grants.json`, then keeps the gateway up until `CANOA_LIVE_OUT/done` exists, so the three harnesses can be
/// run against it from outside. `CANOA_LIVE_MODEL` is a model the person has loaded.
#[test]
#[ignore = "needs Foundry Local running with CANOA_LIVE_MODEL loaded"]
fn live_gateway_for_the_three_harnesses() {
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .try_init();
    let model = std::env::var("CANOA_LIVE_MODEL").expect("CANOA_LIVE_MODEL names a loaded model");
    let out =
        PathBuf::from(std::env::var("CANOA_LIVE_OUT").expect("CANOA_LIVE_OUT names a folder"));
    std::fs::create_dir_all(&out).expect("a folder");
    let accounts = Accounts::with_store(
        out.join("accounts"),
        Box::new(FileStore::new(out.join("accounts"))),
    )
    .expect("opens");
    accounts
        .store_login("id-foundry", &Credentials::new(), None)
        .expect("the account");
    let mut held = Vec::new();
    let mut written = serde_json::Map::new();
    for harness in ["claude", "codex", "pi"] {
        let info = catalog_of(harness).kind(KIND).expect("the kind").clone();
        let mut account = config(harness);
        account.settings = Some(settings(&model));
        let session = accounts
            .begin("local", &account, &info, Runs::plain(harness), harness)
            .expect("begins on the real service");
        let grant = session.grant();
        let settings = guatiao_serde::adapt::from_value::<serde_json::Value>(&grant.settings)
            .expect("settings");
        written.insert(
            harness.to_owned(),
            json!({"kind": grant.kind, "settings": settings, "credentials": grant.credentials.expose().iter().collect::<std::collections::BTreeMap<_, _>>(), "state_dir": grant.state_dir}),
        );
        held.push(session);
    }
    std::fs::write(
        out.join("grants.json"),
        serde_json::Value::Object(written).to_string(),
    )
    .expect("writes");
    let seconds: u64 = std::env::var("CANOA_LIVE_SECONDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1500);
    for _ in 0..seconds * 2 {
        if out.join("done").exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    drop(held);
}

/// The kind and the sign-in method as the control channel's `accounts/catalog` writes them, which the app's own test reads from
/// the same file. `CANOA_WRITE_VECTORS=1` rewrites it.
#[test]
fn the_catalog_the_app_reads_is_the_one_the_host_writes() {
    fn json<T: coqui_acp_api::guatiao::ToValue>(value: &T) -> serde_json::Value {
        let value = value.to_value().expect("converts");
        serde_json::to_value(guatiao_serde::Serializable::from(&value)).expect("JSON")
    }
    let catalog = catalog_of("claude");
    let written = json!({
        "harness": "claude",
        "kinds": [json(catalog.kind(KIND).expect("the kind"))],
        "logins": [json(&catalog.logins[0])],
    });
    let file =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/vectors/foundry_catalog.json");
    let text = format!(
        "{}\n",
        serde_json::to_string_pretty(&written).expect("text")
    );
    if std::env::var_os("CANOA_WRITE_VECTORS").is_some() {
        std::fs::write(&file, &text).expect("writes");
    }
    assert_eq!(
        std::fs::read_to_string(&file)
            .expect("the vector file")
            .replace("\r\n", "\n"),
        text
    );
}

#[test]
fn a_models_window_in_canoa_cfg_reaches_the_engine_opened_for_it_and_a_model_without_one_gets_none()
{
    let dir = Dir::new();
    let file = dir.0.join("canoa.cfg");
    std::fs::write(
        &file,
        "[foundry.windows]\n\"local-model\" = 16384\n\"other\" = 999\n",
    )
    .expect("a canoa.cfg");
    let seen: Seen = Arc::default();
    let gateways = Gateways::in_home(Some(&dir.0));
    gateways.windows_from(file.clone());
    let record = Arc::clone(&seen);
    gateways.engines(Arc::new(move |settings| {
        record
            .lock()
            .expect("lock")
            .push((settings.model.clone(), settings.window));
        Ok(Arc::new(Local) as Arc<dyn Engine>)
    }));
    let table = |model: &str| {
        let mut map = serde_json::Map::new();
        map.insert("model".to_owned(), json!(model));
        Foundry::from_map(&map).expect("settings")
    };
    let _first = gateways
        .ticket("a", &table("local-model"))
        .expect("a gateway");
    let _second = gateways.ticket("b", &table("plain")).expect("a gateway");
    assert_eq!(
        *seen.lock().expect("lock"),
        [
            ("local-model".to_owned(), Some(16384)),
            ("plain".to_owned(), None)
        ]
    );
    // Changing the file changes what the next gateway is given, with no restart.
    std::fs::write(&file, "[foundry.windows]\n\"plain\" = 2048\n").expect("rewritten");
    let _third = gateways.ticket("c", &table("plain")).expect("a gateway");
    assert_eq!(
        seen.lock().expect("lock").last(),
        Some(&("plain".to_owned(), Some(2048)))
    );
}

#[test]
fn a_host_with_no_canoa_cfg_gives_every_model_its_own_window() {
    let dir = Dir::new();
    let seen: Arc<Mutex<Vec<Option<u64>>>> = Arc::default();
    let gateways = Gateways::in_home(Some(&dir.0));
    let record = Arc::clone(&seen);
    gateways.engines(Arc::new(move |settings| {
        record.lock().expect("lock").push(settings.window);
        Ok(Arc::new(Local) as Arc<dyn Engine>)
    }));
    let mut map = serde_json::Map::new();
    map.insert("model".to_owned(), json!("local-model"));
    let _held = gateways
        .ticket("a", &Foundry::from_map(&map).expect("settings"))
        .expect("a gateway");
    assert_eq!(*seen.lock().expect("lock"), [None]);
}
