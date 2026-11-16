# `canoa-claude` — API header

Header-file-style reference for `canoa-claude`: everything Canoa offers for Claude Code in one crate, so it can be used without reading its source. It ships inside the package and is self-contained. Development documentation lives with the source at <https://github.com/Boriken-Dev/canoa>.

MSRV Rust 1.89. Version `0.0.0-alpha.0`, not published (`publish = false`). No Cargo features. `#![forbid(unsafe_code)]`. Depends on `canoa` (feature `store`), `coqui-acp-api`, `coqui-acp-claude`.

## Exports

```rust
pub const PRESET: SourcePreset;
// = SourcePreset::new("Claude", "claude").harness("claude")

#[derive(Debug, Default, Clone, Copy)]
pub struct ClaudeRecipe;
impl canoa::store::recipe::Recipe for ClaudeRecipe { /* every required method */ }

pub use coqui_acp_claude as acp;   // the provider crate, whole: `acp::ClaudeProvider` is the `acp_harness` provider, never redefined here
```

- `PRESET`: how the host starts Claude Code when `canoa.cfg` does not name it: run in process as the `claude` harness.
- `ClaudeRecipe`: Claude Code's Store recipe: one binary per platform from the publisher's release bucket, named by its manifest with a SHA-256 (the manifest's detached signature is not checked); channels `latest` and `stable`.
- `acp`: the `coqui-acp-claude` crate re-exported whole, so a host reaches the provider and its history helpers through this crate and needs no direct dependency. It is the only path to them.

Each item is public at this one path. `ClaudeRecipe` is not a Store provider: `canoa-host` wraps it in its engine as the guatiao provider `claude_store`.

## Gotchas

- The recipe only describes: where the publisher serves the program, how a download is checked and how its version is read. It never runs a publisher's installer and changes nothing outside the version's folder; the host's engine downloads, checks, places and reads the version back.
- A recipe fetches only through the `InstallTools` it is handed, so a download is bounded and checked by the host, and starts no process of its own (a test in `canoa-host` scans this crate for `std::process::Command`).
- The preset's name is what a client lists; a name `canoa.cfg` already uses is left to the config. Its program is `ClaudeRecipe`'s first name, which a test keeps true.
