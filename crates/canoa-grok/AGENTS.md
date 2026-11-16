# `canoa-grok` — API header

Header-file-style reference for `canoa-grok`: everything Canoa offers for Grok in one crate, so it can be used without reading its source. It ships inside the package and is self-contained. Development documentation lives with the source at <https://github.com/Boriken-Dev/canoa>.

MSRV Rust 1.89. Version `0.0.0-alpha.0`, not published (`publish = false`). No Cargo features. `#![forbid(unsafe_code)]`. Depends on `canoa` (feature `store`), `coqui-acp-api`.

## Exports

```rust
pub const PRESET: SourcePreset;
// = SourcePreset::new("Grok", "grok").args(&["agent", "stdio"]).search(&["~/.grok/bin"])

#[derive(Debug, Default, Clone, Copy)]
pub struct GrokRecipe;
impl canoa::store::recipe::Recipe for GrokRecipe { /* every required method */ }
```

- `PRESET`: how the host starts Grok when `canoa.cfg` does not name it: runs `grok agent stdio`, searched for in `~/.grok/bin`.
- `GrokRecipe`: Grok's Store recipe: one binary per platform from the publisher's storage; the publisher gives no checksum, so the plan says the download is not verified; channels `stable`, `alpha`, `enterprise`.

Each item is public at this one path. `GrokRecipe` is not a Store provider: `canoa-host` wraps it in its engine as the guatiao provider `grok_store`.

## Gotchas

- The recipe only describes: where the publisher serves the program, how a download is checked and how its version is read. It never runs a publisher's installer and changes nothing outside the version's folder; the host's engine downloads, checks, places and reads the version back.
- A recipe fetches only through the `InstallTools` it is handed, so a download is bounded and checked by the host, and starts no process of its own (a test in `canoa-host` scans this crate for `std::process::Command`).
- The preset's name is what a client lists; a name `canoa.cfg` already uses is left to the config. Its program is `GrokRecipe`'s first name, which a test keeps true.
