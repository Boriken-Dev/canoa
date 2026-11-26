//! The host's network: the default route of `[network]` installed as the process's, and the proxy variables of the programs the host starts.
//!
//! Every fetch the host makes builds its HTTP client from [`canoa::network::Network`], so they all follow one set of rules: the
//! system's proxy (PAC and WPAD included) and its login, the operating system's certificate verifier and the client certificates of
//! its keystores, with the route the person chose overriding any of it. The programs the host starts (agents, runtimes, git)
//! get the proxy as `HTTP_PROXY`, `HTTPS_PROXY` and `NO_PROXY` when their environment names none.

use core::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError, RwLock};

use canoa::network::{Network, NetworkSettings};

pub(crate) mod proxy_ca;
pub(crate) mod routes;
pub(crate) mod session;
#[cfg(test)]
pub(crate) mod standins;
#[cfg(test)]
pub(crate) mod tests;

use crate::config::{Config, ConfigError};

/// What the programs the host starts reach: the addresses the loaded libraries say, which the Store's programs (their endpoints and
/// the services they reach) and each agent vendor (the API and login hosts of its program) name. A PAC script cannot be written as
/// environment variables, so it is asked about these to choose the proxy a program is given. `[network] endpoints` adds to them.
static PROGRAM_ENDPOINTS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Adds the addresses `urls` to what the programs the host starts reach; one already there is not added again.
pub(crate) fn note_endpoints(urls: impl IntoIterator<Item = String>) {
    let mut kept = PROGRAM_ENDPOINTS
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    for url in urls {
        if !url.is_empty() && !kept.contains(&url) {
            kept.push(url);
        }
    }
}

/// What the programs the host starts reach, from every library loaded so far (the bundled ones first).
pub(crate) fn program_endpoints() -> Vec<String> {
    crate::vendors::load_bundled();
    PROGRAM_ENDPOINTS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

/// Makes `config`'s default route the process's network; see [`routes::install`].
///
/// # Errors
///
/// The default route does not open: its settings are wrong, a variable it names is not set, or what it runs does not start.
pub(crate) fn install(config: &Config) -> Result<(), ConfigError> {
    routes::install(config)
}

/// Records what the process's network was made from, and that it was made again.
fn set_installed(settings: NetworkSettings) {
    *INSTALLED.write().unwrap_or_else(PoisonError::into_inner) = Some(settings);
    GENERATION.fetch_add(1, Ordering::SeqCst);
}

/// How many times [`install`] has made a network: a proxy a session shares is one made from the same installation.
static GENERATION: AtomicU64 = AtomicU64::new(0);

pub(crate) fn installed_generation() -> u64 {
    GENERATION.load(Ordering::SeqCst)
}

/// What [`install`] last made the process's network from.
static INSTALLED: RwLock<Option<NetworkSettings>> = RwLock::new(None);

/// The settings the process's network was made from: the roots it trusts and the client certificate it presents, which a session's
/// own network keeps while it chooses its own proxy. The defaults when none was installed.
pub(crate) fn installed_settings() -> NetworkSettings {
    INSTALLED
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .unwrap_or_default()
}

/// The proxy variables of a program that reaches what programs usually reach.
pub(crate) fn program_variables(network: &Network) -> Vec<(String, String)> {
    network.child_variables(&program_endpoints())
}

/// The proxy variables of a program that reaches `urls`.
pub(crate) fn variables_for(network: &Network, urls: &[String]) -> Vec<(String, String)> {
    network.variables_for(urls)
}
