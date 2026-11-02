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
| Browser WebCrypto coqui-peer method | Experimental canonical method in coqui-peer; vendored exact source in Canoa; UI supports pinned peer ID |
| Native Rust Coqui WebCrypto responder and JS↔Rust proof | Implemented and cross-language CI-tested in coqui-peer; explicit accepted keys and live trust-store revocation supported |
| Authenticated WSS ACP v2 echo endpoint | Implemented and tested with live TLS WebSocket, browser-compatible Coqui proof and ACP v2 prompt lifecycle; test fixture, not AI agent |
| Production agent pairing, persistent agent services, trust-management UI | Pending; no auto-enrollment of unknown peers |
| Running an agent on this computer from the desktop app | Implemented for any ACP program: presets for Claude, Codex and Gemini when installed, each permission request answered in the conversation, nothing left running on close. Rich rendering and themes are planned |
| Agents from the browser's provider in Canoa's web build | Implemented: the connect dialog offers them when a provider is present, and takes only an agent it reports as proven |
| Native service discovery and stdio bridge | Implemented in `coqui-acp-bridge`: `serve --advertise`, `discover` and a native companion the extension asks when the person looks nearby; a stdio agent on the network under a proven peer |
| Local MCP JSON-RPC browser-tool handler | Implemented with test-covered permission checks; not yet available remotely |
| Elevated CDP adapter and local grant UI | Implemented and JS-tested; debug-only manifest, permission options page, console/network event buffer; remote transport pending |
| Browser-facing MCP server and remote session controller | Planned: connect the authenticated peer to the local handler |
| Full remote debugging and audit trail | Planned; local privileged CDP tools are not remote agent access |
| Actual JS/Wasm user site-adapter execution and sandbox | Planned |
| Firefox and Safari browser integrations | Planned |

The experimental ACP v2 echo fixture has an end-to-end test. This is not evidence of production agent interoperability, remote browser access or a security audit.
