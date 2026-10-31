# ACP permission decisions in the browser

Canoa, as an ACP **client**, handles `session/request_permission` calls from any verified ACP agent. This is different from Canoa's own tab/DOM access grants.

The browser extension may make its own decision to present a prompt, select an option automatically under a narrowly scoped user-created rule, reject a request, or cancel outstanding work. It does not need a central Canoa gateway to decide.

## Standard behavior

1. Authenticate the agent peer and bind the request to its ACP session.
2. Validate the request and list **only the options actually offered** by the agent, preserving each `optionId`, `kind` and display name.
3. Apply the browser's own policy for this verified peer, session and action. If there is no applicable rule, show a prompt.
4. On user selection, respond `{ "outcome": { "outcome": "selected", "optionId": "<offered id>" } }`. Do not invent `allow_once` or `allow_always` options.
5. On session cancellation, respond `{ "outcome": { "outcome": "cancelled" } }`. When denying by policy, select an offered reject option if there is one; otherwise cancel.
6. The agent remains responsible for its own runtime enforcement and any agent-side remembered permissions. In ACP v2, an agent owns the remembered scope of `allow_always` or `reject_always`. Canoa's browser-side preferences are independent.

Canoa's `acp-permissions.js` implements request-option validation and response construction. The approval UI and transport-side request handling are **not implemented** yet.

Do not treat permission text supplied by the agent as a trustworthy authorization rule. Unknown future option kinds must be displayed generically and never turned into an automatic allow by guessing their semantics.

## Independent browser grants

Even when Canoa replies `allow_once` to an ACP agent permission request, that does not authorize the agent to inspect a browser tab or run WebMCP tools. The local grant must match the **verified peer, tab, origin, capability and lifetime**. No broker or agent choice can override that separate check.

See [interoperability](interop.md) and the [upstream ACP permission specification](https://agentclientprotocol.com/protocol/v2/tool-calls#requesting-permission).
