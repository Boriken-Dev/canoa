//! The host's network: `[host.network]` installed as the process's, and the proxy variables of the programs the host starts.
//!
//! Every fetch the host makes builds its HTTP client from [`canoa::network::Network`], so they all follow one set of rules: the
//! system's proxy (PAC and WPAD included) and its login, the operating system's certificate verifier and the client certificates of
//! its keystores, with the person's `[host.network]` overriding any of it. The programs the host starts (agents, runtimes, git)
//! get the proxy as `HTTP_PROXY`, `HTTPS_PROXY` and `NO_PROXY` when their environment names none.

use canoa::network::Network;

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
    Ok(())
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
