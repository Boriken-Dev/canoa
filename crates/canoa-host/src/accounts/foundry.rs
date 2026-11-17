//! Foundry Local as an account kind of the host's own.
//!
//! No provider lists it: the host adds the kind and its sign-in method to the catalog of each harness that can be pointed at a
//! gateway, and when a session starts on such an account the host starts the gateway, mints the session's bearer, and hands the
//! harness the account kind of its own that carries an address and a secret (a gateway token for Claude Code, an API key for
//! Codex). Nothing about the model is stored but its name and where the service is: the account holds no credential.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files, locks and threads are std's"
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};

use canoa_foundry::Engine;
use canoa_foundry::account::{self, EngineKind, KIND, Settings as Foundry};
use coqui_acp_api::guatiao::Value;
use coqui_acp_api::{
    AccountCatalog, AccountGrant, AccountKindInfo, Credentials, EndpointSlot, LoginMethodInfo,
    LoginOutcome, MODEL_API, standard,
};
use coqui_acp_harness::provider::schema;

use crate::config::Settings;
use crate::gateway::{Running, Ticket};
use crate::provider::map_of;

/// What opens the engine of an account: the real one reads the running service, a test's answers what it is told.
pub(crate) type Engines =
    Arc<dyn Fn(&Foundry) -> Result<Arc<dyn Engine>, String> + Send + Sync + 'static>;

/// The engine an account's settings ask for: the service the person runs, or the SDK in this process.
pub(crate) fn default_engine(settings: &Foundry) -> Result<Arc<dyn Engine>, String> {
    match settings.engine {
        EngineKind::Service => service_engine(settings),
        EngineKind::Sdk => sdk_engine(settings),
        _ => Err("this build does not know that engine".to_owned()),
    }
}

fn service_engine(settings: &Foundry) -> Result<Arc<dyn Engine>, String> {
    use canoa_foundry::daemon::{DaemonConfig, DaemonEngine};
    let mut config = DaemonConfig::new(settings.model.clone());
    config.endpoint.clone_from(&settings.endpoint);
    let home = std::env::home_dir().unwrap_or_default();
    DaemonEngine::open(&config, &home)
        .map(|engine| Arc::new(engine) as Arc<dyn Engine>)
        .map_err(|error| error.to_string())
}

#[cfg(feature = "foundry")]
fn sdk_engine(settings: &Foundry) -> Result<Arc<dyn Engine>, String> {
    use canoa_foundry::sdk::{SdkConfig, SdkEngine};
    let mut config = SdkConfig::new(settings.model.clone());
    config.library = settings.library.as_ref().map(std::path::PathBuf::from);
    SdkEngine::open(&config)
        .map(|engine| Arc::new(engine) as Arc<dyn Engine>)
        .map_err(|error| error.to_string())
}

#[cfg(not(feature = "foundry"))]
fn sdk_engine(_: &Foundry) -> Result<Arc<dyn Engine>, String> {
    Err("this build of canoa runs models through the service only: the sdk engine needs it built with the foundry feature".to_owned())
}

/// The gateways that are up, one per account while a session holds it, and how to make an engine for one.
#[derive(Default)]
pub(crate) struct Gateways {
    engines: OnceLock<Engines>,
    /// The file each gateway records its requests in.
    log: Option<PathBuf>,
    up: Mutex<BTreeMap<String, Weak<Running>>>,
}

impl core::fmt::Debug for Gateways {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Gateways").finish_non_exhaustive()
    }
}

/// The file in the host's home that the gateways record their requests in.
pub(crate) const GATEWAY_LOG: &str = "foundry-gateway.log";

impl Gateways {
    /// Gateways that record their requests in `log`.
    pub(crate) fn logging_to(log: Option<PathBuf>) -> Self {
        Self {
            log,
            ..Self::default()
        }
    }

    /// Chooses what opens engines, once; a host that never does opens what each account's settings ask for.
    #[cfg(test)]
    pub(crate) fn engines(&self, engines: Engines) {
        let _ = self.engines.set(engines);
    }

    /// A bearer on the gateway of account `id`, started if no session holds one.
    pub(crate) fn ticket(&self, id: &str, settings: &Foundry) -> Result<Ticket, String> {
        let mut up = self.up.lock().unwrap_or_else(PoisonError::into_inner);
        let running = match up.get(id).and_then(Weak::upgrade) {
            Some(running) => running,
            None => {
                let engines = self
                    .engines
                    .get()
                    .cloned()
                    .unwrap_or_else(|| Arc::new(default_engine));
                let engine = engines(settings)?;
                let running = Running::start(engine, self.log.clone())
                    .map_err(|error| format!("the gateway could not listen: {error}"))?;
                up.insert(id.to_owned(), Arc::downgrade(&running));
                running
            }
        };
        drop(up);
        running
            .mint()
            .map_err(|_| "the system has no randomness for a bearer".to_owned())
    }
}

/// Adds the kind and its sign-in method to `catalog`, for a harness that can be given a gateway.
pub(crate) fn extend(catalog: &mut AccountCatalog, harness: &str) {
    if account::route(harness).is_none() || catalog.kinds.iter().any(|kind| kind.id == KIND) {
        return;
    }
    let mut kind = AccountKindInfo::new(KIND, "Foundry Local");
    kind.description = "A model run by Foundry Local on this computer, served to the agent through a gateway the host runs for each session. No credential is kept: the model's name and where the service is are the account.".to_owned();
    kind.settings = schema(&account::settings_schema());
    kind.endpoints = vec![EndpointSlot::new(MODEL_API).key("endpoint")];
    // Codex runs in a folder of its own for the account, where the host writes its configuration.
    kind.state_dir = true;
    kind.notices = account::notices();
    // The gateway answers both wire formats, so an agent that speaks either can use the account.
    kind.provides = vec![
        standard::ANTHROPIC_API.to_owned(),
        standard::OPENAI_API.to_owned(),
    ];
    catalog.kinds.push(kind);
    let mut method = LoginMethodInfo::new(KIND, "Use a Foundry Local model", KIND);
    method.description = "Names a model Foundry Local serves. It is not loaded or checked here: load it in Foundry Local before a session.".to_owned();
    method.inputs = schema(&account::settings_schema());
    catalog.logins.push(method);
}

/// The account the sign-in method makes: its settings are its inputs, and it holds no credential.
///
/// # Errors
///
/// An input that is not a setting of the kind.
pub(crate) fn sign_in(
    inputs: &serde_json::Map<String, serde_json::Value>,
) -> Result<LoginOutcome, String> {
    let settings = Foundry::from_map(inputs)?;
    let mut outcome = LoginOutcome::new(KIND, Credentials::new());
    outcome.settings = Value::from(map_of(&Settings::new(inputs.clone()))?);
    outcome.label = Some(settings.model);
    Ok(outcome)
}

/// Why a session could not be given its gateway.
pub(crate) type Why = String;

/// The grant a harness is given in place of `grant`, a Foundry Local account's: the harness's own kind, the gateway's address
/// and the session's bearer; and the files its folder needs. `ticket` is held for as long as the session lives.
///
/// # Errors
///
/// Settings that are not the kind's, a harness no gateway can be given to, or a gateway that could not start.
pub(crate) fn grant(
    gateways: &Gateways,
    harness: &str,
    grant: &AccountGrant,
) -> Result<(AccountGrant, Ticket), Why> {
    let route =
        account::route(harness).ok_or_else(|| format!("{harness} cannot be given a gateway"))?;
    let table = grant_settings(grant)?;
    let settings = Foundry::from_map(&table)?;
    let ticket = gateways.ticket(&grant.id, &settings)?;
    let address = format!("{}{}", ticket.address(), route.suffix);
    let mut credentials = Credentials::new();
    credentials.insert(route.credential.to_owned(), ticket.bearer().to_owned());
    let mut kept = serde_json::Map::new();
    if let Some(key) = route.address {
        kept.insert(key.to_owned(), serde_json::Value::String(address.clone()));
    }
    for (key, value) in route.fixed {
        kept.insert(
            (*key).to_owned(),
            serde_json::Value::String((*value).to_owned()),
        );
    }
    let mut mapped = AccountGrant::new(grant.id.clone(), route.kind)
        .settings(Value::from(map_of(&Settings::new(kept))?))
        .credentials(credentials)
        .revision(grant.revision);
    if let Some(dir) = &grant.state_dir {
        mapped = mapped.state_dir(dir.clone());
        prepare(harness, &address, Path::new(dir))?;
    }
    Ok((mapped, ticket))
}

fn grant_settings(grant: &AccountGrant) -> Result<serde_json::Map<String, serde_json::Value>, Why> {
    guatiao_serde::adapt::from_value::<serde_json::Map<String, serde_json::Value>>(&grant.settings)
        .map_err(|_| "the account's settings are not a table".to_owned())
}

/// Writes what the harness reads from its folder beside the address: Codex's configuration, which turns off what a gateway
/// cannot serve, and Pi's, which points its OpenAI provider at the gateway.
fn prepare(harness: &str, address: &str, dir: &Path) -> Result<(), Why> {
    let (name, text) = match harness {
        "codex" => ("config.toml", account::codex_config().to_owned()),
        "pi" => ("models.json", account::pi_models(address)),
        _ => return Ok(()),
    };
    let file = dir.join(name);
    std::fs::write(&file, text).map_err(|error| format!("{}: {error}", file.display()))
}

#[cfg(test)]
mod tests;
