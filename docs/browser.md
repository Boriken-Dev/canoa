# Browser integration

The shared Flutter UI is in [`app/`](https://github.com/Boriken-Dev/canoa/tree/main/app) and is intended to run in a standalone PWA, extension full-page tab or narrow side panel.

## Chromium and Edge

Canoa's Manifest V3 Chromium binding lives at [`bindings/browser/chromium/`](https://github.com/Boriken-Dev/canoa/tree/main/bindings/browser/chromium). The extension has its own tab/origin/peer permission checks, browser tool catalogue and code for permission-gated tab actions. The side-panel and full-page entries package the Flutter UI. Agents are reached over a proven link, and the tools are offered as "Remote browser tools" and "A program on a paired machine" below say.

The standard extension does not request `debugger`. An elevated `manifest.debug.json` enables the Chromium `debugger` API in a separately packaged build. The extension now has an MCP JSON-RPC handler and permission-gated CDP commands, tested independently; a trusted remote bridge and production peer-pairing UI are still missing. Local per-tab browser grants can be managed from the extension's Browser Permissions options page. See [browser MCP and DevTools](browser-mcp.md).

## Webpage context

The extension now offers an **Ask Canoa** context-menu action for a page or user-selected text. The selected context is queued in extension session storage and claimed once by the Flutter UI through an extension-only bridge. Query strings, URL fragments and embedded credentials are stripped before attachment. The resulting note is **local only**; no content is transmitted to an agent because authenticated ACP messaging and explicit send confirmation are still pending. Websites cannot send privileged extension commands merely by being open.

## Remote browser tools

An agent that Canoa's own chat started can use the browser's tools, when its bridge runs `serve --mcp-offers`. Nothing listens in the browser, and the agent needs nothing beyond what ACP already requires of it. A program with no session is [below](#a-program-on-a-paired-machine).

1. The chat connects through the extension's service worker. For an agent whose record offers `mcp-offer/1`, the worker opens a proven link and runs ACP on one stream of it.
2. For each session, the worker opens another stream, and the bridge answers with a stdio `mcpServers` entry that the worker adds to the session. The relay still reads no ACP.
3. The agent starts that program, a small shim, as it would any stdio server. The shim shows a token on a loopback port made for that stream, and from then on carries MCP's own messages to the extension.

**What an agent has.** `browser_request_access`, and whatever that agent's peer still holds a grant for, so at first that one tool. It names a capability and a reason, and opens the extension's own window, where a page cannot draw. There the person sees the reason as the agent's own words, and picks the tab and for how long. On a yes the agent's tool list grows, and it is told so. Browser-local checks still decide every call, against the tab, its origin, the capability and the time left, however the agent is identified.

| It takes | To |
| --- | --- |
| Refusing, or no answer in two minutes | No grant, and 30 seconds before that agent can ask again |
| Loading a new page in the tab, closing it, or the time running out | End the grant. A grant the person gives after the tab moved within its site stays for that page |
| Removing it under **Canoa browser permissions** | End the grant, and tell the agent |

**Who has none of this.** A session a page started: only the extension's own chat gets the server, the worker adds it after the page's scope, and a page's connection has no streams at all. A page cannot give this grant, since it could then steer an agent into driving the browser. The debugger tools exist only in the debugger build, as their own capability, and only `http` and `https` tabs can be offered, which leaves out the extension's window and the browser's own pages.

**What stays true.** What an agent reads on a page can try to talk it into things, and a grant lets it act on that, so share a tab as you would show it to the agent yourself. A grant is by peer, so several agents behind one bridge share it. Any program running as you on the agent's machine can read the shim's token file and use what that agent holds; another user on that machine cannot. Anyone able to pair with `acp/1` can already run commands on that machine: the bridge's own documentation says so.

## A program on a paired machine

An MCP client that Canoa did not start, such as an IDE or a CLI agent, can ask for a tab the same way, with no ACP session and nothing listening in the browser. It is off unless a person turns it on, for one machine.

1. On that machine, run `coqui-acp-bridge serve --mcp-offers --advertise NAME`, allowing this browser's peer id, with the agent program it asks for after `--`: none starts, since no ACP runs here. Add it under **Canoa agents**.
2. Under **Canoa browser permissions**, choose it and turn it on. The worker opens a proven link there and keeps one more stream open than are in use, with no ACP on the link.
3. On that machine, `coqui-acp-bridge mcp-config --peer <this browser's peer id>` prints the `mcpServers` entry to paste. The permissions page shows that command, filled in. The entry holds no port and no secret: the shim finds them in a file only that user can read, so it stays good when the browser or the bridge restarts.

A program then starts the shim as it would any stdio server, and has what an agent has: `browser_request_access`, and whatever that peer still holds a grant for. The window says **A program on** the machine, and shows its key, since no agent's name can be proven and Canoa cannot tell which program it is. Everything under "Remote browser tools" holds for it too: browser-local checks decide every call, and a grant ends the same ways.

| It takes | To |
| --- | --- |
| Turning it on | End what that machine held so far, so nothing it was given before reaches its programs without a person seeing who asks |
| Turning it off | End every program's connection, and tell each with a last MCP message before its server ends. The grants that machine held go too, and a window still asking closes |
| Closing the browser | End every program's connection, and every grant. Restarting it brings the link back with no page open and no change to the entry, and a program has to start its server again to get back in. With WebTransport the bridge can take about 25 seconds to notice |
| Nothing connected | The shim says why, on standard error and as the answer to its client's first message, and exits |
| Removing the agent under **Canoa agents** | Turn it off too |

**Several programs at once.** Each has a stream of its own, up to 8, under the one peer's grant, as several agents behind one bridge have now. So what a person allows one, they allow all of them, and that peer's agents in Canoa's chat too.

**What keeps it running.** A browser ends an extension's idle worker after 30 seconds, and the socket goes with it. A WebSocket's traffic counts as activity and a WebTransport's does not, as measured in Edge, so while the link is up the worker makes an extension API call every 20 seconds, which counts on either. An alarm every 30 seconds starts the worker again if it ended anyway, or tries a machine that was off, and a heartbeat on the link notices one gone silent. No tab has to be open.

**Who has none of this.** A page: the setting answers only the extension's own permissions page, a content script cannot reach the storage holding it, and a page's connection still has no streams. Another peer of that bridge: the shim reaches only the peer its entry names, whose key the bridge proved.

**What stays true.** Any program running as that machine's user can read the shim's file, ask you for a tab, and use what that machine holds, and a program you would not trust with a tab should not run as that user there. Another user on it cannot read that file: it has an ACL for its owner alone on Windows, and owner-only modes elsewhere. A chat conversation's token file is not held to that on Windows, where it takes `%LOCALAPPDATA%`'s permissions, and off does not close that way: while a conversation is open, a program running as that user could find its server. A program can only ask, as often as the wait after a refusal lets it, and a person sees each time.

## Other browsers

| Browser | Planned implementation |
| --- | --- |
| Chrome/Chromium | MV3 side panel, userScripts and optional debugger/CDP build |
| Edge | Chromium-compatible extension with browser-specific validation |
| Firefox | WebExtension sidebar and DevTools integration using Firefox-specific APIs |
| Safari | Safari Web Extension and Safari-supported inspection features only |

The browser is the final authority for browsing access. A native broker, agent and website cannot override it.

See [site adapters](site-adapters.md), [ACP permissions](acp-permissions.md) and [architecture](architecture.md).
