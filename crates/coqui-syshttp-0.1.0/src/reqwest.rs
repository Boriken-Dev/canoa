//! `reqwest` sending through the system's proxy and verifying with the system's TLS.
//!
//! The client is a plain [`::reqwest::Client`]: TLS is the family's rustls configuration
//! (`use_preconfigured_tls`). `reqwest` makes its connections itself and cannot retry a route, so
//! with `failover` on (the default) the builder starts a [`coqui_proxy::relay`]: a loopback proxy
//! in this process, protected by a random credential, that is the client's only proxy and sends
//! every request through the family's connector, with its failover, `bad_proxy_ttl`, PAC, bypass
//! rules, SOCKS, Negotiate and NTLM login and proxy reply limits. The relay lives exactly as long
//! as the client and all its clones. The relay runs on a Tokio runtime: [`Builder::runtime`] names one,
//! otherwise the one the building thread is in ([`coqui_proxy::relay::Relay::start`]), and building
//! outside any runtime without it is an error. [`blocking`] is the same for `reqwest::blocking`, whose relay
//! needs no runtime.
//!
//! With `failover(false)` there is no relay: the proxy of each connection is the first usable
//! route of the [`Resolver`] for that URL, asked again for every new connection, so a redirect to
//! another host is routed afresh (`reqwest` asks its `Proxy::custom` for every request it sends,
//! each redirect hop included: a test follows a redirect to another host name and sees both
//! asked). What that path cannot do is listed by [`Builder::not_applied`] rather than left out
//! silently.

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::time::Duration;

use ::reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use ::reqwest::{ClientBuilder, Proxy as ReqwestProxy, Url};
use coqui_proxy::relay::Relay;
use coqui_proxy::{Proxy, ProxyConnector, ProxyScheme, RelaySettings, Resolver, Route};

use crate::{AdapterSettings, Error};

#[cfg(feature = "reqwest-blocking")]
#[cfg_attr(docsrs, doc(cfg(feature = "reqwest-blocking")))]
pub mod blocking;

type Configure = Box<dyn FnOnce(ClientBuilder) -> ClientBuilder + Send>;

/// The URL a proxy that `reqwest` cannot use is replaced by, so a request fails instead of going
/// direct past the proxy the system asked for.
const UNUSABLE: &str = "http://unsupported-proxy.invalid/";

/// Builds a `reqwest` client that follows the system.
#[must_use]
pub struct Builder {
    resolver: Option<Resolver>,
    tls: Option<Arc<rustls::ClientConfig>>,
    connector: Option<ProxyConnector>,
    adapter: AdapterSettings,
    relay: RelaySettings,
    runtime: Option<tokio::runtime::Handle>,
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
            runtime: None,
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
    /// retry inside a request, so this starts the relay of [`coqui_proxy::relay`]; off, the
    /// first usable route is the proxy and no relay is started.
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
    /// times, head size. Its address must be a loopback one.
    pub fn relay_settings(mut self, settings: RelaySettings) -> Self {
        self.relay = settings;
        self
    }

    /// The Tokio runtime the relay runs on (default: the runtime the thread that builds the client is
    /// in; building outside one is an error). The runtime must outlive the client: when it is shut
    /// down, the relay stops serving. Without the relay (`failover(false)`) it is not used.
    pub fn runtime(mut self, runtime: tokio::runtime::Handle) -> Self {
        self.runtime = Some(runtime);
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

    /// What this adapter cannot do, because `reqwest` makes the connection itself.
    ///
    /// With the relay (`failover` on) `reqwest` only talks to a loopback proxy, so the list is
    /// short; without it the proxy is chosen by `reqwest` from the first usable route. With the
    /// relay it also lists the [`RelaySettings`] that mean nothing to a relay on a Tokio runtime.
    #[must_use]
    pub fn not_applied(&self) -> Vec<&'static str> {
        let mut list = not_applied(self.adapter.failover);
        if self.adapter.failover {
            list.extend(
                Relay::builder(ProxyConnector::default().asynch(), Resolver::system())
                    .not_applied(),
            );
        }
        list
    }

    /// The `reqwest` client builder, with this builder's proxy and TLS set, to add what else a
    /// client needs before building it.
    ///
    /// # Errors
    ///
    /// [`Error::Tls`] when the operating system's verifier cannot start; with `failover` on,
    /// [`Error::Proxy`] when the relay cannot start (no Tokio runtime, a refused
    /// [`RelaySettings`], the address in use).
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
                .with_failover(true, self.adapter.bad_proxy_ttl)
                .asynch();
            let relay = Relay::builder(connector, resolver).settings(self.relay);
            let relay = match &self.runtime {
                Some(runtime) => relay.start_on(runtime)?,
                None => relay.start()?,
            };
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
    pub fn build(self) -> Result<::reqwest::Client, Error> {
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
pub fn client() -> Result<::reqwest::Client, Error> {
    Builder::new().build()
}

/// Shorthand for `Builder::new()`.
pub fn builder() -> Builder {
    Builder::new()
}

/// What `reqwest`'s own connector cannot do, with the relay (`failover` on) or without it.
pub(crate) fn not_applied(failover: bool) -> Vec<&'static str> {
    let mut list = alloc::vec![
        "per-host certificate memory: the TLS configuration is chosen without the host name",
        "retries, token jars, hooks, the deadline across redirect hops and the per-hop Proxy-Authorization rules of the session client",
        "the session reply limit, decode chunk and interim-reply bounds: a reqwest body is the caller's to bound",
    ];
    if failover {
        list.push("URL-prefix gateways: skipped to the next route");
        list.push(
            "an Upgrade request over plain http: the relay forwards only CONNECT tunnels and plain requests",
        );
    } else {
        list.push(
            "Negotiate and NTLM proxy login, and the connector's reply limits and user agent",
        );
        list.push(
            "SOCKS 4 and 4a, URL-prefix gateways and proxies with extra headers: skipped to the next route",
        );
        list.push(
            "failover: the first usable route is the proxy, and a route that fails to connect fails the request",
        );
    }
    list
}

/// A `reqwest` proxy that is the relay for every URL, with the relay's credential.
///
/// The credential is the relay's token header, not `Proxy-Authorization`: `reqwest` removes that
/// header from a request when a redirect leads to another host and does not set it again, so a
/// plain `http:` request would get `407` on its second hop, while a proxy's own headers are sent
/// on every hop and on every `CONNECT`.
///
/// The relay is owned by the proxy's closure, which `reqwest` keeps for as long as the client and
/// its clones live, so the relay stops with the last of them and never before.
///
/// `owner` is whatever keeps the relay running (its handle): the closure owns it.
pub(crate) fn relay_proxy<T: Send + Sync + 'static>(
    url: &str,
    name: &str,
    token: &str,
    owner: T,
) -> Result<ReqwestProxy, Error> {
    let target = Url::parse(url)
        .map_err(|_| Error::Http(crate::Reason::new("the relay's address is not a URL")))?;
    let (Ok(name), Ok(mut token)) = (
        HeaderName::from_bytes(name.as_bytes()),
        HeaderValue::from_str(token),
    ) else {
        return Err(Error::Http(crate::Reason::new(
            "the relay's credential is not a header",
        )));
    };
    token.set_sensitive(true);
    let mut headers = HeaderMap::new();
    headers.insert(name, token);
    Ok(ReqwestProxy::custom(move |_url: &Url| -> Option<Url> {
        let _keep: &T = &owner;
        Some(target.clone())
    })
    .headers(headers))
}

/// A `reqwest` proxy that answers each URL with the first usable route of `resolver`.
pub(crate) fn system_proxy(resolver: Resolver) -> ReqwestProxy {
    ReqwestProxy::custom(move |url: &Url| -> Option<Url> {
        let routes = match resolver.lookup(url.as_str()) {
            Ok(routes) => routes,
            Err(error) => {
                // reqwest reads `None` as "no proxy": a failed lookup must not send the request direct.
                tracing::warn!(target: "coqui_syshttp", %error, "no route for a request; the request fails");
                return Url::parse(UNUSABLE).ok();
            }
        };
        let mut refused = Vec::new();
        for route in &routes {
            match route {
                Route::Direct => return None,
                Route::Proxy(proxy) => match usable(proxy) {
                    Ok(address) => return Some(address),
                    Err(reason) => refused.push((proxy.to_string(), reason)),
                },
                _ => {}
            }
        }
        tracing::warn!(target: "coqui_syshttp", refused = ?refused, "reqwest cannot use any proxy offered; the request fails");
        Url::parse(UNUSABLE).ok()
    })
}

/// A proxy as the URL `reqwest` reads, or why it cannot be used.
fn usable(proxy: &Proxy) -> Result<Url, &'static str> {
    if proxy.is_gateway() {
        return Err("a URL-prefix gateway");
    }
    if proxy.headers().next().is_some() {
        return Err("a proxy with extra headers");
    }
    let scheme = match proxy.scheme() {
        ProxyScheme::Http => "http",
        ProxyScheme::Https => "https",
        // The proxy resolves the name, as the connectors of this family do.
        ProxyScheme::Socks5 => "socks5h",
        ProxyScheme::Socks4 => return Err("SOCKS 4"),
        _ => return Err("a proxy scheme reqwest does not speak"),
    };
    let text = proxy.url_with_credentials();
    let rest = text
        .split_once("://")
        .map_or(text.as_str(), |(_, rest)| rest);
    Url::parse(&alloc::format!("{scheme}://{rest}")).map_err(|_| "an address reqwest cannot read")
}

/// A builder with the settings of a session [`SessionSettings`](crate::session::SessionSettings)
/// that `reqwest` can honour: the connect and read timeouts, the deadline as the timeout of a
/// whole request, the user agent and default headers, the idle connections kept per host and for
/// how long, and a redirect policy that follows up to `max_redirects`. A cookie jar of the session
/// layer plugs in with `reqwest::ClientBuilder::cookie_provider` through
/// [`Builder::configure`]. Retries, token jars, hooks, the reply limit and decoding bounds belong
/// to the session client: [`Builder::not_applied`] lists them.
#[cfg(feature = "session-reqwest")]
#[cfg_attr(docsrs, doc(cfg(feature = "session-reqwest")))]
pub fn builder_from(settings: &crate::session::SessionSettings) -> Builder {
    let mut headers = HeaderMap::new();
    for (name, value) in &settings.headers {
        let (Ok(name), Some(Ok(value))) = (
            HeaderName::from_bytes(name.as_bytes()),
            value.as_ref().map(|text| HeaderValue::from_str(text)),
        ) else {
            continue;
        };
        headers.insert(name, value);
    }
    let (connect, read, deadline) = (
        settings.connect_timeout,
        settings.read_timeout,
        settings.deadline,
    );
    let user_agent = settings.user_agent.clone();
    let per_host = settings.pool.max_idle_per_route;
    let idle = settings.pool.idle_timeout;
    let max_redirects = settings.max_redirects as usize;
    Builder::new().configure(move |mut builder| {
        if let Some(connect) = connect {
            builder = builder.connect_timeout(connect);
        }
        if let Some(read) = read {
            builder = builder.read_timeout(read);
        }
        if let Some(deadline) = deadline {
            builder = builder.timeout(deadline);
        }
        builder
            .user_agent(user_agent)
            .default_headers(headers)
            .pool_max_idle_per_host(per_host)
            .pool_idle_timeout(idle)
            .redirect(::reqwest::redirect::Policy::custom(move |attempt| {
                if attempt.previous().len() > max_redirects {
                    attempt.error("too many redirects")
                } else {
                    attempt.follow()
                }
            }))
    })
}
