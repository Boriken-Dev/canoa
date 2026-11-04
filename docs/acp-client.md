# ACP v2 Flutter session controller

The shared Canoa Flutter interface has a tested **ACP v2 JSON-RPC client** and an explicit pinned-key connection dialog, using the experimental WebCrypto method from coqui-peer. The default PWA and extension chat start with offline previews. A remote connection works only when the agent endpoint implements the same method and ACP v2. There is no production pairing workflow or complete agent backend yet; see the [roadmap](roadmap.md).

## Components

| File | Responsibility |
| --- | --- |
| [coqui-web](https://github.com/Boriken-Dev/coqui-web) | Prefer WebTransport and fall back to an explicitly advertised WSS endpoint on transport-opening failure |
| [`app/lib/acp/connect.dart`](https://github.com/Boriken-Dev/canoa/blob/main/app/lib/acp/connect.dart) | Verify expected peer ID, initialize ACP v2 and close a rejected link without retrying on WSS |
| [coqui-acp](https://github.com/Boriken-Dev/coqui-acp) | Correlated JSON-RPC requests and updates, cancellation, session creation and resume, version 1 and 2 |
| [`app/lib/acp/client.dart`](https://github.com/Boriken-Dev/canoa/blob/main/app/lib/acp/client.dart) | No client until the peer is proven |
| [`app/lib/session_store.dart`](https://github.com/Boriken-Dev/canoa/blob/main/app/lib/session_store.dart) | Bind a verified agent to the correct local/remote session, stream transcript fragments and show foreground state |
| [`app/test/acp_v2_test.dart`](https://github.com/Boriken-Dev/canoa/blob/main/app/test/acp_v2_test.dart) | In-memory agent tests, peer-proof rejection, transport downgrade rules and message deduplication |

## ACP v2 semantics

After independently verified peer authentication, the client sends `initialize` using protocol version 2, `capabilities: {}`, and Boriken Dev's Canoa client information. It rejects agents not advertising v2 session support.

Session creation requires an absolute workspace path and sends `session/new`. Existing remote sessions can be attached via `session/resume`, optionally asking for a full transcript with `replayFrom: { "type": "start" }`.

`session/prompt` returns the **ID of an inserted user message**. This does not mean the agent has finished the turn. The UI consumes `session/update` notifications, including `agent_message_chunk`, full `agent_message` replacement, and `state_update` with `running`, `requires_action`, or `idle`. The agent-issued message ID is used to prevent duplicate transcript entries across chunk and full-message updates.

Every `sessionUpdate` kind in the v2 schema has a place on screen. A `plan_update` of `items` replaces the plan panel, a `tool_call_content_chunk` adds one item to its tool's card, a `notice` shows in the conversation as the agent's word with its severity, and `compaction_update` and `compaction_summary_chunk` build one entry per compaction with its summary folded away. A message or thought sent without `content` patches the one with its ID and leaves its text. A replayed user message's `resource`, `resource_link`, image or audio blocks are named in a notice under it, as a message sent with attachments is.

`session/cancel` is a notification, not an RPC expecting a turn-completion response. Agent completion is still delivered as a later idle state update.

## Peer authentication boundary

A verified transport must be produced by a callback that **cryptographically proves the expected peer identity** and answers with a channel tied to that proof, since a plain channel after a proof belongs to whoever carries it. A claimed `peerId` string, TLS hostname, or an unverified server hello is never sufficient. The ordinary ACP tests inject a fake verifier for protocol testing only. The browser method is now sourced from [coqui-peer](https://github.com/Boriken-Dev/coqui-peer/tree/main/methods/webcrypto), bundled unchanged with a source-blob hash test, and driven via a transport-level staged message queue.

Canoa's implementation intentionally **does not** connect the standard UI to unverified remote endpoints. A rejected peer proof closes the link and cannot trigger WebTransport-to-WebSocket downgrade. The native Rust responder and JavaScript↔Rust handshake tests now live in coqui-peer, with Coqui TrustStore-based acceptance and revocation. Canoa also has a [paired WSS ACP echo fixture](https://github.com/Boriken-Dev/canoa/tree/main/examples/acp-echo) that exercises a real TLS socket, Coqui proof and v2 session messages. Missing production work is agent-host pairing UI, persistent keys/sessions, production agent adapters and real remote MCP tools. The echo fixture does not execute or represent an AI agent.

The client negotiates: it requests v2 and accepts v1, whichever the agent selects. A permission request is refused until Canoa has an approval UI.

## Verification

```sh
cd app
flutter analyze
flutter test test/acp_v2_test.dart
flutter test
```

References: [ACP v2 initialization](https://agentclientprotocol.com/protocol/v2/initialization), [session setup](https://agentclientprotocol.com/protocol/v2/session-setup) and [v2 migration](https://agentclientprotocol.com/protocol/v2/migration).

## Message handoff

`coqui_web` owns the bounded queue of complete messages between connection setup and application use. Its `StagedChannel` holds early messages. Neither authenticates the peer or interprets the WebCrypto proof. The authenticated connection is handed to ACP **only after** Coqui succeeds.
