//! `reqwest::blocking` sending through the system's proxy and verifying with the system's TLS.
//!
//! The same builder as the [asynchronous adapter](super), for [`::reqwest::blocking::Client`].
//! With `failover` on (the default) the builder starts a [`coqui_proxy::relay::blocking`] relay:
//! a loopback proxy in this process, protected by a random credential, on its own threads, that
//! is the client's only proxy and sends every request through the family's blocking connector,
//! with its failover, `bad_proxy_ttl`, PAC, bypass rules, SOCKS, Negotiate and NTLM login and
//! proxy reply limits. The relay lives exactly as long as the client and all its clones.
//!
//! `reqwest::blocking` runs a Tokio runtime of its own on a thread of its own; the relay does
//! not add another: it has no runtime and needs none, so a client is built from a plain thread
//! (building a blocking client inside a Tokio runtime is `reqwest`'s own restriction, not this
//! adapter's).
//!
//! With `failover(false)` there is no relay and the proxy of each connection is the first usable
//! route of the [`Resolver`], as in the asynchronous adapter.

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::time::Duration;

use ::reqwest::blocking::{Client, ClientBuilder};
use coqui_proxy::relay::blocking::Relay;
use coqui_proxy::{ProxyConnector, RelaySettings, Resolver};

use super::{not_applied, relay_proxy, system_proxy};
use crate::{AdapterSettings, Error};

type Configure = Box<dyn FnOnce(ClientBuilder) -> ClientBuilder + Send>;

/// Builds a `reqwest::blocking` client that follows the system.
#[must_use]
pub struct Builder {
    resolver: Option<Resolver>,
    tls: Option<Arc<rustls::ClientConfig>>,
    connector: Option<ProxyConnector>,
    adapter: AdapterSettings,
    relay: RelaySettings,
    configure: Vec<Configure>,
}

impl core::fmt::Debug for Builder {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("Builder")
            .field("adapter", &self.adapter)
            .finish_non_exhaustive()
    }
}

impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}

impl Builder {
    /// Starts a client that follows the system: [`Resolver::system`] and the operating system's
    /// TLS verification.
    pub fn new() -> Self {
        Self {
            resolver: None,
            tls: None,
            connector: None,
            adapter: AdapterSettings::default(),
            relay: RelaySettings::default(),
            configure: Vec::new(),
        }
    }

    /// Who decides the proxy (default: [`Resolver::system`]).
    pub fn resolver(mut self, resolver: Resolver) -> Self {
        self.resolver = Some(resolver);
        self
    }

    /// The TLS configuration (default: [`coqui_sslcontext::client_config`], verified by the
    /// operating system). It is not told the host it will be used for.
    pub fn tls_config(mut self, config: Arc<rustls::ClientConfig>) -> Self {
        self.tls = Some(config);
        self
    }

    /// The connector that makes the connections of the relay: its Negotiate policy, proxy TLS,
    /// reply limits and user agent apply. Without the relay (`failover(false)`) `reqwest` makes
    /// its own connections and the connector is not used.
    pub fn connector(mut self, connector: ProxyConnector) -> Self {
        self.connector = Some(connector);
        self
    }

    /// Try the next route when connecting through one fails (default true). `reqwest` cannot
    /// retry inside a request, so this starts the relay of [`coqui_proxy::relay::blocking`]; off,
    /// the first usable route is the proxy and no relay is started.
    pub fn failover(mut self, failover: bool) -> Self {
        self.adapter.failover = failover;
        self
    }

    /// How long a proxy that failed to connect is tried after the others (default 300 s). It
    /// takes effect with the relay, that is with [`Builder::failover`] on.
    pub fn bad_proxy_ttl(mut self, ttl: Duration) -> Self {
        self.adapter.bad_proxy_ttl = ttl;
        self
    }

    /// Both settings of [`Builder::failover`] and [`Builder::bad_proxy_ttl`] at once.
    pub fn adapter_settings(mut self, settings: AdapterSettings) -> Self {
        self.adapter = settings;
        self
    }

    /// The bounds of the relay (default: [`RelaySettings::default`]): connections, head and idle
    /// times, head size, the wait for its threads when it stops. Its address must be a loopback
    /// one.
    pub fn relay_settings(mut self, settings: RelaySettings) -> Self {
        self.relay = settings;
        self
    }

    /// Changes the `reqwest` client builder before the client is made (timeouts, pool, headers,
    /// cookies, redirects). Its proxy and TLS settings are replaced by this builder's.
    pub fn configure(
        mut self,
        configure: impl FnOnce(ClientBuilder) -> ClientBuilder + Send + 'static,
    ) -> Self {
        self.configure.push(Box::new(configure));
        self
    }

    /// What this builder does that is looser than the defaults, the relay and the connector it
    /// was given included. A TLS configuration given to [`Builder::tls_config`] is a raw
    /// `rustls::ClientConfig`, which cannot say what it loosens: it is not inspected.
    #[must_use]
    pub fn non_defaults(&self) -> Vec<String> {
        let mut loose = self.adapter.non_defaults();
        if self.adapter.failover {
            loose.extend(self.relay.non_defaults());
            if let Some(connector) = &self.connector {
                loose.extend(connector.non_defaults());
            }
        }
        loose
    }

    /// What this adapter cannot do, because `reqwest` makes the connection itself: the list of
    /// the asynchronous adapter, which `reqwest::blocking` shares.
    #[must_use]
    pub fn not_applied(&self) -> Vec<&'static str> {
        not_applied(self.adapter.failover)
    }

    /// The `reqwest` client builder, with this builder's proxy and TLS set, to add what else a
    /// client needs before building it.
    ///
    /// # Errors
    ///
    /// [`Error::Tls`] when the operating system's verifier cannot start; with `failover` on,
    /// [`Error::Proxy`] when the relay cannot start (a refused [`RelaySettings`], the address in
    /// use, no thread).
    pub fn client_builder(self) -> Result<ClientBuilder, Error> {
        let config = match self.tls {
            Some(config) => config,
            None => coqui_sslcontext::client_config()?,
        };
        let resolver = self.resolver.unwrap_or_else(Resolver::system);
        let proxy = if self.adapter.failover {
            let connector = self
                .connector
                .unwrap_or_default()
                .with_failover(true, self.adapter.bad_proxy_ttl);
            let relay = Relay::builder(connector, resolver)
                .settings(self.relay)
                .start()?;
            let (url, name, token) = (
                relay.proxy_url(),
                relay.token_header().0,
                relay.token_header().1.to_string(),
            );
            relay_proxy(&url, name, &token, relay)?
        } else {
            system_proxy(resolver)
        };
        // The closures run first, so the proxy and the TLS configuration are the last word.
        let mut builder = ClientBuilder::new();
        for configure in self.configure {
            builder = configure(builder);
        }
        Ok(builder
            .use_preconfigured_tls((*config).clone())
            .proxy(proxy))
    }

    /// The client.
    ///
    /// # Errors
    ///
    /// As [`Builder::client_builder`], and [`Error::Http`] when `reqwest` refuses its
    /// configuration.
    pub fn build(self) -> Result<Client, Error> {
        self.client_builder()?
            .build()
            .map_err(|error| Error::Http(crate::Reason::new(error.without_url().to_string())))
    }
}

/// A client that follows the system, with the defaults of [`Builder`].
///
/// # Errors
///
/// As [`Builder::build`].
pub fn client() -> Result<Client, Error> {
    Builder::new().build()
}

/// Shorthand for `Builder::new()`.
pub fn builder() -> Builder {
    Builder::new()
}
