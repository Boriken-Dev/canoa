# `canoa-bitwarden` — API header

Header-file-style reference for `canoa-bitwarden`: Bitwarden's command line for Canoa in one crate, so it can be used without reading its source. It ships inside the package and is self-contained. Development documentation lives with the source at <https://github.com/Boriken-Dev/canoa>.

MSRV Rust 1.89. Version `0.0.0-alpha.0`, not published (`publish = false`). No Cargo features. `#![forbid(unsafe_code)]`. Depends on `canoa` (feature `store`), `coqui-acp-api`, `serde_json`.

## Exports

```rust
pub const LIBRARY: &str = "canoa-bitwarden";   // the name the library is registered under
pub const STORE_TYPE: &str = "secrets";   // the Store type the program is listed under
pub const RELEASES_API: &str = "https://api.github.com/repos/bitwarden/clients/releases";   // the releases the newest is read from
pub const DOWNLOADS: &str = "https://github.com/bitwarden/clients/releases/download/";   // where a release's files are served

#[derive(Debug, Default, Clone, Copy)]
pub struct BitwardenRecipe;
impl canoa::store::recipe::Recipe for BitwardenRecipe { /* every required method */ }
pub struct BitwardenExtension;   // the `StoreExtension` that adds the type `secrets` (`bitwarden_extension`)

pub fn library(...);   // the Guatiao library that offers `BitwardenStore`, the provider `bitwarden_store` over `BitwardenRecipe`
```

- `BitwardenRecipe`: Bitwarden's command line (`bw`) as a Store program, `bitwarden_store`, and the Store type `secrets` it belongs to (`BitwardenExtension`): the `cli-vYYYY.M.P` release of the clients repository whose zip lists a digest, checked against that digest. Nothing here reads a vault: the Bitwarden secret provider runs `bw`, which is on no search path, only found by name by the spawner a secret provider is handed.
- `BitwardenStore` is made by `canoa::store_provider!` and offered by this crate's library under the name `bitwarden_store`; `canoa-host` names the library in one line of `bundled.rs` and derives its runtime, `PATH`, command and secret-program lists from what the registered stores describe.
- Nothing checks a publisher's signature: the digest is the one GitHub lists.
