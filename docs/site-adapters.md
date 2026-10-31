# User-installed site adapters

A Canoa site adapter is user-installed code that exposes a stable collection of **MCP tools** for one or more website origins. It is useful when a site does not offer WebMCP or a remote MCP service.

## Contract

An adapter manifest names an id, a version, its runtime (`javascript` or `wasm`), a local entrypoint, exact origin matches and the tools it provides. A tool has a unique name, description and JSON Schema for its arguments. A versioned interface will map `tools/list` and `tools/call` to the adapter; the raw source code is never itself a transport.

The manifest validator is in `bindings/browser/chromium/site-adapters.js`. Validation does **not** execute modules, install browser permissions or establish an MCP server.

Example:

```json
{
  "id": "example-shop",
  "version": "0.1.0",
  "runtime": "javascript",
  "entrypoint": "adapter.js",
  "origins": ["https://shop.example", "https://checkout.example"],
  "tools": [
    {
      "name": "search_products",
      "description": "Search products in the approved store",
      "inputSchema": {
        "type": "object",
        "properties": { "query": { "type": "string" } },
        "required": ["query"]
      }
    }
  ]
}
```

## Browser execution model

- For user-provided JavaScript, prefer Chrome's `userScripts` API with its dedicated isolated `USER_SCRIPT` world. Chrome requires the user to enable **Allow User Scripts** in extension settings. Do not execute uploaded code with `eval` in the privileged extension service worker.
- A page-global `MAIN` world is a different trust level: only grant it when an adapter truly needs webpage application state, and treat exchanged values as untrusted.
- WebAssembly adapters need a **separate constrained runtime** that supplies a restricted host-call ABI for DOM, network and browser capabilities. Chrome `userScripts` alone does not provide a Wasm host-call ABI. Wasm code cannot magically access DOM APIs without a host bridge.
- Neither JS nor Wasm adapter may access `chrome.debugger`, extension credentials, cookie stores or unrestricted native messaging directly. The trusted service worker mediates capability-checked host calls. Full debugging is separate.
- Installation requires user consent for the adapter's exact origins and capabilities. An agent invoking an adapter tool requires an independent grant from the browser, scoped to the verified peer and affected tab.
- Adapter manifests and source should be versioned, content-hashed and pinned; upgrades require review if origins, tools or requested capabilities change.
- Scripts must not be loaded from a remote server into privileged MV3 extension pages. UserScripts provides an API for specifically user-supplied scripts; Chrome Web Store policies still apply.

## Planned MCP-facing surface

- `site_adapter_list`: list approved adapters and tools for the active origin
- `site_adapter_call`: invoke a named adapter tool on a site under the verified peer/tab/origin grant
- `site_adapter_install`: UI-owned action requiring explicit user approval; **not** callable by an untrusted remote agent
- `site_adapter_revoke`: revoke access and unregister its scripts

The first implementation phase includes manifest validation only. Script execution, isolated Wasm host calls, trusted transport integration and a management UI are pending.

References: https://developer.chrome.com/docs/extensions/reference/api/userScripts and https://modelcontextprotocol.io/specification
