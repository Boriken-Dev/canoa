# Canoa for Chromium

Canoa is a Manifest V3 extension named **Canoa**, under `bindings/browser/chromium/`.

The canonical multi-session chat UI lives in root `app/` (Flutter); build with `python app/tool/build.py chromium` to embed the UI in the side panel or a full-page tab. A separate PWA build uses the same source. The backend ACP session transport is not yet connected, so chat is currently a local preview.

## Browser-facing building blocks

- `permissions.js`, `acp-permissions.js`: independent browser and ACP permission decisions; both have tests.
- `tools.js`, `browser-tools.js`: local MCP-shaped browser tool catalogue and grant-checked operations, exposed through an internal MCP JSON-RPC tools adapter, but not yet linked to an authenticated remote MCP transport.
- `site-adapters.js`: validates manifests for user-installed JavaScript or Wasm adapters bound to one or multiple website origins; no arbitrary code execution or adapter installation is implemented yet.
- `manifest.json` and `background.js`: MV3 side panel, full-page view, **Ask Canoa** page/selection context menu, local policy and tab-navigation revocation.
- `context-transfer.js` and `bridge.js`: user-selected context is scrubbed of URL query tokens and handed once to the Flutter UI. It is attached **locally** and never sent to an agent until a real transport and send approval exist.
- `manifest.debug.json`: elevated debugger build configuration. Debug control requires separate UI authorization and an authenticated agent connection; the file alone does not enable safe remote CDP control.

## Planned user experience

Match the useful conventions of ChatGPT and Claude in Chrome: **Ask Canoa** on a tab or text selection; attach explicit page/tab context to a chosen session; choose a peer and agent; view multiple chats in a full-page view or a side panel; review an active agent's activity; and grant website and debugging access independently.

An agent can use site adapters for stable website-specific MCP tools, ordinary browser DOM tools, or (with elevated permission) Chromium CDP debugging.

See [browser comparison](../../../docs/browser-comparison.md), [site adapters](../../../docs/site-adapters.md), [interop](../../../docs/interop.md), and [ACP permission policy](../../../docs/acp-permissions.md).

## Current limitations

The authenticated ACP echo fixture exists separately, and the extension now has a local permissions options page, MCP JSON-RPC adapter and debug-only CDP commands. There is still no authenticated remote MCP bridge, production peer pairing UI, installed user-script runtime, cross-browser execution test, or publishable distribution. The options page's grants are not agent identity proofs. The extension build is not ready for Chrome Web Store distribution.

## Build the elevated CDP variant

Use `python app/tool/build.py chromium --debug`. This produces a **separate** unpacked directory at `build/chromium-debug` with the privileged `debugger` permission, leaving `bindings/browser/chromium/manifest.json` unchanged. Load it explicitly using chrome://extensions → Developer mode → Load unpacked. The browser grants page is accessible under the extension Options menu. Installing the debug build and granting browser.debug is a high-privilege action; remote MCP transport and production peer approvals are not wired yet.
