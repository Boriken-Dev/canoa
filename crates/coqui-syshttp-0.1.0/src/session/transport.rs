//! What a client sends through: one request, one reply, no redirect, no credentials added.
//!
//! [`UreqTransport`], the default, sends on a `ureq` agent whose connections are made by this
//! crate's connector: every request asks the resolver for its routes, so a pooled connection is
//! keyed by its route and a redirect to another host is routed afresh.

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use core::time::Duration;
use std::io::{self, Read};
use std::sync::Mutex;
use std::time::Instant;

use coqui_proxy::auth::{OriginAuth, basic_header};
use coqui_proxy::{Environment, Proxy, ProxyConnector, ProxyScheme, ProxySetting, Resolver, Route};
use coqui_sslcontext::{Tls, Trust};
use ureq::config::{AutoHeaderValue, Config};
use ureq::http::{HeaderName, HeaderValue, Method, Request};

use super::body::Body;
use super::env::{CA_BUNDLE, HttpEnv};
use super::error::Error;
use super::headers::{Headers, peer_text};
use super::request::PreparedRequest;
use super::response::{Response, ResponseBuilder};
use super::settings::{PoolSettings, SessionSettings};
use crate::AdapterSettings;
use crate::ureq::{AgentBuilder, OriginAgent};

/// What [`Client`](super::Client) sends through. One request, one reply: no redirect, no
/// credentials added by the caller's layers. Routing, TLS, connection reuse and decoding a `gzip`
/// or `deflate` body are the transport's.
pub trait Transport: Send + Sync + fmt::Debug {
    /// Sends `request`. With `stream` false the reply's body is already read (a body that decodes
    /// to more than the reply limit is [`Error::TooLarge`]); with `stream` true it is read from
    /// the reply.
    ///
    /// # Errors
    ///
    /// [`Error::Timeout`], [`Error::Io`], [`Error::Proxy`], [`Error::Tls`] and
    /// [`Error::Configuration`] for the network and a setting that cannot be used.
    fn send(&self, request: PreparedRequest, stream: bool) -> Result<Response, Error>;

    /// Releases whatever the transport holds (idle connections). A later request connects afresh.
    fn close(&self);
}

/// A reader that fails a body that ends before the length the server stated.
struct Counted {
    inner: Box<dyn Read + Send>,
    expected: Option<u64>,
    received: u64,
}

impl Read for Counted {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let amount = self.inner.read(buffer)?;
        self.received += amount as u64;
        if amount == 0
            && !buffer.is_empty()
            && let Some(expected) = self.expected
            && self.received < expected
        {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                alloc::format!(
                    "the response ended after {} of {expected} bytes",
                    self.received
                ),
            ));
        }
        Ok(amount)
    }
}

#[derive(Clone)]
enum TlsChoice {
    /// A configuration of the caller's, used as it is.
    Fixed(Arc<Tls>),
    /// Built from this trust, and again after `tls_max_age`.
    Trust(Trust),
    /// The system's, or the bundle `CA_BUNDLE` names, and again after `tls_max_age`.
    Environment,
}

impl fmt::Debug for TlsChoice {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Fixed(_) => "Fixed",
            Self::Trust(_) => "Trust",
            Self::Environment => "Environment",
        })
    }
}

/// Makes a [`UreqTransport`].
#[derive(Debug)]
#[must_use]
pub struct UreqTransportBuilder {
    env: HttpEnv,
    resolver: Option<Resolver>,
    tls: TlsChoice,
    proxy_tls: Option<Arc<rustls::ClientConfig>>,
    connector: Option<ProxyConnector>,
    adapter: AdapterSettings,
    pool: PoolSettings,
    tls_max_age: Duration,
    max_reply_bytes: usize,
    decode_chunk: usize,
    origin_auth: Option<OriginAuth>,
    system_proxy_max_age: Duration,
    peer_text_limit: usize,
}

impl Default for UreqTransportBuilder {
    fn default() -> Self {
        let settings = SessionSettings::default();
        Self {
            env: HttpEnv::process(),
            resolver: None,
            tls: TlsChoice::Environment,
            proxy_tls: None,
            connector: None,
            adapter: AdapterSettings::default(),
            pool: settings.pool,
            tls_max_age: settings.tls_max_age,
            max_reply_bytes: settings.max_reply_bytes,
            decode_chunk: settings.decode_chunk,
            origin_auth: None,
            system_proxy_max_age: settings.system_proxy_max_age,
            peer_text_limit: settings.peer_text_limit,
        }
    }
}

impl UreqTransportBuilder {
    /// Where the transport reads its settings (default: the process, through `SYSHTTP_PREFIX`).
    pub fn env(mut self, env: HttpEnv) -> Self {
        self.env = env;
        self
    }

    /// Who decides the proxy (default: [`HttpEnv::resolver`] of the environment).
    pub fn resolver(mut self, resolver: Resolver) -> Self {
        self.resolver = Some(resolver);
        self
    }

    /// The TLS configuration for origins, used as it is (default: the system's, or the bundle
    /// `CA_BUNDLE` names).
    pub fn tls(mut self, tls: Tls) -> Self {
        self.tls = TlsChoice::Fixed(Arc::new(tls));
        self
    }

    /// Who is trusted for origins: [`Trust::Disabled`] turns verification off (the one way to),
    /// [`Trust::Bundle`] names a bundle, [`Trust::System`] is the default.
    pub fn trust(mut self, trust: Trust) -> Self {
        self.tls = TlsChoice::Trust(trust);
        self
    }

    /// The TLS configuration for an `https:` proxy or gateway (default: the system's). Given
    /// beside [`connector`](Self::connector) it is added to that connector, which keeps its bounds
    /// and login policy.
    pub fn proxy_tls(mut self, config: Arc<rustls::ClientConfig>) -> Self {
        self.proxy_tls = Some(config);
        self
    }

    /// How connections and proxy logins are made.
    pub fn connector(mut self, connector: ProxyConnector) -> Self {
        self.connector = Some(connector);
        self
    }

    /// The failover settings of the connections.
    pub fn adapter_settings(mut self, settings: AdapterSettings) -> Self {
        self.adapter = settings;
        self
    }

    /// The idle connections kept.
    pub fn pool(mut self, pool: PoolSettings) -> Self {
        self.pool = pool;
        self
    }

    /// How long the system's TLS configuration is reused (60 s).
    pub fn tls_max_age(mut self, age: Duration) -> Self {
        self.tls_max_age = age;
        self
    }

    /// Most bytes of a reply read whole (8 MiB).
    pub fn max_reply_bytes(mut self, limit: usize) -> Self {
        self.max_reply_bytes = limit;
        self
    }

    /// Most decoded bytes one decompression step produces (256 KiB).
    pub fn decode_chunk(mut self, chunk: usize) -> Self {
        self.decode_chunk = chunk;
        self
    }

    /// Answers an origin server's `401` with Basic, Negotiate or NTLM, as `auth` says. A request
    /// with a body that cannot be sent again is sent without it.
    pub fn origin_auth(mut self, auth: OriginAuth) -> Self {
        self.origin_auth = Some(auth);
        self
    }

    /// How long the system's proxy configuration is reused (60 s).
    pub fn system_proxy_max_age(mut self, age: Duration) -> Self {
        self.system_proxy_max_age = age;
        self
    }

    /// Takes the settings a [`SessionSettings`] holds: the pool, the TLS and proxy ages and the
    /// reply bounds.
    pub fn settings(mut self, settings: &SessionSettings) -> Self {
        self.pool = settings.pool.clone();
        self.tls_max_age = settings.tls_max_age;
        self.max_reply_bytes = settings.max_reply_bytes;
        self.decode_chunk = settings.decode_chunk;
        self.system_proxy_max_age = settings.system_proxy_max_age;
        self.peer_text_limit = settings.peer_text_limit;
        self
    }

    /// Most characters of text from the peer that an error message shows (120).
    pub fn peer_text_limit(mut self, limit: usize) -> Self {
        self.peer_text_limit = limit;
        self
    }

    /// What this transport cannot do, because `ureq` makes the exchange.
    #[must_use]
    pub fn not_applied(&self) -> Vec<&'static str> {
        alloc::vec![
            "max_interim_replies: ureq skips interim (1xx) replies itself and has no limit to set",
        ]
    }

    /// What this builder does that is looser than the defaults.
    #[must_use]
    pub fn non_defaults(&self) -> Vec<String> {
        let mut loose = self.adapter.non_defaults();
        match &self.tls {
            TlsChoice::Trust(Trust::Disabled) => {
                loose.push("trust(Disabled): certificates are not verified".to_string());
            }
            TlsChoice::Fixed(tls) => loose.extend(crate::adapter::tls_non_defaults(tls)),
            _ => {}
        }
        if let Some(connector) = &self.connector {
            loose.extend(connector.non_defaults());
        }
        if let Some(auth) = &self.origin_auth {
            loose.extend(auth.non_defaults());
        }
        loose
    }

    /// The transport.
    ///
    /// # Errors
    ///
    /// [`Error::Configuration`] naming a malformed proxy setting, [`Error::Tls`] when the
    /// verifier cannot start.
    pub fn build(self) -> Result<UreqTransport, Error> {
        self.env.check_proxy_settings()?;
        let mut resolver_settings = coqui_proxy::settings::ResolverSettings::default();
        resolver_settings.settings_refresh = self.system_proxy_max_age;
        resolver_settings.cache_ttl = Some(self.system_proxy_max_age);
        let resolver = self.resolver.unwrap_or_else(|| {
            self.env
                .resolver_with(super::env::ProxyOverrides::default(), resolver_settings)
        });
        let direct = Resolver::builder()
            .environment(Environment::from_pairs(Vec::<(String, String)>::new()))
            .proxy(ProxySetting::Direct)
            .system(false)
            .build();
        let mut connector = self.connector.unwrap_or_default();
        if let Some(config) = &self.proxy_tls {
            connector = connector.with_proxy_tls(Arc::clone(config));
        }
        let inner = Inner {
            env: self.env,
            resolver,
            direct,
            connector,
            adapter: self.adapter,
            choice: self.tls,
            pool: self.pool,
            tls_max_age: self.tls_max_age,
            max_reply_bytes: self.max_reply_bytes,
            decode_chunk: self.decode_chunk,
            origin_auth: self.origin_auth,
            peer_text_limit: self.peer_text_limit,
            cache: Mutex::new(None),
        };
        // The verifier is started now, so a setting that cannot be used is refused now.
        inner.agent_for(Key::default())?;
        Ok(UreqTransport {
            inner: Arc::new(inner),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Key {
    read_wait: Option<Duration>,
    gateway: bool,
    sensitive: bool,
}

struct Agents {
    built: Instant,
    tls: Arc<Tls>,
    map: Vec<(Key, ureq::Agent)>,
}

struct Inner {
    env: HttpEnv,
    resolver: Resolver,
    direct: Resolver,
    connector: ProxyConnector,
    adapter: AdapterSettings,
    choice: TlsChoice,
    pool: PoolSettings,
    tls_max_age: Duration,
    max_reply_bytes: usize,
    decode_chunk: usize,
    origin_auth: Option<OriginAuth>,
    peer_text_limit: usize,
    cache: Mutex<Option<Agents>>,
}

impl Inner {
    fn make_tls(&self) -> Result<Arc<Tls>, Error> {
        match &self.choice {
            TlsChoice::Fixed(tls) => Ok(Arc::clone(tls)),
            TlsChoice::Trust(trust) => Ok(Arc::new(Tls::builder().trust(trust.clone()).build()?)),
            TlsChoice::Environment => {
                let builder = match self.env.find(&[CA_BUNDLE])? {
                    Some((_, path)) => Tls::builder().trust(Trust::Bundle(path.into())),
                    None => Tls::builder(),
                };
                Ok(Arc::new(builder.build().map_err(|error| {
                    match error {
                        coqui_sslcontext::Error::Io(_) => Error::config(
                            self.env
                                .name(CA_BUNDLE)
                                .unwrap_or_else(|_| CA_BUNDLE.to_string()),
                            "names a CA bundle that cannot be read",
                        ),
                        other => Error::Tls(other),
                    }
                })?))
            }
        }
    }

    fn agent_for(&self, key: Key) -> Result<ureq::Agent, Error> {
        let mut guard = self
            .cache
            .lock()
            .map_err(|_| Error::Io(io::Error::other("the transport is poisoned")))?;
        let stale = guard.as_ref().is_none_or(|agents| {
            !matches!(self.choice, TlsChoice::Fixed(_)) && agents.built.elapsed() > self.tls_max_age
        });
        if stale {
            *guard = Some(Agents {
                built: Instant::now(),
                tls: self.make_tls()?,
                map: Vec::new(),
            });
        }
        let Some(agents) = guard.as_mut() else {
            return Err(Error::Io(io::Error::other("no agents")));
        };
        if let Some((_, agent)) = agents.map.iter().find(|(known, _)| *known == key) {
            return Ok(agent.clone());
        }
        let config = Config::builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .max_redirects_will_error(false)
            .user_agent(AutoHeaderValue::None)
            .accept(AutoHeaderValue::None)
            .accept_encoding(AutoHeaderValue::None)
            .max_idle_connections(self.pool.max_idle)
            .max_idle_connections_per_host(self.pool.max_idle_per_route)
            .max_idle_age(if key.sensitive {
                self.pool.reuse_window
            } else {
                self.pool.idle_timeout
            })
            .build();
        let mut builder = AgentBuilder::new()
            .resolver(if key.gateway {
                self.direct.clone()
            } else {
                self.resolver.clone()
            })
            .shared_tls(Arc::clone(&agents.tls))
            .connector(self.connector.clone())
            .config(config)
            .adapter_settings(self.adapter);
        if let Some(wait) = key.read_wait {
            builder = builder.read_wait(wait);
        }
        let agent = builder.build().map_err(|error| match error {
            crate::Error::Tls(error) => Error::Tls(error),
            crate::Error::Proxy(error) => Error::Proxy(error),
            other => Error::Io(io::Error::other(other.to_string())),
        })?;
        agents.map.push((key, agent.clone()));
        Ok(agent)
    }
}

/// The default transport: HTTP/1.1 on `ureq`, through the system's proxy and TLS.
///
/// Every request asks the resolver for its routes. An `http:` or `https:` proxy, a SOCKS proxy or
/// a direct connection is made by the connector; a URL-prefix gateway is asked for
/// `<gateway><path><URL>` with its credential as a header of that request, and the reply keeps
/// the URL asked for; a plain `http` gateway is never offered an `https:` origin. A request that
/// cannot be sent again uses an idle connection only if it carried a reply within the pool's
/// reuse window. Cheap to clone; clones share the connections.
#[derive(Clone)]
pub struct UreqTransport {
    inner: Arc<Inner>,
}

impl fmt::Debug for UreqTransport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UreqTransport")
            .field("tls", &self.inner.choice)
            .finish_non_exhaustive()
    }
}

impl UreqTransport {
    /// Starts a transport that follows the system.
    pub fn builder() -> UreqTransportBuilder {
        UreqTransportBuilder::default()
    }

    /// The transport that follows the system, with the defaults.
    ///
    /// # Errors
    ///
    /// As [`UreqTransportBuilder::build`].
    pub fn new() -> Result<Self, Error> {
        Self::builder().build()
    }
}

fn is_repeatable(method: &Method) -> bool {
    matches!(
        method.as_str(),
        "GET" | "HEAD" | "OPTIONS" | "PUT" | "DELETE" | "PROPFIND"
    )
}

fn map_error(error: ureq::Error, limit: usize) -> Error {
    match error {
        ureq::Error::Timeout(_) => Error::Timeout("the request timed out".to_string()),
        ureq::Error::Io(error)
            if matches!(
                error.kind(),
                io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
            ) =>
        {
            Error::Timeout("the request timed out".to_string())
        }
        ureq::Error::Io(error) => Error::Io(error),
        ureq::Error::HostNotFound => Error::Io(io::Error::new(
            io::ErrorKind::HostUnreachable,
            "the host name could not be resolved",
        )),
        ureq::Error::ConnectProxyFailed(reason) => Error::Io(io::Error::new(
            io::ErrorKind::ConnectionRefused,
            peer_text(&reason, limit),
        )),
        ureq::Error::BadUri(_) | ureq::Error::Http(_) => {
            Error::InvalidRequest("the request cannot be sent".to_string())
        }
        other => Error::Io(io::Error::new(
            io::ErrorKind::InvalidData,
            peer_text(&other.to_string(), limit),
        )),
    }
}

/// The route a request takes: the gateway that answers first, or none (the connector decides).
fn gateway_for(resolver: &Resolver, url: &url::Url) -> Result<Option<Proxy>, Error> {
    for route in resolver.lookup(url.as_str())? {
        match route {
            Route::Proxy(proxy) if proxy.is_gateway() => {
                if matches!(proxy.scheme(), ProxyScheme::Gateway { tls: false })
                    && url.scheme() == "https"
                {
                    // An https origin's request would cross a plain-http gateway in clear text.
                    continue;
                }
                return Ok(Some(proxy));
            }
            _ => return Ok(None),
        }
    }
    Err(Error::Io(io::Error::new(
        io::ErrorKind::Unsupported,
        "a plain-http gateway is never offered an https origin",
    )))
}

fn to_http_headers(
    headers: &Headers,
    extra: &[(String, String)],
) -> Result<ureq::http::HeaderMap, Error> {
    let mut map = ureq::http::HeaderMap::new();
    let bad = || Error::InvalidRequest("a header cannot be sent".to_string());
    let given: Vec<String> = headers
        .keys()
        .iter()
        .map(|name| name.to_ascii_lowercase())
        .collect();
    let pairs = headers.iter().chain(
        extra
            .iter()
            .filter(|(name, _)| !given.contains(&name.to_ascii_lowercase()))
            .map(|(n, v)| (n.as_str(), v.as_str())),
    );
    for (name, value) in pairs {
        let bytes: Vec<u8> = value.chars().map(|character| character as u8).collect();
        let mut value = HeaderValue::from_bytes(&bytes).map_err(|_| bad())?;
        if matches!(
            name.to_ascii_lowercase().as_str(),
            "authorization" | "cookie" | "proxy-authorization"
        ) {
            value.set_sensitive(true);
        }
        map.append(
            HeaderName::from_bytes(name.as_bytes()).map_err(|_| bad())?,
            value,
        );
    }
    Ok(map)
}

impl Transport for UreqTransport {
    fn send(&self, request: PreparedRequest, stream: bool) -> Result<Response, Error> {
        let inner = &self.inner;
        inner.env.check_proxy_settings()?;
        let PreparedRequest {
            method,
            url,
            headers,
            body,
            timeout,
            decode,
        } = request;
        let gateway = gateway_for(&inner.resolver, &url)?;
        let key = Key {
            read_wait: timeout.read,
            gateway: gateway.is_some(),
            sensitive: !(is_repeatable(&method) && body.is_replayable()),
        };
        let agent = inner.agent_for(key)?;
        let mut extra: Vec<(String, String)> = Vec::new();
        let target = match &gateway {
            Some(proxy) => {
                for (name, value) in proxy.headers() {
                    extra.push((name.to_string(), value.to_string()));
                }
                if let Some(credentials) = proxy.credentials() {
                    extra.push(("Proxy-Authorization".to_string(), basic_header(credentials)));
                }
                proxy.gateway_url(url.as_str()).ok_or_else(|| {
                    Error::InvalidRequest("the gateway cannot be asked for this URL".to_string())
                })?
            }
            None => url.to_string(),
        };
        let uri: ureq::http::Uri = target
            .parse()
            .map_err(|_| Error::InvalidRequest("the request URL cannot be sent".to_string()))?;
        let header_map = to_http_headers(&headers, &extra)?;
        let origin_login = inner
            .origin_auth
            .as_ref()
            .filter(|_| body.is_replayable() && gateway.is_none());

        let mut builder = Request::builder().method(method.clone()).uri(uri);
        *builder
            .headers_mut()
            .ok_or_else(|| Error::InvalidRequest("the request cannot be built".to_string()))? =
            header_map;
        let reply = if let Some(auth) = origin_login {
            let bytes = match body {
                Body::Bytes(bytes) => bytes,
                Body::Text(text) => text.into_bytes(),
                _ => Vec::new(),
            };
            let request = builder
                .body(bytes)
                .map_err(|_| Error::InvalidRequest("the request cannot be built".to_string()))?;
            let request = agent
                .configure_request(request)
                .timeout_connect(timeout.connect)
                .timeout_recv_response(timeout.read)
                .build();
            OriginAgent::new(agent.clone(), auth.clone()).run(request)
        } else {
            let send_body = match body {
                Body::Empty => ureq::SendBody::none(),
                other => ureq::SendBody::from_owned_reader(other.into_reader()),
            };
            let request = builder
                .body(send_body)
                .map_err(|_| Error::InvalidRequest("the request cannot be built".to_string()))?;
            let request = agent
                .configure_request(request)
                .timeout_connect(timeout.connect)
                .timeout_recv_response(timeout.read)
                .build();
            agent.run(request)
        }
        .map_err(|error| map_error(error, inner.peer_text_limit))?;

        let (parts, body) = reply.into_parts();
        let mut reply_headers = Headers::new();
        for (name, value) in &parts.headers {
            let text: String = value
                .as_bytes()
                .iter()
                .map(|byte| char::from(*byte))
                .collect();
            reply_headers.append(name.as_str(), text);
        }
        let bodiless = method == Method::HEAD
            || parts.status.is_informational()
            || matches!(parts.status.as_u16(), 204 | 304);
        let expected = (!bodiless)
            .then(|| {
                reply_headers
                    .get("Content-Length")
                    .and_then(|value| value.trim().parse::<u64>().ok())
            })
            .flatten();
        let reader: Box<dyn Read + Send> = Box::new(Counted {
            inner: Box::new(body.into_with_config().limit(u64::MAX).reader()),
            expected,
            received: 0,
        });
        let response = ResponseBuilder::new(parts.status.as_u16())
            .reason(parts.status.canonical_reason().unwrap_or(""))
            .headers(reply_headers)
            .url(url)
            .body(Body::Reader(reader))
            .decode(decode)
            .max_reply_bytes(inner.max_reply_bytes)
            .decode_chunk(inner.decode_chunk)
            .build();
        if stream {
            Ok(response)
        } else {
            response.buffered(None)
        }
    }

    fn close(&self) {
        if let Ok(mut guard) = self.inner.cache.lock() {
            *guard = None;
        }
    }
}
