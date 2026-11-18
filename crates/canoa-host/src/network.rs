//! The host's network: `[host.network]` installed as the process's, and the proxy variables of the programs the host starts.
//!
//! Every fetch the host makes builds its HTTP client from [`canoa::network::Network`], so they all follow one set of rules: the
//! system's proxy (PAC and WPAD included) and its login, the operating system's certificate verifier and the client certificates of
//! its keystores, with the person's `[host.network]` overriding any of it. The programs the host starts (agents, runtimes, git)
//! get the proxy as `HTTP_PROXY`, `HTTPS_PROXY` and `NO_PROXY` when their environment names none.

use core::sync::atomic::{AtomicU64, Ordering};
use core::time::Duration;
use std::sync::{PoisonError, RwLock};

use canoa::network::{Network, NetworkSettings};

pub(crate) mod proxy_ca;
pub(crate) mod session;
#[cfg(test)]
pub(crate) mod standins;
#[cfg(test)]
pub(crate) mod tests;

use crate::config::{ConfigError, NetworkConfig};

/// What the programs the host starts reach: the runtimes' registries and release hosts, and the agents' own services. A PAC script
/// cannot be written as environment variables, so it is asked about these to choose the proxy a program is given. `[host.network]
/// endpoints` adds to them.
pub(crate) const PROGRAM_ENDPOINTS: &[&str] = &[
    "https://registry.npmjs.org/",
    "https://pypi.org/",
    "https://files.pythonhosted.org/",
    "https://github.com/",
    "https://api.github.com/",
    "https://objects.githubusercontent.com/",
    "https://raw.githubusercontent.com/",
    "https://nodejs.org/",
    "https://api.anthropic.com/",
    "https://api.openai.com/",
    "https://chatgpt.com/",
    "https://generativelanguage.googleapis.com/",
    "https://api.x.ai/",
];

/// Makes `config` the process's network.
///
/// # Errors
///
/// A setting that cannot be used: a proxy that cannot be read, a password variable that is not set, a certificate file that is
/// missing or holds nothing usable.
pub(crate) fn install(config: &NetworkConfig) -> Result<(), ConfigError> {
    let settings = config.settings()?;
    let network = Network::new(&settings).map_err(|error| ConfigError::Invalid {
        entry: None,
        what: format!("network: {error}"),
    })?;
    Network::install(network);
    *INSTALLED.write().unwrap_or_else(PoisonError::into_inner) = Some(settings);
    GENERATION.fetch_add(1, Ordering::SeqCst);
    session::registry().set_idle(Duration::from_secs(config.proxy_idle_seconds.unwrap_or(0)));
    Ok(())
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
    let endpoints: Vec<String> = PROGRAM_ENDPOINTS
        .iter()
        .map(|url| (*url).to_owned())
        .collect();
    network.child_variables(&endpoints)
}

/// The proxy variables of a program that reaches `urls`.
pub(crate) fn variables_for(network: &Network, urls: &[String]) -> Vec<(String, String)> {
    network.variables_for(urls)
}
