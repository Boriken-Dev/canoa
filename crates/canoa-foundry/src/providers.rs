//! What this crate offers a Canoa host as a library: the account kind that names a Foundry Local model, the gateway that serves the
//! model to an agent for a session, the Store's program that installs Foundry Local, and the Store type that lists it. The host
//! holds no word of any of them: it loads this library as it loads any other.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "locks and files are std's"
)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};

use canoa::gateway::{GatewayInfo, GatewayLease, GatewayRequest, ModelGateway};
use canoa::store::extension::{ExtensionInfo, ExtensionType, StoreExtension};
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{Provider, Status};
use coqui_acp_api::{
    AccountKeeper, AccountOffer, AccountProvider, AccountProviderInfo, AccountUsage,
    AccountUsageQuery, AccountWatch, Credentials, LoginOutcome, LoginPrompter, LoginRequest,
    ProcessSpawner, RenewReport, RenewRequest, WatchRequest,
};
use serde_json::{Map, Value};

use crate::account::{self, KIND, Settings};
use crate::child::Launch;
use crate::engines::{Engines, default_engine};
use crate::gateway::{Running, Ticket};

/// The vendor of what runs the model, as a session's identity variables name an account's provider.
const VENDOR: &str = "microsoft-foundry";
/// The table of canoa.cfg that holds this library's own settings: `windows` maps a model variant's id to the window it is run with.
pub const TABLE: &str = "foundry";
/// The file in the host's home that the gateways record their requests in.
pub const GATEWAY_LOG: &str = "foundry-gateway.log";
/// The folder in the host's home that holds the adjusted copies of models.
pub const WINDOW_COPIES: &str = "foundry-windows";
/// The folder in the host's home that is the SDK's models folder: the model it runs is linked into it from Foundry Local's.
pub const OWN_MODELS: &str = "foundry-models";
/// The id of the Store type that lists Foundry Local.
pub const STORE_TYPE: &str = "foundry";

fn refused(why: impl AsRef<str>) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, why.as_ref())
}

fn unsupported(why: &str) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_UNSUPPORTED, why)
}

/// The accounts that name a Foundry Local model: no credential is kept, the model's name and where the service is are the account.
#[derive(Default, Debug, Clone, Copy, Provider)]
#[provider(AccountProvider, id = "foundry_accounts", crate = ::coqui_acp_api::guatiao)]
pub struct FoundryAccounts;

impl AccountProvider for FoundryAccounts {
    fn describe(&self) -> Result<AccountProviderInfo, ProviderError> {
        let mut info = AccountProviderInfo::new("Foundry Local");
        info.description = "Names a model that Foundry Local runs on this computer. No credential is kept; the host serves the model to the agent through a gateway for each session.".to_owned();
        Ok(info)
    }

    fn offers(&self) -> Result<AccountOffer, ProviderError> {
        Ok(account::offer())
    }

    fn login(
        &self,
        request: LoginRequest,
        spawner: Object<dyn ProcessSpawner>,
        prompter: Object<dyn LoginPrompter>,
    ) -> Result<LoginOutcome, ProviderError> {
        drop((spawner, prompter));
        if request.method != KIND {
            return Err(refused("Foundry Local has no such way to sign in"));
        }
        let inputs = guatiao_serde::adapt::from_value::<Map<String, Value>>(&request.inputs)
            .map_err(|_| refused("the inputs are not a table"))?;
        let settings = Settings::from_map(&inputs).map_err(refused)?;
        let mut outcome = LoginOutcome::new(KIND, Credentials::new());
        outcome.settings = request.inputs;
        outcome.label = Some(settings.model);
        Ok(outcome)
    }

    fn renew(
        &self,
        request: RenewRequest,
        spawner: Object<dyn ProcessSpawner>,
    ) -> Result<RenewReport, ProviderError> {
        drop((request, spawner));
        Err(unsupported(
            "a Foundry Local account holds no credential to renew",
        ))
    }

    fn watch(
        &self,
        request: WatchRequest,
        spawner: Object<dyn ProcessSpawner>,
        keeper: Object<dyn AccountKeeper>,
    ) -> Result<Object<dyn AccountWatch>, ProviderError> {
        drop((request, spawner, keeper));
        Err(unsupported(
            "a Foundry Local account holds no login to watch",
        ))
    }

    fn usage(
        &self,
        query: AccountUsageQuery,
        spawner: Object<dyn ProcessSpawner>,
        keeper: Object<dyn AccountKeeper>,
    ) -> Result<AccountUsage, ProviderError> {
        drop((spawner, keeper));
        let mut usage =
            AccountUsage::unsupported("a model that runs on this computer has no plan limits");
        usage.account = Some(query.account.id);
        Ok(usage)
    }
}

/// The gateways that are up, one per account while a session holds it, and how to make an engine for one.
#[derive(Default)]
struct Up {
    engines: OnceLock<Engines>,
    running: Mutex<BTreeMap<String, Weak<Running>>>,
}

/// Serves the model of a Foundry Local account to the sessions of an agent: the OpenAI and Anthropic APIs on loopback, translated to
/// chat completions that the engine the account asks for answers.
#[derive(Default, Provider)]
#[provider(ModelGateway, id = "foundry_gateway", crate = ::coqui_acp_api::guatiao)]
pub struct FoundryGateway {
    up: Up,
}

impl core::fmt::Debug for FoundryGateway {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FoundryGateway").finish_non_exhaustive()
    }
}

impl FoundryGateway {
    /// A gateway whose engines are opened by `engines` rather than from an account's settings, for a test with a model of its own.
    #[cfg(test)]
    pub(crate) fn with_engines(engines: Engines) -> Self {
        let gateway = Self::default();
        let _ = gateway.up.engines.set(engines);
        gateway
    }

    /// A bearer on the gateway of `account`, started when no session holds one.
    fn ticket(
        &self,
        account: &str,
        settings: &Settings,
        home: Option<&str>,
        launch: Option<&Launch>,
    ) -> Result<Ticket, String> {
        let home = home.map(PathBuf::from);
        let mut running = self
            .up
            .running
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let up = match running.get(account).and_then(Weak::upgrade) {
            Some(up) => up,
            None => {
                let copies = home.as_ref().map(|home| home.join(WINDOW_COPIES));
                let models = home.as_ref().map(|home| home.join(OWN_MODELS));
                let engines = self.up.engines.get().cloned().unwrap_or_else(|| {
                    let launch = launch.cloned();
                    Arc::new(move |settings: &Settings| {
                        default_engine(
                            settings,
                            (copies.as_deref(), models.as_deref()),
                            launch.as_ref(),
                        )
                    })
                });
                let engine = engines(settings)?;
                let log = home.as_ref().map(|home| home.join(GATEWAY_LOG));
                let up = Running::start(engine, log)
                    .map_err(|error| format!("the gateway could not listen: {error}"))?;
                running.insert(account.to_owned(), Arc::downgrade(&up));
                up
            }
        };
        drop(running);
        up.mint()
            .map_err(|_| "the system has no randomness for a bearer".to_owned())
    }
}

/// The window canoa.cfg's table sets for `model`: a whole number of tokens of at least 1; anything else is left out.
fn window_of(table: &Value, model: &str) -> Option<u64> {
    table
        .get("windows")?
        .get(model)?
        .as_u64()
        .filter(|tokens| *tokens >= 1)
}

impl ModelGateway for FoundryGateway {
    fn describe(&self) -> Result<GatewayInfo, ProviderError> {
        let mut info = GatewayInfo::new(KIND, VENDOR);
        info.table = Some(TABLE.to_owned());
        Ok(info)
    }

    fn open(
        &self,
        request: GatewayRequest,
        spawner: Object<dyn ProcessSpawner>,
    ) -> Result<Object<dyn GatewayLease>, ProviderError> {
        let map = serde_json::from_str::<Map<String, Value>>(&request.settings)
            .map_err(|_| refused("the account's settings are not a table"))?;
        let mut settings = Settings::from_map(&map).map_err(refused)?;
        let table = serde_json::from_str::<Value>(&request.table).unwrap_or(Value::Null);
        settings.window = window_of(&table, &settings.model);
        let launch = Launch {
            spawner: Arc::new(spawner),
            helper: request.helper.clone(),
        };
        let ticket = self
            .ticket(
                &request.account,
                &settings,
                request.home.as_deref(),
                Some(&launch),
            )
            .map_err(|why| ProviderError::new(Status::GUATIAO_ERR_INTERNAL, &why))?;
        Ok(Lease { ticket }.into_object())
    }
}

/// One session's bearer on a gateway; dropping it ends the session's access.
struct Lease {
    ticket: Ticket,
}

impl GatewayLease for Lease {
    fn address(&self) -> Result<String, ProviderError> {
        Ok(self.ticket.address())
    }

    fn bearer(&self) -> Result<String, ProviderError> {
        Ok(self.ticket.bearer().to_owned())
    }
}

canoa::store_provider!(
    /// The Foundry Local store: its command line, service and libraries, installed from the publisher's release.
    FoundryStore, crate::store::FoundryRecipe, "foundry_store"
);

/// Adds the Foundry Local type to the Store: the program that runs its models, installed like any other.
#[derive(Default, Debug, Clone, Copy, Provider)]
#[provider(StoreExtension, id = "foundry_extension", crate = ::coqui_acp_api::guatiao)]
pub struct FoundryExtension;

impl StoreExtension for FoundryExtension {
    fn describe(&self) -> Result<ExtensionInfo, ProviderError> {
        let mut local = ExtensionType::new(STORE_TYPE, "Foundry Local", ["foundry_store"]);
        local.icon = Some("memory".to_owned());
        Ok(ExtensionInfo::new(
            "Foundry Local",
            "Boriken Dev",
            vec![local],
        ))
    }
}

// What this crate offers as a library the host links: handed to `Registry::register_local`, the same way a loaded library is read,
// so it is offered and chosen like any other.
coqui_acp_api::guatiao::local_providers!(
    id = "canoa-foundry",
    version = env!("CARGO_PKG_VERSION"),
    providers = [
        FoundryAccounts,
        FoundryGateway,
        FoundryStore,
        FoundryExtension
    ]
);

#[cfg(test)]
mod tests {
    use std::io::{Read as _, Write as _};
    use std::net::TcpStream;
    use std::time::Duration;

    use canoa::store::ProgramStore;
    use coqui_acp_api::guatiao::library::Registry;
    use coqui_acp_api::{AccountLoss, AccountRenewal, LoginAnswer, LoginForm, OpenUrl};
    use coqui_acp_harness::LocalSpawner;
    use serde_json::json;

    use super::*;
    use crate::{Cancel, Engine, EngineError, ModelInfo};

    struct Local;

    impl Engine for Local {
        fn model(&self) -> Result<ModelInfo, EngineError> {
            Ok(ModelInfo::new("local-model").tools(true))
        }

        fn models(&self) -> Vec<ModelInfo> {
            vec![ModelInfo::new("local-model").tools(true)]
        }

        fn chat(&self, _request: &Value, _cancel: &Cancel) -> Result<Value, EngineError> {
            Err(EngineError::Failed("not asked here".to_owned()))
        }
    }

    /// A prompter that is never asked anything.
    struct Silent;

    impl LoginPrompter for Silent {
        fn say(&self, _text: &str) -> Result<(), ProviderError> {
            Ok(())
        }

        fn open_url(&self, _open: OpenUrl) -> Result<(), ProviderError> {
            Ok(())
        }

        fn ask(&self, _form: LoginForm) -> Result<LoginAnswer, ProviderError> {
            Err(unsupported("nothing to ask"))
        }

        fn cancelled(&self) -> Result<bool, ProviderError> {
            Ok(false)
        }
    }

    /// A keeper nobody tells anything.
    struct NoKeeper;

    impl AccountKeeper for NoKeeper {
        fn renewed(&self, _renewal: AccountRenewal) -> Result<(), ProviderError> {
            Ok(())
        }

        fn lost(&self, _loss: AccountLoss) -> Result<(), ProviderError> {
            Ok(())
        }
    }

    fn spawner() -> Object<dyn ProcessSpawner> {
        LocalSpawner::new().into_object()
    }

    fn request(account: &str, model: &str) -> GatewayRequest {
        let mut request = GatewayRequest::new(account);
        request.settings = json!({"model": model}).to_string();
        request
    }

    fn status(address: &str, bearer: &str) -> Option<u16> {
        let host = address.strip_prefix("http://")?;
        let mut stream = TcpStream::connect(host).ok()?;
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .ok()?;
        let asked = format!(
            "GET /v1/models HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {bearer}\r\n\r\n"
        );
        stream.write_all(asked.as_bytes()).ok()?;
        let mut said = String::new();
        let _ = stream.read_to_string(&mut said);
        said.split_whitespace().nth(1)?.parse().ok()
    }

    fn stand_in() -> FoundryGateway {
        FoundryGateway::with_engines(Arc::new(|_| Ok(Arc::new(Local) as Arc<dyn Engine>)))
    }

    fn offered_by(registry: &Registry) -> [Vec<String>; 4] {
        let sorted = |mut ids: Vec<String>| {
            ids.sort();
            ids
        };
        [
            sorted(
                registry
                    .offers::<dyn AccountProvider>()
                    .map(|offer| offer.id().to_owned())
                    .collect(),
            ),
            sorted(
                registry
                    .offers::<dyn ModelGateway>()
                    .map(|offer| offer.id().to_owned())
                    .collect(),
            ),
            sorted(
                registry
                    .offers::<dyn ProgramStore>()
                    .map(|offer| offer.id().to_owned())
                    .collect(),
            ),
            sorted(
                registry
                    .offers::<dyn StoreExtension>()
                    .map(|offer| offer.id().to_owned())
                    .collect(),
            ),
        ]
    }

    #[test]
    fn the_library_offers_the_account_the_gateway_the_store_and_the_type() {
        let mut registry = Registry::new("canoa", "0");
        let loading = registry
            .register_local(crate::LIBRARY, library)
            .expect("registers");
        assert!(loading.loaded().is_some());
        assert_eq!(
            offered_by(&registry),
            [
                vec!["foundry_accounts".to_owned()],
                vec!["foundry_gateway".to_owned()],
                vec!["foundry_store".to_owned()],
                vec!["foundry_extension".to_owned()],
            ]
        );
    }

    #[test]
    fn the_gateway_says_which_kind_it_serves_and_where_its_own_settings_are() {
        let info = FoundryGateway::default().describe().expect("described");
        assert_eq!((info.kind.as_str(), info.vendor.as_str()), (KIND, VENDOR));
        assert_eq!(info.table.as_deref(), Some(TABLE));
        let offered = FoundryAccounts.offers().expect("offers");
        assert!(
            offered.kind(&info.kind).is_some(),
            "the same library makes the kind"
        );
    }

    #[test]
    fn the_extension_adds_one_type_made_of_the_stores_this_library_offers() {
        let info = FoundryExtension.describe().expect("described");
        assert_eq!(info.types.len(), 1);
        let kind = &info.types[0];
        assert_eq!(kind.problem(), None);
        assert_eq!(kind.id, STORE_TYPE);
        assert_eq!(kind.stores, ["foundry_store"]);
    }

    #[test]
    fn signing_in_names_a_model_and_keeps_no_credential() {
        let named = |inputs: Value| {
            let inputs = guatiao_serde::from_serde(inputs).expect("a value");
            FoundryAccounts.login(
                LoginRequest::new(KIND, "unused").inputs(inputs),
                spawner(),
                Silent.into_object(),
            )
        };
        let outcome = named(json!({"model": "qwen3-14b-generic-gpu:2"})).expect("signs in");
        assert_eq!(outcome.kind, KIND);
        assert_eq!(outcome.label.as_deref(), Some("qwen3-14b-generic-gpu:2"));
        assert!(outcome.credentials.expose().is_empty());
        let settings = guatiao_serde::adapt::from_value::<Map<String, Value>>(&outcome.settings)
            .expect("a table");
        assert_eq!(settings["model"], "qwen3-14b-generic-gpu:2");
        let refused = named(json!({})).expect_err("a model is named");
        assert!(refused.message().contains("model"), "{}", refused.message());
        let other = FoundryAccounts
            .login(
                LoginRequest::new("other", "unused"),
                spawner(),
                Silent.into_object(),
            )
            .expect_err("no such method");
        assert_eq!(other.status, Status::GUATIAO_ERR_BAD_VALUE);
    }

    #[test]
    fn a_model_that_runs_here_has_no_plan_limits_and_nothing_to_renew_or_watch() {
        let grant = coqui_acp_api::AccountGrant::new("id-1", KIND);
        let usage = FoundryAccounts
            .usage(
                AccountUsageQuery::new(grant.clone()),
                spawner(),
                NoKeeper.into_object(),
            )
            .expect("answers");
        assert!(!usage.supported);
        assert_eq!(usage.account.as_deref(), Some("id-1"));
        let renewed = FoundryAccounts
            .renew(RenewRequest::new(grant.clone()), spawner())
            .expect_err("nothing to renew");
        assert_eq!(renewed.status, Status::GUATIAO_ERR_UNSUPPORTED);
        let watched =
            FoundryAccounts.watch(WatchRequest::new(grant), spawner(), NoKeeper.into_object());
        assert!(watched.is_err());
    }

    #[test]
    fn sessions_on_one_account_share_a_gateway_and_each_has_its_own_bearer() {
        let gateway = stand_in();
        let one = gateway
            .open(request("a", "local-model"), spawner())
            .expect("opens");
        let two = gateway
            .open(request("a", "local-model"), spawner())
            .expect("opens");
        let (address, first, second) = (
            one.address().expect("an address"),
            one.bearer().expect("a bearer"),
            two.bearer().expect("a bearer"),
        );
        assert_eq!(address, two.address().expect("an address"));
        assert_ne!(first, second);
        assert!(address.starts_with("http://127.0.0.1:"), "{address}");
        assert_eq!(status(&address, &first), Some(200));
        drop(one);
        assert_ne!(
            status(&address, &first),
            Some(200),
            "the first session's bearer ended"
        );
        assert_eq!(status(&address, &second), Some(200), "the second's did not");
        let other = gateway
            .open(request("b", "local-model"), spawner())
            .expect("opens");
        assert_ne!(
            other.address().expect("an address"),
            address,
            "another account, another gateway"
        );
    }

    #[test]
    fn a_gateway_that_cannot_start_is_a_refusal_the_person_can_read() {
        let gateway = FoundryGateway::with_engines(Arc::new(|_| {
            Err("no Foundry Local service is running".to_owned())
        }));
        let refused = gateway
            .open(request("a", "local-model"), spawner())
            .err()
            .expect("no service");
        assert_eq!(refused.message(), "no Foundry Local service is running");
        let mut stray = GatewayRequest::new("a");
        stray.settings = json!({"model": "m", "password": "x"}).to_string();
        let stray = gateway
            .open(stray, spawner())
            .err()
            .expect("a setting the kind does not have");
        assert!(stray.message().contains("password"), "{}", stray.message());
        let none = gateway
            .open(GatewayRequest::new("a"), spawner())
            .err()
            .expect("no model");
        assert!(none.message().contains("model"), "{}", none.message());
    }

    #[test]
    fn a_models_window_in_the_table_reaches_the_engine_opened_for_it_and_a_model_without_one_gets_none()
     {
        type Seen = Arc<Mutex<Vec<(String, Option<u64>)>>>;
        let seen: Seen = Arc::default();
        let record = Arc::clone(&seen);
        let gateway = FoundryGateway::with_engines(Arc::new(move |settings| {
            record
                .lock()
                .expect("lock")
                .push((settings.model.clone(), settings.window));
            Ok(Arc::new(Local) as Arc<dyn Engine>)
        }));
        let with = |account: &str, model: &str, table: &Value| {
            let mut asked = request(account, model);
            asked.table = table.to_string();
            asked
        };
        let table =
            json!({"windows": {"local-model": 16384, "other": 999, "zero": 0, "text": "big"}});
        let _held = [
            gateway.open(with("a", "local-model", &table), spawner()),
            gateway.open(with("b", "plain", &table), spawner()),
            gateway.open(with("c", "zero", &table), spawner()),
            gateway.open(with("d", "text", &table), spawner()),
            gateway.open(with("e", "local-model", &Value::Null), spawner()),
        ]
        .map(|lease| lease.expect("opens"));
        assert_eq!(
            *seen.lock().expect("lock"),
            [
                ("local-model".to_owned(), Some(16384)),
                ("plain".to_owned(), None),
                ("zero".to_owned(), None),
                ("text".to_owned(), None),
                ("local-model".to_owned(), None),
            ]
        );
    }

    #[test]
    fn the_gateways_files_are_kept_in_the_hosts_home() {
        let home = std::env::temp_dir().join(format!("canoa-foundry-home-{}", std::process::id()));
        std::fs::create_dir_all(&home).expect("a folder");
        let gateway = stand_in();
        let mut asked = request("a", "local-model");
        asked.home = Some(home.to_string_lossy().into_owned());
        let lease = gateway.open(asked, spawner()).expect("opens");
        let address = lease.address().expect("an address");
        let bearer = lease.bearer().expect("a bearer");
        assert_eq!(status(&address, &bearer), Some(200));
        // The request is on the record, in the file the library names.
        let log = std::fs::read_to_string(home.join(GATEWAY_LOG)).expect("the log");
        assert!(log.contains("/v1/models"), "{log}");
        drop(lease);
        let _ = std::fs::remove_dir_all(&home);
    }
}
