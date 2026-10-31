# coqui_net_transport (Dart binding prototype)

Transport-neutral `MessageHandoff` keeps one ordered message subscription while a higher-level authentication method (typically `coqui-peer`) consumes its initial handshake messages. After verification, any queued messages are delivered to ACP, MCP or another application protocol.

No keys, trust database, identity checks or authentication handshake live in this package. The application must call `handoff()` **only after** the selected authentication method verifies its expected peer. This includes WebTransport first with WebSocket fallback, but the actual WebTransport/WS dialers still live in Canoa pending their extraction into this library.

The package is not published; Canoa uses the workspace path dependency at `bindings/dart/coqui_net_transport`. Its standalone tests run via Canoa's Flutter suite.
