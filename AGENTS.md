# Canoa — repository orientation

Canoa uses a Rust workspace with focused crates, public contracts, cross-platform tests and a `bindings/` directory for browser/language integrations.

## Layout

| Path | Responsibility |
| --- | --- |
| `crates/canoa/` | Registry, service descriptors and deny-by-default authorization |
| `docs/` | Architecture and interoperability contracts |
| `bindings/browser/chromium/` | MV3 browser extension code and permission policy |
| `bindings/` | Other language/browser implementations as needed |
| `examples/` | Runnable examples, not empty placeholders |

## Rules

- Canoa is one implementation of an interoperable concept. No proprietary agent gateway is required.
- ACP over WS/WSS is the transport contract. Default to WSS; plain WS is only a deliberate local-development exception.
- Browser clients must authenticate peer identity through an independently implementable versioned wire profile; a claimed peer ID or TLS hostname alone is not proof of pairing.
- The extension owns its permissions for webpages and agents independently of the native host.
- Permissions are scoped by verified peer, client origin, tab/document and capability, and default to deny. Navigation and revocation close the access.
- Agent permissions and browser permissions are distinct: neither can override the other's rejection.
- Peer identity and key continuity should reuse coqui-peer primitives but not require clients to use the Rust crate.
- ACP, MCP, and WebMCP are different protocols/APIs; never assert a webpage's WebMCP tool is an HTTP MCP server.
- Add crates only for genuine public contracts. No forwarding-only crates.

## Checks

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features
```
