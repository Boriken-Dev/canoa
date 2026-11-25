# Developers

These pages are for people who build {{ name }}, run its host on a server, or write providers, agents and clients for it. If you only want to use the app, start with the [Quick Start](index.md) and the user guide.

| Topic | Pages |
| --- | --- |
| Build it | [Getting started](getting-started.md), [One file per platform](portable.md), [Offline deps branch](offline-deps.md) |
| How it fits together | [Architecture](architecture.md), [ACP and MCP](protocols.md), [Interoperability](interop.md), [Transport selection](transport.md), [ACP v2 Flutter client](acp-client.md) |
| Run and configure | [The host](host.md), [Desktop and configuration](configuration.md), [The portal](portal.md), [Words, symbols and brands](themes_and_brands.md) |
| Extend it | [Providers written as scripts](script_providers.md), [Views from agents and overlays](a2ui_views.md), [User site adapters](site-adapters.md) |
| The browser side | [Browser integration](browser.md), [Browser MCP and DevTools](browser-mcp.md), [ACP permission requests](acp-permissions.md), [Browser comparisons](browser-comparison.md) |
| What exists | [Roadmap](roadmap.md), [Rust API](https://boriken-dev.github.io/canoa/rust/canoa/index.html), [the source](https://github.com/Boriken-Dev/canoa) |

{{ name }} is **not** a new agent wire protocol. ACP is the client and agent interface; MCP is the tool interface, and coqui-peer supplies reusable identity and pairing primitives. Implementations may differ provided they agree on a published wire profile and authentication contract.
