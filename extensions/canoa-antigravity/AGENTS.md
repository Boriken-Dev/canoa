# `canoa-antigravity` — API header

Header-file-style reference for `canoa-antigravity`: everything Canoa offers for Antigravity in one crate, so it can be used without reading its source. It ships inside the package and is self-contained. Development documentation lives with the source at <https://github.com/Boriken-Dev/canoa>.

MSRV Rust 1.89. Version `0.0.0-alpha.0`, not published (`publish = false`). No Cargo features. `#![forbid(unsafe_code)]`. Depends on `canoa` (features `store`, `vendor`, `usage`), `coqui-acp-api`, `coqui-acp-harness` (feature `antigravity`), `serde_json`.

## Exports

```rust
pub const PRESET: SourcePreset;
pub const ACCESS: Target;   // coqui_acp_harness::Target
// = SourcePreset::new("Antigravity", "agy").agent("antigravity").harness("antigravity").args(&["acp"]).search(&["%LOCALAPPDATA%/agy/bin"]).usage("antigravity")

#[derive(Debug, Default, Clone, Copy)]
pub struct AgyRecipe;
impl canoa::store::recipe::Recipe for AgyRecipe { /* every required method */ }
```

- `PRESET`: how the host runs Antigravity when `canoa.cfg` does not name it: through the `antigravity` harness provider (`agy acp`), searched for in `%LOCALAPPDATA%/agy/bin`, with the `antigravity` usage reader.
- `ACCESS`: how an access setting (sandbox, network, approvals) is said in Antigravity's own words, under the AOS harness id `antigravity`; `canoa-host` finds it by that id (`bundled.rs`) to say what became of each setting.
- `AgyRecipe`: Antigravity's Store recipe: the build the publisher's manifest names, SHA-512 from it; only the newest build (a pinned version is unsupported).

Each item is public at this one path. `AgyRecipe` is made a Store provider by `canoa::store_provider!`: `AgyStore` is the guatiao provider `agy_store` over it, offered by this crate's library.

## Description and library

```rust
pub const MARK: &str;                 // the contents of mark.svg: the vendor's mark as SVG paths
pub struct AgyUsage;                   // a `usage_reader` provider (id `antigravity_usage`, reader id `antigravity`, config `AgySettings`, `stored`): with no process (`root` 0) reads the account and quota from agy's stored login and Google's service; in a session, from the loopback port agy's own language server opens; bucket ids `agy:...`; settle 10 s, retries after 2, 4, 8 and 16 s, cached 15 s, a private agent 40 s
pub struct AgyAccounts;                // the `account_provider` provider (id `antigravity_accounts`, config `AgySettings`): the automatic `local` account and the sign-in `sign_in`, "Sign in to Antigravity"
pub struct AgySettings { executable, renew_poll_ms, command_timeout_ms, login_args, login_poll_ms, login_hosts, home, gemini_dir, keyring, keyring_timeout_ms, client_program, token_url, userinfo_url, service, consumer_service, gcp_service, user_agent, timeout_ms, max_answer_bytes }   // Default; FromValue from an entry's `account_settings`
pub const SIGN_IN: &str = "sign_in";
pub const NOT_SIGNED_IN: &str;         // a stored-login reading's failure when no login is stored
pub struct AgyVendor;                  // an `agent_vendor` provider (id `antigravity_vendor`): title, program, vendor, registry id, model variable, credential standards, reported names, modes, mark
pub fn library(host: Host) -> Option<LibraryParts>;   // what the crate offers, for `Registry::register_local`: AntigravityProvider (`acp_harness`, id `antigravity`), AgyAccounts (`account_provider`, id `antigravity_accounts`), AgyVendor, GeminiVendor, AgyStore, and AgyUsage
```

The host lists the crate in `bundled.rs` and nowhere else names the agent: its name, mark and standards reach the host and the application through `providers/list`, as an outside library's would. `PRESET` names the agent it starts (`.agent("antigravity")`).

## Gotchas

- `AgyUsage` answers `describe` only; each `open` makes a reading that remembers the port that answered and the models the agent listed in `session/new` (`UsageReading::opened` takes that answer's JSON text). The host owns when to read, and hands the reading the processes of the agent's tree as a `process_members` object (`canoa::usage_reader`); a reading that finds no port says it is worth asking again (`Retry::NotYet`), one whose port did not answer says `Retry::NoAnswer`, any other failure is final. A successful reading also says who is signed in (`Reading::account`: the email, else the name, and the plan). With `root` 0 the reading starts nothing: it reads agy's stored login (the keyring entry `gemini`/`antigravity`, else the `.gemini` files) and asks Google's service, through `coqui_acp_harness::antigravity::AgyLogin`, one reader kept per set of settings in the process, so a token it refreshed in memory and the OAuth client that refreshed it serve the next reading; no login is `NOT_SIGNED_IN` (`Retry::Never`), a login or answer that cannot be read says which step failed (the account is still said when it is known). In a session (`root` the agent's process) it reads the language server's port through `AgyQuota`, as before. The account provider (`antigravity_accounts`) wraps coqui-acp's stdio accounts of `agy` for its kinds, renewals, watches and models, and signs in by `sign_in`, "Sign in to Antigravity": with a login stored it runs nothing and answers the `local` account named by its email; else it runs `<executable> <login_args>` (`agy acp --login`), offers the person the first page it prints on one of `login_hosts`, asks for an authorization code when the program asks for one and types it in, looks at the stored login every `login_poll_ms` and ends the program once one is there; a program that ends with none stored is `login refused`. The reader and the account provider are configured from the entry's `account_settings`; a test points `home` at its own folder and sets `keyring = false` and the three addresses to a stand-in.

- The recipe only describes: where the publisher serves the program, how a download is checked and how its version is read. It never runs a publisher's installer and changes nothing outside the version's folder; the host's engine downloads, checks, places and reads the version back.
- A recipe fetches only through the `InstallTools` it is handed, so a download is bounded and checked by the host, and starts no process of its own (a test in `canoa-host` scans this crate for `std::process::Command`).
- The preset's name is what a client lists; a name `canoa.cfg` already uses is left to the config. Its program is `AgyRecipe`'s first name, which a test keeps true.
- The host starts the program with the profile's access: `--sandbox` for a workspace or read-only sandbox and `--dangerously-skip-permissions` for `approvals = "never"`, appended after the preset's arguments by `canoa launch`.
