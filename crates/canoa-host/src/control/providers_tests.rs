use canoa::preset::SourcePreset;
use canoa::usage_reader::{ProcessMembers, ReaderInfo, UsageReader, UsageReading};
use canoa::vendor::{AgentVendor, StandardTitle, VendorInfo};
use coqui_acp_api::guatiao::Provider;
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::{
    AccountKeeper, AccountKindInfo, AccountModelsQuery, AccountOffer, AccountProvider,
    AccountProviderInfo, AccountUsage, AccountUsageQuery, AccountWatch, LoginMethodInfo,
    LoginOutcome, LoginPrompter, LoginRequest, ProcessSpawner, RenewReport, RenewRequest,
    WatchRequest,
};
use serde_json::Value;

use super::providers::{bundled_registry, gateways_of, rows_of, usage_readers_of};
use crate::bundled::{self, Bundle};
use crate::makers::Makers;

/// An agent no crate of Canoa's names: a vendor description and an account provider, offered by a library of its own.
mod standin {
    use super::*;

    #[derive(Default, Debug, Provider)]
    #[provider(AgentVendor, id = "zed_vendor", crate = ::coqui_acp_api::guatiao)]
    pub(super) struct ZedVendor;

    impl AgentVendor for ZedVendor {
        fn describe(&self) -> Result<VendorInfo, ProviderError> {
            let mut info = VendorInfo::new("zed", "Zed");
            info.vendor = Some("zedco".to_owned());
            info.standards = vec![StandardTitle::new("zed-api", "Zed API")];
            info.mark =
                Some("<svg viewBox=\"0 0 10 10\"><path d=\"M0 0H10V10Z\"/></svg>".to_owned());
            Ok(info)
        }
    }

    #[derive(Default, Debug, Provider)]
    #[provider(AccountProvider, id = "zed_accounts", crate = ::coqui_acp_api::guatiao)]
    pub(super) struct ZedAccounts;

    fn unsupported() -> ProviderError {
        ProviderError::new(
            coqui_acp_api::guatiao::Status::GUATIAO_ERR_UNSUPPORTED,
            "a stand-in",
        )
    }

    impl AccountProvider for ZedAccounts {
        fn describe(&self) -> Result<AccountProviderInfo, ProviderError> {
            Ok(AccountProviderInfo::new("Zed accounts"))
        }

        fn offers(&self) -> Result<AccountOffer, ProviderError> {
            let mut offer = AccountOffer::new();
            offer.kinds.push(AccountKindInfo::new("zed-key", "Zed key"));
            offer
                .logins
                .push(LoginMethodInfo::new("key", "Paste a key", "zed-key"));
            Ok(offer)
        }

        fn login(
            &self,
            _: LoginRequest,
            _: Object<dyn ProcessSpawner>,
            _: Object<dyn LoginPrompter>,
        ) -> Result<LoginOutcome, ProviderError> {
            Err(unsupported())
        }

        fn renew(
            &self,
            _: RenewRequest,
            _: Object<dyn ProcessSpawner>,
        ) -> Result<RenewReport, ProviderError> {
            Err(unsupported())
        }

        fn watch(
            &self,
            _: WatchRequest,
            _: Object<dyn ProcessSpawner>,
            _: Object<dyn AccountKeeper>,
        ) -> Result<Object<dyn AccountWatch>, ProviderError> {
            Err(unsupported())
        }

        fn usage(
            &self,
            _: AccountUsageQuery,
            _: Object<dyn ProcessSpawner>,
            _: Object<dyn AccountKeeper>,
        ) -> Result<AccountUsage, ProviderError> {
            Err(unsupported())
        }

        fn models(
            &self,
            _: AccountModelsQuery,
            _: Object<dyn ProcessSpawner>,
            _: Object<dyn AccountKeeper>,
        ) -> Result<coqui_acp_api::ModelListing, ProviderError> {
            Err(unsupported())
        }
    }

    /// A gateway's account provider: no agent, so no harness and no vendor is its owner.
    #[derive(Default, Debug, Provider)]
    #[provider(AccountProvider, id = "gate_accounts", crate = ::coqui_acp_api::guatiao)]
    pub(super) struct GateAccounts;

    impl AccountProvider for GateAccounts {
        fn describe(&self) -> Result<AccountProviderInfo, ProviderError> {
            Ok(AccountProviderInfo::new("Gate accounts"))
        }

        fn offers(&self) -> Result<AccountOffer, ProviderError> {
            let mut offer = AccountOffer::new();
            offer
                .kinds
                .push(AccountKindInfo::new("gate-key", "Gate key").provides(["gate-api"]));
            Ok(offer)
        }

        fn login(
            &self,
            _: LoginRequest,
            _: Object<dyn ProcessSpawner>,
            _: Object<dyn LoginPrompter>,
        ) -> Result<LoginOutcome, ProviderError> {
            Err(unsupported())
        }

        fn renew(
            &self,
            _: RenewRequest,
            _: Object<dyn ProcessSpawner>,
        ) -> Result<RenewReport, ProviderError> {
            Err(unsupported())
        }

        fn watch(
            &self,
            _: WatchRequest,
            _: Object<dyn ProcessSpawner>,
            _: Object<dyn AccountKeeper>,
        ) -> Result<Object<dyn AccountWatch>, ProviderError> {
            Err(unsupported())
        }

        fn usage(
            &self,
            _: AccountUsageQuery,
            _: Object<dyn ProcessSpawner>,
            _: Object<dyn AccountKeeper>,
        ) -> Result<AccountUsage, ProviderError> {
            Err(unsupported())
        }

        fn models(
            &self,
            _: AccountModelsQuery,
            _: Object<dyn ProcessSpawner>,
            _: Object<dyn AccountKeeper>,
        ) -> Result<coqui_acp_api::ModelListing, ProviderError> {
            Err(unsupported())
        }
    }

    #[derive(Default, Debug, Provider)]
    #[provider(UsageReader, id = "zed_usage", crate = ::coqui_acp_api::guatiao)]
    pub(super) struct ZedUsage;

    impl UsageReader for ZedUsage {
        fn describe(&self) -> Result<ReaderInfo, ProviderError> {
            Ok(ReaderInfo::new("zed", "zed:"))
        }

        fn open(
            &self,
            _: Object<dyn ProcessMembers>,
        ) -> Result<Object<dyn UsageReading>, ProviderError> {
            Err(unsupported())
        }
    }

    coqui_acp_api::guatiao::local_providers!(
        id = "zed",
        version = "0.0.0",
        providers = [ZedVendor, ZedAccounts, GateAccounts, ZedUsage]
    );
}

fn zed() -> Bundle {
    Bundle {
        name: "zed",
        library: standin::library,
        preset: SourcePreset::new("Zed", "zed").agent("zed"),
    }
}

fn rows(bundles: &[Bundle]) -> Vec<Value> {
    let registry = bundled_registry(bundles).expect("the libraries are accepted");
    rows_of(&[("bundled", &registry)], bundles, &Makers::default())
}

fn ids(rows: &[Value]) -> Vec<&str> {
    rows.iter()
        .map(|row| row["id"].as_str().unwrap_or_default())
        .collect()
}

fn row<'a>(rows: &'a [Value], id: &str) -> &'a Value {
    rows.iter()
        .find(|row| row["id"] == id)
        .unwrap_or_else(|| panic!("no row {id} in {rows:?}"))
}

#[test]
fn the_bundled_agents_are_listed_with_what_each_says_of_itself() {
    let rows = rows(bundled::ALL);
    assert_eq!(
        ids(&rows),
        [
            "claude",
            "codex",
            "antigravity",
            "grok",
            "opencode",
            "pi",
            "gemini"
        ],
        "{rows:?}"
    );
    let claude = row(&rows, "claude");
    assert_eq!(claude["title"], "Claude");
    assert_eq!(claude["program"], "Claude Code");
    assert_eq!(claude["vendor"], "anthropic");
    assert_eq!(claude["aos"], "claude-code");
    assert_eq!(claude["source"], "Claude");
    assert_eq!(claude["harness"], true);
    assert!(
        claude["mark"]
            .as_str()
            .is_some_and(|mark| mark.contains("<path"))
    );
    let kinds: Vec<_> = claude["accounts"]["kinds"]
        .as_array()
        .expect("its account provider's kinds")
        .iter()
        .map(|kind| kind["id"].as_str().unwrap_or_default())
        .collect();
    assert!(kinds.contains(&"claude-login"), "{kinds:?}");
    assert!(
        !claude["accounts"]["logins"]
            .as_array()
            .expect("logins")
            .is_empty()
    );
    // A program with no harness provider of its own has no accounts and no harness, and is still an agent.
    let grok = row(&rows, "grok");
    assert_eq!(grok["harness"], false);
    assert!(grok["accounts"].is_null());
    assert_eq!(grok["source"], "Grok");
    assert_eq!(grok["vendor"], "xai");
}

#[test]
fn a_build_without_an_agent_lists_no_trace_of_it() {
    let without: Vec<Bundle> = bundled::ALL
        .iter()
        .filter(|bundle| bundle.name != "canoa-claude")
        .copied()
        .collect();
    let rows = rows(&without);
    assert_eq!(
        ids(&rows),
        ["codex", "antigravity", "grok", "opencode", "pi", "gemini"],
        "{rows:?}"
    );
    let text = serde_json::to_string(&rows).expect("rows are JSON");
    for word in ["claude", "Claude", "anthropic"] {
        assert!(!text.contains(word), "{word} is still listed: {text}");
    }
}

#[test]
fn an_agent_a_library_adds_is_listed_with_its_kinds_like_a_bundled_one() {
    let mut all: Vec<Bundle> = bundled::ALL.to_vec();
    all.push(zed());
    let rows = rows(&all);
    let zed = row(&rows, "zed");
    assert_eq!(zed["title"], "Zed");
    assert_eq!(zed["vendor"], "zedco");
    assert_eq!(zed["standards"][0]["title"], "Zed API");
    assert_eq!(zed["source"], "Zed");
    assert_eq!(zed["harness"], false);
    assert_eq!(zed["accounts"]["provider"], "zed_accounts");
    assert_eq!(zed["accounts"]["title"], "Zed accounts");
    assert_eq!(zed["accounts"]["kinds"][0]["id"], "zed-key");
    assert_eq!(zed["accounts"]["logins"][0]["produces"], "zed-key");
    // The others are listed as before.
    assert_eq!(rows.len(), 8);
}

#[test]
fn a_gateway_a_library_carries_is_listed_with_its_kind_and_vendor_and_its_accounts_are_no_agents() {
    let listed = gateways_of();
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!(listed[0]["kind"], "foundry-local");
    assert_eq!(listed[0]["vendor"], "microsoft-foundry");
    assert_eq!(listed[0]["provider"], "foundry_accounts");
    assert_eq!(listed[0]["title"], "Foundry Local");
    // The kind joins the catalogs of the harnesses that can be given a gateway: no agent of its own is listed for it.
    let registry = bundled_registry(bundled::ALL).expect("the libraries are accepted");
    let rows = rows_of(&[("bundled", &registry)], bundled::ALL, &Makers::default());
    assert!(
        rows.iter().all(|row| row["id"] != "foundry"),
        "no row for a gateway's accounts: {rows:?}"
    );
}

#[test]
fn the_usage_readers_offered_are_listed_bundled_first_then_a_librarys_own() {
    let bundled_only = bundled_registry(bundled::ALL).expect("the libraries are accepted");
    assert_eq!(
        usage_readers_of(&[("bundled", &bundled_only)]),
        [serde_json::json!({ "id": "antigravity", "origin": "bundled" })]
    );
    let library = bundled_registry(&[zed()]).expect("the library is accepted");
    let again = bundled_registry(bundled::ALL).expect("the libraries are accepted");
    assert_eq!(
        usage_readers_of(&[
            ("bundled", &bundled_only),
            ("zed_ext", &library),
            ("other", &again)
        ]),
        [
            serde_json::json!({ "id": "antigravity", "origin": "bundled" }),
            serde_json::json!({ "id": "zed", "origin": "zed_ext" }),
        ],
        "an id some registry has already offered is listed once, as the first"
    );
}

#[test]
fn the_control_method_lists_only_the_loaded_agents_to_a_reader_of_the_config() {
    let control = super::Control::open(
        None,
        super::DEFAULT_MAX_RECORDS,
        super::DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens");
    let line = serde_json::to_vec(
        &serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "providers/list", "params": {}}),
    )
    .expect("encodes");
    let reader = super::Caller::Peer {
        id: "sha256:aa".to_owned(),
        grants: [super::Grant::ConfigRead].into_iter().collect(),
    };
    let answer: Value =
        serde_json::from_slice(&control.handle(&reader, &line).expect("answered")).expect("JSON");
    let listed = answer["result"]["providers"]
        .as_array()
        .unwrap_or_else(|| panic!("{answer}"));
    assert_eq!(
        ids(listed),
        [
            "claude",
            "codex",
            "antigravity",
            "grok",
            "opencode",
            "pi",
            "gemini"
        ]
    );
    assert_eq!(
        answer["result"]["usage_readers"][0]["id"], "antigravity",
        "{answer}"
    );
    let other = super::Caller::Peer {
        id: "sha256:bb".to_owned(),
        grants: [super::Grant::LedgerRead].into_iter().collect(),
    };
    let refused: Value =
        serde_json::from_slice(&control.handle(&other, &line).expect("answered")).expect("JSON");
    assert_eq!(refused["error"]["code"], super::DENIED, "{refused}");
}

#[test]
fn the_apps_fixture_is_what_this_build_lists() {
    let registry = bundled_registry(bundled::ALL).expect("the libraries are accepted");
    let listed = serde_json::json!({
        "providers": rows(bundled::ALL),
        "usage_readers": usage_readers_of(&[("bundled", &registry)]),
        "gateways": gateways_of(),
        "agent_entry": super::agent_entries::agent_entry(),
        "local_agent": super::agent_entries::local_agent(),
    });
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../app/test/fixtures/providers_list.json");
    if std::env::var_os("CANOA_WRITE_PROVIDERS").is_some() {
        let mut text = serde_json::to_string_pretty(&listed).expect("rows are JSON");
        text.push('\n');
        std::fs::write(&path, text).expect("the fixture is written");
    }
    let held: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("the fixture is committed"))
            .expect("the fixture is JSON");
    assert!(
        held == listed,
        "regenerate it: CANOA_WRITE_PROVIDERS=1 cargo test -p canoa-host --lib providers_tests"
    );
}

#[test]
fn an_account_provider_no_agent_owns_is_a_maker_in_any_library_and_is_listed_as_no_agent() {
    // `zed_accounts` is the account provider of the agent `zed` its library describes; `gate_accounts` is nobody's.
    let mut all: Vec<Bundle> = bundled::ALL.to_vec();
    all.push(zed());
    let registry = bundled_registry(&all).expect("the libraries are accepted");
    let made: Vec<String> = crate::makers::makers_in(&registry)
        .into_iter()
        .map(|maker| maker.provider)
        .collect();
    assert!(made.iter().any(|id| id == "gate_accounts"), "{made:?}");
    assert!(made.iter().all(|id| id != "zed_accounts"), "{made:?}");
    let rows = rows_of(&[("zed", &registry)], &all, &Makers::default());
    assert!(
        rows.iter().all(|row| row["id"] != "gate"),
        "a maker's provider is no agent row: {rows:?}"
    );
    assert_eq!(row(&rows, "zed")["accounts"]["provider"], "zed_accounts");
}
