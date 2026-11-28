# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Changed

- Built against guatiao 1cf0f67f, coqui-transport-pipe 08f0e4d2 (a too-long pipe path is `Error::PathTooLong`, refused as an endpoint like `InvalidName`) and coqui-transport-web 51ec0f12.

### Fixed

- The WebSocket provider runs each upgrade on a thread of its own, at most `max_handshakes` at once (default 64, a setting). A peer that connected and said nothing held the listener's only thread for up to `listen.limits.handshake`, delaying every other peer and a stop; closing the listener now abandons upgrades under way. The pipe provider has no such step: it takes a stream and starts its channel at once.

### Added

- The `transport` guatiao kind: `Transport` (`describe`, `dial`, `listen`, `non_defaults`), `TransportListener` and
  `TransportAcceptor`. Connections are `guatiao-channel` channels; a listening transport hands each peer to the host's acceptor. Every
  method is required, and the models (`TransportInfo`, `DialRequest`, `ListenRequest`, `NonDefault`) refuse a key they do not
  declare. The kinds' description (`include/coqui_transport_guatiao.kinds.json`), the models' schemas and a Python module for
  providers with no Rust are committed, and a test pins the floor hashes and every model's keys.
- `Driver`, `Acceptor` and the `transport_provider!` macro: a transport of `coqui-transport`, with its settings described by
  `coqui-builder`, becomes a provider of the kind in the crate that holds its types. The provider's configuration is its settings, so
  the offer's schema is the settings' schema and a host's configuration is checked against it first; `non_defaults` is generated from
  the description. `settings::{schema, encode, decode, non_defaults}` do the same for any described type.
- `serve`: the connection and listener threads, the channel rules (back-pressure, order, each end told once) and the refusal statuses
  (`Refusal`), written once for every driver.
- Providers `pipe` (feature `pipe`, over `coqui-transport-pipe`) and `websocket` (feature `websocket`, over `coqui-transport-web`),
  registered with `pipe::register` and `websocket::register`.
- `threads::done`, the `unload` hook of a library that offers providers.
- Feature `contract`: `Contract`, the suite every provider is held to, and the host side of the kind it is written with.
- An example `cdylib` that offers the pipe provider to any guatiao host (`examples/pipe_library.rs`), and a transport written in
  Python (`examples/provider-python`), each loaded through a registry and held to the contract.

[Unreleased]: https://github.com/Boriken-Dev/coqui-transport-guatiao/commits/main
