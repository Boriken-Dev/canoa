# `canoa-claude` — API header

Header-file-style reference for `canoa-claude`: everything Canoa offers for Claude Code in one crate, so it can be used without reading its source. It ships inside the package and is self-contained. Development documentation lives with the source at <https://github.com/Boriken-Dev/canoa>.

MSRV Rust 1.89. Version `0.0.0-alpha.0`, not published (`publish = false`). No Cargo features. `#![forbid(unsafe_code)]`. Depends on `canoa` (feature `store`), `coqui-acp-api`, `coqui-acp-claude`.

`ClaudeVendor` also answers `AgentVendor`'s per-agent methods: `account_variables`, `login_file`, `shell_advice` and `sessions`/`session_history` (the program's own session folder, read for the host's `history/importable` and `history/import`), and how the host gives a session what Claude Code takes as a local plugin loaded through `_meta`: `skills_delivery` (the `canoa-profile` plugin), `hooks_delivery` (the hooks plugin whose commands run the host's forwarder), `hook_support`, and `access_target` (`claude-code`, applied per session).

## Exports

```rust
pub const LOOK: &str;
// = the contents of looks/claude.json: Claude's look as the JSON document `looks/list` serves

pub const PRESET: SourcePreset;
pub const ACCESS: Target;   // coqui_acp_harness::Target
// = SourcePreset::new("Claude", "claude").harness("claude")

#[derive(Debug, Default, Clone, Copy)]
pub struct ClaudeRecipe;
pub struct ClaudeStore;   // the `program_store` provider `claude_store` over it (`canoa::store_provider!`), offered by `library`
impl canoa::store::recipe::Recipe for ClaudeRecipe { /* every required method */ }

pub use coqui_acp_claude as acp;   // the provider crate, whole: `acp::ClaudeProvider` is the `acp_harness` provider and `acp::ClaudeAccounts` the `account_provider`, never redefined here
```

- `PRESET`: how the host starts Claude Code when `canoa.cfg` does not name it: run in process as the `claude` harness.
- `ACCESS`: how an access setting (sandbox, network, approvals) is said in Claude Code's own words, under the AOS harness id `claude-code`; `canoa-host` finds it by that id (`bundled.rs`) to say what became of each setting.
- `ClaudeRecipe`: Claude Code's Store recipe: one binary per platform from the publisher's release bucket, named by its manifest with a SHA-256 (the manifest's detached signature is not checked); channels `latest` and `stable`.
- `acp`: the `coqui-acp-claude` crate re-exported whole, so a host reaches the provider and its history helpers through this crate and needs no direct dependency. It is the only path to them.

Each item is public at this one path. `ClaudeRecipe` is made a Store provider by `canoa::store_provider!`: `ClaudeStore` is the guatiao provider `claude_store` over it, offered by this crate's library.

## Description and library

```rust
pub const MARK: &str;                 // the contents of mark.svg: the vendor's mark as SVG paths
pub struct ClaudeVendor;                  // an `agent_vendor` provider (id `claude_vendor`): title, program, vendor, registry id, model variable, credential standards, reported names, modes, mark, and its look
pub fn library(host: Host) -> Option<LibraryParts>;   // what the crate offers, for `Registry::register_local`: `acp::ClaudeProvider`, `acp::ClaudeAccounts` and the vendor description
```

The host lists the crate in `bundled.rs` and nowhere else names the agent: its name, mark and standards reach the host and the application through `providers/list`, as an outside library's would. `PRESET` names the agent it starts (`.agent("claude")`).

## Gotchas

- `LOOK` is colour tokens only, an original palette in the spirit of the product: no logo, no font. The host serves it unchanged; its token names are those of `canoa-host`'s `looks/list`.
- The recipe only describes: where the publisher serves the program, how a download is checked and how its version is read. It never runs a publisher's installer and changes nothing outside the version's folder; the host's engine downloads, checks, places and reads the version back.
- A recipe fetches only through the `InstallTools` it is handed, so a download is bounded and checked by the host, and starts no process of its own (a test in `canoa-host` scans this crate for `std::process::Command`).
- The preset's name is what a client lists; a name `canoa.cfg` already uses is left to the config. Its program is `ClaudeRecipe`'s first name, which a test keeps true.
