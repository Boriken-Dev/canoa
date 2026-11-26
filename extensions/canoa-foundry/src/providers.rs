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
    AccountKeeper, AccountModelsQuery, AccountOffer, AccountProvider, AccountProviderInfo,
    AccountRef, AccountUsage, AccountUsageQuery, AccountWatch, Credentials, LoginOutcome,
    LoginPrompter, LoginRequest, ModelListing, ProcessSpawner, RenewReport, RenewRequest,
    WatchRequest,
};
use serde_json::Value;

use crate::account::{self, KIND, Settings};
use crate::child::Launch;
use crate::engines::{Engines, default_engine};
use crate::gateway::{Running, Ticket};
use crate::model_store::FoundryModels;

/// The vendor of what runs the model, as a session's identity variables name an account's provider.
const VENDOR: &str = "microsoft-foundry";
/// The table of canoa.cfg that holds this library's own settings: `models` maps a model variant's id to what is kept for it, and
/// `window` there is the window it is run with.
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

    fn provides(&self, account: AccountRef) -> Result<Vec<String>, ProviderError> {
        account.check_id()?;
        account::offer()
            .kind(&account.kind)
            .map(|kind| kind.provides.clone())
            .ok_or_else(|| refused("this provider keeps no account of that kind"))
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
        let settings =
            Settings::from_value(&request.inputs).map_err(|why| refused(why.to_string()))?;
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

    fn signed_in(
        &self,
        _: coqui_acp_api::SignedInQuery,
    ) -> Result<coqui_acp_api::SignedInCheck, ProviderError> {
        Err(unsupported("Foundry Local keeps no login of its own"))
    }

    fn models(
        &self,
        query: AccountModelsQuery,
        spawner: Object<dyn ProcessSpawner>,
        keeper: Object<dyn AccountKeeper>,
    ) -> Result<ModelListing, ProviderError> {
        drop((spawner, keeper));
        query.account.check_id()?;
        Ok(ModelListing::unknown(
            "a model that runs on this computer is listed by the gateway that serves it, once a session holds the account",
        ))
    }
}

/// The models a gateway's engine could serve, as a listing: the one the account serves is marked as the one in use when none is
/// named.
fn listing(models: Vec<crate::ModelInfo>, serving: &str) -> ModelListing {
    ModelListing::of(
        models
            .into_iter()
            .map(|model| {
                let mut info = coqui_acp_api::ModelInfo::new(model.id.clone());
                info.is_default = model.id == serving;
                // What the catalog says the model is for: a chat picker leaves an embedding model out. A task nobody here knows
                // leaves the model reporting nothing.
                if let Some(task) = model.task.as_deref() {
                    let mut capabilities: Vec<&str> = crate::models::capabilities_of(task);
                    if capabilities.contains(&"chat") {
                        if model.tools {
                            capabilities.push("tool_use");
                        }
                        if model.images {
                            capabilities.push("image_input");
                        }
                    }
                    if !capabilities.is_empty() {
                        info = info.with_capabilities(
                            capabilities,
                            coqui_acp_api::CapabilitiesSource::Listed,
                        );
                    }
                }
                if let Some(window) = model.context {
                    info = info.with_limits(coqui_acp_api::ModelLimits::new().context(window));
                }
                let mut can = Vec::new();
                if model.tools {
                    can.push("tools".to_owned());
                }
                if model.images {
                    can.push("images".to_owned());
                }
                if let Some(window) = model.context {
                    can.push(format!("{window} tokens of context"));
                }
                if !can.is_empty() {
                    info.description = Some(can.join(", "));
                }
                info
            })
            .collect(),
    )
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
        .get("models")?
        .get(model)?
        .get("window")?
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
        let mut settings =
            Settings::from_value(&request.settings).map_err(|why| refused(why.to_string()))?;
        let table =
            guatiao_serde::adapt::from_value::<Value>(&request.table).unwrap_or(Value::Null);
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
        Ok(Lease {
            ticket,
            serving: settings.model,
        }
        .into_object())
    }
}

/// One session's bearer on a gateway; dropping it ends the session's access.
struct Lease {
    ticket: Ticket,
    /// The model the account serves.
    serving: String,
}

impl GatewayLease for Lease {
    fn address(&self) -> Result<String, ProviderError> {
        Ok(self.ticket.address())
    }

    fn bearer(&self) -> Result<String, ProviderError> {
        Ok(self.ticket.bearer().to_owned())
    }

    fn models(&self) -> Result<ModelListing, ProviderError> {
        Ok(listing(self.ticket.models(), &self.serving))
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
        local.models = Some("foundry_models".to_owned());
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
        FoundryExtension,
        FoundryModels
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
    use serde_json::{Map, json};

    use super::*;
    use crate::{Cancel, Engine, EngineError, ModelInfo};

    #[test]
    fn an_account_provides_what_its_kind_lists_and_another_kind_is_refused() {
        let kind = account::offer()
            .kind(KIND)
            .expect("the kind")
            .provides
            .clone();
        assert!(!kind.is_empty());
        assert_eq!(
            FoundryAccounts
                .provides(AccountRef::new("id-one", KIND))
                .expect("its own kind"),
            kind
        );
        assert_eq!(
            FoundryAccounts
                .provides(AccountRef::new("id-one", "another-kind"))
                .expect_err("not a kind it keeps")
                .status,
            Status::GUATIAO_ERR_BAD_VALUE
        );
    }

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
        request.settings = guatiao_serde::from_serde(json!({"model": model})).expect("value");
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
    fn a_lease_lists_the_models_of_its_engine_and_marks_the_one_the_account_serves() {
        let gateway = stand_in();
        let lease = gateway
            .open(request("a", "local-model"), spawner())
            .expect("opens");
        let listing = lease.models().expect("listed");
        assert!(listing.known, "{listing:?}");
        let ids: Vec<&str> = listing
            .models
            .iter()
            .map(|model| model.id.as_str())
            .collect();
        assert_eq!(ids, ["local-model"]);
        assert!(listing.models[0].is_default);
        assert_eq!(listing.models[0].description.as_deref(), Some("tools"));
        let other = gateway
            .open(request("b", "another-model"), spawner())
            .expect("opens");
        assert!(
            !other.models().expect("listed").models[0].is_default,
            "only the model the account serves is the one in use"
        );
    }

    #[test]
    fn the_account_provider_lists_none_for_a_model_a_gateway_serves() {
        let listing = FoundryAccounts
            .models(
                AccountModelsQuery::new(coqui_acp_api::AccountGrant::new("a", KIND)),
                spawner(),
                NoKeeper.into_object(),
            )
            .expect("answered");
        assert!(!listing.known && listing.unknown_because.is_some());
    }

    #[test]
    fn a_gateway_that_cannot_start_is_a_refusal_the_person_can_read() {
        let gateway = FoundryGateway::with_engines(Arc::new(|_| {
            Err("no Foundry Local service is running".to_owned())
        }));
        let Err(refused) = gateway.open(request("a", "local-model"), spawner()) else {
            panic!("no service");
        };
        assert_eq!(refused.message(), "no Foundry Local service is running");
        let mut stray = GatewayRequest::new("a");
        stray.settings =
            guatiao_serde::from_serde(json!({"model": "m", "password": "x"})).expect("value");
        let Err(stray) = gateway.open(stray, spawner()) else {
            panic!("a setting the kind does not have");
        };
        assert!(stray.message().contains("password"), "{}", stray.message());
        let Err(none) = gateway.open(GatewayRequest::new("a"), spawner()) else {
            panic!("no model");
        };
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
            asked.table = guatiao_serde::from_serde(table.clone()).expect("value");
            asked
        };
        let table = json!({"models": {"local-model": {"window": 16384}, "other": {"window": 999}, "zero": {"window": 0}, "text": {"window": "big"}, "bare": {}}});
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

    #[test]
    fn the_listing_carries_what_the_catalog_says_each_model_is_for_and_its_window() {
        let listed = listing(
            vec![
                ModelInfo::new("qwen")
                    .task("chat")
                    .tools(true)
                    .context(8192),
                ModelInfo::new("bge").task("embedding"),
                ModelInfo::new("whisper").task("speech"),
                ModelInfo::new("mystery"),
            ],
            "qwen",
        );
        let by = |id: &str| {
            listed
                .models
                .iter()
                .find(|model| model.id == id)
                .expect("listed")
        };
        assert_eq!(by("qwen").capabilities, ["chat", "tool_use"]);
        assert_eq!(
            by("qwen")
                .limits
                .as_ref()
                .and_then(|limits| limits.context_tokens),
            Some(8192)
        );
        assert_eq!(by("bge").capabilities, ["embed"]);
        assert_eq!(by("whisper").capabilities, ["speech_to_text"]);
        assert!(
            !by("mystery").reports_capabilities(),
            "a task nobody knows is not guessed"
        );
        assert!(by("qwen").is_default);
    }
}
