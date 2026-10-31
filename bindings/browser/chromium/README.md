# Canoa for Chromium

Canoa is a Manifest V3 browser extension named **Canoa**, under `bindings/browser/chromium/`.

## Implemented building blocks

- `permissions.js` and `permissions.test.mjs`: browser-owned, deny-by-default peer/tab/origin/capability grant decisions, expiry and revocation.
- `acp-permissions.js` and its tests: valid ACP `session/request_permission` choices, including standard selected/cancelled results; never fabricate an option.
- `tools.js`: MCP tool catalogue (browser tab list, page snapshot, click, fill, WebMCP discovery/invocation and advertised site MCP endpoints).
- `browser-tools.js`: local dispatcher for those operations. It must receive a cryptographically verified peer from a trusted caller, and checks the extension's grants before any tab action. The adapter does not yet connect it to an MCP server.
- `manifest.json` / `background.js`: MV3 skeleton, ephemeral browser-local grant storage and tab navigation revocation. No external webpage messages.

## Still required before release

A working popup/approval UI, authenticated ACP WebSocket client and its interoperable coqui-peer browser handshake, an MCP provider/transport for agents, browser integration wiring, native discovery adapter (optional), automated browser integration tests and distribution packaging.

Without a paired peer and user-consent UI, the extension intentionally exposes no connected agent session and cannot yet serve MCP tools.

## Security boundary

The browser independently decides permission requests for its own tabs and WebMCP tools, and how to respond to ACP agent `session/request_permission`. A remote ACP peer's grant is not permission to read browser tabs. A browser's approval cannot override the agent's own enforcement.

See [architecture](../../../docs/architecture.md), [interop](../../../docs/interop.md) and [ACP permissions](../../../docs/acp-permissions.md).
