# `canoa-codex` — API header

Header-file-style reference for `canoa-codex`: everything Canoa offers for Codex in one crate, so it can be used without reading its source. It ships inside the package and is self-contained. Development documentation lives with the source at <https://github.com/Boriken-Dev/canoa>.

MSRV Rust 1.89. Version `0.0.0-alpha.0`, not published (`publish = false`). No Cargo features. `#![forbid(unsafe_code)]`. Depends on `canoa` (feature `store`), `coqui-acp-api`, `serde_json`, `coqui-acp-codex`.

## Exports

```rust
pub const LOOK: &str;
// = the contents of looks/chatgpt.json: ChatGPT's look as the JSON document `looks/list` serves

pub const PRESET: SourcePreset;
// = SourcePreset::new("Codex", "codex").harness("codex")

#[derive(Debug, Default, Clone, Copy)]
pub struct CodexRecipe;
impl canoa::store::recipe::Recipe for CodexRecipe { /* every required method, and `self_updates` */ }

pub use coqui_acp_codex as acp;   // the provider crate, whole: `acp::CodexProvider` is the `acp_harness` provider and `acp::CodexAccounts` the `account_provider`, never redefined here
```

- `PRESET`: how the host starts Codex when `canoa.cfg` does not name it: run in process as the `codex` harness.
- `CodexRecipe`: Codex's Store recipe: the `codex-package` tar.gz from the release site, SHA-256 from the release's checksums and the channel's digest; no setting stops its own updater.
- `acp`: the `coqui-acp-codex` crate re-exported whole, so a host reaches the provider and its history helpers through this crate and needs no direct dependency. It is the only path to them.

Each item is public at this one path. `CodexRecipe` is not a Store provider: `canoa-host` wraps it in its engine as the guatiao provider `codex_store`.

## Gotchas

- `LOOK` is colour tokens only, an original palette in the spirit of the product: no logo, no font. The host serves it unchanged; its token names are those of `canoa-host`'s `looks/list`.
- The recipe only describes: where the publisher serves the program, how a download is checked and how its version is read. It never runs a publisher's installer and changes nothing outside the version's folder; the host's engine downloads, checks, places and reads the version back.
- A recipe fetches only through the `InstallTools` it is handed, so a download is bounded and checked by the host, and starts no process of its own (a test in `canoa-host` scans this crate for `std::process::Command`).
- The preset's name is what a client lists; a name `canoa.cfg` already uses is left to the config. Its program is `CodexRecipe`'s first name, which a test keeps true.
