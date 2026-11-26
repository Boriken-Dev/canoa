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

pub const SCRIPT_LIBRARIES: &[&str];   // the folders of the JavaScript libraries below
pub const SCRIPT_FILES: &[(&str, &str)];   // the Bitwarden secret provider (`secret_provider`, JavaScript) and the SDK modules it imports, as (path, text)

pub fn library(...);   // the Guatiao library that offers `BitwardenStore`, the provider `bitwarden_store` over `BitwardenRecipe`
```

- `BitwardenRecipe`: Bitwarden's command line (`bw`) as a Store program, `bitwarden_store`, and the Store type `secrets` it belongs to (`BitwardenExtension`). The releases are the catalog of its scope (`bw`): `BitwardenReleases` (a `catalog_source`, id `bitwarden_releases`, default label `github`, `repo = "bitwarden/clients"`) reads the clients repository's last hundred releases, keeps the `cli-vYYYY.M.P` ones and lists each one's zip (`bw-<platform>-<version>.zip`, or from 2026.11.0 `bw-oss-<platform>-<version>.zip`) with the SHA-256 GitHub lists for it; a newer release with no listed file is said in a notice beside the one offered. Nothing here reads a vault: the Bitwarden secret provider runs the program when the person has set it up.
- `BitwardenStore` is made by `canoa::store_provider!` and offered by this crate's library under the name `bitwarden_store`; `canoa-host` names the library in one line of `bundled.rs` and derives its runtime, `PATH`, command and secret-program lists from what the registered stores describe.
- The digest is the one GitHub lists: the publisher's signature is not checked.
- `SCRIPT_FILES` is the Bitwarden secret provider: a `secret_provider` written in JavaScript (`script/bitwarden/`), reading one part of a vault item through `bw get`. A host that serves JavaScript libraries keeps the files in a folder and offers the provider with its other secret providers; a host built without this crate has no Bitwarden vault. The SDK modules under `script/bitwarden/sdk/` are copies made by `tools/sync_host_sdk.py`.
