//! What an entry offers before a session starts, asked as the account provider of an account lists it.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use canoa::gateway::{GatewayInfo, GatewayLease, GatewayRequest, ModelGateway};
use canoa_foundry::FoundryAccounts;
use canoa_foundry::account::KIND;
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::{Credentials, ModelInfo, ModelListing, ProcessSpawner};

use super::{Asking, entry_with};
use crate::accounts::Accounts;
use crate::accounts::store::FileStore;
use crate::config::Config;
use crate::gateways::Offer;

/// A directory of its own that goes with the test.
struct Dir(PathBuf);

impl Dir {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "canoa-options-{}-{}",
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

/// A gateway whose leases list two local models, the first the one the account serves.
struct Stand;

struct Listed;

impl GatewayLease for Listed {
    fn address(&self) -> Result<String, ProviderError> {
        Ok("http://127.0.0.1:1".to_owned())
    }

    fn bearer(&self) -> Result<String, ProviderError> {
        Ok("a-bearer".to_owned())
    }

    fn models(&self) -> Result<ModelListing, ProviderError> {
        let mut small = ModelInfo::new("local-small");
        small.is_default = true;
        Ok(ModelListing::of(vec![small, ModelInfo::new("local-large")]))
    }
}

impl ModelGateway for Stand {
    fn describe(&self) -> Result<GatewayInfo, ProviderError> {
        Ok(GatewayInfo::new(KIND, "stand-in"))
    }

    fn open(
        &self,
        _request: GatewayRequest,
        _spawner: Object<dyn ProcessSpawner>,
    ) -> Result<Object<dyn GatewayLease>, ProviderError> {
        Ok(Listed.into_object())
    }
}

#[test]
fn an_account_a_gateway_serves_lists_the_models_of_the_gateway_through_the_same_choices() {
    let dir = Dir::new();
    let root = dir.0.join("accounts");
    let store = Accounts::with_store(root.clone(), Box::new(FileStore::new(root))).expect("opens");
    store.gateways().serving(vec![
        Offer::new(
            Arc::new(Stand),
            "foundry_accounts",
            Arc::new(FoundryAccounts),
        )
        .expect("an offer"),
    ]);
    store
        .store_login("id-foundry", &Credentials::new(), None)
        .expect("the account is made, and holds no credential");
    let config = Config::from_str_in(
        "[host.agents.Claude]\nharness = \"claude\"\naccounts = [\"chat\"]\n\
         [host.accounts.chat]\nkind = \"foundry-local\"\nid = \"id-foundry\"\n\
         settings = { model = \"local-small\" }\n",
        std::path::Path::new(""),
    )
    .expect("parses");
    let asking = Asking {
        account: Some(("chat".to_owned(), store)),
        ..Asking::default()
    };
    let choices = entry_with(&config, "Claude", &asking).expect("an entry");
    assert!(choices.known, "{choices:?}");
    let ids: Vec<&str> = choices
        .models
        .iter()
        .map(|model| model.id.as_str())
        .collect();
    assert_eq!(ids, ["local-small", "local-large"]);
    assert!(choices.models[0].is_default && !choices.models[1].is_default);
}

/// A gateway whose lease lists the models its account's id names: `shared` and one of its own for each account.
struct PerAccount;

struct OfItsOwn(String);

impl GatewayLease for OfItsOwn {
    fn address(&self) -> Result<String, ProviderError> {
        Ok("http://127.0.0.1:1".to_owned())
    }

    fn bearer(&self) -> Result<String, ProviderError> {
        Ok("a-bearer".to_owned())
    }

    fn models(&self) -> Result<ModelListing, ProviderError> {
        let mut shared = ModelInfo::new("shared");
        shared.is_default = true;
        shared.efforts = vec!["low".to_owned()];
        Ok(ModelListing::of(vec![
            ModelInfo::new(format!("only-{}", self.0)),
            shared,
        ]))
    }
}

impl ModelGateway for PerAccount {
    fn describe(&self) -> Result<GatewayInfo, ProviderError> {
        Ok(GatewayInfo::new(KIND, "stand-in"))
    }

    fn open(
        &self,
        request: GatewayRequest,
        _spawner: Object<dyn ProcessSpawner>,
    ) -> Result<Object<dyn GatewayLease>, ProviderError> {
        Ok(OfItsOwn(request.account).into_object())
    }
}

#[test]
fn the_models_of_a_pool_show_each_model_once_with_every_account_that_lists_it() {
    let dir = Dir::new();
    let root = dir.0.join("accounts");
    let store = Accounts::with_store(root.clone(), Box::new(FileStore::new(root))).expect("opens");
    store.gateways().serving(vec![
        Offer::new(
            Arc::new(PerAccount),
            "foundry_accounts",
            Arc::new(FoundryAccounts),
        )
        .expect("an offer"),
    ]);
    for id in ["id-a", "id-b"] {
        store
            .store_login(id, &Credentials::new(), None)
            .expect("an account that holds no credential");
    }
    let config = Config::from_str_in(
        "[host.agents.Claude]\nharness = \"claude\"\naccounts = [\"both\"]\n\
         [host.accounts.a]\nkind = \"foundry-local\"\nid = \"id-a\"\n\
         settings = { model = \"shared\" }\n\
         [host.accounts.b]\nkind = \"foundry-local\"\nid = \"id-b\"\n\
         settings = { model = \"shared\" }\n\
         [host.pools.both]\nstandard = \"anthropic-api\"\naccounts = [\"a\", \"b\"]\n",
        std::path::Path::new(""),
    )
    .expect("parses");
    let asking = |account: &str| Asking {
        account: Some((account.to_owned(), Arc::clone(&store))),
        ..Asking::default()
    };
    let pool = entry_with(&config, "Claude", &asking("both")).expect("an entry");
    assert!(pool.known, "{pool:?}");
    let ids: Vec<&str> = pool.models.iter().map(|model| model.id.as_str()).collect();
    assert_eq!(
        ids,
        ["only-id-a", "shared", "only-id-b"],
        "each model once, in the order first met"
    );
    assert_eq!(
        pool.model_accounts,
        [vec!["a"], vec!["a", "b"], vec!["b"]],
        "{pool:?}"
    );
    let json = pool.to_json();
    assert_eq!(
        json["models"][1]["accounts"],
        serde_json::json!(["a", "b"]),
        "{json}"
    );
    assert_eq!(
        json["models"][1]["efforts"],
        serde_json::json!(["low"]),
        "{json}"
    );
    assert_eq!(
        json["accounts"],
        serde_json::json!([{"name": "a", "listed": true}, {"name": "b", "listed": true}])
    );
    // A model is refused for the pool as it is for an account, naming the pool and listing what its accounts list.
    let why = pool
        .refuses("the pool \"both\"", "nothing")
        .expect("not listed");
    assert!(
        why.contains("\"nothing\"") && why.contains("both") && why.contains("only-id-b"),
        "{why}"
    );
    // One account of it lists its own models only.
    let one = entry_with(&config, "Claude", &asking("a")).expect("an entry");
    assert!(
        one.model_accounts.is_empty() && one.members.is_empty(),
        "{one:?}"
    );
    assert_eq!(one.models.len(), 2);
}

/// An account another harness's provider made is listed by that provider, which knows what its kind is: Pi's own account
/// provider keeps no account of an Anthropic API key.
#[test]
fn an_account_another_harness_made_is_listed_by_the_account_provider_that_made_it() {
    let dir = Dir::new();
    let root = dir.0.join("accounts");
    let store = Accounts::with_store(root.clone(), Box::new(FileStore::new(root))).expect("opens");
    let mut key = Credentials::new();
    key.insert("api_key", "sk-not-a-real-key");
    store
        .store_login("id-shared", &key, None)
        .expect("the account is made");
    // Claude Code is a program that is not there: only an account provider that starts it can say so.
    let missing = dir
        .0
        .join(format!("no-such-claude{}", std::env::consts::EXE_SUFFIX));
    let config = Config::from_str_in(
        &format!(
            "[host.agents.Pi]\nharness = \"pi\"\naccounts = [\"shared\"]\n\
             [host.agents.Claude]\nharness = \"claude\"\nexecutable = {}\n\
             [host.accounts.shared]\nkind = \"anthropic-api-key\"\nid = \"id-shared\"\n",
            toml_string(&missing.to_string_lossy())
        ),
        std::path::Path::new(""),
    )
    .expect("parses");
    let asking = Asking {
        account: Some(("shared".to_owned(), store)),
        any_account: true,
        ..Asking::default()
    };
    let choices = entry_with(&config, "Pi", &asking).expect("an entry");
    assert!(!choices.known, "{choices:?}");
    assert_eq!(choices.unknown_kind, Some("not_installed"), "{choices:?}");
    assert!(
        choices
            .unknown_because
            .as_deref()
            .is_some_and(|why| why.contains("Claude Code")),
        "{choices:?}"
    );
}

/// A TOML basic string of `text`.
fn toml_string(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}
