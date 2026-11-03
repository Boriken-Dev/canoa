# Architecture

Canoa is one implementation of an interoperable gateway pattern. Compatible ACP clients/servers and other browser bridges are first-class peers; nothing requires a central Canoa service.

## Interfaces

```text
Website  --(extension-controlled page grant)--> Browser extension (Canoa)
                                                           |
                                                 ACP over WebTransport or WSS
                                                 + verified coqui-peer identity
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

Browsers can connect directly over WebTransport/WSS to an endpoint supporting the published **experimental Coqui WebCrypto method**, provided the server's peer key is pinned out-of-band and the server has separately approved the client's identity. The browser handshake is maintained in coqui-peer, not Canoa. A native host is an **optional adapter** for mDNS discovery, local stdio/pipe agents, and exposing browser MCP tools to native agents; it is not the sole permission authority or mandatory for WSS.

## Permission authorities

- **Browser extension:** final authority for tab disclosure, DOM read/write, webpage WebMCP invocation, webpage-to-agent exposure and prompts. Maintains its own scoped grants and revocation.
- **Peer handshake/trust:** [coqui-peer-noise](https://github.com/Boriken-Dev/coqui-peer-noise) or [coqui-peer-webcrypto](https://github.com/Boriken-Dev/coqui-peer-webcrypto), the method stored with the agent's key and never negotiated, proves the browser/client key and a pinned server key before ACP, and every ACP message after that is inside the session it ends in. The host separately owns pairing, revocation and authorization: [`canoa-host`](host.md) accepts a key a person compares, and a removed key loses its link.
- **ACP agent:** independently manages sessions and its internal permissions.
- **MCP client/provider:** independent authorization and tool execution policy. A browser grant cannot pre-approve remote MCP tool calls.
- **Native gateway (if installed):** enforces its own peer and client restrictions but **cannot override** the browser's deny decision.

Trust requires all relevant parties to approve; an allow from one layer is not an allow from another.

## Browser and page tools

Canoa's Chromium implementation lives under `bindings/browser/chromium` and uses Manifest V3. Its local permission store is not shared with other browser implementations. For cross-browser interoperability, standardize the ACP transport and peer-auth proof, not permission database internals.

Browser-provided MCP tools may read bounded DOM text, inspect permitted tabs, interact with visible elements, and discover/invoke opt-in WebMCP tools. Site-provided MCP endpoints and WebMCP descriptions are untrusted discovery candidates and cannot grant themselves or an agent access.

A Chrome extension cannot directly browse DNS-SD/mDNS through general WebExtension APIs. It asks `coqui-acp-bridge`'s native companion when the person chooses to look nearby, and an endpoint entered by hand works without one.

## Status

The [roadmap](roadmap.md) is the one record of what exists.

See [interop.md](interop.md) for the minimum interoperability profile.

## Authentication and transport layering

The sources of the browser's cryptographic handshakes are `Boriken-Dev/coqui-peer-noise` and `Boriken-Dev/coqui-peer-webcrypto`. Canoa bundles an **unchanged, Git-blob-pinned copy** of each so that the extension and PWA work offline and satisfy MV3 local-code restrictions. `app/web/canoa_peer_bridge.mjs` adapts this method to Canoa; it contains no independent cryptographic handshake.

The `coqui_web` Dart package owns pending-message storage across the pre-application phase, in `StagedChannel`. The coqui-peer method verifies the remote public key and informs Canoa of the verified peer ID; only then does the connection become available to ACP.

**Authentication is not pairing.** A pinned user-entered fingerprint is required in the connection dialog, and no unrecognized peer is auto-enrolled. A key that connected once waits on the host until a person pairs it, in the desktop app's Hosting dialog, which shows the whole id to compare, or with `canoa-host accept`; a native program can pair by a PIN the host shows. The desktop app runs the host as its child. coqui-peer holds the native Rust responder.
