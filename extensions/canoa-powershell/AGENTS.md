# `canoa-powershell` — API header

Header-file-style reference for `canoa-powershell`: PowerShell 7 for Canoa in one crate, so it can be used without reading its source. It ships inside the package and is self-contained. Development documentation lives with the source at <https://github.com/Boriken-Dev/canoa>.

MSRV Rust 1.89. Version `0.0.0-alpha.0`, not published (`publish = false`). No Cargo features. `#![forbid(unsafe_code)]`. Depends on `canoa` (feature `store`), `coqui-acp-api`, `serde_json`.

## Exports

```rust
pub const LIBRARY: &str = "canoa-powershell";   // the name the library is registered under
pub const RELEASES_API: &str = "https://api.github.com/repos/PowerShell/PowerShell/releases";   // the releases the newest is read from
pub const DOWNLOADS: &str = "https://github.com/PowerShell/PowerShell/releases/download/";   // where a release's files are served

#[derive(Debug, Default, Clone, Copy)]
pub struct PowerShellRecipe;
impl canoa::store::recipe::Recipe for PowerShellRecipe { /* every required method */ }

pub fn library(...);   // the Guatiao library that offers `PowerShellStore`, the provider `powershell_store` over `PowerShellRecipe`
```

- `PowerShellRecipe`: PowerShell 7 as a Store program, `powershell_store` (Windows and Linux): the zip or `tar.gz` of the newest release that is no prerelease, kept as `powershell` with `pwsh` on the `PATH`, checked against the SHA-256 GitHub lists for the file.
- `PowerShellStore` is made by `canoa::store_provider!` and offered by this crate's library under the name `powershell_store`; `canoa-host` names the library in one line of `bundled.rs` and derives its runtime, `PATH`, command and secret-program lists from what the registered stores describe.
- Nothing checks a publisher's signature: the digest is the one GitHub lists.
