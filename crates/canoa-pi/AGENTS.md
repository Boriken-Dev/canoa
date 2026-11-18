# `canoa-pi` — API header

Header-file-style reference for `canoa-pi`: everything Canoa offers for Pi in one crate, so it can be used without reading its source. It ships inside the package and is self-contained. Development documentation lives with the source at <https://github.com/Boriken-Dev/canoa>.

MSRV Rust 1.89. Version `0.0.0-alpha.0`, not published (`publish = false`). No Cargo features. `#![forbid(unsafe_code)]`. Depends on `canoa` (feature `store`), `coqui-acp-api`, `coqui-acp-pi`.

## Exports

```rust
pub const PRESET: SourcePreset;
// = SourcePreset::new("Pi", "pi").harness("pi").search(&["~/.pi/agent/bin", "~/.local/bin"]).managed("pi")

#[derive(Debug, Default, Clone, Copy)]
pub struct PiRecipe;
impl canoa::store::recipe::Recipe for PiRecipe { /* every required method, and `launch`, `version_command` and `finish` (the Node and the npm step) */ }

pub const NODE_VERSION: &str;   // "24.22.2", the Node Pi is installed with

pub use coqui_acp_pi as acp;   // the provider crate, whole: `acp::PiProvider` is the `acp_harness` provider and `acp::PiAccounts` the `account_provider`, never redefined here
```

- `PRESET`: how the host starts Pi when `canoa.cfg` does not name it: run in process as the `pi` harness, searched for in `~/.pi/agent/bin` and `~/.local/bin`, and from the Store's managed copy when there is one.
- `PiRecipe`: Pi's Store recipe: the publisher's standalone Node (checked against its `SHASUMS256.txt`), then Pi installed into it by that Node's own npm with install scripts off; npm fetches Pi's packages itself, from the registry, and checks each against the registry's integrity, so the host does not see those bytes.
- `acp`: the `coqui-acp-pi` crate re-exported whole, so a host reaches the provider and its history helpers through this crate and needs no direct dependency. It is the only path to them.

Each item is public at this one path. `PiRecipe` is not a Store provider: `canoa-host` wraps it in its engine as the guatiao provider `pi_store`.

## Description and library

```rust
pub const MARK: &str;                 // the contents of mark.svg: the vendor's mark as SVG paths
pub struct PiVendor;                  // an `agent_vendor` provider (id `pi_vendor`): title, program, vendor, registry id, model variable, credential standards, reported names, modes, mark
pub fn library(host: Host) -> Option<LibraryParts>;   // what the crate offers, for `Registry::register_local`: `acp::PiProvider`, `acp::PiAccounts` and the vendor description
```

The host lists the crate in `bundled.rs` and nowhere else names the agent: its name, mark and standards reach the host and the application through `providers/list`, as an outside library's would. `PRESET` names the agent it starts (`.agent("pi")`).

## Gotchas

- The recipe only describes: where the publisher serves the program, how a download is checked and how its version is read. It never runs a publisher's installer and changes nothing outside the version's folder; the host's engine downloads, checks, places and reads the version back.
- A recipe fetches only through the `InstallTools` it is handed, so a download is bounded and checked by the host, and starts no process of its own (a test in `canoa-host` scans this crate for `std::process::Command`).
- The preset's name is what a client lists; a name `canoa.cfg` already uses is left to the config. Its program is `PiRecipe`'s first name, which a test keeps true.
