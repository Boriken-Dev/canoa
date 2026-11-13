## Offline dependencies

The [deps branch](https://github.com/Boriken-Dev/canoa/tree/deps) keeps a pub/ and a cdn/ cache. Refreshing uses the [GitHub Actions workflow](https://github.com/Boriken-Dev/canoa/actions/workflows/deps.yml), which fetches resolved Dart hosted packages and versioned, SHA256-verified Unicode fallback fonts. CanvasKit stays local by using --no-web-resources-cdn.

The initial branch is only a scaffold. A cache is ready when manifest.json reports status complete. The Flutter SDK, engine, native toolchains and deployment rewrites for font fallbacks are separate requirements.

The mirror carries only what compilation needs: each hosted package without its `test`, `tests`, `example`, `examples`, `benchmark`, `benchmarks`, `doc`, `docs` and `.github` directories, and without any remaining file a malware or archive scanner would refuse (archives, executables, key files, pack files, opaque data). `manifest.json` records what was dropped. Git dependencies, such as `coqui-acp` and `coqui-web`, are not mirrored (a bare clone is pack files): the manifest lists each one's URL and pinned revision from `app/pubspec.lock`, and pub fetches them from GitHub, so resolving them needs the network. Hosted packages resolve offline: create a git worktree for deps, point PUB_CACHE at its pub directory and run flutter pub get --offline. Never mirror tokens or private registry credentials.


### Building the desktop app from the deps branch

`python app/tool/build.py desktop` builds offline by default. It finds a checkout of the deps branch (`CANOA_DEPS`, else a detached worktree of `origin/deps` or `deps` that it adds beside the repository as `<repository>-deps`), checks that the hosted packages in its `pubspec.lock` are the ones in `app/pubspec.lock`, and seeds `target/offline-pub-cache` with them. Each git dependency is copied from the pub cache that already holds it at the locked revision (`PUB_CACHE`, else the platform's default); that cache is only read. It then runs `flutter pub get --offline` and builds with `--no-pub`.

When anything is missing (no deps checkout, a hosted package whose version or hash differs or is not mirrored, a git dependency not in the pub cache, or `pub get --offline` failing), the build says why in one line and goes online. `--online` skips the deps branch; `--offline` stops with the reason instead of going online.

A `.env` at the repository root sets `FLUTTER_ROOT`, `PUB_CACHE`, `CANOA_DEPS`, `CARGO_HOME` and `RUSTUP_HOME`; a variable already in the environment wins. See `.env.example`.

### Crates

The deps branch's `crates/` is a cargo vendor directory: every package `Cargo.lock` names, from crates.io and from each pinned git revision, as `<name>-<version>/` with a `.cargo-checksum.json`. A package's tests, examples, benchmarks and fixtures are left out, a git package has the workspace settings it inherits written into its manifest, and a binary that a build reads is written as text (an `include_bytes!` becomes an array literal, ring's prebuilt objects become hex that its `build.rs` decodes), so that no file in the branch is one a scanner would refuse.

`python tools/mirror_crates.py sync DEPS_DIR` makes `DEPS_DIR/crates` equal to `Cargo.lock` (fetching what is missing or changed and checking each crate's SHA-256 against the lock); `check DEPS_DIR` re-hashes every file and names what is missing, changed or stale; `use DEPS_DIR` writes a cargo home outside the repository whose `config.toml` replaces crates.io and each git source by `crates/` and turns the network off. Commit `crates/` to the deps branch with the pub refresh.

`build.py desktop` covers cargo the way it covers pub: when `crates/` covers `Cargo.lock` (package names, registry checksums and git commits; `check` does the whole comparison), the build runs with `CARGO_HOME` at that cargo home and `CARGO_NET_OFFLINE=true`; when it does not, the whole build goes online with the reason, and `--offline` stops instead. The native plugin starts cargo itself, so `--locked` is not passed; with the network off cargo cannot change the lock. The cargo home replaces `~/.cargo/config.toml` for that build.
