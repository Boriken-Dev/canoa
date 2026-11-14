# Contributing to coqui-transport-guatiao

## Licence

This project is distributed under the **Mozilla Public License 2.0** (`LICENSE`). Its
copyleft is per file:

- **Using coqui-transport-guatiao imposes nothing on your code.** Depend on the crate and ship a
  closed-source application. The only obligation is that this crate's own source stays
  available to the people you ship it to, which it is, here.
- **Changing this crate's files does carry an obligation.** A modified file must be published
  under the MPL 2.0. Your files beside it are yours, under whatever licence you choose.
- A combined work may be distributed under terms of your choosing (MPL 2.0 section 3.3), and
  MPL 2.0 remains compatible with the GPL.

## Contributor licence agreement

A first pull request needs a one-time signature. The bot will comment with the exact sentence
to reply with; read [`CLA.md`](CLA.md) first. It exists so the project can be relicensed later
without tracking down every past contributor. You keep the copyright in everything you write.

## Before you open a pull request

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features
```

`cargo test` builds the native-library example the library test loads; a narrow run of that test needs
`cargo build --examples --features pipe` first. The Python provider's tests need a Python 3.9 or later with its
shared library (`COQUI_TRANSPORT_TEST_PYTHON` names one); without it they are ignored, and a run counts them as
not run.

The kinds' description, schemas and Python module are generated. After a change to a kind or a model:

```bash
COQUI_TRANSPORT_GUATIAO_WRITE_KINDS=1 cargo test --test kinds
PYTHONPATH=<guatiao checkout>/bindings/python/guatiao/src python python/write_kinds.py
```

The floor hashes in `tests/kinds.rs` change only in a commit that means to break every provider already built.

All checks must pass. [`AGENTS.md`](AGENTS.md) is the crate's API header and ships in the package; keep it current with
the public API in the same commit. Files are LF, on every platform. The Rust version is 1.89.
