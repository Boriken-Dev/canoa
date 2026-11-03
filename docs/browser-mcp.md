# Browser MCP and Chromium DevTools

The Chromium extension has an **MCP JSON-RPC handler** and a grant-checked browser tool dispatcher. Nothing listens in the browser: it reaches an agent over a proven link, either declared to a session of Canoa's own chat or standing for a program on a paired machine, as [browser integration](browser.md) describes. The peer is what the link's proof showed. Do not expose the handler through website messages or accept a caller-supplied peerId as verification.

## Available MCP methods

The handler supports initialize (2025-06-18 and 2025-11-25 profiles), ping, tools/list and tools/call, with logging/setLevel for a program with no session, whose server sends one log message: the one saying a person turned it off. Listing is filtered by the verified peer's browser-owned grants; each call is separately authorized by peer, origin, tab ID and capability.

## A client outside any ACP session

`coqui-acp-bridge mcp-config --peer <browser peer id>` prints an entry for any stdio MCP client, with no secret in it:

```json
{ "mcpServers": { "canoa-browser": { "type": "stdio", "command": "/abs/path/coqui-acp-bridge", "args": ["mcp-shim", "--peer", "sha256:…"], "env": {} } } }
```

Before a person allows a tab, `tools/list` answers `browser_request_access` alone. With nothing connected, `initialize` is answered with a JSON-RPC error saying so, and the server exits. The official SDK's stdio client and real Claude Code were both run from this entry against real Edge.

The Browser Permissions page requests actual Chrome host access and then stores a short-lived browser-owned grant. A cryptographically verified peer is still required before the MCP handler accepts any request. Granting a PeerId typed into the page does not automatically pair it.

Read and interaction tools include browser_list_tabs, browser_read_page, browser_click, browser_fill, browser_list_webmcp, browser_call_webmcp and browser_discover_mcp. Website MCP hints are read but never connected automatically.

## Elevated Chromium DevTools

The normal manifest has **no debugger permission**. For diagnostics, run `python app/tool/build.py chromium --debug` from the repository root. This writes a separate unpacked extension to `build/chromium-debug/`, using `manifest.debug.json` as its manifest. Chrome's Load unpacked action can load that directory without changing the standard manifest. A second browser.debug grant is still required for the authenticated peer and specific tab and origin.

browser_debug_command uses chrome.debugger.attach/sendCommand with an allowlisted subset of the Chrome DevTools Protocol: Runtime.enable, Runtime.evaluate, Runtime.getProperties, DOM.getDocument, DOM.querySelector, DOM.describeNode, CSS.getComputedStyleForNode, Log.enable, Network.enable, Network.getResponseBody, Performance.enable, Performance.getMetrics and Page.captureScreenshot.

Runtime.evaluate can execute arbitrary JavaScript in the inspected tab and may access sensitive page data; it is an elevated permission, not ordinary browser.read. Responses are size bounded. The adapter detaches on tab close and navigation. browser_debug_detach releases the attachment. browser_debug_events retrieves bounded console logs, JavaScript exceptions and network events from an active CDP attachment (after enabling the respective CDP domains). Unsupported CDP commands are rejected by default.

This is a diagnostic implementation, not a production remote debugging endpoint. A service worker cannot be treated as a trusted principal merely because it receives a message from a Canoa page. An agent pairing UI, an audit trail and a run against a real agent are still required. The extension now includes a local Browser Permissions page under its Options menu, allowing explicit per-peer, tab, origin, capability and expiry grants; this page is not itself proof of an authenticated remote agent.

## Browser differences

Chrome and Edge share the Chromium adapter. Firefox has different debugger/DevTools APIs, and Safari has separate extension limitations; neither is implemented by this Chromium-specific CDP path.

See Chrome's debugger API documentation: https://developer.chrome.com/docs/extensions/reference/api/debugger/
