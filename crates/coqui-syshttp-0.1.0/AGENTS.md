# `coqui-syshttp` — API header

Header-file-style reference for the `coqui-syshttp` crate: what it re-exports, where each item is
documented, and what it adds. It ships inside the package and is self-contained. Development
documentation lives with the source at <https://github.com/Boriken-Dev/coqui-syshttp>.

The crate holds no proxy or TLS logic of its own. Two crates do the work and are documented in their
own `AGENTS.md`:

- [coqui-proxy](https://github.com/Boriken-Dev/coqui-proxy): which proxy a request uses and a
  connection through it (resolver, PAC, WPAD, bypass rules, login, CONNECT and SOCKS connectors).
- [coqui-sslcontext](https://github.com/Boriken-Dev/coqui-sslcontext): rustls configurations that
  follow the system (trust, keystore client certificates, pins).

| Feature | Turns on | Needed for | Default |
| --- | --- | --- | --- |
| `pac` | `coqui-proxy/pac` | PAC scripts and WPAD on macOS and Unix | off |
| `negotiate` | `coqui-proxy/negotiate` | Negotiate and NTLM login (SSPI on Windows, GSSAPI on Unix) | off |
| `tls` | `coqui-sslcontext` with `system`, `coqui-proxy/tls` | the `tls` module; `https:` proxies and PAC locations | off |
| `pkcs11` | `tls`, `coqui-sslcontext/pkcs11` | client certificates on PKCS #11 tokens | off |
| `ureq` | `tls`, `ureq` 3.5, `rustls` | the `ureq` module and `agent()` | off |
| `hyper` | `tls`, `coqui-proxy/hyper`, `hyper`, `hyper-util`, `tokio`, `tokio-rustls`, `tower-service` | the `hyper` module: `client()` | off |
| `reqwest` | `tls`, `coqui-proxy/relay`, `reqwest` 0.13 (no TLS backend of its own, `socks`), `rustls`, `tracing`, `tokio` (the runtime handle) | the `reqwest` module: `client()` | off |
| `reqwest-blocking` | `reqwest`, `reqwest/blocking`, `coqui-proxy/relay-blocking` | the `reqwest::blocking` module: `client()` | off |
| `session` | `ureq`, `url`, `regex`, `flate2`, `encoding_rs`, `percent-encoding` | the `session` module | off |
| `json` | `session`, `serde`, `serde_json` | `Response::json`, `FetchBuilder::json` | off |
| `session-reqwest` | `session`, `reqwest`, `reqwest/cookies` | the session jars as `reqwest` cookie stores, `reqwest::builder_from` | off |

MSRV Rust 1.89, edition 2024. `#![forbid(unsafe_code)]`; no process-wide state; prints nothing.

## Re-exports (same names and paths as before the split)

From coqui-proxy at the root: `Resolver`, `ResolverBuilder`, `ChildEnvironment`, `ProxySetting`,
`OnMissingEngine`, `Route`, `Proxy`, `ProxyBuilder`, `ProxyScheme`, `Credentials`, `Secret`,
`BypassRules`, `Dialect`, `Environment`, `ProxyVariables`, `ProxyConnector`,
`ProxyConnectorBuilder`, `Connection`, `Negotiate`, `NegotiateBuilder`, `OriginAuth`,
`OriginAuthBuilder`, `OriginLogin`. Modules: `auth`, `system`, `pac` (feature `pac`), and
`proxy` (the whole crate). Feature `tls`: `tls` is the whole of coqui-sslcontext (`tls::Tls`,
`tls::TlsBuilder`, `tls::Trust`, `tls::Selector`, `tls::Identity`, ...).

## Root items of this crate

```rust
pub fn agent() -> Result<ureq::Agent, Error>;   // feature `ureq`; ureq::AgentBuilder::new().build()

#[non_exhaustive]
pub enum Error {
    Proxy(coqui_proxy::Error),            // From; source() is the inner error
    Tls(coqui_sslcontext::Error),         // feature `tls`; From
    Http(Reason),
}                                         // also From<std::io::Error> (as Proxy(Io))
pub struct Reason;                        // Display; Reason::new(text)
```

## Module `ureq` (feature `ureq`)

```rust
impl AgentBuilder {
    pub fn new() -> Self;
    pub fn resolver(self, resolver: Resolver) -> Self;        // default Resolver::system()
    pub fn tls(self, tls: coqui_sslcontext::Tls) -> Self;     // default: the system's
    pub fn connector(self, connector: ProxyConnector) -> Self;
    pub fn config(self, config: ureq::config::Config) -> Self; // its own proxy and TLS settings are unused
    pub fn failover(self, failover: bool) -> Self;            // default true: next route on a connect failure
    pub fn bad_proxy_ttl(self, ttl: Duration) -> Self;        // default 300 s: a failed proxy is tried last
    pub fn adapter_settings(self, settings: AdapterSettings) -> Self;
    pub fn read_wait(self, wait: Duration) -> Self;            // longest wait on a connection at each step, whatever ureq allows
    pub fn shared_tls(self, tls: Arc<Tls>) -> Self;            // one Tls for several agents (instead of tls)
    pub fn non_defaults(&self) -> Vec<String>;
    pub fn build(self) -> Result<ureq::Agent, Error>;
}
impl OriginAgent {                                            // answers an origin's 401
    pub fn new(agent: ureq::Agent, auth: OriginAuth) -> Self;
    pub fn agent(&self) -> &ureq::Agent;
    pub fn run(&self, request: ureq::http::Request<impl AsRef<[u8]>>)
        -> Result<ureq::http::Response<ureq::Body>, ureq::Error>;
}
```

The agent resolves no name itself: every request asks the resolver for its routes and tries them in
order. A plain `http:` URL through a proxy is tunnelled with `CONNECT`. A failed origin login is an
I/O error of kind `PermissionDenied`.

## Adapter settings (`coqui_syshttp::AdapterSettings`)

`AdapterSettings { failover: bool = true, bad_proxy_ttl: Duration = 300 s }` (non_exhaustive, `Default`,
`non_defaults()`), taken by the builders of all three adapters as `failover(bool)`, `bad_proxy_ttl(Duration)`
and `adapter_settings(AdapterSettings)`. Failover is only for connecting: once a request is sent, a failure is
the caller's.

## Module `hyper` (feature `hyper`)

```rust
pub fn client<B>() -> Result<Client<HttpsConnector, B>, Error>;   // hyper_util legacy client, Builder::new().build()
pub fn builder<B>() -> Builder<B>;
impl<B> Builder<B> {
    pub fn new() -> Self;
    pub fn resolver(self, resolver: Resolver) -> Self;            // default Resolver::system()
    pub fn tls(self, tls: coqui_sslcontext::Tls) -> Self;         // default: the system's, ALPN h2 and http/1.1
    pub fn connector(self, connector: ProxyConnector) -> Self;
    pub fn failover(self, bool) -> Self; pub fn bad_proxy_ttl(self, Duration) -> Self; pub fn adapter_settings(self, AdapterSettings) -> Self;
    pub fn configure(self, f: impl FnOnce(&mut hyper_util::client::legacy::Builder) + Send + 'static) -> Self;
    pub fn client(self, client: Client<HttpsConnector, B>) -> Self;   // build() returns a client you made
    pub fn connector_service(&self) -> Result<HttpsConnector, Error>; // the service alone, for your own client
    pub fn non_defaults(&self) -> Vec<String>;
    pub fn build(self) -> Result<Client<HttpsConnector, B>, Error>;  // B: hyper Body + Send + Unpin + 'static
}
HttpsConnector::new(ProxyConnector, Resolver, impl Into<Arc<Tls>>) -> Self; .resolver() -> &Resolver
// impl tower_service::Service<hyper::Uri> { Response = Stream (hyper Read + Write + Connection), Error = Error }
```

Every URI asks the resolver for its routes, connects through the first that works (coqui-proxy's
asynchronous connector: `CONNECT` or SOCKS, a plain `http:` URI too) and, for `https:`, speaks TLS over the
tunnel with `Tls::client_config(host)` (so the keystore's per-host certificate memory applies; the
configuration is made on `spawn_blocking` because a keystore may ask for a PIN). Needs a Tokio runtime.

## Module `reqwest` (feature `reqwest`)

```rust
pub fn client() -> Result<reqwest::Client, Error>;
pub fn builder() -> Builder;
impl Builder {
    pub fn new() -> Self;
    pub fn resolver(self, resolver: Resolver) -> Self;
    pub fn tls_config(self, config: Arc<rustls::ClientConfig>) -> Self;  // default coqui_sslcontext::client_config()
    pub fn connector(self, connector: ProxyConnector) -> Self;
    pub fn failover(self, bool) -> Self; pub fn bad_proxy_ttl(self, Duration) -> Self; pub fn adapter_settings(self, AdapterSettings) -> Self;
    pub fn relay_settings(self, RelaySettings) -> Self;                   // the relay's bounds (failover on)
    pub fn runtime(self, tokio::runtime::Handle) -> Self;                 // the runtime the relay runs on (failover on)
    pub fn configure(self, f: impl FnOnce(reqwest::ClientBuilder) -> reqwest::ClientBuilder + Send + 'static) -> Self;
    pub fn client_builder(self) -> Result<reqwest::ClientBuilder, Error>;  // proxy and TLS set; add more, then build
    pub fn build(self) -> Result<reqwest::Client, Error>;
    pub fn non_defaults(&self) -> Vec<String>;
    pub fn not_applied(&self) -> Vec<&'static str>;                       // what reqwest's own connector cannot do
}
```

`reqwest` makes its own connections and cannot retry a route (its connector layer is sealed), so its TLS is
`use_preconfigured_tls` (the system's verification, no host name, so no per-host certificate memory) and its
proxy is one of two things:

- **`failover` on (default): a relay.** `client_builder()` and `build()` start a `coqui_proxy::relay::Relay`
  (a loopback HTTP proxy on the caller's Tokio runtime, listening on `127.0.0.1` with a random port and a
  random per-relay credential) over the builder's `connector` (made asynchronous, with `failover` and
  `bad_proxy_ttl`) and `resolver`, and give `reqwest` that one proxy through `Proxy::custom(..).headers(..)`
  carrying the relay's token header (not `Proxy-Authorization`: `reqwest` drops that on a redirect to another
  host and the second hop would get `407`). Every request, `https:` (`CONNECT`) or `http:` (absolute form), therefore
  gets the connector's failover and failed-proxy memory, PAC, rules, SOCKS 4/4a/5, Negotiate and NTLM login,
  proxy TLS and reply limits, and a direct route is dialled by the relay. **The relay lives exactly as long as
  the client and all its clones** (the proxy closure `reqwest` keeps owns the `RelayHandle`); the builder may
  be dropped. Building **needs a Tokio runtime** for the relay: the one the building thread is in, or the one
  `runtime(handle)` names (`Error::Proxy(Io)` otherwise, its message names `start_on`; a relay on a given
  runtime stops serving when that runtime is shut down); `relay_settings` sets the
  bounds (connections, head time and size, idle time; the address must be loopback, else `Error::Proxy`).
  `not_applied()` then lists only per-host certificate memory, the session client's own features, URL-prefix
  gateways (skipped to the next route) and `Upgrade` requests over plain `http:`.
- **`failover(false)`: `Proxy::custom`** answering the first usable route of the resolver for each URL (HTTP,
  HTTPS, SOCKS 5 with the proxy resolving the name); SOCKS 4/4a, gateways and proxies with extra headers are
  skipped to the next route and, when none is usable, the request fails (it never goes direct past the proxy
  the system asked for). A route that fails to connect fails the request; Negotiate and NTLM proxy login and
  the connector's reply limits are not available, and `not_applied()` says so. No runtime is needed.

### Module `reqwest::blocking` (feature `reqwest-blocking`)

```rust
pub fn client() -> Result<reqwest::blocking::Client, Error>;
pub fn builder() -> Builder;   // reqwest::blocking::Builder
impl Builder {
    pub fn new() -> Self;
    pub fn resolver(self, Resolver) -> Self; pub fn tls_config(self, Arc<rustls::ClientConfig>) -> Self; pub fn connector(self, ProxyConnector) -> Self;
    pub fn failover(self, bool) -> Self; pub fn bad_proxy_ttl(self, Duration) -> Self; pub fn adapter_settings(self, AdapterSettings) -> Self;
    pub fn relay_settings(self, RelaySettings) -> Self;
    pub fn configure(self, f: impl FnOnce(reqwest::blocking::ClientBuilder) -> reqwest::blocking::ClientBuilder + Send + 'static) -> Self;
    pub fn client_builder(self) -> Result<reqwest::blocking::ClientBuilder, Error>;
    pub fn build(self) -> Result<reqwest::blocking::Client, Error>;
    pub fn non_defaults(&self) -> Vec<String>; pub fn not_applied(&self) -> Vec<&'static str>;
}
```

The async adapter's surface for `reqwest::blocking::Client`, without `runtime`: with `failover` on, `build()` starts
a `coqui_proxy::relay::blocking::Relay` (threads of its own, **no Tokio runtime needed**, the builder's `connector`
with `failover` and `bad_proxy_ttl`) and gives `reqwest` its loopback address and token header as the only proxy,
exactly as the async adapter does. The relay lives as long as the client and its clones (the `Proxy::custom`
closure owns the handle) and, when the last clone drops, closes its listener and joins its threads. `failover(false)`
is the plain `Proxy::custom` route. `reqwest::blocking` runs a Tokio runtime of its own on a thread of its own; the
relay adds none. Building a blocking client inside a Tokio runtime is `reqwest`'s restriction. `not_applied()` is
the async adapter's list.

## Errors

`Error` wraps the two crates' errors; a message names what failed and never quotes input. The `session` module has its
own `session::Error` (`Configuration { variable, reason }`, `Hook`, `Response`, `InvalidRequest`, `TooLarge`, `Redirect`,
`Io`, `Timeout`, `Tls`, `Proxy`), with `is_connection_failure()` and `is_timeout()`.

## Module `session` (feature `session`)

A client with memory above `ureq`, a port of the Python library `syshttp` 0.1.0: the same settings,
the same environment variable names (`SYSHTTP_*`, below) and the same file formats, so a Python and a
Rust program in one environment share them. Secrets (`Secret`): token and cookie values, never in
`Debug`, `Display`, an error or a log; a token with CR, LF or NUL is `Error::Configuration` naming the key.

```rust
// session::{Client, ClientBuilder, RequestBuilder, HookRequest}
Client::builder() -> ClientBuilder
ClientBuilder::{settings(SessionSettings), header(name, Option<value>), user_agent, timeout(connect, read),
    deadline(Option<Duration>), env(HttpEnv), resolver, tls(Tls), proxy_tls(Arc<ClientConfig>), trust(Trust),
    connector, adapter_settings, origin_auth(OriginAuth), transport(impl Transport), cookies(CookieStore),
    tokens(TokenStore), cookie_hosts(groups), auth_provider(Arc<dyn AuthProvider>),
    hook(Regex, Fn(&Client, &mut HookRequest) -> Result<Option<Response>, BoxError>), build() -> Result<Client, Error>}
Client::{request(Method, url) -> RequestBuilder, follow(Method, url, &Response) -> RequestBuilder, settings(), transport(),
    close(), cookies() -> Vec<Cookie>, token_for(&Url)}                 // Clone; clones share state
RequestBuilder::{body(impl Into<Body>), header(name, Option<value>), stream(bool), redirects(bool),
    timeout(connect, read), deadline(Option<Duration>), auth(Auth), send() -> Result<Response, Error>}
session::fetch(url) -> FetchBuilder   // method, header, body, form, json (feature json), timeout, client, transport,
                                      // client_options, reauth(Arc<dyn AuthProvider>), send(); the body is read whole
Auth::{Basic(Credentials), Header(Secret)}      trait AuthProvider { fn ensure(&self, &Client, &Url) -> Result<(), BoxError> }
Body::{Empty, Bytes, Text, Reader, Chunks}      Timeouts { connect, read }      Headers (case-blind, ordered)
Response: status(), reason(), headers(), url(), impl Read, chunks(n), content(), text(), text_with(enc, TextErrors),
    json::<T>() (json), buffered(limit), read_limited(what, limit), json_limited(what, limit), discard(limit),
    error_for_status() -> Result<Response, StatusError>, is_redirect(), close()
ResponseBuilder::{new(status), reason, headers, url, body, decode, max_reply_bytes, decode_chunk, build()}
trait Transport { send(PreparedRequest, stream: bool) -> Result<Response, Error>; close() }
PreparedRequest::prepare(method, url, headers, body, timeout) -> Result<PreparedRequest, Error>
UreqTransport::{builder(), new()}  UreqTransportBuilder::{env, resolver, tls, trust, proxy_tls, connector, adapter_settings,
    pool, tls_max_age, max_reply_bytes, decode_chunk, origin_auth, settings, not_applied, build}
session::{peer_text(text, limit), split_userinfo(url), retry_after(&Headers, now)}
```

Behaviour: every request and redirect hop asks the resolver again (a pooled connection is keyed by its route);
a URL-prefix gateway (`SYSHTTP_PROXY_TYPE=prefix[:/endpoint]`) is asked for `<gateway><path><URL>`, a plain-http
gateway never an `https:` origin. `GET`/`HEAD`/`OPTIONS` follow redirects (`303` becomes `GET`); other methods are
sent once more on `307`/`308` (`PROPFIND` also `301`-`303`) to the same origin, any other 3xx is `Error::Redirect`;
crossing scheme, host or port drops `Authorization`, `Cookie` and `Proxy-Authorization` for that hop and later ones.
A status in `RetrySettings::statuses` or a failed connection (not a timeout, not `NoReachableProxy`) is retried for a
method in `RetrySettings::methods` with a body that can be sent twice: the first retry at once, then `backoff` doubled,
never less than `Retry-After`, at most `backoff_max`; the deadline bounds the sum; a status that outlasts the
retries is returned. `gzip` and `deflate` are decoded unless the caller set `Accept-Encoding`, in steps of
`decode_chunk`; a body shorter than its `Content-Length` is an error; over `max_reply_bytes` is `Error::TooLarge`.

`SessionSettings` (build from `Default`, every bound a field): `connect_timeout` 10 s, `read_timeout` 60 s (each wait,
`None` unlimited), `deadline` none, `user_agent` `coqui-syshttp/<version>`, `headers` `Accept: */*`, `max_redirects` 30,
`retry` {`count` 3, `backoff` 1 s, `backoff_max` 120 s, `statuses` 408 429 500 502 503 504, `methods` GET HEAD OPTIONS
PUT DELETE TRACE PROPFIND}, `max_reply_bytes` 8 MiB, `decode_chunk` 256 KiB, `max_interim_replies` 8, `pool` {`max_idle` 8,
`max_idle_per_route` 1, `reuse_window` 1 s, `idle_timeout` 15 s}, `cookies` {`store` `Memory`|`PerHost`|`Key`|`Jar`, `hosts`,
`max_age` 400 d, `max_per_host` 180, `max_bytes` 4096}, `tokens.store` `None`|`PerHost`|`Key`|`Jar`, `tls_max_age` 60 s,
`system_proxy_max_age` 60 s, `peer_text_limit` 120.

Sub-modules: `session::env` (`HttpEnv::{process, builder, prefixes, name, find, get, family, environment,
system_proxy_enabled, proxy_auth_header, proxy_type, configured_proxy, check_proxy_settings, resolver(ProxyOverrides),
resolver_with}` and the setting-name constants), `session::cookies` (`Cookie`, `CookieLike`, `HostAliases`,
`parse_set_cookie`, `SetCookieContext`, `domain_covers`, `load_netscape`, `save_netscape`), `session::jar`
(`CookieJar`, `TokenJar`, `MemoryCookieJar`, `MemoryTokenJar`, `FileCookieJar`, `FileTokenJar`, `store_root`,
`default_root`, `safe_name`, `HOME`), `session::hooks` (`Hook`, `HookKind`, `from_env`, `matching`, `skipped`,
`split_command`, `curl_command`, `wrapper_environment`, the `SYSHTTP_HOOK*` constants).

Not ported from Python: import-time patching (`autopatch`), and the `_PY` / `SYSHTTP_DEFAULT_TRANSPORT` targets that
name Python code (`ClientBuilder::hook` and `ClientBuilder::transport` replace them). A curl front end is not in this
crate; `session::hooks` ports its data. `ureq` sends the standard methods only (a `PROPFIND` needs a transport of your
own) and skips interim replies itself (`UreqTransportBuilder::not_applied()`). `Response::content()` is the Python
`content` (`bytes` would collide with `std::io::Read::bytes`). With feature `session-reqwest`, `MemoryCookieJar` and
`FileCookieJar` implement `reqwest::cookie::CookieStore` over the same files and `reqwest::builder_from(&SessionSettings)`
maps what `reqwest` can honour; `reqwest` asks its `Proxy::custom` for every request it sends, each redirect hop included.

## Environment variables

The `session` module reads these through `HttpEnv`, each name as `<PREFIX><NAME>` for every prefix of
`SYSHTTP_PREFIX` (a `:`-separated list, taken literally and upper-cased; blank or unset is one empty
prefix), the first one set winning. The `SYSHTTP_` names are Python syshttp's, kept unchanged so a
Python and a Rust program share them. A malformed value is `Error::Configuration` naming the variable,
never quoting the value. Without feature `session` this crate reads none; coqui-proxy reads the proxy
variables (`*_PROXY`, `ALL_PROXY`, `NO_PROXY`, `REQUEST_METHOD`, `XDG_CONFIG_HOME`, `HOME`).

| Variable | Meaning | Default |
| --- | --- | --- |
| `SYSHTTP_PREFIX` | prefixes of every other name (read as written, no prefix of its own) | none |
| `SYSHTTP_PROXY` | a proxy for this crate's traffic, before the machine's | unset |
| `SYSHTTP_PROXY_AUTH` | its credential, a header value sent verbatim | the URL's userinfo as Basic |
| `SYSHTTP_PROXY_AUTH_HEADER` | the header that credential rides in | `Proxy-Authorization` |
| `SYSHTTP_PROXY_TYPE` | `connect`, or `prefix[:/endpoint]` for a URL-prefix gateway | `connect` |
| `SYSHTTP_SYSTEM_PROXY` | whether the system's proxy configuration is consulted (`1/true/yes/on`, `0/false/no/off`) | on |
| `HTTP_PROXY_AUTH` | the machine proxy's `Proxy-Authorization` value | unset |
| `HTTP_PROXY`, `HTTPS_PROXY`, `FTP_PROXY`, `WS_PROXY`, `WSS_PROXY`, `ALL_PROXY`, `NO_PROXY` | the conventional ones (lower case first under each prefix); when any proxy variable is set they alone decide | unset |
| `CA_BUNDLE` | a PEM file that replaces the system's trust for origins (`CURL_CA_BUNDLE` and `REQUESTS_CA_BUNDLE` are not read) | system trust |
| `SYSHTTP_HOME` | the jar root: cookies under `cookies/<host>.txt`, tokens under `tokens/<host>.token` | `%LOCALAPPDATA%\syshttp`, `~/Library/Application Support/syshttp`, `$XDG_STATE_HOME/syshttp` or `~/.local/state/syshttp` |
| `LOCALAPPDATA`, `XDG_STATE_HOME` | the platform's state directory, read unprefixed | platform |
| `SYSHTTP_COOKIE_HOSTS` | hosts that share cookies: hosts split by `,`, groups by `;` | none |
| `SYSHTTP_HOOKS_<KEY>` | a hook's URL pattern (a regular expression searched in the URL) | none |
| `SYSHTTP_HOOKS_<KEY>_CURL` | the wrapper a curl front end runs for it | none |
| `SYSHTTP_HOOK_SKIP` | KEYs not to run (comma list) | none |
| `SYSHTTP_HOOK_URL`, `SYSHTTP_HOOK_KEY`, `SYSHTTP_HOOK_CURL` | given to a wrapper (`hooks::wrapper_environment`) | not read |

`SYSHTTP_IMPORTPATCH`, `SYSHTTP_DEFAULT_TRANSPORT` and `SYSHTTP_HOOKS_<KEY>_PY` name Python code and are not read.

## Gotchas

- `ureq` 3 middleware cannot send a request twice, so an origin login is `OriginAgent::run`, not a
  middleware. A body-less POST needs an empty `Vec<u8>` body: `()` makes `ureq` send it chunked.
- `tls::Error` and `proxy::Error` are different types; `Error` converts both with `?`.
