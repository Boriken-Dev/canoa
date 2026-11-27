//! What every HTTP-stack adapter (`ureq`, `hyper`, `reqwest`) is told about routes that fail.

use core::time::Duration;

/// How an adapter reacts to a route that does not connect.
///
/// The same two settings in every adapter: with `failover` the next route of an answer is tried
/// when connecting through one fails (never once a request was sent), and a proxy that failed is
/// tried after the others for `bad_proxy_ttl`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct AdapterSettings {
    /// Try the next route when one fails to connect (true).
    pub failover: bool,
    /// How long a proxy that failed to connect is tried last (300 s).
    pub bad_proxy_ttl: Duration,
}

impl AdapterSettings {
    /// The default failover.
    pub const DEFAULT_FAILOVER: bool = true;
    /// The default time a failed proxy is tried last.
    pub const DEFAULT_BAD_PROXY_TTL: Duration = Duration::from_secs(300);
}

impl Default for AdapterSettings {
    fn default() -> Self {
        Self {
            failover: Self::DEFAULT_FAILOVER,
            bad_proxy_ttl: Self::DEFAULT_BAD_PROXY_TTL,
        }
    }
}

impl AdapterSettings {
    /// What these settings do that is looser than the defaults: an empty list is the strict
    /// configuration.
    #[must_use]
    pub fn non_defaults(&self) -> alloc::vec::Vec<alloc::string::String> {
        let mut loose = alloc::vec::Vec::new();
        if !self.failover {
            loose.push(alloc::string::String::from(
                "failover(false): only the first route of an answer is tried",
            ));
        }
        loose
    }
}

/// The lines a [`Tls`](coqui_sslcontext::Tls) gives for what it does that is looser than the
/// defaults, in the words the other `non_defaults()` use.
#[cfg(feature = "tls")]
pub(crate) fn tls_non_defaults(
    tls: &coqui_sslcontext::Tls,
) -> alloc::vec::Vec<alloc::string::String> {
    tls.non_defaults()
        .into_iter()
        .map(|entry| {
            alloc::format!(
                "tls {}({}): the default is {}",
                entry.setting,
                entry.value,
                entry.default
            )
        })
        .collect()
}
