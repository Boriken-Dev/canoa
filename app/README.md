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
A transport-only WebTransport-first/WSS-fallback connector exists, but no authenticated ACP peer handshake, real remote session state, persistence
or remote MCP viewer is connected yet.

`lib/session_store.dart` is the shared view model. Transport and browser
permissions must remain independent of it. A real transport must perform the
interoperable peer-auth handshake and bind a verified peer before attaching
an ACP session.
