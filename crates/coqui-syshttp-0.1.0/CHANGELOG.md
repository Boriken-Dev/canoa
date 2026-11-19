# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- `Resolver`: the proxy for a URL, from `NO_PROXY`, an explicit setting, the proxy environment
  variables, and the operating system (WinHTTP on Windows; Internet Options, `scutil --proxy`, GNOME,
  MATE, KDE and NetworkManager settings elsewhere), with PAC, WPAD, per-proxy failure memory and a
  login for proxies that have none.
- Bypass lists in the `NO_PROXY`, WinINet, macOS, GNOME and KDE dialects (`BypassRules`).
- `pac::PacScript` (feature `pac`): PAC scripts run in the Boa engine with the Netscape and
  Microsoft `*Ex` builtins, a time limit per call and WPAD discovery.
- `ProxyConnector`: a direct connection or an HTTP `CONNECT` tunnel through an `http:` or
  `https:` proxy, logged in with Basic credentials, a proxy's extra headers, or Negotiate and NTLM
  (SSPI on Windows, GSSAPI elsewhere; feature `negotiate`).
- `tls::Tls` (feature `tls`): `rustls` client configurations verified by the operating system, with
  client certificates chosen by the CAs a server names, from `Selector`s over Windows CNG stores,
  the macOS Keychain and PKCS #11 modules (feature `pkcs11`), with the `AutoSelectCertificateForUrls`
  policy of Chrome and Edge.
- `Resolver::child_environment`: `HTTP_PROXY`, `HTTPS_PROXY` and `NO_PROXY` for a child process
  from the proxy the resolver decides, loopback and the bypass list included; a PAC script or
  WPAD is resolved for the endpoints the caller names, and `ChildEnvironment` reports what the
  variables cannot say.
- `client::AgentBuilder` (feature `ureq`): a `ureq` agent that follows the system's proxy, proxy
  login and TLS.
- `OriginAuth` and `OriginLogin`: Basic, Negotiate and NTLM answered to an origin server's `401`,
  for the hosts a policy names, as the logon session (SSPI, the Kerberos ticket cache) or given
  credentials. `client::OriginAgent` (feature `ureq`) runs requests through it, on one pooled
  connection, with the body sent again for each leg.
- SOCKS 4, 4a and 5 (with username and password) in `ProxyConnector`; the host is sent as a name
  for the proxy to resolve.
- `Resolver::child_environment` writes a SOCKS proxy as `ALL_PROXY` (`socks5h://`, `socks4a://`)
  when it serves every scheme the other variables do not.
- The `<subnet>` entry of a bypass list: a host that resolves to a link-local address or to one on
  a network of a local interface.
- WPAD from DHCP option 252 on Unix, from the lease NetworkManager or `dhclient` recorded, ahead of
  the DNS names.
