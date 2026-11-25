# `canoa-node` — API header

Header-file-style reference for `canoa-node`: Node.js for Canoa in one crate, so it can be used without reading its source. It ships inside the package and is self-contained. Development documentation lives with the source at <https://github.com/Boriken-Dev/canoa>.

MSRV Rust 1.89. Version `0.0.0-alpha.0`, not published (`publish = false`). No Cargo features. `#![forbid(unsafe_code)]`. Depends on `canoa` (feature `store`), `coqui-acp-api`, `guatiao`, `guatiao-intake`, `serde_json`.

## Exports

```rust
pub const LIBRARY: &str = "canoa-node";   // the name the library is registered under
pub const NODE_ORIGIN: &str = "https://nodejs.org/dist/";   // where the builds and their digests are served

#[derive(Debug, Default, Clone, Copy)]
pub struct NodeRecipe;
impl canoa::store::recipe::Recipe for NodeRecipe { /* every required method */ }

pub fn library(...);   // the Guatiao library that offers `NodeStore`, the provider `node_store` over `NodeRecipe`
```

- `NodeRecipe`: Node.js as a Store program: `node_store`, nodejs.org's own builds (channels `lts` and `current`) unpacked into the Store's folder as `node`. The releases are the catalog of its scope (`node`): `NodeReleases` (a `catalog_source`, id `node_releases`, default label `nodejs`) reads `index.json` for the release each channel is at and each release folder's `SHASUMS256.txt.asc`, taking the checksums only from the text a signature by the release team verifies (`KEYS`, `keys/release-keys.asc`, the eight active releasers; `read_catalog(origin, fetch, demanded)`, `signed(message, demanded)`).
- `NodeStore` is made by `canoa::store_provider!` and offered by this crate's library under the name `node_store`; `canoa-host` names the library in one line of `bundled.rs` and derives its runtime, `PATH`, command and secret-program lists from what the registered stores describe.
- The signature is the release team's: a checksums file whose signature fails, or is by a key an entry's `keys` do not name, refuses the catalog. Keys of releasers who join are added to `keys/release-keys.asc` with the extension's release.
