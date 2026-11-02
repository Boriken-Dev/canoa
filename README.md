# Canoa

[![Version](https://img.shields.io/badge/version-0.0.0--alpha.0-blue.svg)](https://github.com/Boriken-Dev/canoa/blob/main/Cargo.toml)
[![Rust version](https://img.shields.io/badge/rust-1.89%2B-orange.svg)](https://github.com/Boriken-Dev/canoa/blob/main/Cargo.toml)
[![License: MPL-2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/canoa/blob/main/LICENSE)
[![Docs](https://img.shields.io/badge/docs-online-blue.svg)](https://boriken-dev.github.io/canoa/)
[![Rust CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/canoa/test.yml?label=Rust%20CI)](https://github.com/Boriken-Dev/canoa/actions/workflows/test.yml)
[![Browser CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/canoa/browser.yml?label=Browser%20CI)](https://github.com/Boriken-Dev/canoa/actions/workflows/browser.yml)
[![Flutter CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/canoa/flutter.yml?label=Flutter%20CI)](https://github.com/Boriken-Dev/canoa/actions/workflows/flutter.yml)
[![Desktop CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/canoa/desktop.yml?label=Desktop%20CI)](https://github.com/Boriken-Dev/canoa/actions/workflows/desktop.yml)
[![Offline deps](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/canoa/offline.yml?label=Offline%20deps)](https://github.com/Boriken-Dev/canoa/actions/workflows/offline.yml)
[![ACP WSS CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/canoa/acp-reference.yml?label=ACP%20WSS)](https://github.com/Boriken-Dev/canoa/actions/workflows/acp-reference.yml)
[![Docs CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/canoa/docs.yml?label=Docs%20CI)](https://github.com/Boriken-Dev/canoa/actions/workflows/docs.yml)

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

The UI starts with offline preview sessions. A manually pinned Coqui peer connection can now be tested against [the WSS ACP echo fixture](https://github.com/Boriken-Dev/canoa/blob/main/examples/acp-echo/README.md), which is not a production agent. To package it for the Chromium extension, run `python app/tool/build.py chromium`; inspect [extension instructions](https://boriken-dev.github.io/canoa/browser/) before loading it as unpacked.

Canoa is one implementation on two shared libraries: [coqui-web](https://github.com/Boriken-Dev/coqui-web) for channels a browser can open, and [coqui-acp](https://github.com/Boriken-Dev/coqui-acp) for Agent Client Protocol sessions, version 1 and version 2.

The [project naming convention](https://boriken-dev.github.io/canoa/naming/) uses `boriken.dev` and reverse-DNS identifiers such as `dev.boriken.canoa` for application and native host identities.

## Desktop, configuration, and offline dependencies

The desktop app supports the AI6-style [Canoa.cfg](https://boriken-dev.github.io/canoa/configuration/) TOML file with a tray context menu (Open, Settings, Hide, Exit). On Windows and macOS two icon clicks open the app; Linux systems that do not forward tray clicks use **Open Canoa** in the menu. The [deps branch](https://github.com/Boriken-Dev/canoa/tree/deps) mirrors the Dart Pub cache and Unicode fallback CDN assets, populated by the [Refresh deps workflow](https://github.com/Boriken-Dev/canoa/actions/workflows/deps.yml).

The Chromium extension now has a local [MCP JSON-RPC tool adapter, options-based permission grants and elevated CDP command bridge](https://boriken-dev.github.io/canoa/browser-mcp/). Remote MCP/WSS exposure is still gated on an authenticated gateway and peer pairing UI.

## Development

Rust 1.89+:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features
(cd bindings/browser/chromium && npm test)
```

The [documentation](https://boriken-dev.github.io/canoa/) contains the [architecture](https://boriken-dev.github.io/canoa/architecture/), [interoperability profile](https://boriken-dev.github.io/canoa/interop/), [browser design](https://boriken-dev.github.io/canoa/browser/), and [roadmap](https://boriken-dev.github.io/canoa/roadmap/).

## License

MPL-2.0; see [LICENSE](https://github.com/Boriken-Dev/canoa/blob/main/LICENSE).
