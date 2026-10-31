# Architecture

Canoa mediates between clients (native applications, language libraries and browser origins) and agent services.

## Logical flow

1. **Discover** potential local DNS-SD/mDNS services or load explicit remote registrations.
2. **Verify** peer identities through an authenticated transport and `coqui-peer` trust state. DNS-SD names and addresses are untrusted.
3. **Authorize** a client for a specific peer, protocol and capability through an explicit grant.
4. **Connect** the client to an ACP session or another supported protocol. Never allow pages to bypass the broker and collect raw bearer credentials.
5. **Revoke** grants without altering an agent's internal permission decisions.

## Boundaries

- Client-facing calls must bind a principal to an authenticated native application or browser origin; never trust a `client_id` sent by a web page.
- For the browser, use a Manifest V3 extension with an optional native messaging companion for DNS-SD and local process access. A webpage's injected bridge is not an authorization boundary.
- The broker is the security boundary. It validates requested service, granted capabilities, peer verification, and intended protocol before forwarding anything.
- Localhost URLs must be verified against DNS rebinding, redirects, CSRF, and hostile local processes. Pairing is independent of CORS.
- The core crate currently contains only the *decision model*. Its caller-supplied `verified` boolean is insufficient to establish security on its own.

## Implementation milestones

1. A tested registry and explicit deny-by-default policy (initial scaffold).
2. Endpoint inventory, DNS-SD/mDNS discovery, endpoint de-duplication, TTL expiry, and manual registrations.
3. Peer identity validation and persistent pairing using `coqui-peer`, scoped grants and revocation.
4. Native gateway service, ACP transport bridge, replay/cancellation boundaries and CLI.
5. Canoa Chrome/Firefox extension, user-consent UX, origin-scoped access and JS SDK.
6. Other language bindings and transport/protocol adapters as concrete needs arise.
