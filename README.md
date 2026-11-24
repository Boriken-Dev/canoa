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
[![Docs CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/canoa/docs.yml?label=Docs%20CI)](https://github.com/Boriken-Dev/canoa/actions/workflows/docs.yml)

**Discover. Connect. Route.** A protocol-neutral gateway and multi-session client connecting applications to local and remote AI agents. Canoa is a standalone [Boriken Dev](https://boriken.dev) project, not tied to a particular agent provider, platform or browser. It aims to make [Agent Client Protocol (ACP)](https://agentclientprotocol.com/) peers accessible through preferred WebTransport (HTTPS) with WSS fallback and a documented peer-authentication profile.

*Canoa* is a historically attested Taíno word for canoe: a vessel carrying people and goods between places. Canoa carries agent sessions and tools between applications.

> **Status: early development.** The Rust crates are a deny-by-default registry (`canoa`), the host that serves agents to paired keys (`canoa-host`, the `canoa` program), that host as a library behind a C ABI (`canoa-ffi`) and the Windows portable launcher (`canoa-launch`). The Flutter app is a multi-session chat: the desktop app hosts and runs agents (Claude, Codex, Pi, any ACP agent) itself, and the web app and extension connect to a host over a verified ACP link; the Chromium extension offers permission-gated browser tools to agents over that link and validates site-adapter manifests (running an adapter is planned). **Pairing, `canoa serve` and the desktop app's hosting work and are tested; see the [roadmap](docs/roadmap.md) for what is not yet available.** No packages have been released.

## Features and direction

- **One chat app:** root `app/` contains a responsive Flutter client for full-page, sidebar, desktop, PWA and optional Wasm builds; the browser extension packages the same app.
- **Agent independent:** WebTransport is preferred; a secure WebSocket fallback is used only when opening it fails. Peer identity is verified separately.
- **Browser owned permissions:** approve access by verified peer, origin, tab and capability, with cancellation and revocation.
- **Bidirectional MCP:** agents can request approved DOM/browser tools and website-provided WebMCP; webpages can advertise MCP endpoints.
- **Developer debugging:** explicit elevated Chromium CDP mode; Firefox and Safari require separate adapters.
- **User site adapters:** attach JavaScript or Wasm tools to one or several exact website origins; manifest validation is implemented, runtime sandbox remains planned.
- **Multi-session:** one workspace for multiple ACP sessions, with v1 and v2 create/resume/prompt and streamed transcript handling over a verified peer link.

## Layout

| Path | Responsibility |
| --- | --- |
| `crates/canoa/` | Rust registry and authorization model |
| `crates/canoa-host/` | `canoa-host`: the `canoa` program, which serves the agents and providers canoa.cfg names to paired keys (loopback by default, or a pipe or socket), the web app, and `canoa portal` |
| `extensions/canoa-{claude,codex,grok,pi,antigravity,opencode}/` | One agent each: its source preset and Store recipe (and its coqui-acp provider where it has one) |
| `extensions/canoa-foundry/` | Foundry Local gateway (OpenAI and Anthropic APIs over a local model) and `foundry-local` account provider |
| `extensions/canoa-litellm/` | LiteLLM proxy account provider (one virtual key for OpenAI and Anthropic APIs, SSO or key, budget reporting) |
| `extensions/canoa-openai/` | OpenAI `account_speech` provider for accounts that hold an OpenAI API key |
| `extensions/canoa-whisper/` | whisper.cpp models as a Store type, and the `speech_provider` that runs them inside the host |
| `crates/canoa-ffi/` | The host as a library inside an application, behind a small C ABI |
| `crates/canoa-launch/` | The portable `CanoaUI.exe` launcher and `canoa-pack` |
| `bindings/python/canoa-sdk/`, `bindings/js/canoa-sdk/` | Python and JavaScript SDKs for writing Canoa providers (`UsageReader`, `ProfileItem`, `ProgramStore`, etc.) |
| `examples/providers/` | Example script providers per kind in Python and JavaScript |
| `app/native/` | The app's own plugin: builds `canoa-ffi` with the desktop runner |
| `tools/` | Offline dependency mirroring |
| `app/web_noise/` | `coqui-peer-noise` as WebAssembly for the web app's Noise method, built by `app/tool/build_noise.py` |
| `app/` | Canonical Flutter chat, canoa.cfg settings and desktop tray UI |
| `crates/canoa-host/examples/` | Test fixtures, built by `cargo build -p canoa-host --examples`: `stdio_agent` is an ACP agent to serve with `canoa serve` |
| `clients/browser/chromium/` | Chromium extension, browser permissions and tools |
| `clients/office/` | Office add-in manifests and task pane pages served by the host (`[host.web] office = true`) |
| `docs/` | Architecture, protocols, browser support and developer guides |

## Try the current UI

```sh
python app/tool/build.py desktop          # builds the desktop app and prints the path of CanoaUI
python app/tool/build.py desktop --run    # or runs it (needs flutter and cargo on PATH)
```

The desktop app carries the host as a library and lives in the tray. For the web app, `python app/tool/build_noise.py && (cd app && flutter build web --release --csp --no-web-resources-cdn)` then `canoa serve --web app/build/web`; see [getting started](https://boriken-dev.github.io/canoa/getting-started/). A manually pinned Coqui peer connection can also be tested against `canoa serve` with the `stdio_agent` example (`cargo build -p canoa-host --bin canoa --examples`, a canoa.cfg with `[host.agents.echo]` naming `target/debug/examples/stdio_agent` as its `program`, then `canoa accept` for the key that connects), which is not a production agent. To package the UI for the Chromium extension, run `python app/tool/build.py chromium`; inspect [extension instructions](https://boriken-dev.github.io/canoa/browser/) before loading it as unpacked.

Canoa is one implementation on two shared libraries: [coqui-transport](https://github.com/Boriken-Dev/coqui-transport) and [coqui-transport-web](https://github.com/Boriken-Dev/coqui-transport-web) for channels a browser can open, and [coqui-acp](https://github.com/Boriken-Dev/coqui-acp) for Agent Client Protocol sessions, version 1 and version 2.

## Desktop, configuration, and offline dependencies

The desktop app reads its settings from [canoa.cfg](https://boriken-dev.github.io/canoa/configuration/), one file in TOML, YAML or JSON chosen by its first line and read through [anycfg](https://github.com/jose-pr/anycfg-rs), and has a tray context menu (Open, Hosting, Settings, Hide, Exit). Closing the window keeps it and its agents running; a double click on the tray icon opens the chat on Windows, and Linux, whose tray icons pass no clicks on, uses **Open Canoa** in the menu. It runs `canoa` as its child, and its **Hosting** dialog adds agents and pairs browsers. The [deps branch](https://github.com/Boriken-Dev/canoa/tree/deps) mirrors the Dart Pub cache and Unicode fallback CDN assets, populated by the [Refresh deps workflow](https://github.com/Boriken-Dev/canoa/actions/workflows/deps.yml).

The Chromium extension now has a local [MCP JSON-RPC tool adapter, options-based permission grants and elevated CDP command bridge](https://boriken-dev.github.io/canoa/browser-mcp/). An agent of Canoa's own chat, and with a person's say an MCP client on a paired machine, reach those tools over a proven link with nothing listening in the browser: see [browser integration](https://boriken-dev.github.io/canoa/browser/).

## One file per platform

`python app/tool/build.py desktop --portable` makes the desktop app as one file: on Windows `target/canoa-windows/CanoaUI.exe`, a launcher
(`crates/canoa-launch`) that carries the built app folder, unpacks it once per version into `%LOCALAPPDATA%\Canoa\<version>\`, checks it
against a manifest of SHA-256 digests on every start and runs it; on Linux an AppImage. macOS ships `Canoa.app`. See
[one file per platform](https://boriken-dev.github.io/canoa/portable/) for the release recipe and what an unsigned file shows.

## The host

`canoa serve` offers each `[host]` entry of canoa.cfg, a stdio ACP agent or a Guatiao provider, to keys you paired, on loopback unless an entry's `listen` says otherwise. The desktop app starts it, starts it again if it ends, and ends it with its agents on quit. See [the host](https://boriken-dev.github.io/canoa/host/).

## Development

Rust 1.89+:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features
(cd clients/browser/chromium && npm test)
(cd clients/office && npm test)
(cd bindings/js/canoa-sdk && npm test && node tools/write_kinds.mjs --check)
(cd bindings/python/canoa-sdk && python tools/write_kinds.py --check)
```

The [documentation](https://boriken-dev.github.io/canoa/) contains the [architecture](https://boriken-dev.github.io/canoa/architecture/), [interoperability profile](https://boriken-dev.github.io/canoa/interop/), [browser design](https://boriken-dev.github.io/canoa/browser/), and [roadmap](https://boriken-dev.github.io/canoa/roadmap/).

## License

MPL-2.0; see [LICENSE](https://github.com/Boriken-Dev/canoa/blob/main/LICENSE). Third-party notices, including the Office.js library the Office add-in carries, are in [THIRD_PARTY_NOTICES.md](https://github.com/Boriken-Dev/canoa/blob/main/THIRD_PARTY_NOTICES.md).
