# Canoa

**Discover. Connect. Route.**

Canoa is a standalone Boriken Dev project for discovering, pairing with, and routing connections to local and remote agent services. Its identity and connection model is independent of any one agent protocol, runtime, language or browser.

> Status: initial scaffold. The Rust library implements only an in-memory registry and deny-by-default selection rules. Networking, identity proof, persistent permissions, protocol forwarding and the browser extension are not implemented.

## Design

- Discovery via DNS-SD/mDNS and manual remote endpoint configuration.
- Identity verification and pairing through existing [coqui-peer](https://github.com/Boriken-Dev/coqui-peer) rather than new cryptographic protocols.
- Per-client, per-agent grants; browser integrations must supply independently verified origins.
- Agent Client Protocol (ACP) first, with room for other protocols.
- Native gateway and CLI, browser extension (also called **Canoa**), and language bindings.

## Repository

- `crates/canoa`: core Rust types and in-memory registry.
- `docs/architecture.md`: trust boundaries and future gateway components.
- `extensions/browser`: extension implementation will live here.
- `examples`: end-to-end examples will live here.

A new crate is added only when it owns a distinct contract or runtime boundary; avoid forwarding-only crates.

## Development

Rust 1.89+:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features
```

## License

MPL-2.0. See [LICENSE](LICENSE).
