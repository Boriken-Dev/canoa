# Browser integration

The shared Flutter UI is in [`app/`](https://github.com/Boriken-Dev/Canoa/tree/main/app) and is intended to run in a standalone PWA, extension full-page tab or narrow side panel.

## Chromium and Edge

Canoa's Manifest V3 Chromium binding lives at [`bindings/browser/chromium/`](https://github.com/Boriken-Dev/Canoa/tree/main/bindings/browser/chromium). The extension has its own tab/origin/peer permission checks, browser tool catalogue and code for permission-gated tab actions. The side-panel and full-page entries package the Flutter UI. The native transport and verified agent identity are **not wired yet**.

The standard extension does not request `debugger`. An elevated `manifest.debug.json` enables the Chromium `debugger` API in a separately packaged build. The extension now has an MCP JSON-RPC handler and permission-gated CDP commands, tested independently; a trusted remote bridge and production peer-pairing UI are still missing. Local per-tab browser grants can be managed from the extension's Browser Permissions options page. See [browser MCP and DevTools](browser-mcp.md).

## Webpage context

The extension now offers an **Ask Canoa** context-menu action for a page or user-selected text. The selected context is queued in extension session storage and claimed once by the Flutter UI through an extension-only bridge. Query strings, URL fragments and embedded credentials are stripped before attachment. The resulting note is **local only**; no content is transmitted to an agent because authenticated ACP messaging and explicit send confirmation are still pending. Websites cannot send privileged extension commands merely by being open.

## Remote browser tools

MCP tools can be served to verified agents when a standards-compliant MCP server/adapter has been connected to the extension. Browser-local permission checks still decide every tab operation, even when the remote peer is authenticated.

## Other browsers

| Browser | Planned implementation |
| --- | --- |
| Chrome/Chromium | MV3 side panel, userScripts and optional debugger/CDP build |
| Edge | Chromium-compatible extension with browser-specific validation |
| Firefox | WebExtension sidebar and DevTools integration using Firefox-specific APIs |
| Safari | Safari Web Extension and Safari-supported inspection features only |

The browser is the final authority for browsing access. A native broker, agent and website cannot override it.

See [site adapters](site-adapters.md), [ACP permissions](acp-permissions.md) and [architecture](architecture.md).
