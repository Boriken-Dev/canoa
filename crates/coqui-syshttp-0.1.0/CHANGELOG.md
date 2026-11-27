# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Changed

- The proxy half moved to [coqui-proxy](https://github.com/Boriken-Dev/coqui-proxy) and the TLS half to
  [coqui-sslcontext](https://github.com/Boriken-Dev/coqui-sslcontext). Both are re-exported at the
  paths they had (root items, `auth`, `system`, `pac`, and `tls`), and as `proxy` and `tls`.
- A client sends a certificate from the keystore only when a rule, policy or chooser picks one for the server (coqui-sslcontext
  `CertificatePolicy`, `TlsBuilder::certificate_policy`); the sole fitting identity is no longer sent unasked unless
  `TlsBuilder::send_sole_candidate(true)` is set, which `Tls::non_defaults()` lists.
- The `client` module is now `ureq`.
- `Error` wraps the two crates' errors (`Proxy`, `Tls`) and has an `Http` variant.

### Fixed

- A jar token is sent only over `https:` or to a loopback host, and only to the port it is kept for (the host's entry
  serves the scheme's default port, `host:port` another one). `TokenSettings::{plain_http, any_port}` and
  `ClientBuilder::{token_plain_http, token_any_port}` lift either rule and are listed by
  `SessionSettings::non_defaults()`.
- A `reqwest` or `reqwest::blocking` client with `failover(false)` fails a request whose proxy lookup fails (an
  erroring PAC, a malformed proxy setting) instead of sending it direct.
- A cookie `Domain` that is a public suffix is refused unless it is the host: the Public Suffix List (feature `psl`, on
  with `session`) joins the registry heuristic. `CookieSettings::{public_suffixes, public_suffix_list}` add suffixes
  and turn the list off (listed by `SessionSettings::non_defaults()`); `SetCookieContext::{public_suffixes,
  public_suffix_list}`.
- `session::ClientBuilder::non_defaults()` and `Client::non_defaults()` list what the client loosens: its settings,
  failover, the trust and `Tls` given, the origin login policy and the connector. The `ureq`, `hyper` and
  `UreqTransport` builders also list the `Tls` and connector they were given, and the `reqwest` builders their
  connector (a raw `rustls::ClientConfig` cannot be inspected).
- `UreqTransportBuilder::proxy_tls` beside `connector` adds the TLS configuration to the given connector instead of
  replacing it with a default one, so the connector's reply limits, timeout, user agent and login policy hold.
- `reqwest::Builder::configure` and its blocking twin run before the proxy and TLS configuration are set, as their doc
  says, so a closure that calls `no_proxy()` or `proxy(..)` can no longer take the relay out of the request's path.
- The bounds that were constants are settings with the same values: `SessionSettings::drain_bytes` (64 KiB, the
  redirect body read to keep a connection), `FileJarBuilder::file_retries` (10 tries, 10 ms) with
  `cookies::{load_netscape_with, save_netscape_with}` and `jar::FileRetry`, and `UreqTransportBuilder::peer_text_limit`.
  The cut of a redirect target and of a transport error's text in a message follows `peer_text_limit` (120) instead
  of a fixed 200.

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
