# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Changed

- The proxy half moved to [coqui-proxy](https://github.com/Boriken-Dev/coqui-proxy) and the TLS half to
  [coqui-sslcontext](https://github.com/Boriken-Dev/coqui-sslcontext). Both are re-exported at the
  paths they had (root items, `auth`, `system`, `pac`, and `tls`), and as `proxy` and `tls`.
- The `client` module is now `ureq`.
- `Error` wraps the two crates' errors (`Proxy`, `Tls`) and has an `Http` variant.

### Added

- `reqwest` failover: with `failover` on (the default) the builder starts a `coqui_proxy::relay` on the
  caller's Tokio runtime and gives `reqwest` that loopback proxy and its random credential as its only proxy, so
  `failover`, `bad_proxy_ttl`, SOCKS 4, Negotiate and NTLM login and the connector's reply limits apply; the relay
  lives as long as the client and its clones. `reqwest::Builder::relay_settings`. `failover(false)` keeps the
  `Proxy::custom` path. `reqwest::Builder::runtime` names the Tokio runtime handle the relay runs on.
- `reqwest::blocking` (feature `reqwest-blocking`): the same builder for `reqwest::blocking::Client`, with failover
  through `coqui_proxy::relay::blocking`, a relay on threads of its own that needs no Tokio runtime and stops, joining
  them, after the last clone of the client drops.
- `hyper` and `reqwest` modules (features of the same names): a `hyper-util` client and a `reqwest` client
  that follow the system's proxy, proxy login and TLS; `AdapterSettings` (`failover`, `bad_proxy_ttl`) on the
  three adapters' builders, `ureq::AgentBuilder::{failover, bad_proxy_ttl}`.
- `agent()`: a `ureq` agent that follows the system, in one call.
- `session` module (feature `session`, with `json` and `session-reqwest`): a port of the Python library
  `syshttp` on `ureq`. `session::Client` owns redirects, cookies, tokens, hooks, retries and a deadline, reading
  the same `SYSHTTP_*` settings, cookie files and token files as the Python library; `session::fetch`;
  `HttpEnv` (prefixes, the proxy settings as a `Resolver`), `SessionSettings` (every bound a setting),
  `cookies`, `jar` (memory and file jars, `store_root`), `hooks` (the environment's hook data) and
  `UreqTransport` (URL-prefix gateways, `CA_BUNDLE`, a pool with a reuse window, bounded `gzip` and `deflate`
  decoding). Not ported: import-time patching and the settings that name Python code.
- `reqwest::builder_from(&SessionSettings)` and `reqwest` cookie stores over the session jars (feature
  `session-reqwest`); `ureq::AgentBuilder::{read_wait, shared_tls}`; `VERSION`.
- `tools/sdk.lock` and the `deps` branch's pin in it; `packaging/container/Containerfile` builds the library and runs its tests
  from the `deps` branch with no network, through `python3 tools/build.py container build|test`; `.env.example` lists the
  overrides.
