---
brand_names: false
---

## Offline dependencies

The [deps branch](https://github.com/Boriken-Dev/canoa/tree/deps) keeps a pub/ and a cdn/ cache. Refreshing uses the [GitHub Actions workflow](https://github.com/Boriken-Dev/canoa/actions/workflows/deps.yml), which fetches resolved Dart hosted packages and versioned, SHA256-verified Unicode fallback fonts. CanvasKit stays local by using --no-web-resources-cdn.

The fonts the web engine falls back to for a glyph Roboto lacks (symbols, arrows, emoji, CJK and every script: 724 files, 20.7 MiB) are mirrored the same way, under `cdn/fonts.gstatic.com/s/`, from `tools/fallback_fonts.sha256`, which pins each file's SHA-256. `tools/mirror_deps.py` (or `python tools/fallback_fonts.py mirror DEPS_DIR`) downloads them and refuses a file that changed; after a Flutter upgrade `python tools/fallback_fonts.py pin DIR` rewrites the list from the SDK's engine, and its diff is the review. `build.py` for the web copies them from the deps checkout into `build/web/assets/fonts/fallback/` and stops, naming the file, when one is missing or differs; it never fetches them.

`tools/cdn_files.json` pins the fonts above by host, path and SHA-256 (`python tools/cdn_files.py DEPS_DIR`, or `mirror_deps.py`, downloads
them and refuses a file that changed).

Office.js is mirrored whole, from its repository: `cdn/office-js/` holds the tree of one commit of github.com/OfficeDev/office-js, with its
`LICENSE.md`, exactly as published (772 files, 83.6 MiB at commit `498e88261bc5`). Nothing in it is renamed, filtered or rewritten, and it is
not trimmed to what compiles: the owner asked for the library as published, and Microsoft's license terms let it be distributed only
unmodified. `tools/office_js.json` pins the commit and each file's SHA-256; `python tools/office_js.py mirror DEPS_DIR` (or `mirror_deps.py`) fetches that
commit's source archive over HTTPS (no clone) and refuses a tree that differs, and `python tools/office_js.py pin [--commit SHA]` moves the pin
to the head of the `release` branch, the diff of the list being the review. `manifest.json` records the commit and every file's hash.
`crates/canoa-host/build.rs` carries the files the add-in's pane loads, and `LICENSE.md`, from the deps checkout (`CANOA_DEPS`) into the host and
checks each against the list; it never fetches, and without the mirror it stops, naming `CANOA_OFFICE_JS=cdn` as the way to have the add-in
load Office.js from Microsoft's CDN instead. Nothing under `cdn/` is fetched by any build.

A cache is ready when manifest.json reports status complete. The Flutter SDK, engine, native toolchains and deployment rewrites for font fallbacks are separate requirements.

The mirror carries only what compilation needs: each hosted package without its `test`, `tests`, `example`, `examples`, `benchmark`, `benchmarks`, `doc`, `docs` and `.github` directories, and without any remaining file a malware or archive scanner would refuse (archives, executables, key files, pack files, opaque data). `manifest.json` records what was dropped. Git dependencies, such as `coqui-acp` and `coqui-transport-web`, are carried as source trees, not clones (a bare clone is pack files): `pub/git/<repository>-<revision>/` holds, at the revision `app/pubspec.lock` names, the path each package of that repository uses (`git archive`'s files, so the packages that depend on each other by path keep their layout), filtered like a hosted package; the manifest's `gitPackages` lists them. Pub reads a git package from its cache without touching git when the checkout folder `<repository>-<revision>` is there, so `build.py` seeds its offline pub cache with these folders (a `.git/pub-packages` list, which pub writes after cloning, and a bare repository of the remote that holds one empty commit and a replace ref from the locked revision to it, because pub asks that repository for the revision before it trusts the checkout and git's packs are not carried), and `flutter pub get --offline --enforce-lockfile` resolves hosted and git packages from the branch, with the lock unchanged. Never mirror tokens or private registry credentials.


### Building the desktop app from the deps branch

`python app/tool/build.py desktop` builds offline by default. It finds a checkout of the deps branch (`CANOA_DEPS`, else a detached worktree of `origin/deps` or `deps` that it adds beside the repository as `<repository>-deps`), checks that the hosted packages in its `pubspec.lock` are the ones in `app/pubspec.lock`, and seeds `target/offline-pub-cache` with them. Each git dependency is copied from the branch's `pub/git/`, else from the pub cache that already holds it at the locked revision (`PUB_CACHE`, else the platform's default); that cache is only read. It then runs `flutter pub get --offline` and builds with `--no-pub`.

When anything is missing (no deps checkout, a hosted package whose version or hash differs or is not mirrored, a git dependency neither on the branch nor in the pub cache, or `pub get --offline` failing), the build says why in one line and goes online. `--online` skips the deps branch; `--offline` stops with the reason instead of going online.

A `.env` at the repository root sets `FLUTTER_ROOT`, `PUB_CACHE`, `CANOA_DEPS`, `CARGO_HOME` and `RUSTUP_HOME`; a variable already in the environment wins. See `.env.example`.

### Crates

The deps branch's `crates/` is a cargo vendor directory: every package `Cargo.lock` names, from crates.io and from each pinned git revision, as `<name>-<version>/` with a `.cargo-checksum.json`. A package's tests, examples, benchmarks and fixtures are left out, a git package has the workspace settings it inherits written into its manifest, and a binary that a build reads is written as text (an `include_bytes!` becomes an array literal, ring's prebuilt objects become hex that its `build.rs` decodes), so that no file in the branch is one a scanner would refuse.

A vendor directory holds one copy of a name and version, and two locks may need two sources of one package, so each lock has its own folder, all named in `VENDORS` in `tools/mirror_crates.py`:

| Folder | Lock | Used by |
| --- | --- | --- |
| `crates/` | `Cargo.lock` | the host, `canoa-ffi`, the app's native plugin |
| `crates-web_noise/` | `app/web_noise/Cargo.lock` | `app/tool/build_noise.py`: the Noise bridge pins `coqui-peer-noise` at a revision the workspace does not use |
| `crates-wasm-bindgen-cli/` | the `Cargo.lock` packaged with `wasm-bindgen-cli` at the version `app/web_noise/Cargo.lock` pins for `wasm-bindgen` | `cargo install --offline --locked wasm-bindgen-cli` |

`python tools/mirror_crates.py sync DEPS_DIR` makes each folder equal to its lock (fetching what is missing or changed and checking each crate's SHA-256 against the lock; the command line's own checksum against crates.io's index); `check DEPS_DIR` re-hashes every file and names what is missing, changed or stale; `use DEPS_DIR [--vendor FOLDER]` writes a cargo home outside the repository whose `config.toml` replaces crates.io and each git source by that folder and turns the network off (`crates/` unless `--vendor` says another). `--vendor` limits any of them to one folder. `sync` also has the branch's `.gitattributes` mark each folder `-text`, since each file's hash is in its `.cargo-checksum.json`. Commit the folders to the deps branch with the pub refresh (`tools/repo.py` runs `sync` and `check`, and a change to `Cargo.lock`, `app/web_noise/Cargo.lock` or `app/pubspec.lock` triggers it).

`python app/tool/build_noise.py --deps DEPS_DIR` (or `CANOA_DEPS`) builds the Noise method from `crates-web_noise/` with the network off, and `--install-bindgen` installs the command line from `crates-wasm-bindgen-cli/` with `cargo install --offline --locked`; without `--deps` both come from crates.io and GitHub as before. The `wasm32-unknown-unknown` target is rustup's and is not on the branch.

`build.py desktop` covers cargo the way it covers pub: when `crates/` covers `Cargo.lock` (package names, registry checksums and git commits; `check` does the whole comparison), the build runs with `CARGO_HOME` at that cargo home and `CARGO_NET_OFFLINE=true`; when it does not, the whole build goes online with the reason, and `--offline` stops instead. The native plugin starts cargo itself, so `--locked` is not passed; with the network off cargo cannot change the lock. The cargo home replaces `~/.cargo/config.toml` for that build.
