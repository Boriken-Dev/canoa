# Getting started

## Requirements

- Rust 1.89 or later to build the crates.
- Node.js 22 or later to run Chromium extension policy tests.
- Flutter from its `main` branch at commit `c0205700e0` (`flutter --version`: 3.51.0-1.0.pre-291, Dart 3.15.0-173.0.dev) for the standalone application, with the windowing feature on (`flutter config --enable-windowing`). The app is built and tested on that commit; the stable channel is too old, and a later `main` may need changes.

## Rust crates

```sh
git clone https://github.com/Boriken-Dev/canoa.git
cd canoa
cargo test --workspace --all-features
cargo doc --workspace --no-deps --all-features
```

The workspace holds `canoa` (the registry of services and client grants), `canoa-host` (the `canoa` program: it serves agents to paired keys, the web app, and `canoa portal`), `canoa-ffi` (the host as a library behind a C ABI) and `canoa-launch` (the portable Windows launcher). Peer identity, pairing and transport come from the coqui crates the host builds on. `cargo build -p canoa-host --bin canoa` makes the program; see [the host](host.md).

## Desktop app

The desktop app is the whole product on one machine: it carries the host as a library, so it runs the agents canoa.cfg names (Claude, Codex, Pi or any ACP agent) itself, with no `canoa` program installed. It needs `flutter` and `cargo` on PATH, and on Linux the system libraries listed in [configuration](configuration.md#building-desktop-runners).

```sh
python app/tool/build.py desktop          # builds, then prints the path of CanoaUI (CanoaUI.exe on Windows)
python app/tool/build.py desktop --run    # runs it with flutter run instead
python app/tool/build.py desktop --debug  # a debug build; --platform windows|linux|macos names the platform (default: this machine's)
python app/tool/build.py desktop --portable   # one file: CanoaUI.exe on Windows, an AppImage on Linux; see [one file per platform](portable.md)
```

The first run generates the platform's runner (`flutter create`) in `app/`, names the executable `CanoaUI` and, on Windows, gives it Canoa's icon; none of that is committed. `app/pubspec.lock` is put back if Flutter rewrites it. Closing the window leaves Canoa in the tray; see [the desktop section of the app's README](https://github.com/Boriken-Dev/canoa/blob/main/app/README.md#desktop-tray-and-canoacfg) and [the host](host.md#in-the-desktop-app) for hosting and pairing.

## Web app

```sh
python app/tool/build_noise.py   # the Noise method as WebAssembly, into app/web/noise
(cd app && flutter build web --release --csp --no-web-resources-cdn)   # app/build/web; the host's policy needs --csp
canoa serve --web app/build/web                                           # served on loopback
```

The web app has no agents of its own: it connects to a host by its printed URL and peer id, proves its key, and the host's owner accepts that key with `canoa accept`. See [the web app](host.md#the-web-app). `python app/tool/build.py pwa` makes the same bundle without `--csp`, for any other static server; `--wasm` output may need COOP/COEP headers and a compatible browser. For development, `cd app && flutter run -d chrome`.

## In a container

`packaging/container/Containerfile` builds the Linux host (`canoa`, with the web app and Office.js built in) and the web app in one image, from Boriken Dev's own copies of what it needs, and `build.sh` runs it and copies the results out:

```sh
sh packaging/container/build.sh          # the image `canoa`; canoa and site/ in ./out
podman run --rm canoa --version
```

`CONTAINER_ENGINE=docker` works the same, and `--print` shows the commands. The pins are build arguments, `FLUTTER_COMMIT` and `DEPS_COMMIT`, and the environment overrides them (`build.sh` passes `FLUTTER_COMMIT`, `FLUTTER_VERSION_TAG`, `FLUTTER_VERSION_TAG_COMMIT`, `DEPS_COMMIT`, `FLUTTER_REPOSITORY`, `DEPS_REPOSITORY` and `FLUTTER_STORAGE_BASE_URL` through):

- **Flutter** is cloned from [`Boriken-Dev/flutter`](https://github.com/Boriken-Dev/flutter), a mirror with the blobs a scanner refuses removed and `bin/internal/engine.version` written in, so its commits are not upstream's. The default `33d7f282aa` is upstream `c0205700e0`, the commit above. The mirror has no `3.51.0-0.2.pre` tag, which Flutter counts commits from to get its version (`3.51.0-1.0.pre-291`, what `app/pubspec.yaml` asks for), so the build makes that tag at the mirror's copy of upstream's `bc6e01a3ac` (`FLUTTER_VERSION_TAG`, `FLUTTER_VERSION_TAG_COMMIT`); a new `FLUTTER_COMMIT` may need both moved.
- **Crates, hosted packages, the fallback fonts and Office.js** come from the `deps` branch at `DEPS_COMMIT`, which mirrors the `Cargo.lock` and `app/pubspec.lock` of the commit it names (`manifest.json`). The host is built `--offline --locked` with `tools/mirror_crates.py use` as its cargo home, and the app's packages are resolved `--offline --enforce-lockfile`. Move `DEPS_COMMIT` with the lockfiles: a deps commit that does not cover them stops the build before it compiles.
- **The app's git packages** (`coqui_acp` and the others in `app/pubspec.lock`) are cloned from `github.com/Boriken-Dev` at the revisions the lock names; the deps branch does not carry them.

Four steps reach beyond those: the base image and Debian packages, `flutter precache --web` (the Dart SDK and the web engine, from Flutter's storage; `FLUTTER_STORAGE_BASE_URL` names another server), and the Noise method's WebAssembly, whose crate has a lock of its own that the deps branch does not mirror (rustup's `wasm32` target, `wasm-bindgen-cli` and its crates). Every other step runs with the network off. The image's architecture is the machine's own: an x86-64 image is built on an x86-64 machine (or with `-- --platform linux/amd64`), with no change to the files.

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
(cd clients/browser/chromium && npm test)
```

Load `clients/browser/chromium/` using `chrome://extensions` → Developer mode → Load unpacked. In a regular website, use the **Ask Canoa** context-menu action to attach its page or selected text to the local Flutter conversation. The packaging helper copies the Flutter UI to `ui/`. The extension connects the chat to a host over a proven link and offers an agent the browser's tools once a person grants them; see [browser integration](browser.md). The manifest's optional host permissions and userScripts policy are not a substitute for explicit user consent.

See [browser integration](browser.md).

## Authenticated WSS smoke test

`canoa serve` with the `stdio_agent` example (`cargo build -p canoa-host --bin canoa --examples`) provides a local test endpoint that uses mutually authenticated Coqui peer proof and ACP. Name `target/debug/examples/stdio_agent` as the `program` of a `[host.agents.echo]` entry in canoa.cfg, run `canoa serve`, connect from the browser with the printed URL and peer id, and `canoa accept` the key that connects. Its only behavior is echoing messages, not invoking an AI model or browser tool.
