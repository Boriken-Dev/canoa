# Canoa for Chromium

Early implementation of Canoa's Chrome/Chromium browser interface.

## Supported extension-side behavior

- The toolbar popup grants temporary read or interaction access to a selected tab.
- The extension describes native-host-callable browser operations, including bounded page snapshots, clicking/filling visible elements and WebMCP tools.
- Page-registered WebMCP tools can be listed and invoked on browsers that expose `document.modelContext`.
- A site can advertise an MCP endpoint with `<link rel="mcp" href="/mcp">`; this is **Canoa's convention, not an official MCP discovery standard**. The extension only reports the URL; it never fetches or connects.
- No global host permissions; pages do not have permission to send privileged messages to the service worker.

## Not yet implemented

- The native messaging host `dev.boriken.canoa` and the authenticated MCP server exposing the tools over stdio/Streamable HTTP.
- Peer pairing, persistent permissions and remote MCP server connections.
- Automated extension tests, Chrome Web Store packaging and installation instructions for the native host.

To inspect the extension, open `chrome://extensions`, enable Developer mode and load this directory as unpacked. The popup will work, but native host communication will fail until the Canoa native host exists and is installed.

**Chrome's native messaging registration** requires an executable and a host manifest with `allowed_origins` set to the extension ID. Chrome does not install this automatically.
