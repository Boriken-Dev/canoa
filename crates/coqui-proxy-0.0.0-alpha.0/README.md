# coqui-proxy

[![Version](https://img.shields.io/crates/v/coqui-proxy.svg)](https://crates.io/crates/coqui-proxy)
[![Rust version](https://img.shields.io/crates/msrv/coqui-proxy.svg)](https://crates.io/crates/coqui-proxy)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](https://github.com/Boriken-Dev/coqui-proxy/blob/main/LICENSE)
[![Docs](https://img.shields.io/docsrs/coqui-proxy)](https://docs.rs/coqui-proxy)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/coqui-proxy/test.yaml)](https://github.com/Boriken-Dev/coqui-proxy/actions/workflows/test.yaml)

**Which proxy should this request use, and connect me through it.** The proxy variables, the
operating system's settings, PAC and WPAD, bypass rules and proxy login, answered as a list of
routes, and a blocking connector that follows them over HTTP `CONNECT` and SOCKS 4/4a/5. The
sans-I/O cores are public, so an async driver reuses them.

## Features

- One decision order on every operating system: bypass list, an explicit setting, added
  sources, the proxy variables, the system (WinHTTP, Internet Options, `scutil`, CFNetwork,
  GNOME, MATE, KDE, NetworkManager, libproxy), then direct.
- PAC scripts run in-process by the Boa engine under time, loop, recursion and size limits; WPAD
  through DHCP option 252 and DNS devolution; custom builtins; `PREFIX` gateways.
- Bypass rules in five dialects (`NO_PROXY`, WinInet, macOS, GSettings, KDE), CIDR, `<local>`,
  `<subnet>`, `<-loopback>`, and a union operator.
- `CONNECT` through HTTP and HTTPS proxies with Basic, Negotiate and NTLM login (SSPI on
  Windows, GSSAPI elsewhere), SOCKS 4/4a/5 with user and password, and origin-server login.
- Every bound is a setting with a default; nothing is held process-wide; the library prints
  nothing and changes no environment variable.

## Installation

```sh
cargo add --git https://github.com/Boriken-Dev/coqui-proxy coqui-proxy
```

| Feature | Adds | Default |
| --- | --- | --- |
| `pac` | `boa_engine`, `gethostname`: `pac::PacScript`, and PAC and WPAD off Windows | off |
| `tls` | `rustls` (ring), `rustls-platform-verifier`: `https:` proxies and PAC locations | off |
| `negotiate` | Unix: `libgssapi`: Negotiate and NTLM login (Windows needs no crate) | off |
| `libproxy` | `libloading`: `system::LibProxySource` (Unix) | off |
| `tokio` | `tokio`, `tokio-rustls`: `asynch::ProxyConnector`, `asynch::lookup` | off |
| `hyper` | `tokio`, `hyper`, `hyper-util`, `tower-service`: `hyper::Connector`, a `tower` service for hyper's client | off |
| `relay` | `hyper` with its server and client, `http-body-util`, `bytes`, `getrandom`, `subtle`: `relay::Relay`, a loopback proxy started in-process on a Tokio runtime | off |
| `relay-blocking` | `httparse`, `getrandom`, `subtle`: `relay::blocking::Relay`, the same relay on its own threads, with no Tokio | off |

## Quick start

The proxy a URL goes through, as the system says (a list of routes; `Route::Direct` is no proxy):

```rust
let routes = coqui_proxy::Resolver::system().lookup("https://example.com/")?;
assert!(!routes.is_empty());
# Ok::<(), coqui_proxy::Error>(())
```

One proxy, or `None` for direct (build a `Resolver` to reuse the work):

```rust
let proxy: Option<coqui_proxy::Proxy> = coqui_proxy::Proxy::for_url("https://example.com/")?;
# let _ = proxy;
# Ok::<(), coqui_proxy::Error>(())
```

Connect through the first route that works:

```rust,no_run
use coqui_proxy::{ProxyConnector, Resolver};

let resolver = Resolver::system();
let stream = ProxyConnector::default().connect_url(&resolver, "https://example.com/")?;
# let _ = stream;
# Ok::<(), coqui_proxy::Error>(())
```

A client that cannot take a connector (`reqwest`) is given the relay: a loopback proxy started
inside the process, protected by a random credential, that sends each request through the same
connector, so failover, PAC, bypass rules and proxy login apply:

```rust,ignore
use coqui_proxy::{ProxyConnector, Resolver, relay::Relay};

# async fn run() -> Result<(), coqui_proxy::Error> {
let relay = Relay::builder(ProxyConnector::default().asynch(), Resolver::system()).start()?;
// Hand `relay.proxy_url()` and `relay.credentials()` to the client; keep `relay` alive with it.
# let _ = relay;
# Ok(())
# }
```

`start()` uses the Tokio runtime the caller is in; `start_on(&handle)` takes a runtime handle, for a client built
outside it. A program with no Tokio uses `relay::blocking::Relay` (feature `relay-blocking`), which runs on its own
threads over the blocking connector and stops, joining them, when its handle is dropped:

```rust,ignore
use coqui_proxy::{ProxyConnector, Resolver, relay::blocking::Relay};

let relay = Relay::builder(ProxyConnector::default(), Resolver::system()).start()?;
# let _ = relay;
# Ok::<(), coqui_proxy::Error>(())
```

The environment for a child process that cannot ask, from the same decision:

```rust
let environment = coqui_proxy::Resolver::system().child_environment(["https://registry.example/"])?;
for (name, value) in environment.pairs() {
    let _ = (name, value);
}
# Ok::<(), coqui_proxy::Error>(())
```

A resolver of your own: an explicit proxy with a login, a bypass list, and a bound changed.

```rust
use coqui_proxy::{
    BypassRules, Credentials, Dialect, Environment, Proxy, ProxySetting, Resolver,
    ResolverSettings, Route,
};

let mut settings = ResolverSettings::default();
settings.cache_capacity = 64;
let resolver = Resolver::builder()
    .environment(Environment::from_pairs([("X", "y")]))
    .proxy(ProxySetting::Proxy(Proxy::parse("http://proxy.example:3128")?))
    .credentials(Credentials::new("ada", "s3cret"))
    .no_proxy(BypassRules::parse("*.corp.example, 10.0.0.0/8", Dialect::NoProxy))
    .resolver_settings(settings)
    .build();
assert!(matches!(resolver.lookup("https://example.com/")?[0], Route::Proxy(_)));
assert_eq!(resolver.lookup("https://a.corp.example/")?, [Route::Direct]);
# Ok::<(), coqui_proxy::Error>(())
```

## API overview

| Module | Purpose |
| --- | --- |
| crate root | `Resolver`, `Proxy`, `Route`, `BypassRules`, `Environment`, `ProxyConnector`, `Error`, the settings types |
| `source` | `ProxySource`, `ChainSource`, `lookup`: anything that answers for a URL |
| `system` | `SystemSettings`, `SystemSource`, `WinHttpSource`, `CfNetworkSource`, `LibProxySource` |
| `pac` | `PacScript`, `PacSource`, `FindProxy`, `discover`, `builtin_names` (feature `pac`) |
| `asynch` | the connector on Tokio: `ProxyConnector`, `Connection`, `lookup` (feature `tokio`) |
| `hyper` | `Connector`, a `tower` service that returns a tunnel (feature `hyper`) |
| `relay` | `Relay`, `RelayBuilder`, `RelayHandle`: a loopback proxy started in-process, for clients that cannot take a connector (feature `relay`); `relay::blocking` is the same without Tokio (feature `relay-blocking`) |
| `connect` | the sans-I/O cores: `core::{ConnectRequest, ConnectResponse, ProxyLogin}`, `socks::{Greeting, Request, Reply}` |
| `auth` | `Negotiate`, `OriginAuth`, `OriginLogin`, `negotiate_available` |

The header with every signature is [AGENTS.md](https://github.com/Boriken-Dev/coqui-proxy/blob/main/AGENTS.md).

## How a URL's proxy is decided

1. The bypass list (`NO_PROXY`, or the list given): those hosts go direct.
2. An explicit [`ProxySetting`](https://docs.rs/coqui-proxy): a proxy, routes, a PAC location, or direct.
3. Sources added with `ResolverBuilder::source`, in order; the first with an opinion answers.
4. The proxy variables, when any is set: they alone decide, and a scheme with no variable goes
   direct.
5. The system: WinHTTP on Windows (it does WPAD and PAC itself), else the desktop's settings with
   auto-detection, the PAC URL, the PAC script and the manual proxies.
6. Direct.

A proxy that refused a connection is tried last for `failure_memory`; answers can rotate and be
probed (`ResolverSettings::{rotate, probe}`).

## Platforms

| Platform | System settings | Verified |
| --- | --- | --- |
| Windows | WinHTTP, Internet Options; SSPI login | native tests |
| Linux | GNOME, MATE, KDE, NetworkManager (`nmcli`, `dbus-send`), libproxy | native tests; squid and dante as real peers |
| macOS | `scutil`, CFNetwork | native tests |
| other Unix | the environment; the Linux readers where their programs exist | not run |

## Development

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

Real peers (`squid`, `dante-server`, `libproxy`) are opt-in: `COQUI_PROXY_TEST_SQUID=1`,
`COQUI_PROXY_TEST_DANTE=1`, `COQUI_PROXY_TEST_LIBPROXY=1` with
`cargo test --features libproxy --test real_peers -- --ignored`.

### Releasing

No release is cut yet; the crate is consumed by git revision.

## License

MIT OR Apache-2.0, at your option. See [LICENSE](https://github.com/Boriken-Dev/coqui-proxy/blob/main/LICENSE).
