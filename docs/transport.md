# Canoa agent transport profile

Canoa prefers **WebTransport over HTTPS**, with **WebSocket over WSS** as its fallback. A direct WebSocket connection is also supported for a peer that does not advertise WebTransport.

This borrows Guatiao's separation of a message channel from a higher-level protocol, plus the browser/Dart-VM conditional WebTransport implementation. Unlike Guatiao's current `RemoteLink.connect`, Canoa actually attempts a fallback on **opening** failure.

## Endpoint descriptor

One logical agent peer advertises both URIs if available:

```json
{
  "webTransport": "https://agent.example:4433/acp",
  "webSocket": "wss://agent.example/acp"
}
```

These are transport locations, **not authentication evidence**. Both must resolve to the **same independently verified peer** after the connection is established. No peer is trusted because the host, IP address or display name matches.

## Wire framing

- **WebTransport** (a Canoa-specific draft profile): a single reliable bidirectional stream, `u32` little-endian length + UTF-8 JSON-RPC message. Maximum 1 MiB per message. Do not use unreliable datagrams for ACP commands or transcript updates.
- **WebSocket** (the experimental remote ACP transport profile): one complete JSON-RPC message per **text frame**, not Guatiao's binary message format.
- ACP initialization, session methods, notifications, cancellation, ordered updates and permissions are **identical** above the framing adapter.
- WebTransport is not an official upstream ACP transport yet. A Canoa-compatible agent needs to support this documented stream framing, and interoperability tests between independent implementations are still required.

## Fallback rules

1. Validate the advertised `https://` and `wss://` endpoints, disallow URL credentials and query tokens.
2. Try the WebTransport endpoint with a connection timeout; if unsupported, blocked, or unable to open a stream, close it and try WSS.
3. **Only fallback before peer authentication, ACP initialization or any session command.** A failed peer proof or a disconnected authenticated session is an error—not permission to silently downgrade to another transport.
4. Run the same coqui-peer-compatible verified identity handshake on the chosen channel. WSS certificate validation or a WebTransport browser certificate pin is not a substitute for that identity proof.
5. Resuming a lost agent session requires a separate explicit ACP reconnect/resume operation after a new authenticated connection. Never replay an uncertain prompt or action automatically.

Plain WS is available only with explicit local-development opt-in and a loopback address; production connections require encryption.

## Platform support

| Target | WebTransport | WebSocket |
| --- | --- | --- |
| Browser/Flutter Web (including Wasm builds) | Browser WebTransport via JS interop | Browser WebSocket |
| Native Flutter/Dart VM | Native backend pending; try fallback | Supported via web_socket_channel |
| Chrome/Edge side panel | Same Flutter web transport selection | Same WebSocket fallback |

The Canoa app transport classes are in `app/lib/transport/agent_transport.dart` and `contracts.dart`, including typed pre-auth fallback failures, testable dialers, and endpoint validation. A browser WebTransport endpoint must have an explicit port. The transport endpoints must share a host. A pinned certificate or protocol failure never silently downgrades to WebSocket. They establish a message channel **only**; an authenticated live ACP chat is not yet exposed until the versioned peer-authentication profile is implemented.

## References

- [Guatiao Dart remote implementation](https://github.com/Boriken-Dev/guatiao/tree/main/bindings/dart/guatiao/lib/src/remote)
- [Agent Client Protocol](https://agentclientprotocol.com/)
- [ACP Python SDK web transport (experimental)](https://agentclientprotocol.github.io/python-sdk/web-transport/)
- [WebTransport API](https://developer.mozilla.org/en-US/docs/Web/API/WebTransport_API)

## Coqui layering

The shared `coqui-net-transport` Dart binding owns a bounded, ordered queue during protocol setup, with no cryptographic decisions. Peer key verification is done by the experimental coqui-peer browser method, whose canonical source is outside the Canoa repository. Canoa uses a pinned, unmodified bundle copy and supplies a separate application permission decision.

The Canoa code that performs `hello/challenge/proof/accept` frame exchange is a transport adapter for the Coqui method; it is not a second authentication mechanism. Canoa's default client continues to fail closed when the peer key is unpinned or the remote method is unsupported.
