# Canoa interoperability profile

Canoa is one implementation of a browser-to-agent gateway model. A compatible implementation does not need to run Canoa or use its libraries.

## Minimum remote agent interface

1. **Agent Client Protocol (ACP)** is the application protocol. Use the published ACP lifecycle and JSON-RPC methods, not Canoa-specific method replacements.
2. **WebTransport over HTTPS** is preferred, with **WebSocket over WSS** as fallback. WebTransport uses a Canoa draft profile with an ordered bidirectional stream and little-endian length-prefixed UTF-8 JSON-RPC messages; WebSocket carries complete JSON-RPC text frames. WebTransport is not yet an upstream ACP standard transport; both addresses must be advertised and bound to the same verified peer. See [transport selection](transport.md).
3. **Authenticated peer identity** must be verified **before** an ACP session is exposed. The `coqui-peer` family is the reference implementation for peer identity, pairing and trust, but merely claiming a `PeerId` string is not proof. A conforming non-Rust implementation needs the same documented on-wire proof and key-rotation semantics; using the Rust crate is not required. A versioned browser-compatible peer-auth profile for WebSockets still needs to be specified and tested against at least two independent implementations.
4. **Capabilities** are negotiated by ACP. Browser tools are exposed as MCP only when the remote agent can consume them through a supported MCP connection or an explicitly negotiated ACP/MCP bridge. The MCP-over-ACP extension is currently experimental, not assumed mandatory.
5. **Local policy** is owned by each client implementation. There is no global Canoa permission authority.

## Two independent permissions

### Client -> agent

The caller, including a webpage represented by the extension, must be allowed to access the verified agent and requested ACP capabilities. The browser decides whether the *page* may use the connection, and a remote agent separately decides what the session may do.

### Agent -> browser

The browser extension owns grants for agents to see or interact with browser tabs. It MUST decide locally for every tool call. A native broker cannot bypass a browser refusal or silently broaden a grant.

A grant is scoped to at least:
- the **verified peer identity** (not a server-supplied display name);
- the **browser tab and committed document origin** (and preferably the document identity);
- the **capability**, e.g. `browser.read`, `browser.interact`, `webmcp.invoke`;
- an expiry/lifetime and revocation state.

Default deny. Navigation or tab closure revokes tab-scoped grants. An authenticated peer never implicitly receives browser access, and approval to read a page never permits actions that modify it.

Websites can register WebMCP tools or advertise independent remote MCP endpoints. Both are *untrusted descriptions*: publication is not user permission and must not automatically establish a connection. Browser-to-site permissions, agent-to-browser permissions and agent-side tool approval are separate decisions.

## Browser WebSocket, WebTransport and peer-auth constraints

Browsers do not offer a general API for arbitrary TLS client-certificate selection or custom request headers in the WebSocket constructor. The browser-compatible authentication profile must work at the application layer **after WebTransport/WSS establishment, before ACP messages**, or through a trusted local companion with verified peer identity. WSS establishes server certificate validation, not peer pairing by itself.

Do not send long-lived tokens in URL query strings; they can leak through logs. A challenge/response handshake may use the coqui-peer peer-proof and pairing primitives, but its wire framing and state machine must be published, versioned and interoperability-tested before being called a standard.

Implementations may discover endpoints through DNS-SD or manual pairing. Discovery is not a trust proof.

## Conformance tests (planned)

- ACP `initialize`, session creation, notifications, cancellation and disconnect/reconnect behavior over WS/WSS.
- Peer proof, pairing, wrong-key rejection, key rotation, revoked identity and replay resistance between independent implementations.
- Local browser denial of unauthorized origins, capabilities, peers, tabs and navigated documents.
- No implicit privilege escalation from a page-advertised MCP endpoint or WebMCP tool.
- Browser/grant denial continues to apply when native gateway approves or the server requests broader permissions.

## References

- https://agentclientprotocol.com/
- https://docs.rs/agent-client-protocol-http/
- https://modelcontextprotocol.io/
- https://github.com/Boriken-Dev/coqui-peer
