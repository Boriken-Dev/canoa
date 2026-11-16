# `canoa-grok` — API header

Header-file-style reference for `canoa-grok`: everything Canoa offers for Grok in one crate, so it can be used without reading the source. It ships inside the package and is self-contained. Development documentation lives with the source at <https://github.com/Boriken-Dev/canoa>.

MSRV Rust 1.89. Version `0.0.0-alpha.0`, not published (`publish = false`). `#![forbid(unsafe_code)]`. Depends on `canoa` (feature `store`), `coqui-acp-api`.

## Exports

| Item | Type | What it is |
| --- | --- | --- |
| `PRESET` | `canoa::preset::SourcePreset` | `SourcePreset::new("Grok", "grok")`: runs `grok agent stdio`, searched for in `~/.grok/bin` |
| `GrokRecipe` | `canoa::store::Recipe` | Grok's Store recipe: one binary per platform from the publisher's storage; the publisher gives no checksum, so the plan says the download is not verified; channels `stable`, `alpha`, `enterprise` |

`GrokRecipe` is `Debug`, `Default`, `Clone`, `Copy`. It implements `canoa::store::Recipe`; nothing runs it alone. The host's engine (`canoa_host::store::engine::Engine`) plays it, and `canoa-host` wraps it as the guatiao provider `grok_store` (provider ids are the host's, in `canoa_host::store::registry`).

## Behaviour that is easy to get wrong

- The recipe only describes: where the publisher serves the program, how a download is checked and how its version is read. It never runs a publisher's installer and changes nothing outside the version's folder; the engine downloads, checks, places and reads the version back.
- A recipe fetches only through the `InstallTools` it is handed, so a download is bounded and checked by the host.
- The preset's name and program are what a client lists and what the host looks for on `PATH`; a name `canoa.cfg` already uses is left to the config.
