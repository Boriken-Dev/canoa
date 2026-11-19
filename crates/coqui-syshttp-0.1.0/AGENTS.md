# `coqui-syshttp` — API header

Header-file-style reference for the `coqui-syshttp` crate: every public item with its
signature, contract and gotchas, so the crate can be used without reading its source. It
ships inside the package and is self-contained. Development documentation lives with the
source at <https://github.com/Boriken-Dev/coqui-syshttp>.

| Feature | Adds | Needed for | Default |
| --- | --- | --- | --- |
| `pac` | `boa_engine`, `gethostname` | `pac::PacScript`; PAC and WPAD on macOS and Unix | off |
| `tls` | `rustls` (ring), `rustls-platform-verifier`, `x509-cert`, `sha2`, `serde_json`; Windows: `rustls-cng`, `windows-registry`; macOS: `security-framework` | the `tls` module, `https:` proxies, `https:` PAC locations | off |
| `pkcs11` | `cryptoki`, and `tls` | `tls::Pkcs11Source` | off |
| `negotiate` | Unix: `libgssapi` (needs the system GSSAPI library and libclang to build) | Negotiate and NTLM login to a proxy or an origin server; Windows uses SSPI with no extra crate | off |
| `ureq` | `ureq` 3, and `tls` | the `client` module | off |

MSRV Rust 1.89, edition 2024. A default build depends on `http`, `percent-encoding`, `base64`,
`thiserror`, `tracing`, `if-addrs`, `coqui-net-acl` and, on Windows, `windows-sys`. `coqui-net-acl` is a git
dependency pinned by revision.

Root re-exports: `Resolver`, `ResolverBuilder`, `ChildEnvironment`, `ProxySetting`, `OnMissingEngine`, `Route`,
`Proxy`, `ProxyBuilder`, `ProxyScheme`, `Credentials`, `Secret`, `BypassRules`, `Dialect`,
`Environment`, `ProxyVariables`, `ProxyConnector`, `ProxyConnectorBuilder`, `Connection`,
`Negotiate`, `NegotiateBuilder`, `OriginAuth`, `OriginAuthBuilder`, `OriginLogin`, `Error`, `Reason`. Public modules: `auth`, `system`, and, behind
their features, `pac`, `tls`, `client`.

`unsafe` is confined to `src/system/winhttp.rs` (WinHTTP) and `src/auth/sspi.rs` (SSPI), both
Windows only, each block with a `// SAFETY:` comment; everything else is
`#![deny(unsafe_code)]`. The library prints nothing, installs no logger, changes no environment
variable and keeps one piece of process-wide state: the DNS cache of the thread that runs a PAC
script. It emits `tracing` events at `debug`, and at `warn` for a keystore that cannot be listed.

## Resolver (`coqui_syshttp`)

```rust
impl Resolver {
    pub fn system() -> Self;                                  // Resolver::builder().build()
    pub fn builder() -> ResolverBuilder;
    pub fn lookup(&self, url: &str) -> Result<Vec<Route>, Error>;
    pub fn report_failure(&self, proxy: &Proxy);
    pub fn clear(&self);
}
impl ResolverBuilder {
    pub fn environment(self, Environment) -> Self;
    pub fn proxy(self, ProxySetting) -> Self;
    pub fn no_proxy(self, BypassRules) -> Self;
    pub fn system(self, bool) -> Self;                        // default true
    pub fn settings(self, SystemSettings) -> Self;
    pub fn wpad(self, bool) -> Self;                          // default false
    pub fn credentials(self, Credentials) -> Self;
    pub fn timeout(self, Duration) -> Self;                   // default 10 s
    pub fn on_missing_engine(self, OnMissingEngine) -> Self;  // default Error
    pub fn build(self) -> Resolver;
}
pub enum ProxySetting { Proxy(Proxy), Pac(String), Direct }   // non_exhaustive
pub enum OnMissingEngine { Error, Direct }                    // non_exhaustive
```

`Resolver` is `Clone`, `Send` and `Sync`; it reads nothing until the first `lookup`. `lookup`
returns the routes to try in order and never an empty list. The order of deciding, the same on
every operating system:

1. `NO_PROXY` (from the environment, or the list given to `no_proxy`, which replaces it): those
   hosts are `[Route::Direct]`. Loopback always is, unless the list holds `<-loopback>`.
2. `proxy(...)`, when given, decides alone (`ProxySetting::Pac` is a PAC URL, a `file:` URL or a path).
3. When the environment holds any non-empty `<scheme>_proxy` or `all_proxy`, the variables
   alone decide, per scheme (`ws` falls back to `http`, `wss` to `https`, then `all`); a scheme
   with no variable is direct. The lowercase name wins over its uppercase form; inside a CGI
   request (`REQUEST_METHOD` set) an uppercase `HTTP_PROXY` is ignored. A value is a proxy URL,
   a PAC result (`PROXY a:3128; SOCKS5 b:1080; DIRECT`), a PAC location (`file:` or an `http(s)`
   URL with a path) or PAC script text (anything containing `FindProxyForURL`).
4. With `system(true)` and no environment proxy: on Windows WinHTTP answers (WPAD, PAC and the
   user's NTLM or Kerberos login for the PAC download included), falling back to the manual
   proxies of Internet Options; elsewhere `SystemSettings`: WPAD when `auto_detect` is set (or
   `wpad(true)` and the system has no settings), then `pac_url`, then `pac_script`, then
   the manual proxies with the bypass list. A source that fails passes to the next and is tried
   again after 30 s. A PAC that is configured but cannot run without the `pac` feature is
   `Error::MissingEngine` (or direct with `OnMissingEngine::Direct`). Settings are read again
   every 60 s and each URL's answer is cached as long.
5. Direct.

`credentials` goes to every HTTP or HTTPS proxy in an answer that has none of its own (never to
SOCKS). `report_failure` moves that proxy to the end of later answers for 5 minutes.
`environment(...)` takes a snapshot: the default is `Environment::process()` taken when the
resolver is built.

```rust
pub enum Route { Direct, Proxy(Proxy) }                       // non_exhaustive
impl Route { pub fn parse_pac_result(text: &str) -> Vec<Route>; }
```

`parse_pac_result` reads `DIRECT`, `PROXY`, `HTTP`, `HTTPS`, `SOCKS` (SOCKS 4), `SOCKS4`, `SOCKS5`,
case-blind; an entry with an unknown keyword or no host is dropped; nothing usable is `[Direct]`.

## Child processes (`coqui_syshttp`)

```rust
impl Resolver {
    pub fn child_environment<S: AsRef<str>>(&self, endpoints: impl IntoIterator<Item = S>)
        -> Result<ChildEnvironment, Error>;
}
impl ChildEnvironment {   // Clone, Eq; Debug masks credentials
    pub fn pairs(&self) -> Vec<(String, String)>;
    pub fn http_proxy(&self) -> Option<&Proxy>;  pub fn https_proxy(&self) -> Option<&Proxy>;
    pub fn no_proxy(&self) -> &[String];
    pub fn per_endpoint(&self) -> bool;
    pub fn socks_proxy(&self) -> Option<&Proxy>;
    pub fn unexpressed(&self) -> &[String];
}
```

The variables a program you start needs, from what the resolver decides. A proxy that one setting
names for every host (`proxy(...)`, the proxy variables, the system's manual proxies) becomes
`HTTP_PROXY` or `HTTPS_PROXY`, with its credentials percent-encoded in the URL; the bypass list and
loopback (`localhost,127.0.0.1,::1`, unless `<-loopback>`) become `NO_PROXY`. A SOCKS proxy that
serves every scheme without a proxy of its own is `ALL_PROXY` (`socks5h://` or `socks4a://`, so the
proxy resolves the names; `socks_proxy()`). `pairs()` also gives the lowercase names where names are
case-sensitive (some programs read only `http_proxy`), and is empty when everything goes direct
(`NO_PROXY` alone is never given).

A proxy chosen per URL (a PAC script, WPAD, a KDE list naming the hosts that use the proxy)
cannot be one variable, so it is asked about `endpoints`, the URLs the child will use:
`per_endpoint()` is then true; the first proxy answered for a scheme is set for it; an endpoint
answered direct is added to `NO_PROXY` by host; an endpoint answered with a different proxy is
listed in `unexpressed()`. With no endpoints a per-URL proxy sets nothing. Every endpoint is also
checked against a proxy that every host uses, so an endpoint covered by a bypass rule `NO_PROXY`
cannot spell (`<local>`, a glob) is still added by host.

Not expressible, and listed in `unexpressed()`: a bypass entry that is `<local>`, `<subnet>`, a glob, a
network with a port, or unreadable; an exact host of the WinINet or macOS lists is written as a bare name,
which `NO_PROXY` reads as the host and its subdomains (a widening); `<subnet>`; a SOCKS proxy when
a scheme goes direct or through another proxy (`ALL_PROXY` would send it through SOCKS too); and
`ws:`/`wss:` endpoints count as `http:`/`https:`. An endpoint
that is not a URL with a scheme and host is `Error::Parse`. The resolver's `credentials` are in the
URLs, so the child's environment holds a secret: set it on the child only.

## Proxies and credentials (`coqui_syshttp`)

```rust
pub struct Proxy;                       // Clone, Eq, Hash; Display = scheme://host:port; Debug masks secrets
impl Proxy {
    pub fn builder(ProxyScheme, impl Into<String> /* host */, u16 /* port */) -> ProxyBuilder;
    pub fn parse(&str) -> Result<Proxy, Error>;     // also FromStr
    pub fn scheme(&self) -> ProxyScheme;  pub fn host(&self) -> &str;  pub fn port(&self) -> u16;
    pub fn credentials(&self) -> Option<&Credentials>;
    pub fn headers(&self) -> impl Iterator<Item = (&str, &str)>;
    pub fn authority(&self) -> String;                // host:port, IPv6 bracketed
    pub fn url_with_credentials(&self) -> String;     // for a child process's environment
}
impl ProxyBuilder {
    pub fn credentials(self, Credentials) -> Self;
    pub fn header(self, impl Into<String>, impl Into<String>) -> Self;  // to the proxy only
    pub fn build(self) -> Result<Proxy, Error>;       // refuses a header name that is not a token, a value with CR, LF or NUL
}
pub enum ProxyScheme { Http, Https, Socks4, Socks5 }  // non_exhaustive; default_port(), as_str()
pub struct Credentials;  impl Credentials { pub fn new(user, password) -> Self; username(); password() }
pub struct Secret;       impl Secret { pub fn new(impl Into<String>) -> Self; pub fn expose(&self) -> &str }
```

`Proxy::parse` reads `[scheme://][user[:password]@]host[:port][/]`: no scheme is `http`; `socks` is
SOCKS 4; the port defaults per scheme (80, 443, 1080); credentials are percent-decoded; a path
other than `/` is an error (that is a PAC location). Headers are sent in the `CONNECT` only,
never through the tunnel; a header named `Proxy-Authorization` replaces the computed one. Passwords
and header values never appear in `Debug` or `Display`.

## Bypass lists, environment (`coqui_syshttp`)

```rust
impl BypassRules {
    pub fn none() -> Self;  pub fn parse(&str, Dialect) -> Self;
    pub fn from_entries<'a>(impl IntoIterator<Item = &'a str>, Dialect) -> Self;
    pub fn entries(&self) -> impl Iterator<Item = &str>;  pub fn skipped(&self) -> impl Iterator<Item = &str>;
    pub fn is_empty(&self) -> bool;
    pub fn matches(&self, url: &str) -> bool;
}
pub enum Dialect { NoProxy, WinInet, MacOs, GSettings, Kde, KdeReversed }   // non_exhaustive; KdeReversed names the hosts that use the proxy
impl Environment {
    pub fn process() -> Self;  pub fn from_pairs(impl IntoIterator<Item = (K, V)>) -> Self;
    pub fn get(&self, name: &str) -> Option<&str>;
    pub fn proxy_variables(&self) -> Option<ProxyVariables>;
}
impl ProxyVariables { pub fn no_proxy(&self) -> BypassRules;  pub fn has_proxies(&self) -> bool }
```

Entries: a host (exact; in `NoProxy` and `Kde`, and for a leading `.` or `*.` in every dialect, the
host and its subdomains), a `*`/`?` glob (case-blind), a CIDR (`10.0.0.0/8`, `169.254/16`,
`2001:db8::/32`; IP-literal hosts only, no DNS), `host:port` (the URL's port, or its scheme's
default), `<local>` (no dot), `<-loopback>`, and `*` as the whole list. A `*` among other entries
is skipped by `NoProxy` (as curl does) and matches everything in the others. `<subnet>` is a host
that resolves (through the system's resolver, so a lookup waits for DNS) to a link-local address or
to one on a network a local interface is on. `Environment::from_pairs` isolates a test or a
host from the process; the library never reads the environment except through `process()` and
`SystemSettings::read` (`XDG_CONFIG_HOME`, `HOME`).

## System settings (`coqui_syshttp::system`)

```rust
pub struct SystemSettings { pub source, pub state, pub auto_detect: bool, pub pac_url: Option<String>,
    pub pac_script: Option<String>, pub proxies: BTreeMap<String, Proxy>, pub bypass: BypassRules }  // non_exhaustive
impl SystemSettings {
    pub fn read() -> SystemSettings;                  // default (state Unset) when nothing is readable
    pub fn builder(SettingsSource, SettingsState) -> SystemSettingsBuilder;
}
impl SystemSettingsBuilder {
    pub fn auto_detect(self, bool) -> Self;  pub fn pac_url(self, impl Into<String>) -> Self;
    pub fn pac_script(self, impl Into<String>) -> Self;
    pub fn proxy(self, impl Into<String> /* http|https|ftp|socks */, Proxy) -> Self;
    pub fn bypass(self, BypassRules) -> Self;  pub fn build(self) -> SystemSettings;
}
pub enum SettingsSource { None, Windows, MacOs, Gnome, Mate, Kde, NetworkManager }   // non_exhaustive
pub enum SettingsState { Direct, Configured, Unset }                                  // non_exhaustive
```

Windows reads Internet Options (`WinHttpGetIEProxyConfigForCurrentUser`). macOS runs
`scutil --proxy`. Other Unix reads GNOME and MATE (`gsettings`), KDE (`kioslaverc`) and
NetworkManager (`nmcli`), the first with a proxy configured winning, else the first with settings;
KDE's `ProxyType=4` (use the environment) and NetworkManager's `none` count as no settings. A
program is run with no stdin, `LC_ALL=C` and a 5 s limit; failure reads as no settings. `socks=`
in a Windows `ProxyServer` is SOCKS 4; macOS, GNOME, MATE and KDE SOCKS are SOCKS 5.

## Connecting (`coqui_syshttp`)

```rust
impl ProxyConnector {
    pub fn builder() -> ProxyConnectorBuilder;       // Default: 30 s, no Negotiate, User-Agent coqui-syshttp
    pub fn connect(&self, route: &Route, host: &str, port: u16) -> Result<Connection, Error>;
    pub fn connect_url(&self, resolver: &Resolver, url: &str) -> Result<Connection, Error>;
}
impl ProxyConnectorBuilder {
    pub fn timeout(self, Duration) -> Self;  pub fn negotiate(self, Negotiate) -> Self;
    pub fn user_agent(self, impl Into<String>) -> Self;
    pub fn proxy_tls(self, Arc<rustls::ClientConfig>) -> Self;   // feature `tls`: TLS to an https: proxy
    pub fn build(self) -> ProxyConnector;
}
pub enum Connection { Tcp(TcpStream), Tls(..) }       // non_exhaustive; Read + Write; socket() -> &TcpStream
```

`connect` with `Route::Direct` opens a TCP connection to `host:port`. With a proxy it sends
`CONNECT host:port` (a plain `http:` origin is tunnelled too, so a proxy that allows `CONNECT`
only to 443 refuses it) with `Proxy-Authorization: Basic` from the proxy's credentials, then its
extra headers, and returns the tunnel after a `200`. A `407` with no credentials configured, or
that refuses them, is `Error::Login`; any other status is `Error::Refused`. A SOCKS proxy
(`Socks4`, `Socks5`) is spoken to directly: SOCKS 5 with no login or username/password (RFC 1929),
the host always sent as a name for the proxy to resolve (a literal address as an address); SOCKS 4
sends an IPv4 address as is and a name as SOCKS 4a, the proxy's user name as the user id, and
cannot name an IPv6 address (`Error::Unsupported`). A refused login is `Error::Login`, any other
refusal `Error::Refused`. SOCKS 5 GSSAPI login is not spoken. For a proxy with no credentials a `Negotiate` policy naming it answers a
`407` offering `Negotiate` (preferred) or `NTLM` with a handshake on the same connection, at most
`max_legs` requests, tokens at most `max_token` bytes, never over plain `http:` to a proxy that is
not loopback unless the policy allows it. `connect_url` asks the resolver, tries each route in
turn, reports a proxy that failed to connect with `report_failure`, and stops at a refused login;
no route connecting is `Error::NoReachableProxy`.

## Login (`coqui_syshttp::auth`)

```rust
pub fn basic_header(&Credentials) -> String;          // "Basic <base64 user:password>"
pub const fn negotiate_available() -> bool;           // feature `negotiate` on Windows or Unix
pub const DEFAULT_MAX_LEGS: usize = 5;  pub const DEFAULT_MAX_TOKEN: usize = 65_536;
impl Negotiate {
    pub fn builder() -> NegotiateBuilder;             // names nobody
    pub fn wants_proxy(&self, host: &str) -> bool;
    pub fn credentials(&self) -> Option<&Credentials>;  pub fn plain_http(&self) -> bool;
    pub fn max_legs(&self) -> usize;  pub fn max_token(&self) -> usize;
}
impl NegotiateBuilder {
    pub fn proxies<I: IntoIterator<Item = S>, S: AsRef<str>>(self, I) -> Self;  // names, "*.suffix", "*"
    pub fn credentials(self, Credentials) -> Self;    // DOMAIN\user or user@REALM; none = logon session / credential cache
    pub fn plain_http(self, bool) -> Self;  pub fn max_legs(self, usize) -> Self;  pub fn max_token(self, usize) -> Self;
    pub fn build(self) -> Result<Negotiate, Error>;
}
```

Host names compare without case, brackets or a trailing dot; `*.suffix` covers hosts below a
domain, not the domain. Windows uses the `Negotiate` and `NTLM` SSPI packages with the service
name `HTTP/<proxy host>`; Unix uses GSSAPI with `HTTP@<proxy host>` over SPNEGO (NTLM needs the
`gss-ntlmssp` mechanism, and explicit credentials need a Kerberos KDC). Without the `negotiate`
feature a policy that names the proxy fails the login with `Error::Login`. Channel binding is not
sent. For an origin server see `OriginAuth`.

### An origin server's `401`

```rust
impl OriginAuth {
    pub fn builder() -> OriginAuthBuilder;             // names nobody
    pub fn wants_host(&self, host: &str) -> bool;
    pub fn login(&self, url: &str) -> Result<Option<OriginLogin>, Error>;   // None: no rule covers the host
    pub fn plain_http(&self) -> bool;  pub fn max_legs(&self) -> usize;  pub fn max_token(&self) -> usize;
    pub fn max_body(&self) -> usize;
}
impl OriginAuthBuilder {
    pub fn basic<I, S>(self, hosts: I, Credentials) -> Self;        // hosts: names, "*.suffix", "*"
    pub fn negotiate<I, S>(self, hosts: I) -> Self;                 // logon session / Kerberos cache
    pub fn negotiate_as<I, S>(self, hosts: I, Credentials) -> Self; // DOMAIN\user or user@REALM
    pub fn plain_http(self, bool) -> Self;  pub fn max_legs(self, usize) -> Self;
    pub fn max_token(self, usize) -> Self;  pub fn max_body(self, usize) -> Self;
    pub fn build(self) -> Result<OriginAuth, Error>;
}
impl OriginLogin {      // one request's login; sans-IO
    pub fn answer(&mut self, www_authenticate: &[&str]) -> Result<Option<Answer>, Error>;  // after a 401
    pub fn finish(&mut self, www_authenticate: &[&str]) -> Result<(), Error>;              // after a non-401
}
impl Answer { pub fn authorization(&self) -> &str;  pub fn probe(&self) -> bool; }
pub const DEFAULT_MAX_BODY: usize = 1 << 20;
```

`answer` takes the `WWW-Authenticate` values of a `401` and returns the `Authorization` value to
retry with; `Ok(None)` means nothing to retry with (no scheme a rule covers, or the credentials were
refused) and the `401` is the answer. A host may have several rules: `Negotiate` (preferred) or
`NTLM` is answered by a Negotiate rule, `Basic` by a Basic rule; a Negotiate rule that cannot start
(no ticket, no mechanism) falls back to a Basic rule, else is the error. Windows uses SSPI with the
service name `HTTP/<host>`; Unix GSSAPI with `HTTP@<host>`. A password is not sent over plain `http:`
to a host that is not loopback unless `plain_http(true)` (`Error::Login`). `probe()` is true for the
first message of an NTLM login: send that leg with no body (the real one goes with the last leg).
`finish` reads the closing token of a mechanism that proves the server (Kerberos). NTLM
authenticates the connection, so every leg has to use the same one.

## PAC (`coqui_syshttp::pac`, feature `pac`)

```rust
impl PacScript {
    pub fn builder(impl Into<String> /* script text */) -> PacScriptBuilder;
    pub fn from_location(&str, Duration /* download */) -> Result<PacScript, Error>;
    pub fn find_proxy_for_url(&self, url: &str, host: &str) -> Result<String, Error>;
    pub fn routes(&self, url: &str) -> Result<Vec<Route>, Error>;
}
impl PacScriptBuilder { pub fn timeout(self, Duration) -> Self /* default 5 s */; pub fn build(self) -> Result<PacScript, Error>; }
pub const MAX_PAC_BYTES: usize = 1 << 20;
```

A script runs on a thread of its own in the Boa engine with a loop limit of 20 million iterations
and a recursion limit of 400. A call that overruns the time limit is abandoned and the script is
refused for 30 s, then started afresh. `from_location` takes an `http(s)` URL (downloaded directly,
never through a proxy; `https` needs `tls`; up to 3 redirects), a `file:` URL or a path; text is
UTF-8, else Latin-1. Builtins: the Netscape set plus `dnsResolveEx`, `myIpAddressEx`,
`isResolvableEx`, `isInNetEx`, `sortIpAddressList`, `getClientVersion`, `convert_addr`.
`dnsResolve` caches for 30 s and each lookup is bounded at 2 s. `shExpMatch` treats only `*` and `?`
as wildcards, case-sensitively. `weekdayRange`, `dateRange` and `timeRange` answer as Chromium's PAC
library does: `timeRange(9, 17)` runs to 17:59:59 and does not wrap; `dateRange(24, "DEC")` keeps
both ends in the current month; an unknown day or month name is false. A result that is not a
string is `DIRECT`. WPAD tries the URL the DHCP server handed out (option 252) and then
`http://wpad.<domain>/wpad.dat` for each parent domain of this host's name, stopping above a public
suffix. Option 252 is read from the lease the system's DHCP client recorded, not from the network:
NetworkManager's `nmcli -t -f DHCP4,DHCP6 device show` and the `dhclient` lease files under
`/var/lib/dhcp`, `/var/lib/dhclient` and `/var/lib/NetworkManager` (an option named `wpad`,
`option-252`, `private_252` and the like); macOS and other clients are not read. Windows asks WinHTTP,
which does its own DHCP discovery.

## TLS (`coqui_syshttp::tls`, feature `tls`)

```rust
impl Tls {
    pub fn builder() -> TlsBuilder;
    pub fn client_config(&self, host: &str) -> Result<Arc<rustls::ClientConfig>, Error>;  // cached per host
    pub fn selector(&self) -> Option<&Arc<Selector>>;
    pub fn non_defaults(&self) -> Vec<NonDefault>;    // NonDefault { setting, value, default }
}
impl TlsBuilder {
    pub fn trust(self, Trust) -> Self;                          // default System
    pub fn client_certificates(self, ClientCertificates) -> Self;  // default System
    pub fn alpn<I, P: Into<Vec<u8>>>(self, I) -> Self;
    pub fn build(self) -> Result<Tls, Error>;
}
pub enum Trust { System, Roots(Vec<CertificateDer<'static>>), Disabled }      // non_exhaustive
pub enum ClientCertificates { None, System, Selector(Arc<Selector>), Identity(Identity) }  // non_exhaustive
```

`Trust::System` verifies with CryptoAPI (Windows), the Security framework (macOS) and the system's
roots (elsewhere); `Roots` replaces that trust exactly; `Disabled` verifies nobody and is listed by
`non_defaults()`. The crypto provider is `ring`. A configuration is made per server name, because
the certificate chosen for a host is remembered for that host; it keeps its own TLS session cache,
and a resumed session sends no certificate.

```rust
impl Selector {
    pub fn builder() -> SelectorBuilder;  pub fn system() -> Selector;
    pub fn identities(&self) -> Vec<Identity>;  pub fn refresh(&self);  pub fn forget(&self, Option<&str>);
    pub fn candidates(&self, &CertificateRequest) -> Vec<Identity>;
    pub fn select(&self, &CertificateRequest) -> Option<Identity>;
    pub fn resolver(self: &Arc<Self>, Option<String>) -> Arc<dyn rustls::client::ResolvesClientCert>;
}
impl SelectorBuilder {
    pub fn source(self, impl IdentitySource + 'static) -> Self;
    pub fn rules(self, impl IntoIterator<Item = AutoSelectRule>) -> Self;
    pub fn browser_policy(self, bool) -> Self;        // default false
    pub fn chooser(self, Chooser) -> Self;            // Arc<dyn Fn(&CertificateRequest, &[Identity]) -> Option<usize>>
    pub fn remember(self, bool) -> Self;              // default true
    pub fn cache_ttl(self, Duration) -> Self;         // default 60 s
    pub fn build(self) -> Selector;
}
pub trait IdentitySource: Send + Sync + Debug { fn identities(&self) -> Result<Vec<Identity>, Error>; }
```

Candidates are what a browser offers: valid now; no extended key usage, `clientAuth` or
`anyExtendedKeyUsage`, and `digitalSignature` when key usage is present; a key type the server's
signature schemes allow; issued under a CA the server names (the leaf's issuer, or any subject or
issuer in its chain) when it names any; the one expiring last first. The choice, in order: the
identity remembered for the host, the first matching rule (`rules`, then the browsers' policies when
enabled), the chooser, the only candidate when there is exactly one. Otherwise no certificate is
sent and the server decides. A source that fails to list is logged and skipped. `Selector::system()`
is the operating system's keystores (`CngSource::current_user("MY")`, `KeychainSource`,
`Pkcs11Source::system_default()`) and the browsers' policies; with none of them it sends nothing.
`ClientCertificates::System` (the default) therefore presents a keystore certificate to any server
that names a CA the certificate was issued under.

```rust
impl Identity {
    pub fn new(Vec<CertificateDer<'static>>, Arc<dyn rustls::sign::SigningKey>, impl Into<String>) -> Result<Identity, Error>;
    pub fn from_pem(certificates: &[u8], key: &[u8]) -> Result<Identity, Error>;  // PKCS#8, PKCS#1 or SEC1, unencrypted
    pub fn info(&self) -> &CertificateInfo;  pub fn label(&self) -> &str;  pub fn chain(&self) -> &[CertificateDer<'static>];
}
impl CertificateInfo {   // subject(), issuer() (RFC 4514), not_after(), key_type(), client_auth(), fingerprint() -> &[u8; 32],
    pub fn from_chain(&[CertificateDer<'_>]) -> Result<Self, Error>;           // valid_at(SystemTime), issued_under(&[Vec<u8>])
}
pub enum KeyType { Rsa, EcP256, EcP384, EcP521, Ed25519 }   // non_exhaustive; schemes(), algorithm(), usable_with(&[SignatureScheme])
pub struct CertificateRequest;  // new(Option<String>, Vec<Vec<u8>>, Vec<SignatureScheme>); host(), authorities(), signature_schemes()
```

Keystore sources:

- `CngSource` (Windows): `current_user(name)`, `local_machine(name)`, `from_store(Arc<rustls_cng::store::CertStore>, label)`.
  Lists certificates that have a private key openable silently; the key is opened when the first
  signature is asked for, so listing never touches a smart card. The chain is Windows' own, root excluded.
- `KeychainSource` (macOS): `KeychainSource::new()`. The key stays in the Keychain, the Secure
  Enclave or on the card; only the leaf certificate is sent.
- `Pkcs11Source` (feature `pkcs11`): `Pkcs11Source::builder(module_path)` with `.pin(Secret)` and
  `.token_label(..)`, `Pkcs11Source::system_default()` (p11-kit's proxy module, when installed).
  Lists X.509 certificates that have a private key with the same `CKA_ID`; signs RSA (PKCS#1 v1.5 and
  PSS) and ECDSA (P-256, P-384, P-521) through the module. A key that needs a login with no PIN
  given is not listed. Ed25519 is not signed.

```rust
pub struct AutoSelectRule;  AutoSelectRule::builder(pattern).issuer(NameFilter).subject(NameFilter).build()
pub enum NameFilter { Any, Contains(String), Attributes(Vec<(String, String)>) }     // non_exhaustive
pub fn rules_from_policy<'a>(impl IntoIterator<Item = &'a str>) -> Vec<AutoSelectRule>;  // JSON policy entries
pub fn browser_policies() -> Vec<AutoSelectRule>;     // Edge, Chrome, Chromium AutoSelectCertificateForUrls
```

Patterns are `example.com`, `[*.]example.com` (it and subdomains), `*.example.com` (subdomains) and `*`;
a scheme, port or path is ignored. Windows reads the registry (`HKLM` then `HKCU`), Linux the managed
JSON files under `/etc/opt/{edge,chrome}/policies/managed` and `/etc/chromium/policies/managed`;
macOS managed preferences are not read.

## `ureq` (`coqui_syshttp::client`, feature `ureq`)

```rust
impl AgentBuilder {
    pub fn new() -> Self;                             // also Default
    pub fn resolver(self, Resolver) -> Self;          // default Resolver::system()
    pub fn tls(self, Tls) -> Self;                    // default Tls::builder().build()
    pub fn connector(self, ProxyConnector) -> Self;
    pub fn config(self, ureq::config::Config) -> Self;  // its own proxy and TLS settings are not used
    pub fn build(self) -> Result<ureq::Agent, Error>;
}
```

The agent resolves no host name itself: every request asks the resolver for its routes and connects
through the first that works, so a host that exists only behind a proxy needs no DNS where the
client runs. `https:` is spoken with the `Tls` (server verification, client certificate chosen by
the host), `http:` through a proxy is tunnelled with `CONNECT`. The agent uses `ureq`'s unversioned
transport traits, which can change in a `ureq` minor release.

```rust
impl OriginAgent {
    pub fn new(agent: ureq::Agent, auth: OriginAuth) -> Self;       // Clone
    pub fn agent(&self) -> &ureq::Agent;
    pub fn run(&self, request: http::Request<impl AsRef<[u8]>>) -> Result<http::Response<ureq::Body>, ureq::Error>;
}
```

`OriginAgent` wraps any agent (the one `AgentBuilder` makes, or Canoa's) and answers an origin
server's `401` as its `OriginAuth` says. `ureq` middleware cannot send a request twice, so the
requests to a host with a rule run through `OriginAgent::run` itself, on the agent's pooled
connection (NTLM needs the same one for every leg); the body is bytes held in memory and sent once
per leg, except the first NTLM message (no body). A request to another host, or one with its own
`Authorization` header, is sent as it is. A `401` from another host than the request's (after a
redirect) is not answered, and no `Authorization` follows a redirect. The agent's
`http_status_as_error` applies to the final reply; a per-request setting is replaced by it for a
host with a rule. A failed login is an `ureq::Error::Io` of kind `PermissionDenied`. Requests made
with `agent.get(..)` do not go through the policy: use `run`.

## Errors

`Error` is `#[non_exhaustive]`, `std::error::Error`, and never quotes its input. Variants: `Parse`
(a proxy, URL or setting that cannot be read), `Pac`, `MissingEngine`, `Settings`, `NoReachableProxy`,
`Refused` (a proxy refused a `CONNECT`), `Login` (credentials refused, or a Negotiate step failed),
`Unsupported` (an `https:` proxy without `tls`, an IPv6 address in SOCKS 4), `Tls`, `Io`. `Reason` is the clause inside
(`Display` only).

## Environment variables

Read through `Environment::process()` unless an `Environment` is given: `HTTP_PROXY`,
`HTTPS_PROXY`, `FTP_PROXY`, any `<SCHEME>_PROXY`, `ALL_PROXY` (either case; the lowercase name wins),
`NO_PROXY`/`no_proxy`, and `REQUEST_METHOD` (a CGI request ignores `HTTP_PROXY`). `SystemSettings::read`
reads `XDG_CONFIG_HOME` and `HOME` for KDE's `kioslaverc`.

## Gotchas

- A plain `http:` origin through a proxy is tunnelled with `CONNECT`, not forwarded in absolute
  form; a proxy that allows `CONNECT` only to port 443 refuses it.
- The environment decides alone once any proxy variable is set: a scheme with no variable goes
  direct and the operating system is not asked.
- WinHTTP caches PAC results per session, and answers from the user's settings, not from
  `SystemSettings` given to the builder (those replace WinHTTP).
- `Pkcs11Source` loads its module once and keeps it for the life of the source; two sources over
  the same module in one process share its initialisation.
- A `tracing` subscriber is the binary's to install; nothing is printed otherwise.
