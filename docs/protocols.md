# Protocol adapters

ACP is the first supported agent protocol, but it is not the discovery or trust protocol.

The adapter should forward ACP JSON-RPC messages while respecting agent session ownership, lifecycle, cancellation and agent-originated permission requests. Support the stable transport(s) explicitly implemented by a given agent; do not advertise WebSocket or HTTP support merely because they exist as candidates.

Browser permissions to connect to an agent and ACP tool-execution permissions inside the agent are distinct layers. An authenticated browser grant must not silently approve ACP `session/request_permission` calls, nor expose filesystem/terminal access without explicit policy.

The core registry contains no ACP implementation. Protocol-specific adapters belong outside it.
