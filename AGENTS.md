# Canoa — repository orientation

Canoa uses a Rust workspace with explicit public contracts, focused crates and cross-platform tests.

## Layout

| Path | Responsibility |
| --- | --- |
| `crates/canoa/` | Registry, service descriptors and grant checks |
| `docs/` | Protocol and architecture contracts |
| `bindings/browser/chromium/` | Manifest V3 Canoa extension (Chrome/Chromium) |
| `bindings/browser/` | Browser-facing integration and optional shared SDK |
| `bindings/` | Other language bindings as they are implemented |
| `examples/` | Real executable scenarios, not empty placeholders |

## Rules

- Discovery does not prove identity. Browser pages, tool metadata and endpoint advertisements are untrusted.
- Clients are identified by a trusted integration, not an unauthenticated payload. Browser origins must be established by the extension's trusted context.
- Pairing reuses `coqui-peer` and its method crates instead of duplicating cryptography.
- Client-to-agent access and agent-to-page access are separate grants; browser navigation revokes page grants.
- Tool listing and read access must not imply permission to click, fill, execute WebMCP tools, or connect to a site's MCP endpoint.
- MCP `tools/list` / `tools/call` must be implemented in a properly authenticated native gateway; extension messages are not an MCP transport.
- Native messaging hosts must be installed, pinned to the extension ID and authenticate their remote/local clients; do not expose a permissive localhost server.
- `document.modelContext` is optional and experimental. Do not assume webpage tools can be reached in every browser.
- Keep the native gateway separate from the browser. A webpage cannot directly request a privileged native connection.
- A new crate or binding must own a real public contract; no forwarding-only crates.

## Checks

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features
```
