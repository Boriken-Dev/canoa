# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Fixed

- An encrypted PKCS#8 key asking for more key derivation work than `KeySettings` allows (10 000 000
  PBKDF2 iterations, 1 GiB of scrypt memory) is refused before any is done, instead of stalling or
  aborting the process; raising a bound is listed by `KeySettings::non_defaults()`.
- `ServerTls::non_defaults()` lists client authentication against the system roots, which accept any
  publicly issued client certificate.
- A key log file is created mode 0600 on Unix, and an existing one open to group or others is
  warned about (no owner-only ACL on Windows).
- A client sends a certificate only when a rule, a browser policy, a chooser or the new
  `CertificatePolicy` picks one: the only fitting candidate is no longer sent to whatever server asks.
  `SelectorSettings::send_sole_candidate` (listed by `non_defaults()`) restores it.
- A pin counts only a certificate the leaf chains to; pins with `Trust::Disabled` are refused.
- A rejected PIN is recorded atomically, before the PIN callback is asked again; a failed
  signature drops its PKCS#11 session; PINs and key files are wiped from memory.
- A CA limited to DNS names or to IP networks excludes the other kind of name.

### Added

- `CertificatePolicy` and `CertificateChoice` (`None`, `Default`, `First`, `Pin`): the caller's rule per
  server host, set with `SelectorBuilder::policy` or `TlsBuilder::certificate_policy`.
- Client configurations that follow the system (`Tls`, `TlsBuilder`, `client_config`): OS trust,
  roots, PEM bundles, public key pins (`Pins`, `Trust::Pinned`), TLS version bounds, key logging.
- Client certificates held in Windows CNG, the macOS Keychain and PKCS#11 tokens, chosen as a
  browser chooses them (`Selector`, `IdentityQuery`, browser auto-select policy including macOS
  managed preferences).
- The PIN model (`Pin`, `PinRequest`, `PinAnswer`): silent first, a keystore's own prompt only on
  request, a rejected PIN never sent again; PKCS#11 always-authenticate keys, token filters and
  card reinsertion.
- Identities from PEM, DER, PKCS#12 and encrypted PKCS#8 keys (`Identity`, `KeyPassword`).
- Server configurations with optional or required client certificates (`ServerTls`).
- A test certificate authority with name constraints (feature `issue`) and software identities
  that count signatures (feature `testing`).
- `tools/sdk.lock` and the `deps` branch's pin in it; `packaging/container/Containerfile` builds the library and runs its tests
  from the `deps` branch with no network, through `python3 tools/build.py container build|test`; `.env.example` lists the
  overrides.
