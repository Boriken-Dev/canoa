# `coqui-proxy` — API header

Header-file-style reference for the `coqui-proxy` crate: every public item with its signature,
contract and gotchas, so the crate can be used without reading its source. It ships inside the
package and is self-contained. Development documentation lives with the source at
<https://github.com/Boriken-Dev/coqui-proxy>.

MSRV Rust 1.89, edition 2024. A default build depends on `http`, `percent-encoding`, `base64`,
`thiserror`, `tracing`, `if-addrs`, `coqui-net-acl` and, on Windows, `windows-sys`.

| Feature | Adds | Needed for |
| --- | --- | --- |
| `pac` | `boa_engine`, `boa_gc`, `gethostname` | `pac::*`; PAC and WPAD off Windows |
| `tls` | `rustls` (ring), `rustls-platform-verifier` | `https:` proxies and PAC locations; `ProxyConnectorBuilder::proxy_tls`, `PacSettings::tls` |
| `negotiate` | Unix: `libgssapi` | Negotiate and NTLM login (Windows SSPI needs no crate) |
| `libproxy` | Unix: `libloading` | `system::LibProxySource` |

Imports: the root re-exports `Resolver`, `ResolverBuilder`, `ProxySetting`, `OnMissingEngine`,
`ChildEnvironment`, `Proxy`, `ProxyBuilder`, `ProxyScheme`, `Route`, `Credentials`, `Secret`,
`BypassRules`, `Dialect`, `Environment`, `ProxyVariables`, `EnvSource`, `EnvSourceBuilder`,
`ProxyConnector`, `ProxyConnectorBuilder`, `Connection`, `Negotiate`, `NegotiateBuilder`,
`OriginAuth`, `OriginAuthBuilder`, `OriginLogin`, `ProxySource`, `ChainSource`, `Error`, `Reason`,
`UnsupportedProxy`, `ConnectorSettings`, `PacSettings`, `ReaderSettings`, `ResolverSettings`,
`VERSION`. Public modules: `auth`, `connect` (`core`, `socks`), `pac`, `settings`, `source`,
`system`.

`unsafe` is in `src/auth/sspi.rs` (SSPI with the logon session), `src/system/winhttp.rs`
(`WinHttpGetProxyForUrl`, Internet Options), `src/system/libproxy.rs` (libproxy opened at run time)
and `src/system/cfnetwork.rs` (CFNetwork lookup and PAC run). No maintained safe crate covers
them; the rest of the crate is `#![deny(unsafe_code)]`. `tests/guards.rs` keeps the list.

## Resolver (`coqui_proxy::Resolver`)

```rust
Resolver::system() -> Resolver
Resolver::builder() -> ResolverBuilder
Resolver::lookup(&self, url: &str) -> Result<Vec<Route>, Error>      // never empty
Resolver::report_failure(&self, proxy: &Proxy)                       // tried last for failure_memory
Resolver::clear(&self)                                               // failures, answers, PAC scripts
Resolver::scheme_proxies(&self, url: &str) -> Result<BTreeMap<String, Proxy>, Error>
Resolver::child_environment<S: AsRef<str>>(&self, endpoints: impl IntoIterator<Item = S>) -> Result<ChildEnvironment, Error>
Resolver::settings(&self) -> &ResolverSettings
Resolver::sources(&self) -> &[Arc<dyn ProxySource>]
Resolver::no_proxy(&self) -> &BypassRules
Resolver::cache_len(&self) -> usize
Resolver::non_defaults(&self) -> Vec<String>                         // what is looser than the defaults
```

A resolver is cheap to clone and shared between threads; it reads nothing until the first lookup.
The order is: bypass list, explicit `ProxySetting`, added sources, proxy variables (they alone
decide when any is set), the system, direct. A proxy that refused a connection moves to the end of
an answer for `failure_memory`; with `rotate` each lookup of the same answer starts at the next
proxy; with `probe` an unreachable proxy is skipped and `Error::NoReachableProxy` is returned when
none is reachable.

`ResolverBuilder`: `environment(Environment)` (default: the process's), `proxy(ProxySetting)`,
`no_proxy(BypassRules)`, `source(impl ProxySource + 'static)`, `system(bool)` (true),
`settings(SystemSettings)` (instead of reading the OS), `wpad(bool)` (false), `credentials(Credentials)`
(for every HTTP proxy without a login), `timeout(Duration)` (10 s), `on_missing_engine(OnMissingEngine)`,
`resolver_settings(ResolverSettings)` (all bounds at once), `non_defaults()`, `build()`.

`enum ProxySetting { Proxy(Proxy), Routes(Vec<Route>), Pac(String), Direct }` (non_exhaustive):
`FromStr` reads a proxy URL, a gateway, a PAC result, a PAC location or script text, `direct`;
`routes() -> Option<Vec<Route>>` (`None` for `Pac`); a `ProxySource`; `From<Proxy>`.
`enum OnMissingEngine { Error (default), Direct }`.

`ChildEnvironment`: `pairs()`, `http_proxy()`, `https_proxy()`, `socks_proxy()`, `no_proxy()`,
`per_endpoint()`, `unexpressed()`.

## Proxies and routes

```rust
Proxy::parse(text: &str) -> Result<Proxy, Error>       // [scheme://][user[:password]@]host[:port]; FromStr
Proxy::for_url(url: &str) -> Result<Option<Proxy>, Error>   // a Resolver::system() per call; None = direct
Proxy::builder(scheme: ProxyScheme, host: impl Into<String>, port: u16) -> ProxyBuilder
Proxy::{scheme, host, port, authority, credentials, headers, url_with_credentials}
Proxy::{is_gateway, path, gateway_url(&self, target: &str) -> Option<String>}
ProxyBuilder::{credentials(Credentials), header(name, impl Into<Secret>), path(String), build() -> Result<Proxy, Error>}
enum ProxyScheme { Http, Https, Socks4, Socks5, Gateway { tls: bool } }   // non_exhaustive; default_port(), as_str()
enum Route { Direct, Proxy(Proxy) }                        // non_exhaustive
Route::parse_pac_result(text: &str) -> Vec<Route>          // PROXY, HTTP, HTTPS, SOCKS, SOCKS4, SOCKS5, DIRECT, PREFIX <url>
Credentials::new(user, password); .username(); .password()
Secret::new(value); .expose()                              // prints as ***
```

`Display` of a `Proxy` is `scheme://host:port` plus a gateway's path, never a credential; `Debug`
masks the password and shows header names only. A URL-prefix gateway (`prefix+http`,
`prefix+https`, path default `/`) is asked for `gateway_url(target)`; connectors answer
`Error::Unsupported` for it and an HTTP client adapter skips it to the next route. A path on any
other proxy is an error. An unusable entry of a PAC result is dropped with a `tracing` warning;
nothing usable left is `[Route::Direct]`.

## Bypass rules (`BypassRules`, `Dialect`)

```rust
BypassRules::parse(text: &str, dialect: Dialect) -> BypassRules
BypassRules::from_entries<'a>(entries: impl IntoIterator<Item = &'a str>, dialect: Dialect) -> BypassRules
BypassRules::none() -> BypassRules              // only loopback bypasses
BypassRules::matches(&self, url: &str) -> bool
BypassRules::{entries, skipped, is_empty, loopback_proxied}
BypassRules::with_dns_timeout(Duration) / with_interface_cache_ttl(Duration)   // for <subnet> (5 s, 10 s)
impl BitOr for BypassRules                      // bypasses what either bypasses
enum Dialect { NoProxy, WinInet, MacOs, GSettings, Kde, KdeReversed }
```

Entries: host, `*.suffix`, glob, CIDR, `host:port`, `<local>`, `<subnet>`, `<-loopback>` (switches
the implicit loopback bypass off), `*` alone (everything). Loopback always bypasses unless
`<-loopback>`. An entry that cannot be read is skipped, listed in `skipped()` and logged.

## Environment (`Environment`, `ProxyVariables`, `EnvSource`)

`Environment::process()`, `Environment::from_pairs(pairs)` (a snapshot; `from_pairs([("X","y")])`
is "no proxy variables"), `get(name)`, `proxy_variables() -> Option<ProxyVariables>`.
Lowercase variables win; with `REQUEST_METHOD` set an uppercase `HTTP_PROXY` is ignored
(httpoxy); `ws` and `wss` fall back to `http` and `https`, then `ALL_PROXY`. `ProxyVariables`:
`no_proxy() -> BypassRules`, `has_proxies()`.

`EnvSource::builder().environment(..).proxies(BTreeMap<String, ProxySetting>).no_proxy(..).build()`:
the variables as a `ProxySource`; `proxies()`, `no_proxy()`. A PAC location gives no opinion there.

## Sources (`coqui_proxy::source`)

```rust
trait ProxySource: Send + Sync + Debug { fn routes(&self, url: &str) -> Result<Option<Vec<Route>>, Error>; }   // None = no opinion
source::lookup(source: &dyn ProxySource, url: &str) -> Result<Vec<Route>, Error>   // Direct when no opinion
ChainSource::new(impl IntoIterator<Item = Box<dyn ProxySource>>); .sources()
```

Implemented by `ProxySetting`, `EnvSource`, `ChainSource`, `system::SystemSource`,
`system::WinHttpSource` (Windows), `system::CfNetworkSource` (macOS), `system::LibProxySource`
(feature `libproxy`), `pac::PacSource`.

## System settings (`coqui_proxy::system`)

```rust
SystemSettings::read() -> SystemSettings
SystemSettings::read_with(&ReaderSettings) -> SystemSettings
SystemSettings::builder(source: SettingsSource, state: SettingsState) -> SystemSettingsBuilder
SystemSettings::to_source(&self, wpad: bool, timeout: Duration, on_missing_engine: OnMissingEngine) -> SystemSource
SystemSettingsBuilder::{auto_detect(bool), pac_url, pac_script, proxy(scheme, Proxy), bypass(BypassRules), build()}
enum SettingsSource { None, Windows, MacOs, Gnome, Mate, Kde, NetworkManager }
enum SettingsState { Direct, Configured, Unset }
```

Public fields of `SystemSettings`: `source`, `state`, `auto_detect`, `pac_url`, `pac_script`,
`proxies`, `bypass`. Readers: Windows Internet Options and WinHTTP; macOS `scutil --proxy`; on
other Unix `gsettings` (GNOME, MATE), KDE's `kioslaverc` (`XDG_CONFIG_HOME`, `HOME`), then
NetworkManager through `nmcli`, else `dbus-send`; the first desktop with a proxy configured wins.
Helper programs run in the C locale with no stdin and are ended after
`ReaderSettings::command_timeout`.

`WinHttpSource::new(timeout: Duration) -> Option<WinHttpSource>` (Windows),
`CfNetworkSource::{new(), with_deadline(Duration), deadline()}` (macOS; a PAC script runs on a
private run-loop thread bounded by the deadline),
`LibProxySource::load(path: Option<impl AsRef<Path>>) -> Result<LibProxySource, Error>`
(`libproxy.so.1` by default; `DEFAULT_LIBRARY`).

## PAC (`coqui_proxy::pac`, feature `pac`)

```rust
PacScript::builder(source: impl Into<String>) -> PacScriptBuilder
PacScriptBuilder::{timeout(Duration), settings(&PacSettings), function(name, impl Fn(&[String]) -> Result<String, Error> + Send + Sync + 'static), build()}
PacScript::from_location(location: &str, &PacSettings)     // http(s) URL, file: URL or path
PacScript::from_file(path, PacEncoding, &PacSettings); PacScript::from_url(url, &PacSettings)
PacScript::{find_proxy_for_url(url, host) -> Result<String, Error>, routes(url) -> Result<Vec<Route>, Error>, source(), timeout()}
trait FindProxy: Send + Sync { fn find_proxy_for_url(&self, url: &str, host: &str) -> Result<String, Error>; }   // PacScript and Fn(&str, &str) -> Result<String, Error>
PacSource::new(impl FindProxy + 'static); .with_browser_compatibility(bool); .browser_compatibility(); .script()
pac::discover(fqdn: Option<&str>, &PacSettings) -> Result<Option<PacScript>, Error>
pac::builtin_names() -> &'static [&'static str]
enum PacEncoding { Utf8 (default; BOM dropped), Latin1 }
pac::DEFAULT_MAX_PAC_BYTES
```

A script runs on a thread of its own; a call past `call_timeout` is abandoned and the script is
refused for `refusal_after_overrun`, then restarted. Downloads are direct, never through a proxy.
The builtins are those of the PAC specification and Microsoft's `*Ex` set; the name-lookup cache
belongs to the script. A `Resolver` downloads a script again after `download_cache_ttl` and
discovers WPAD again after `wpad_cache_ttl`.

## Connecting (`ProxyConnector`)

```rust
ProxyConnector::default(); ProxyConnector::builder() -> ProxyConnectorBuilder
ProxyConnectorBuilder::{timeout(Duration), user_agent(String), negotiate(Negotiate), settings(ConnectorSettings), proxy_tls(Arc<rustls::ClientConfig>) /*tls*/, non_defaults(), build()}
ProxyConnector::connect(&self, route: &Route, host: &str, port: u16) -> Result<Connection, Error>
ProxyConnector::connect_url(&self, resolver: &Resolver, url: &str) -> Result<Connection, Error>
ProxyConnector::{with_timeout(Duration), non_defaults()}
enum Connection { Tcp(TcpStream), Tls(Box<StreamOwned<..>>) /*tls*/ }   // Read + Write; socket()
```

`connect_url` takes the first route that works and reports each proxy that does not
(`Resolver::report_failure`); a refused login stops the search. A plain `http:` origin is
tunnelled too. With `tls`, an `https:` proxy is verified by the platform verifier unless
`proxy_tls` gives a configuration.

The sans-I/O cores (`coqui_proxy::connect`): `core::{ConnectRequest, ConnectResponse, Progress<T>,
ProxyLogin, Outcome, BodyFraming, decode_chunk_size}` and `socks::{Greeting, Method, PasswordLogin,
Request, Version, Reply}`. Every `decode` takes the bytes read so far and returns
`Progress::Incomplete(n)`, `Progress::Complete { value, consumed }` or an error; none panics on any
input.

## Login (`coqui_proxy::auth`)

`Negotiate::builder().proxies(hosts).credentials(..).plain_http(bool).max_legs(n).max_token(n).build()`
(defaults 5 legs, 64 KiB; `plain_http(true)` allows a Negotiate login over plain `http:` and is
listed by `non_defaults()`); `auth::negotiate_available() -> bool`; `auth::basic_header(&Credentials)`.
`OriginAuth::builder().basic(hosts, creds).negotiate(hosts).negotiate_as(hosts, creds)
.plain_http(bool).max_legs(n).max_token(n).max_body(n).build()`; `OriginAuth::login(url) ->
Result<Option<OriginLogin>, Error>`; `OriginLogin::{answer(&[&str]) -> Result<Option<Answer>, Error>,
finish(&[&str])}`; `Answer::{authorization(), probe()}`. Basic over plain `http:` to a host that is
not loopback is refused unless `plain_http(true)`.

## Settings (`coqui_proxy::settings`)

Every bound is a field with a `DEFAULT_*` constant; each type has `Default` and is
`#[non_exhaustive]`, so build it from `Default` and assign fields.

| Type::field | Default |
| --- | --- |
| `ResolverSettings::timeout` | 10 s (PAC download and WPAD probe) |
| `::settings_refresh`, `::source_retry`, `::failure_memory` | 60 s, 30 s, 5 min |
| `::cache_ttl` (`Option`), `::cache_capacity` | 60 s, 1024 |
| `::rotate`, `::rotation_capacity` | false, 1024 |
| `::wpad`, `::system`, `::on_missing_engine` | false, true, `Error` |
| `::probe`, `::probe_timeout`, `::probe_failure_memory` | false, 5 s, 30 s |
| `::interface_cache_ttl`, `::bypass_dns_timeout` | 10 s, 5 s |
| `::reader`, `::pac` | `ReaderSettings`, `PacSettings` |
| `ReaderSettings::command_timeout` | 5 s |
| `::dhcp_lease_paths` | `/var/lib/dhcp`, `/var/lib/dhclient`, `/var/lib/NetworkManager` |
| `::gsettings_cache_ttl`, `::winhttp_timeout`, `::cfnetwork_deadline` | 10 s, 10 s, 5 s |
| `PacSettings::wpad_cache_ttl`, `::call_timeout`, `::refusal_after_overrun` | 300 s, 5 s, 30 s |
| `::loop_iterations`, `::recursion`, `::max_script_bytes` | 20 000 000, 400, 1 MiB |
| `::download_timeout`, `::max_redirects`, `::download_cache_ttl` | 10 s, 3, 300 s |
| `::dns_timeout`, `::dns_cache_ttl`, `::dns_cache_capacity` | 2 s, 30 s, 256 |
| `::wpad_probe_timeout`, `::browser_compatibility`, `::tls` | 3 s, false, system verifier |
| `ConnectorSettings::timeout`, `::user_agent`, `::happy_eyeballs_delay` | 30 s, `coqui-proxy`, 250 ms |
| `::max_response_head_bytes`, `::max_response_headers`, `::max_response_body_bytes` | 64 KiB, 100, 64 KiB |
| `Negotiate::max_legs`, `::max_token` | 5, 64 KiB |
| `OriginAuth::max_body` | 1 MiB |

A whole PAC download may take three times `download_timeout`.

## Errors

`Error` (non_exhaustive, `thiserror`): `Parse(&'static str)`, `Pac(Reason)`, `MissingEngine`
(`missing_feature() -> Option<&'static str>` names `pac`), `Settings(Reason)`, `NoReachableProxy`,
`Refused(Reason)` (a proxy answered a `CONNECT` with a refusal), `Login(Reason)`,
`Unsupported(Reason)`, `UnsupportedProxy(Box<UnsupportedProxy>)` (`client()`, `proxies()`,
`reasons()`), `Tls(Reason)` (a TLS configuration of an `https:` proxy or PAC location), `Io`.
`Reason::new(text)` makes a reason. A message names what failed and never quotes input: a proxy
URL, a PAC script or a credential may be a secret.

## Environment variables

Read by `Environment::process()`: `HTTP_PROXY`, `HTTPS_PROXY`, `FTP_PROXY`, `WS_PROXY`,
`WSS_PROXY`, `ALL_PROXY` (and lowercase), `NO_PROXY`, `REQUEST_METHOD` (httpoxy guard), and by the
system readers `XDG_CONFIG_HOME` and `HOME` (KDE's `kioslaverc`). Helper programs run with
`LC_ALL=C`. The library sets none.

## Gotchas

- PAC is run only with the `pac` feature (Windows asks WinHTTP and needs none); without it a PAC
  location is `Error::MissingEngine` unless `OnMissingEngine::Direct`.
- Anything looser than the defaults is listed by `non_defaults()`: `Negotiate::plain_http(true)`,
  `OriginAuth::plain_http(true)`, `<-loopback>`, `OnMissingEngine::Direct`.
- A gateway is not a tunnel: a `ProxyConnector` answers `Error::Unsupported` for it.
- The library prints nothing, exits never, and keeps no process-wide state.
- `Resolver::lookup` blocks (DNS, downloads, helper programs); async callers use
  `spawn_blocking`.
