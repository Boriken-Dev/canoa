# ACP v2 Flutter session controller

The shared Canoa Flutter interface has a tested **ACP v2 JSON-RPC client**, but it ships an experimental pinned-key WebCrypto method from coqui-peer and an explicit connection dialog, but has a CI-tested native Rust Coqui responder and cross-language interoperability test, but still lacks a production pairing workflow and a complete agent backend. The default PWA/extension chat starts with offline previews. A remote connection can be attempted using the pinned-key dialog **only** when the agent endpoint implements the same experimental Coqui method and ACP v2.

## Components

| File | Responsibility |
| --- | --- |
| [`app/lib/transport/agent_transport.dart`](https://github.com/Boriken-Dev/Canoa/blob/main/app/lib/transport/agent_transport.dart) | Prefer WebTransport and fall back to an explicitly advertised WSS endpoint on transport-opening failure |
| [`app/lib/acp/connect.dart`](https://github.com/Boriken-Dev/Canoa/blob/main/app/lib/acp/connect.dart) | Verify expected peer ID, initialize ACP v2 and close a rejected link without retrying on WSS |
| [`app/lib/acp/client.dart`](https://github.com/Boriken-Dev/Canoa/blob/main/app/lib/acp/client.dart) | Correlated JSON-RPC requests and updates, cancellation, session creation and resume |
| [`app/lib/session_store.dart`](https://github.com/Boriken-Dev/Canoa/blob/main/app/lib/session_store.dart) | Bind a verified agent to the correct local/remote session, stream transcript fragments and show foreground state |
| [`app/test/acp_v2_test.dart`](https://github.com/Boriken-Dev/Canoa/blob/main/app/test/acp_v2_test.dart) | In-memory agent tests, peer-proof rejection, transport downgrade rules and message deduplication |

## ACP v2 semantics

After independently verified peer authentication, the client sends `initialize` using protocol version 2, `capabilities: {}`, and Boriken Dev's Canoa client information. It rejects agents not advertising v2 session support.

Session creation requires an absolute workspace path and sends `session/new`. Existing remote sessions can be attached via `session/resume`, optionally asking for a full transcript with `replayFrom: { "type": "start" }`.

`session/prompt` returns the **ID of an inserted user message**. This does not mean the agent has finished the turn. The UI consumes `session/update` notifications, including `agent_message_chunk`, full `agent_message` replacement, and `state_update` with `running`, `requires_action`, or `idle`. The agent-issued message ID is used to prevent duplicate transcript entries across chunk and full-message updates.

`session/cancel` is a notification, not an RPC expecting a turn-completion response. Agent completion is still delivered as a later idle state update.

## Peer authentication boundary

A verified transport must be produced by a callback that **cryptographically proves the expected peer identity**. A claimed `peerId` string, TLS hostname, or an unverified server hello is never sufficient. The ordinary ACP tests inject a fake verifier for protocol testing only. The browser method is now sourced from [coqui-peer](https://github.com/Boriken-Dev/coqui-peer/tree/main/methods/webcrypto), bundled unchanged with a source-blob hash test, and driven via a transport-level staged message queue.

Canoa's implementation intentionally **does not** connect the standard UI to unverified remote endpoints. A rejected peer proof closes the link and cannot trigger WebTransport-to-WebSocket downgrade. The native Rust responder and JavaScript↔Rust handshake tests now live in coqui-peer, with Coqui TrustStore-based acceptance and revocation. Canoa also has a [paired WSS ACP echo fixture](../examples/acp-echo/README.md) that exercises a real TLS socket, Coqui proof and v2 session messages. Missing production work is agent-host pairing UI, persistent keys/sessions, production agent adapters and real remote MCP tools. The echo fixture does not execute or represent an AI agent.

The ACP v2 controller is **not** a complete v1 compatibility implementation. It intentionally fails closed if an agent negotiates v1; a versioned adapter should be added rather than speaking v2 messages under a v1 negotiation.

## Verification

```sh
cd app
flutter analyze
flutter test test/acp_v2_test.dart
flutter test
```

References: [ACP v2 initialization](https://agentclientprotocol.com/protocol/v2/initialization), [session setup](https://agentclientprotocol.com/protocol/v2/session-setup) and [v2 migration](https://agentclientprotocol.com/protocol/v2/migration).

## Message handoff

`bindings/dart/coqui_net_transport` owns the bounded queue of complete messages between connection setup and application use. The `StagedAgentConnection` is a thin adapter from Canoa's transport interfaces. Neither component authenticates the peer or interprets the WebCrypto proof. The authenticated connection is handed to ACP **only after** Coqui succeeds.
