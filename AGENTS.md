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

## Project identity

- Publisher and primary domain: **Boriken Dev** (`boriken.dev`); GitHub organization `Boriken-Dev`.
- Use `dev.boriken.*` for reverse-DNS identifiers (Android application IDs, Apple bundle IDs, native messaging hosts). Canoa uses `dev.boriken.canoa` as its base identifier.
- Keep ecosystem-native package naming: Rust crates use hyphens (`coqui-net-transport`), Dart packages underscores, and scoped npm packages must use a verified scope. Do not invent a claimed publishing scope.
- Documentation remains at GitHub Pages until DNS is configured for a custom domain. Do not claim hypothetical URLs are live.
- See `docs/naming.md` for the naming registry.

## Rules

- Canoa is one implementation of an interoperable concept. No proprietary agent gateway is required.
- Prefer WebTransport over HTTPS, with explicit WSS fallback on connection/path timeout or failure; plain WS is a deliberate loopback-only development exception. Do not retry unacknowledged agent operations automatically.
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
