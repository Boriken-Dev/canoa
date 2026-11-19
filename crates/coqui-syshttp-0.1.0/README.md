# coqui-syshttp

[![Version](https://img.shields.io/crates/v/coqui-syshttp.svg)](https://crates.io/crates/coqui-syshttp)
[![Rust version](https://img.shields.io/crates/msrv/coqui-syshttp.svg)](https://crates.io/crates/coqui-syshttp)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](https://github.com/Boriken-Dev/coqui-syshttp/blob/main/LICENSE)
[![Docs](https://img.shields.io/docsrs/coqui-syshttp)](https://docs.rs/coqui-syshttp)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/coqui-syshttp/test.yaml)](https://github.com/Boriken-Dev/coqui-syshttp/actions/workflows/test.yaml)

HTTP plumbing that **follows the system by default**: the system's proxy (environment variables,
Windows Internet Options, macOS and desktop settings, PAC and WPAD), proxy login (Basic,
Negotiate and NTLM), the operating system's certificate verifier, and client certificates from
the operating system's keystores, chosen the way a browser chooses them. Every override is
explicit. Full API reference: <https://docs.rs/coqui-syshttp>.

## Features

- **Proxy per URL**: `Resolver` answers with the routes to try, in order. The proxy variables
  decide when any is set, else the operating system does; `NO_PROXY` and the system's own bypass
  list go direct first.
- **Child processes**: `Resolver::child_environment` writes the system's proxy as
  `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY` (for SOCKS) and `NO_PROXY` for a program you start, and
  says what those variables cannot hold.
- **PAC and WPAD**: WinHTTP does both on Windows; elsewhere the Boa JavaScript engine runs the
  script (feature `pac`) with the Netscape and Microsoft `*Ex` builtins, a time limit per call,
  and WPAD discovery from DHCP option 252 (the lease the system's client recorded) and DNS.
- **Proxy login**: Basic from credentials, extra headers sent to the proxy only, and
  Negotiate and NTLM through SSPI on Windows or GSSAPI elsewhere (feature `negotiate`) for the
  proxies a policy names. SOCKS 4, 4a and 5 (with username and password) are spoken too.
- **Origin login**: `OriginAuth` answers a server's `401` with Basic, Negotiate (Kerberos) or NTLM,
  as the logon session or given credentials, for the hosts a policy names; `client::OriginAgent`
  does it for `ureq`.
- **System TLS**: a `rustls` configuration verified by CryptoAPI, the Security framework or the
  system's roots (feature `tls`), or by roots you give.
- **Keystore client certificates**: Windows CNG stores, the macOS Keychain and PKCS #11 modules
  (feature `pkcs11`) answer a server's certificate request, filtered by the CAs the server
  names, with Chrome and Edge's auto-select policy and rules of your own.
- **`ureq` integration**: an agent that does all of the above (feature `ureq`).

## Installation

```bash
cargo add coqui-syshttp
```

A default build has only the proxy resolver and the `CONNECT` connector. Each capability is a
feature:

| Feature | Adds | Needed for |
| --- | --- | --- |
| `pac` | `boa_engine`, `gethostname` | PAC scripts and WPAD on macOS and Unix; Windows needs it only for a script you load by hand |
| `tls` | `rustls`, `rustls-platform-verifier`, `x509-cert`, `sha2`, `serde_json`; `rustls-cng` and `windows-registry` on Windows, `security-framework` on macOS | system TLS, keystore client certificates, `https:` proxies and `https:` PAC locations |
| `pkcs11` | `cryptoki` (and `tls`) | client certificates on smart cards and tokens |
| `negotiate` | `libgssapi` on Unix (the system's GSSAPI library and libclang to build) | Negotiate and NTLM login to a proxy or an origin server |
| `ureq` | `ureq` 3 (and `tls`) | an agent that follows the system |

## Quick start

The proxy for a URL, from the environment first and then the operating system:

```rust
use coqui_syshttp::{Environment, Resolver, Route};

let resolver = Resolver::builder()
    .environment(Environment::from_pairs([("HTTPS_PROXY", "http://proxy.corp:3128")]))
    .system(false)
    .build();
let routes = resolver.lookup("https://example.com/")?;
assert!(matches!(&routes[0], Route::Proxy(proxy) if proxy.host() == "proxy.corp"));
# Ok::<(), coqui_syshttp::Error>(())
```

`Resolver::system()` is the same with this process's environment and no override.

The variables for a program you start (a package manager, `git`, an agent):

```rust,no_run
use coqui_syshttp::Resolver;

let child = Resolver::system().child_environment(["https://registry.example/"])?;
for (name, value) in child.pairs() {
    // command.env(name, value);
}
// A PAC script is asked about the endpoints named above; child.unexpressed() lists what is left.
# Ok::<(), coqui_syshttp::Error>(())
```

A tunnel through a proxy that wants Basic:

```rust,no_run
use coqui_syshttp::{Credentials, Proxy, ProxyConnector, ProxyScheme, Route};

let proxy = Proxy::builder(ProxyScheme::Http, "proxy.corp", 3128)
    .credentials(Credentials::new("ada", "s3cret"))
    .build()?;
let tunnel = ProxyConnector::default().connect(&Route::Proxy(proxy), "example.com", 443)?;
# let _ = tunnel;
# Ok::<(), coqui_syshttp::Error>(())
```

An agent whose requests follow the system's proxy and whose TLS is verified by the operating
system, presenting a keystore certificate where a server asks for one:

```rust,no_run
# #[cfg(feature = "ureq")]
# fn demo() -> Result<(), Box<dyn std::error::Error>> {
use coqui_syshttp::client::AgentBuilder;

let agent = AgentBuilder::new().build()?;
let body = agent.get("https://example.com/").call()?.body_mut().read_to_string()?;
# let _ = body;
# Ok(())
# }
```

## API overview

| Module | Purpose |
| --- | --- |
| root | `Resolver`, `ChildEnvironment`, `Route`, `Proxy`, `Credentials`, `BypassRules`, `Environment`, `ProxyConnector`, `Connection`, `Negotiate`, `OriginAuth`, `OriginLogin`, `Error` |
| `system` | the operating system's proxy settings as data: `SystemSettings::read` |
| `auth` | `basic_header`, the `Negotiate` policy for proxies, the `OriginAuth` policy for origin servers, their limits |
| `pac` | `PacScript`: a PAC script evaluated in-process (feature `pac`) |
| `tls` | `Tls`, `Trust`, `Selector`, `Identity`, keystore sources, auto-select rules (feature `tls`) |
| `client` | `AgentBuilder`: a `ureq` agent that follows the system; `OriginAgent`: it answers an origin's `401` (feature `ureq`) |

## How a URL's proxy is decided

1. `NO_PROXY` (or the bypass list given to the builder): those hosts go direct. Loopback always
   does, unless the list says `<-loopback>`.
2. An explicit `ProxySetting` (a proxy, a PAC location, or none) decides alone.
3. When any proxy variable is set (`HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY`, ... any case, the
   lowercase spelling first), the variables alone decide, per scheme; a scheme with no variable
   goes direct. A value may be a proxy URL, a PAC result (`PROXY a:1; DIRECT`), a PAC location or a
   PAC script.
4. Otherwise the operating system: WinHTTP on Windows; elsewhere auto-detection, the PAC URL, the
   PAC script and then the manual proxies, in that order. A source that fails passes to the next
   and is tried again after 30 seconds. The settings are read again every minute.
5. Direct.

## Platforms

| | Windows | macOS | Linux and other Unix |
| --- | --- | --- | --- |
| Settings | Internet Options (WinHTTP) | `scutil --proxy` | GNOME, MATE, KDE, NetworkManager |
| PAC and WPAD | WinHTTP | Boa (`pac`) | Boa (`pac`) |
| Negotiate and NTLM | SSPI | GSSAPI | GSSAPI (NTLM needs `gss-ntlmssp`) |
| Client certificates | CNG store | Keychain | PKCS #11 |
| Server verification | CryptoAPI | Security framework | the system's roots |

Not done: SOCKS 5 GSSAPI login, DHCP option 252 on macOS (Windows asks WinHTTP), and the Chrome and
Edge policy on macOS. A Kerberos exchange with a domain controller has not been run against a real
domain; the Windows tests complete Negotiate and NTLM against the security package on the same
machine, and the Unix GSSAPI path is exercised only as far as a ticket cache allows.

## Development

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

`--all-features` on Linux and macOS needs the system's GSSAPI library and libclang (on Debian,
`libkrb5-dev` and `libclang-dev`). Tests use loopback stand-ins only: a proxy that requires
Basic or Negotiate, a PAC file, and a TLS server that asks for a certificate from a test CA; every
key is generated by the test. The Windows keystore test imports a certificate into an in-memory
store and deletes its key afterwards. The PKCS #11 test needs SoftHSM and is run with
`COQUI_SYSHTTP_TEST_PKCS11=1 cargo test --all-features -- --ignored`.

### Releasing

This project follows [Semantic Versioning](https://semver.org/) and keeps a
[`CHANGELOG.md`](https://github.com/Boriken-Dev/coqui-syshttp/blob/main/CHANGELOG.md). Pushing a
tag matching `v*` triggers the release workflow: test gate, package, GitHub release, publish.

## License

MIT OR Apache-2.0, at your option: see
[LICENSE-MIT](https://github.com/Boriken-Dev/coqui-syshttp/blob/main/LICENSE-MIT) and
[LICENSE-APACHE](https://github.com/Boriken-Dev/coqui-syshttp/blob/main/LICENSE-APACHE).
