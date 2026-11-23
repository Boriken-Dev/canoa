//! The LiteLLM library the host carries, against a loopback stand-in for the proxy: one account of the kind runs a Claude Code entry and
//! a Codex entry, sits in the pools of both of its standards, and one read of the key's budget serves them; a used-up budget stops it
//! in both.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

#[path = "../../../../extensions/canoa-litellm/tests/standin/mod.rs"]
mod standin;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use coqui_acp_api::{CacheLifetime, Credentials, Status};
use coqui_acp_api::{LoginPrompter as _, ProcessSpawner as _};
use standin::{Proxy, VIRTUAL_KEY};

use crate::accounts::Accounts;
use crate::accounts::store::FileStore;
use crate::config::Config;
use crate::kind;
use crate::options::limits::Endpoints;
use crate::options::{harness_of, offering};
use crate::pools::gather::{candidates, forget_reports};
use crate::pools::turn::{Candidate, Cursor, Turn, decide};

struct Dir(PathBuf);

impl Dir {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "canoa-litellm-{}-{}",
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

fn config(address: &str) -> Config {
    let text = format!(
        "[host.agents.Claude]\nharness = \"claude\"\naccounts = [\"lite\"]\n\
         [host.agents.Codex]\nharness = \"codex\"\naccounts = [\"lite\"]\n\
         [host.accounts.lite]\nkind = \"litellm\"\nid = \"id-lite\"\nsettings = {{ base_url = \"{address}\" }}\n"
    );
    Config::from_str_in(&text, std::path::Path::new("")).expect("parses")
}

fn turn() -> Turn<'static> {
    Turn {
        current: None,
        model: None,
        idle: None,
        lifetime: CacheLifetime::Default,
        now: 1_800_000_000,
    }
}

#[test]
fn one_litellm_account_runs_both_harnesses_on_one_budget_and_a_spent_budget_stops_it_in_both() {
    let proxy = Proxy::start();
    let dir = Dir::new();
    let root = dir.0.join("accounts");
    let store = Accounts::with_store(root.clone(), Box::new(FileStore::new(root))).expect("opens");
    // What a sign-in made: the key, and what renews it.
    store
        .store_login(
            "id-lite",
            &Credentials::from_iter([
                ("api_key", VIRTUAL_KEY),
                ("refresh_token", "refresh-1"),
                ("client_id", "client-1"),
            ]),
            None,
        )
        .expect("stored");
    let config = config(&proxy.address);
    let mut built = Vec::new();
    for (entry, pool) in [("Claude", "any-anthropic-api"), ("Codex", "any-openai-api")] {
        let entry = config.entry(entry).expect("an entry");
        let harness = harness_of(entry, Arc::default()).expect("a provider");
        let harness = offering(&config, entry, harness, &store, true).expect("offered");
        let (_, _, info) = harness.account_named("lite", false).expect("offered here");
        assert_eq!(info.provides, ["openai-api", "anthropic-api"]);
        let pool = harness
            .pool_named(pool, true)
            .expect("the host's pool of its standard");
        assert!(
            pool.members.iter().any(|member| member.name == "lite"),
            "{pool:?}"
        );
        built.push(harness);
    }

    // Each harness is given the key and no more than its standard names; the account's own provider is given what renews it.
    for harness in &built {
        let held = harness.hold_account("lite", "test", true).expect("held");
        let given: Vec<String> = held.grant().credentials.expose().keys().cloned().collect();
        assert_eq!(given, ["api_key"], "{}", harness.id());
        assert_eq!(held.provider_grant().credentials.expose().len(), 3);
    }

    // The models are the proxy's, for the one account, whichever harness asks.
    for harness in &built {
        let listed = crate::options::as_account(&config, harness, "lite", true);
        assert!(listed.known, "{:?}", listed.unknown_because);
        let ids: Vec<&str> = listed
            .models
            .iter()
            .map(|model| model.id.as_str())
            .collect();
        assert_eq!(ids, ["claude-y", "gpt-x"], "{}", harness.id());
    }

    // One read of the key's budget serves both pools.
    let names = ["lite".to_owned()];
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
    assert_eq!(proxy.to("/key/info").len(), 1, "{:?}", proxy.seen());
    for pool in &room {
        let report = pool[0].usage.as_ref().expect("the budget");
        assert_eq!(report.buckets[0].id, "budget");
        assert_eq!(report.buckets[0].status, Status::Ok);
        let with_other = [
            pool[0].clone(),
            Candidate {
                name: "other".to_owned(),
                ..Candidate::default()
            },
        ];
        assert_eq!(
            decide("p", &Cursor::default(), &with_other, &turn())
                .expect("room")
                .account,
            "lite"
        );
    }

    // The two APIs spend on the one key, and when the budget is gone the account is left in both pools.
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
    proxy.spend(VIRTUAL_KEY, 10.0);
    forget_reports();
    for harness in &built {
        let pool = read(harness);
        assert_eq!(
            pool[0].usage.as_ref().expect("report").buckets[0].status,
            Status::Exhausted
        );
        let with_other = [
            pool[0].clone(),
            Candidate {
                name: "other".to_owned(),
                ..Candidate::default()
            },
        ];
        let chosen =
            decide("p", &Cursor::default(), &with_other, &turn()).expect("another has room");
        assert_eq!(chosen.account, "other", "{}", harness.id());
        let alone = decide("p", &Cursor::default(), &pool, &turn()).expect_err("nothing has room");
        assert!(alone.contains("lite"), "{alone}");
    }
    assert_eq!(proxy.to("/key/info").len(), 2, "one more read, for both");
    // The key went only to the proxy, as the credential of each request.
    assert!(
        proxy
            .seen()
            .iter()
            .all(|each| each.credential == VIRTUAL_KEY)
    );
}

/// A person who opens the page the sign-in shows, in a browser that is the stand-in's.
struct Browsing {
    proxy: Arc<Proxy>,
}

impl coqui_acp_api::LoginPrompter for Browsing {
    fn say(&self, _text: &str) -> Result<(), coqui_acp_api::guatiao::library::ProviderError> {
        Ok(())
    }

    fn open_url(
        &self,
        open: coqui_acp_api::OpenUrl,
    ) -> Result<(), coqui_acp_api::guatiao::library::ProviderError> {
        let proxy = Arc::clone(&self.proxy);
        std::thread::spawn(move || {
            proxy.browse(&open.url);
        });
        Ok(())
    }

    fn ask(
        &self,
        _form: coqui_acp_api::LoginForm,
    ) -> Result<coqui_acp_api::LoginAnswer, coqui_acp_api::guatiao::library::ProviderError> {
        Err(coqui_acp_api::guatiao::library::ProviderError::new(
            coqui_acp_api::guatiao::Status::GUATIAO_ERR_UNSUPPORTED,
            "nothing to ask",
        ))
    }

    fn cancelled(&self) -> Result<bool, coqui_acp_api::guatiao::library::ProviderError> {
        Ok(false)
    }
}

#[test]
fn a_key_the_proxy_stopped_accepting_is_renewed_by_the_host_through_the_refresh_token_and_read_again()
 {
    use coqui_acp_api::AccountProvider as _;
    let proxy = Arc::new(Proxy::start());
    let dir = Dir::new();
    let root = dir.0.join("accounts");
    let store = Accounts::with_store(root.clone(), Box::new(FileStore::new(root))).expect("opens");
    let request = coqui_acp_api::LoginRequest::new("litellm-sso", "unused").inputs(
        canoa::api_gateway::value(&serde_json::json!({"base_url": proxy.address})),
    );
    let made = canoa_litellm::LiteLlmAccounts
        .login(
            request,
            coqui_acp_harness::LocalSpawner::new().into_object(),
            Browsing {
                proxy: Arc::clone(&proxy),
            }
            .into_object(),
        )
        .expect("signed in");
    store
        .store_login(
            "id-lite",
            made.credentials.expose(),
            made.expires_at.clone(),
        )
        .expect("stored");
    let config = config(&proxy.address);
    let account = config.accounts.get("lite").expect("the account");
    // The key ends at the proxy, as a day later it would.
    proxy.expire("sk-issued-1");
    let row = crate::options::usage_for(&config, ("lite", account), &store, &Endpoints::default())
        .expect("read after a renewal");
    let report: coqui_acp_api::UsageReport = serde_json::from_value(row).expect("a report");
    assert!(report.supported, "{report:?}");
    assert_eq!(report.buckets[0].id, "budget");
    // What the host holds now is the renewed key and the next refresh token.
    let stored = store.load("id-lite").expect("reads").expect("a record");
    assert_eq!(
        stored.credentials.get("api_key").map(String::as_str),
        Some("sk-issued-2")
    );
    assert_eq!(
        stored.credentials.get("refresh_token").map(String::as_str),
        Some("refresh-2")
    );
    assert_eq!(stored.revision, 2, "the sign-in, then the renewal");
    let lines: Vec<String> = proxy.seen().iter().map(|each| each.line.clone()).collect();
    assert_eq!(
        lines.iter().filter(|line| *line == "POST /token").count(),
        2,
        "{lines:?}"
    );
}
