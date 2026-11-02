# Canoa

**Discover. Connect. Route.**

Canoa provides a common place to discover, authenticate and interact with local or remote agent services, without binding users to a single provider or client. It includes a responsive Flutter multi-session interface, browser-specific bindings that can offer authorized page/DevTools functionality through MCP, and a small Rust registry for a planned native host.

!!! warning "Development status"
    Early development: nothing here implies a published binary or public service. The [roadmap](roadmap.md) is the one record of what exists.

## Start here

- [Getting started](getting-started.md) to build the current Rust core or preview the Flutter interface.
- [Architecture](architecture.md) explains peer identity, browser authority and tool permission boundaries.
- [Interoperability](interop.md) and [Transport selection](transport.md) explain preferred WebTransport, WSS fallback and peer verification.
- [Browser integration](browser.md) describes the Chromium extension, side panel, full-page UI and future cross-browser adapters.
- [Site adapters](site-adapters.md) describe user-installed JavaScript and Wasm MCP tools for websites.
- [Roadmap](roadmap.md) distinguishes working components from planned transports.

## Scope

Canoa is **not** a new agent wire protocol. ACP is the client/agent interface; MCP is the tool interface, and coqui-peer supplies reusable identity and pairing primitives. Implementations may differ provided they agree on a published wire profile and authentication contract.

See the [source repository](https://github.com/Boriken-Dev/canoa) or the [Rust API](https://boriken-dev.github.io/canoa/rust/canoa/index.html).
