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

**Discover. Connect. Route.** Canoa is one chat window for the coding agents on your computer and the ones you reach elsewhere: Claude, Codex, Pi and any agent that speaks the [Agent Client Protocol (ACP)](https://agentclientprotocol.com/). You choose what each agent may do, and it asks before it does more. It also runs in a browser tab, in a browser extension's side panel and beside a mail or a document in Office. A [Boriken Dev](https://boriken.dev) project, not tied to a particular agent provider, platform or browser.

![The Canoa window with a session waiting for an answer](docs/assets/screens/window-permission-dark.png#gh-dark-mode-only)
![The Canoa window with a session waiting for an answer](docs/assets/screens/window-permission-light.png#gh-light-mode-only)

## Quick Start

**Download:** no release has been published yet, so there is no file to download. Build it from source (below, or [Getting started](https://boriken-dev.github.io/canoa/getting-started/)); the [releases page](https://github.com/Boriken-Dev/canoa/releases) will list the files of each release once there is one, and the [documentation's Quick Start](https://boriken-dev.github.io/canoa/) offers the one for your system.

1. **Get Canoa.** `python app/tool/build.py desktop --portable` makes one file: `CanoaUI.exe` on Windows, an AppImage on Linux (needs `flutter` and `cargo`).
2. **Open it and sign an agent in.** The window lists the agents it found on this computer; add an account under **Settings, Accounts** if an agent needs one.
3. **Start a session.** Press **Ctrl+N**, choose **Workspace** and a folder, and type what you want. When the agent wants to change a file or run a command, a card asks you: **Yes** lets it, **Deny** refuses.

<p>
<img src="docs/assets/screens/window-new-session-phone-dark.png#gh-dark-mode-only" alt="A new session at phone width" width="260">
<img src="docs/assets/screens/window-new-session-phone-light.png#gh-light-mode-only" alt="A new session at phone width" width="260">
<img src="docs/assets/screens/window-permission-phone-dark.png#gh-dark-mode-only" alt="A permission card at phone width" width="260">
<img src="docs/assets/screens/window-permission-phone-light.png#gh-light-mode-only" alt="A permission card at phone width" width="260">
</p>

The [user guide](https://boriken-dev.github.io/canoa/guide/window/) walks through the window, [permissions](https://boriken-dev.github.io/canoa/guide/permissions/), [the Store](https://boriken-dev.github.io/canoa/guide/store/), [accounts](https://boriken-dev.github.io/canoa/guide/accounts/), [settings](https://boriken-dev.github.io/canoa/guide/settings/), [the browser extension](https://boriken-dev.github.io/canoa/guide/browser-extension/), [the Office add-in](https://boriken-dev.github.io/canoa/guide/office-addin/) and [the web app and the portal](https://boriken-dev.github.io/canoa/guide/web-and-portal/), each with a worked example.

> **Status: early development.** Pairing, `canoa serve` and the desktop app's hosting work and are tested; see the [roadmap](docs/roadmap.md) for what is not yet available. No packages have been released.

*Canoa* is a historically attested Taíno word for canoe: a vessel carrying people and goods between places. Canoa carries agent sessions and tools between applications.

## Developers

Canoa is one implementation on shared libraries: [coqui-transport](https://github.com/Boriken-Dev/coqui-transport) and [coqui-transport-web](https://github.com/Boriken-Dev/coqui-transport-web) for channels a browser can open, and [coqui-acp](https://github.com/Boriken-Dev/coqui-acp) for Agent Client Protocol sessions, version 1 and version 2. WebTransport is preferred, with a secure WebSocket fallback only when opening it fails, and peer identity is verified separately. The Rust workspace holds a deny-by-default registry (`canoa`), the host (`canoa-host`, the `canoa` program), that host as a library behind a C ABI (`canoa-ffi`) and the portable Windows launcher (`canoa-launch`).

The [developer documentation](https://boriken-dev.github.io/canoa/developers/) covers the [architecture](https://boriken-dev.github.io/canoa/architecture/), the [interoperability profile](https://boriken-dev.github.io/canoa/interop/), [building and running the host](https://boriken-dev.github.io/canoa/host/), [canoa.cfg](https://boriken-dev.github.io/canoa/configuration/), [one file per platform](https://boriken-dev.github.io/canoa/portable/), [browser integration](https://boriken-dev.github.io/canoa/browser/), the Python and JavaScript SDKs for providers, the [offline deps branch](https://boriken-dev.github.io/canoa/offline-deps/) and the [roadmap](https://boriken-dev.github.io/canoa/roadmap/). The Rust API is at <https://boriken-dev.github.io/canoa/rust/canoa/index.html>.


### Layout

| Path | Responsibility |
| --- | --- |
| `crates/canoa/` | Rust registry and authorization model |
| `crates/canoa-host/` | `canoa-host`: the `canoa` program, which serves the agents and providers canoa.cfg names to paired keys (loopback by default, or a pipe or socket), the web app, and `canoa portal` |
| `extensions/canoa-{claude,codex,grok,pi,antigravity,opencode}/` | One agent each: its source preset and Store recipe (and its coqui-acp provider where it has one) |
| `extensions/canoa-foundry/` | Foundry Local gateway (OpenAI and Anthropic APIs over a local model) and `foundry-local` account provider |
| `extensions/canoa-litellm/` | LiteLLM proxy account provider (one virtual key for OpenAI and Anthropic APIs, SSO or key, budget reporting) |
| `extensions/canoa-openai/` | OpenAI `account_speech` provider for accounts that hold an OpenAI API key |
| `extensions/canoa-whispercpp/` | whisper.cpp models as a Store type, and the `speech_provider` that runs them inside the host |
| `extensions/canoa-candle/` | Whisper in pure Rust (Candle): its models as a Store type, and the `speech_provider` that runs them inside the host |
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

### Checks

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
