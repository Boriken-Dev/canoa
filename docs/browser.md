# Browser integration

The shared Flutter UI is in [`app/`](https://github.com/Boriken-Dev/canoa/tree/main/app) and is intended to run in a standalone PWA, extension full-page tab or narrow side panel.

## Chromium and Edge

Canoa's Manifest V3 Chromium binding lives at [`bindings/browser/chromium/`](https://github.com/Boriken-Dev/canoa/tree/main/bindings/browser/chromium). The extension has its own tab/origin/peer permission checks, browser tool catalogue and code for permission-gated tab actions. The side-panel and full-page entries package the Flutter UI. The native transport and verified agent identity are **not wired yet**.

The standard extension does not request `debugger`. An elevated `manifest.debug.json` enables the Chromium `debugger` API in a separately packaged build. The extension now has an MCP JSON-RPC handler and permission-gated CDP commands, tested independently; a trusted remote bridge and production peer-pairing UI are still missing. Local per-tab browser grants can be managed from the extension's Browser Permissions options page. See [browser MCP and DevTools](browser-mcp.md).

## Webpage context

The extension now offers an **Ask Canoa** context-menu action for a page or user-selected text. The selected context is queued in extension session storage and claimed once by the Flutter UI through an extension-only bridge. Query strings, URL fragments and embedded credentials are stripped before attachment. The resulting note is **local only**; no content is transmitted to an agent because authenticated ACP messaging and explicit send confirmation are still pending. Websites cannot send privileged extension commands merely by being open.

## Remote browser tools

An agent that Canoa's own chat started can use the browser's tools, when its bridge runs `serve --mcp-offers`. Nothing listens in the browser, and the agent needs nothing beyond what ACP already requires of it:

1. The chat connects through the extension's service worker. For an agent whose record offers `mcp-offer/1`, the worker opens a proven link and runs ACP on one stream of it.
2. For each session, the worker opens another stream, and the bridge answers with a stdio `mcpServers` entry that the worker adds to the session. The relay still reads no ACP.
3. The agent starts that program, a small shim, as it would any stdio server. The shim shows a token on a loopback port made for that stream, and from then on carries MCP's own messages to the extension.

**What an agent has.** At first, one tool: `browser_request_access`. It names a capability and a reason, and opens the extension's own window, where a page cannot draw. There the person sees the reason as the agent's own words, and picks the tab and for how long. On a yes the agent's tool list grows, and it is told so. Browser-local checks still decide every call, against the tab, its origin, the capability and the time left, however the agent is identified.

| It takes | To |
| --- | --- |
| Refusing, or no answer in two minutes | No grant, and 30 seconds before that agent can ask again |
| Navigating the tab, closing it, or the time running out | End the grant |
| Removing it under **Canoa browser permissions** | End the grant, and tell the agent |

**Who has none of this.** A session a page started: only the extension's own chat gets the server, the worker adds it after the page's scope, and a page's connection has no streams at all. A page cannot give this grant, since it could then steer an agent into driving the browser. The debugger tools exist only in the debugger build, as their own capability, and only `http` and `https` tabs can be offered, which leaves out the extension's window and the browser's own pages.

**What stays true.** What an agent reads on a page can try to talk it into things, and a grant lets it act on that, so share a tab as you would show it to the agent yourself. A grant is by peer, so several agents behind one bridge share it. Any program running as you can read the shim's environment and use what that agent holds. Anyone able to pair with `acp/1` can already run commands on that machine: the bridge's own documentation says so.

## Other browsers

| Browser | Planned implementation |
| --- | --- |
| Chrome/Chromium | MV3 side panel, userScripts and optional debugger/CDP build |
| Edge | Chromium-compatible extension with browser-specific validation |
| Firefox | WebExtension sidebar and DevTools integration using Firefox-specific APIs |
| Safari | Safari Web Extension and Safari-supported inspection features only |

The browser is the final authority for browsing access. A native broker, agent and website cannot override it.

See [site adapters](site-adapters.md), [ACP permissions](acp-permissions.md) and [architecture](architecture.md).
