# Protocol roles

Canoa supports two distinct protocol directions: **ACP** for applications connecting to agents, and **MCP** for agents consuming tools and capabilities exposed by a browser or a website.

## Where ACP runs

A `[host]` entry in canoa.cfg is a stdio ACP agent, a [Guatiao](https://github.com/Boriken-Dev/guatiao) provider or an `acp_harness` provider (Claude Code, Codex and Pi through their Rust translators), served to paired keys by [`canoa`](host.md). An entry listens on an address, on a Windows named pipe or Unix socket (`pipe:NAME`, `unix:PATH`) for a native client on the same machine, or through a transport provider (`transport = "<id>"`); the host's own code reaches its entries in process. The overlays a profile applies are read through the Agent Overlay Specification ([coqui-aos](https://github.com/Boriken-Dev/coqui-aos)). ACP versions 1 and 2 are both spoken, and delegation between sessions uses A2A tasks.

## ACP over WebTransport and WebSocket

ACP JSON-RPC uses Canoa's draft ordered-stream framing on WebTransport, or complete text messages over WSS, according to the remote transport profile the agent implements. Canoa cannot presume that every ACP stdio agent also implements WebSocket. The endpoint advertises its ACP protocol version and supports a compatible WS profile. `wss://` is the production default; plaintext `ws://` requires an explicit, local-only developer opt-in.

Peer authentication is separate from ACP. Canoa uses `coqui-peer` semantics and a **documented interoperable, versioned WebSocket peer-auth profile** (the WebCrypto and Noise methods), not a secret Canoa token or an unspecified claim of identity. Since a browser cannot select a TLS client certificate or add arbitrary WebSocket headers, browser peer proof must be possible after WSS setup, before sending ACP messages.

## ACP v2 Flutter session client

The Dart implementation in `app/lib/acp/client.dart` supports ACP **v2** (and accepts v1) `initialize`, `session/new`, `session/resume` (with optional replay), `session/prompt`, `session/cancel`, `session/close`, and session update notifications. It requests `protocolVersion: 2` and advertises only the capabilities it implements.

The v2 prompt response only acknowledges **message insertion**. Agent message chunks and `state_update` events report progress and completion; no uncertain prompt is replayed on disconnect. Incoming unauthorized client methods are rejected, and permission requests default to cancellation.

`connectVerifiedAcp` in `app/lib/acp/connect.dart` opens WebTransport with WSS fallback on transport setup failure, then calls a **trusted peer verifier** before ACP initialization. `connectBrowserAcp` in `app/lib/acp/browser_connect.dart` supplies the production one, which proves the pinned peer with the WebCrypto or Noise method; test fakes are not a user-facing authentication method. The client requests ACP v2 and accepts v1. See [the ACP client](acp-client.md).

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
