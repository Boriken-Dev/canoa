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

The browser cryptographic methods are owned by [coqui-peer-noise](https://github.com/Boriken-Dev/coqui-peer-noise) and [coqui-peer-webcrypto](https://github.com/Boriken-Dev/coqui-peer-webcrypto) and vendored in `app/web/vendor` at the exact recorded Git blobs in `COQUI-SOURCE.json`, one entry per file. The app proves by WebCrypto; the extension by either. Browser-side UI code never implements its own ECDSA handshake. The early-message handoff is provided by `coqui_web` as a local Dart package, not a Canoa authentication class.

To upgrade the browser method, update its canonical Coqui source and tests first, then replace the exact vendored source and source metadata together; the vendor tests will reject an unrecorded change.

## Smoke testing remote chat

Use the [authenticated ACP echo fixture](../examples/acp-echo/README.md) to verify the WebSocket path from the connection dialog. It requires trusted TLS, explicit Coqui peer allowlisting, and a pinned server key. This is diagnostic only, not a deployed agent service.

## Agents on this computer

The desktop app runs any program that speaks ACP on its standard streams, with no bridge and no pairing, since it is the person's own program. The connect dialog offers Claude, Codex, Gemini and Pi when what each needs is installed: Claude and Codex through their ACP adapters, fetched with `npx`, driving the tool you have already signed in to, so its subscription applies and no API key is involved; Pi through `pi-acp`, also fetched with `npx`, with whatever model and key Pi itself is set up with. Any other agent runs by typing its command in the dialog, or by naming it in `Canoa.cfg`, under `[agents.NAME]` with `program` and `args`. With no folder named, it starts in your home directory. The first run downloads the adapter. Each permission request waits in the conversation until you answer, and closing the session ends the agent and anything it started.

Canoa can only show what an agent asks. An adapter drives your installed tool under that tool's own settings, so what you have already allowed there, such as Claude's `permissions.allow` rules, runs without a question here. Tool calls still appear in the conversation as they happen.

An agent's Markdown is untrusted: images are not fetched, and a link is copied rather than opened.

## Desktop tray and Canoa.cfg

The app reads a TOML Canoa.cfg just like AI6's OO.cfg. Closing the window keeps the app, its conversations and what it hosts running in the tray, whose menu has Open, Hosting, Settings, Hide and Exit; a double click on the icon opens the chat on Windows, and on Linux the menu's Open does. Where Linux shows no tray icons at all, closing minimizes the window instead, and Settings has **Quit Canoa**. Refer to the [configuration guide](../docs/configuration.md). Build platform runners with flutter create --org dev.boriken --platforms=windows,linux,macos . in app/ before flutter build windows/linux/macos.

The desktop app runs `canoa-host` as its child (`lib/desktop/host_supervisor.dart`): the one `CANOA_HOST` names, else the one beside its executable. **Hosting** adds an agent, pairs and removes keys and shows a PIN; see [the host](../docs/host.md#in-the-desktop-app).

`integration_test/desktop_test.dart` runs the app on a real desktop (`flutter test integration_test/desktop_test.dart -d windows`, or `-d linux`) with `CANOA_TEST_HOST`, `CANOA_TEST_NODE` and `CANOA_TEST_APP` as `test/host_test.dart` describes.
