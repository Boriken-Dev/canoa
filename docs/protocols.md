# Protocol adapters

Canoa uses ACP primarily for applications interacting with agent services. MCP provides the reverse direction: agents use browser tools that Canoa exposes, as well as services hosted by websites.

## Agent Client Protocol

ACP is the agent session protocol, not a discovery, identity or trust system. ACP transports and agent permissions are negotiated by the adapter and service; the broker never forwards a privileged internal token or launch metadata supplied by an untrusted page.

## Model Context Protocol

The native Canoa broker will implement an MCP **server** using the standard `tools/list` and `tools/call` operations. Its client-facing transport must be stdio or authenticated Streamable HTTP, not Chrome's native messaging wire. Exposed browser tools will include browsing approved tabs, reading bounded page text, interacting with elements under a separate grant, listing/invoking page WebMCP tools, and enumerating endpoint advertisements.

The Chromium extension will speak a small **internal** JSON message contract over native messaging to the broker. That contract must not be confused with an MCP transport. Client origin, peer identity, session permissions, request timeouts, and denial handling are the broker's responsibilities.

## WebMCP

Chromium's experimental `document.modelContext` API allows pages to register client-side tools; it is not an HTTP server, and it does not expose MCP resources or prompts. Canoa will treat its output as untrusted data and will not silently invoke tools with side effects. Some versions of Chromium require developer flags or trials; feature detection is mandatory.

## MCP endpoint advertisement

Canoa initially accepts a `<link rel="mcp" href="/mcp">` hint (a Canoa-specific opt-in convention), along with published MCP discovery metadata when supported by the broker. Advertisements are never treated as credentials or as consent to connect.

References: https://modelcontextprotocol.io/specification ; https://github.com/webmachinelearning/webmcp ; https://github.com/modelcontextprotocol/ext-server-card
