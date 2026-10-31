# Protocol roles

Canoa supports two distinct protocol directions: **ACP** for applications connecting to agents, and **MCP** for agents consuming tools and capabilities exposed by a browser or a website.

## ACP over WebSocket

ACP JSON-RPC is exchanged over WS/WSS according to the remote transport profile implemented by the agent. Canoa cannot presume that every ACP stdio agent also implements WebSocket. The endpoint advertises its ACP protocol version and supports a compatible WS profile. `wss://` is the production default; plaintext `ws://` requires an explicit, local-only developer opt-in.

Peer authentication is separate from ACP. Canoa aims to use `coqui-peer` semantics and a **documented interoperable, versioned WebSocket peer-auth profile**, not a secret Canoa token or an unspecified claim of identity. Since a browser cannot select a TLS client certificate or add arbitrary WebSocket headers, browser peer proof must be possible after WSS setup, before sending ACP messages.

## MCP: browser tools exposed to agents

The Chromium extension owns policy decisions for tab access. Browser tools such as `browser_read_page`, `browser_click`, `browser_fill`, and WebMCP discovery/invocation are provided only after a local browser grant. A compatible MCP server may forward such calls into the extension, but its approval is insufficient to override a browser denial.

ACP and MCP must not be confused: simply opening an ACP WebSocket does not advertise an MCP server to the agent. An agent must explicitly support the MCP transport used by Canoa, or negotiate a standardized MCP-over-ACP extension where available.

## Website-provided MCP

A webpage can register JS tools through WebMCP, or it can advertise an independently hosted MCP service. WebMCP is page-local, not an HTTP MCP server. Endpoint advertisements are only hints, subject to browser consent, endpoint verification, redirects/SSRF checks in the appropriate broker, and any remote MCP authentication.

## Sources

- https://agentclientprotocol.com/
- https://docs.rs/agent-client-protocol-http/
- https://modelcontextprotocol.io/
- https://github.com/webmachinelearning/webmcp
