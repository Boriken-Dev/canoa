# Roadmap and implementation status

This page distinguishes tested code from planned integration work.

| Component | Status |
| --- | --- |
| Rust in-memory registry and scoped authorization model | Implemented and tested |
| Chromium permission policy and ACP request option validation | Implemented and tested |
| Browser tool catalogue, local dispatcher and site-adapter manifest validator | Implemented as an **unwired scaffold** |
| Flutter multi-session standalone UI | Offline preview; transport not connected |
| Packaged Flutter extension side panel and full page | Build script and manifest, not browser integration tested |
| Browser-to-chat selected-tab/selection context handoff | Planned |
| Authenticated ACP WS/WSS and cross-language peer handshake | Planned |
| Native service discovery and optional stdio bridge | Planned |
| Browser-facing MCP server and remote session controller | Planned |
| CDP full-debugging mode and approvals | Planned |
| Actual JS/Wasm user site-adapter execution and sandbox | Planned |
| Firefox and Safari browser integrations | Planned |

No ACP interoperability, remote browser access or security guarantee is asserted for incomplete components.
