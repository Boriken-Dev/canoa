# `canoa-pi` — API header

Header-file-style reference for `canoa-pi`: everything Canoa offers for Pi in one crate, so it can be used without reading the source. It ships inside the package and is self-contained. Development documentation lives with the source at <https://github.com/Boriken-Dev/canoa>.

MSRV Rust 1.89. Version `0.0.0-alpha.0`, not published (`publish = false`). `#![forbid(unsafe_code)]`. Depends on `canoa` (feature `store`), `coqui-acp-api`, `coqui-acp-pi`.

## Exports

| Item | Type | What it is |
| --- | --- | --- |
| `PRESET` | `canoa::preset::SourcePreset` | `SourcePreset::new("Pi", "pi")`: run in process as the `pi` harness, searched for in `~/.pi/agent/bin` and `~/.local/bin`, and from the Store's managed copy when there is one |
| `PiRecipe` | `canoa::store::Recipe` | Pi's Store recipe: the publisher's standalone Node (checked against its `SHASUMS256.txt`), then Pi installed into it by that Node's npm with install scripts off; npm fetches the packages itself, so the host does not see those bytes |
| `PiProvider` | the `acp_harness` provider | re-exported from `coqui-acp-pi`, never redefined |
| `acp` | the `coqui-acp-pi` crate | re-exported whole, for the harness code that drives Pi directly |
| `NODE_VERSION` | `&str` | The Node the recipe installs Pi with (`24.22.2`) |

`PiRecipe` is `Debug`, `Default`, `Clone`, `Copy`. It implements `canoa::store::Recipe`; nothing runs it alone. The host's engine (`canoa_host::store::engine::Engine`) plays it, and `canoa-host` wraps it as the guatiao provider `pi_store` (provider ids are the host's, in `canoa_host::store::registry`).

## Behaviour that is easy to get wrong

- The recipe only describes: where the publisher serves the program, how a download is checked and how its version is read. It never runs a publisher's installer and changes nothing outside the version's folder; the engine downloads, checks, places and reads the version back.
- A recipe fetches only through the `InstallTools` it is handed, so a download is bounded and checked by the host.
- The preset's name and program are what a client lists and what the host looks for on `PATH`; a name `canoa.cfg` already uses is left to the config.
