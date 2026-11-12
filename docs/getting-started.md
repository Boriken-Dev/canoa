# Getting started

## Requirements

- Rust 1.89 or later to build the core.
- Node.js 22 or later to run Chromium extension policy tests.
- Flutter SDK with Dart 3.9 or newer for the standalone application. The exact Flutter version used by AI6's experimental windowing build is **not** required for this ordinary Material 3 UI.

## Rust core

```sh
git clone https://github.com/Boriken-Dev/canoa.git
cd canoa
cargo test --workspace --all-features
cargo doc --workspace --no-deps --all-features
```

The Rust crate currently implements only an in-memory registry and explicit grants, not network identity verification.

## Desktop app

The desktop app is the whole product on one machine: it carries the host as a library, so it runs the agents canoa.cfg names (Claude, Codex, Pi or any ACP agent) itself, with no `canoa` program installed. It needs `flutter` and `cargo` on PATH, and on Linux the system libraries listed in [configuration](configuration.md#building-desktop-runners).

```sh
python app/tool/build.py desktop          # builds, then prints the path of CanoaUI (CanoaUI.exe on Windows)
python app/tool/build.py desktop --run    # runs it with flutter run instead
python app/tool/build.py desktop --debug  # a debug build; --platform windows|linux|macos names the platform (default: this machine's)
```

The first run generates the platform's runner (`flutter create`) in `app/`, names the executable `CanoaUI` and, on Windows, gives it Canoa's icon; none of that is committed. `app/pubspec.lock` is put back if Flutter rewrites it. Closing the window leaves Canoa in the tray; see [the desktop section of the app's README](https://github.com/Boriken-Dev/canoa/blob/main/app/README.md#desktop-tray-and-canoacfg) and [the host](host.md#in-the-desktop-app) for hosting and pairing.

## Web app

```sh
python app/tool/build_noise.py   # the Noise method as WebAssembly, into app/web/noise
(cd app && flutter build web --release --csp --no-web-resources-cdn)   # app/build/web; the host's policy needs --csp
canoa serve --web app/build/web                                           # served on loopback
```

The web app has no agents of its own: it connects to a host by its printed URL and peer id, proves its key, and the host's owner accepts that key with `canoa accept`. See [the web app](host.md#the-web-app). `python app/tool/build.py pwa` makes the same bundle without `--csp`, for any other static server; `--wasm` output may need COOP/COEP headers and a compatible browser. For development, `cd app && flutter run -d chrome`.

## Tests

```sh
cd app
flutter pub get
flutter analyze
flutter test
```

`flutter pub get` with an older Flutter rewrites `app/pubspec.lock`: restore it before committing.

## Chromium extension

```sh
python app/tool/build.py chromium
(cd bindings/browser/chromium && npm test)
```

Load `bindings/browser/chromium/` using `chrome://extensions` → Developer mode → Load unpacked. In a regular website, use the **Ask Canoa** context-menu action to attach its page or selected text to the local Flutter conversation. The packaging helper copies the Flutter UI to `ui/`; it does not yet connect a live ACP or MCP server. The manifest's optional host permissions and userScripts policy are not a substitute for explicit user consent.

See [browser integration](browser.md).

## Authenticated WSS smoke test

`canoa serve` with the `stdio_agent` example (`cargo build -p canoa-host --bin canoa --examples`) provides a local test endpoint that uses mutually authenticated Coqui peer proof and ACP. Name `target/debug/examples/stdio_agent` as the `program` of a `[host.agents.echo]` entry in canoa.cfg, run `canoa serve`, connect from the browser with the printed URL and peer id, and `canoa accept` the key that connects. Its only behavior is echoing messages, not invoking an AI model or browser tool.
