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

## Errors

`Error` wraps the two crates' errors; a message names what failed and never quotes input.

## Environment variables

None read by this crate; coqui-proxy reads the proxy variables (`*_PROXY`, `ALL_PROXY`,
`NO_PROXY`, `REQUEST_METHOD`, `XDG_CONFIG_HOME`, `HOME`).

## Gotchas

- `ureq` 3 middleware cannot send a request twice, so an origin login is `OriginAgent::run`, not a
  middleware. A body-less POST needs an empty `Vec<u8>` body: `()` makes `ureq` send it chunked.
- `tls::Error` and `proxy::Error` are different types; `Error` converts both with `?`.
