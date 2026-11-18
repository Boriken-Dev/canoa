# `canoa-opencode` — API header

Header-file-style reference for `canoa-opencode`: everything Canoa offers for OpenCode in one crate, so it can be used without reading its source. It ships inside the package and is self-contained. Development documentation lives with the source at <https://github.com/Boriken-Dev/canoa>.

MSRV Rust 1.89. Version `0.0.0-alpha.0`, not published (`publish = false`). No Cargo features. `#![forbid(unsafe_code)]`. Depends on `canoa` (feature `store`), `coqui-acp-api`, `serde_json`.

## Exports

```rust
pub const PRESET: SourcePreset;
// = SourcePreset::new("OpenCode", "opencode").harness("opencode").search(&["~/.opencode/bin"])

#[derive(Debug, Default, Clone, Copy)]
pub struct OpenCodeRecipe;
impl canoa::store::recipe::Recipe for OpenCodeRecipe { /* every required method */ }
```

- `PRESET`: how the host starts OpenCode when `canoa.cfg` does not name it: as the built-in `opencode` harness (coqui-acp's provider runs `opencode acp` in the host's process and takes accounts), the program searched for in `~/.opencode/bin`.
- `OpenCodeRecipe`: OpenCode's Store recipe: the release's archive for the platform (`opencode-<windows|linux>-<x64|arm64>`, a `.zip` on Windows and a `.tar.gz` on Linux, each holding the one program) from the publisher's GitHub releases, checked against the SHA-256 the release lists for that file. With no version it takes GitHub's newest release; a version a person names is read from its own tag (`v<version>`). No channels. A release that lists no archive or no digest for the platform is refused.

Each item is public at this one path. `OpenCodeRecipe` is not a Store provider: `canoa-host` wraps it in its engine as the guatiao provider `opencode_store`.

## Gotchas

- The recipe only describes: where the publisher serves the program, how a download is checked and how its version is read (`opencode --version` prints the bare number). It never runs a publisher's installer and changes nothing outside the version's folder; the host's engine downloads, checks, places and reads the version back.
- A recipe fetches only through the `InstallTools` it is handed, so a download is bounded and checked by the host, and starts no process of its own (a test in `canoa-host` scans this crate for `std::process::Command`).
- The Linux archives are the glibc builds; a musl system needs the program installed by other means. GitHub's newest release is on the 1.x line; the 2.x line is reached by naming a version.
- The copy is started with `OPENCODE_DISABLE_AUTOUPDATE=1`.
- The preset's name is what a client lists; a name `canoa.cfg` already uses is left to the config. Its program is `OpenCodeRecipe`'s first name, which a test keeps true.
- OpenCode has no sandbox: the host starts it with the profile's access as the one variable `OPENCODE_PERMISSION` (a JSON object of tool to `allow`, `ask` or `deny`), from the access table in `coqui-acp-harness`. The variable is read after the person's own `opencode.json` `permission`, so it wins over it for the session.
- OpenCode reads skills from the real `~/.claude/skills` and `~/.agents/skills` of the account it runs under, whatever `HOME` is; `OPENCODE_DISABLE_EXTERNAL_SKILLS=1` stops that.
