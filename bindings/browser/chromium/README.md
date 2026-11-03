# Canoa for Chromium

Canoa is a Manifest V3 extension named **Canoa**, under `bindings/browser/chromium/`.

The canonical multi-session chat UI lives in root `app/` (Flutter); build with `python app/tool/build.py chromium` to embed the UI in the side panel or a full-page tab, where it reaches agents through the same provider a page does. A separate PWA build uses the same source.

## A provider of `document.acp`

The extension implements [ACP in the browser](https://boriken-dev.github.io/coqui-acp/browser/), the contract in coqui-acp, so a page can find and use a person's agents without knowing it is Canoa.

- `acp-page.js` runs in the page's world and defines the registry; everything it does is the page's to tamper with.
- `acp-content.js` sits between it and the service worker, and adds what the page cannot forge: whether the person has just acted.
- `acp-gateway.js` decides: grants, per-origin ids, and coqui-acp's scope on every message. It trusts only the origin the browser reports.
- `acp-provider.js` puts that on Chrome's APIs, and `acp-request.html` is the window that asks the person, where a page cannot draw over it.
- `acp-dial.js` opens a proven, sealed WebSocket. Agents are added by hand, or chosen from what `acp-discovery.js` reads out of coqui-acp-bridge's companion when the person asks to look nearby. What it finds is a claim: nothing is ticked for the person, and a key it named is tested only by the proof at connect.

Right-click the toolbar icon for **Let this site ask for my agents** and **Canoa agents**.

`npm test` runs coqui-acp's conformance checks against this code. `e2e/run.mjs` runs it in a real browser, under a page with a strict CSP, against an agent on a real socket, with `CANOA_E2E_BRIDGE=<path to coqui-acp-bridge>` through the real companion, bridge and stdio agent over WebTransport and WebSocket, and with `CANOA_E2E_PAGES=<dir>` from a Rust page in WebAssembly and from Canoa's own connection code, as `e2e/run.mjs` describes: `CANOA_E2E_BROWSER=<path to Edge> node e2e/run.mjs`. With a bridge, it also runs an agent with a real MCP client through the browser's tools, and with `CANOA_E2E_REAL_AGENT=claude` and `CLAUDE_CODE_EXECUTABLE` set, a real Claude turn, which spends a little of the subscription. `e2e/standing.mjs` runs a program with no session from what `coqui-acp-bridge mcp-config` prints, with the official MCP SDK's client (`npm install` in `e2e/`) and, with `CANOA_E2E_REAL_AGENT=claude-code`, real Claude Code; it says at its top how to leave it idle. Chrome's own builds ignore `--load-extension`, so use Edge or Chrome for Testing there.

## Browser-facing building blocks

- `permissions.js`, `acp-permissions.js`: independent browser and ACP permission decisions; both have tests.
- `tools.js`, `browser-tools.js`: the browser tool catalogue and its grant-checked operations.
- `mcp.js`, `mcp-access.js`, `acp-tools.js`: those tools as an MCP server for an agent of Canoa's own chat, over its proven link, with `browser_request_access` the one tool it has before a person grants more. See [browser integration](../../../docs/browser.md).
- `acp-standing.js`: the same for a program on one paired machine with no ACP session, which a person turns on under **Canoa browser permissions**.
- `site-adapters.js`: validates manifests for user-installed JavaScript or Wasm adapters bound to one or multiple website origins; no arbitrary code execution or adapter installation is implemented yet.
- `manifest.json` and `background.js`: MV3 side panel, full-page view, **Ask Canoa** page/selection context menu, local policy and tab-navigation revocation.
- `context-transfer.js` and `bridge.js`: user-selected context is scrubbed of URL query tokens and handed once to the Flutter UI. It is attached **locally** and never sent to an agent until a real transport and send approval exist.
- `manifest.debug.json`: elevated debugger build configuration. Debug control requires separate UI authorization and an authenticated agent connection; the file alone does not enable safe remote CDP control.

## Planned user experience

Match the useful conventions of ChatGPT and Claude in Chrome: **Ask Canoa** on a tab or text selection; attach explicit page/tab context to a chosen session; choose a peer and agent; view multiple chats in a full-page view or a side panel; review an active agent's activity; and grant website and debugging access independently.

An agent can use site adapters for stable website-specific MCP tools, ordinary browser DOM tools, or (with elevated permission) Chromium CDP debugging.

See [browser comparison](../../../docs/browser-comparison.md), [site adapters](../../../docs/site-adapters.md), [interop](../../../docs/interop.md), and [ACP permission policy](../../../docs/acp-permissions.md).

## Current limitations

There is still no production peer pairing UI, installed user-script runtime, cross-browser execution test, or publishable distribution. The options page's grants are not agent identity proofs. The extension build is not ready for Chrome Web Store distribution.

## Build the elevated CDP variant

Use `python app/tool/build.py chromium --debug`. This produces a **separate** unpacked directory at `build/chromium-debug` with the privileged `debugger` permission, leaving `bindings/browser/chromium/manifest.json` unchanged. Load it explicitly using chrome://extensions → Developer mode → Load unpacked. The browser grants page is accessible under the extension Options menu. Installing the debug build and granting browser.debug is a high-privilege action; remote MCP transport and production peer approvals are not wired yet.
