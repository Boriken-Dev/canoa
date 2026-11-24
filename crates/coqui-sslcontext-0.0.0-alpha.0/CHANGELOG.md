# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Fixed

- A pin counts only a certificate the leaf chains to; pins with `Trust::Disabled` are refused.
- A rejected PIN is recorded atomically, before the PIN callback is asked again; a failed
  signature drops its PKCS#11 session; PINs and key files are wiped from memory.
- A CA limited to DNS names or to IP networks excludes the other kind of name.

### Added

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
