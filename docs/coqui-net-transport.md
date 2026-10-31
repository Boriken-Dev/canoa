# Shared reliable links: proposed `coqui-net-transport`

Canoa's transport selection and the experience in [Guatiao](https://github.com/Boriken-Dev/guatiao), [AI6](https://github.com/jose-pr/ai6) and LineupMux make a case for a reusable, language-independent connection layer named **`coqui-net-transport`**. This page is a design proposal, **not an existing crate or released wire standard**.

`coqui-serial` already exists for local COM/tty ports, TCP serial consoles and serial-over-WebSocket. Use **net-transport** because the library selects WebTransport, WebSocket and transport fallback; it is not a hardware serial driver. The name follows Boriken Dev's existing `coqui-net-acl` convention.

## Repository review

| Repository | Existing capability | Reusable part |
| --- | --- | --- |
| **Guatiao** | Rust `quinn` QUIC, browser Dart/JS WebTransport, WebSocket and length-framed bidirectional streams; peer TLS via `coqui-peer-tls` | Message-channel contracts, transport adapters, framing, deadlines, cryptographic identity integration |
| **AI6** | Guatiao frames over authenticated WebSocket for terminal attachment; reconnect with event sequence cursor, bounded queues and read/write leases | Resume cursors, explicit permissions, recoverable session state |
| **LineupMux** | Provider/source networking and adaptive relay bandwidth; linked browser ↔ app remote via an HTTP mailbox | Network state/availability, application reconnect and user-granted control; no separate WebTransport stack was found |
| **Canoa** | Dart WebTransport-first, WSS-fallback connector; ACP framing and endpoint validation | Protocol-neutral transport selection; active-link recovery policy |

LineupMux's relay adaptation changes video bandwidth and stream quality, not QUIC MTU. It is not a drop-in path-MTU mechanism.

## Proposed boundaries

```text
ACP / MCP / Guatiao / streaming / other applications
                  |
      application session + resume adapter
                  |
       coqui-net-transport  (proposed shared crate)
   ordered reliable channel, path health, dial
   WebTransport preferred, WSS fallback
                  |
   coqui-peer  +  coqui-peer-tls
   independent peer identity, pairing, trust
                  |
    QUIC/WebTransport  |  TLS WebSocket
```

The Rust crate would define a small sans-runtime `Link` / `Connector` contract, explicit `EndpointSet`, `LinkError` classes and `FailoverPolicy`. Native Rust feature-gated drivers could use Quinn/rustls and tungstenite; browser bindings in Dart/JS must use the browser's WebTransport and WebSocket APIs. Do not assume a Rust TCP implementation can simply be compiled to browser Wasm.

Keep the wire profile protocol-agnostic: `coqui-net-transport` offers **reliable, ordered message boundaries** or a byte-stream abstraction; ACP JSON-RPC text frames, Guatiao binary frames and media remain the applications' responsibility. Do not introduce a global proprietary transport handshake unless peers explicitly negotiate its version. An authenticated `coqui-peer` handshake gates application traffic, but a socket's TLS alone does not prove an invited peer identity.

## Transport choice and runtime fallback

1. Dial the expressly advertised `https://` WebTransport endpoint first. Prefer one reliable bidirectional stream.
2. On unsupported WebTransport, timeout, QUIC path failure or a suspect packet-loss/MTU blackhole, use the *explicitly advertised* `wss://` fallback. Never synthesize a fallback hostname or infer peer identity from DNS.
3. If the established WebTransport link later stops making forward progress, a session supervisor may open WSS, but **must reauthenticate the same previously trusted peer** before delivering any agent message.
4. An agent permission rejection, peer-key mismatch, TLS/certificate failure positively identified as such, or malformed negotiated protocol is **not** a transport problem and must not trigger downgrade.
5. Stop sending on the failed path. Reconnect and **reconcile** the old application session: ACP load/resume if supported, AI6 cursor replay with sequence numbers, Guatiao's own link rules, or media re-subscription. In-flight non-idempotent actions are of *unknown outcome* unless the application provides acknowledgments/idempotency keys; never resend them automatically.
6. Track path failures for a bounded cooldown per endpoint/network. Optional return to WebTransport should happen only after a new authenticated channel and an application-defined safe handoff (not while commands are outstanding).

### MTU and loss

QUIC requires a path supporting at least **1200 bytes of UDP payload** (RFC 9000 §14). If the path cannot carry that, QUIC must stop using it; connection to WebTransport will fail or stall even though TLS/WebSocket over TCP might work.

Browser JavaScript generally cannot distinguish a QUIC packet-blackhole, an unsupported HTTP/3 route or an MTU error from a generic connection failure. `coqui-net-transport` must therefore accept **unavailable / timed out / no forward progress / suspected MTU** as recoverable *transport* signals. For the usual low-MTU path (<1200 bytes of UDP payload), WebTransport may fail or time out *before opening a stream*; an ordinary bounded dial timeout triggers WSS fallback without a special MTU probe. A client may also use bounded end-to-end liveness checks on a verified channel rather than guessing MTU from the sizes of ACP messages. Idle sessions are not "stalled" merely because nobody has typed; a negotiated ping/pong or outstanding-operation deadline is needed to detect failure.

Do not weaken certificate pinning to cure an opaque browser error: only fall back to a previously configured WSS endpoint that can independently prove the same accepted `coqui-peer` identity. If the runtime cannot tell a security failure from a path failure, the policy must be explicit and conservative, or prompt the user. The current Canoa transport still fails closed for ambiguous pinned WebTransport errors; further work is needed once the browser-compatible peer proof exists.

## Draft API surfaces

- `EndpointSet`: explicit WebTransport and WSS URLs, advertised peer ID, certificate policy and supported protocols.
- `Link::send` / `Link::receive` / `Link::close`: bounded, ordered delivery with backpressure, not a guarantee of remote application execution.
- `PathHealth`: timers, missed authenticated liveness probes, transport closure and optional native QUIC path diagnostics.
- `Connector::dial`: prefer WebTransport, fallback to WSS on recoverable opening failures.
- `Connector::recover`: open alternate transport, verify *same* peer, return `RecoveryRequired` with old session identifier; no hidden automatic command replay.
- `LinkError`: distinct `Unavailable`, `PathLost`, `NoProgress`, `Security`, `PeerRejected`, `Protocol` and `Cancelled` cases.
- `ResumePolicy` is **not** part of `coqui-net-transport`; each application owns its own session semantic and idempotency contract.

Canoa implements a [pure transport-recovery policy](https://github.com/Boriken-Dev/Canoa/blob/main/app/lib/transport/recovery_policy.dart) with tests as an interim prototype. It intentionally does not claim to implement a browser peer-proof handshake, production liveness probes or recovered ACP sessions.

## Naming and project identity

The source organization is **Boriken Dev** (`boriken.dev`); reverse-DNS identifiers use `dev.boriken.*` (for example `dev.boriken.canoa`). Rust package names use hyphens (`coqui-net-transport`) rather than reverse-DNS notation. See [naming conventions](naming.md).

## Extraction sequence

1. Complete and test the transport-neutral contract in Canoa and Guatiao without importing ACP session semantics.
2. Create the independent **Boriken-Dev/coqui-net-transport** repository and Rust crate when the API stabilizes. Keep Rust 2024, minimum Rust 1.89, and match Boriken Dev's licensing policy.
3. Move Rust QUIC/WSS drivers behind optional features; add Dart and JavaScript bindings with browser-native WebTransport rather than shipping an unsuitable native-Wasm socket.
4. Test a real server on both endpoints, including synthetic UDP/QUIC blackholes, path changes, a WSS fallback, mismatched identities, retransmission ambiguity and recovery without duplicate writes.
5. Migrate Guatiao/Canoa consumers, then selectively AI6 and LineupMux where they benefit. Do not force unrelated app-specific streaming and terminal behavior into the shared crate.

## References

- [Guatiao remote links](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/dart/guatiao/lib/src/remote/link.dart)
- [AI6 session connection](https://github.com/jose-pr/ai6/blob/main/clients/outer-office/lib/terminal/session_connection.dart)
- [RFC 9000 §14 — QUIC datagram size and path MTU](https://www.rfc-editor.org/rfc/rfc9000#section-14)
