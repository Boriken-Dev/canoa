# `canoa-node` — API header

Header-file-style reference for `canoa-node`: Node.js for Canoa in one crate, so it can be used without reading its source. It ships inside the package and is self-contained. Development documentation lives with the source at <https://github.com/Boriken-Dev/canoa>.

MSRV Rust 1.89. Version `0.0.0-alpha.0`, not published (`publish = false`). No Cargo features. `#![forbid(unsafe_code)]`. Depends on `canoa` (feature `store`), `coqui-acp-api`.

## Exports

```rust
pub const LIBRARY: &str = "canoa-node";   // the name the library is registered under
pub const NODE_ORIGIN: &str = "https://nodejs.org/dist/";   // where the builds and their digests are served

#[derive(Debug, Default, Clone, Copy)]
pub struct NodeRecipe;
impl canoa::store::recipe::Recipe for NodeRecipe { /* every required method */ }

pub fn library(...);   // the Guatiao library that offers `NodeStore`, the provider `node_store` over `NodeRecipe`
```

- `NodeRecipe`: Node.js as a Store program: `node_store`, nodejs.org's own builds (channels `lts` and `current`), each file checked against the `SHASUMS256.txt` of its release and unpacked into the Store's folder as `node`.
- `NodeStore` is made by `canoa::store_provider!` and offered by this crate's library under the name `node_store`; `canoa-host` names the library in one line of `bundled.rs` and derives its runtime, `PATH`, command and secret-program lists from what the registered stores describe.
- Nothing checks a publisher's signature: the digest is the release's own.
