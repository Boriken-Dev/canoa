# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- `mux`: named streams inside one link, with credit per stream, metadata on open, a distinct cause for every end, and an optional heartbeat, in Rust, Dart and JavaScript, on one set of vectors.
- `peers` and `link`: claims and grants per peer behind a `PeerStore` trait with audit events, and serving a link under them, failing closed, with a capability document and per-protocol admission.
- `auth` and `pairing` (feature `sealed`): Noise and WebCrypto identities, proof, the sealed channel, and PIN pairing over any channel.
- `forward` (feature `forward`): `tcp/1`, one TCP connection per stream in either direction, each target its own grant.
- The wire is the one `coqui-agentic-peer` speaks (`link/1`, `tcp/1`, every frame byte), so a link of either library talks to a link of the other; `tests/interop.rs` runs both ends under both proof methods.
