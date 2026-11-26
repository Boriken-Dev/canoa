//! The routes of the host: how its own traffic, an account's and a session's programs' leave, each a provider of the `network_route`
//! kind with settings the person wrote in `[network.routes]`.
//!
//! The host opens a route's lease once for every user whose route, settings and variables are equal and keeps it while the host runs
//! (a route's settings changing replaces it; what was started with the old one keeps it). Nothing here names a route but the two every
//! host has, `system` and `direct`.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "locks, threads and the clock are std's"
)]

use std::collections::{BTreeMap, HashMap};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::Duration;

use canoa::network::{Network, NetworkSettings};
use canoa::network_route::{HostVariable, LeaseStatus, OpenRequest, RouteInfo, RouteLease};
use coqui_acp_api::guatiao::library::Object;
use indexmap::IndexMap;
use serde_json::Value;
use sha2::{Digest, Sha256};

pub(crate) mod catalog;
mod command;
mod direct;
mod manual;
mod pac;
mod shared;
mod system;

pub(crate) use catalog::RouteCatalog;
use command::CommandRoute;
use direct::DirectRoute;
use manual::ManualRoute;
use pac::PacRoute;
use system::SystemRoute;

use crate::config::{Config, ConfigError, DIRECT_ROUTE, RouteConfig, SYSTEM_ROUTE};

coqui_acp_api::guatiao::local_providers!(
    id = "canoa-network-routes",
    version = env!("CARGO_PKG_VERSION"),
    providers = [
        SystemRoute,
        DirectRoute,
        ManualRoute,
        PacRoute,
        CommandRoute
    ]
);

/// How long a route may take to open when `[network] open_seconds` does not say.
const OPEN_SECONDS: u64 = 10;

/// Why a route could not be used, in words that name the route and the setting and hold no password.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RouteFailure {
    /// The route.
    pub(crate) route: String,
    /// What kind of failure.
    pub(crate) kind: FailureKind,
    /// What the person can act on.
    pub(crate) why: String,
}

/// What a [`RouteFailure`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FailureKind {
    /// The host has no such route, or its settings are wrong (a variable that is not set, a file that cannot be read).
    Settings,
    /// What the route runs did not start, or too many leases are open.
    Unreachable,
    /// It did not open in time.
    Timeout,
}

impl core::fmt::Display for RouteFailure {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "route {}: {}", self.route, self.why)
    }
}

impl core::error::Error for RouteFailure {}

/// What the host knows of its routes since the last [`install`].
struct State {
    routes: IndexMap<String, RouteConfig>,
    default: String,
    catalog: RouteCatalog,
    base_dir: Option<String>,
    open_seconds: u64,
    max_leases: Option<usize>,
    /// The route of each account that names one, by the account's id.
    accounts: BTreeMap<String, String>,
}

/// An open lease and what is made from it.
struct Held {
    network: Arc<Network>,
    settings: NetworkSettings,
    status: LeaseStatus,
    provider: String,
    _lease: Object<dyn RouteLease>,
}

static STATE: RwLock<Option<Arc<State>>> = RwLock::new(None);
static HELD: Mutex<Option<HashMap<String, Arc<Held>>>> = Mutex::new(None);

fn state() -> Option<Arc<State>> {
    STATE.read().unwrap_or_else(PoisonError::into_inner).clone()
}

fn failure(route: &str, kind: FailureKind, why: impl Into<String>) -> RouteFailure {
    RouteFailure {
        route: route.to_owned(),
        kind,
        why: why.into(),
    }
}

/// The host variables a route's settings name, as its provider's `secrets` say, that are set.
fn variables_of(info: &RouteInfo, settings: &Value) -> Vec<HostVariable> {
    info.secrets
        .iter()
        .filter_map(|key| settings.get(key)?.as_str())
        .filter_map(|name| Some(HostVariable::new(name, std::env::var(name).ok()?)))
        .collect()
}

/// What the lease of a route is, as the pool keys it: equal keys are one lease.
fn lease_key(provider: &str, settings: &Value, variables: &[HostVariable]) -> String {
    let mut hash = Sha256::new();
    for variable in variables {
        hash.update(variable.name.as_bytes());
        hash.update([0]);
        hash.update(variable.value.expose().as_bytes());
        hash.update([0]);
    }
    format!(
        "{provider}\n{settings}\n{}",
        data_encoding::HEXLOWER.encode(&hash.finalize())
    )
}

/// The provider and settings of the route `name`: a table of `[network.routes]`, or `system` and `direct` with none.
fn definition(state: &State, name: &str) -> Result<(String, Value), RouteFailure> {
    if let Some(route) = state.routes.get(name) {
        return Ok((route.provider.clone(), route.settings.clone()));
    }
    if name == SYSTEM_ROUTE || name == DIRECT_ROUTE {
        return Ok((name.to_owned(), Value::Object(serde_json::Map::new())));
    }
    Err(failure(
        name,
        FailureKind::Settings,
        "the host has no such route",
    ))
}

/// The request a lease of `name` is opened or judged with.
fn request_of(state: &State, scope: &str, info: &RouteInfo, settings: &Value) -> OpenRequest {
    let mut request = OpenRequest::new(scope);
    request.variables = variables_of(info, settings);
    request.base_dir.clone_from(&state.base_dir);
    request
}

/// The lease of `name`, opened now or found.
fn held(state: &State, name: &str, scope: &str) -> Result<Arc<Held>, RouteFailure> {
    let (provider, settings) = definition(state, name)?;
    let route = state
        .catalog
        .instance(&provider, &settings)
        .map_err(|why| failure(name, FailureKind::Settings, why))?;
    let info = route
        .describe()
        .map_err(|error| failure(name, FailureKind::Settings, error.to_string()))?;
    let request = request_of(state, scope, &info, &settings);
    let key = lease_key(&provider, &settings, &request.variables);
    {
        let mut held = HELD.lock().unwrap_or_else(PoisonError::into_inner);
        let held = held.get_or_insert_with(HashMap::new);
        if let Some(found) = held.get(&key) {
            return Ok(Arc::clone(found));
        }
        if state.max_leases.is_some_and(|max| held.len() >= max) {
            return Err(failure(
                name,
                FailureKind::Unreachable,
                "[network] max_leases leases are open already",
            ));
        }
    }
    let wait = Duration::from_secs(state.open_seconds);
    let (sender, receiver) = mpsc::channel();
    let opening = Arc::clone(&route);
    std::thread::spawn(move || {
        let _ = sender.send(opening.open(request));
    });
    let lease = match receiver.recv_timeout(wait) {
        Ok(Ok(lease)) => lease,
        Ok(Err(error)) => {
            return Err(failure(name, FailureKind::Unreachable, error.to_string()));
        }
        Err(_) => {
            return Err(failure(
                name,
                FailureKind::Timeout,
                format!("it did not open within {} seconds", state.open_seconds),
            ));
        }
    };
    let reach = lease
        .reach()
        .map_err(|error| failure(name, FailureKind::Unreachable, error.to_string()))?;
    let settings = reach
        .settings()
        .map_err(|error| failure(name, FailureKind::Settings, error.to_string()))?;
    let network = Network::new(&settings)
        .map_err(|error| failure(name, FailureKind::Settings, error.to_string()))?;
    let status = lease.status().unwrap_or_else(|_| LeaseStatus::new("open"));
    let made = Arc::new(Held {
        network: Arc::new(network),
        settings,
        status,
        provider,
        _lease: lease,
    });
    let mut held = HELD.lock().unwrap_or_else(PoisonError::into_inner);
    Ok(Arc::clone(
        held.get_or_insert_with(HashMap::new)
            .entry(key)
            .or_insert(made),
    ))
}

/// Makes `config`'s routes the host's: opens the default route and installs its network as the process's.
///
/// # Errors
///
/// The default route does not open: its settings are wrong, a variable it names is not set, or what it runs does not start.
pub(crate) fn install(config: &Config) -> Result<(), ConfigError> {
    let network = &config.network;
    let state = Arc::new(State::of(config));
    *HELD.lock().unwrap_or_else(PoisonError::into_inner) = None;
    *STATE.write().unwrap_or_else(PoisonError::into_inner) = Some(Arc::clone(&state));
    let host = held(&state, &state.default, "host").map_err(|error| ConfigError::Invalid {
        entry: None,
        what: format!("network: {error}"),
    })?;
    let mut settings = host.settings.clone();
    settings.endpoints.clone_from(&network.endpoints);
    let installed = Network::new(&settings).map_err(|error| ConfigError::Invalid {
        entry: None,
        what: format!("network: {error}"),
    })?;
    Network::install(installed);
    super::set_installed(settings);
    super::session::registry().set_idle(Duration::from_secs(network.idle_seconds.unwrap_or(0)));
    Ok(())
}

impl State {
    fn of(config: &Config) -> Self {
        let network = &config.network;
        Self {
            routes: network.routes.clone(),
            default: network.default_route().to_owned(),
            catalog: RouteCatalog::of(config),
            base_dir: config.network.base_dir.clone(),
            open_seconds: network.open_seconds.unwrap_or(OPEN_SECONDS),
            max_leases: network.max_leases,
            accounts: config
                .accounts
                .values()
                .filter_map(|account| Some((account.id.clone(), account.route.clone()?)))
                .collect(),
        }
    }
}

/// The network of the route `route`, or the host's when `None`.
///
/// # Errors
///
/// The host has no such route, or it does not open.
pub(crate) fn network(route: Option<&str>) -> Result<Arc<Network>, RouteFailure> {
    let Some(state) = state() else {
        return Ok(Network::current());
    };
    match route {
        None => Ok(Network::current()),
        Some(name) => Ok(Arc::clone(&held(&state, name, "host")?.network)),
    }
}

/// The settings of the network the route `route` makes, when `None` the host's: the roots it trusts, the certificate it presents and
/// the proxy it names.
///
/// # Errors
///
/// As [`network`].
pub(crate) fn settings(route: Option<&str>) -> Result<NetworkSettings, RouteFailure> {
    let Some(state) = state() else {
        return Ok(super::installed_settings());
    };
    match route {
        None => Ok(super::installed_settings()),
        Some(name) => Ok(held(&state, name, "host")?.settings.clone()),
    }
}

/// The network of the route the account whose id is `id` names, when it names one.
///
/// # Errors
///
/// The account's route is not one the host has, or does not open.
pub(crate) fn account_network(id: &str) -> Result<Option<Arc<Network>>, RouteFailure> {
    let Some(state) = state() else {
        return Ok(None);
    };
    account_network_in(&state, id)
}

fn account_network_in(state: &State, id: &str) -> Result<Option<Arc<Network>>, RouteFailure> {
    let Some(route) = state.accounts.get(id) else {
        return Ok(None);
    };
    Ok(Some(Arc::clone(
        &held(state, route, &format!("account:{id}"))?.network,
    )))
}

/// What the page says of a route: whether it opens, and what its lease is doing.
#[derive(Debug, Clone)]
pub(crate) struct Standing {
    /// How many leases of this route are open.
    pub(crate) leases: usize,
    /// What the last one says it is doing.
    pub(crate) status: Option<String>,
    /// What is wrong with its settings now.
    pub(crate) problems: Vec<String>,
}

/// How the route `name` of `config` stands, judged with nothing started.
pub(crate) fn standing(catalog: &RouteCatalog, config: &Config, name: &str) -> Standing {
    let (provider, settings) = match config.network.routes.get(name) {
        Some(route) => (route.provider.clone(), route.settings.clone()),
        None => (name.to_owned(), Value::Object(serde_json::Map::new())),
    };
    let problems = problems_of(catalog, config, &provider, &settings);
    let held = HELD.lock().unwrap_or_else(PoisonError::into_inner);
    let open: Vec<&Arc<Held>> = held
        .iter()
        .flat_map(HashMap::values)
        .filter(|each| each.provider == provider)
        .collect();
    Standing {
        leases: open.len(),
        status: open.last().map(|each| each.status.description.clone()),
        problems,
    }
}

/// What judging the provider `provider` with `settings` finds wrong; a route that cannot be made at all is one problem.
pub(crate) fn problems_of(
    catalog: &RouteCatalog,
    config: &Config,
    provider: &str,
    settings: &Value,
) -> Vec<String> {
    let route = match catalog.instance(provider, settings) {
        Ok(route) => route,
        Err(why) => return vec![why],
    };
    let info = match route.describe() {
        Ok(info) => info,
        Err(error) => return vec![error.to_string()],
    };
    let mut request = OpenRequest::new("host");
    request.variables = variables_of(&info, settings);
    request.base_dir = config.network.base_dir.clone();
    match route.check(request) {
        Ok(checked) => checked.problems,
        Err(error) => vec![error.to_string()],
    }
}

/// The routes of `config`'s host as the page lists them, in order: `system` and `direct`, then the person's.
pub(crate) fn names(config: &Config) -> Vec<String> {
    [SYSTEM_ROUTE, DIRECT_ROUTE]
        .into_iter()
        .map(str::to_owned)
        .chain(config.network.routes.keys().cloned())
        .collect()
}

#[cfg(test)]
mod tests;
