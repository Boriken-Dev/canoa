# Architecture

Canoa connects native applications and browser clients to agent services, and also makes approved browser capabilities available to agents as MCP tools.

## Core flow: application to agent

1. Discover candidates via DNS-SD/mDNS or manual remote configuration.
2. Authenticate a peer and resolve its trusted identity through coqui-peer.
3. Match the requesting principal to an explicit capability grant.
4. Select a protocol adapter (ACP first), initiate or attach to a session and route frames.
5. Revocation affects future requests and can close active routed sessions.

The Rust `canoa` crate currently contains only an in-memory decision model. Its caller-supplied `verified` boolean is **not** a network security boundary.

## Reverse flow: agent to browser (MCP)

```text
MCP client / agent
      |
authenticated MCP connection (stdio or protected Streamable HTTP)
      |
Canoa native broker  [MCP server: tools/list, tools/call]
      |
Chrome native messaging (length-prefixed JSON, extension-ID allowlist)
      |
bindings/browser/chromium/ (MV3 extension)
      |
approved browser tab
      |---------------- DOM snapshot / click / fill
      |---------------- WebMCP tools (document.modelContext)
      |---------------- advertised site MCP endpoint (discovery only)
```

The broker authenticates MCP callers and owns the permission and origin checks before forwarding any requests. The extension validates the tab and the user's read/interaction grants again. No agent or webpage receives raw broker or agent credentials.

The Chromium code is a **client scaffold**. Its native host is not yet implemented, so agents cannot currently connect. Native messaging is not itself MCP.

### Browser permissions

- Canoa uses `activeTab` to obtain temporary access when the user clicks its toolbar action.
- The popup explicitly approves read-only or interaction access to the active tab.
- Grants are per tab and origin, stored in browser session storage and revoked on navigation, tab closure or explicit user action.
- Only an extension-owned service worker accepts messages from the native host. No externally-connectable web message channel is registered.
- The agent's tool permissions and Canoa's browser grants remain separate; read permission does not authorize writes.
- Never assume web-supplied tool descriptions or HTML are instructions. Treat results as untrusted content.

### Page-owned MCP services

Two different kinds of page publishing are supported by the design:

1. **WebMCP:** pages register JavaScript tools with `document.modelContext.registerTool()`; Canoa may list/call those tools within an approved tab using the experimental API.
2. **Remote MCP:** sites may advertise a separately hosted MCP server. Canoa recognizes opt-in `<link rel="mcp" href="/mcp">` as a **Canoa-specific convention**, and in a future broker phase may consult the evolving MCP Server Card / AI Catalog conventions.

Advertisements are candidates, not authenticated connections. No advertisement is automatically fetched or connected from the extension. The native broker will enforce HTTPS, endpoint identity, redirect and SSRF rules, and user approval, then perform MCP OAuth if needed.

## Implementation milestones

1. Rust in-memory registry, deny-by-default grants and CI (implemented).
2. Chromium MV3 scaffold: tab consent, bounded DOM tools, WebMCP discovery/invocation, advertised endpoint enumeration (implemented as a native-host client; broker pending).
3. Native host and authenticated MCP tool server with lifecycle, timeouts and per-caller policy (pending).
4. DNS-SD/mDNS and manual endpoint inventory, identity proof and pairing through coqui-peer (pending).
5. ACP session routing and remote MCP clients (pending).
6. More language bindings and browser targets as needed.
