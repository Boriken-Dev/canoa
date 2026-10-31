# Browser integration reference: ChatGPT and Claude

Review date: 2026-10-31. This is product research, not a claim that Canoa implements these features.

## Products and differences

| Feature | ChatGPT browser extension | Claude in Chrome | Canoa direction |
| --- | --- | --- | --- |
| Sidebar chat | Yes, tied to the user's conversations | Yes, with Cowork integration | Flutter `app/` reused in side panel and full-page UI |
| Existing signed-in browsing context | Yes | Yes | Browser-specific extension integration |
| Select tabs/selected text as context | Yes; tab mentions and Ask ChatGPT | Yes; page assistance and browser tasks | Explicit target-picker and context menu |
| Multi-tab work | Yes | Yes | Peer/session-scoped tab access and optional agent tab groups |
| Developer troubleshooting | Full CDP in elevated developer mode | Browser debugger, DOM/console/network inspections, Claude Code integration | Explicit per-peer/session full-debug mode; CDP on Chromium |
| Permission presets | Once/site/all-site, blocked-site list; separate history approval | Manual/automatic/skip; site grants and protected actions | Manual, site-scoped and trusted full-control modes with separate capabilities |
| Desktop/web continuity | Side chat and desktop conversations are shared | Side panel and Claude client workflows | One Flutter multi-session state model; persistence and transport pending |
| Agent-agnostic ACP/peer compatibility | Not a public standard interface | Not a public standard interface | Required: independently implemented ACP WS/WSS + peer-auth profile |
| Third-party site adapters | No corresponding user-script MCP feature established in reviewed docs | Recorded workflows available in some modes | User-installable JS/Wasm MCP tools for designated sites |

Important: OpenAI also publishes **ChatGPT Search**, which merely changes the address-bar search provider. It is unrelated to browser automation. **ChatGPT Atlas** is a separate browser with integrated Ask ChatGPT and Agent mode; do not conflate its browser security limits with the Chrome extension.

ChatGPT documents support for Chrome, Edge, Brave, Opera and Vivaldi; side chat currently supports Chrome, Edge, Brave and Vivaldi. A browser's actual API support must still be checked independently for Canoa.

## User expectations worth adopting

1. Opening the side panel should keep the current page visible; full-page mode should show the same conversations.
2. Selecting a tab or text and choosing **Ask Canoa** should make that precise item available as context to the selected session, without broadcasting it to other agents.
3. Users should see the verified peer, agent name, session, target tabs, permission mode, and active actions.
4. A browser task should normally use dedicated tabs or groups so it does not unexpectedly take over the active tab.
5. A developer can opt in to a privileged **Full debugging** session for a known peer and tab group, using CDP on Chromium. Exposing console, network, execution, debugger and storage internals is high risk and must not be silently inferred from ordinary site access.
6. Per-origin allow once/site/global policy is different from browser-history access, local file access, downloads, and debugging access; do not combine them in one undifferentiated toggle.
7. Native code and connected MCP tools are not implicitly allowed because a peer has access to webpage text.
8. The app and extension need a common session index and message history, but extension grants remain locally enforced by the browser and revocable at any time.

## Where Canoa goes further

- **Transport-neutral identity:** ACP-over-WSS and a documented peer-auth profile, rather than one company's account.
- **Remote sessions:** explicitly bind remote session ID and verified agent peer, with attach, detach, prompt, cancel, resume or load according to the agent's advertised ACP capabilities.
- **Bidirectional MCP:** agent-to-browser tools and user-approved page-published WebMCP/remote MCP services.
- **Site adapters:** locally user-installed JS or Wasm modules that surface stable MCP tool APIs for one or more opted-in websites.
- **Cross-browser:** Chromium/Edge first, Firefox separate debugging backend; Safari support only to the extent its extension APIs permit.

## Security

No website can self-authorize an agent, impersonate its trusted identity, or cause the extension to auto-enable debugging. Website text, tool descriptions and site adapters are untrusted input. Grant decisions belong to the browser extension; the peer handshake and ACP session permission responses are independent decisions.

Full browser control is intentionally more powerful than either OpenAI's or Claude's default browser interaction modes. Exposing arbitrary CDP commands gives a remote agent access to sensitive browser state, so it needs a conspicuous opt-in, clear scope, activity indicator, and immediate detach/revoke.

## Primary references

- ChatGPT browser extension: https://learn.chatgpt.com/docs/chrome-extension
- ChatGPT Chrome Web Store: https://chromewebstore.google.com/detail/chatgpt/hehggadaopoacecdllhhajmbjkdcmajg
- ChatGPT developer mode: https://help.openai.com/en/articles/11369540-using-codex-with-your-chatgpt-plan
- ChatGPT Atlas: https://help.openai.com/en/articles/12628199
- Claude in Chrome: https://support.claude.com/en/articles/12012173-get-started-with-claude-in-chrome
- Claude in Chrome permission policy: https://support.claude.com/en/articles/12902446-claude-in-chrome-permissions-guide
