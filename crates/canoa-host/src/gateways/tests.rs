//! The gateways of the libraries this host carries: the kind of a gateway's accounts in a harness's catalog, the sign-in that makes
//! one, and the grant a session is given. A stand-in gateway answers for the library's, which `canoa-foundry`'s own tests run.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use canoa::gateway::{GatewayInfo, GatewayLease, GatewayRequest, ModelGateway};
use canoa_foundry::FoundryAccounts;
use canoa_foundry::account::KIND;
use coqui_acp_api::guatiao::Status;
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::{
    AccountCatalog, AccountKeeper, AccountKindInfo, AccountOffer, AccountProvider,
    AccountProviderInfo, AccountUsage, AccountUsageQuery, AccountWatch, LoginAnswer, LoginForm,
    LoginOutcome, LoginRequest, OpenUrl, RenewReport, RenewRequest, WatchRequest, standard,
};
use serde_json::json;

use super::*;
use crate::accounts::login::{Interaction, sign_in};
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
            "canoa-gateways-{}-{}",
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

/// What a stand-in gateway was asked, and how many leases of it are held.
#[derive(Default)]
struct Record {
    asked: Mutex<Vec<GatewayRequest>>,
    live: AtomicUsize,
    numbered: AtomicUsize,
}

impl Record {
    fn asked(&self) -> Vec<GatewayRequest> {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// A gateway that gives each session a bearer of its own at one address, or refuses with the reason it was built with.
struct Stand {
    record: Arc<Record>,
    refuses: Option<String>,
}

struct Lease {
    number: usize,
    record: Arc<Record>,
}

impl GatewayLease for Lease {
    fn address(&self) -> Result<String, ProviderError> {
        Ok("http://127.0.0.1:4000".to_owned())
    }

    fn bearer(&self) -> Result<String, ProviderError> {
        Ok(format!("bearer-{}", self.number))
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        self.record.live.fetch_sub(1, Ordering::SeqCst);
    }
}

impl ModelGateway for Stand {
    fn describe(&self) -> Result<GatewayInfo, ProviderError> {
        let mut info = GatewayInfo::new(KIND, "stand-in");
        info.table = Some("stand".to_owned());
        Ok(info)
    }

    fn open(
        &self,
        request: GatewayRequest,
        _spawner: Object<dyn ProcessSpawner>,
    ) -> Result<Object<dyn GatewayLease>, ProviderError> {
        self.record
            .asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        if let Some(why) = &self.refuses {
            return Err(ProviderError::new(Status::GUATIAO_ERR_INTERNAL, why));
        }
        self.record.live.fetch_add(1, Ordering::SeqCst);
        let number = self.record.numbered.fetch_add(1, Ordering::SeqCst);
        Ok(Lease {
            number,
            record: Arc::clone(&self.record),
        }
        .into_object())
    }
}

fn stand_in(refuses: Option<&str>) -> (Offer, Arc<Record>) {
    let record = Arc::new(Record::default());
    let gateway = Stand {
        record: Arc::clone(&record),
        refuses: refuses.map(str::to_owned),
    };
    let offer = Offer::new(
        Arc::new(gateway),
        "stand_accounts",
        Arc::new(FoundryAccounts),
    )
    .expect("an offer");
    (offer, record)
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
        id: "id-local".to_owned(),
        settings: Some(settings("local-model")),
    }
}

fn accounts(dir: &Dir, offer: Offer) -> Arc<Accounts> {
    let accounts = Accounts::with_store(
        dir.0.join("accounts"),
        Box::new(FileStore::new(dir.0.join("accounts"))),
    )
    .expect("opens");
    accounts.gateways().serving(vec![offer]);
    accounts
        .store_login("id-local", &Credentials::new(), None)
        .expect("the account is made, and holds no credential");
    accounts
}

fn text(grant: &AccountGrant, key: &str) -> String {
    let settings = guatiao_serde::adapt::from_value::<serde_json::Map<String, serde_json::Value>>(
        &grant.settings,
    )
    .expect("settings");
    settings[key].as_str().expect("text").to_owned()
}

fn bearer(grant: &AccountGrant, key: &str) -> String {
    grant
        .credentials
        .expose()
        .get(key)
        .expect("a bearer")
        .to_owned()
}

/// A sign-in that never asks.
struct Mute;

impl Interaction for Mute {
    fn say(&self, _text: &str) {}

    fn open_url(&self, _open: &OpenUrl) {}

    fn ask(&self, _form: &LoginForm) -> Result<LoginAnswer, String> {
        Err("nothing to ask".to_owned())
    }

    fn cancelled(&self) -> bool {
        false
    }
}

/// An account provider that makes nothing: the harness's own, which lists no sign-in method of a gateway's kind.
struct Plain;

impl AccountProvider for Plain {
    fn describe(&self) -> Result<AccountProviderInfo, ProviderError> {
        Ok(AccountProviderInfo::new("Plain"))
    }

    fn offers(&self) -> Result<AccountOffer, ProviderError> {
        Ok(AccountOffer::new())
    }

    fn login(
        &self,
        _request: LoginRequest,
        _spawner: Object<dyn ProcessSpawner>,
        _prompter: Object<dyn coqui_acp_api::LoginPrompter>,
    ) -> Result<LoginOutcome, ProviderError> {
        Err(ProviderError::new(
            Status::GUATIAO_ERR_UNSUPPORTED,
            "the harness's own provider makes no such account",
        ))
    }

    fn renew(
        &self,
        _request: RenewRequest,
        _spawner: Object<dyn ProcessSpawner>,
    ) -> Result<RenewReport, ProviderError> {
        Err(ProviderError::new(Status::GUATIAO_ERR_UNSUPPORTED, "no"))
    }

    fn watch(
        &self,
        _request: WatchRequest,
        _spawner: Object<dyn ProcessSpawner>,
        _keeper: Object<dyn AccountKeeper>,
    ) -> Result<Object<dyn AccountWatch>, ProviderError> {
        Err(ProviderError::new(Status::GUATIAO_ERR_UNSUPPORTED, "no"))
    }

    fn usage(
        &self,
        _query: AccountUsageQuery,
        _spawner: Object<dyn ProcessSpawner>,
        _keeper: Object<dyn AccountKeeper>,
    ) -> Result<AccountUsage, ProviderError> {
        Err(ProviderError::new(Status::GUATIAO_ERR_UNSUPPORTED, "no"))
    }
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
fn the_library_offers_the_gateway_and_the_host_finds_its_kind_provider_and_vendor() {
    let offer = for_kind(KIND).expect("a gateway for the kind");
    assert_eq!(offer.provider, "foundry_accounts");
    assert_eq!(vendor_of(KIND), Some("microsoft-foundry"));
    assert_eq!(vendor_of("anthropic-api-key"), None);
    assert!(account_providers().any(|id| id == "foundry_accounts"));
    assert!(login_provider(KIND).is_some());
    assert!(login_provider("claude-login").is_none());
}

#[test]
fn signing_in_names_a_model_and_keeps_no_credential_whichever_provider_the_harness_has() {
    let dir = Dir::new();
    let (offer, _) = stand_in(None);
    let accounts = accounts(&dir, offer);
    let mut inputs = serde_json::Map::new();
    inputs.insert("model".to_owned(), json!("qwen3-14b-generic-gpu:2"));
    // The harness's own provider makes no such account: the library's does, and the host finds it by the method.
    let outcome =
        sign_in(&accounts, &Plain, KIND, &inputs, Arc::new(Mute), None).expect("signs in");
    assert_eq!(outcome.kind, KIND);
    assert_eq!(outcome.label.as_deref(), Some("qwen3-14b-generic-gpu:2"));
    assert!(outcome.credentials.expose().is_empty());
    let refused = sign_in(
        &accounts,
        &Plain,
        KIND,
        &serde_json::Map::new(),
        Arc::new(Mute),
        None,
    )
    .expect_err("a model is named");
    assert!(refused.to_string().contains("model"), "{refused}");
    let own = sign_in(
        &accounts,
        &FoundryAccounts,
        KIND,
        &inputs,
        Arc::new(Mute),
        None,
    )
    .expect("its own provider signs in the same");
    assert_eq!(own.kind, KIND);
}

#[test]
fn a_claude_session_is_given_a_gateway_token_and_its_address() {
    let dir = Dir::new();
    let (offer, record) = stand_in(None);
    let accounts = accounts(&dir, offer);
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
    assert_eq!(text(&grant, "base_url"), "http://127.0.0.1:4000");
    assert_eq!(bearer(&grant, "auth_token"), "bearer-0");
    assert_eq!(
        held.report()["kind"],
        KIND,
        "the account is still the one the library made"
    );
    assert_eq!(record.live.load(Ordering::SeqCst), 1);
    drop(held);
    assert_eq!(
        record.live.load(Ordering::SeqCst),
        0,
        "the session's lease went with it"
    );
}

#[test]
fn the_gateway_is_asked_for_the_account_its_settings_its_home_its_own_table_and_the_hosts_helper() {
    let dir = Dir::new();
    let (offer, record) = stand_in(None);
    let accounts = accounts(&dir, offer);
    let file = dir.0.join("canoa.cfg");
    std::fs::write(&file, "[stand.windows]\n\"local-model\" = 16384\n").expect("a canoa.cfg");
    accounts.config_from(file);
    accounts.helper_program(crate::cli::Program::new(
        PathBuf::from("host-program"),
        vec!["--canoa-child".into()],
    ));
    let info = catalog_of("claude").kind(KIND).expect("the kind").clone();
    let _held = accounts
        .begin(
            "local",
            &config("claude"),
            &info,
            Runs::plain("claude"),
            "a session",
        )
        .expect("begins");
    let asked = record.asked();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].account, "id-local");
    let kept: serde_json::Value = serde_json::from_str(&asked[0].settings).expect("JSON");
    assert_eq!(kept, json!({"model": "local-model"}));
    let table: serde_json::Value = serde_json::from_str(&asked[0].table).expect("JSON");
    assert_eq!(table, json!({"windows": {"local-model": 16384}}));
    assert_eq!(
        asked[0].home.as_deref().map(std::path::Path::new),
        Some(dir.0.as_path()),
        "the folder the accounts' own folder is in"
    );
    assert_eq!(asked[0].helper, ["host-program", "--canoa-child", "helper"]);
}

#[test]
fn a_host_with_no_canoa_cfg_and_no_program_of_its_own_asks_for_no_table_and_no_helper() {
    let dir = Dir::new();
    let (offer, record) = stand_in(None);
    let accounts = accounts(&dir, offer);
    let info = catalog_of("claude").kind(KIND).expect("the kind").clone();
    let _held = accounts
        .begin(
            "local",
            &config("claude"),
            &info,
            Runs::plain("claude"),
            "a session",
        )
        .expect("begins");
    let asked = record.asked();
    assert_eq!(asked[0].table, "{}");
    assert!(asked[0].helper.is_empty());
}

#[test]
fn a_codex_session_is_given_an_api_key_the_v1_address_and_a_folder_that_turns_off_what_a_gateway_cannot_serve()
 {
    let dir = Dir::new();
    let (offer, _) = stand_in(None);
    let accounts = accounts(&dir, offer);
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
    assert_eq!(text(&grant, "openai_base_url"), "http://127.0.0.1:4000/v1");
    assert_eq!(bearer(&grant, "api_key"), "bearer-0");
    let folder = PathBuf::from(grant.state_dir.as_deref().expect("a folder"));
    let config = std::fs::read_to_string(folder.join("config.toml")).expect("written");
    assert!(config.contains("multi_agent = false") && config.contains("web_search = \"disabled\""));
}

#[test]
fn a_pi_session_is_given_an_api_key_in_the_openai_variable_and_a_folder_that_names_the_gateway() {
    let dir = Dir::new();
    let (offer, _) = stand_in(None);
    let accounts = accounts(&dir, offer);
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
    assert_eq!(bearer(&grant, "api_key"), "bearer-0");
    let folder = PathBuf::from(grant.state_dir.as_deref().expect("a folder"));
    let models: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(folder.join("models.json")).expect("written"),
    )
    .expect("JSON");
    assert_eq!(
        models["providers"]["openai"]["baseUrl"],
        "http://127.0.0.1:4000/v1"
    );
}

#[test]
fn an_account_made_for_one_harness_runs_another_in_that_harness_s_own_kind_and_keeps_its_standards()
{
    let dir = Dir::new();
    let (offer, _) = stand_in(None);
    let accounts = accounts(&dir, offer);
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
fn sessions_on_one_account_each_hold_a_lease_of_their_own_and_let_it_go_with_the_session() {
    let dir = Dir::new();
    let (offer, record) = stand_in(None);
    let accounts = accounts(&dir, offer);
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
    assert_ne!(
        bearer(&one.grant(), "auth_token"),
        bearer(&two.grant(), "auth_token")
    );
    assert_eq!(record.live.load(Ordering::SeqCst), 2);
    drop(one);
    assert_eq!(record.live.load(Ordering::SeqCst), 1);
    drop(two);
    assert_eq!(record.live.load(Ordering::SeqCst), 0);
}

#[test]
fn a_gateway_that_cannot_start_is_a_refusal_the_person_can_read_and_lets_the_account_go() {
    let dir = Dir::new();
    let (offer, _) = stand_in(Some("no model is loaded"));
    let broken = accounts(&dir, offer);
    let info = catalog_of("claude").kind(KIND).expect("the kind").clone();
    let refused = broken
        .begin(
            "local",
            &config("claude"),
            &info,
            Runs::plain("claude"),
            "a session",
        )
        .expect_err("no model");
    assert_eq!(refused, Refusal::Gateway("no model is loaded".to_owned()));
    assert!(refused.client_reason().contains("no model is loaded"));
    assert!(!broken.held("id-local"), "the account was let go");
    // A harness no gateway can be given to is refused by name.
    let (offer, _) = stand_in(None);
    let other = Dir::new();
    let accounts = accounts(&other, offer);
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
