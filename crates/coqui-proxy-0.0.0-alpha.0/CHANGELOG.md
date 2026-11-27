# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- Which proxy a URL goes through: `Resolver`, the proxy variables, the system's settings (WinHTTP,
  Internet Options, `scutil`, CFNetwork, GNOME, MATE, KDE, NetworkManager, libproxy), PAC and
  WPAD, bypass rules in five dialects with CIDR, `<local>`, `<subnet>` and `<-loopback>`.
- `ProxyConnector`: HTTP `CONNECT` and SOCKS 4/4a/5 with Basic, Negotiate and NTLM login, and
  origin-server login (`OriginAuth`).
- Sans-I/O cores for the `CONNECT` exchange, its login and the SOCKS frames, with decoders that
  take any bytes.
- `ProxySource`, `ChainSource`, `EnvSource`, `SystemSource`, `PacSource`, `WinHttpSource`,
  `CfNetworkSource` and `LibProxySource` (feature `libproxy`); `ResolverBuilder::source`.
- URL-prefix gateways (`prefix+http`, `prefix+https`, `PREFIX` in PAC results), `Proxy::for_url`,
  `Resolver::scheme_proxies`, `BypassRules` union, rotation and probing of answers.
- PAC: custom builtins, files in UTF-8 or Latin-1, downloads, WPAD discovery, browser
  compatibility, reload of stale scripts.
- `asynch` (feature `tokio`): the connector on Tokio over the same sans-I/O cores, Happy Eyeballs
  (`ConnectorSettings::happy_eyeballs_delay`), `asynch::lookup`; `hyper::Connector` (feature
  `hyper`), a `tower` service for hyper's client. `ConnectorSettings::{failover, bad_proxy_ttl}`
  and `ProxyConnectorBuilder::{failover, bad_proxy_ttl}`: a connector remembers its own failed
  proxies and can try only the first route.
- Settings types for every bound (`ResolverSettings`, `ReaderSettings`, `PacSettings`,
  `ConnectorSettings`) and `non_defaults()` on resolvers and connectors.
- `relay` (feature `relay`): `Relay`, `RelayBuilder`, `RelayHandle`, `RelaySettings`: a loopback HTTP proxy
  started inside the process that sends `CONNECT` and absolute-form `http:` requests through the
  connector, with a random per-relay `Proxy-Authorization: Basic` credential, for clients that cannot take
  a connector (`reqwest`) but should still get failover, PAC, bypass rules and proxy login. The credential is
  also accepted in the `x-coqui-relay-token` header, which a client keeps across a redirect to another host.
- `relay::Relay::start_on` and `RelayBuilder::start_on`: start the relay on a Tokio runtime handle the caller names,
  from any thread; `start()` outside a runtime now says to call it.
- `relay::blocking` (feature `relay-blocking`, no Tokio): `Relay`, `RelayBuilder`, `RelayHandle`: the relay on `std::net`
  with an accept thread and a thread for each connection, over the blocking connector, with the async relay's security
  model and settings. A connection over `max_connections` is answered `503`; dropping the handle closes the listener,
  ends the connections and joins the threads.
- `RelaySettings::{shutdown_wait, poll_interval}` (blocking relay only) and `not_applied()` on both relays, which lists
  the settings that mean nothing to that relay.
- `tools/sdk.lock` and the `deps` branch's pin in it; `packaging/container/Containerfile` builds the library and runs its tests
  from the `deps` branch with no network, through `python3 tools/build.py container build|test`; `.env.example` lists the
  overrides.
- `ProxyConnector::with_proxy_tls` (feature `tls`): the same connector speaking to an `https:` proxy with another TLS
  configuration, so a caller can add it without rebuilding the connector.

### Fixed

- The asynchronous relay closes the connection after a `407`, as the blocking relay does, so a client without the
  credential cannot hold a connection slot with repeated requests. The blocking relay waits for a connection's first
  request only `header_timeout`; `idle_timeout` applies between authenticated requests.
