//! The client: a transport, the headers and timeouts every request is sent with, the redirect
//! rules above the transport, and what a client keeps between requests (cookies, tokens, hooks,
//! retries).

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use core::time::Duration;
use std::collections::HashSet;
use std::sync::Mutex;
use std::thread::ThreadId;
use std::time::SystemTime;

use coqui_proxy::auth::{OriginAuth, basic_header};
use coqui_proxy::{ProxyConnector, Resolver, Secret};
use coqui_sslcontext::{Tls, Trust};
use regex::Regex;
use ureq::http::Method;
use url::Url;

use super::auth::{Auth, AuthProvider};
use super::body::Body;
use super::deadline::Deadline;
use super::env::HttpEnv;
use super::error::{BoxError, Error};
use super::headers::{Headers, check_headers, check_url, shown};
use super::jar::{FileCookieJar, FileTokenJar};
use super::redirect::{
    CREDENTIAL_HEADERS, is_followed, redirect_location, refused, resent_statuses,
};
use super::request::{PreparedRequest, Timeouts};
use super::response::Response;
use super::retry;
use super::settings::{CookieStore, SessionSettings, TokenStore};
use super::state::{Cookies, JarKey, Tokens, cookie_aliases};
use super::transport::{Transport, UreqTransport};
use crate::AdapterSettings;

/// What a hook is given to look at and change: the request as `RequestBuilder::send` is about to
/// send it.
#[derive(Debug)]
#[non_exhaustive]
pub struct HookRequest {
    /// The method.
    pub method: Method,
    /// The URL the caller asked for.
    pub url: Url,
    /// Headers laid over the client's: a value of `None` removes the name.
    pub headers: Vec<(String, Option<String>)>,
    /// The body.
    pub body: Body,
    /// Whether the reply's body is left to the caller.
    pub stream: bool,
    /// Whether redirects are followed (`None`: `GET`, `HEAD` and `OPTIONS` follow).
    pub redirects: Option<bool>,
    /// The waits of this request (`None`: the client's).
    pub timeout: Option<Timeouts>,
    /// The deadline of this request (`None`: the client's; `Some(None)`: none).
    pub deadline: Option<Option<Duration>>,
    /// The `Authorization` of this request.
    pub auth: Option<Auth>,
}

type HookFn = dyn Fn(&Client, &mut HookRequest) -> Result<Option<Response>, BoxError> + Send + Sync;

struct HookEntry {
    key: String,
    pattern: Regex,
    run: Box<HookFn>,
}

#[derive(Default)]
struct Busy {
    ensuring: HashSet<ThreadId>,
    in_hook: HashSet<ThreadId>,
}

struct Inner {
    settings: SessionSettings,
    transport: Arc<dyn Transport>,
    owned: bool,
    cookies: Cookies,
    tokens: Option<Tokens>,
    auth_provider: Option<Arc<dyn AuthProvider>>,
    hooks: Vec<HookEntry>,
    busy: Mutex<Busy>,
}

/// A transport with the settings every request is sent with, the rules for what happens after a
/// redirect, and the state that belongs to a client. Cheap to clone; clones share everything.
///
/// The client, not the transport, follows redirects. `GET`, `HEAD` and `OPTIONS` follow up to
/// `max_redirects`; a redirect that changes the scheme, host or port drops `Authorization`,
/// `Cookie` and `Proxy-Authorization` (and nothing else; the jar's cookies are chosen again for
/// each hop). Any other method is sent without following: a 307 or 308 (for `PROPFIND` also a
/// 301, 302 or 303) to the same scheme, host and port is sent once more with the same method and
/// body, and any other 3xx, a redirect to another origin or a second redirect is an
/// [`Error::Redirect`].
///
/// State is chosen by the host of the URL asked for (the origin), never by a proxy's or a
/// gateway's. In [`RequestBuilder::send`] (not in [`Client::follow`]): the auth provider runs
/// first and its errors are swallowed; hooks run on the caller's URL, and a request a hook makes
/// through the client runs none of them again (the guard is per thread); a status in the retry
/// list, or a failed connection, is retried for a method that is safe to send twice, and a status
/// that outlasts the retries is returned, not raised.
#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
}

impl fmt::Debug for Client {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Headers are left out: they carry credentials.
        formatter.write_str("Client")
    }
}

/// Makes a [`Client`].
#[must_use]
pub struct ClientBuilder {
    settings: SessionSettings,
    env: Option<HttpEnv>,
    resolver: Option<Resolver>,
    tls: Option<Tls>,
    proxy_tls: Option<Arc<rustls::ClientConfig>>,
    trust: Option<Trust>,
    connector: Option<ProxyConnector>,
    adapter: AdapterSettings,
    origin_auth: Option<OriginAuth>,
    transport: Option<Arc<dyn Transport>>,
    auth_provider: Option<Arc<dyn AuthProvider>>,
    hooks: Vec<HookEntry>,
}

impl fmt::Debug for ClientBuilder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClientBuilder")
            .field("settings", &self.settings.non_defaults())
            .finish_non_exhaustive()
    }
}

impl Default for ClientBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ClientBuilder {
    /// A client with Python syshttp's defaults.
    pub fn new() -> Self {
        Self {
            settings: SessionSettings::default(),
            env: None,
            resolver: None,
            tls: None,
            proxy_tls: None,
            trust: None,
            connector: None,
            adapter: AdapterSettings::default(),
            origin_auth: None,
            transport: None,
            auth_provider: None,
            hooks: Vec::new(),
        }
    }

    /// All the settings at once.
    pub fn settings(mut self, settings: SessionSettings) -> Self {
        self.settings = settings;
        self
    }

    /// A header sent with every request, laid over the user agent; a `None` value removes the
    /// name (the default `Accept: */*` included).
    pub fn header(mut self, name: impl Into<String>, value: Option<impl Into<String>>) -> Self {
        self.settings
            .headers
            .push((name.into(), value.map(Into::into)));
        self
    }

    /// The `User-Agent` (default `coqui-syshttp/<version>`).
    pub fn user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.settings.user_agent = user_agent.into();
        self
    }

    /// The time to connect and the time to wait for the server at each step of a reply (10 s and
    /// 60 s); `None` waits without limit.
    pub fn timeout(mut self, connect: Option<Duration>, read: Option<Duration>) -> Self {
        self.settings.connect_timeout = connect;
        self.settings.read_timeout = read;
        self
    }

    /// The time a whole request may take, redirects, retries and the reading of the body included
    /// (default none). Zero is refused when the client is built.
    pub fn deadline(mut self, deadline: Option<Duration>) -> Self {
        self.settings.deadline = deadline;
        self
    }

    /// Where the client and its transport read their settings (default: the process).
    pub fn env(mut self, env: HttpEnv) -> Self {
        self.env = Some(env);
        self
    }

    /// Who decides the proxy of the default transport (default: the settings of the environment).
    pub fn resolver(mut self, resolver: Resolver) -> Self {
        self.resolver = Some(resolver);
        self
    }

    /// The TLS configuration of the default transport for origins, used as it is.
    pub fn tls(mut self, tls: Tls) -> Self {
        self.tls = Some(tls);
        self
    }

    /// The TLS configuration of the default transport for an `https:` proxy or gateway.
    pub fn proxy_tls(mut self, config: Arc<rustls::ClientConfig>) -> Self {
        self.proxy_tls = Some(config);
        self
    }

    /// Who the default transport trusts for origins: [`Trust::Disabled`] turns verification off,
    /// [`Trust::Bundle`] names a bundle (default: `CA_BUNDLE`, else the system's).
    pub fn trust(mut self, trust: Trust) -> Self {
        self.trust = Some(trust);
        self
    }

    /// How the default transport connects.
    pub fn connector(mut self, connector: ProxyConnector) -> Self {
        self.connector = Some(connector);
        self
    }

    /// The failover settings of the default transport.
    pub fn adapter_settings(mut self, settings: AdapterSettings) -> Self {
        self.adapter = settings;
        self
    }

    /// Answers an origin server's `401` with Basic, Negotiate or NTLM in the default transport.
    pub fn origin_auth(mut self, auth: OriginAuth) -> Self {
        self.origin_auth = Some(auth);
        self
    }

    /// A transport of your own, which stays yours: the settings that configure the default
    /// transport (`resolver`, `tls`, `proxy_tls`, `trust`) are refused beside it.
    pub fn transport(mut self, transport: impl Transport + 'static) -> Self {
        self.transport = Some(Arc::new(transport));
        self
    }

    /// Where cookies are kept (default: in memory).
    pub fn cookies(mut self, store: CookieStore) -> Self {
        self.settings.cookies.store = store;
        self
    }

    /// Where tokens are read (default: none).
    pub fn tokens(mut self, store: TokenStore) -> Self {
        self.settings.tokens.store = store;
        self
    }

    /// Groups of hosts that share cookies (default: `SYSHTTP_COOKIE_HOSTS`).
    pub fn cookie_hosts<G, H>(mut self, groups: impl IntoIterator<Item = G>) -> Self
    where
        G: IntoIterator<Item = H>,
        H: Into<String>,
    {
        self.settings.cookies.hosts = Some(
            groups
                .into_iter()
                .map(|group| group.into_iter().map(Into::into).collect())
                .collect(),
        );
        self
    }

    /// Makes the client authenticated before each request.
    pub fn auth_provider(mut self, provider: Arc<dyn AuthProvider>) -> Self {
        self.auth_provider = Some(provider);
        self
    }

    /// Runs `hook` before each request whose URL `pattern` is found in, in the order hooks were
    /// added. It may change the request in place, and may return a reply to be used instead of
    /// sending. A hook that fails is an [`Error::Hook`]; a request a hook makes through the
    /// client runs no hook.
    pub fn hook(
        mut self,
        pattern: Regex,
        hook: impl Fn(&Client, &mut HookRequest) -> Result<Option<Response>, BoxError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        let key = alloc::format!("HOOK{}", self.hooks.len() + 1);
        self.hooks.push(HookEntry {
            key,
            pattern,
            run: Box::new(hook),
        });
        self
    }

    /// The client.
    ///
    /// # Errors
    ///
    /// [`Error::Configuration`] naming the setting or the variable for one that cannot be used (a
    /// zero deadline or timeout, a malformed proxy or cookie-host variable),
    /// [`Error::InvalidRequest`] for a transport of your own beside settings that configure the
    /// default one, and [`Error::Tls`] when the system's verifier cannot start.
    pub fn build(self) -> Result<Client, Error> {
        let settings = self.settings;
        let zero = |name: &str, value: Option<Duration>| {
            if value.is_some_and(|limit| limit.is_zero()) {
                Err(Error::config(name, "must be a positive duration, or none"))
            } else {
                Ok(())
            }
        };
        zero("deadline", settings.deadline)?;
        zero("connect_timeout", settings.connect_timeout)?;
        zero("read_timeout", settings.read_timeout)?;
        let env = self.env.unwrap_or_default();
        env.check_proxy_settings()?;
        let (transport, owned) = match self.transport {
            Some(transport) => {
                if self.resolver.is_some()
                    || self.tls.is_some()
                    || self.proxy_tls.is_some()
                    || self.trust.is_some()
                    || self.connector.is_some()
                    || self.origin_auth.is_some()
                {
                    return Err(Error::InvalidRequest(
                        "resolver, tls, proxy_tls, trust, connector and origin_auth configure the default transport: a client given a transport leaves them to it"
                            .to_string(),
                    ));
                }
                (transport, false)
            }
            None => {
                let mut builder = UreqTransport::builder()
                    .env(env.clone())
                    .settings(&settings)
                    .adapter_settings(self.adapter);
                if let Some(resolver) = self.resolver {
                    builder = builder.resolver(resolver);
                }
                if let Some(tls) = self.tls {
                    builder = builder.tls(tls);
                }
                if let Some(trust) = self.trust {
                    builder = builder.trust(trust);
                }
                if let Some(config) = self.proxy_tls {
                    builder = builder.proxy_tls(config);
                }
                if let Some(connector) = self.connector {
                    builder = builder.connector(connector);
                }
                if let Some(auth) = self.origin_auth {
                    builder = builder.origin_auth(auth);
                }
                (Arc::new(builder.build()?) as Arc<dyn Transport>, true)
            }
        };
        let aliases = cookie_aliases(settings.cookies.hosts.as_ref(), &env)?;
        let (jar, key): (Option<Arc<dyn super::jar::CookieJar>>, JarKey) =
            match &settings.cookies.store {
                CookieStore::Memory => (None, JarKey::ByHost),
                CookieStore::PerHost => (
                    Some(Arc::new(
                        FileCookieJar::builder().env(env.clone()).build_cookies(),
                    )),
                    JarKey::ByHost,
                ),
                CookieStore::Key(name) => (
                    Some(Arc::new(
                        FileCookieJar::builder().env(env.clone()).build_cookies(),
                    )),
                    JarKey::Named(name.clone()),
                ),
                CookieStore::Jar(jar) => (Some(Arc::clone(jar)), JarKey::ByHost),
            };
        let tokens = match &settings.tokens.store {
            TokenStore::None => None,
            TokenStore::PerHost => Some(Tokens::new(
                Arc::new(FileTokenJar::builder().env(env.clone()).build_tokens()),
                JarKey::ByHost,
            )),
            TokenStore::Key(name) => Some(Tokens::new(
                Arc::new(FileTokenJar::builder().env(env.clone()).build_tokens()),
                JarKey::Named(name.clone()),
            )),
            TokenStore::Jar(jar) => Some(Tokens::new(Arc::clone(jar), JarKey::ByHost)),
        };
        let cookies = Cookies::new(jar, key, aliases, settings.cookies.clone());
        Ok(Client {
            inner: Arc::new(Inner {
                settings,
                transport,
                owned,
                cookies,
                tokens,
                auth_provider: self.auth_provider,
                hooks: self.hooks,
                busy: Mutex::new(Busy::default()),
            }),
        })
    }
}

/// One request to send: made by [`Client::request`] or [`Client::follow`], sent by
/// [`RequestBuilder::send`].
#[must_use]
pub struct RequestBuilder {
    client: Client,
    method: Method,
    url: String,
    body: Body,
    headers: Vec<(String, Option<String>)>,
    stream: bool,
    redirects: Option<bool>,
    timeout: Option<Timeouts>,
    deadline: Option<Option<Duration>>,
    auth: Option<Auth>,
    followed_from: Option<Url>,
    pending: Option<Error>,
}

impl fmt::Debug for RequestBuilder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RequestBuilder")
            .field("method", &self.method)
            .finish_non_exhaustive()
    }
}

impl RequestBuilder {
    /// The body; only [`Body::Empty`], [`Body::Bytes`] and [`Body::Text`] can be sent again after
    /// a redirect, a dropped connection or a retry.
    pub fn body(mut self, body: impl Into<Body>) -> Self {
        self.body = body.into();
        self
    }

    /// A header of this request, laid over the client's; a `None` value removes the name.
    pub fn header(mut self, name: impl Into<String>, value: Option<impl Into<String>>) -> Self {
        self.headers.push((name.into(), value.map(Into::into)));
        self
    }

    /// Leaves the reply's body to the caller (default: it is read before `send` returns, and a
    /// decoded body over the reply limit is [`Error::TooLarge`]).
    pub fn stream(mut self, stream: bool) -> Self {
        self.stream = stream;
        self
    }

    /// `false` returns the first answer of a `GET`, `HEAD` or `OPTIONS` as it is.
    pub fn redirects(mut self, follow: bool) -> Self {
        self.redirects = Some(follow);
        self
    }

    /// The waits of this request: to connect, and at each step of the reply.
    pub fn timeout(mut self, connect: Option<Duration>, read: Option<Duration>) -> Self {
        self.timeout = Some(Timeouts { connect, read });
        self
    }

    /// The deadline of this request (`None`: none, whatever the client has).
    pub fn deadline(mut self, deadline: Option<Duration>) -> Self {
        self.deadline = Some(deadline);
        self
    }

    /// The `Authorization` of this request, sent to the origin of its URL unless a header names an
    /// `Authorization`; it beats a token of the client.
    pub fn auth(mut self, auth: Auth) -> Self {
        self.auth = Some(auth);
        self
    }

    /// Sends the request and returns the final reply.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidRequest`] for a URL (credentials in it included) or a header with a control
    /// character, before anything is sent; [`Error::Redirect`] for a redirect that is not
    /// followed; [`Error::Timeout`], [`Error::Io`], [`Error::Proxy`], [`Error::TooLarge`],
    /// [`Error::Hook`] and [`Error::Configuration`] as they apply.
    pub fn send(self) -> Result<Response, Error> {
        let client = self.client.clone();
        client.run(self)
    }
}

impl Client {
    /// Starts a client with the defaults.
    pub fn builder() -> ClientBuilder {
        ClientBuilder::new()
    }

    /// The settings the client was built with.
    #[must_use]
    pub fn settings(&self) -> &SessionSettings {
        &self.inner.settings
    }

    /// The transport: the one the client was given, or the one it made.
    #[must_use]
    pub fn transport(&self) -> &dyn Transport {
        &*self.inner.transport
    }

    /// Releases the connections of the transport the client made for itself; one it was given
    /// stays the caller's. A later request connects afresh. Harmless when called again.
    pub fn close(&self) {
        if self.inner.owned {
            self.inner.transport.close();
        }
    }

    /// A request: `method` to `url`.
    pub fn request(&self, method: Method, url: impl AsRef<str>) -> RequestBuilder {
        RequestBuilder {
            client: self.clone(),
            method,
            url: url.as_ref().to_string(),
            body: Body::Empty,
            headers: Vec::new(),
            stream: false,
            redirects: None,
            timeout: None,
            deadline: None,
            auth: None,
            followed_from: None,
            pending: None,
        }
    }

    /// Where `response`, the answer to a request for `url`, redirects to, as a request of
    /// `method`: it carries what `url` is sent with, except that `Authorization`, `Cookie` and
    /// `Proxy-Authorization` are left off whenever it goes to another scheme, host or port than
    /// `url`'s, and what it redirects to is followed in turn. A redirect with no usable `Location`
    /// is an [`Error::Redirect`] at `send`. Cookies apply to every hop; the token, hooks and
    /// retries belong to [`Client::request`].
    pub fn follow(&self, method: Method, url: &str, response: &Response) -> RequestBuilder {
        let mut builder = self.request(method.clone(), "");
        match check_url(url) {
            Err(error) => builder.pending = Some(error),
            Ok(origin) => match redirect_location(&origin, response) {
                None => {
                    builder.pending = Some(refused(
                        &method,
                        &origin,
                        response,
                        None,
                        "no usable Location",
                        self.inner.settings.peer_text_limit,
                    ));
                }
                Some(location) => {
                    builder.url = location.to_string();
                    builder.followed_from = Some(origin);
                }
            },
        }
        builder
    }

    // -- headers, deadline

    fn sent_headers(&self, headers: &[(String, Option<String>)], auth: Option<&Auth>) -> Headers {
        let settings = &self.inner.settings;
        let base = [("User-Agent".to_string(), Some(settings.user_agent.clone()))];
        let mut sent = Headers::merged([&base[..], &settings.headers, headers]);
        if let Some(auth) = auth
            && !sent.contains("Authorization")
        {
            let value = match auth {
                Auth::Basic(credentials) => basic_header(credentials),
                Auth::Header(secret) => secret.expose().to_string(),
            };
            sent.insert("Authorization", value);
        }
        sent
    }

    fn clock(
        &self,
        deadline: Option<Option<Duration>>,
        method: &Method,
        url: &Url,
    ) -> Result<Option<Deadline>, Error> {
        let seconds = deadline.unwrap_or(self.inner.settings.deadline);
        if seconds.is_some_and(|limit| limit.is_zero()) {
            return Err(Error::config(
                "deadline",
                "must be a positive duration, or none",
            ));
        }
        Ok(
            seconds
                .map(|seconds| Deadline::new(seconds, alloc::format!("{method} {}", shown(url)))),
        )
    }

    // -- the guards

    fn guarded(&self, ensuring: bool) -> bool {
        let id = std::thread::current().id();
        self.inner.busy.lock().is_ok_and(|busy| {
            if ensuring {
                busy.ensuring.contains(&id)
            } else {
                busy.in_hook.contains(&id)
            }
        })
    }

    fn set_guard(&self, ensuring: bool, on: bool) {
        let id = std::thread::current().id();
        if let Ok(mut busy) = self.inner.busy.lock() {
            let set = if ensuring {
                &mut busy.ensuring
            } else {
                &mut busy.in_hook
            };
            if on {
                set.insert(id);
            } else {
                set.remove(&id);
            }
        }
    }

    fn ensure(&self, url: &Url) {
        let Some(provider) = &self.inner.auth_provider else {
            return;
        };
        if self.guarded(true) {
            return;
        }
        self.set_guard(true, true);
        // Its errors are swallowed: a request goes on as it is.
        let _ = provider.ensure(self, url);
        self.set_guard(true, false);
    }

    // -- sending

    /// One request through the transport, with the cookies of `url` and the ones the reply sets.
    fn state_send(
        &self,
        method: &Method,
        url: &Url,
        headers: &Headers,
        body: Body,
        timeout: Timeouts,
    ) -> Result<Response, Error> {
        self.inner.cookies.before(url);
        let mut sent = headers.clone();
        if !sent.contains("Cookie")
            && let Some(cookie) = self.inner.cookies.header(url, SystemTime::now())
        {
            sent.insert("Cookie", cookie.expose());
        }
        let prepared = PreparedRequest::prepare(method.clone(), url.as_str(), sent, body, timeout)?;
        let response = self.inner.transport.send(prepared, true)?;
        let values: Vec<String> = response
            .headers()
            .get_all("Set-Cookie")
            .into_iter()
            .map(str::to_string)
            .collect();
        if !values.is_empty() {
            self.inner.cookies.store(url, &values, SystemTime::now());
        }
        Ok(response)
    }

    fn capped(timeout: Timeouts, clock: Option<&Deadline>) -> Result<Timeouts, Error> {
        clock.map_or(Ok(timeout), |clock| clock.cap(timeout))
    }

    fn drain(&self, response: Response) {
        response.discard(Some(64 * 1024));
    }

    /// A request that carries no body, following its redirects up to `max_redirects`.
    fn send_following(
        &self,
        method: &Method,
        url: &Url,
        headers: Headers,
        timeout: Timeouts,
        first: Option<&Url>,
        clock: Option<&Deadline>,
    ) -> Result<Response, Error> {
        let home = first.unwrap_or(url).origin();
        let mut headers = headers;
        let mut method = method.clone();
        let mut current = url.clone();
        let mut hops = 0_u32;
        loop {
            if current.origin() != home {
                for name in CREDENTIAL_HEADERS {
                    headers.remove(name);
                }
            }
            let response = self.state_send(
                &method,
                &current,
                &headers,
                Body::Empty,
                Self::capped(timeout, clock)?,
            )?;
            if !response.is_redirect() {
                return Ok(response);
            }
            let limit = self.inner.settings.peer_text_limit;
            let Some(location) = redirect_location(&current, &response) else {
                let error = refused(
                    &method,
                    &current,
                    &response,
                    None,
                    "no usable Location",
                    limit,
                );
                self.drain(response);
                return Err(error);
            };
            if hops >= self.inner.settings.max_redirects {
                self.drain(response);
                return Err(Error::Redirect(alloc::format!(
                    "more than {} redirects from {}",
                    self.inner.settings.max_redirects,
                    shown(url)
                )));
            }
            let status = response.status();
            self.drain(response);
            if status == 303 && method != Method::HEAD {
                method = Method::GET;
            }
            current = location;
            hops += 1;
        }
    }

    /// A request that must not be redirected the way a browser does: a redirect with a status in
    /// the method's resend list to the same scheme, host and port is sent once more with the same
    /// method and body, and any other 3xx is an error naming the status and the `Location`.
    fn send_once(
        &self,
        method: &Method,
        url: &Url,
        headers: &Headers,
        body: Body,
        timeout: Timeouts,
        clock: Option<&Deadline>,
    ) -> Result<Response, Error> {
        let again = body.replay();
        let response =
            self.state_send(method, url, headers, body, Self::capped(timeout, clock)?)?;
        if !(300..400).contains(&response.status()) {
            return Ok(response);
        }
        let limit = self.inner.settings.peer_text_limit;
        let location = redirect_location(url, &response);
        let resent = resent_statuses(method);
        let reason = if !resent.contains(&response.status()) {
            let list: Vec<String> = resent.iter().map(ToString::to_string).collect();
            alloc::format!(
                "only {} and {} send the request again",
                list[..list.len() - 1].join(", "),
                list[list.len() - 1]
            )
        } else if let Some(target) = location.as_ref() {
            if target.origin() != url.origin() {
                "it leaves the scheme, host and port of the request".to_string()
            } else if let Some(body) = again {
                self.drain(response);
                let second =
                    self.state_send(method, target, headers, body, Self::capped(timeout, clock)?)?;
                if !(300..400).contains(&second.status()) {
                    return Ok(second);
                }
                let next = redirect_location(target, &second);
                let error = refused(
                    method,
                    target,
                    &second,
                    next.as_ref(),
                    "a second redirect",
                    limit,
                );
                self.drain(second);
                return Err(error);
            } else {
                "the request body cannot be sent twice".to_string()
            }
        } else {
            "no usable Location".to_string()
        };
        let error = refused(method, url, &response, location.as_ref(), &reason, limit);
        self.drain(response);
        Err(error)
    }

    #[allow(clippy::too_many_arguments, reason = "the options of one request")]
    fn dispatch(
        &self,
        method: &Method,
        url: &Url,
        sent: &Headers,
        body: Body,
        redirects: Option<bool>,
        stream: bool,
        timeout: Timeouts,
        clock: Option<&Deadline>,
        first: Option<&Url>,
    ) -> Result<Response, Error> {
        check_headers(sent)?;
        let mut response = if is_followed(method) && matches!(body, Body::Empty) {
            if redirects == Some(false) && first.is_none() {
                self.state_send(
                    method,
                    url,
                    sent,
                    Body::Empty,
                    Self::capped(timeout, clock)?,
                )?
            } else {
                self.send_following(method, url, sent.clone(), timeout, first, clock)?
            }
        } else {
            self.send_once(method, url, sent, body, timeout, clock)?
        };
        if let Some(clock) = clock {
            response.bound(clock.clone());
        }
        if !stream {
            let limit = self.inner.settings.max_reply_bytes;
            response = response.buffered(Some(limit))?;
        }
        Ok(response)
    }

    fn run(&self, request: RequestBuilder) -> Result<Response, Error> {
        if let Some(error) = request.pending {
            return Err(error);
        }
        let url = check_url(&request.url)?;
        let timeout = request.timeout.unwrap_or(Timeouts {
            connect: self.inner.settings.connect_timeout,
            read: self.inner.settings.read_timeout,
        });
        if let Some(first) = &request.followed_from {
            // A redirect the caller follows: cookies apply to every hop; the token, hooks and
            // retries belong to `request`.
            let sent = self.sent_headers(&request.headers, request.auth.as_ref());
            let clock = self.clock(request.deadline, &request.method, &url)?;
            return self.dispatch(
                &request.method,
                &url,
                &sent,
                Body::Empty,
                None,
                request.stream,
                timeout,
                clock.as_ref(),
                Some(first),
            );
        }
        self.ensure(&url);
        let mut hooked = HookRequest {
            method: request.method,
            url,
            headers: request.headers,
            body: request.body,
            stream: request.stream,
            redirects: request.redirects,
            timeout: request.timeout,
            deadline: request.deadline,
            auth: request.auth,
        };
        if !self.guarded(false) && !self.inner.hooks.is_empty() {
            self.set_guard(false, true);
            let outcome = self.run_hooks(&mut hooked);
            self.set_guard(false, false);
            if let Some(response) = outcome? {
                return Ok(response);
            }
        }
        self.send_request(hooked)
    }

    fn run_hooks(&self, request: &mut HookRequest) -> Result<Option<Response>, Error> {
        let asked = request.url.to_string();
        for hook in &self.inner.hooks {
            if !hook.pattern.is_match(&asked) {
                continue;
            }
            match (hook.run)(self, request) {
                Ok(Some(response)) => return Ok(Some(response)),
                Ok(None) => {}
                Err(source) => {
                    return Err(Error::Hook {
                        key: hook.key.clone(),
                        request: alloc::format!("{} {}", request.method, shown(&request.url)),
                        source,
                    });
                }
            }
        }
        Ok(None)
    }

    /// One request with its token, retries and deadline.
    fn send_request(&self, request: HookRequest) -> Result<Response, Error> {
        let HookRequest {
            method,
            url,
            headers,
            body,
            stream,
            redirects,
            timeout,
            deadline,
            mut auth,
        } = request;
        if auth.is_none()
            && let Some(tokens) = &self.inner.tokens
            && let Some(token) = tokens.authorization(&url)?
        {
            auth = Some(Auth::Header(token));
        }
        let timeout = timeout.unwrap_or(Timeouts {
            connect: self.inner.settings.connect_timeout,
            read: self.inner.settings.read_timeout,
        });
        let sent = self.sent_headers(&headers, auth.as_ref());
        let clock = self.clock(deadline, &method, &url)?;
        let retry = &self.inner.settings.retry;
        let replayable = body.is_replayable();
        let retried = retry.count > 0 && retry.methods.contains(&method) && replayable;
        let mut body = Some(body);
        let mut attempt = 0_u32;
        loop {
            let this = if replayable {
                body.as_ref().and_then(Body::replay).unwrap_or_default()
            } else {
                body.take().unwrap_or_default()
            };
            let reply_headers = match self.dispatch(
                &method,
                &url,
                &sent,
                this,
                redirects,
                stream,
                timeout,
                clock.as_ref(),
                None,
            ) {
                Err(error) if error.is_connection_failure() => {
                    if !retried || attempt >= retry.count {
                        return Err(error);
                    }
                    None
                }
                Err(error) => return Err(error),
                Ok(response) => {
                    if !(retried
                        && attempt < retry.count
                        && retry.statuses.contains(&response.status()))
                    {
                        return Ok(response);
                    }
                    let headers = response.headers().clone();
                    self.drain(response);
                    Some(headers)
                }
            };
            attempt += 1;
            let mut wait = retry::wait(
                attempt,
                retry.backoff,
                retry.backoff_max,
                reply_headers.as_ref(),
                SystemTime::now(),
            );
            if let Some(clock) = &clock {
                wait = wait.min(clock.check()?);
            }
            if !wait.is_zero() {
                std::thread::sleep(wait);
            }
        }
    }

    /// `Authorization` as a token jar gives it, for the origin of `url`; for callers that send
    /// requests of their own.
    ///
    /// # Errors
    ///
    /// [`Error::Configuration`] naming the key for a token with a CR, LF or NUL.
    pub fn token_for(&self, url: &Url) -> Result<Option<Secret>, Error> {
        match &self.inner.tokens {
            Some(tokens) => tokens.authorization(url),
            None => Ok(None),
        }
    }

    /// Every cookie the client keeps, as a snapshot.
    #[must_use]
    pub fn cookies(&self) -> Vec<super::cookies::Cookie> {
        self.inner.cookies.all()
    }
}
