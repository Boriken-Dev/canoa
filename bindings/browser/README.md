# Browser bindings

Browser integration is organized under `bindings/browser/`, alongside any future Python, Node or other language bindings.

- `chromium/`: Chrome/Chromium Manifest V3 extension, branded **Canoa**.
- `sdk/`: reserved for a portable browser-facing JavaScript API when implemented.
- `firefox/`: a distinct manifest/host configuration when implemented; do not assume Chromium's native messaging manifest is portable.

The extension is a client of a separately installed native Canoa broker. A browser extension alone does not supply an MCP transport reachable by agents.
