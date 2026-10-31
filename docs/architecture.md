# Architecture

Canoa is one implementation of an interoperable gateway pattern. Compatible ACP clients/servers and other browser bridges are first-class peers; nothing requires a central Canoa service.

## Interfaces

```text
Website  --(extension-controlled page grant)--> Browser extension (Canoa)
                                                           |
                                                 ACP JSON-RPC over WSS/WS
                                                 + verified peer authentication
                                                           |
                                          Any compatible ACP agent or gateway
                                                           |
                                               Agent's own session permissions

MCP agent/client  -- MCP --> compatible MCP provider / Canoa broker
                                      |
                          approved browser MCP tools
                                      |
                          Browser extension local policy
                                      |
                             user-approved tab
                                  |       |
                            DOM actions  WebMCP tools
```

Browsers may connect directly over WSS once they implement the peer-authentication profile and have a verified peer. A native host is an **optional adapter** for mDNS discovery, local stdio/pipe agents, and exposing browser MCP tools to native agents; it is not the sole permission authority or mandatory for WSS.

## Permission authorities

- **Browser extension:** final authority for tab disclosure, DOM read/write, webpage WebMCP invocation, webpage-to-agent exposure and prompts. Maintains its own scoped grants and revocation.
- **Peer handshake/trust:** verifies endpoint identity and key continuity before granting any session. `coqui-peer` provides useful primitives but does not make the browser peer-auth wire format an existing standard.
- **ACP agent:** independently manages sessions and its internal permissions.
- **MCP client/provider:** independent authorization and tool execution policy. A browser grant cannot pre-approve remote MCP tool calls.
- **Native gateway (if installed):** enforces its own peer and client restrictions but **cannot override** the browser's deny decision.

Trust requires all relevant parties to approve; an allow from one layer is not an allow from another.

## Browser and page tools

Canoa's Chromium implementation lives under `bindings/browser/chromium` and uses Manifest V3. Its local permission store is not shared with other browser implementations. For cross-browser interoperability, standardize the ACP transport and peer-auth proof, not permission database internals.

Browser-provided MCP tools may read bounded DOM text, inspect permitted tabs, interact with visible elements, and discover/invoke opt-in WebMCP tools. Site-provided MCP endpoints and WebMCP descriptions are untrusted discovery candidates and cannot grant themselves or an agent access.

A Chrome extension cannot directly browse DNS-SD/mDNS through general WebExtension APIs. It may use a native companion for discovery, but a manually entered WSS endpoint can work without one once peer authentication is implemented.

## Roadmap

1. Rust registry and deny-by-default rules (implemented).
2. Browser-local, peer/tab/origin/capability permission model (implemented as isolated policy module; integrated transport pending).
3. Browser MCP tool catalogue and local permission-gated dispatch (implemented as an unwired scaffold); browser UI and transport integration remain pending.
4. Browser-compatible versioned coqui-peer wire profile and independently verified WSS/WS ACP bridge (pending).
5. Native discovery adapter, agent-facing MCP service and optional browser tool relay (pending).
6. Independent implementation conformance tests and additional language bindings (pending).

See [interop.md](interop.md) for the minimum interoperability profile.
