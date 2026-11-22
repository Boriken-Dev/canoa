//! An account whose kind provides several standards is one account wherever it is used: a stand-in maker's kind provides two
//! standards, and the harnesses that accept either offer it, the host's pools of both hold it, and one usage report stops it in all.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use coqui_acp_api::guatiao::Status;
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::{
    AccountKeeper, AccountKindInfo, AccountModelsQuery, AccountOffer, AccountProvider,
    AccountProviderInfo, AccountUsage, AccountUsageQuery, AccountWatch, Bucket, CacheLifetime,
    Coverage, Credentials, LoginMethodInfo, LoginOutcome, LoginPrompter, LoginRequest,
    ModelListing, ProcessSpawner, Refill, RenewReport, RenewRequest, WatchRequest, standard,
};

use super::*;
use crate::accounts::Accounts;
use crate::accounts::store::FileStore;
use crate::config::Config;
use crate::kind;
use crate::options::limits::Endpoints;
use crate::options::{harness_of, offering};
use crate::pools::gather::candidates;
use crate::pools::turn::{Candidate, Cursor, Turn, decide};

/// The kind the stand-in makes, and the account that holds it.
const KIND: &str = "standin-two";

struct Dir(PathBuf);

impl Dir {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "canoa-makers-{}-{}",
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

/// An account maker whose one kind provides two standards, and whose usage says what the test set: how many times it was asked,
/// and whether the account's one allowance is used up.
struct Two {
    asked: Arc<AtomicUsize>,
    used_up: Arc<AtomicBool>,
}

fn refuse() -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_UNSUPPORTED, "a stand-in does not")
}

impl AccountProvider for Two {
    fn describe(&self) -> Result<AccountProviderInfo, ProviderError> {
        Ok(AccountProviderInfo::new("Stand-in"))
    }

    fn offers(&self) -> Result<AccountOffer, ProviderError> {
        let mut offer = AccountOffer::new();
        offer.kinds = vec![
            AccountKindInfo::new(KIND, "Stand-in")
                .provides([standard::ANTHROPIC_API, standard::OPENAI_API]),
        ];
        offer.logins = vec![LoginMethodInfo::new("standin-key", "A key", KIND)];
        Ok(offer)
    }

    fn provides(&self, account: coqui_acp_api::AccountRef) -> Result<Vec<String>, ProviderError> {
        self.offers()?
            .kind(&account.kind)
            .map(|kind| kind.provides.clone())
            .ok_or_else(|| {
                ProviderError::new(
                    coqui_acp_api::guatiao::Status::GUATIAO_ERR_BAD_VALUE,
                    "no such kind",
                )
            })
    }

    fn login(
        &self,
        _request: LoginRequest,
        _spawner: Object<dyn ProcessSpawner>,
        _prompter: Object<dyn LoginPrompter>,
    ) -> Result<LoginOutcome, ProviderError> {
        Ok(LoginOutcome::new(KIND, Credentials::new()))
    }

    fn renew(
        &self,
        _request: RenewRequest,
        _spawner: Object<dyn ProcessSpawner>,
    ) -> Result<RenewReport, ProviderError> {
        Err(refuse())
    }

    fn watch(
        &self,
        _request: WatchRequest,
        _spawner: Object<dyn ProcessSpawner>,
        _keeper: Object<dyn AccountKeeper>,
    ) -> Result<Object<dyn AccountWatch>, ProviderError> {
        Err(refuse())
    }

    fn usage(
        &self,
        query: AccountUsageQuery,
        _spawner: Object<dyn ProcessSpawner>,
        _keeper: Object<dyn AccountKeeper>,
    ) -> Result<AccountUsage, ProviderError> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        let mut bucket = Bucket::new("spend");
        bucket.models = Coverage::All;
        bucket.refill = Refill::FixedWindow {
            length: None,
            resets_at: Some(4_000_000_000),
        };
        bucket.status = if self.used_up.load(Ordering::SeqCst) {
            coqui_acp_api::Status::Exhausted
        } else {
            coqui_acp_api::Status::Ok
        };
        let mut usage = AccountUsage::new();
        usage.account = Some(query.account.id);
        usage.buckets = vec![bucket];
        Ok(usage)
    }

    fn models(
        &self,
        query: AccountModelsQuery,
        _spawner: Object<dyn ProcessSpawner>,
        _keeper: Object<dyn AccountKeeper>,
    ) -> Result<ModelListing, ProviderError> {
        // One model for each standard the grant provides.
        Ok(ModelListing::of(
            query
                .account
                .provides
                .iter()
                .map(|standard| coqui_acp_api::ModelInfo::new(format!("model-of-{standard}")))
                .collect(),
        ))
    }
}

fn maker(asked: &Arc<AtomicUsize>, used_up: &Arc<AtomicBool>) -> Maker {
    let provider: Arc<dyn AccountProvider> = Arc::new(Two {
        asked: Arc::clone(asked),
        used_up: Arc::clone(used_up),
    });
    let accounts = provider.offers().expect("offers");
    Maker::new("standin_accounts", accounts, provider)
}

const CONFIG: &str = "\
[host.agents.Claude]\nharness = \"claude\"\naccounts = [\"shared\"]\n\
[host.agents.Codex]\nharness = \"codex\"\naccounts = [\"shared\"]\n\
[host.accounts.shared]\nkind = \"standin-two\"\nid = \"id-shared\"\n";

fn store(dir: &Dir) -> Arc<Accounts> {
    let root = dir.0.join("accounts");
    let store = Accounts::with_store(root.clone(), Box::new(FileStore::new(root))).expect("opens");
    store
        .store_login(
            "id-shared",
            &Credentials::from_iter([("api_key", "k-shared")]),
            None,
        )
        .expect("stored");
    store
}

fn turn(current: Option<&str>) -> Turn<'_> {
    Turn {
        current,
        model: None,
        idle: None,
        lifetime: CacheLifetime::Default,
        now: 1_800_000_000,
    }
}

#[test]
fn one_account_of_two_standards_runs_both_harnesses_sits_in_both_pools_and_one_report_stops_it_in_both()
 {
    let (asked, used_up) = (
        Arc::new(AtomicUsize::new(0)),
        Arc::new(AtomicBool::new(false)),
    );
    let _serving = stand_ins::serving(vec![maker(&asked, &used_up)]);
    let dir = Dir::new();
    let store = store(&dir);
    let config = Config::from_str_in(CONFIG, std::path::Path::new("")).expect("parses");

    // Each harness accepts one of the two standards, and offers the account and the host's pool of its standard.
    let mut built = Vec::new();
    for (entry, pool) in [("Claude", "any-anthropic-api"), ("Codex", "any-openai-api")] {
        let entry = config.entry(entry).expect("an entry");
        let harness = harness_of(entry, Arc::default()).expect("a provider");
        let harness = offering(&config, entry, harness, &store, true).expect("offered");
        let (_, account, info) = harness
            .account_named("shared", false)
            .expect("offered here");
        assert_eq!(account.owner, None, "an account names no harness");
        assert_eq!(
            info.provides,
            [standard::ANTHROPIC_API, standard::OPENAI_API]
        );
        let pool = harness
            .pool_named(pool, true)
            .expect("the host's pool of its standard");
        assert!(
            pool.members
                .iter()
                .any(|member| member.name == "shared" && member.listed),
            "{pool:?}"
        );
        built.push(harness);
    }
    assert_eq!(
        config.accounts.len(),
        1,
        "one account, never a copy per harness"
    );

    // Both pools ask for its usage: it is read once, and each is told the same.
    let names = ["shared".to_owned()];
    let read = |harness: &kind::Harness| {
        candidates(
            &config,
            harness,
            &names,
            (None, None),
            (&[], &Endpoints::default()),
            true,
        )
    };
    let room = [read(&built[0]), read(&built[1])];
    assert_eq!(
        asked.load(Ordering::SeqCst),
        1,
        "one report serves both pools"
    );
    for pool in &room {
        let usage = pool[0].usage.as_ref().expect("a report");
        assert_eq!(usage.buckets.len(), 1);
        let one: Vec<Candidate> = vec![
            pool[0].clone(),
            Candidate {
                name: "other".to_owned(),
                ..Candidate::default()
            },
        ];
        let chosen = decide("p", &Cursor::default(), &one, &turn(None)).expect("room");
        assert_eq!(chosen.account, "shared", "room is left");
    }

    // The same account used up stops it in both pools, from the one report.
    used_up.store(true, Ordering::SeqCst);
    let kept = crate::pools::gather::forget_reports();
    assert!(kept >= 1);
    for harness in &built {
        let pool = read(harness);
        let one = vec![
            pool[0].clone(),
            Candidate {
                name: "other".to_owned(),
                ..Candidate::default()
            },
        ];
        let chosen = decide("p", &Cursor::default(), &one, &turn(None)).expect("another has room");
        assert_eq!(chosen.account, "other", "the used-up account is left");
    }
    assert_eq!(asked.load(Ordering::SeqCst), 2, "one more report, for both");
}

#[test]
fn an_account_of_two_standards_is_listed_once_for_every_harness_that_runs_it() {
    let (asked, used_up) = (
        Arc::new(AtomicUsize::new(0)),
        Arc::new(AtomicBool::new(false)),
    );
    let _serving = stand_ins::serving(vec![maker(&asked, &used_up)]);
    let dir = Dir::new();
    let store = store(&dir);
    let config = Config::from_str_in(CONFIG, std::path::Path::new("")).expect("parses");
    for entry in ["Claude", "Codex"] {
        let entry = config.entry(entry).expect("an entry");
        let harness = harness_of(entry, Arc::default()).expect("a provider");
        let harness = offering(&config, entry, harness, &store, true).expect("offered");
        let listed = crate::options::as_account(&config, &harness, "shared", false);
        assert!(listed.known, "{:?}", listed.unknown_because);
        // The maker is given every standard the account provides, so one list serves both harnesses.
        let ids: Vec<&str> = listed
            .models
            .iter()
            .map(|model| model.id.as_str())
            .collect();
        assert_eq!(
            ids,
            ["model-of-anthropic-api", "model-of-openai-api"],
            "{}",
            entry.name
        );
    }
}

#[test]
fn an_account_a_maker_made_fits_only_where_a_standard_it_provides_is_accepted_the_harness_that_names_it_included()
 {
    use crate::accounts::matching::{borrowed, fits};
    let (asked, used_up) = (
        Arc::new(AtomicUsize::new(0)),
        Arc::new(AtomicBool::new(false)),
    );
    let _serving = stand_ins::serving(vec![maker(&asked, &used_up)]);
    let account = crate::config::AccountConfig {
        owner: None,
        kind: KIND.to_owned(),
        id: "id".to_owned(),
        settings: None,
    };
    let lists = |kind: &str| {
        let mut catalog = coqui_acp_api::AccountCatalog::new();
        catalog.kinds.push(AccountKindInfo::new(kind, "K"));
        catalog
    };
    assert!(
        borrowed(&Makers::default(), &account, "claude", Some(&lists(KIND))),
        "a maker's kind is applied as a standard, by every harness"
    );
    let own = crate::config::AccountConfig {
        kind: "claude-login".to_owned(),
        ..account.clone()
    };
    assert!(
        !borrowed(
            &Makers::default(),
            &own,
            "claude",
            Some(&lists("claude-login"))
        ) && borrowed(
            &Makers::default(),
            &own,
            "codex",
            Some(&lists("codex-chatgpt"))
        )
    );
    let mut catalog = coqui_acp_api::AccountCatalog::new();
    catalog.accepts = vec![coqui_acp_api::Accepted::new(standard::ANTHROPIC_API)];
    let only_openai = AccountKindInfo::new(KIND, "K").provides([standard::OPENAI_API]);
    let why = fits(
        &Makers::default(),
        &catalog,
        "claude",
        &account,
        &only_openai,
    )
    .expect_err("it speaks only a standard Claude does not accept, though Claude named it");
    assert!(
        why.contains("openai-api") && why.contains("anthropic-api"),
        "{why}"
    );
}

#[test]
fn a_kind_that_names_an_address_fits_only_a_harness_that_can_point_its_standard_at_it() {
    use crate::accounts::matching::fits;
    use coqui_acp_api::{Accepted, AcceptedSlot, AccountCatalog};
    let catalog = |slot: AcceptedSlot| {
        let mut catalog = AccountCatalog::new();
        catalog.accepts = vec![Accepted::new(standard::OPENAI_API).slot(slot)];
        catalog
    };
    let moves = catalog(AcceptedSlot::new("api_key", "own-key").base_url("openai_base_url", None));
    let fixed = catalog(AcceptedSlot::new("api_key", "own-key"));
    let account = crate::config::AccountConfig {
        owner: None,
        kind: "litellm".to_owned(),
        id: "id".to_owned(),
        settings: Some(crate::config::Settings::new(
            serde_json::json!({"base_url": "https://llm.example.com"})
                .as_object()
                .cloned()
                .expect("a table"),
        )),
    };
    let info = AccountKindInfo::new("litellm", "L").provides([standard::OPENAI_API]);
    assert!(fits(&Makers::default(), &moves, "codex", &account, &info).is_ok());
    let why = fits(&Makers::default(), &fixed, "pi", &account, &info)
        .expect_err("it cannot be pointed at the address");
    assert!(
        why.contains("pi") && why.contains("openai-api") && why.contains("address"),
        "{why}"
    );
}

#[test]
fn a_library_canoa_cfg_names_cannot_take_a_kind_or_a_sign_in_method_an_earlier_library_offers() {
    let mut taken = Taken::default();
    for maker in offered() {
        taken.note(&maker.accounts);
    }
    let provider: Arc<dyn AccountProvider> = Arc::new(Two {
        asked: Arc::new(AtomicUsize::new(0)),
        used_up: Arc::new(AtomicBool::new(false)),
    });
    let offer = |kinds: &[&str], logins: &[(&str, &str)]| {
        let mut offer = AccountOffer::new();
        offer.kinds = kinds
            .iter()
            .map(|id| AccountKindInfo::new(*id, "K").provides([standard::OPENAI_API]))
            .collect();
        offer.logins = logins
            .iter()
            .map(|(id, produces)| LoginMethodInfo::new(*id, "M", *produces))
            .collect();
        Maker::new("ext_accounts", offer, Arc::clone(&provider))
    };
    // A kind a bundled library offers stays the bundled one's, and with it the sign-in methods that make it; another sign-in method
    // whose id a bundled library uses is left out too, and the rest are kept.
    let bundled_kind = offered()[0].accounts.kinds[0].id.clone();
    let bundled_method = offered()[0].accounts.logins[0].id.clone();
    let claimed = taken
        .claim(
            offer(
                &[&bundled_kind, "ext-kind"],
                &[
                    (&bundled_method, "ext-kind"),
                    ("ext-key", "ext-kind"),
                    ("ext-other", &bundled_kind),
                ],
            ),
            "examples",
        )
        .expect("one kind is its own");
    let kinds: Vec<&str> = claimed
        .accounts
        .kinds
        .iter()
        .map(|kind| kind.id.as_str())
        .collect();
    let methods: Vec<&str> = claimed
        .accounts
        .logins
        .iter()
        .map(|login| login.id.as_str())
        .collect();
    assert_eq!(kinds, ["ext-kind"]);
    assert_eq!(methods, ["ext-key"]);
    // What it kept is now taken: a later library offering the same kind offers nothing, and so does one that offers only a bundled kind.
    assert!(
        taken
            .claim(offer(&["ext-kind"], &[("again", "ext-kind")]), "later")
            .is_none()
    );
    assert!(
        taken
            .claim(offer(&[&bundled_kind], &[]), "bundled-only")
            .is_none()
    );
}
