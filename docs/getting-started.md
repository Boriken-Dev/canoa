# Getting started

## Requirements

- Rust 1.89 or later to build the core.
- Node.js 22 or later to run Chromium extension policy tests.
- Flutter SDK with Dart 3.9 or newer for the standalone application. The exact Flutter version used by AI6's experimental windowing build is **not** required for this ordinary Material 3 UI.

## Rust core

```sh
git clone https://github.com/Boriken-Dev/Canoa.git
cd Canoa
cargo test --workspace --all-features
cargo doc --workspace --no-deps --all-features
```

The Rust crate currently implements only an in-memory registry and explicit grants, not network identity verification.

## Flutter app

```sh
cd app
flutter pub get
flutter run -d chrome
flutter test
flutter analyze
```

The UI opens with sample sessions. Sending a message only adds it to the local preview, **not** to a remote agent.

```sh
flutter build web --release
flutter build web --release --wasm
```

For the standalone PWA, deploy the generated `app/build/web` bundle under a server that supports the renderer requirements. Flutter's Wasm output may need COOP/COEP headers and a compatible browser.

## Chromium extension

```sh
python app/tool/build.py chromium
(cd bindings/browser/chromium && npm test)
```

Load `bindings/browser/chromium/` using `chrome://extensions` → Developer mode → Load unpacked. In a regular website, use the **Ask Canoa** context-menu action to attach its page or selected text to the local Flutter conversation. The packaging helper copies the Flutter UI to `ui/`; it does not yet connect a live ACP or MCP server. The manifest's optional host permissions and userScripts policy are not a substitute for explicit user consent.

See [browser integration](browser.md).

## Authenticated WSS smoke test

The [ACP echo fixture](https://github.com/Boriken-Dev/Canoa/tree/main/examples/acp-echo) provides a local test endpoint that uses mutually authenticated Coqui WebCrypto and ACP v2. See its README for TLS certificate setup, explicit browser peer allowlisting and pinning the server PeerId. Its only behavior is echoing messages, not invoking an AI model or browser tool.
