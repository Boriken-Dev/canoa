# Browser bindings

The Canoa browser bindings provide a local permission authority for tools acting on tabs and site-provided services.

- `chromium/`: Chrome/Chromium MV3 extension, named **Canoa**.
- Additional browsers and SDKs may be added as independent implementations.

The extension may connect directly to WSS ACP peers (once their handshake is implemented) and does not require the Canoa native daemon for authorization. An optional native host may discover mDNS services, launch stdio agents or route MCP tools for native MCP clients.

See [the interoperability profile](../../docs/interop.md).
