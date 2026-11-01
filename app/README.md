# Canoa Flutter app

The canonical Canoa chat UI lives at the repository root under `app/`. The
standalone PWA, desktop builds and Chrome extension side panel/full-page
window share this Dart/Flutter application; the extension retains privileged
browser operations and browser-owned permission policy in
`bindings/browser/chromium/`.

## Development

```sh
cd app
flutter pub get
flutter run -d chrome
flutter test
flutter analyze
```

## Output choices

```sh
cd app
flutter build web --release
flutter build web --release --wasm
```

A platform-independent packager at `tool/build.py` can build a standalone
PWA or copy a locally bundled Flutter output into the Chromium extension.
The `--wasm` output is appropriate for deployment on a server that can set
cross-origin isolation headers; its use inside a Chromium Manifest V3 extension
must also be checked against the extension's CSP and renderer requirements.

## App state

The current UI offers a responsive session list, independent transcripts and
a composer. It starts in an **explicit offline preview**. Messages typed into
a preview conversation are kept only in memory and are never sent to an agent.
The WebTransport-first/WSS-fallback transport is connected to an ACP v2 controller **through an injected verified-peer interface**. The Flutter session store and composer can process streaming text, insertion acknowledgments and state transitions from a verified client. The experimental coqui-peer WebCrypto method and explicit pinned-peer dialog are available. Production Coqui pairing/host authorization, session persistence and remote MCP browsing are still missing; the normal app remains in offline preview.

`lib/session_store.dart` is the shared view model. Transport and browser
permissions must remain independent of it. A real transport must perform the
interoperable peer-auth handshake and bind a verified peer before attaching
an ACP session.

## ACP v2 integration

The [ACP client](lib/acp/client.dart) and [connection orchestrator](lib/acp/connect.dart) intentionally cannot send ACP methods before a supplied verifier proves the expected peer identity. The `SessionStore.attachVerifiedClient` API binds that initialized client to one local/remote session and routes updates by the agent-issued Session ID. See [protocol documentation](../docs/acp-client.md).

## Shared dependencies

The browser cryptographic method is owned by [coqui-peer](https://github.com/Boriken-Dev/coqui-peer/tree/main/methods/webcrypto) and vendored in `app/web/vendor` at the exact recorded Git blob in `COQUI-SOURCE.json`. Browser-side UI code never implements its own ECDSA handshake. The early-message handoff is provided by `coqui_web` as a local Dart package, not a Canoa authentication class.

To upgrade the browser method, update its canonical Coqui source and tests first, then replace the exact vendored source and source metadata together; the vendor tests will reject an unrecorded change.

## Smoke testing remote chat

Use the [authenticated ACP echo fixture](../examples/acp-echo/README.md) to verify the WebSocket path from the connection dialog. It requires trusted TLS, explicit Coqui peer allowlisting, and a pinned server key. This is diagnostic only, not a deployed agent service.

## Desktop tray and Canoa.cfg

The app reads a TOML Canoa.cfg just like AI6's OO.cfg. The tray can keep sessions running after the main window is hidden, with Open, Settings, Hide and Exit commands. On supported Windows/macOS tray hosts, a double click opens the main window; Linux AppIndicator requires Open from its menu. Refer to the [configuration guide](../docs/configuration.md). Build platform runners with flutter create --org dev.boriken --platforms=windows,linux,macos . in app/ before flutter build windows/linux/macos.
