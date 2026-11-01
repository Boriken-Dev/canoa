# coqui-net-transport

**Reliable, ordered network transports — independent of application protocols.**

This provisional Rust crate lives in Canoa's workspace while its contract is tested and prepared for extraction to an independent `Boriken-Dev/coqui-net-transport` repository. It has **not** been published to crates.io.

It currently implements bounded little-endian stream message framing and a pure transport-recovery policy. Network adapters, WebTransport/QUIC, WebSocket engines, authenticated liveness probes, and cross-language bindings are not implemented by the Rust crate.

The policy distinguishes a missing/blocked QUIC path, timeout, failed forward progress or suspected PMTU black hole from certificate rejection, peer-proof failure or application rejection. It may permit WSS fallback to an independently authenticated peer, and after application traffic it explicitly requires application-level reconciliation, never blind replay of prompts.

`coqui-peer` remains responsible for peer identity, invitations and trust. ACP, MCP, Guatiao's value wire protocol and AI6/LineupMux session semantics remain in their respective applications.

See [the shared transport design](https://boriken-dev.github.io/Canoa/coqui-net-transport/).
