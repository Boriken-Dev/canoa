# Getting started

## Requirements

- Rust 1.89 or later to build the crates.
- Node.js 22 or later to run Chromium extension policy tests.
- Flutter from its `main` branch at commit `c0205700e0` (`flutter --version`: 3.51.0-1.0.pre-291, Dart 3.15.0-173.0.dev) for the standalone application, with the windowing feature on (`flutter config --enable-windowing`). The app is built and tested on that commit; the stable channel is too old, and a later `main` may need changes. `tools/flutter.pin` names it once (the commit, the version and the engine); see [the Flutter SDK](#the-flutter-sdk) for getting it without a request to Google's storage.

## The Flutter SDK

Every build of {{ name }} uses the SDK `tools/sdk.lock` names (the rows `app flutter`, `flutter-upstream`, `flutter-version`, `flutter-engine` and `dart`, which `tools/flutter.pin` repeats for `build.sh` and `sdk check` holds equal), and none of them fetches it, or its engine, from Google. The framework is [`Boriken-Dev/flutter`](https://github.com/Boriken-Dev/flutter), a mirror of Flutter `main` with the blobs a scanner refuses removed, at `33d7f282aa` (upstream `c0205700e0`); the engine, the Dart SDK and the web artifacts are the assets of that repository's release `engine-<hash>`, Google's files unchanged with a SHA-256 manifest.

```sh
python tools/build.py sdk install --groups app --prefix ~/sdk   # the mirror's checkout at the pin, precached from the release, in ~/sdk/git/flutter/<commit>
python tools/build.py flutter precache-check --flutter-root ~/sdk/git/flutter/33d7f282aaef --platform web,linux   # fails when an SDK is not the pin or lacks a platform's engine files
python tools/build.py sdk check                                  # fails when tools/flutter.pin, the Containerfile or another file disagrees with tools/sdk.lock
python tools/flutter_pin.py check                                # fails when a workflow or this page names another SDK
```

`sdk install` needs `git`, Python and the network to github.com only; `[flutter] precache` of `tools/build.cfg` names the platforms it precaches (web, windows and linux). An SDK already at upstream's `c0205700e0` is the pin too, when its engine is the pinned one. The same SDK is in the images `ghcr.io/boriken-dev/flutter-web`, `flutter-linux` and `flutter-windows` (tag `33d7f282aa`, the first ten digits of the mirror's commit), which `packaging/container/Containerfile` and the workflows use: `FROM` one to build in a container, or start from it as a development environment. To move to another Flutter, change `tools/flutter.pin` and the rows of `tools/sdk.lock`, publish the new engine's release, build the images, and let `sdk check` and `flutter_pin.py check` name what is left.

## Rust crates

```sh
git clone https://github.com/Boriken-Dev/canoa.git
cd canoa
cargo test --workspace --all-features
cargo doc --workspace --no-deps --all-features
```

The workspace holds `canoa` (the registry of services and client grants), `canoa-host` (the `{{ program }}` program: it serves agents to paired keys, the web app, and `{{ program }} portal`), `canoa-ffi` (the host as a library behind a C ABI) and `canoa-launch` (the portable Windows launcher). Peer identity, pairing and transport come from the coqui crates the host builds on. `cargo build -p canoa-host --bin canoa` makes the program; see [the host](host.md).

## Desktop app

The desktop app is the whole product on one machine: it carries the host as a library, so it runs the agents {{ config_file }} names (Claude, Codex, Pi or any ACP agent) itself, with no `{{ program }}` program installed. It needs `flutter` and `cargo` on PATH, and on Linux the system libraries listed in [configuration](configuration.md#building-desktop-runners).

```sh
python app/tool/build.py desktop          # builds, then prints the path of {{ executable }} ({{ executable }}.exe on Windows)
python app/tool/build.py desktop --run    # runs it with flutter run instead
python app/tool/build.py desktop --debug  # a debug build; --platform windows|linux|macos names the platform (default: this machine's)
python app/tool/build.py desktop --portable   # one file: {{ executable }}.exe on Windows, an AppImage on Linux; see [one file per platform](portable.md)
```

The first run generates the platform's runner (`flutter create`) in `app/`, names the executable `{{ executable }}` and, on Windows, gives it {{ name }}'s icon; none of that is committed. `app/pubspec.lock` is put back if Flutter rewrites it. Closing the window leaves {{ name }} in the tray; see [the desktop section of the app's README](https://github.com/Boriken-Dev/canoa/blob/main/app/README.md#desktop-tray-and-canoacfg) and [the host](host.md#in-the-desktop-app) for hosting and pairing.

## Web app

```sh
python app/tool/build_noise.py   # the Noise method as WebAssembly, into app/web/noise
(cd app && flutter build web --release --csp --no-web-resources-cdn)   # app/build/web; the host's policy needs --csp
{{ program }} serve --web app/build/web                                           # served on loopback
```

The web app has no agents of its own: it connects to a host by its printed URL and peer id, proves its key, and the host's owner accepts that key with `{{ program }} accept`. See [the web app](host.md#the-web-app). `python app/tool/build.py pwa` makes the same bundle without `--csp`, for any other static server; `--wasm` output may need COOP/COEP headers and a compatible browser. For development, `cd app && flutter run -d chrome`.

## In a container

`packaging/container/Containerfile` builds the Linux host (`{{ program }}`, with the web app and Office.js built in) and the web app in one image, from Boriken Dev's own copies of what it needs, and `build.sh` runs it and copies the results out:

```sh
sh packaging/container/build.sh          # the image `canoa`; {{ program }} and site/ in ./out
podman run --rm canoa --version
```

`CONTAINER_ENGINE=docker` works the same, and `--print` shows the commands. The pins are the Flutter pin, which `build.sh` reads from `tools/flutter.pin` and passes as the build argument `FLUTTER_IMAGE_TAG`, and `DEPS_COMMIT`; the environment overrides them (`build.sh` passes `FLUTTER_IMAGE_TAG`, `DEPS_COMMIT` and `DEPS_REPOSITORY` through):

- **Flutter** is the image `ghcr.io/boriken-dev/flutter-web` for the pin, which holds the [`Boriken-Dev/flutter`](https://github.com/Boriken-Dev/flutter) checkout at `33d7f282aa` (upstream `c0205700e0`, the commit above) with its tags, and the engine, the Dart SDK and the web artifacts from the mirror's release. The build adds Debian's `python3` and points Flutter's storage at a dead address, so a step that needed Google would fail rather than fetch; `site_build.py` stops when the SDK is not the pin's.
- **Crates, hosted and git packages, the fallback fonts and Office.js** come from the `deps` branch at `DEPS_COMMIT`, which mirrors the `Cargo.lock`, `app/web_noise/Cargo.lock` and `app/pubspec.lock` of the commit it names (`manifest.json`). The host is built `--offline --locked` with `tools/build.py deps use` as its cargo home; the Noise method is built from `crates-web_noise/` and `wasm-bindgen-cli` installed from `crates-wasm-bindgen-cli/` (`app/tool/build_noise.py --deps`); and the app's packages, the git ones from `pub/git/` included, are resolved `--offline --enforce-lockfile`. Move `DEPS_COMMIT` with the lockfiles: a deps commit that does not cover them stops the build before it compiles.

Three steps reach beyond the deps branch, none of them Google's or pub.dev's: the base images and Debian packages, the Flutter image (`ghcr.io`) and rustup's `wasm32-unknown-unknown` target (`static.rust-lang.org`), each in a stage of its own (`base`, `flutter`, `wasm_target`), as is the fetch of the deps branch itself. Every other step runs with the network off, and `podman build --network=none` builds everything after those stages. The image's architecture is the machine's own: an x86-64 image is built on an x86-64 machine (or with `-- --platform linux/amd64`), with no change to the files.

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

Load `clients/browser/chromium/` using `chrome://extensions` → Developer mode → Load unpacked. In a regular website, use the **Ask {{ name }}** context-menu action to attach its page or selected text to the local Flutter conversation. The packaging helper copies the Flutter UI to `ui/`. The extension connects the chat to a host over a proven link and offers an agent the browser's tools once a person grants them; see [browser integration](browser.md). The manifest's optional host permissions and userScripts policy are not a substitute for explicit user consent.

See [browser integration](browser.md).

## Authenticated WSS smoke test

`{{ program }} serve` with the `stdio_agent` example (`cargo build -p canoa-host --bin canoa --examples`) provides a local test endpoint that uses mutually authenticated Coqui peer proof and ACP. Name `target/debug/examples/stdio_agent` as the `program` of a `[host.agents.echo]` entry in {{ config_file }}, run `{{ program }} serve`, connect from the browser with the printed URL and peer id, and `{{ program }} accept` the key that connects. Its only behavior is echoing messages, not invoking an AI model or browser tool.
