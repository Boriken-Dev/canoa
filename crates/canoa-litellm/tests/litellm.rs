//! A LiteLLM account against a loopback stand-in for the proxy: signing in through its SSO (a stand-in plays the browser), setting a
//! key, the models it lists, the usage it reports as buckets, and the renewal of a signed-in key by its refresh token.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod standin;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use canoa::api_gateway::value;
use canoa_litellm::LiteLlmAccounts;
use canoa_litellm::account::{API_KEY, CLIENT_ID, KIND, REFRESH_TOKEN, SET_KEY, SIGN_IN};
use coqui_acp_api::guatiao::Status;
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::{
    AccountGrant, AccountKeeper, AccountLoss, AccountModelsQuery, AccountProvider, AccountRenewal,
    AccountUsageQuery, Credentials, LoginAnswer, LoginForm, LoginPrompter, LoginRequest, OpenUrl,
    ProcessSpawner, RenewRequest, UnknownKind,
};
use coqui_acp_api::{Refill, Status as BucketStatus, Unit};
use coqui_acp_harness::LocalSpawner;
use serde_json::{Value, json};
use standin::{Proxy, VIRTUAL_KEY};

/// What the stand-in browser is told and what the person is asked.
struct Person {
    proxy: Arc<Proxy>,
    /// The key the person types when asked for one.
    key: String,
    /// Whether the person closes the page instead of signing in.
    gives_up: AtomicBool,
    /// Something to send the loopback address before the real redirect.
    stray: Option<String>,
    pages: Arc<Mutex<Vec<String>>>,
}

impl LoginPrompter for Person {
    fn say(&self, _text: &str) -> Result<(), ProviderError> {
        Ok(())
    }

    fn open_url(&self, open: OpenUrl) -> Result<(), ProviderError> {
        self.pages
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(open.url.clone());
        if self.gives_up.load(Ordering::SeqCst) {
            return Ok(());
        }
        let (proxy, stray, url) = (Arc::clone(&self.proxy), self.stray.clone(), open.url);
        // A browser goes on its own once the page is open.
        std::thread::spawn(move || {
            if let Some(stray) = stray {
                let redirect = standin::query_of(&url)["redirect_uri"].clone();
                let _ = standin::get(&format!("{redirect}{stray}"));
            }
            proxy.browse(&url);
        });
        Ok(())
    }

    fn ask(&self, _form: LoginForm) -> Result<LoginAnswer, ProviderError> {
        let mut secrets = Credentials::new();
        secrets.insert("api_key", format!(" {}\n", self.key));
        Ok(LoginAnswer::new(value(&json!({})), secrets))
    }

    fn cancelled(&self) -> Result<bool, ProviderError> {
        Ok(self.gives_up.load(Ordering::SeqCst))
    }
}

struct NoKeeper;

impl AccountKeeper for NoKeeper {
    fn renewed(&self, _renewal: AccountRenewal) -> Result<(), ProviderError> {
        Ok(())
    }

    fn lost(&self, _loss: AccountLoss) -> Result<(), ProviderError> {
        Ok(())
    }
}

fn person(proxy: &Arc<Proxy>) -> Person {
    Person {
        proxy: Arc::clone(proxy),
        key: VIRTUAL_KEY.to_owned(),
        gives_up: AtomicBool::new(false),
        stray: None,
        pages: Arc::default(),
    }
}

fn spawner() -> Object<dyn ProcessSpawner> {
    LocalSpawner::new().into_object()
}

fn sign_in(
    method: &str,
    base: &str,
    person: Person,
    timeout_ms: u64,
) -> Result<coqui_acp_api::LoginOutcome, ProviderError> {
    let mut request = LoginRequest::new(method, "unused").inputs(value(&json!({"base_url": base})));
    request.timeout_ms = timeout_ms;
    LiteLlmAccounts.login(request, spawner(), person.into_object())
}

fn grant(base: &str, held: &[(&str, &str)]) -> AccountGrant {
    AccountGrant::new("id-one", KIND)
        .settings(value(&json!({"base_url": base})))
        .credentials(Credentials::from_iter(held.iter().copied()))
        .provides(["openai-api", "anthropic-api"])
}

fn usage(grant: AccountGrant) -> Result<coqui_acp_api::AccountUsage, ProviderError> {
    LiteLlmAccounts.usage(
        AccountUsageQuery::new(grant),
        spawner(),
        NoKeeper.into_object(),
    )
}

#[test]
fn signing_in_through_the_proxys_sso_hands_back_a_key_and_what_renews_it() {
    let proxy = Arc::new(Proxy::start());
    let who = person(&proxy);
    let pages = Arc::clone(&who.pages);
    let outcome = sign_in(SIGN_IN, &proxy.address, who, 30_000).expect("signed in");
    assert_eq!(outcome.kind, KIND);
    assert_eq!(outcome.label.as_deref(), Some("jo@example.com"));
    let held = outcome.credentials.expose();
    assert_eq!(held.get(API_KEY), Some("sk-issued-1"));
    assert_eq!(held.get(REFRESH_TOKEN), Some("refresh-1"));
    assert!(
        held.get(CLIENT_ID)
            .is_some_and(|client| client.starts_with("client-"))
    );
    // It lasts the day the proxy said, as a time the host can read.
    let expires = time::OffsetDateTime::parse(
        outcome.expires_at.as_deref().expect("an expiry"),
        &time::format_description::well_known::Rfc3339,
    )
    .expect("RFC 3339");
    let left = expires.unix_timestamp() - time::OffsetDateTime::now_utc().unix_timestamp();
    assert!((86_300..=86_400).contains(&left), "{left}");
    // The settings name the proxy and nothing secret.
    let settings =
        guatiao_serde::adapt::from_value::<serde_json::Map<String, Value>>(&outcome.settings)
            .expect("settings");
    assert_eq!(Value::Object(settings), json!({"base_url": proxy.address}));
    assert!(!format!("{outcome:?}").contains("sk-issued-1"));
    // The page the person was sent to is the proxy's, asking for the code flow with PKCE and the loopback redirect.
    let page = pages.lock().unwrap_or_else(PoisonError::into_inner)[0].clone();
    assert!(
        page.starts_with(&format!("{}/authorize?response_type=code", proxy.address)),
        "{page}"
    );
    assert!(
        page.contains("code_challenge_method=S256")
            && page.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A"),
        "{page}"
    );
    assert!(!page.contains("sk-issued"), "no key in a page");
    let token = proxy.to("/token");
    assert_eq!(token.len(), 1);
}

#[test]
fn a_request_to_the_loopback_address_that_is_not_the_proxys_redirect_does_not_end_the_sign_in() {
    let proxy = Arc::new(Proxy::start());
    let mut who = person(&proxy);
    who.stray = Some("?code=stolen&state=not-this-one".to_owned());
    let outcome =
        sign_in(SIGN_IN, &proxy.address, who, 30_000).expect("the true redirect still signs in");
    assert_eq!(
        outcome.credentials.expose().get(API_KEY),
        Some("sk-issued-1")
    );
}

#[test]
fn a_proxy_with_no_sign_in_says_so_and_a_person_who_gives_up_ends_it() {
    let old = Arc::new(Proxy::start_with(false));
    let why = sign_in(SIGN_IN, &old.address, person(&old), 30_000).expect_err("no sign-in");
    assert!(
        why.message().contains("offers no sign-in") && why.message().contains("key"),
        "{}",
        why.message()
    );
    let proxy = Arc::new(Proxy::start());
    let who = person(&proxy);
    who.gives_up.store(true, Ordering::SeqCst);
    let ended = sign_in(SIGN_IN, &proxy.address, who, 30_000).expect_err("cancelled");
    assert_eq!(ended.status, Status::GUATIAO_ERR_GONE);
    assert_eq!(ended.message(), "login cancelled");
    // Nobody goes to the page: the time runs out.
    let proxy = Proxy::start();
    let mut request =
        LoginRequest::new(SIGN_IN, "unused").inputs(value(&json!({"base_url": proxy.address})));
    request.timeout_ms = 400;
    let late = LiteLlmAccounts
        .login(request, spawner(), Away.into_object())
        .expect_err("timed out");
    assert_eq!(late.status, Status::GUATIAO_ERR_GONE);
    assert_eq!(late.message(), "login timed out");
}

/// A person who opens the page and never signs in.
struct Away;

impl LoginPrompter for Away {
    fn say(&self, _text: &str) -> Result<(), ProviderError> {
        Ok(())
    }

    fn open_url(&self, _open: OpenUrl) -> Result<(), ProviderError> {
        Ok(())
    }

    fn ask(&self, _form: LoginForm) -> Result<LoginAnswer, ProviderError> {
        Err(ProviderError::new(
            Status::GUATIAO_ERR_UNSUPPORTED,
            "nothing to ask",
        ))
    }

    fn cancelled(&self) -> Result<bool, ProviderError> {
        Ok(false)
    }
}

#[test]
fn setting_a_key_keeps_it_as_the_one_credential_and_the_address_as_the_proxys_root() {
    let proxy = Arc::new(Proxy::start());
    let outcome = sign_in(
        SET_KEY,
        &format!("{}/v1/", proxy.address),
        person(&proxy),
        30_000,
    )
    .expect("set");
    assert_eq!(outcome.kind, KIND);
    assert_eq!(outcome.credentials.expose().get(API_KEY), Some(VIRTUAL_KEY));
    assert_eq!(
        outcome.credentials.expose().len(),
        1,
        "a key you were given has nothing to renew it with"
    );
    assert_eq!(outcome.expires_at, None);
    let settings =
        guatiao_serde::adapt::from_value::<serde_json::Map<String, Value>>(&outcome.settings)
            .expect("settings");
    assert_eq!(Value::Object(settings), json!({"base_url": proxy.address}));
    // No request was made to the proxy to set a key: it is read when it is used.
    assert!(proxy.seen().is_empty(), "{:?}", proxy.seen());
    let empty = Person {
        key: "  ".to_owned(),
        ..person(&proxy)
    };
    assert_eq!(
        sign_in(SET_KEY, &proxy.address, empty, 30_000)
            .expect_err("no key")
            .status,
        Status::GUATIAO_ERR_BAD_VALUE
    );
}

#[test]
fn the_models_are_the_proxys_list_for_the_key_and_a_refused_key_is_said() {
    let proxy = Proxy::start();
    let listing = LiteLlmAccounts
        .models(
            AccountModelsQuery::new(grant(&proxy.address, &[(API_KEY, VIRTUAL_KEY)])),
            spawner(),
            NoKeeper.into_object(),
        )
        .expect("an answer");
    assert!(listing.known, "{:?}", listing.unknown_because);
    let ids: Vec<&str> = listing
        .models
        .iter()
        .map(|model| model.id.as_str())
        .collect();
    assert_eq!(ids, ["claude-y", "gpt-x"]);
    let refused = LiteLlmAccounts
        .models(
            AccountModelsQuery::new(grant(&proxy.address, &[(API_KEY, "sk-not-a-key")])),
            spawner(),
            NoKeeper.into_object(),
        )
        .expect("an answer");
    assert_eq!(refused.unknown_kind, Some(UnknownKind::Failed));
    let said = refused.unknown_because.expect("why");
    assert!(
        said.contains("401") && !said.contains("sk-not-a-key"),
        "{said}"
    );
    // The key went to the proxy and to nobody else.
    assert!(
        proxy
            .seen()
            .iter()
            .all(|each| each.credential == VIRTUAL_KEY || each.credential == "sk-not-a-key")
    );
}

#[test]
fn usage_is_the_keys_budget_as_buckets_and_one_report_of_it_stops_the_account_when_spent() {
    let proxy = Proxy::start();
    let held = [(API_KEY, VIRTUAL_KEY)];
    let report = usage(grant(&proxy.address, &held)).expect("usage");
    assert!(report.supported);
    assert_eq!(report.account.as_deref(), Some("id-one"));
    let ids: Vec<&str> = report
        .buckets
        .iter()
        .map(|bucket| bucket.id.as_str())
        .collect();
    assert_eq!(ids, ["budget", "rpm"]);
    let budget = &report.buckets[0];
    assert_eq!(budget.unit, Unit::MicroUsd);
    assert_eq!((budget.used, budget.limit), (Some(0), Some(10_000_000)));
    assert_eq!(budget.status, BucketStatus::Ok);
    assert!(
        matches!(
            budget.refill,
            Refill::FixedWindow {
                resets_at: Some(4_070_908_800),
                ..
            }
        ),
        "{:?}",
        budget.refill
    );
    assert_eq!(report.buckets[1].limit, Some(100));
    // Two programs of two APIs spend on the one key; the one budget is what is read, and it is used up.
    assert_eq!(
        standin::post(
            &format!("{}/v1/messages", proxy.address),
            VIRTUAL_KEY,
            false
        ),
        200
    );
    assert_eq!(
        standin::post(
            &format!("{}/v1/responses", proxy.address),
            VIRTUAL_KEY,
            true
        ),
        200
    );
    let after = usage(grant(&proxy.address, &held)).expect("usage");
    assert_eq!(after.buckets[0].used, Some(20_000));
    proxy.spend(VIRTUAL_KEY, 10.0);
    let spent = usage(grant(&proxy.address, &held)).expect("usage");
    assert_eq!(spent.buckets[0].status, BucketStatus::Exhausted);
    // A key with no budget reports its spend and no limit, and cannot say what is left.
    proxy.budget(None);
    let free = usage(grant(&proxy.address, &held)).expect("usage");
    assert_eq!(
        (free.buckets[0].limit, free.buckets[0].status.clone()),
        (None, BucketStatus::Unknown)
    );
}

#[test]
fn a_key_the_proxy_no_longer_knows_is_gone_so_the_host_renews_it() {
    let proxy = Proxy::start();
    proxy.expire(VIRTUAL_KEY);
    let gone = usage(grant(&proxy.address, &[(API_KEY, VIRTUAL_KEY)])).expect_err("expired");
    assert_eq!(gone.status, Status::GUATIAO_ERR_GONE);
    assert!(!gone.message().contains(VIRTUAL_KEY));
    // A proxy with no such route says usage is not offered, which is an answer.
    let none = usage(grant("http://127.0.0.1:1", &[(API_KEY, VIRTUAL_KEY)]));
    assert!(none.is_err(), "nothing answers there");
}

#[test]
fn a_signed_in_key_is_renewed_by_its_refresh_token_which_the_proxy_rotates() {
    let proxy = Arc::new(Proxy::start());
    let outcome = sign_in(SIGN_IN, &proxy.address, person(&proxy), 30_000).expect("signed in");
    let first: Vec<(String, String)> = outcome
        .credentials
        .expose()
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let held: Vec<(&str, &str)> = first
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    proxy.expire("sk-issued-1");
    assert_eq!(
        usage(grant(&proxy.address, &held))
            .expect_err("expired")
            .status,
        Status::GUATIAO_ERR_GONE
    );
    let renewed = LiteLlmAccounts
        .renew(RenewRequest::new(grant(&proxy.address, &held)), spawner())
        .expect("renewed");
    let after = renewed.credentials.expect("new credentials");
    assert_eq!(after.expose().get(API_KEY), Some("sk-issued-2"));
    assert_eq!(after.expose().get(REFRESH_TOKEN), Some("refresh-2"));
    assert_eq!(
        after.expose().get(CLIENT_ID),
        outcome.credentials.expose().get(CLIENT_ID)
    );
    assert!(renewed.expires_at.is_some());
    // It reads with the new key, and the used refresh token is no good any more.
    let held_now = [
        (API_KEY, "sk-issued-2"),
        (REFRESH_TOKEN, "refresh-2"),
        (
            CLIENT_ID,
            held.iter()
                .find(|(key, _)| *key == CLIENT_ID)
                .expect("client")
                .1,
        ),
    ];
    assert!(
        usage(grant(&proxy.address, &held_now))
            .expect("usage")
            .supported
    );
    let again = LiteLlmAccounts
        .renew(RenewRequest::new(grant(&proxy.address, &held)), spawner())
        .expect_err("the first refresh token was used");
    assert_eq!(again.status, Status::GUATIAO_ERR_GONE);
    assert!(
        again.message().contains("sign in again") && !again.message().contains("refresh-1"),
        "{}",
        again.message()
    );
    // A key someone gave has nothing to renew it with, and the report says so.
    let given = LiteLlmAccounts
        .renew(
            RenewRequest::new(grant(&proxy.address, &[(API_KEY, VIRTUAL_KEY)])),
            spawner(),
        )
        .expect("an answer");
    assert!(given.credentials.is_none());
    assert!(
        given
            .not_renewed_because
            .is_some_and(|why| why.contains("sign in again"))
    );
}

/// The proxy serves both APIs on the one address and the one key, whichever way the account was made.
#[test]
fn an_account_provides_both_apis_and_another_kind_is_refused() {
    let provides =
        |kind: &str| LiteLlmAccounts.provides(coqui_acp_api::AccountRef::new("id-one", kind));
    assert_eq!(
        provides(KIND).expect("its own kind"),
        ["openai-api", "anthropic-api"]
    );
    assert_eq!(
        provides("another-kind")
            .expect_err("not a kind it keeps")
            .status,
        Status::GUATIAO_ERR_BAD_VALUE
    );
}
