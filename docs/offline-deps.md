## Offline dependencies

The [deps branch](https://github.com/Boriken-Dev/canoa/tree/deps) keeps a pub/ and a cdn/ cache. Refreshing uses the [GitHub Actions workflow](https://github.com/Boriken-Dev/canoa/actions/workflows/deps.yml), which fetches resolved Dart hosted packages and versioned, SHA256-verified Unicode fallback fonts. CanvasKit stays local by using --no-web-resources-cdn.

The fonts the web engine falls back to for a glyph Roboto lacks (symbols, arrows, emoji, CJK and every script: 724 files, 20.7 MiB) are mirrored the same way, under `cdn/fonts.gstatic.com/s/`, from `tools/fallback_fonts.sha256`, which pins each file's SHA-256. `tools/mirror_deps.py` (or `python tools/fallback_fonts.py mirror DEPS_DIR`) downloads them and refuses a file that changed; after a Flutter upgrade `python tools/fallback_fonts.py pin DIR` rewrites the list from the SDK's engine, and its diff is the review. `build.py` for the web copies them from the deps checkout into `build/web/assets/fonts/fallback/` and stops, naming the file, when one is missing or differs; it never fetches them.

`tools/cdn_files.json` pins every other CDN file by host, path and SHA-256: the fonts above and Office.js with the files it loads
(124 files under `cdn/officeapis.public.onecdn.static.microsoft/1/`). `python tools/cdn_files.py mirror DEPS_DIR` (or `mirror_deps.py`)
downloads them and refuses a file that changed; `python tools/cdn_files.py pin-office DIR` reads Office.js's loader, which names the files it
asks for, downloads them and rewrites the Office.js entries, and the diff of the list is the review. `/1/` is an alias Microsoft moves, so the
digests say which copy it is. `crates/canoa-host/build.rs` carries the files from the deps checkout (`CANOA_DEPS`) into the host and checks
each against the list; it never fetches, and without the mirror it stops, naming `CANOA_OFFICE_JS=cdn` as the way to have the add-in load
Office.js from Microsoft's CDN instead. Nothing under `cdn/` is fetched by any build.

A cache is ready when manifest.json reports status complete. The Flutter SDK, engine, native toolchains and deployment rewrites for font fallbacks are separate requirements.

The mirror carries only what compilation needs: each hosted package without its `test`, `tests`, `example`, `examples`, `benchmark`, `benchmarks`, `doc`, `docs` and `.github` directories, and without any remaining file a malware or archive scanner would refuse (archives, executables, key files, pack files, opaque data). `manifest.json` records what was dropped. Git dependencies, such as `coqui-acp` and `coqui-transport-web`, are not mirrored (a bare clone is pack files): the manifest lists each one's URL and pinned revision from `app/pubspec.lock`, and pub fetches them from GitHub, so resolving them needs the network. Hosted packages resolve offline: create a git worktree for deps, point PUB_CACHE at its pub directory and run flutter pub get --offline. Never mirror tokens or private registry credentials.


### Building the desktop app from the deps branch

`python app/tool/build.py desktop` builds offline by default. It finds a checkout of the deps branch (`CANOA_DEPS`, else a detached worktree of `origin/deps` or `deps` that it adds beside the repository as `<repository>-deps`), checks that the hosted packages in its `pubspec.lock` are the ones in `app/pubspec.lock`, and seeds `target/offline-pub-cache` with them. Each git dependency is copied from the pub cache that already holds it at the locked revision (`PUB_CACHE`, else the platform's default); that cache is only read. It then runs `flutter pub get --offline` and builds with `--no-pub`.

When anything is missing (no deps checkout, a hosted package whose version or hash differs or is not mirrored, a git dependency not in the pub cache, or `pub get --offline` failing), the build says why in one line and goes online. `--online` skips the deps branch; `--offline` stops with the reason instead of going online.

A `.env` at the repository root sets `FLUTTER_ROOT`, `PUB_CACHE`, `CANOA_DEPS`, `CARGO_HOME` and `RUSTUP_HOME`; a variable already in the environment wins. See `.env.example`.

### Crates

The deps branch's `crates/` is a cargo vendor directory: every package `Cargo.lock` names, from crates.io and from each pinned git revision, as `<name>-<version>/` with a `.cargo-checksum.json`. A package's tests, examples, benchmarks and fixtures are left out, a git package has the workspace settings it inherits written into its manifest, and a binary that a build reads is written as text (an `include_bytes!` becomes an array literal, ring's prebuilt objects become hex that its `build.rs` decodes), so that no file in the branch is one a scanner would refuse.

`python tools/mirror_crates.py sync DEPS_DIR` makes `DEPS_DIR/crates` equal to `Cargo.lock` (fetching what is missing or changed and checking each crate's SHA-256 against the lock); `check DEPS_DIR` re-hashes every file and names what is missing, changed or stale; `use DEPS_DIR` writes a cargo home outside the repository whose `config.toml` replaces crates.io and each git source by `crates/` and turns the network off. Commit `crates/` to the deps branch with the pub refresh.

`build.py desktop` covers cargo the way it covers pub: when `crates/` covers `Cargo.lock` (package names, registry checksums and git commits; `check` does the whole comparison), the build runs with `CARGO_HOME` at that cargo home and `CARGO_NET_OFFLINE=true`; when it does not, the whole build goes online with the reason, and `--offline` stops instead. The native plugin starts cargo itself, so `--locked` is not passed; with the network off cargo cannot change the lock. The cargo home replaces `~/.cargo/config.toml` for that build.
