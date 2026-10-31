# Canoa for Chromium

Canoa is a Manifest V3 browser extension. It is independently responsible for deciding which verified peer can access which tab, document origin and browser capability. The permission decision cannot be delegated solely to a native gateway.

The initial module `permissions.js` implements the isolated, deny-by-default decision logic for verified peers, tab/origin scoping, individual browser tools and expiry. This is a building block, not a substitute for browser UI and a verified authentication handshake. A native host or WSS peer must supply a genuinely verified identity; never trust a `peerId` supplied by an untrusted webpage.

ACP over WS/WSS is the required external agent transport. WSS is the production profile. Coqui-peer's peer-proof format for browser WebSockets needs a publicly documented, versioned wire profile before any implementation can claim compatibility.

The full browser popup, WebMCP integration, MCP tool transport and native discovery adapter remain to be implemented.
