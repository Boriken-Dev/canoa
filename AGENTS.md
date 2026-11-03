# Canoa — repository orientation

Canoa uses a Rust workspace with focused crates, public contracts, cross-platform tests and a `bindings/` directory for browser/language integrations.

| Shipped header | Covers |
| --- | --- |
| [`crates/canoa/AGENTS.md`](crates/canoa/AGENTS.md) | the `canoa` crate's public API |
| [`crates/canoa-host/AGENTS.md`](crates/canoa-host/AGENTS.md) | `canoa-host`: Canoa.cfg's `[host]`, its command line and library |

## Layout

| Path | Responsibility |
| --- | --- |
| `app/` | The Flutter chat client: web, desktop and the extension's UI |
| `crates/canoa/` | Registry, service descriptors and deny-by-default authorization, which the host uses |
| `crates/canoa-host/` | The host: serves stdio ACP agents and Guatiao providers from Canoa.cfg to paired keys, on loopback |
| `bindings/browser/chromium/` | MV3 browser extension code and permission policy |
| `examples/acp-echo/` | A paired WSS ACP v2 test endpoint, not an agent |
| `docs/` | Architecture and interoperability contracts; `docs/roadmap.md` is the one status table |
| `tools/` | Offline dependency mirroring |

## Project identity

- Publisher and primary domain: **Boriken Dev** (`boriken.dev`); GitHub organization `Boriken-Dev`.
- Use `dev.boriken.*` for reverse-DNS identifiers (Android application IDs, Apple bundle IDs, native messaging hosts). Canoa uses `dev.boriken.canoa` as its base identifier.
- Keep ecosystem-native package naming: Rust crates use hyphens (`coqui-web`), Dart packages underscores, and scoped npm packages must use a verified scope. Do not invent a claimed publishing scope.
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

## Commands

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features
CANOA_TEST_PYTHON=python3 cargo test -p canoa-host --test python_source   # a Python with its shared library; ignored without
(cd app && flutter analyze && flutter test)
node --test --test-force-exit "app/test/*.test.mjs"
(cd app && flutter test integration_test/desktop_test.dart -d windows)   # or -d linux; needs CANOA_TEST_HOST, CANOA_TEST_NODE, CANOA_TEST_APP
# the chat's connection code against the real host; needs Node 22 or later
cargo build -p canoa-host --bin canoa-host --examples
(cd app && CANOA_TEST_HOST=../target/debug/canoa-host CANOA_TEST_NODE=node flutter test test/host_test.dart)
(cd bindings/browser/chromium && npm test)
(cd examples/acp-echo && npm install && npm test)   # needs OpenSSL
mkdocs build --strict
```

`flutter pub get` with an older Flutter rewrites `app/pubspec.lock`: restore it before committing.

## CI and release

Checks run on a developer's own machines, by the commands above. `test.yml`, `flutter.yml`, `browser.yml`, `desktop.yml`,
`acp-reference.yml`, `offline.yml` and `deps.yml` run only when started by hand. `docs.yml` publishes Pages from `main`.
`release.yml` gates a `v*` tag, running `test.yml`, and publishes no package yet. Pushing a `v*` tag needs the owner's consent for that release.
