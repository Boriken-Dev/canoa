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
- Settings types for every bound (`ResolverSettings`, `ReaderSettings`, `PacSettings`,
  `ConnectorSettings`) and `non_defaults()` on resolvers and connectors.
