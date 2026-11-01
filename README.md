# Canoa

[![Version](https://img.shields.io/badge/version-0.0.0--alpha.0-blue.svg)](https://github.com/Boriken-Dev/Canoa/blob/main/Cargo.toml)
[![Rust version](https://img.shields.io/badge/rust-1.89%2B-orange.svg)](https://github.com/Boriken-Dev/Canoa/blob/main/Cargo.toml)
[![License: MPL-2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/Canoa/blob/main/LICENSE)
[![Docs](https://img.shields.io/badge/docs-online-blue.svg)](https://boriken-dev.github.io/Canoa/)
[![Rust CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/Canoa/test.yml?label=Rust%20CI)](https://github.com/Boriken-Dev/Canoa/actions/workflows/test.yml)
[![Browser CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/Canoa/browser.yml?label=Browser%20CI)](https://github.com/Boriken-Dev/Canoa/actions/workflows/browser.yml)
[![Flutter CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/Canoa/flutter.yml?label=Flutter%20CI)](https://github.com/Boriken-Dev/Canoa/actions/workflows/flutter.yml)
[![Desktop CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/Canoa/desktop.yml?label=Desktop%20CI)](https://github.com/Boriken-Dev/Canoa/actions/workflows/desktop.yml)
[![Offline deps](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/Canoa/offline.yml?label=Offline%20deps)](https://github.com/Boriken-Dev/Canoa/actions/workflows/offline.yml)
[![ACP WSS CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/Canoa/acp-reference.yml?label=ACP%20WSS)](https://github.com/Boriken-Dev/Canoa/actions/workflows/acp-reference.yml)
[![Docs CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/Canoa/docs.yml?label=Docs%20CI)](https://github.com/Boriken-Dev/Canoa/actions/workflows/docs.yml)

**Discover. Connect. Route.** A protocol-neutral gateway and multi-session client connecting applications to local and remote AI agents. Canoa is a standalone [Boriken Dev](https://boriken.dev) project, not tied to a particular agent provider, platform or browser. It aims to make [Agent Client Protocol (ACP)](https://agentclientprotocol.com/) peers accessible through preferred WebTransport (HTTPS) with WSS fallback and a documented peer-authentication profile.

*Canoa* is a historically attested Taíno word for canoe: a vessel carrying people and goods between places. Canoa carries agent sessions and tools between applications.

> **Status: early development.** The Rust core has an in-memory registry and deny-by-default policies. The Flutter app has a multi-session **offline preview** and a tested ACP v2 client for verified connections; the Chromium extension has a side-panel scaffold, local browser tool handlers, permission tests and site-adapter validation. **An authenticated WSS ACP v2 echo fixture is now integration-tested; production agent pairing, general-purpose live agent services, remote MCP and debugging transport are not yet available.** No packages have been released.

## Features and direction

- **One chat app:** root `app/` contains a responsive Flutter client for full-page, sidebar, desktop, PWA and optional Wasm builds; the browser extension packages the same app.
- **Agent independent:** WebTransport is preferred; a secure WebSocket fallback is used only when opening it fails. Peer identity is verified separately.
- **Browser owned permissions:** approve access by verified peer, origin, tab and capability, with cancellation and revocation.
- **Bidirectional MCP:** agents can request approved DOM/browser tools and website-provided WebMCP; webpages can advertise MCP endpoints.
- **Developer debugging:** explicit elevated Chromium CDP mode; Firefox and Safari require separate adapters.
- **User site adapters:** attach JavaScript or Wasm tools to one or several exact website origins; manifest validation is implemented, runtime sandbox remains planned.
- **Multi-session:** one workspace for multiple ACP sessions, with v2 create/resume/prompt and streamed transcript handling available after verified peer integration.

## Layout

| Path | Responsibility |
| --- | --- |
| `crates/canoa/` | Rust registry and authorization model |
| `crates/coqui-net-transport/` | Shared transport framing and fallback policy prototype |
| `bindings/dart/coqui_net_transport/` | Protocol-neutral queued message handoff during transport setup |
| `app/web/vendor/` | Pinned browser WebCrypto method sourced from `coqui-peer` |
| `app/` | Canonical Flutter chat, Canoa.cfg settings and desktop tray UI |
| `examples/acp-echo/` | Paired WSS ACP v2 echo test endpoint and integration tests |
| `bindings/browser/chromium/` | Chromium extension, browser permissions and tools |
| `bindings/` | Additional browser and language bindings |
| `docs/` | Architecture, protocols, browser support and developer guides |

## Try the current UI

```sh
cd app
flutter pub get
flutter run -d chrome
# Standalone web / Wasm builds:
flutter build web --release
flutter build web --release --wasm
```

The UI starts with offline preview sessions. A manually pinned Coqui peer connection can now be tested against [the WSS ACP echo fixture](https://github.com/Boriken-Dev/Canoa/blob/main/examples/acp-echo/README.md), which is not a production agent. To package it for the Chromium extension, run `python app/tool/build.py chromium`; inspect [extension instructions](https://boriken-dev.github.io/Canoa/browser/) before loading it as unpacked.

See the [shared `coqui-net-transport` proposal](https://boriken-dev.github.io/Canoa/coqui-net-transport/) for factoring WebTransport/WSS, peer verification and MTU-aware recovery across Canoa, Guatiao, AI6 and LineupMux.

The [project naming convention](https://boriken-dev.github.io/Canoa/naming/) uses `boriken.dev` and reverse-DNS identifiers such as `dev.boriken.canoa` for application and native host identities.

## Desktop, configuration, and offline dependencies

The desktop app supports the AI6-style [Canoa.cfg](https://boriken-dev.github.io/Canoa/configuration/) TOML file with a tray context menu (Open, Settings, Hide, Exit). On Windows and macOS two icon clicks open the app; Linux systems that do not forward tray clicks use **Open Canoa** in the menu. The [deps branch](https://github.com/Boriken-Dev/Canoa/tree/deps) mirrors the Dart Pub cache and Unicode fallback CDN assets, populated by the [Refresh deps workflow](https://github.com/Boriken-Dev/Canoa/actions/workflows/deps.yml).

The Chromium extension now has a local [MCP JSON-RPC tool adapter, options-based permission grants and elevated CDP command bridge](https://boriken-dev.github.io/Canoa/browser-mcp/). Remote MCP/WSS exposure is still gated on an authenticated gateway and peer pairing UI.

## Development

Rust 1.89+:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features
(cd bindings/browser/chromium && npm test)
```

The [documentation](https://boriken-dev.github.io/Canoa/) contains the [architecture](https://boriken-dev.github.io/Canoa/architecture/), [interoperability profile](https://boriken-dev.github.io/Canoa/interop/), [browser design](https://boriken-dev.github.io/Canoa/browser/), and [roadmap](https://boriken-dev.github.io/Canoa/roadmap/).

## License

MPL-2.0; see [LICENSE](https://github.com/Boriken-Dev/Canoa/blob/main/LICENSE).
