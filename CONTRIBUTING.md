# Contributing to Canoa

## Licence

This project is distributed under the **Mozilla Public License 2.0** (`LICENSE`). Its copyleft is
per file: changing one of this project's files means publishing that file under the MPL 2.0, and
your own files beside it stay yours, under whatever licence you choose. A combined work may be
distributed under terms of your choosing (MPL 2.0 section 3.3), and MPL 2.0 remains compatible with
the GPL.

## Contributor licence agreement

A first pull request needs a one-time signature. The bot will comment with the exact sentence to
reply with; read [`CLA.md`](CLA.md) first. It exists so the project can be relicensed later without
tracking down every past contributor. You keep the copyright in everything you write.

## Before you open a pull request

Run the checks for what you changed:

```bash
# Rust
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features

# Flutter
(cd app && flutter analyze && flutter test)

# Chromium extension (Node 22 or later)
(cd bindings/browser/chromium && npm test)

# ACP echo fixture (Node 22 or later, OpenSSL)
(cd examples/acp-echo && npm install && npm test)

# Documentation
mkdocs build --strict
```

Files are LF, on every platform. The Rust version is 1.89, checked in CI beside stable.
