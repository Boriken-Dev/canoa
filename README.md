# Canoa

**Discover. Connect. Route.**

Canoa is a standalone Boriken Dev project implementing interoperable discovery, authentication, permissions and routing between applications and local or remote AI agents. It supports a language-agnostic core and a browser extension, all branded **Canoa**.

> Status: early development. The Rust library contains an in-memory service registry and deny-by-default selection rules. Direct ACP-over-WebSocket connections, a browser-compatible peer-auth profile, browser MCP tools, and browser UI are not implemented yet.

## Interoperability

Canoa is **one implementation**, not a proprietary gateway protocol. Other implementations can connect through compatible **ACP over WS/WSS** and a common **peer-authentication wire profile**. The [interop specification](docs/interop.md) explains the transport, peer-proof and conformance requirements. The browser-compatible `coqui-peer` auth profile must be published and validated before it is treated as interoperable.

## Capabilities

- Discover local DNS-SD/mDNS services and configure remote agent endpoints.
- Pair and authenticate endpoints, reusing [coqui-peer](https://github.com/Boriken-Dev/coqui-peer) identity and trust primitives where appropriate.
- **Browser-managed permissions:** the extension keeps its own per-peer, per-tab, per-origin capability grants, independently from any local broker or remote agent.
- ACP session access over WebSocket, with WSS as the default and explicit local-only WS exceptions.
- Agent-facing browser MCP tools (DOM snapshot, page interaction and WebMCP).
- Website-advertised MCP endpoints, treated as discovery hints requiring independent authorization.
- Native tools and language bindings as the functionality becomes available; a native companion is optional for direct WSS browser access.

## Layout

| Path | Responsibility |
| --- | --- |
| `crates/canoa/` | Protocol-neutral Rust registry and authorization primitives |
| `docs/architecture.md` | Security and architectural boundaries |
| `docs/interop.md` | ACP WS/WSS and peer-auth interoperability contract |
| `bindings/browser/chromium/` | Canoa Chrome/Chromium extension |
| `bindings/` | Future browser and language integrations |
| `examples/` | Real runnable examples as features are implemented |

## Development

Rust 1.89 or later:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features
```

## License

MPL-2.0. See [LICENSE](LICENSE).
