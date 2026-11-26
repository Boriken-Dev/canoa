# coqui-syshttp

[![Version](https://img.shields.io/crates/v/coqui-syshttp.svg)](https://crates.io/crates/coqui-syshttp)
[![Rust version](https://img.shields.io/crates/msrv/coqui-syshttp.svg)](https://crates.io/crates/coqui-syshttp)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](https://github.com/Boriken-Dev/coqui-syshttp/blob/main/LICENSE)
[![Docs](https://img.shields.io/docsrs/coqui-syshttp)](https://docs.rs/coqui-syshttp)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/coqui-syshttp/test.yaml)](https://github.com/Boriken-Dev/coqui-syshttp/actions/workflows/test.yaml)

HTTP that **follows the system by default**, in one call: the system's proxy (environment
variables, Windows Internet Options, macOS and desktop settings, PAC and WPAD), proxy login,
the operating system's certificate verifier, and client certificates from the operating
system's keystores. The proxy half lives in
[coqui-proxy](https://github.com/Boriken-Dev/coqui-proxy) and the TLS half in
[coqui-sslcontext](https://github.com/Boriken-Dev/coqui-sslcontext); this crate re-exports both
at the paths it has always had and adds the HTTP client that uses them. Full API reference:
<https://docs.rs/coqui-syshttp>.

## Features

- **One call**: `coqui_syshttp::agent()` is a `ureq` agent whose requests follow the system's
  proxy, log in to it, and verify and authenticate with the system's TLS (feature `ureq`).
- **Everything of the two crates, at the same paths**: `Resolver`, `Route`, `Proxy`,
  `ProxyConnector`, `OriginAuth`, ... from coqui-proxy at the root, `auth`, `system` and `pac`
  as modules, and coqui-sslcontext as `tls`.
- **Three HTTP stacks**: the same system proxy, proxy login and TLS for `ureq` (blocking), `hyper-util`
  (`hyper::client()`) and `reqwest` (`reqwest::client()`, and `reqwest::blocking::client()`), with the same
  failover settings. `reqwest` cannot retry a route, so its failover runs through an in-process loopback relay
  (on a Tokio runtime for the async client, `Builder::runtime` to name it; on threads of its own, no Tokio, for
  the blocking one).
- **Origin login**: `ureq::OriginAgent` answers a server's `401` with Basic, Negotiate or NTLM
  for the hosts a policy names.
- **Sessions**: `session::Client` owns redirects, cookies, tokens, hooks, retries and a deadline above
  `ureq`, with every setting read through `SYSHTTP_*` environment variables, the same names and
  file formats as the Python library `syshttp` (feature `session`).
- **One error type**: `Error` wraps the proxy and TLS crates' errors.

## Installation

```bash
cargo add --git https://github.com/Boriken-Dev/coqui-syshttp coqui-syshttp --features ureq
```

A default build re-exports the proxy resolver and the `CONNECT` connector. Each capability is a
feature:

| Feature | Turns on | Needed for |
| --- | --- | --- |
| `pac` | `coqui-proxy/pac` | PAC scripts and WPAD on macOS and Unix |
| `negotiate` | `coqui-proxy/negotiate` | Negotiate and NTLM login to a proxy or an origin server |
| `tls` | `coqui-sslcontext` (with its `system` feature), `coqui-proxy/tls` | the `tls` module, `https:` proxies and PAC locations |
| `pkcs11` | `tls`, `coqui-sslcontext/pkcs11` | client certificates on smart cards and tokens |
| `ureq` | `tls`, `ureq` 3 | the `ureq` module and `agent()` |
| `hyper` | `tls`, `coqui-proxy/hyper`, `hyper-util`, `tokio-rustls` | the `hyper` module and `hyper::client()` |
| `reqwest` | `tls`, `coqui-proxy/relay`, `reqwest` 0.13 | the `reqwest` module and `reqwest::client()` |
| `reqwest-blocking` | `reqwest`, `reqwest/blocking`, `coqui-proxy/relay-blocking` | `reqwest::blocking::client()`, failover through a relay with no Tokio of its own |
| `session` | `ureq`, `url`, `regex`, `flate2`, `encoding_rs` | the `session` module |
| `json` | `session`, `serde_json` | JSON bodies and replies of the session layer |
| `session-reqwest` | `session`, `reqwest`, `reqwest/cookies` | the session jars as `reqwest` cookie stores, `reqwest::builder_from` |

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

An agent whose requests follow the system's proxy and whose TLS is verified by the operating
system, presenting a keystore certificate where a server asks for one:

```rust,no_run
# #[cfg(feature = "ureq")]
# fn demo() -> Result<(), Box<dyn std::error::Error>> {
let agent = coqui_syshttp::agent()?;
let body = agent.get("https://example.com/").call()?.body_mut().read_to_string()?;
# let _ = body;
# Ok(())
# }
```

## Sessions

A `session::Client` is a client with memory: it follows redirects (dropping `Authorization`,
`Cookie` and `Proxy-Authorization` when a hop changes scheme, host or port), keeps cookies by host
in memory or in Netscape files, sends a token from a jar as `Authorization`, retries transient
statuses and dropped connections for methods that are safe to send twice, and bounds a whole
request with a deadline. Routing is asked again for every hop. Python syshttp's settings are read
unchanged, under the prefixes of `SYSHTTP_PREFIX`:

```rust,no_run
# #[cfg(feature = "session")]
# fn demo() -> Result<(), Box<dyn std::error::Error>> {
use coqui_syshttp::session::{Client, CookieStore, fetch};
use std::time::Duration;

let client = Client::builder()
    .cookies(CookieStore::PerHost) // files under SYSHTTP_HOME, one per host
    .deadline(Some(Duration::from_secs(30)))
    .build()?;
let mut reply = client.request("GET".parse()?, "https://example.com/").send()?;
let text = reply.text()?;
# let _ = text;

// One request through a client made for the call:
let reply = fetch("https://example.com/").send()?;
assert!(reply.status() < 600);
# Ok(())
# }
```

Every bound is a setting of `session::SessionSettings` with Python syshttp's default (timeouts of
10 s and 60 s, 30 redirects, 3 retries, an 8 MiB reply limit, ...). Token and cookie values are
`Secret`s: they appear in no `Debug` output, error or log. Hooks are code given to
`ClientBuilder::hook`, or data (`session::hooks`) for a front end that runs a wrapper; there is no
import-time patching and no `module:attr` loading, because Rust code is linked, not imported by
name. `ureq` sends the standard methods only, so a `PROPFIND` needs a transport of your own.
`Response::content()` is the body whole (`bytes` would collide with `std::io::Read::bytes`).

## API overview

| Item | Purpose |
| --- | --- |
| root | everything coqui-proxy exports at its root (`Resolver`, `Route`, `Proxy`, `Credentials`, `BypassRules`, `ProxyConnector`, `OriginAuth`, ...), `Error`, `Reason`, `agent()` |
| `proxy` | the whole coqui-proxy crate |
| `auth`, `system`, `pac` | coqui-proxy's modules (`pac` behind its feature) |
| `tls` | the whole coqui-sslcontext crate (feature `tls`) |
| `ureq` | `AgentBuilder`, `OriginAgent` (feature `ureq`) |
| `hyper` | `client()`, `Builder`, `HttpsConnector` (feature `hyper`) |
| `reqwest` | `client()`, `Builder`, `builder_from` (feature `reqwest`, the last with `session-reqwest`) |
| `reqwest::blocking` | `client()`, `Builder` for `reqwest::blocking::Client` (feature `reqwest-blocking`) |
| `session` | `Client`, `fetch`, `HttpEnv`, `SessionSettings`, `cookies`, `jar`, `hooks` (feature `session`) |

How a URL's proxy is decided, the platform table and every setting are documented in
[coqui-proxy](https://github.com/Boriken-Dev/coqui-proxy).

## Development

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

`--all-features` on Linux and macOS needs the system's GSSAPI library and libclang. Tests use
loopback stand-ins only (a Basic proxy, a login origin, a TLS server that asks for a certificate
from a test CA); every key is generated by the test.

### Building without a network, and in a container

The `deps` branch holds every crate `Cargo.lock` names as text, and `tools/sdk.lock` records the toolchains, `rustup` and the
`deps` commit a build is made with. `tools/build.py` (Python 3.11, standard library only; `python3 tools/build.py --help`)
holds them to the repository and builds from them:

```text
python3 tools/build.py sdk check                       # the lock, the Containerfile's pins and the deps commit agree
python3 tools/build.py container build --target build  # compile the library offline, in a container
python3 tools/build.py container test                  # `cargo test --all-features` offline, in the same container
python3 tools/build.py container build --print         # the overrides and the engine command, nothing run
```

The library ships no image: `packaging/container/Containerfile` has the `sdk`, `build` and `test` stages and nothing to run.
Where a build gets its toolchain, mirrors, base image or Visual Studio paths from is set in `.env` or `.env.local` (both
ignored by git; `.env.example` lists every key and `python3 tools/build.py help env` explains them). A pin changes in
`tools/sdk.lock` first; `sdk check` fails until the Containerfile follows. To change `Cargo.lock`, refresh the branch on a
clean checkout of main with `python3 tools/build.py deps refresh` and push `deps` before `main`.

### Releasing

No release is cut yet; the crate is consumed by git revision. Changes are listed in
[`CHANGELOG.md`](https://github.com/Boriken-Dev/coqui-syshttp/blob/main/CHANGELOG.md).

## License

MIT OR Apache-2.0, at your option. See
[LICENSE](https://github.com/Boriken-Dev/coqui-syshttp/blob/main/LICENSE).
