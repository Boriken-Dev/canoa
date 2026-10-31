# Canoa

**Discover. Connect. Route.**

Canoa is a standalone Boriken Dev project for discovering, pairing with, and routing connections to local and remote agent services. Its identity and connection model is independent of any one protocol, language or browser.

> Status: early development. The Rust library contains an in-memory service registry and deny-by-default selection rules. The Chromium extension is a preliminary client of a **not-yet-implemented** native host; it is not a standalone MCP server and cannot currently serve connected agents.

## Capabilities and direction

- Discover local DNS-SD/mDNS services and manually registered remote endpoints.
- Authenticate identities and pair peers using [coqui-peer](https://github.com/Boriken-Dev/coqui-peer).
- Grant or revoke access by verified client, service and capability.
- Connect applications to ACP agents and other supported protocols.
- **Browser as an MCP tool provider:** expose user-approved page inspection and interaction tools, plus discover and invoke a page's registered WebMCP tools.
- **Website-provided MCP:** detect opt-in site advertisements for their own remote MCP endpoints and pass candidates to the broker for verification and consent; never silently connect or relay credentials.
- Support a native gateway and CLI, the **Canoa** browser extension, and language bindings.

## Repository layout

| Path | Responsibility |
| --- | --- |
| `crates/canoa/` | Protocol-neutral Rust registry and authorization primitives |
| `docs/` | Architecture and protocol contracts |
| `bindings/browser/chromium/` | Canoa Manifest V3 Chrome/Chromium extension |
| `bindings/browser/` | Browser integration guidance and shared browser-facing contracts |
| `bindings/{python,node,...}/` | Future language bindings, added when implemented |
| `examples/` | Runnable examples, added with real implementations |

The Rust core intentionally has no network dependencies yet. Add a new crate only for a real public contract or runtime boundary; avoid forwarding-only crates.

## Development

Rust 1.89 or later:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features
```

See [AGENTS.md](AGENTS.md) and [docs/architecture.md](docs/architecture.md).

## License

MPL-2.0. See [LICENSE](LICENSE).
