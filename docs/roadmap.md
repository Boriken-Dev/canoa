# Roadmap and implementation status

This page distinguishes tested code from planned integration work.

| Component | Status |
| --- | --- |
| Rust in-memory registry and scoped authorization model | Implemented and tested |
| Chromium permission policy and ACP request option validation | Implemented and tested |
| Browser tool catalogue, local dispatcher and site-adapter manifest validator | Implemented as an **unwired scaffold** |
| Flutter multi-session standalone UI | Offline preview by default; tested verified ACP client binding and streaming transcript support |
| Packaged Flutter extension side panel and full page | Build script and manifest, not browser integration tested |
| Browser-to-chat user-selected page or text handoff | Implemented as local-only context attachment; no remote send |
| WebTransport-preferred / WSS fallback agent message transport | Implemented in Dart; wired to ACP v2 through an injected peer-verification callback |
| MTU/path-loss recovery decision and WSS redial with injected peer proof | Implemented as an isolated, tested transport prototype; live session supervision pending |
| ACP v2 initialize, create/resume, prompt, cancel, close and streaming updates | Implemented behind verified-channel interface, tested with simulated agent |
| Browser-compatible, production coqui-peer proof and pairing UI | Planned; no direct live connections are exposed by default |
| Native service discovery and optional stdio bridge | Planned |
| Browser-facing MCP server and remote session controller | Planned |
| CDP full-debugging mode and approvals | Planned |
| Actual JS/Wasm user site-adapter execution and sandbox | Planned |
| Firefox and Safari browser integrations | Planned |

No ACP interoperability, remote browser access or security guarantee is asserted for incomplete components.
