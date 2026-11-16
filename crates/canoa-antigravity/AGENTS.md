# `canoa-antigravity` — API header

Header-file-style reference for `canoa-antigravity`: everything Canoa offers for Antigravity in one crate, so it can be used without reading its source. It ships inside the package and is self-contained. Development documentation lives with the source at <https://github.com/Boriken-Dev/canoa>.

MSRV Rust 1.89. Version `0.0.0-alpha.0`, not published (`publish = false`). No Cargo features. `#![forbid(unsafe_code)]`. Depends on `canoa` (feature `store`), `coqui-acp-api`, `serde_json`.

## Exports

```rust
pub const PRESET: SourcePreset;
// = SourcePreset::new("Antigravity", "agy").args(&["acp"]).search(&["%LOCALAPPDATA%/agy/bin"]).usage("antigravity")

#[derive(Debug, Default, Clone, Copy)]
pub struct AgyRecipe;
impl canoa::store::recipe::Recipe for AgyRecipe { /* every required method */ }
```

- `PRESET`: how the host starts Antigravity when `canoa.cfg` does not name it: runs `agy acp`, searched for in `%LOCALAPPDATA%/agy/bin`, with the `antigravity` usage reader.
- `AgyRecipe`: Antigravity's Store recipe: the build the publisher's manifest names, SHA-512 from it; only the newest build (a pinned version is unsupported).

Each item is public at this one path. `AgyRecipe` is not a Store provider: `canoa-host` wraps it in its engine as the guatiao provider `agy_store`.

## Gotchas

- The recipe only describes: where the publisher serves the program, how a download is checked and how its version is read. It never runs a publisher's installer and changes nothing outside the version's folder; the host's engine downloads, checks, places and reads the version back.
- A recipe fetches only through the `InstallTools` it is handed, so a download is bounded and checked by the host, and starts no process of its own (a test in `canoa-host` scans this crate for `std::process::Command`).
- The preset's name is what a client lists; a name `canoa.cfg` already uses is left to the config. Its program is `AgyRecipe`'s first name, which a test keeps true.
