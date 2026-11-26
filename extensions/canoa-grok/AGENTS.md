# `canoa-grok` — API header

Header-file-style reference for `canoa-grok`: everything Canoa offers for Grok in one crate, so it can be used without reading its source. It ships inside the package and is self-contained. Development documentation lives with the source at <https://github.com/Boriken-Dev/canoa>.

MSRV Rust 1.89. Version `0.0.0-alpha.0`, not published (`publish = false`). No Cargo features. `#![forbid(unsafe_code)]`. Depends on `canoa` (feature `store`), `coqui-acp-api`, `guatiao`, `guatiao-intake`, `serde_json`.

## Exports

```rust
pub const PRESET: SourcePreset;
pub const ACCESS: Target;   // coqui_acp_harness::Target
// = SourcePreset::new("Grok", "grok").agent("grok").harness("grok").args(&["agent", "stdio"]).search(&["~/.grok/bin"])

#[derive(Debug, Default, Clone, Copy)]
pub struct GrokRecipe;
impl canoa::store::recipe::Recipe for GrokRecipe { /* every required method */ }
```

- `PRESET`: how the host runs Grok when `canoa.cfg` does not name it: through the `grok` harness provider (`grok agent stdio`), searched for in `~/.grok/bin`.
- `ACCESS`: how an access setting (sandbox, network, approvals) is said in Grok's own words, under the AOS harness id `grok`; `canoa-host` finds it by that id (`bundled.rs`) to say what became of each setting.
- `GrokRecipe`: Grok's Store recipe: one binary per platform from the publisher's storage; channels `stable`, `alpha`, `enterprise`. The releases are the catalog of its scope (`grok`): `GrokReleases` (a `catalog_source`, id `grok_releases`, default label `xai`) reads each channel's one-line file and lists the builds. The publisher gives no checksum, so every file is listed as one nothing checks, and it is offered as not verified (`[store.grok.catalogs.xai]` with `provider = "grok_releases"`).

Each item is public at this one path. `GrokRecipe` is made a Store provider by `canoa::store_provider!`: `GrokStore` is the guatiao provider `grok_store` over it, offered by this crate's library.

## Description and library

```rust
pub const MARK: &str;                 // the contents of mark.svg: the vendor's mark as SVG paths
pub struct GrokVendor;                  // an `agent_vendor` provider (id `grok_vendor`): title, program, vendor, registry id, model variable, credential standards, reported names, modes, mark
pub fn library(host: Host) -> Option<LibraryParts>;   // what the crate offers, for `Registry::register_local`: GrokProvider (`acp_harness`, id `grok`), GrokAccounts (`account_provider`, id `grok_accounts`), GrokVendor, and GrokStore
```

The host lists the crate in `bundled.rs` and nowhere else names the agent: its name, mark and standards reach the host and the application through `providers/list`, as an outside library's would. `PRESET` names the agent it starts (`.agent("grok")`).

## Gotchas

- The recipe only describes: where the publisher serves the program, how a download is checked and how its version is read. It never runs a publisher's installer and changes nothing outside the version's folder; the host's engine downloads, checks, places and reads the version back.
- A recipe fetches only through the `InstallTools` it is handed, so a download is bounded and checked by the host, and starts no process of its own (a test in `canoa-host` scans this crate for `std::process::Command`).
- The preset's name is what a client lists; a name `canoa.cfg` already uses is left to the config. Its program is `GrokRecipe`'s first name, which a test keeps true.
- The host starts the program with the profile's access: `GROK_SANDBOX` in its environment (`off`, `workspace` or `read-only`) and, for `approvals = "never"`, `_meta` `yoloMode` on each session request. The preset's arguments are not changed (Grok's options go before the word `stdio`, which is why the access uses neither).
