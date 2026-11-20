# `canoa-antigravity` — API header

Header-file-style reference for `canoa-antigravity`: everything Canoa offers for Antigravity in one crate, so it can be used without reading its source. It ships inside the package and is self-contained. Development documentation lives with the source at <https://github.com/Boriken-Dev/canoa>.

MSRV Rust 1.89. Version `0.0.0-alpha.0`, not published (`publish = false`). No Cargo features. `#![forbid(unsafe_code)]`. Depends on `canoa` (features `store`, `vendor`, `usage`), `coqui-acp-api`, `coqui-acp-harness` (feature `antigravity`), `serde_json`.

## Exports

```rust
pub const PRESET: SourcePreset;
pub const ACCESS: Target;   // coqui_acp_harness::Target
// = SourcePreset::new("Antigravity", "agy").args(&["acp"]).search(&["%LOCALAPPDATA%/agy/bin"]).usage("antigravity")

#[derive(Debug, Default, Clone, Copy)]
pub struct AgyRecipe;
impl canoa::store::recipe::Recipe for AgyRecipe { /* every required method */ }
```

- `PRESET`: how the host starts Antigravity when `canoa.cfg` does not name it: runs `agy acp`, searched for in `%LOCALAPPDATA%/agy/bin`, with the `antigravity` usage reader.
- `ACCESS`: how an access setting (sandbox, network, approvals) is said in Antigravity's own words, under the AOS harness id `antigravity`; `canoa-host` finds it by that id (`bundled.rs`) to say what became of each setting.
- `AgyRecipe`: Antigravity's Store recipe: the build the publisher's manifest names, SHA-512 from it; only the newest build (a pinned version is unsupported).

Each item is public at this one path. `AgyRecipe` is made a Store provider by `canoa::store_provider!`: `AgyStore` is the guatiao provider `agy_store` over it, offered by this crate's library.

## Description and library

```rust
pub const MARK: &str;                 // the contents of mark.svg: the vendor's mark as SVG paths
pub struct AgyUsage;                   // a `usage_reader` provider (id `antigravity_usage`, reader id `antigravity`): reads the account's quota from the loopback port agy's own language server opens once a session is open; bucket ids `agy:...`; settle 10 s, retries after 2, 4, 8 and 16 s, cached 15 s, a private agent 40 s
pub struct AgyVendor;                  // an `agent_vendor` provider (id `antigravity_vendor`): title, program, vendor, registry id, model variable, credential standards, reported names, modes, mark
pub fn library(host: Host) -> Option<LibraryParts>;   // what the crate offers, for `Registry::register_local`: the vendor description of Antigravity and of Gemini CLI (`GeminiVendor`: Google's other agent has no crate of its own), its Store and its usage reader, no harness provider
```

The host lists the crate in `bundled.rs` and nowhere else names the agent: its name, mark and standards reach the host and the application through `providers/list`, as an outside library's would. `PRESET` names the agent it starts (`.agent("antigravity")`).

## Gotchas

- `AgyUsage` answers `describe` only; each `open` makes a reading that remembers the port that answered and the models the agent listed in `session/new` (`UsageReading::opened` takes that answer's JSON text). The host owns when to read, and hands the reading the processes of the agent's tree as a `process_members` object (`canoa::usage_reader`); a reading that finds no port says it is worth asking again (`Retry::NotYet`), one whose port did not answer says `Retry::NoAnswer`, any other failure is final. The crate does not start the agent: the host starts one only to read, from the entry's program and arguments.

- The recipe only describes: where the publisher serves the program, how a download is checked and how its version is read. It never runs a publisher's installer and changes nothing outside the version's folder; the host's engine downloads, checks, places and reads the version back.
- A recipe fetches only through the `InstallTools` it is handed, so a download is bounded and checked by the host, and starts no process of its own (a test in `canoa-host` scans this crate for `std::process::Command`).
- The preset's name is what a client lists; a name `canoa.cfg` already uses is left to the config. Its program is `AgyRecipe`'s first name, which a test keeps true.
- The host starts the program with the profile's access: `--sandbox` for a workspace or read-only sandbox and `--dangerously-skip-permissions` for `approvals = "never"`, appended after the preset's arguments by `canoa launch`.
