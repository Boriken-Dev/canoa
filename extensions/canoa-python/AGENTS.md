# `canoa-python` — API header

Header-file-style reference for `canoa-python`: Python for Canoa in one crate, so it can be used without reading its source. It ships inside the package and is self-contained. Development documentation lives with the source at <https://github.com/Boriken-Dev/canoa>.

MSRV Rust 1.89. Version `0.0.0-alpha.0`, not published (`publish = false`). No Cargo features. `#![forbid(unsafe_code)]`. Depends on `canoa` (feature `store`), `coqui-acp-api`, `guatiao`, `guatiao-intake`, `serde_json`.

## Exports

```rust
pub const LIBRARY: &str = "canoa-python";   // the name the library is registered under
pub const PYTHON_RELEASES: &str = "https://github.com/astral-sh/python-build-standalone/releases/";   // where the builds are listed and served

#[derive(Debug, Default, Clone, Copy)]
pub struct PythonRecipe;
impl canoa::store::recipe::Recipe for PythonRecipe { /* every required method */ }

pub fn library(...);   // the Guatiao library that offers `PythonStore`, the provider `python_store` over `PythonRecipe`
```

- `PythonRecipe`: CPython as a Store program: `python_store`, the standalone builds of the python-build-standalone releases (channels `3.13`, `3.14`, `3.12`, `3.11`) unpacked as `python`. The releases are the catalog of its scope (`python`): `PythonReleases` (a `catalog_source`, id `python_releases`, default label `astral`) reads the newest release's `SHA256SUMS` and lists each final build with its SHA-256, the newest of each minor series a channel.
- `PythonStore` is made by `canoa::store_provider!` and offered by this crate's library under the name `python_store`; `canoa-host` names the library in one line of `bundled.rs` and derives its runtime, `PATH`, command and secret-program lists from what the registered stores describe.
- The digest is the release's own: no publisher signature exists for the builds.
