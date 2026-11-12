# `canoa-launch` — API header

Header-file-style reference for `canoa-launch`: the portable Windows `CanoaUI.exe` and the packer that feeds it. Development documentation lives with the source at <https://github.com/Boriken-Dev/canoa>; the release recipe is `docs/portable.md`.

MSRV Rust 1.89. Version `0.0.0-alpha.0`, not published (`publish = false`). Two binaries and a library. `#![forbid(unsafe_code)]`. Depends on `sha2`, `zip` and `flate2`; on Windows `windows-permissions` (owner of a folder) and `win-msgbox` (a failed launch's message box).

## Programs

| Program | Does |
| --- | --- |
| `canoa-launch` | The launcher. Windows subsystem `windows` (no console). Starts the app out of `<LOCALAPPDATA>\Canoa\<version>\` with its own arguments, waits, exits with the app's status; on failure exit 1 with the reason on stderr and, on Windows, in a message box |
| `canoa-pack <app folder> <version> <payload.zip>` | Packs the folder, reads it back as the launcher will, requires `CanoaUI.exe`, writes the archive and prints its SHA-256. Exit 2 on usage, 1 on failure |

Build the launcher with the payload: `CANOA_LAUNCH_PAYLOAD=<payload.zip> cargo build --release -p canoa-launch --bin canoa-launch`. `build.rs` compiles the file and its SHA-256 in; unset, the launcher carries nothing and exits 1 with "carries no app" (what `cargo test` builds). `app/tool/portable.py` does both steps.

Environment the launcher reads: `LOCALAPPDATA` (Windows; `XDG_DATA_HOME`, else `$HOME/.local/share`, elsewhere, so tests run on any platform), which must be absolute; `CANOA_LAUNCH_NO_DIALOG` (set: no message box, for automated runs).

## Library (`canoa_launch`)

```rust
pub fn pack(dir: &Path, version: &str) -> Result<Vec<u8>>;      // deterministic zip: manifest entry first, then every file, deflated
pub fn install(base: &Path, payload: &Payload<'_>) -> Result<Installed>;
pub fn prune(base: &Path, current: &str) -> Vec<String>;         // names removed; best effort
pub fn differs(dir: &Path, manifest: &Manifest) -> Result<Option<String>>;  // why a folder is not exactly the manifest
pub fn sha256_hex(bytes: &[u8]) -> String;

pub struct Payload<'a> { pub bytes: &'a [u8], pub sha256: &'a str }   // .manifest() checks the digest, reads the manifest
pub struct Installed { pub dir: PathBuf, pub manifest: Manifest, pub reused: bool }  // .program(name) only if listed
pub struct Manifest { pub version: String, pub entries: Vec<Entry> }  // new/parse/text/digest/entry; canonical text only
pub struct Entry { pub path: String, pub size: u64, pub sha256: [u8; 32] }
pub enum Error { Io { context, source }, Refused(String), Payload(String) }
pub const MARKER: &str = "canoa-launch.manifest"; pub const APP_PROGRAM: &str = "CanoaUI.exe";
pub const KEEP_VERSIONS: usize = 2; pub const MAX_FILES: usize = 20_000; pub const MAX_BYTES: u64 = 4 << 30;
```

`Refused` means a safety check failed and nothing was run (link, junction, other owner, group- or world-writable folder on Unix, a program not in the manifest). `Payload` means the archive or its manifest is not usable.

## Behaviour that is easy to get wrong

- A version folder is reused only when every file hashes to the manifest, no file beyond the manifest and the manifest copy exists, nothing in it is a link, and the manifest copy is identical. Anything else is replaced whole: unpacked beside, then renamed in. A rebuild with the same version replaces in place, and fails while the old folder is in use.
- Unpacking checks each file's size and SHA-256 while writing; a mismatch removes the temporary folder and leaves no version folder.
- Names are validated by one rule at pack time, manifest parse time and unpack time (`names.rs`): relative, `/`, no `.`/`..`, no drive, `:` stream, device name, trailing dot or space, control character; names differing only in case are one name; no file may be named like the manifest copy.
- `prune` removes only a real, user-owned directory holding a manifest copy whose version equals the folder's name (plus its own `.unpack-*` older than an hour, `.old-*`, `.gone-*`). It renames a folder before deleting, so a folder still in use stays.
- Ownership: a folder is accepted when its owner is the owner of a file this process creates beside it (on Unix that alone); on Windows also the owner of the folder that holds it, Administrators and the system account, read with `GetNamedSecurityInfo` through `windows-permissions`. An elevated window makes Administrators-owned files, so one person starting Canoa elevated and not elevated must both work. Parents of `%LOCALAPPDATA%\Canoa` are not checked (a redirected profile is legitimate). ACLs are not read.
- The launcher links the C runtime statically (`.cargo/config.toml` here, so only a cargo run from this folder gets it: `portable.py` and the tests run it so; an env `RUSTFLAGS` overrides it, and the launcher test reads the import table to notice) and sets `DependentLoadFlags` to System32 in `build.rs`. The app starts with `current_dir` its unpacked folder; a relative `CANOA_CONFIG` is made absolute first, the only working-folder dependence found in the app.
- The base folder's owner is checked against the probe's owner, the owner of the user's data folder (its parent), Administrators and the system account, never against itself.
- The launcher is `windows_subsystem = "windows"`: with no redirected handles the app's console output goes nowhere. The nested test build (`tests/launcher.rs`) builds the launcher into `target/tmp/launcher-with-payload`.
