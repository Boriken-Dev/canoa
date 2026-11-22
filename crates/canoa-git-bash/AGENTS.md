# `canoa-git-bash` — API header

Header-file-style reference for `canoa-git-bash`: Git Bash for Canoa in one crate, so it can be used without reading its source. It ships inside the package and is self-contained. Development documentation lives with the source at <https://github.com/Boriken-Dev/canoa>.

MSRV Rust 1.89. Version `0.0.0-alpha.0`, not published (`publish = false`). No Cargo features. `#![forbid(unsafe_code)]`. Depends on `canoa` (feature `store`), `coqui-acp-api`, `serde_json`.

## Exports

```rust
pub const LIBRARY: &str = "canoa-git-bash";   // the name the library is registered under
pub const RELEASES_API: &str = "https://api.github.com/repos/git-for-windows/git/releases";   // the releases the newest is read from
pub const DOWNLOADS: &str = "https://github.com/git-for-windows/git/releases/download/";   // where a release's files are served

#[derive(Debug, Default, Clone, Copy)]
pub struct GitBashRecipe;
impl canoa::store::recipe::Recipe for GitBashRecipe { /* every required method */ }

pub fn library(...);   // the Guatiao library that offers `GitBashStore`, the provider `git_bash_store` over `GitBashRecipe`
```

- `GitBashRecipe`: Git for Windows' portable release as a Store program, `git_bash_store` (Windows only): `bin/bash.exe`, `sh.exe` and `git.exe` kept as `git-bash`, each file checked against the SHA-256 GitHub lists for it.
- `GitBashStore` is made by `canoa::store_provider!` and offered by this crate's library under the name `git_bash_store`; `canoa-host` names the library in one line of `bundled.rs` and derives its runtime, `PATH`, command and secret-program lists from what the registered stores describe.
- Nothing checks a publisher's signature: the digest is the one GitHub lists.
