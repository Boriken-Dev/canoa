# `canoa-host` — API header

Header-file-style reference for the `canoa-host` crate, whose command-line program is `canoa`: every public item with its signature, contract and gotchas, so the crate can be used without reading its source. It ships inside the package and is self-contained. Development documentation lives with the source at <https://github.com/Boriken-Dev/canoa>.

MSRV Rust 1.89. Version `0.0.0-alpha.0`, not published (`publish = false`): its git dependencies are not released. It depends on `canoa`, `clap`, `coqui-acp`, `coqui-acp-api`, `coqui-acp-bridge`, `coqui-acp-claude`, `coqui-acp-codex`, `coqui-acp-harness` (`kind`), `coqui-acp-history` and `coqui-acp-provider` and `coqui-acp-secrets` (coqui-acp `34de3e9`), `coqui-aos` (`815aa3c`), `coqui-agentic-peer` (`8dad1766`, which brings `coqui-peer-noise`), `coqui-peer`, `coqui-peer-webcrypto`, `coqui-transport-pipe` (`1bf161f`), `coqui-transport` and `coqui-transport-web` (`websocket`), `coqui-builder` (the error a listener builder returns), `guatiao` (`provider`, `load`), `guatiao-serde` (no format), `httparse`, `indexmap`, `percent-encoding`, `rustls` (`ring`), `serde`, `serde_json`, `toml`, `tracing` and `tracing-subscriber`, and for accounts `keyring-core` with the platform's store (`windows-native-keyring-store`, `apple-native-keyring-store`, `zbus-secret-service-keyring-store`), `aes-gcm`, `zeroize`, `time` (`parsing`) and `rpassword`.

| Feature | Adds | Needed for | Default |
| --- | --- | --- | --- |
| `web` | the site `CANOA_WEB_DIST` names at build time, built into the binary; `web::BUILT_IN` is `true` | serving the web app with no directory | off |

`CANOA_WEB_DIST` must be an absolute directory holding `index.html` (a `flutter build web` output): with `web` on and it unset, relative or without an index, the build stops. Names starting with a dot are left out; a link leading out of the directory stops the build. `--all-features` therefore needs it set too.

The root re-exports `SITE_META`, `ACCOUNT_META`, `CACHE_META`, `CACHE_LIFETIMES`, `cache_lifetime_name`, `cache_lifetime_named`, `AccountConfig`, `AccountStore`, `StoreConfig`, `Config`, `ConfigError`, `Entry`, `EntryKind`, `HarnessOptions`, `Listen`, `Origin`, `Settings`, `UsageSource`, `WebConfig`, `Harness`, `Holder`, `Home`, `HomeLock`, `LockError`, `Peer`, `PeerState`, `PeersError`, `HostLine`, `ProviderError`, `ProviderOptions`, `serve_stdio`, `Listening`, `ServeError`, `ServeOptions`, `Serving`, `Stopper`, `Local`, `LocalError`, `Link`, `LinkError`, `ToolSpec`, `WebError`, `WebServer` and `WebStopper`. Public modules: `cli`, `config`, `harness`, `home`, `options`, `peers`, `pipe`, `profile`, `provider`, `serve`, `tool`, `web`.

No `unsafe` code (`#![forbid(unsafe_code)]`). Loading a provider library or source runs its code; see `serve_stdio`.

## canoa.cfg (`canoa_host::config`)

The host reads `[host]`, `[usage] limit_phrases` (`Config::limit_phrases`: `None` when unset, which means coqui-acp-api's `DEFAULT_LIMIT_PHRASES`; a list replaces them, `[]` infers nothing, an empty or blank phrase is refused) and `[permissions] protected` (`Config::protected_paths`: a string with a pattern a line, or a list of strings; `None` when unset; it is handed to the built-in Pi source and to nothing else); the app's tables (`desktop`, `network`, `assets`, `appearance`, `agents`, `overlays`, `profile`, and `[usage] warn_permille`) are not read. Every table the host reads refuses a key it does not know.

```toml
[host]
origins = ["https://canoa.example"]  # optional: browser origins that may connect; empty or absent lets any, and the proof decides
account_store = "os"                  # optional: where accounts keep their credentials, "os" (the default) or "file"; see Accounts

[host.agents.NAME]                    # a program speaking ACP on its standard streams
program = "gemini"                    # this or harness, one of the two; found as the OS finds a command
args = ["--acp"]
profiles = ["review", "plain"]        # optional: [host.profiles] a client may choose among, the first by default
usage = "antigravity"                 # optional, with program: read the agent's quota too; only "antigravity" (agy acp); see Usage
title = "Claude for review"             # optional: what a client shows for this entry instead of its name; 1 to 64 characters, not blank, no control characters (MAX_TITLE)

[host.agents.OTHER]                   # an acp_harness provider chosen by id, run by the host itself; see Serving
harness = "claude"                    # "claude", "codex" (built in) or the id of an outside provider that library or source loads; takes no args
executable = "/opt/claude/bin/claude" # claude and codex: a name found as the OS finds a command, or a path from the file's directory
permission_mode = "default"           # claude only: one of PERMISSION_MODES; default "default"
setting_sources = ["project"]         # claude only: of user, project, local; [] loads none; default Claude Code's own
approval_policy = "on-request"        # codex only: one of APPROVAL_POLICIES; default "on-request"
sandbox = "workspace-write"           # codex only: one of SANDBOXES; default "workspace-write"
bypass = false                        # claude and codex: offer the Bypass mode, which asks for nothing, to a paired client; default false
modes = ["plan", "accept_edits"]      # claude and codex: the modes a paired client may choose, widening the default (the entry's own mode and stricter ones); Bypass still needs `bypass`
cache_lifetime = "short"              # optional, with harness: how long the provider's prompt cache lives, one of CACHE_LIFETIMES ("default", "short", "long"); the entry's default for a session whose client chooses none; absent leaves the client's (the chat asks "long")
accounts = ["work", "personal"]       # optional, with harness: the [host.accounts] a client may run a session on, the first by default; absent runs the harness on the login it has where it runs

[host.agents.OUTSIDE]                 # an outside acp_harness provider
harness = "acp_echo_harness_echo_harness"  # its id; library or source is required for an id the host does not carry
source = "guatiao_source_py.dll"      # or library = "..."; exactly one; paths from the file's directory
import_search = "library"            # with source only, as for a provider
settings = { title = "Echo" }         # the provider's own configuration, checked against its schema when the host starts

[host.agents.OUTSIDE.source_settings] # with source only, as for a provider
dirs = ["harness"]

[host.profiles.NAME]                  # a profile (see Profiles below), read as the person's own layer
overlays = ["team"]                   # optional: AOS overlays the app installed in overlays/ beside this file
allow_mcp = { team = "sha256:…" }     # optional: an overlay's MCP server and the definition digest it may start at
allow_hooks = { team = "sha256:…" }   # optional: an overlay and the tree digest its hooks may run at; see Overlay hooks
context = { text = "..." }            # and every key of the profile schema

[skill_pins]                          # top level: a skill from elsewhere (name, without regard to ASCII case) and the tree digest it must have
review = "sha256:…"

[host.accounts.work]                  # a login for an entry's `accounts`; written by `canoa accounts add`, and never holds a credential
harness = "claude"                    # a provider, as an entry's harness
kind = "claude-login"                 # one of the kinds the provider's catalog lists
id = "0b7d3c1e-5a52-4f0e-9d0f-6c1f5d5b8e21"   # Canoa writes it when it makes the account; never reused, and the key of its stored credential
settings = { base_url = "https://llm.example" }   # optional: the kind's settings that are not secrets, checked against its schema

[host.providers.NAME]                 # a Guatiao agent_provider, from a library or a source
library = "echo_provider.dll"         # a library file, relative to the file's directory; or:
source = "guatiao_source_py.dll"      # a source file, the same way; exactly one of the two
provider = "echo_provider"            # required; the provider's id
max_sessions = 8                      # sessions one connection may hold
import_search = "library"            # with source only, Windows: "library" or "path"; default library when import_dirs names a directory, else path, which is logged as a warning

[host.providers.NAME.source_settings]  # with source only; what the source is added with
dirs = ["python"]                     # for guatiao_source_py: directories of manifests, each <name>.guatiao[.<major>[.<minor>]]
paths = ["/opt/guatiao/python"]       # and what goes on sys.path; relative paths are made absolute from the file's directory
isolation = "shipped"                 # optional: "shipped" (the source's default), "isolated" or "inherit", which is logged as configuration
home = "python/runtime"               # optional: the Python installation the interpreter runs from; made absolute too
import_dirs = ["python/runtime"]      # optional, Windows: where python3.dll is found; made absolute too

[host.providers.NAME.settings]        # the provider's configuration, for one that is built from one
prefix = "hi"                         # checked against the provider's schema when a connection loads it

[host.tools.NAME]                     # a command-line tool with no ACP, served as an ACP agent; see "A tool entry" below
program = "pi"                        # required; found as the OS finds a command
args = ["--mode", "rpc"]
runs = "session"                      # required: "session" or "message"
unasked = "what it does without asking"   # required, not blank; shown when a session opens
prompt = { type = "prompt", message = "{prompt}" }   # session: required, a table; message: refused
cancel = { type = "abort" }           # session only; optional
resume_args = ["--session", "{session}"]  # message only; optional
max_sessions = 8                      # sessions one connection may hold
[host.tools.NAME.kinds]               # optional: a kind template's result -> one of ACP's tool kinds
bash = "execute"
[[host.tools.NAME.events]]            # the rules, in order
when = { type = "message_update", "assistantMessageEvent.type" = "text_delta" }
send = "text"
text = "{assistantMessageEvent.delta}"

# Any of the three tables, each optional:
listen = "127.0.0.1:7801"             # default 127.0.0.1:0, any free port; another address is configuration, logged; or "pipe:NAME" (Windows) or "unix:PATH" (Unix) for a native client on this machine, see "A pipe or Unix socket"
working_roots = ["/home/me/src"]      # absolute, no . or ..; a session's cwd must be under one; default any
                                      # a Windows share (\\server\share\src) is a root and a cwd like any path; \\?\UNC\ is compared and forwarded plain; a device path is refused
worktrees = true                      # agents and tools: a git worktree per session inside working_roots; see "Worktrees"
peers = ["sha256:…"]                  # paired keys that may use it; default every paired key
max_peers = 4                         # connections at once

[host.web]                            # optional: the web app on a listener of its own; see "The web app" below
dir = "site"                          # the site, relative to the file's directory; without it the site built in (feature web)
listen = "127.0.0.1:7800"             # the default; beyond loopback only with cert and key
cert = "web.crt"                      # PEM: the certificate, then any intermediates; with key, HTTPS
key = "web.key"                       # PEM: its private key
hosts = ["canoa.example"]             # Host names a request may carry besides localhost and an IP address
connect = ["https://fonts.gstatic.com"]   # origins the page may connect to besides the entries; each logged as configuration
max_connections = 64                  # the limits, each at its default; 0 is refused
max_connections_per_address = 16
connection_seconds = 600              # a connection's whole life, however busy
max_request_bytes = 16384
max_requests = 1000                   # per connection
request_seconds = 10
idle_seconds = 30
send_seconds = 30
immutable_seconds = 31536000          # how long a hashed file may be kept; 0 allowed
```

Names are 1 to 64 letters, digits, `-` or `_`, unique across the three tables. Two entries may not name the same non-zero address, nor may an entry and `[host.web]`.

```rust
pub const DEFAULT_MAX_PEERS: usize = 4;
pub const DEFAULT_MAX_SESSIONS: usize = 8;
pub const MAX_NAME: usize = 64;
pub const MAX_TITLE: usize = 64;

#[non_exhaustive] pub struct Config { pub origins: Vec<String>, pub entries: Vec<Entry>, pub profiles: IndexMap<String, HostProfile>, pub web: Option<WebConfig>, pub limit_phrases: Option<Vec<String>>, pub protected_paths: Option<Vec<String>>, pub accounts: IndexMap<String, AccountConfig>, pub account_store: AccountStore, pub store: StoreConfig, pub skill_pins: BTreeMap<String, String> }   // Default, Clone, Eq
#[non_exhaustive] pub struct WebConfig { pub dir: Option<PathBuf>, pub listen: SocketAddr, pub tls: Option<WebTls>, pub hosts: Vec<String>, pub connect: Vec<String>, pub limits: web::Limits, pub keep_out: Vec<PathBuf> }   // Default, Clone, Eq
pub fn WebConfig::new() -> WebConfig            // the site built in, on web::DEFAULT_LISTEN, plain HTTP, nothing else allowed, default limits
// keep_out is never read from the file: `serve` fills it with canoa.cfg's directory and the identity home
#[non_exhaustive] pub struct WebTls { pub cert: PathBuf, pub key: PathBuf }   // Clone, Eq
pub fn WebTls::new(cert: impl Into<PathBuf>, key: impl Into<PathBuf>) -> WebTls
pub fn Config::load(path: &Path) -> Result<Config, ConfigError>
pub fn Config::from_str_in(text: &str, base: &Path) -> Result<Config, ConfigError>   // touches no file
pub fn Config::entry(&self, name: &str) -> Option<&Entry>

#[non_exhaustive] pub enum Listen { Address(SocketAddr), Pipe(PipeName) }   // Clone, Eq, Display (the text `listen` is written in), From<SocketAddr>
pub fn Listen::parse(text: &str) -> Result<Listen, String>   // "ADDRESS:PORT", "pipe:NAME" or "unix:PATH"; the error is a clause naming `listen` and quoting nothing
pub const fn Listen::address(&self) -> Option<SocketAddr>    // None for a pipe
#[non_exhaustive] pub struct HostProfile { pub profile: Profile, pub overlays: Vec<String>, pub allow_mcp: BTreeMap<String, String>, pub allow_hooks: BTreeMap<String, String>, pub directory: PathBuf, pub skill_pins: BTreeMap<String, String> }
#[non_exhaustive] pub struct Entry { pub name: String, pub kind: EntryKind, pub listen: Listen, pub working_roots: Vec<PathBuf>, pub worktrees: bool, pub peers: Option<Vec<PeerId>>, pub max_peers: usize, pub title: Option<String>, pub local: bool }   // local: served over Local only, no address; only the host's own sources are
#[non_exhaustive] pub enum EntryKind { Agent { program: OsString, args: Vec<OsString>, profiles: Vec<String>, usage: Option<UsageSource> }, Harness { harness: String, origin: Option<Origin>, executable: Option<OsString>, options: HarnessOptions, settings: Option<Settings>, profiles: Vec<String>, accounts: Vec<String> }, Provider { origin: Origin, provider: String, settings: Option<Settings>, directory: PathBuf, max_sessions: usize }, Tool { spec: ToolSpec } }
#[non_exhaustive] pub struct AccountConfig { pub harness: String, pub kind: String, pub id: String, pub settings: Option<Settings> }   // Clone, Eq: one [host.accounts.NAME]
#[non_exhaustive] pub struct StoreConfig { pub protected: Option<Vec<String>>, pub credential_variables: Option<Vec<String>> }   // Default, Clone, Eq: `[host.store]`; None is the recommended list
#[non_exhaustive] pub enum AccountStore { Os, File }   // Copy, Default (Os), Eq: `[host] account_store`
pub const CACHE_META: &str = "dev.boriken.canoa/cache_lifetime";   // the `_meta` key of `initialize`: {"lifetime": "default"|"short"|"long", "chosen"?: bool}; `chosen` (true when absent) says the person chose it for this session and it beats the entry's `cache_lifetime`; false is the client's default for its kind of session, which the entry's setting replaces; no key leaves the entry's, else the provider's default; any other value turns the client away (-32602)
pub const SITE_META: &str = "dev.boriken.canoa/site";   // the `_meta` key of a webpage's `session/new`, `session/load` and `session/resume`, in the request ({"site": "<host name>"}) and in the answer to `session/new` ({"site", "id", "folder"}); see "A webpage's session folder"
pub const ACCOUNT_META: &str = "dev.boriken.canoa/account";   // the `_meta` key of `initialize`, in the request (the account's name) and in the answer ({"name", "kind"})
#[non_exhaustive] pub enum UsageSource { Antigravity }   // Copy; `usage = "antigravity"` on an agent entry
pub fn UsageSource::named(name: &str) -> Option<UsageSource>
#[non_exhaustive] pub enum Origin { Library(PathBuf), Source { file: PathBuf, settings: Settings, import_search: ImportSearch } }   // paths absolute
#[non_exhaustive] pub enum ImportSearch { Library, Path }   // Copy
pub const fn ImportSearch::as_str(self) -> &'static str     // "library", "path"
pub fn ImportSearch::named(text: &str) -> Option<ImportSearch>
pub fn ImportSearch::default_for(settings: &Settings) -> ImportSearch   // Library when import_dirs names a directory, else Path
pub struct Settings(..)                         // Clone, Default, Eq; Debug shows keys, never values
pub fn Settings::new(map: serde_json::Map<String, Value>) -> Settings
pub fn Settings::as_map(&self) -> &serde_json::Map<String, Value>
```

- `entries` keeps the file's order, every agent before every provider, every provider before every tool. A file with no `[host]` gives no entries.
- An agent names `program` (with optional `args`) or `harness` (a provider id, not empty), one of the two; a `harness` other than `claude`, `codex` or `pi` with no `library` or `source`, `args` with `harness`, and `executable`, `permission_mode`, `approval_policy`, `sandbox`, `bypass`, `cache_lifetime`, `library`, `source`, `source_settings`, `import_search` or `settings` with `program` are refused, and so is another harness's key or a value not in its list. `executable`, `permission_mode`, `approval_policy`, `sandbox`, `bypass`, `setting_sources` and `modes` are the `claude`, `codex` and `pi` providers' settings (`bypass` is `allow_bypass`; `setting_sources` is Claude Code's alone, `permission_mode` too, `approval_policy` and `sandbox` Codex's; `pi` takes none of those four and has `settings` for the rest: `node` with `script`, `protected`, `isolated`) and are refused for any other id, which takes `settings`; one the entry's `settings` also holds is refused as set twice. A `modes` list is logged at `info` when the host starts. The `library` or `source`, `source_settings` and `import_search` keys read as a provider entry's. An `executable` holding `/` or `\` is made absolute against the file's directory, since the command line runs in a session's directory, which a client chooses. A harness entry is an `agent` in `check`'s and `serve`'s lines.

```rust
#[non_exhaustive] pub enum Harness { Claude, Codex }   // Copy, Eq; canoa_host::harness::Harness: what `canoa agent claude|codex` serves on standard streams; not an entry's harness, which is a provider id
pub const fn Harness::as_str(self) -> &'static str    // "claude", "codex"
pub fn Harness::named(name: &str) -> Option<Harness>
#[non_exhaustive] pub struct HarnessOptions { pub permission_mode, pub approval_policy, pub sandbox: Option<String>, pub bypass: bool, pub setting_sources: Option<Vec<String>>, pub modes: Option<Vec<Mode>>, pub cache_lifetime: Option<CacheLifetime> }   // Default, Eq
pub const BUILT_IN_HARNESSES: &[&str];  // claude, codex, pi: the acp_harness providers compiled into the host
pub const PERMISSION_MODES: &[&str];   // default, acceptEdits, plan, dontAsk, bypassPermissions, auto, manual
pub const SETTING_SOURCES: &[&str];    // user, project, local
pub const APPROVAL_POLICIES: &[&str];  // untrusted, on-failure, on-request, never
pub const SANDBOXES: &[&str];          // read-only, workspace-write, danger-full-access
```
- `[host.store]`: `protected` and `credential_variables`, each a string with a pattern a line or an array of such strings, in gitignore syntax (a leading `!` allows what an earlier line protected, the last matching line wins, `#` comments, `~/` the home, a drive or `/` the root, otherwise anywhere, a folder's pattern covers what is under it, names compare whatever their case). Absent, the recommended list applies (`store::protect::RECOMMENDED_PLACES`, `RECOMMENDED_VARIABLES`); a list written replaces it, an empty one protects nothing. `protected` is the places a program store's declared change may not be at, under or hold (`~/.ssh`, `~/.config/autostart`, ...) and the shared folders it may enter only by a place of its own (`~/.config` then `!~/.config/*`); `credential_variables` is the variable names an install is never given and a launch never sets (`*TOKEN`, `*_API_KEY`, `SSH_AUTH_SOCK`, ...). It is `[host.store]` and not `[permissions] protected` because the question differs: a state folder `~/.name` is a program's own place here, and the app's `.*` would protect it. Any other key is refused by name. `Options::from_config` applies both.
- `[host.accounts.NAME]`: a name as an entry's, with `harness`, `kind` and `id` (1 to 128 bytes, no control character, another account's refused) required and `settings` optional. Any other key is refused by name (`api_key is not a key of an account`), never quoting its value: **a credential never goes in canoa.cfg**. `[host] account_store` is `os` or `file`. An agent entry's `accounts` names accounts the file holds, none twice; with `program` it is refused. When `serve` starts, each account an entry offers is checked against the provider's catalog: its harness is the entry's, its kind is listed, its `settings` fit the kind's schema (a key the kind marks as a credential is refused as one, naming the key) and an endpoint setting is an `https` address or an `http` one to this machine.
- `[host.profiles.NAME]`: a name as an entry's; `overlays` valid AOS overlay names; `allow_mcp` and `allow_hooks` values `sha256:` and 64 lower-case hex digits (`allow_hooks` keys valid overlay names); the rest `Profile::from_table` as `LayerKind::User`, its relative paths from the file's directory. An agent's `profiles` must name profiles the file holds, each once. Errors say `profile NAME:` and a setting, never a value.
- `[skill_pins]` (top level, read whether or not there is a `[host]`): each key a skill name (a letter or digit, then letters, digits, `_`, `.`, `-`; no device name), each value `sha256:` and 64 hex digits; two keys equal without regard to ASCII case, or anything else, refuse the file with `[skill_pins]` and what is wrong. Kept by lower-case name in `Config::skill_pins` and in each `HostProfile::skill_pins`.
- An entry's `listen` is `ADDRESS:PORT`, `pipe:NAME` or `unix:PATH` (`Listen::parse`). `pipe:NAME` is a Windows named pipe `\\.\pipe\NAME`, and on Unix the socket `$XDG_RUNTIME_DIR/NAME.sock` (else `$TMPDIR/NAME.sock`); `unix:PATH` is an absolute socket path, and is refused on Windows. A name or path `coqui-transport-pipe` cannot use is refused naming `listen`, never quoting it. Two entries cannot share a pipe or an address with a port; a pipe never conflicts with `[host.web]`'s address.
- A tool entry's `listen`, `working_roots`, `worktrees`, `peers` and `max_peers` are read as in the other tables; the rest is a `ToolSpec`, refused as a whole when `ToolSpec::check` refuses it. Its errors name a setting and a rule's index, never a value.
- `library` and `source` are made absolute against `base` (the file's directory for `load`); `directory` is `base` made absolute, where the provider's process runs.
- In `source_settings`, every text in a `dirs`, `manifests`, `paths` or `import_dirs` list, and `home` when it is text, is made absolute against the directory, since a source may otherwise resolve it against a later working directory or its own file's; other keys, `isolation` included, pass as written, for the source to refuse.
- `source_settings` and `settings` are TOML tables held as JSON. A date, or a float that is not a number, is refused; nothing checks them further until a connection loads the provider, since checking means loading its code.
- A leading byte-order mark is ignored; anything else not UTF-8 is a parse error.
- `[host.web]`: `dir`, `cert` and `key` are made absolute against the directory; `cert` and `key` go together; `listen` beyond loopback without them is refused; `hosts` are names of letters, digits, `-` and `.`, kept in lower case; each `connect` source is `http`, `https`, `ws` or `wss`, `://`, a host of letters, digits, `-` and `.`, an optional port and an optional path of letters, digits, `/`, `-`, `.`, `_` and `~`, nothing else; every limit but `immutable_seconds` must be at least 1. Errors say `[host]: web:` and a setting, never a value. The table read is not checked for a site: `WebServer::bind` does that.

## Serving (`canoa_host::serve`)

```rust
pub const MAX_LINE: usize = 16 * 1024 * 1024;
pub const MAX_UNPROVEN: usize = 64;
pub const PROOF_LIMIT: Duration = 15 s;
pub const FIRST_MESSAGE: Duration = 60 s;
pub const ACP_CAPABILITY: &str = "acp/1";
pub const OUTPUT_LIMIT: usize = 1024 * 1024;   // what a provider's process may print per connection
pub const START_LIMIT: Duration = 30 s;        // for a provider's process to connect back

#[non_exhaustive] pub struct ServeOptions { pub provider_program: PathBuf, pub program_args: Vec<OsString>, pub key_limit: Option<u64>, pub rekey: bool, pub nothing_to_serve: bool, pub state_dir: Option<PathBuf> }
pub fn ServeOptions::new(provider_program: impl Into<PathBuf>) -> ServeOptions   // each method's defaults: key limit 2^38, rekey on
pub const fn ServeOptions::with_key_limit(self, bytes: u64) -> ServeOptions      // no floor; above the default the log lists it as a non-default setting
pub const fn ServeOptions::with_rekey(self, rekey: bool) -> ServeOptions
pub fn ServeOptions::with_program_args(self, args: Vec<OsString>) -> ServeOptions   // before the subcommand on provider_program's command line
pub const fn ServeOptions::with_nothing_to_serve(self, allowed: bool) -> ServeOptions   // a configuration with no entry binds, and serves nothing until stopped
pub const fn ServeOptions::with_local_sources(self, on: bool) -> ServeOptions   // also serve the host's own sources over Local; see "The host's own sources"
pub fn ServeOptions::with_state_dir(self, dir: impl Into<PathBuf>) -> ServeOptions   // where a private agy read gets its empty working directory, `<dir>/usage-read`; `canoa serve` gives the home; none, and no private agent is started

pub fn Serving::bind(config: &Config, identity: coqui_agentic_peer::auth::Identity, store: TrustStore, options: &ServeOptions) -> Result<Serving, ServeError>
pub fn Serving::method(&self) -> coqui_agentic_peer::auth::Method   // the identity's: every entry proves by it
pub fn Serving::peer(&self) -> PeerId          // the identity's id, which a client pins
pub fn Serving::listening(&self) -> &[Listening]
pub fn Serving::stopper(&self) -> Stopper      // Clone, Send: for another thread
pub fn Serving::local(&self) -> Local          // Clone, Send: the way into every entry from inside this process; outlives run(self)
pub fn Local::entries(&self) -> Vec<&str>      // the entries it reaches, in the configuration's order
pub fn Local::open(&self, entry: &str) -> Result<Link, LocalError>   // a connection inside this process: no proof, no trust store; the entry's agent starts on the first message
#[non_exhaustive] pub enum LocalError { NoEntry, Stopped, Thread }
pub fn Link::send(&self, message: &[u8]) -> Result<(), LinkError>    // waits up to 30 s while 256 are unread
pub fn Link::receive(&self, timeout: Option<Duration>) -> Result<Option<Vec<u8>>, LinkError>   // None: the other end closed and all was read
pub fn Link::close(&self)                      // ends both ends and the agent; dropping a Link does it
#[non_exhaustive] pub enum LinkError { TooLong, Timeout, Ended }
pub fn Serving::run(self)                      // takes connections until Stopper::stop or the process ends; returns with every listener closed and every agent ended
pub fn Stopper::stop(&self)                    // ends every listener, link and agent
pub fn Stopper::stop_agents(&self)             // ends every agent process running, and each later one before it serves; the listeners stay; returns once all have ended, whichever thread ends them
#[non_exhaustive] pub struct Listening { pub name: String, pub kind: &'static str, pub url: String, pub harness: Option<String>, pub title: Option<String>, pub local: bool, pub missing: Vec<String> }   // local: no address (url is empty); missing: programs a local entry needs that were not found; url: ws://ADDRESS, or pipe:NAME / unix:PATH for an entry on a pipe
pub fn Listening::on_address(&self) -> bool    // whether a browser can dial it; false for a pipe entry   // kind: "agent", "provider", "tool" or "control" (the control channel, which `/canoa.json` never lists); harness: a harness entry's provider id; title: an agent entry's title
```

- `stop_agents` ends each agent and each provider process with what it started (job object on Windows, process group on Unix), and turns every later proven connection away before its agent serves it. The listeners stay bound.
- `stop` does that and ends the rest, so an application can start a host again in its own process: each listener closes within 100 ms, when `run` returns, and a fixed address can be bound again; a proven link ends within about a second; a link still in its proof ends by `PROOF_LIMIT` at the latest, holding no address. An entry could one day be served from a stream in this process rather than a program: `run` keeps what serves a connection behind one private enum.

- `bind` binds every entry before any is served, so a failure leaves none listening. An entry that listens on a pipe is bound by `coqui-transport-pipe` with its defaults (see "A pipe or Unix socket"); a failure is `ServeError::Pipe`, and `Listening::url` is then `pipe:NAME` or `unix:PATH`, as `Listen` prints it, not a `ws://` URL. An entry on an address beyond loopback is bound as a plain WebSocket listener with `plain_beyond_loopback` set (the person chose the address) and logged at `info` as `non-default: host.agents.NAME.listen is ADDRESS, default loopback`: every message after the proof is sealed, so the address is the person's to choose.
- Each connection: the WebSocket handshake, then the proof of the identity's method, Noise (`coqui-peer-noise`) or WebCrypto (`coqui-peer-webcrypto`), within `PROOF_LIMIT`; a client of the other method fails it, and the log names both methods. Then the trust store decides (a stranger that proved its key is recorded as waiting), then the entry's `peers`, through a `canoa::Registry` holding one verified `Service` per entry and a grant of `ACP_CAPABILITY` per listed key. Every later message is sealed in the proof's session, which is checked against the store every second: a removed key loses its link.
- On an agent or tool entry, worktree requests (see "Worktrees") are answered by the host before anything else, up to 16 per connection, then the connection ends; once a first other message has come, one is answered `-32600`.
- The first message must be plain ACP: a link of streams (`coqui_agentic_peer::mux`) is refused. Then a fresh process serves that connection, relayed by `coqui_acp_bridge::relay`, and is stopped with its process tree when either side ends.
- An agent entry runs `program args`. Every message from the client to it is parsed and the host's own serialization forwarded: `params` other than an object is answered `-32602`, since a list would fill a request's fields in order; `_meta` is removed at every depth, except that an `initialize` is given coqui-acp's usage opt-in again, `{"notify": <the client's own wish, false unless it asked>, "account": <the entry's name>}` under `dev.boriken.coqui.acp/usage` in the client's capabilities (`clientCapabilities`, or `capabilities` from version 2), and every usage report the agent sends back (the `_coqui_acp/usage_update` notification's, and an answer to `_coqui_acp/usage`) leaves with `account` set to the entry's name whatever the agent named, so a client never reads an account the host did not assign; when the agent's `initialize` answer does not advertise the usage capability, the host stands in for it (`usage_splice`): it adds the capability with `sources: ["inferred"]`, reads each failed `session/prompt` answer (the error's message and data) and each `refusal` (what the agent said in that turn) for the entry's limit phrases, keeps one report per entry shared by its connections (bucket `inferred`, exhausted, `account` the entry's name, `revision` and `observed_at` the host's), answers `_coqui_acp/usage` itself without forwarding it and sends `_coqui_acp/usage_update` to a client that opted in with `notify` when the report changes; an inferred hit ends at its `resets_at`, else at `observed_at` plus its `retry_after`, else when a turn of the agent ends well, and a good turn does not clear one whose end has not come; with `limit_phrases = []` the host adds and answers nothing; an agent that advertises the capability is untouched; before the `initialize` answer shows the agent does not advertise it (and after an `initialize` that failed) the host only forwards, and while it speaks for the agent it drops the agent's own `_coqui_acp/usage_update` and any answer to a usage request it answered; agent text is kept only for a session with a prompt open; a request whose `params.mcpServers` holds anything but `http` or `sse` over `https`, or `acp`, is answered `-32602` and never reaches the agent; with `working_roots`, so is one whose `cwd` or `additionalDirectories` is outside them; what is not one JSON object is answered `-32700` or `-32600`. A refused notification is dropped.
- `usage = "antigravity"` on an agent entry (`program`, never `harness`) makes the host read its agy's quota as well (`usage_splice/agy.rs`). The `initialize` answer's capability then says `sources: ["reported", "inferred"]` and `needs_session: true`, and the entry's report is the buckets read (coqui-acp's `AgyQuota`: ids `agy:<first model id>`, permille, `reported`) beside any inferred hit, its `revision` the sum of the two counts. A reading looks at the processes that are the agent's: the one the host started and those below it (`usage_splice/members.rs`: on Windows PowerShell's `Get-CimInstance Win32_Process` parent table, elsewhere `/bin/ps -A -o pid=,ppid=,pgid=` taking children by parent and the process group the agent leads; a table that cannot be had leaves the agent's own process; on Windows each row carries the process's creation time and a child counts only if created at or after its parent, so an orphan whose dead parent's id was reused is not adopted). The agent's own process is tried alone first and the tree used from the first time that finds no port, which is the case below `canoa launch` for a profile. Models are `session/new`'s `models.availableModels[].{modelId, name}`. A reading runs on a worker thread of the connection, never the relay thread: after a `session/new` or `session/load` answer (at once, then after 2, 4, 8 and 16 seconds while the agent has no port), after each prompt answer, and for `_coqui_acp/usage` from a reading younger than 15 seconds, else after one; the answer and any `_coqui_acp/usage_update` are written by the connection's receive loop on its next 50 ms tick. With no session open, `_coqui_acp/usage` starts a private `agy acp` (the entry's program and args, the host's environment), initializes, opens an empty session in `<state_dir>/usage-read`, reads, and ends it; one at a time per entry, not within 15 seconds of the last start, 40 seconds at most. The private agent is started through the host's record of its agents, so stopping the host ends it and none starts while it stops; it is also ended when its process ends early. A request that comes while one reads waits for it. None starts for an entry with `profiles` (its environment is not applied to the private agent, so it would read the host's account, not the connection's) or when `ServeOptions::state_dir` is none; the answer is then what the entry holds. The entry's buckets are shared by its connections, except for an entry with `profiles`, where each connection keeps its own, so no connection is answered from another's reading. The account's name, email and picture never leave the reader.
- **An agent entry with `profiles`** runs `provider_program launch -- PROGRAM ARGS...` instead, whose first line of standard input the host writes: the profile's environment (see `profile::launch`). The client chooses a profile by name in its first message, `initialize`, under `params._meta["dev.boriken.canoa/profile"]` (`profile::PROFILE_META`); none takes the first. A first message that is not `initialize` (after any worktree requests, which the host answers itself, see "Worktrees") is answered `-32600` (`-32602` when it names a profile), a name the entry does not offer or a value that is not text `-32602`, each logged, and then no reference is read and no process starts; a choice in any later message is answered `-32602` and logged. A reference that cannot be resolved (`from_env` unset, `from_file` unreadable) answers `-32603` and starts nothing. Each overlay the profile names is read from `overlays/NAME` beside canoa.cfg (4096 files, 64 MiB at most), with no link or excluded name, and must have the digest `overlays/NAME.json` records; else it is left out and reported. A skill an overlay names from elsewhere is delivered like a bundled one only from what the app fetched, which the host never fetches itself: `overlays/.skills/LOWERCASE-NAME/` (the files its tree digest covers, 1024 files and 16 MiB at most) and `overlays/.skills/LOWERCASE-NAME.json` (`name`, `source`, `digest`, `overlays`, ...). It is used when the record's `name` is the skill's, its `overlays` lists this overlay, for a source entry its `source` is the entry's and the entry's `pin` (if any) and canoa.cfg's `[skill_pins]` entry for the name (if any) are both its `digest`, the record is a JSON object of at most 64 KiB, neither it, `.skills` nor the skill's directory is a link, the files have exactly that digest, and they pass `coqui_aos::read_fetched_skill` (a `SKILL.md` with YAML front matter naming it). Otherwise it is not delivered and the report says why: `not supported` with `from SOURCE, not fetched` or `a named skill; no catalog resolved it here` when nothing applies, `refused` when a record or the files do not check. The skills of all the profile's overlays are planned together (`coqui_aos::resolve_skills`, no catalogs): a skill from elsewhere never takes a name another overlay's skill holds, whatever its overlay's `priority`; one kept out is reported `refused` under `skills.NAME` with its overlay, a name more than one overlay offers under `skills.NAME.collision` with the overlay delivered, and a delivered one from plain `http`/`git://` or a private network address under `skills.NAME.insecure-transport` or `skills.NAME.private-source`. A delivered skill with no pin, from the overlay or `[skill_pins]`, is also reported under `skills.NAME.unpinned`, and one whose front matter has keys beyond `name`, `description`, `license`, `compatibility` and `metadata` under `skills.NAME.skill-keys`. A client may also narrow the overlays one connection starts with, by `initialize`'s `_meta["dev.boriken.canoa/overlays"]` (`profile::OVERLAYS_META`), a list of names (anything else is `-32602` and logged): only the profile's overlays among them are read, each other the profile lists is reported `off` under `overlay` ("not chosen for this session"), and a name the profile does not list adds nothing and is reported `not supported`. The person's own layers and the agent's are never narrowed. The agent's `initialize` answer decides the harness (from `agentInfo.name`, else an entry named `Claude`, `Codex`, `Gemini` or `Pi`) and what it takes (`systemPrompt`, `additionalDirectories`, MCP transports, by ACP v1 or v2 capability paths); then every `session/new`, `session/load` and `session/resume`, after the scope above, gets the context as `systemPrompt` (`append`, before a client's own blocks), the MCP servers in `mcpServers` (v1's stdio form untyped; a client's server of a name the profile holds, started or not, is `-32602`), skills written once per connection to a private temporary directory (removed after the agent ends) as the Claude plugin `canoa-profile` in `_meta.claudeCode.options.plugins` or a directory Codex reads (its `skills` folder under the Agent Skills directory inside) in `additionalDirectories`. A session request before `initialize` is answered `-32600`. A stdio server's `command` goes absolute: as written, from its layer's directory when it holds a separator, else the first match on the host's `PATH` (absolute entries only; `PATHEXT` on Windows), refused when none is found. An overlay's MCP server starts only when `allow_mcp` names it at its digest (`sha256` of `{"layer", "server"}` as the view writes them, and `"program"`, the stdio program found); off, it is logged with that digest. The answer to `initialize` and to each session request carries `_meta[PROFILE_META]` = `{"name", "harness", "summary", "items": [{"key", "layer", "state", "how", "digest"?}]}`, `state` being `applied`, `not supported`, `off` or `refused`: names, never values. No project profile is read.
- **A harness entry** runs an `acp_harness` provider, chosen by its id, in this process for each connection, over the same channel and scope as an agent entry. `bind` gives each entry a registry of its own: `claude`, `codex` and `pi` (coqui-acp-claude's, coqui-acp-codex's and coqui-acp-pi's providers, compiled into the host and registered with `local_providers!`) for an entry with no `library` or `source`, and only what the entry's file offers for one that has it (config refuses an origin on `claude`, `codex` or `pi`), the way a provider entry loads it but in this process, not a process of its own; it then builds each entry's instance from the entry's settings, which the provider checks against its schema (`ServeError::Harness` names the entry and the reason, no value). A connection starts with `StartRequest` (the entry's `working_roots`, which hold a session and a resumed Codex thread; with a profile also the profile's environment and, when the entry has roots, the directory the connection's skills are written under, `Connection::skills`; an entry with no roots gives none), the host's `LocalSpawner` (the provider starts its program as this host's child, ended with everything it started when the connection or the host ends), a sink that hands the provider's messages to the client and a keeper for no account (no account, `harness-login`), and the cache lifetime the connection asks: the client's choice for the session in `initialize`'s `_meta` (`CACHE_META`), else the entry's `cache_lifetime`, else the client's default for its kind of session, else the provider's own (`default`). Claude Code's provider sets `CLAUDE_CODE_PROMPT_CACHE_TTL` and `CLAUDE_CODE_SUBAGENT_PROMPT_CACHE_TTL` to `5m` for `short` and `1h` for `long`; Codex's says it ignores it (`canoa options` prints each provider's sentence as `cache_lifetime`, and an entry's own setting as `cache_lifetime_setting`). With a profile the host builds a second instance with `allow_client_plugins = true` where the provider has that setting (Claude Code's), for the one plugin it writes. `stop_agents` closes every connection at once, which makes each provider close its sessions and end each command line with what it started, waits for no client that has stopped reading, and returns once each provider has said `closed` or after 10 s each. A provider that ends the connection with a failure ends it, and the reason is logged at `info`. How the built instance departs from its schema's defaults (Guatiao's `schema::differences`) and the provider's `non_defaults` are logged at `info` as `non-default: host.agents.NAME: SETTING is VALUE, default D`, so `settings = { allow_bypass = true }` is logged as `bypass = true` is. The host sets `max_requests` to 64 for `claude` and `codex` unless `settings` say another. A harness's notices and exit reasons no longer reach the log (the kind has no route for them); a provider's own `closed` status and reason do, at `info`. The `mode` option offers Bypass, the mode that asks for nothing, only when the entry says `bypass = true` (off by default; harness only): a key paired to someone else reaches the entry, and without this switch it could take full control of this machine by choosing that mode. Off, Claude Code is not started allowing it and a client that sets it is refused; on, the host logs `host.agents.NAME.bypass is on` at `info` as configuration, not as a warning. The app's own `canoa agent claude|codex` sources take `--allow-bypass`, since the person using the app is the person who configured it.
- A profile reads an agent's `initialize` answer where its version puts what it says: `capabilities` and `info` in version 2, `agentCapabilities` and `agentInfo` in version 1. `coqui-acp-claude` is Claude Code and `coqui-acp-codex` Codex.
- A tool entry is served as an agent entry whose program is `provider_program` itself: `provider_program tool --name NAME --spec JSON`, the entry as `ToolSpec` serializes it. So the scope above applies to its clients, and the process, with every tool process it starts, ends with the connection.
- A provider entry is not filtered: its facade reads the client's `_meta` launch values (`context`, `model`, `effort`, an opaque `meta`) and hands them to the provider, and refuses an environment name. What a provider does with them is the provider's.
- A provider entry runs `provider_program provider (--library L | --source S) --provider ID --max-sessions N --directory D --settings-on-stdin [--import-search library|path] [--working-root R]... --connect A`, so a library or source is loaded only in that connection's process, which runs in `D`. The command line holds paths, names and the address only. For each connection the host listens on a loopback port of its own, `A`; writes the entry's `source_settings` and `settings` and a one-time token of 32 random bytes as the first line of the process's standard input (`HostLine`), then closes it; and takes the first connection to `A` that sends the token within `START_LIMIT`, up to 8 that do not. ACP runs on that socket alone. What the process writes to standard output or error goes to the host's standard error, `OUTPUT_LIMIT` bytes per connection in all, the rest read and dropped after one warning; none of it reaches the client. The process and what it starts are ended with the connection, by job object on Windows and process group elsewhere. Its facade refuses any `mcpServers` and checks `working_roots` itself.
- On Windows a source entry whose `import_search` is `path` is logged as `bind` binds it: at `info` as `non-default: host.providers.NAME.import_search is path, default library: …` when the file names it, at `warn` as `host.providers.NAME.import_search is path, because import_dirs names no directory: …` (saying to set `import_dirs` or `import_search = "library"`) when the host took it as the default. `Origin::Source::import_search_named` tells the two apart.
- A provider entry whose `source_settings` hold `isolation = "inherit"` is logged at `info` as `bind` binds it: `non-default: host.providers.NAME.source_settings.isolation is inherit, default shipped`. No other source setting is judged.
- `max_peers` connections per entry at once; past it a proven peer is turned away. Past `MAX_UNPROVEN` handshakes at once across entries, a new connection is dropped.

```rust
#[non_exhaustive] pub enum ServeError { NothingToServe, Listen { entry: String, failure: Failure }, Pipe { entry: String, error: pipe::Error }, Proof(String), Harness { entry: String, what: String }, Accounts { what: String } }
```

## A pipe or Unix socket (`canoa_host::pipe`)

```rust
pub use coqui_transport_pipe::{Error, Listener, PipeName, Stream};
#[derive(Debug)] pub struct PipeChannel;   // Send, Sync; coqui_transport::Channel
impl PipeChannel {
    pub fn new(stream: coqui_transport_pipe::Stream, max_message: usize) -> PipeChannel   // a connected stream
    pub fn connect(name: &PipeName, max_message: usize) -> Result<PipeChannel, Error>     // a client; Error::NotListening / AccessDenied / TimedOut
    pub fn peer(&self) -> io::Result<coqui_transport_pipe::OsIdentity>                    // who the system says is at the other end
}
```

- An entry whose `listen` is `pipe:NAME` or `unix:PATH` is served as one on an address is: a fresh agent per proven connection, the same proof of the host's method, the same trust store (a key that is not paired is turned away and waits for `canoa accept`), `peers`, `max_peers`, worktree requests and the 64 unproven connections at once, shared with the sockets. Nothing differs after the connection opens.
- What differs is the transport: the operating system's access control stands before the proof, and `coqui-transport-pipe`'s defaults are kept (Windows: a security descriptor for the current user alone, plus the integrity label of an elevated host, the first instance of the name taken so nobody else holds it, remote clients refused; Unix: a `0600` socket in a directory only its owner (or root) can write, a stale socket replaced and a live one not). Neither replaces the key proof, and the proof does not trust the access control: the same user's other processes reach the pipe, and so would anyone the descriptor let in. The peer's process and user are logged at `debug`.
- A message is a little-endian `u32` length and then its body (coqui-transport-web's length-prefixed stream format), with no WebSocket handshake and no origin check. `PipeChannel` reads on demand with no read-ahead, so a bound `set_max_message` changes applies to the next message the peer sends. Zero length, over the bound, or a peer that leaves mid-message ends the channel (`Failure::Protocol` or `PathLost`); a clean end between messages is `Ok(None)`. A write that times out ends the channel (a cancelled pipe write on Windows leaves the stream's position unknown).
- Every peer on pipes shares one bucket of the trust store's waiting queue, since there is no address to tell them apart by.
- The served web page never offers or allows a pipe entry: `/canoa.json` and the page's Content-Security-Policy leave it out (`Listening::on_address`), and no browser can reach it.
- A pipe name on Windows is visible to every user: the first instance is taken at bind, so a second host on one name is `ServeError::Pipe` (`NameTaken`), but a client that connects before any host bound reaches whoever bound first, and only the key proof tells it from the host.
## The web app (`canoa_host::web`)

```rust
pub const DEFAULT_LISTEN: SocketAddr = 127.0.0.1:7800;
pub const HOST_INDEX: &str = "/canoa.json";
pub const BUILT_IN: bool;                       // cfg!(feature = "web")

#[non_exhaustive] pub struct Limits { pub max_connections: usize, pub max_connections_per_address: usize, pub connection_time: Duration, pub max_request_bytes: usize, pub max_requests: usize, pub request_time: Duration, pub idle_time: Duration, pub send_time: Duration, pub immutable_age: Duration }   // Copy, Eq, Default
// Default: 64, 16, 600 s, 16 KiB, 1000, 10 s, 30 s, 30 s, 365 days; each with a const builder:
pub const fn Limits::with_max_connections(self, count: usize) -> Limits   // and with_max_connections_per_address,
    // with_connection_time, with_max_request_bytes, with_max_requests, with_request_time, with_idle_time, with_send_time,
    // with_immutable_age. A time too long for the clock to add counts as a year.

pub fn WebServer::bind(web: &WebConfig, serving: &Serving) -> Result<WebServer, WebError>
pub const fn WebServer::local_addr(&self) -> SocketAddr
pub fn WebServer::url(&self) -> String          // http(s)://ADDRESS, or http(s)://hosts[0]:PORT when bound to every interface
pub fn WebServer::run(self)                     // takes connections until WebStopper::stop or the process ends; returns with the address free
pub fn WebServer::stopper(&self) -> WebStopper  // Clone, for another thread
pub fn WebStopper::stop(&self)                  // a connection already open ends after the answer it is sending, or when it next goes quiet

#[non_exhaustive] pub enum WebError { NoSite, Site { dir: PathBuf, why: &'static str }, PlainBeyondLoopback(SocketAddr), Limit(&'static str), Connect, Tls { file: PathBuf, why: String }, Listen { address: SocketAddr, source: io::Error } }
```

- `bind` refuses plain HTTP beyond loopback (`PlainBeyondLoopback`), a zero limit but `immutable_age` (`Limit`, naming the canoa.cfg setting), a `connect` source of another form than the file allows (`Connect`), a directory that cannot be resolved, holds no `index.html`, or is or holds one of `keep_out` once resolved (`Site`), and no directory in a build without `web` (`NoSite`); reads `cert` (every PEM certificate, in order) and `key` (one PEM private key) into a rustls server configuration with its safe default versions (TLS 1.2 and 1.3), the `ring` provider, ALPN `http/1.1` and no client certificate (`Tls`, naming the file, never its contents); and logs at `info` each limit above its default as `non-default: host.web.SETTING is V, default D`, a `listen` beyond loopback, and each `connect` source as `non-default: host.web.connect lets the page connect to SOURCE…`; on Unix a `key` file others may read is logged at `warn`, since nothing in the file asked for it. Plain HTTP beyond loopback stays refused: the page is not sealed like an entry's link, so whoever is on the path could rewrite the code that holds the person's key. Then it binds (`Listen`). The site's directory is resolved once, at `bind`.
- HTTP/1.1 and 1.0, `GET` and `HEAD` only (else 405 with `Allow: GET, HEAD`). Refused with 400: a request with a body (`Transfer-Encoding`, more than one `Content-Length`, or one other than 0), no `Host` or two (HTTP/1.0 included), a target that does not start with `/` (absolute-form included), anything `httparse` refuses. A `Host` that is not `localhost`, an IP address or one of `hosts`, with any port, is refused with 421, which stops a page of another site reaching this one by DNS rebinding. More than 64 headers or `max_request_bytes` is 431; a head not complete within `request_time` is 408. Every refusal closes the connection.
- **Paths.** The target's path (the query left out) is split at `/` and each segment percent-decoded; refused with 404 is any segment that is empty (but a last one), `.` or `..`, starts or ends with a dot, ends with a space, holds a control character, `/`, `\`, `:`, `*`, `?`, `"`, `<`, `>` or `|` once decoded, is not UTF-8, or is a Windows device name with any extension (`CON`, `PRN`, `AUX`, `NUL`, `CONIN$`, `CONOUT$`, `CLOCK$`, `COM` or `LPT` and a digit or `¹`, `²`, `³`). The file is the resolved path (every link followed) under the resolved root, each segment of which, once resolved, must pass the same rules: one that resolves outside the root, or to a name a request could not give (an NTFS short name such as `GIT~1` resolving to `.git`, a link to a dot-name), is refused with 404 and logged, whatever it is, and a directory is never listed. From the site built in, the path is looked up as written.
- **The app's routes.** A path naming nothing, or a directory, whose last segment has no `.` is answered with the root's `index.html` (200, `no-cache`); any other missing file is 404.
- **`/canoa.json`** (`HOST_INDEX`, the query left out, before any file of that name): `{"peer", "method", "entries": [{"name", "kind", "url", "harness"?, "title"?}]}`, the host's id and method and each entry as `serve` prints it, `Cache-Control: no-store`. It names the host for a page to fill in; it proves nothing, and connecting still runs the proof both ways and needs the page's key accepted.
- **Headers on every answer**: `Content-Security-Policy: default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; media-src 'self' blob:; connect-src 'self' URL...; worker-src 'self' blob:; manifest-src 'self'; object-src 'none'; base-uri 'self'; form-action 'none'; frame-ancestors 'none'`, the `connect-src` naming each entry's `url`, then each `connect` source; over HTTPS, to a request naming one of `hosts`, `Strict-Transport-Security: max-age=31536000` (never to `localhost` or an address, which other servers share); `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, `X-Frame-Options: DENY`, `Cross-Origin-Opener-Policy: same-origin`, `Cross-Origin-Resource-Policy: same-origin`, `Date`, `Connection`. A file carries `Content-Type` by its extension (`application/octet-stream` when unknown), an `ETag` (weak, of length and modification time, from a directory; of the contents, built in), and `Cache-Control: public, max-age=IMMUTABLE_AGE, immutable` when its name holds a hash (a part of 8 or more hex digits after the first, split at `.`, `-`, `_`: `main.3f9a1b2c.js`), else `no-cache`. `If-None-Match` naming its tag is answered 304. No range, no compression.
- **Connections**: one thread each, `max_connections` at once and `max_connections_per_address` from one address (past either a new one is closed unanswered), each ended after `connection_time` whatever it is doing; kept alive in HTTP/1.1 unless `Connection: close`, for `max_requests` requests; the first head must arrive within `request_time` of the connection (TLS handshake included), each later one within `idle_time` to start and `request_time` from its first byte; a write that makes no progress for `send_time` ends the connection.

## One provider on standard streams (`canoa_host::provider`)

```rust
#[non_exhaustive] pub struct ProviderOptions { pub origin: Origin, pub provider: String, pub settings: Option<Settings>, pub working_roots: Vec<PathBuf>, pub max_sessions: usize }
pub fn ProviderOptions::new(origin: Origin, provider: impl Into<String>) -> ProviderOptions   // no settings, any cwd, the facade's default max_sessions
pub fn serve_stdio(options: &ProviderOptions) -> Result<(), ProviderError>
pub fn serve_connected(options: &ProviderOptions, address: SocketAddr, token: &str) -> Result<(), ProviderError>   // loopback only

#[non_exhaustive] pub struct HostLine { pub source_settings: Option<Settings>, pub settings: Option<Settings>, pub token: Option<String> }   // Default, Eq; Debug shows no value or token
pub fn HostLine::to_line(&self) -> Vec<u8>                                           // one line of JSON, no line break
pub fn HostLine::read(input: &mut impl BufRead, max: usize) -> Result<HostLine, ProviderError>   // one line of at most max bytes
```

`serve_stdio` loads the code, takes the provider, and runs `coqui_acp_provider::AcpFacade` on standard input and output until the client closes them. There, loaded code shares both with the transport: a Python `print()`, or a subprocess that inherits them, writes into the client's ACP stream or takes its messages. `serve_connected` connects to `address`, sends `token` and a line break, then loads the code and runs the facade on that socket, so the process's own standard streams are the provider's alone; it is what `serve` runs.

- **A library** is mapped with Guatiao's default import search. On Windows: the library's own directory, the executable's, the system's; not the working directory or `PATH`. Elsewhere the platform's: `LD_LIBRARY_PATH`, the file's run path (its own directory only through `$ORIGIN`), the system's.
- **A source** is added with `settings` as a Guatiao map, then the registry is refreshed once; a library the source refuses is logged and passed over. On Windows the import search is the entry's `import_search`: an import the source names itself that the default search leaves missing (`python3.dll`, for Python) is mapped from the first of `import_dirs` holding it, and with `ImportSearch::Path` else the Windows directory, else **the working directory (canoa.cfg's, under `serve`) or a directory on `PATH`**, in that order; the source is then opened with the default search, so its other imports come from where they did. An import of an import is not looked for. Elsewhere the search is the platform's (`LD_LIBRARY_PATH`, the file's run path, the system's).
- **The provider**: with no `settings`, its own instance; a provider built only from a configuration is `NeedsConfiguration`. With `settings`, an instance built from them: Guatiao checks them against the schema the provider declares, then the provider may refuse them; a provider that is its own one instance is `TakesNoSettings`.

**Loading runs code** with this process's rights, before any check, and trusts whoever can write any file it reads or the directories holding them. For a library, that is the file. For `guatiao_source_py` it is also: the Python whose shared library the loader finds (on Windows the first `python3.dll` in `import_dirs` or the search above, then the `python3X.dll` it names); `GUATIAO_LIBRARY` in this process's environment, which picks the native library Guatiao's Python binding loads; every directory in `paths`; every manifest in `dirs` and `manifests` and every directory its own `paths` names; every module those import; and what the source's `isolation` lets the interpreter read:

| `isolation` | The interpreter also reads |
| --- | --- |
| `shipped` (the source's default) | the standard library and `site-packages` of `home` or the Python found, with their `.pth` files; a `canoa._pth` or `pyvenv.cfg` beside the `canoa` executable (`CanoaUI._pth` beside the app's, when it hosts), and on Windows that executable's directory, which CPython puts on `sys.path`, so whoever can write that directory chooses where modules come from. Not the `PYTHON*` variables or the user's own site-packages. |
| `isolated` | the standard library of `home` or the Python found, and nothing beside the executable, in `site-packages` or in the environment |
| `inherit` | all of `shipped`'s, the user's own site-packages, and the `PYTHON*` variables this process inherits from the host (`PYTHONPATH`, `PYTHONHOME`). `serve` logs it at `info` as configuration. |

`HostLine::read` reads up to the line break and leaves the rest to the transport. The line is `{"canoa_host": 1, "source_settings": {...}, "settings": {...}, "token": "..."}`; anything else, a client's ACP message included, is `ProviderError::HostLine`.

## A tool entry (`canoa_host::tool`)

A command-line tool with no ACP, served as an ACP agent (version 1) by the rules of its entry. No tool has code here.

```rust
pub const NOTICES_META_KEY: &str = "dev.boriken.canoa/notices";
pub const CANCEL_GRACE: Duration = 10 s;
pub const MAX_TEXT: usize = 1024 * 1024;      // text per update; longer is split, a tool_done output is cut
pub const MAX_LABEL: usize = 4096;            // a title, an id, a failure, after control and format characters go
pub const MAX_ASKS: usize = 16;               // permission requests one process may have waiting
pub const MAX_SPEC: usize = 16 * 1024;        // an entry as JSON, since it goes on a command line

#[non_exhaustive] pub struct ToolSpec { pub program: String, pub args: Vec<String>, pub runs: Runs, pub unasked: String, pub prompt: Option<Value>, pub cancel: Option<Value>, pub resume_args: Vec<String>, pub kinds: BTreeMap<String, String>, pub max_sessions: usize, pub events: Vec<Rule> }   // Clone, Eq, Serialize, Deserialize (deny_unknown_fields)
pub fn ToolSpec::check(&self) -> Result<(), String>          // what serde cannot check; names a setting, never a value. Also refuses an entry over MAX_SPEC or holding a NUL, and in `message` mode an argument that is `{prompt}` alone not right after `--`
pub fn ToolSpec::notices(&self, name: &str) -> Vec<String>   // what session/new lists under NOTICES_META_KEY
#[non_exhaustive] pub enum Runs { Session, Message }
#[non_exhaustive] pub struct Rule { pub when: Map<String, Value>, pub send: Action, pub text, id, title, kind, input, output, error: Option<String>, pub status: Option<ToolStatus>, pub stop: Option<Stop>, pub allow, reject, line: Option<Value> }
#[non_exhaustive] pub enum Action { Text, Thought, Tool, ToolDone, Failure, End, Session, Ask, Reply, Ignore }   // TOML: snake_case
#[non_exhaustive] pub enum ToolStatus { Completed, Failed }
#[non_exhaustive] pub enum Stop { EndTurn, Cancelled, MaxTokens, Refusal }
pub fn serve(spec: ToolSpec, name: &str, input: impl Read, output: impl Write + Send + 'static) -> Result<(), ToolError>
pub fn serve_stdio(spec: ToolSpec, name: &str) -> Result<(), ToolError>
#[non_exhaustive] pub enum ToolError { Spec(String), Transport(io::Error) }
```

- **Rules.** Each JSON line the tool writes is taken by the first rule whose `when` paths all hold their values (JSON equality; an empty `when` takes every line). A path is dotted steps of letters, digits, `_` and `-`; a number indexes a list. A line that is not JSON, or that no rule takes, is passed over, and so is every line of a `message` process that a later prompt replaced.
- **Fields per `send`**, each other field refused: `text`, `thought`, `failure`: `text`. `tool`: `id`, `title`; optional `kind`, `input` (a path, its value as `rawInput`, left out over `MAX_TEXT`). `tool_done`: `id`; optional `status` (`completed`), `output`. `end`: optional `stop` (`end_turn`) or `error`, not both. `session`: `id`. `ask`: `title`, `allow`, `reject`. `reply`: `line`. `ignore`: none.
- **Templates.** In text fields, `{path}` is the line's value there: a string as it is, null or missing as nothing, anything else as compact JSON; a `{` not opening a path is kept. In `prompt`, `cancel`, `allow`, `reject` and `line`, every string value is a template, a string that is one placeholder alone takes the value with its type (null when missing), and keys are kept as written. The prompt's context is `{"prompt": text, "session": the tool's session or ""}`; `args` and `resume_args` (`message` only) take the same context; `allow`, `reject` and `line` take the line that matched. Every line written, to the tool or the client, is one line: U+2028 and U+2029 are escaped as ` ` and ` `.
- **`kind`**: the rendered text when it is one of ACP's tool kinds (`read`, `edit`, `delete`, `move`, `search`, `execute`, `think`, `fetch`, `switch_mode`, `other`), else its value in `kinds`, else `other`. `check` refuses a `kinds` value that is not one.
- **ACP.** `initialize` answers version 1, no `loadSession`, no image, audio or embedded context. `session/new` needs an absolute `cwd` on this machine (on Windows a drive path; a share or device path is refused before anything looks at it) that is a directory, and `mcpServers` empty or absent (`-32602`), at most `max_sessions` per connection (`-32600`); it answers `{"sessionId", "_meta": {NOTICES_META_KEY: notices}}`. In `session` mode it starts the process there, `-32603` naming the entry when it cannot. `session/prompt` takes text and `resource_link` blocks (a link as its `uri`), joined by line breaks, anything else `-32602`; one turn at a time per session (`-32600`); it is answered `{"stopReason"}` when the turn ends, or `-32603` with the failure's text as `message`. `session/cancel` as below. Everything else is `-32601`; a notification other than `session/cancel` is dropped.
- **A turn ends** at an `end` rule; a `failure` before an `end_turn` makes it fail with that text. A `session` process that exits ends the turn with `-32603` and the session with it: every later prompt is `-32603`. A `message` process that exits ends its turn: `end_turn` (or its failure) on a success status, `-32603` with the status otherwise. A process that closes its output and has not exited within 2 s counts as failed.
- **Cancel.** A cancelled turn is answered `cancelled` however it ends. `session` with `cancel`: the line is written, and a turn still running `CANCEL_GRACE` later has its process ended and the session with it. Otherwise the process is ended at once; in `session` mode the session with it.
- **`ask`** sends `session/request_permission` (v1: `toolCall` `{"toolCallId": "canoa-ask-N", "title", "status": "pending", "rawInput": {"text": the whole question, cut to MAX_TEXT}}`, options `allow`/`allow_once` and `reject`/`reject_once`) and writes `allow` only for an answer selecting `allow` while the turn that asked runs uncancelled; any other answer and an error write `reject`. A request that comes with no turn running, cancelled, or `MAX_ASKS` waiting is refused at once, without asking. When a turn is cancelled or ends, every question of its session still waiting gets `reject` then and is forgotten, so a later answer reaches nothing; so is one whose process was replaced or ended.
- **Processes** run `program` (with `args`; in `message` mode `resume_args` first once a `session` rule has recorded one) in the session's `cwd`, with this process's environment. A line to one goes through a queue of 64; a full queue ends it. Its standard error goes to this process's, `OUTPUT_LIMIT` bytes per process. A line of output over `serve::MAX_LINE` ends it. On Windows each process is in a job object of its own, ended with it, and runs with `NoDefaultCurrentDirectoryInExePath` set, so a `.cmd` tool does not take a program it names bare from the client's `cwd`; on Unix it stays in this process's group, and ending it ends only it. When `input` ends, each process's input is closed, and it is ended 1 s later; under `serve` that does not happen, since the host ends the whole connection's process tree at once.

## Profiles (`canoa_host::profile`)

The schema of a profile, what an agent is launched with, and its merge in layers. The app reads the same schema; `tests/vectors/profile.json` holds both to the same views and refusals. `serve` applies a `[host.profiles]` profile to an agent entry that offers it (see Serving).

```rust
pub const PROFILE_META: &str = "dev.boriken.canoa/profile";
/// `initialize`'s `_meta` key naming, as a list of names, the overlays this connection starts with: only a narrowing of what the profile lists.
pub const OVERLAYS_META: &str = "dev.boriken.canoa/overlays";
pub fn launch(program: &OsStr, args: &[OsString], max: usize) -> io::Error
```

`launch` reads one line of at most `max` bytes, `{"canoa_host_launch": 1, "env": [[NAME, VALUE], ...]}`, runs `program` with those variables added, passes the rest of standard input to it, and exits this process with its status (1 for a signal); it returns only an error: `InvalidData` for a line of another form, or why the program did not start. It is `canoa launch`.

```toml
model = "m"                      # effort, mode likewise: text, not empty, no control character
context = { text = "..." }       # or { file = "CONTEXT.md" }: exactly one
unset = ["model"]                # model, effort, mode or context of a lower layer; not beside that key
share_env = ["GITHUB_TOKEN"]     # variables overlays and projects may read; refused in those layers themselves
[env.NAME]                       # NAME: a letter or _, then letters, digits or _, up to 128
value = "v"                      # exactly one of value, from_env (a variable's name), from_file (a path), remove = true
[skills.NAME]                    # NAME, and mcp's and headers': 1 to 64 letters, digits, - or _
path = "skills/review"           # or remove = true
[mcp.NAME]
type = "stdio"                   # stdio: command, args, [mcp.NAME.env.X]; http or sse: url (http:// or https://), [mcp.NAME.headers.X]
command = "npx"                  # or remove = true alone
```

```rust
pub const MAX_NAME: usize = 64;
pub const MAX_ENV_NAME: usize = 128;
pub const MAX_SECRET_FILE: u64 = 1024 * 1024;
pub const RESERVED_ENV: &[&str];            // PATH, HOME, NODE_OPTIONS, CLAUDE_CONFIG_DIR, ...: 42 names
pub const RESERVED_ENV_PREFIXES: &[&str];   // LD_, DYLD_, PYTHON, GIT_, NPM_CONFIG_, CANOA_, COQUI_
pub const RESERVED_ENV_SUFFIXES: &[&str];   // _BASE_URL, _PROXY
pub fn is_reserved_env(name: &str) -> bool  // any of those, without case, and every name coqui_aos::is_sensitive_variable names
pub fn full_commit_id(reference: &str) -> bool   // 40 or 64 lower-case hex digits

#[non_exhaustive] pub enum LayerKind { BuiltIn, Overlay, User, Project, Agent, Session }   // Copy, Ord: lowest first
pub const fn LayerKind::as_str(self) -> &'static str      // "built-in", "overlay", "user", "project", "agent", "session"
pub const fn LayerKind::is_confined(self) -> bool         // Overlay and Project
pub struct LayerId { pub kind: LayerKind, pub name: String }   // Display: "user", "overlay:team", "agent:Claude"
pub fn LayerId::new(kind: LayerKind, name: impl Into<String>) -> LayerId

#[non_exhaustive] pub struct Profile { pub model, effort, mode: Option<String>, pub context: Option<Context>, pub unset: Vec<Scalar>, pub share_env: Vec<String>, pub env: IndexMap<String, Change<EnvValue>>, pub skills: IndexMap<String, Change<Skill>>, pub mcp: IndexMap<String, Change<McpServer>>, pub contexts: Vec<Context> }   // Default; contexts only from an AOS overlay
pub fn Profile::from_str(text: &str, kind: LayerKind) -> Result<Profile, ProfileError>    // touches no file, reads no variable
pub fn Profile::from_table(table: toml::Table, kind: LayerKind) -> Result<Profile, ProfileError>
#[non_exhaustive] pub enum EnvValue { Value(String), FromEnv(String), FromFile(String), Text(String), Path(String), Reference { reference: String, optional: bool }, Paths(Vec<(String, bool)>) }   // Debug never shows a Value's or Text's text; the last four only from an AOS overlay
#[non_exhaustive] pub enum Context { Text(String), File(String) }
#[non_exhaustive] pub struct Skill { pub path: String }   pub fn Skill::new(path) -> Skill
#[non_exhaustive] pub enum McpServer { Stdio { command, args, env }, Http { url, headers }, Sse { url, headers } }
#[non_exhaustive] pub enum Change<T> { Set(T), Remove }
#[non_exhaustive] pub enum Scalar { Model, Effort, Mode, Context }

#[non_exhaustive] pub struct Layer { pub id: LayerId, pub dir: Option<PathBuf>, pub profile: Profile, pub aos: Option<Box<AosLayer>> }
pub fn Layer::new(id: LayerId, dir: Option<PathBuf>, profile: Profile) -> Layer
pub fn merge(layers: &[Layer]) -> Effective
#[non_exhaustive] pub struct Effective { pub model, effort, mode: Option<Sourced<String>>, pub context: Option<Sourced<Context>>, pub env, skills, mcp: IndexMap<String, Sourced<_>>, pub removed: Vec<Removed>, pub share_env: IndexMap<String, LayerId>, pub refused: Vec<Refused>, pub contexts: Vec<Sourced<Context>>, pub overlays: Vec<OverlayReport>, pub aos: Vec<AosLayer> }
#[non_exhaustive] pub struct Refused { pub key: String, pub layer: LayerId, pub reads: String }
#[non_exhaustive] pub struct Sourced<T> { pub value: T, pub layer: LayerId, pub dir: Option<PathBuf> }
#[non_exhaustive] pub struct Removed { pub key: String, pub layer: LayerId, pub was: LayerId }
pub fn Effective::view(&self) -> serde_json::Value          // references as written, never resolved
pub fn Effective::environment(&self, lookup: impl Fn(&str) -> Option<String>) -> Result<Vec<(String, Secret)>, ProfileError>
pub fn resolve(key: &str, value: &EnvValue, dir: Option<&Path>, confined: bool, lookup: impl Fn(&str) -> Option<String>) -> Result<Secret, ProfileError>
pub struct Secret(..)   pub fn Secret::expose(&self) -> &str   // Debug "Secret(..)", Display "(hidden)"

#[non_exhaustive] pub enum OverlaySource { Git { repo: String, reference: String, path: String }, Dir(PathBuf) }
pub fn OverlaySource::parse_git(source: &str) -> Result<OverlaySource, ProfileError>   // [git+]<repository>@<ref>[#<path>], an AOS source string

#[non_exhaustive] pub struct OverlayInput<'a> { pub identity, root, os: &'a str, pub harness: Option<&'a str>, pub pinned: bool, pub tree: &'a [aos::TreeEntry] }
pub fn OverlayInput::new(identity, root, os, tree) -> OverlayInput   // no harness, not pinned
pub fn overlay_layer(input: &OverlayInput<'_>) -> Result<Layer, OverlayRefusal>   // reads nothing but input.tree
#[non_exhaustive] pub struct AosLayer { pub identity, root, os, digest: String, pub pinned: bool, pub manifest: Map<String, Value>, pub files: BTreeSet<String>, pub texts: BTreeMap<String, Vec<u8>>, pub references: Vec<(String, String, bool)>, pub not_applied: Vec<String>, pub elsewhere: Vec<coqui_aos::SkillEntry> }
#[non_exhaustive] pub struct OverlayReport { pub layer: LayerId, pub digest: String, pub not_applied, ungranted, unresolved: Vec<String>, pub non_defaults: Vec<coqui_aos::NonDefaultSetting> }   pub fn OverlayReport::view(&self) -> Value
#[non_exhaustive] pub struct OverlayRefusal { pub code: &'static str, pub subject: String }   // code: name, digest, manifest, invalid, env-file, reserved

#[non_exhaustive] pub enum ProfileError { Parse { line, column }, Invalid { key: String, rule: Rule }, Resolve { key: String, what: &'static str } }
#[non_exhaustive] pub enum Rule { Name, Path, Form, Value, Reserved }
pub const fn ProfileError::code(&self) -> &'static str     // "parse", "name", "path", "form", "value", "reserved", "resolve"
```

- **Merge**: lowest first. A scalar a later layer sets replaces the lower one; `unset` takes it away first. A named entry a later layer sets replaces the lower one whole and keeps its place; `Change::Remove` takes it out, and set again later it comes last. `removed` lists only what was there to remove, in order.
- **Layers from elsewhere** (`is_confined`: overlay, project) read only shared variables: `share_env` of every other layer, wherever it sits, makes `Effective::share_env` (each name with the first layer naming it). An env entry of a confined layer whose `FromEnv` names a variable not shared, or an MCP server of one with such a value in its `env` or `headers`, is not merged: it goes to `refused` with the first such variable, and the lower layer's entry stays. A confined layer that sets a variable `is_reserved_env` names, in `env` or a stdio server's `env`, is refused at parse time (`Rule::Reserved`, key `env.NAME` or `mcp.NAME.env.NAME`); removing one is allowed. `share_env` in a confined layer is `Rule::Form`.
- **The view** has `model`, `effort`, `mode`, `context` when set, and always `env`, `skills`, `mcp`, `removed`, `share_env` (`{"name", "layer"}`) and `refused` (`{"key", "layer", "reads"}`).
- **Paths** are judged on the text, the same on every platform. In a confined layer (overlay, project) a path is relative and stays below its directory: no leading `/`, `\` or `~`, no `:`, no `..` segment. Elsewhere any non-empty path without NUL.
- **Errors** never quote a value: `Invalid.key` names the setting (`env.A`, `mcp.a.headers.T`), or for a refused name the table holding it (`env`, `mcp.a.env`). The first refusal in a document is, in order: anything serde refuses (an unknown key, a wrong type, a key twice) as `Parse`; then `model`, `effort`, `mode`, `context`, `unset`, then `env`, `skills` and `mcp` entries in the document's order.
- **`resolve`** reads `FromEnv` through `lookup` and `FromFile` from `dir` (an absolute path only outside a confined layer), less one final `\n` or `\r\n`. The path is canonicalized and that path is what is read; for a confined layer it must be under `dir` canonicalized, so a link out of the project is refused. It must be a regular file of at most `MAX_SECRET_FILE` bytes, UTF-8. Call it only when a session starts.
- **`parse_git`** reads an AOS source string (`coqui_aos::Source::parse`): the path follows the first `#`, without leading or trailing `/`, and must be an AOS relative path; the ref follows the last `@` when it holds no `/`, `:` or `\`. Refused besides what AOS refuses: a source that is not Git, or names no ref; a repository holding a space or control character, or a scheme other than `https`, `http`, `ssh`, `git`, `file` in lowercase; `file://` with a host, or a path opening with `\\` or `//` (another host's share); `http://` or `git://` at a ref that is not a `full_commit_id`, since nothing else checks what a plaintext transport carries; a ref `git check-ref-format` refuses, or one opening with `-`. `Dir` is never parsed here.
- **`overlay_layer`** makes the layer of an AOS overlay from its tree, whose digest it computes: the root variable `<NAME>_OVERLAY_ROOT` (a `Value`), then each env file's and `env`'s variables as written, then path additions per variable, merged as any layer's; bundled skills `skills/<name>` (a skill the overlay names by source or by a name for a catalog is not bundled: it is in `AosLayer::elsewhere`, never in `profile.skills` from this function); MCP servers (a command with a separator made absolute under the root; `env_passthrough` and `headers_env` as optional `env:` references; no header over plain `http` to another host); `contexts` from rules, routing lines, `context` and the harness's `context`; `model` and `effort` from `[harness.<input.harness>]`. Mapping entries are taken in key byte order. Refused whole: a name AOS refuses, a tree it cannot digest, an unreadable or invalid `overlay.cfg` (problems under `/about` alone never refuse), a missing or unreadable env file, a variable `is_reserved_env` names. Not applied, and listed in `not_applied`: the `bin` and `lib` defaults, env scripts, setup, dependencies, `runtime`, `scripts`, `mcp.skills`, an MCP `timeout` and working directory, other harnesses' hints and the harness's own keys.
- **An AOS layer in a merge** adds an `OverlayReport` (grants: the merged `share_env`, as `env` provider keys) to `overlays`, and its `AosLayer` to `aos`. `unset = ["context"]` also takes back every `contexts` entry below. `Effective::environment` builds the overlays' environment first, lowest first, over this process's environment through `lookup`: every root variable, then each overlay's env files, `env` and path additions with substitution. A placeholder reads what overlays set and the shared variables only; any other variable is seen by a path addition and `if_unset`, never a placeholder. A variable an overlay won takes the built value (an optional reference not resolved leaves it out); an error is `Resolve` with key `overlay:<identity>.env.<subject>`. The view adds `contexts` and `overlays` when there are any.

## Overlay hooks (`canoa_host::profile`)

A hook is a program an overlay runs at a named moment of a session (`hook.<ext>` and `[hooks]` in the overlay's manifest, the specification's `hooks.md`). Canoa fires each event it sees over ACP, for the overlays of a profile (a `[host.profiles]` profile, or canoa.cfg's top level for a source of the host's own). It fires nothing for an overlay it does not read, and a hook never grants: nothing it prints approves a prompt, a tool call or a session.

- **Off until allowed, by digest.** An overlay's hooks run only when canoa.cfg names the overlay's tree digest (`allow_hooks = { team = "sha256:..." }` in `[host.profiles.NAME]`, `[agents.NAME.hooks]` team = "sha256:..." for a source of the host's own), or when the person, asked by the host, allows them for one session (`_canoa/profile/approve`: the question carries `hooks`, one entry per overlay with `overlay`, `digest`, `changed`, `failClosed` and its `events`, each `{event, run, timeout, onFailure, match?, sets?, fires}`; the answer's `allowHooks` lists overlay names, and a name the question did not list is ignored). Both allowances name the files' digest, so a changed overlay is off again (`changed` says it was allowed at another version) and never allowed by itself. An overlay with a hook that has `on_failure = "refuse"` is not activated while its hooks are off (the report says `left out`): a guard that is off would fail open. Only a client that is this process's own link and says it answers (`APPROVE_META`) is asked; every other client gets canoa.cfg's allowances only.
- **What runs.** Each hook runs from a private copy of the overlay's files made from the bytes the digest covered (removed with the connection), with that copy as its working directory, in its own process group (a job object on Windows) that ends when it does, with nothing inherited: `PATH` (absolute entries only), a few variables of the host's own that name no credential (`HOME`, `LANG`, `TMPDIR`... or `SystemRoot`, `TEMP`... on Windows), `AOS_HOOK_EVENT` and the overlay's root variable (the copy's path). The event is the first argument; the payload, exactly the keys the specification gives the event, is one JSON object on standard input. Interpreters are the first of `python3`/`python` (`python`/`python3` on Windows), `node`, `pwsh`/`powershell`, `bash`, `sh` found on the host's `PATH`; a hook whose interpreter is missing fails as `not-run`.
- **Bounds** are `coqui_aos::Settings`' defaults: 60 seconds a run (or the entry's `timeout`, if shorter), 120 seconds an event, 64 KiB of output (read no further), 1 MiB of payload (the hook is not run), 1024 characters of `reason`. Standard error is read and dropped. A run that fails (`exit`, `timeout`, `too-large`, `grant`, `output`, `not-run`, `payload-too-large`, `event-timeout`) applies nothing and is logged by cause, never quoting its output; with `on_failure = "refuse"` it also refuses the event (a prompt is blocked, a tool call denied, a session or the agent's start refused).
- **Output** goes where the specification says and no further: `context` is added as a pair of content blocks (a label Canoa wrote, then the hook's text alone, so nothing it prints can pass for the label), `decision = "block"` stops a prompt, `"deny"` a tool call, `"ask"` leaves it to the person. `context` for a new session goes into `systemPrompt` when the agent says it takes one; otherwise, and for a resumed session and for `post-tool`, it goes after the user's own blocks in that session's next prompt, once. `env` is applied only for `pre-env` (the agent's process environment; a name the profile may not set from an overlay, `is_reserved_env`, is left out and reported); for every other event it is `hook-not-applied`, since ACP has no way to set a variable for the agent's later commands.
- **Session identity.** `session` in a payload is Canoa's own id for the session (random, per connection), never the agent's.
- **The report** (`_meta[PROFILE_META]`, and the profile dialog) lists `hooks.NAME` (off, with `hook-not-allowed`) or `hooks.NAME.EVENT` per hook: `applied`, `applied` with `hook-partial`, or `not supported` with `hook-unsupported`; a failure appears as `hooks.NAME.EVENT` refused with `hook-failed: CAUSE` on the answer to the next session request.

What fires, per event and harness. Every harness is reached through ACP, so the table is the same for all of them except where it says:

| Event | Canoa fires it | Output applied | Cannot |
| --- | --- | --- | --- |
| `pre-env` | Before the agent starts, once per connection, for hooks canoa.cfg allows | `env` (the agent's environment), a failed guard starts no agent | the person's allowance (nobody can be asked yet) |
| `session-start` | `session/new` as `new`; `session/load` and `session/resume` as `resume` | `context`; a failed guard answers the request `-32000` | `clear`, `compact`, `fork` (never sent: a hook matching only those is `hook-unsupported`); `env` (`hook-not-applied`) |
| `session-end` | `session/close`, and for each session still open when the connection ends | none | a killed host, an agent that exits without closing |
| `prompt` | Each `session/prompt`, before the agent has it (declared only) | `context`, `block` (the client gets `stopReason: "refusal"` and a message naming the overlay and the reason) | `env` |
| `turn-end` | The agent's answer to a prompt, after it was passed on | none | continuing the turn |
| `pre-tool` | Each `session/request_permission` (declared only) | `deny` (the host rejects it itself with the request's `reject_once`, else `reject_always`, else cancels, and the client is never asked), `ask` and no decision (the client is asked as it would be) | A call the agent's own rules allow, which asks nothing (`hook-partial`); `context` and `env` (`hook-not-applied`) |
| `post-tool` | The `tool_call_update` that ends a call, `completed` or `failed` (declared only) | `context` (next prompt) | none; the call has run |
| `cd`, `pre-compact` | Never: ACP shows neither | none | `hook-unsupported` |

By harness (`harness` in a payload is the AOS id, from the agent's `initialize` or an entry named `Claude`, `Codex`, `Gemini` or `Pi`; any other entry is `canoa` for `pre-env`): `session-start` context reaches the system prompt of an agent that says it takes one (the Claude and Codex adapters when they announce it) and the first prompt of one that does not (Pi, Gemini CLI, OpenCode, and any agent that says nothing). A tool's `kind` is the agent's own ACP `kind`, `other` when it sends none, and an `other` call runs every `pre-tool` and `post-tool` hook whatever its `match`; `tool.name` is Claude's `_meta.claudeCode.toolName`, else the call's `title`. Canoa writes no native hook into Claude Code's or Codex's configuration, so what only those could see (a call their rules allow, compaction, `clear`, a `cd`) is not fired.

## AOS overlays (`coqui-aos`)

`canoa_host` has no AOS module of its own: overlays are read, validated, digested, composed and their environment built by the [`coqui-aos`](https://github.com/Boriken-Dev/coqui-aos) crate (a git dependency at one revision; its `AGENTS.md` is the API header), which passes the specification's conformance vectors in full. What is Canoa's own is how a profile uses them (`profile::overlay_layer`, `profile::hosted`), held to `tests/vectors/profile.json`:

- **Reading.** The manifest is `coqui_aos::read_overlay` at the default `Settings`: `OVERLAY.md`'s front matter (restricted YAML, or the format its first line names) or `overlay.cfg` (TOML, JSON after `#!json`, YAML after `#!yaml`); a tag, anchor, alias or merge key is refused, and so are both files together (`manifest` refusal, subject the reader's class). A body of `OVERLAY.md` that is context becomes a `Context::Text` (the front matter is no context), never the file; `CONTEXT.md` beside it is not applied and is listed in `not_applied` as `context: CONTEXT.md, ignored ...`. Mapping order is the document's order; every order Canoa shows is key byte order.
- **A tree** is a `coqui_aos::OverlayTree` (every member's path and bytes), as `coqui_aos::read_tree` reads a directory (`OverlayInput::tree`); `overlay_layer` digests it again with `digest_entries`. A path under an excluded name is not in a tree `read_tree` returns, so it is neither digested nor delivered.
- **Limits** are `coqui_aos::Settings`: an installed overlay at the defaults (4096 files, 64 MiB), a host-profile skill's directory at 1024 files and 16 MiB.
- **The environment** is `coqui_aos::composed_environment` over the overlays in composition order, once after each overlay (`build_each`), from an `Environment` of the process variables an overlay's text names (any word of its manifest or env files), each marked secret unless a layer of the person's own shares it: a placeholder reads what overlays set and the shared variables only, while a path addition and `if_unset` still see the rest. `Built::owned` is a variable an overlay set (one the process already had with that value is not). A refusal is `ProfileError::Resolve` with key `overlay:<identity>.env.<subject>`.
- **The report** (`OverlayReport`) is `coqui_aos::effective` over the manifest less its env files and its required references nothing shares, which the report lists as `unresolved`: `ungranted` and `non_defaults` are the specification's for what Canoa applies (`unpinned`, `env-passthrough`, `secret-read`, `env-sensitive`, `insecure-transport`; not `env-script`, `setup-network`, `deps-network` and the rest, since nothing runs). When that call refuses (a partial lock, an env-path conflict) the report lists none, since the environment would refuse too.
- **A server over plain `http`** to another host (`insecure-transport` in that report) never gets `headers_env`.

## The control channel (`canoa_host::control`)

What belongs to no agent entry: the session ledger, the worktrees and, later, the overlay store. One host-level service,
named `@control` (the `@` is not a name an entry can have), speaking JSON-RPC 2.0, one message to a line as ACP does, but not ACP:
no `initialize`, requests and their answers only. Errors: `-32601` no such method, `-32602` bad params, `-32001` **denied** (the
caller has no grant for it), `-32002` **refused** (a limit), `-32003` not found.

**Who reaches it, and as whom**

- *The app*, in this process: `Local::open("@control")` (`canoa_ffi_link_open` with that entry name). The caller is the **owner**: every
  method, every record. Nothing on a network can obtain a `Local`.
- *A paired client*, from the outside: only when `[host.control]` names an address or pipe (`listen`, as an entry has it; default
  none, so no network control) and lists the key under `grants`. The connection proves its key as an entry's does (same method,
  same pairing); a paired key `grants` does not name is turned away after the proof, as an entry's `peers` turns one away. The
  caller is that **peer**: only the methods its grants name.

```toml
[host.control]
listen = "127.0.0.1:0"                              # or a pipe, as an entry has it; without it there is no network control
grants = { "sha256:…" = ["ledger.read", "ledger.write", "worktree", "store.read"] }   # per key; a key not here gets nothing
max_records = 10000                                 # ledger records the host keeps, per host; the oldest by `updated` are refused at, not dropped
max_record_bytes = 262144                           # one ledger record
```

| Grant | Methods |
| --- | --- |
| `ledger.read` | `ledger/list` |
| `ledger.write` | `ledger/put`, `ledger/remove`, `ledger/clear` |
| `worktree` | `worktree/new`, `worktree/status`, `worktree/remove`, `worktree/diff` |
| `store.read` | `store/list`, `store/report`, `store/latest`, `store/plan` |
| *(none)* | `store/apply`, `store/rollback`, `store/remove`: **owner only**; `[host.control]` refuses `store.write` as a grant |
| *(none)* | `history/export`, `history/move`: **owner only**; no grant names them |

**The ledger** is the list of the person's sessions, kept by the host in its home (`control/ledger.json`, written whole, atomically)
as opaque JSON objects the host never reads into: it keeps an `id`, the record, who put it and when.

- Every record has an **owner**: `"local"` for the app, else the peer's id. A peer sees and changes only the records it owns, and
  `put` makes it the owner whatever the request says; the owner (the app) sees and changes all.
- **Quotas.** `max_records` and 64 MiB of records are each owner's own (the app's, and each key's), the whole ledger holds 256 MiB, and the last 64 MiB are the app's alone, so a full ledger never refuses the app. A bad `ledger.json` is moved to `ledger.json.bad-<ms>` and an empty ledger begins (logged at `warn`); the file is written, synced and renamed, and `control/` and the file are `0700`/`0600` on Unix. A key's connection silent for 15 minutes is ended.
- `ledger/list` returns the caller's own records only; the owner (the app) asks for every owner's with `"all": true`, which a key's request cannot do. The app reads only its own, so nothing a key put is ever taken for a session. `{"after"?: id, "limit"?: 1..=500, default 100}` -> `{"records": [{"id", "updated", "owner", "record"}], "next"?: id}`,
  newest `updated` first; `next` is the `after` of the following page.
- `ledger/put` `{"id": text of 1 to 128 characters with no control character, "record": object}` -> `{"id", "updated"}`; replaces the
  record of that id (a peer cannot replace another owner's: `-32001`); `updated` is the host's clock, in milliseconds. Refused
  `-32002` past `max_record_bytes`, or past `max_records` for a new id.
- `ledger/remove` `{"id"}` -> `{"removed": bool}`; `ledger/clear` `{}` -> `{"removed": n}`, the caller's own records.
- A request over 16 MiB, or not JSON, ends the connection; `-32700` first when it can be answered.

**Session history.** The host records each session it serves in the session-history format (`coqui-acp-history`) in its home,
`history/`, one `<sha256 of entry and session id>.jsonl` a session in a directory and files only this user reads (`0700`/`0600`
on Unix). A turn is written when it ends, and a connection that ends inside one writes it as ended with an error, so a stop
loses at most the turn in flight. It records what reaches the agent and what the agent says, after the host's own checks, and
never a message over the limit: a session that passes a limit keeps what it had and nothing more.

```toml
[host.history]
record = true                       # false records nothing
max_session_bytes = 16777216        # one session's history; at most 64 MiB, since a history is read whole to be moved
max_sessions = 1000                 # sessions with a history; a new one past it is not recorded
max_total_bytes = 1073741824        # every history together
```

| Method | Params | Result |
| --- | --- | --- |
| `history/export` | `entry`, `session`, `max_bytes`? (1 to 4 MiB, default 1 MiB), `after`? | `{"id", "text", "next"?}`: one page of whole lines of the history as it is, the first with the header; `next` is the `after` of the following page |
| `history/move` | `entry`, `session`, `to` (an entry that runs a harness), `cwd`? (absolute; the history's own by default, and under the target's working roots) | `{"session", "entry", "cwd", "replayed", "compacted", "seededFrom"?, "recorded", "losses": [{"kind", "count", "detail"?}]}` |

- `entry` and `session` are the source entry's name and the session id its client used. A session with no history, an
  entry that is not a harness and a host that records nothing answer `-32003` or `-32002` with the reason.
- A move imports the history into the target at a **new** UUID the host makes (never an existing session), by the provider's
  `import_session`: Claude and Codex write their own session files; an entry whose provider replays (`replay_max_bytes` set on
  its settings) carries the history as context in the first prompt and says so (`replayed`, and `replayed_as_prompt` among the
  losses); any other provider refuses with its reason. The new session then has a history of its own (`recorded`), which can be
  exported or moved again. A client opens it by `session/resume` or `session/load` of that id on the target's entry.
- `losses` is what did not carry (`thinking`, `tool_as_note`, `attachment_by_digest`, `replayed_as_prompt`, `history_cut`,
  `item_cut`, `non_text_omitted`, `summary_missing`), counts only, never the lost content. The history is context, never
  commands: nothing is run again.
- Both methods are the owner's alone, over the app's own link: a recorded session holds what the person and the agent said,
  tool output included, which a paired key has no grant to read.

**`worktree/*`** takes the params and answers the results of `_canoa/worktree/*` ("Worktrees" below), with every check of
theirs unchanged (branch names, repositories led out by links or share paths, no hooks, `MAX_WORKTREES`). The owner (the app)
may name any absolute path on this computer; a key with the `worktree` grant only paths under the `working_roots` of the entries
whose `peers` allow it (all paired keys when an entry has none) and that have `worktrees` on, else `-32602`. The entry-level
`_canoa/worktree/*` stays for a client that reaches an agent over the network and holds no control grant.

**The program Store** (`canoa_host::store`, `Programs`) is carried the same way. `provider` names a store (`claude_store`, `codex_store`,
`grok_store`, `agy_store`, `pi_store`; `ServeOptions::with_programs` offers others), a program kept under the user's programs folder.
The params and results are the JSON of `canoa::store`'s models:

| Method | Params | Result |
| --- | --- | --- |
| `store/list` | none | `{"stores": [StoreInfo + "provider"]}` |
| `store/report` | `provider`, `probe`? (bool), `pick`? | `ProgramReport`: the copies the host finds; no network |
| `store/latest` | `provider`, `channel`? | `LatestReport` |
| `store/plan` | `provider`, `action` (`install`, `update`, `uninstall`), `channel`?, `version`? | `InstallPlan`, with its `token`: what an apply would do, nothing written |
| `store/apply` | `provider`, `plan` (as handed back) | `{"applied": bool, "version", "dir", "lines"}`; `applied` is false when an update has nothing to do |
| `store/rollback` | `provider` | `{"provider", "version", "dir"}` |
| `store/remove` | `provider` | `{"provider", "removed": true}` |

- The host applies **its own copy** of the plan it made (found by provider and `token`, used once, good for 15 minutes), so a plan a
  client forged or kept is refused `-32002`.
- An apply takes long and says where it is: the host sends `store/progress` notifications `{"id": the apply's request id, "text",
  "fraction"?}` on the same link before the answer, and a question the install asks the person comes as the request `store/ask`
  with the install's form (`{"title", "schema"}`, `ask-N` as its id), answered with `{"values", "secrets"}` as a result. The link
  closing, or ten minutes with no answer, cancels the install. Other requests on the link are still answered meanwhile.
- `store/list`, `store/report`, `store/latest` and `store/plan` need `store.read`; `store/report` with `probe` or `pick`, which runs a program of the user's, is the owner's, and a plan a key asks for is not kept. `store/apply`, `store/rollback` and
  `store/remove` are **the owner's alone**: there is no `store.write` grant and `[host.control]` refuses the name, since an install
  runs code as the user. A store that is not one of the host's is `-32003`; a plan or install the host refuses `-32002`.
- The overlay store (AOS overlays) follows as `overlay/*` under the same two grants; it is not built.

## Worktrees

No public API: what `serve` answers on an agent or tool entry with `working_roots` and `worktrees` on (the default), before its agent starts. Off, or with no root, each is `-32602`.

| Method | Params | Result |
| --- | --- | --- |
| `_canoa/worktree/new` | `cwd`, `branch` | `{"path", "branch", "repo"}`: `path` and `repo` written under the root as configured |
| `_canoa/worktree/status` | `path` | `{"exists": false}`, or `{"exists": true, "branch" (null when detached), "detached", "changed", "commits", "listing"}` |
| `_canoa/worktree/remove` | `path`, `discard` (bool, default false) | `{"removed": true, "branch", "branchKept", "commits"}` |
| `_canoa/worktree/diff` | `path`, `base` (a commit's hex id; HEAD when absent) | `{"exists": false}`, or `{"exists": true, "head", "branch", "base", "added", "removed", "files", "more"}`; a file is `{"path", "status" (`A`, `M`, `D`), "added", "removed", "binary", "patch", "cut", "print"}` |

- **A branch** is `canoa/` then runs of `a-z0-9` joined by single hyphens, 70 bytes at most; any other is `-32602` before git runs.
- **A path** (`cwd`, `path`) must be plain absolute, on a drive on Windows (a share or device path is refused unlooked at), under a root as written, and, resolved by `canonicalize`, under a root resolved the same way. A missing `path` is `{"exists": false}` for `status`.
- **Every request** first refuses a repository (`-32602`) when a `.git` file from the directory up to the roots, or its `commondir`, names a share or device; when `git config --local` or `--worktree` holds a key matching `^(filter|include|includeif)\.|^core\.attributesfile$`; when `rev-parse --git-common-dir` resolved is not the main worktree's `.git` (a submodule, `--separate-git-dir`) or is outside the roots; or when it has `objects/info/alternates`.
- **`new`**: the repository's main worktree (`git worktree list --porcelain -z`, its first entry; a bare one refused) resolved must be under a resolved root, and so must its parent; the worktree goes in `<parent>/<name>.worktrees/<branch less canoa/>`. An existing `.worktrees` that is a link or not a directory, a worktree already there, or `MAX_WORKTREES` (32) worktrees of the repository already in it, is refused. Then `git worktree add -b BRANCH -- PATH HEAD` from `cwd`; a result whose resolved path is not itself is removed again and refused.
- **`status` and `remove`** take only a worktree `new` would have made: its own top level (`rev-parse --show-toplevel`), not the main worktree, where `new` puts the branch `canoa/<its directory's name>`, whatever its HEAD is now; else `-32602`. `changed` counts what `git status --porcelain=v1 -z --untracked-files=all --ignored=matching --ignore-submodules=all` lists, and `listing` is the SHA-256 of that output in hexadecimal; `commits` counts `rev-list HEAD [refs/heads/BRANCH] --not --exclude=BRANCH --branches --remotes --tags`.
- **`diff`** is read only and takes any repository the checks above accept, not only a worktree `new` made. It lists what differs from `base` in the folder `path` names (paths relative to it): tracked files by `git diff` with no external diff program and no text conversion, and untracked files that are small text, shown as added lines. `patch` holds the hunks with no file header, cut at 96 KiB (`cut`); a binary file, or one over 20,000 changed lines, has none. `print` stands for the file's change, so two reads with one print show it changed the same way. At most 300 files are listed (`more`), the totals count all. Git runs with `--no-optional-locks`, so it never rewrites the index. A `base` that is not a commit of the repository is `-32602`.
- **`remove`** refuses a detached HEAD reaching any commit no branch, tag or remote has, whatever `discard` says, and anything `changed` counts without `discard` (git itself would remove ignored files). Then `git worktree remove [--force] -- PATH` (`--force` only with `discard`), and, when the branch exists and holds no commit of its own, `git update-ref -d refs/heads/BRANCH TIP` at the tip counted: a branch that moved meanwhile stays (`branchKept`).
- **Git** is `git` on the host's `PATH`, run as `git -C DIR -c core.hooksPath=NUL|/dev/null -c core.fsmonitor=false ...` with every `GIT_` variable of the host's removed, `GIT_TERMINAL_PROMPT=0` and `GIT_CEILING_DIRECTORIES` the roots' parents, ended after 120 s; a failure is `-32603` `git failed: ` and up to 1 KiB of git's own words, control characters made spaces.
- A request is refused, never forwarded, once the agent runs (`-32600`), and a notification of one is dropped.

## Pairing (`canoa_host::peers`)

```rust
#[non_exhaustive] pub struct Peer { pub id: PeerId, pub state: PeerState }
#[non_exhaustive] pub enum PeerState { Accepted, Waiting { source: Option<SocketAddr> } }
pub fn list(store: &TrustStore) -> Result<Vec<Peer>, PeersError>     // accepted, then waiting, each by id
pub fn accept(store: &TrustStore, id: &str) -> Result<PeerId, PeersError>
pub fn revoke(store: &TrustStore, id: &str) -> Result<bool, PeersError>   // false: the store did not know it
```

A key is paired when the store accepts it. `accept` takes only a key that is waiting, so a person pairs a client by connecting with it once, then accepting the id the client shows.

## One host on a home (`canoa_host::home`)

```rust
pub fn Home::at(dir: impl Into<PathBuf>) -> Home
pub fn Home::of_user() -> Result<Home, coqui_agentic_peer::Error>   // COQUI_AGENTIC_PEER_HOME, else the platform's place
pub fn Home::dir(&self) -> &Path
pub fn Home::identity(&self, method: Option<Method>) -> Result<Identity, coqui_agentic_peer::Error>   // as coqui_agentic_peer::home, in this directory
pub fn Home::store(&self) -> Result<TrustStore, coqui_agentic_peer::Error>
pub fn Home::lock(&self, name: &str) -> Result<HomeLock, LockError>
pub fn Home::holder(&self) -> Option<Holder>   // by what it wrote; proves nothing about the lock
#[non_exhaustive] pub struct Holder { pub name: String, pub pid: u32 }
#[non_exhaustive] pub enum LockError { Held { dir: PathBuf, holder: Option<Holder> }, Io { dir: PathBuf, source: io::Error } }
```

- `lock` takes the operating system's lock, made with its parents when absent: on Unix on the directory itself, on Windows on `host.lock` in it, held open so that nobody can delete or rename it meanwhile; removing a file cannot let a second host in. It writes `{"pid", "name"}` to `host.holder` through a file of a name nobody could know in advance, made only where nothing is. So a holder that dies gives it up however it died, and a process id used again proves nothing. `Held` says who, or `None` when the holder has not said within half a second. Two locks in one process conflict too.
- Dropping a `HomeLock` removes `host.holder` and gives the lock up; `host.lock`, on Windows, stays.
- The lock is only as good as the filesystem's: a home on WSL's `/mnt/c` (9p) is not locked against a host on Windows, and a network filesystem is untested.
- What `Display` says of `Held` names the holder, its process and the home. `canoa serve` and the app show it.
- A file system that has no such lock is `Io`: that host does not start.

## Delegation (`[host.delegate]`)

An agent session hands a task to another source. Each task is an A2A task (`coqui-a2a`, `tasks` feature) of the host's one delegation agent, run as a normal host session of that source opened in process with `Local`, whose only prompt is the task text.

```rust
#[non_exhaustive] pub struct DelegateConfig { pub enabled: bool, pub sources: Option<Vec<String>>, pub max_children: usize, pub max_running: usize, pub max_depth: usize, pub task_bytes: usize, pub result_bytes: usize, pub history_bytes: usize }   // Default, Clone, Eq: `Config::delegate`
pub const DELEGATE_META: &str = "dev.boriken.canoa/delegate";   // the `_meta` key of `initialize` that asks for the tools
```
- `[host.delegate]` keys (any other is refused): `enabled` (true), `sources` (unset: every entry the delegating connection may use; a list narrows it, an empty one allows none), `max_children` per session (8), `max_running` per host (16), `max_depth` (1: a session hands out tasks, a task hands out none), `task_bytes` (65536), `result_bytes` (65536, the most of a reply one `task_status` carries), `history_bytes` (1048576, a task's updates and reply the host keeps). No floor; a value away from its default is logged at start.
- Offered only when the client's `initialize` `_meta` carries `DELEGATE_META: true`, `enabled` is not false, and the agent takes an MCP server: over HTTP when its `initialize` says `mcpCapabilities.http` (`capabilities.session.mcp.http` in version 2), else over stdio through the bridge below (version 1 requires stdio of every agent; version 2 must say `capabilities.session.mcp.stdio`). Then each `session/new`/`load`/`resume` gets an MCP entry named `canoa-delegate` (any client entry of that name is replaced): an HTTP one at `http://127.0.0.1:PORT/mcp` with an `Authorization: Bearer` header, or the bridge's stdio one. The bearer is 32 random bytes as 64 hex digits, minted per session, valid once the session's id is known, revoked when the connection ends, kept only as a digest, never logged. One loopback server per host; it refuses a request with no or another bearer (401), an `Origin` or `Sec-Fetch-*` header or a `Host` other than its own socket (403), any path but `/mcp`, any method but POST,  and a chunked body. A connection has 2 seconds to send its head before any bearer is checked and counts against 32 such connections until it has shown one; after that a call counts against its session's 8 (a ninth is 429), so one session's long `task_status` waits cannot crowd out another's.
- **The stdio bridge** is `canoa mcp-bridge` (hidden; `canoa-ffi` runs it as `--canoa-child mcp-bridge`, so an application with no `canoa` on disk has it too): a stdio MCP server that relays each request line to the loopback server and the answer back. The host writes it as the entry `{name, command, args, env}` with `command` and `args` the host's own program (`ServeOptions::provider_program` and `program_args`) then `mcp-bridge`, and `env` holding `CANOA_DELEGATE_URL` and `CANOA_DELEGATE_TOKEN`, so the bearer is never on a command line another user can list; version 2 adds `type: "stdio"`. It reads both variables from its environment and ends with status 2 and a reason (never the token) when the URL is missing, is not `http://` on a loopback address (`127.0.0.0/8`, `::1`, or `localhost` resolving only there), or the token is missing or holds a character a header cannot carry. It holds no state: one HTTP request per line, up to 8 at once (answers come as they finish, by id), a notification the server takes gets no answer, a request the server cannot answer (no or revoked bearer, no listener, a busy session) gets a JSON-RPC error with code -32000, a line that is not JSON gets -32700, and it exits 0 when standard input closes (in-flight calls are dropped) and 1 when a line is longer than 16 MiB or reading fails.
- Tools (JSON answers only): `delegate_task {source, task, folder?, title?}` answers `{taskId, state}`; `task_status {taskId, wait_ms?}` answers `{taskId, state, title, source, message?, result?, truncated?, resultBytes?}` (`state` is `submitted`, `working`, `input_required`, `completed`, `failed`, `canceled` or `rejected`; a wait ends on a state change or a terminal state, and a timeout cancels nothing); `cancel {taskId}`. They are `SendMessage` (`returnImmediately`, `tenant` = source), `GetTask`/`SubscribeToTask` and `CancelTask` with the caller `local:CONNECTION:SESSION`, so a session sees only its own tasks (another's id is `TaskNotFound`).
- Privilege, checked before any process starts: the source is one the connection may use (an entry's `peers` for a key, any for the owner) and in `sources`; the folder is canonicalised (links, `..`) and equal to or inside the session's `cwd` and its entry's working roots; the mode is the session's own only when the task's agent is the same kind (the same `harness` provider, or the same program) and lists it, since a mode id means something only to the agent that named it; otherwise `manual`, else `default`, else the task is `rejected` (no argument sets a mode); `max_children`, `max_running`, `task_bytes`. The task's agent is given no `fs`/`terminal` capability, and its `session/request_permission` goes to the delegating connection on the parent's session id with `toolCallId` `TASKID:ORIGINAL` and `_meta["dev.boriken.canoa/delegate"] = {taskId, title, source}`; the task reads `input_required` until the person answers, and no answer or a closed connection cancels it.
- **To the client** (a client that opted in; a task's own connection is never sent these): the host sends the notification `_canoa/delegate/event` `{sessionId, taskId, event?, update?}`. `event` is A2A's `StreamResponse` for the task (`task` first, with `metadata` `{title, source, folder}`, then `statusUpdate` and `artifactUpdate`, the last status ending the stream), `update` is one `session/update` of the task's agent as it sent it. Updates beyond 1024 waiting in the outbox are dropped (the history keeps them); states never are. The client may ask `_canoa/delegate/history {sessionId, taskId}` for the updates the host kept of a task, a running one or one of the last `max_children` that ended in the session (`{taskId, updates}`, or `-32602` for another id), and `_canoa/delegate/stop {sessionId, taskId?}` to cancel every task of the session or one (`{}`); both are answered on the stream and never reach the agent. The host's own messages in the `_canoa/` namespace pass the scope's filter only while the host is sending them (`scope::host_own`).
- Stop: the parent's `session/cancel` and the `cancel` tool cancel the task (`session/cancel`, then the process is ended after a grace period); a closed connection or a stopped host ends every task and its process.
- Built-in translators: Claude and Codex (the host's `harness` entries) and Grok say `mcpCapabilities.http` and take the entry; Antigravity says none, so gets none.

## Secret prompts (`coqui-acp-secrets`)

An agent asks the person for a password or a key, or shows them one, and neither the model, its transcript, the host's history nor its log ever holds the value. There is no setting: a session is given the tools when its client offered them and its agent takes MCP servers over HTTP or over stdio.

- **Offered by the app.** The client's `initialize` carries `clientCapabilities._meta["dev.boriken.coqui.acp/secrets"]` (`capabilities._meta` in ACP v2), `{"version": 1, "methods": ["request", "show"], "bindings": [..]}`. The host reads it on the raw stream, before the scope removes `_meta`. A binding is `ecdsa-p256-sha256` for a host whose key is WebCrypto and `x25519-hmac-sha256` for a Noise one: an app that does not list the host's binding gets no tools.
- **Whose identity.** The frontend of a connection is its own proven peer, whose key the host knows from pairing (the trust store), and every answer is bound to it and to the host's key; one signed by any other identity is refused. A connection whose peer is the host's own identity (a task's link, an application hosting the library through `Local`) has no other identity to bind with, so its sessions get no secret tools. The host's key is used in this process as the library's `unprotected` signer does (the process already holds it).
- **The tools.** Each `session/new`, `session/load` or `session/resume` of such a client gets an HTTP MCP entry named `canoa-secrets` (any client entry of that name is replaced) at `http://127.0.0.1:PORT/mcp` with an `Authorization: Bearer` header: 32 random bytes as 64 hex digits, minted per session, kept as a digest, valid once the session's id is known, revoked when the session closes or the connection ends. One loopback server per host, started at the first session; it refuses no or another bearer (401), an `Origin` or `Sec-Fetch-*` header, a `Host` other than its own socket (403), any path but `/mcp`, any method but POST, a chunked body, more than 4 calls of one bearer at once (429) and a body over 256 KiB. The tools are `secret_request`, `secret_use` and `secret_show` of `coqui-acp-secrets` (its header says what they take and answer); a session's secrets are cleared with it.
- **Asked of the app.** `_coqui_acp/secret_request`, `secret_show_open` and `secret_show_deliver`, and a use's approval as `session/request_permission` (options `allow-once`, `allow-session`, `reject`; the card shows the command, the program by full path and the notice that the folder's scripts belong to the agent). They are written on the client's own stream with ids `canoa-secret-N-HEX`, and the answers are taken off it: neither passes the scope, the profile, the history or the delegation. An agent's own request or notification whose parsed method starts with `_coqui_acp/secret_` or `_canoa/` is dropped (`scope::dropped`, on every entry), and so is any line of the agent's this host's JSON reader cannot read (a lone surrogate, nesting past 128 levels, a number out of range), which a more lenient reader in the client might take for one (logged once per connection, without the line), so it cannot put a value on the person's card in the host's name.
- **The mode decides a use.** A secret's use is a permission: in a mode the app calls Bypass (`bypass`, `bypassPermissions`, `full_access`, `full-access`, `fullaccess`, `yolo`) it is approved without asking; in any other the person is asked, with allow once and allow for the session. **The person's choice decides, never the agent's word:** only the client's own `session/set_mode` or `session/set_config_option`, answered OK by the agent, for a mode the session listed when it opened, puts a session in Bypass. What the agent says of itself (the mode it starts in, a `current_mode_update` or `config_option_update`, an echo of the options) can only lower it, so an agent that starts in Bypass or moves there itself has the person asked.
- **Stdio agents.** An agent that does not say `mcpCapabilities.http` gets the same tools as a stdio server running `canoa mcp-bridge` (the delegation tools' bridge), the address and bearer in its environment.
- **Not served.** A Noise app whose page does not know the host's static key.

## Accounts

An agent entry with `harness` can offer accounts: logins of the provider's, each its own `[host.accounts.NAME]`. The provider owns how its harness takes one (kinds, variables, files, login commands: its `accounts` catalog); the host owns which exist, where their credentials rest, who holds one and what a renewal does.

- **Where a credential rests.** In the operating system's credential store (Windows Credential Manager, macOS Keychain, Secret Service) through `keyring-core` and the platform's store crate, never in canoa.cfg, the ledger or a log. The service is `dev.boriken.canoa.` and a digest of the home's path, so two homes never share a credential, and an entry's name holds a digest of the account's id, never the id. On Windows the credentials are made local to the machine (persistence `Local`) and are not roamed with the profile. Windows holds at most 2560 bytes in one credential and a Codex `auth.json` is several kilobytes, so a record (the credentials, `revision`, `expires_at`, `needs_sign_in`) is cut into chunks of 2048 bytes, one entry each, under a generation that an index entry names: a write that stops half way leaves the old record whole, one writer at a time per account in the process, an index that cannot be read is never written over, and a removal deletes every generation it can find. `[host] account_store = "file"` keeps a file per account in `HOME/accounts/` instead, sealed with AES-256-GCM to a key (`store.key`, 32 bytes) beside it, the account's id in what is authenticated; the key is written whole beside its place and linked in without replacing another's, and each file is written beside its place and renamed over it, with the directory synced; for a host with no credential store (a headless Linux), logged as `non-default: host.account_store is file` since it protects against a copied file and not against anyone who can read the directory. **The accounts directory is the owner's alone**: `HOME/accounts/` and everything made inside it are mode 0700 and 0600 on Unix, and on Windows `accounts/` is given a protected ACL naming this user and the system, which what is made inside inherits (an `accounts/` that already exists is left as it is; a directory whose ACL cannot be set is refused). One directory per account, named by a digest of its id: `state/` is where the harness runs, `lease` the file lock below, `placed` a note of the credential files placed in `state/`.
- **A session on an account.** The client names the account in `initialize`'s `_meta["dev.boriken.canoa/account"]` (its name); absent, the entry's first; a name the entry does not offer, or an account held, is answered `-32602` before anything starts. The answer's `_meta` names `{"name", "kind"}` and never a value. The provider is given the `AccountGrant` (the stored credentials, `revision`, and `state_dir` for a kind that needs one) and a spawner whose file methods serve that directory; the files it places there are taken away once the last connection on the account ends, the harness's own state stays. Usage is reported under the account's id. Each start logs the kind's notices, and `non-default: the host's environment sets NAME…` for a credential variable of the host's own that the account takes the place of (the provider removes every one it knows).
- **One holder.** A kind whose catalog says `rotates` is held by one connection from its start until it ends, across processes (an exclusive lock on `lease`); a kind that does not rotate takes a shared lock, so a command that signs in again or removes the account waits for every connection. A second session on a connection that holds a rotating account (`session/new`, `fork`, `load` or `resume`) is answered `-32602` until the first is closed; sessions are counted from the provider's successful answers, so a failed open counts for nothing, and only `session/close` of a session the connection holds frees it. `canoa accounts login|remove` are refused while a connection holds the account; `list` shows `in use` for a rotating one. `harness-login` is never leased or stored.
- **Renewals.** What the provider reports through the account keeper is stored when it is the next of its chain or newer than the stored copy, and never when it holds no credential, an empty one, or lacks a key the stored login has. A provider's hold names the revision of the grant it began from in every renewal of the hold and counts them, so the next report of a source whose last stored revision is the stored one is accepted whatever revision it names; any other report is stored only if its `from_revision` is not older than the stored `revision`, its `expires_at` (RFC 3339) not earlier, and the credentials differ. The stored `revision` grows by one each time. A sequence a provider instance has already told is ignored. A `lost` login sets `needs_sign_in` and keeps the last copy, which the next start uses; a sign-in (`canoa accounts login`) replaces it and clears the mark. A blank copy is never stored.
- **Signing in.** A login method runs in a directory made for it under `HOME/accounts/login/`, removed however it ended, with the provider's prompts relayed through the `Front`: `say`, `open_url` (with a device code) and `ask`, which a front without typed input refuses (`Front::ask` answering `None`) and the provider then fails the sign-in. An application hosting the library gets that front today: it can list, and can add an account only by a method that never asks, until sealed prompts exist. An `ask` is never answered from anywhere but the front. A sign-in directory is private, locked while the sign-in lives, and removed when it ends however it ended, a cancel included (the process waits up to three seconds for that before it ends); those a crash left, and the credential files a crash left in an account's `state/` (from the `placed` note), are swept when `Accounts` opens and before each sign-in, skipping any a live process holds. `--input` is checked against the method's input schema, and a secret input is refused on the command line.

## The command line as a library (`canoa_host::cli`)

```rust
pub const STATUS_HELD: u8 = 3;
pub trait Front: Send + Sync {
    fn print(&self, line: &str) -> io::Result<()>;     // a line of results: standard output for `canoa`
    fn say(&self, line: &str);                         // a line of diagnostics: standard error for `canoa`
    fn when_input_closes(&self, status: u8, then: Box<dyn FnOnce() + Send>) -> io::Result<()>;   // for --until-stdin-closes
    fn program(&self) -> io::Result<Program>;          // what runs provider, tool and launch for this host
    fn home(&self) -> Option<PathBuf>;                 // None: this user's
    fn holder(&self) -> &str;                          // what serve calls itself to another host
    fn ask(&self, prompt: &str, secret: bool) -> io::Result<Option<String>>;   // one typed line, a secret without echo; Ok(None): this front cannot take one (an application asks in its own forms)
    fn local(&self, local: Local) {}                   // default: ignored. serve hands over its way in, bound and before "ready"
}
#[non_exhaustive] pub struct Program { pub path: PathBuf, pub args: Vec<OsString> }   // args go before the subcommand
pub const fn Program::new(path: PathBuf, args: Vec<OsString>) -> Program
pub struct Cli;                                        // clap::Parser
pub const fn Cli::is_a_hosts_own(&self) -> bool        // provider, tool, agent or launch
pub fn run(cli: Cli, front: &dyn Front) -> Result<(), Failed>
pub struct Failed { pub status: u8, pub why: String }
```

Everything `canoa` does is here; `src/main.rs` is a `Front` on its own standard streams, and `canoa-ffi` another. `run` neither exits nor writes anywhere but through the front. With a `Front` that returns from `when_input_closes`, `serve --until-stdin-closes` returns once it has stopped, its lock given up.


## Program stores (`canoa_host::store`)

The host's end of `canoa::store`'s `program_store` kind: it finds, checks, installs and switches the agent programs Canoa runs, through
stores that know each program. Nothing runs at startup, and the host holds no program name, command or URL: those are the stores'.

- `shelf::Shelf` (`<programs>/<program>/{lock, current.json, versions/<version>-<nonce>/}`): `Shelf::default_root()` is
  `%LOCALAPPDATA%\Canoa\programs`, `~/Library/Application Support/Canoa/programs` or `$XDG_DATA_HOME/Canoa/programs`. `ProgramDir::lock`
  takes the program's OS lock, refuses a folder another user owns or others can write (Unix; on Windows it inherits `%LOCALAPPDATA%`'s access
  list, which is not read: no vetted crate checks an owner or an access list, and this crate holds no `unsafe`, so on Windows the folder is only
  refused when it is a link) and removes the version folders `current.json` does not name. `Locked::stage`, `commit` (one rename of
  `current.json` is the atomic switch; the old copy becomes `previous`; folders other than current and previous are pruned, `keep_versions`
  default 2), `abandon`, `rollback`, `remove`. `commit` refuses a launch path that is not a regular file inside the staged folder.
- `footprint::{Watch, Footprint, uncovered}`: before and after snapshots of profile files, `PATH` directories, the home's top level
  (and, in the suite, the whole sandbox) and, on Windows, `HKCU\Environment`, `Run`, `RunOnce` and the `Uninstall` names. `uncovered` is
  what no declared `Change` covers; `restore_registry` puts back the values of `HKCU` `Environment`, `Run`, `RunOnce`, the policy `Run` and the command processor's key (Windows).
- `tools::LocalTools` (`InstallTools`): HTTPS only (loopback `http` under `loopback_http`, a test's setting); every redirect hop's origin must be
  one of the store's `endpoints`; `ToolsLimits` (5 redirects, 60 s to a response, 30 min a body, 100 000 files, 4 GiB unpack) each a
  setting; the digest is computed as the bytes are written. `Transport` is the network seam (`HttpTransport` is `ureq` with rustls, trusting the operating
  system's verifier and store by default (`Roots::Platform`, so a TLS-intercepting proxy the system trusts works; `Roots::Webpki` is the other choice).
- `protect::Protected`: the list matcher (`new(lines)`, `protects(path, home)`, `holds(path, home)` for a path that is an ancestor of a named place, `protects_name(variable)`), a copy of coqui-acp-pi's. The host holds no list in code beyond `RECOMMENDED_PLACES` and `RECOMMENDED_VARIABLES`. `Programs` refuses a plan whose change is the home itself, is on `Options::protected`, holds a place it names, or (a state folder) is not named for the program; the `Run` a store runs a program with still strips the recommended credential variables.
- `exec::GuardedSpawner` refuses what a store must never start (relative program, `.cmd`/`.bat`/`.ps1`, a working directory outside the
  staged folder, a credential variable) and sets `NoDefaultCurrentDirectoryInExePath=1`. `exec::Run` runs a program to its end, relaying each cleaned
  line (`clean`: control and bidirectional characters out, 4 KiB) and keeping cancel and deadline.
- `programs::Programs`: `store`, `report` (the copies, no network), `latest`, `plan`, `apply`/`apply_store`, `rollback`, `remove`. An apply takes the
  lock, stages, gives the store an environment with every home and temp directory inside the staged folder, snapshots before and after, and
  only then switches. An undeclared change (`StoreError::Undeclared`) removes the folder, leaves `current.json`, names each item and puts back
  the `HKCU\Environment` values. After the store's `apply` returns the host goes on
  watching for `Options::settle` (2 s) and measures then: a change in that window is undeclared. A writer outside the job that waits longer than
  the window is not detectable. A failed install that declared changes is reported as `StoreError::LeftBehind`, naming them. Registry values are
  put back only where undeclared and still what the install wrote; a value the person changed since is left and named. Installs and updates on a
  shelf run one at a time (`Shelf::apply_lock`). It refuses an elevated host (root; on Windows `is_elevated`) and an `ELEVATION` plan unless
  `Options::allow_elevated`.
- `engine::{Engine, Recipe}` and `extensions::{anthropic, codex, grok, agy, pi}`: the five built-in stores, `registry::built_in()`. A recipe names the
  publisher's documents and the version's shape; the engine downloads, unpacks, runs the recipe's extra step and reads the version back.
  Claude: one binary, SHA-256 from the manifest (its signature is not checked). Codex: `codex-package` tar.gz, SHA-256 from the release's
  checksums and the channel's digest; no switch stops its updater (`self_updates`). Grok: one binary, no checksum published, so `Unverified`.
  agy: SHA-512 from the manifest, only the newest build (a pinned version is `UNSUPPORTED`). Pi: Node's standalone build (SHASUMS256.txt, its signature is not
  checked) then that Node's npm with `--ignore-scripts`; npm fetches the packages itself, so the host does not see those bytes.
- **Pi's launch** is a Node and a script, not one file: `Launch.path` is the absolute Node, `Launch.settings` holds `node` (the same path) and
  `script` (the absolute path of Pi's `dist/bundle/cli.js`), and `Launch.environment` sets `PI_SKIP_VERSION_CHECK=1`. A provider that serves
  Pi takes `node` and `script` as settings and starts the Node with the script, never a `.cmd` shim.
- **Known gaps, stated in each plan's notices:** Claude's manifest signature (`manifest.json.sig`, key published at downloads.claude.ai/keys) and Node's
  `SHASUMS256.txt` signature are not checked, only the digests over TLS; a digest fetched from the origin that serves the file does not
  catch a compromised publisher. npm's own downloads for Pi are not seen by the host.
- `contract::run(store, fixture)`: the suite (`describe.*`, `program.*`, `latest.*`, `plan.*`, `apply.*`, `uninstall.*`), strict and offline, on a `World` of
  documents and a stand-in program built by `cargo test` (`examples/version_standin.rs`, with a trailer appended per version). `echo` is the
  good example; `tests/store_suite.rs` has stores that break one rule each. The registry keys are the real ones and are restored.
- A store reaches a harness only through the host: `Launch.settings` (`{"executable": path}`) merged under an entry's own, and `Launch.environment`
  added to its requests; only the built-in Pi source reads a managed copy so far (its `node` and `script`; "The host's own sources"), and wiring it into `[host.agents]` is not here yet.

**What the host holds a store to, and what it does not (a security review, 2026-11)**

- A store is code running as the user. The host's checks catch a store's mistakes and an install's side effects; they are not a sandbox.
- The host applies the plan it made, found by its token (`Programs::plan` keeps it); a client's copy only names it. The token covers the
  version, platform, launch, every download and its check, and every declared change. A change that declares a place of its own is the only
  kind accepted: a root, the home itself or an ancestor of it, all of HKCU, is refused (`the plan declares ...`). `USER_PATH` covers
  the registry `PATH` value gaining exactly the declared directory at its end, and a profile whose only change is a line that is exactly a `PATH`
  assignment adding it (no whitespace or `\` in a POSIX directory, quotes balanced); a `REGISTRY` change never covers the user `PATH`.
- An install is measured on every outcome, a failure, a cancel and a timeout included: the machine is snapshotted before and after, the
  registry values put back, and the folder removed. Nothing is exempted for moving: a profile line, a `PATH` directory or a `Run` key that
  changes is undeclared, and the error asks the person to try again. During an install the other programs' folders under the programs root are watched too.
- The programs root is `Shelf::default_root()` or below (`Shelf::configured` refuses another) until an access-list check exists.
- Downloads nothing checks (Grok's) are refused unless `Options::allow_unverified`; `current.json` records each file's `verified`. Endpoints may be a prefix
  (`https://storage.googleapis.com/antigravity-public/antigravity-cli/`), which a URL and every redirect has to start with.
- `Programs::launch` reads `current.json` against a digest the host keeps in `<programs>/.digests/`, outside the program's folder, refuses a launch
  outside the copy or a copy outside the shelf, and checks every file of the copy (the files an archive unpacked too) against what was recorded
  at install: a changed, missing or added file refuses. Records survive a rollback. A person with write access to the programs folder can rewrite
  both `current.json` and the digest: the check is against a stray edit, not against the owner's own account.
- A plan is kept by the host per (provider, token) for `Options::plan_ttl` (15 minutes) and `plan_limit` (32), and applied once. The token covers the
  action and the version being replaced. A plan with a registry key, a state folder or a profile file is checked by name: the home's sensitive
  trees and their ancestors are refused, a state folder must be named for the program (or under such a folder), a registry key must be the user's
  `Path` or the program's own `Software\<vendor>\<program>` or `Uninstall` entry. `USER_PATH` names the directory it adds; it covers a profile line
  that is exactly `export PATH=<dir>:$PATH`, `fish_add_path <dir>` or a PowerShell `$env:Path = "<dir>;$env:Path"`, or the registry value gaining
  exactly that directory at its end. An endpoint path with a dot segment or an encoded slash (after one decoding) is refused.
  `Shelf::configured` resolves `..` before it checks the root is the default's; `Shelf::unchecked` is for tests.
- A launch is recorded only when its variables are not `PATH`, `NODE_*`, `LD_*`, `DYLD_*` or credentials, and its `executable`, `node` and `script` settings lie in the version's folder.
- An install is given an environment without the person's `npm_config_*`, `NODE_*` and `SSH_AUTH_SOCK`; Pi's npm is told its registry. Pi's packages
  are not pinned by a shrinkwrap yet (`npm ci` from a per-version lockfile is a later step).
- `Programs::report(probe)` runs the host's own copy only, and of the outside ones the one the person picked (`pick`).
- A downgrade (an update to an older version) is planned with a notice that says so.
- Install, update and remove are for the person who owns the host: nothing here serves them to a paired peer, and a later phase that does has to ask the person.
- Not checked yet, by decision: Claude's `manifest.json.sig` and Node's `SHASUMS256.txt.sig`. A pure-Rust OpenPGP verifier (`pgp`, rpgp) is the way; it adds
  its RustCrypto tree to this crate and the pinned keys (Anthropic's signing key, and the Node release key of the pinned Node) to the source.

Tests: `cargo build -p canoa-host --examples` first, then `cargo test -p canoa-host --all-features --test store_tools --test store_shelf
--test store_programs --test store_suite --test store_extensions`. `tests/store_live.rs` (ignored) installs the real programs:
`CANOA_STORE_LIVE=claude,codex,grok,agy,pi cargo test -p canoa-host --test store_live -- --ignored --nocapture`.

## Errors

- `ConfigError` (`#[non_exhaustive]`): `Read { path, source }`; `Parse { line, column, what }`, 1-based, `what` in the schema's words; `Invalid { entry, what }`. No variant quotes a value from the file; an entry's name is quoted.
- `ProviderError` (`#[non_exhaustive]`): `Load(String)`, `Source(String)`, `Missing`, `WrongKind`, `NeedsConfiguration`, `TakesNoSettings`, `Settings(String)` (names a setting, never its value, unless the provider's own words do), `HostLine(&'static str)`, `Transport(io::Error)`, `Facade(String)`.
- `PeersError` (`#[non_exhaustive]`): `NotAnId`, `NotWaiting`, `AlreadyAccepted`, `Store(String)`.
- `WebError` (`#[non_exhaustive]`): see "The web app". `source()` is the `io::Error` of `Listen`.

## Command line

```text
canoa serve [--config FILE] [--until-stdin-closes] [--allow-empty] [--local-sources] [--method noise|webcrypto] [--key-limit BYTES] [--no-rekey]
                 [--web [DIR]] [--web-listen ADDRESS]
canoa check [--config FILE] [--format text|json]
canoa id [--method noise|webcrypto [--force]]
canoa peers [--format text|json]
canoa accept PEER
canoa revoke PEER
canoa pair [--listen ADDR] [--timeout-seconds N] [--until-stdin-closes] [--method noise|webcrypto [--force]]
canoa provider (--library FILE | --source FILE [--import-search library|path]) --provider ID [--working-root DIR]... [--max-sessions N]
                    [--directory DIR] [--settings-on-stdin [--connect ADDRESS]]
canoa tool --name NAME --spec JSON
canoa agent claude [--executable FILE] [--permission-mode MODE] [--setting-sources LIST] [--strict-mcp-config]
                   [--no-permission-prompts] [--allow-client-plugins] COMMON...
canoa agent codex [--executable FILE] [--approval-policy POLICY] [--sandbox SANDBOX] COMMON...
      COMMON: [--model M] [--effort E] [--max-acp-version 1|2] [--max-sessions N] [--working-root DIR]...
              [--no-client-mcp-servers] [--allow-network-paths] [--cache-lifetime default|short|long]
canoa launch PROGRAM [ARGS]...
canoa options (claude | codex [--executable FILE] | NAME --entry [--config FILE]) [--format text|json]
canoa accounts list [--config FILE] [--format text|json]
canoa accounts add NAME --harness ID --method METHOD [--input KEY=VALUE]... [--until-stdin-closes] [--config FILE] [--format text|json]
canoa accounts login NAME [--method METHOD] [--input KEY=VALUE]... [--until-stdin-closes] [--config FILE] [--format text|json]
canoa accounts remove NAME [--config FILE]
```

- `agent claude|codex` serves the installed Claude Code or Codex as an ACP agent (version 1 or 2, whichever the client asks, up to `--max-acp-version`) on standard input and output, through coqui-acp's translators, with no Node; notices and exits go to standard error as `canoa agent NAME: ...`; a version 2 message's end is written without `content: []`, as for a harness entry. `--executable` defaults to `COQUI_ACP_CLAUDE_EXECUTABLE` or `COQUI_ACP_CODEX_EXECUTABLE`, else `claude` or `codex`. Plugins from a client are off unless `--allow-client-plugins`. A `--working-root` holds a session's `cwd`, more directories and plugins, and a resumed Codex thread's own directory. It takes the process's standard streams, so `canoa-ffi` runs it only through `--canoa-child`, as `provider`, `tool`, `launch` and `mcp-bridge`. The command line reads its own variables (`CLAUDE_CONFIG_DIR`, `CLAUDE_CODE_OAUTH_TOKEN`, `CODEX_HOME`, ...) from the environment it runs in. Exit 0 when the client closes, 1 when reading it failed, 2 for a wrong invocation.
- `options` prints what a source offers before a session starts, asked of its `acp_harness` provider's `models` with no session and no model turn (`canoa_host::options`). `claude` and `codex` run as `canoa agent` runs them (`--executable`, else `COQUI_ACP_CLAUDE_EXECUTABLE` or `COQUI_ACP_CODEX_EXECUTABLE`, else the harness's name); with `--entry`, `NAME` is a `[host.agents]` entry of canoa.cfg (`--config`): a `harness` entry is asked through its own provider and settings, any other entry (a program on standard streams, a provider, a tool) answers `known: false` with the reason. `--format json` prints one object: `{"known", "unknown_because"?, "program"?, "program_version"?, "cache_lifetime"?, "cache_lifetime_setting"?, "models": [{"id", "name"?, "description"?, "resolved"?, "version"?, "efforts": [..], "default_effort"?, "is_default"?}]}`, `program_version` being the first line of the program's `--version` when it answers within 10 s; `text` prints one `ID<TAB>NAME<TAB>EFFORTS` line per model. Claude Code lists `default`, `opus`, `fable`, `sonnet` and `haiku` with the model each resolves to and the efforts each takes; Codex lists its models with each one's own efforts. It uses the machine's own login and takes the process's usual streams, so `canoa-ffi` runs it as an in-process job; the app caches the answer, the host does not. Exit 2 for a source that is neither built in nor an entry.
- `accounts` (see "Accounts"): `add` signs in by one of the provider's login methods (`--method` naming none or an unknown one lists them, status 2) and appends `[host.accounts.NAME]` to canoa.cfg, made if absent; `login` signs in again to an account (`--method` only when its kind has several); `remove` forgets the stored credential and the account's directory and takes the block `add` wrote out of canoa.cfg (an account written some other way is for its writer to remove; an entry that offers it keeps it, status 2); `list` prints `NAME HARNESS KIND STATE` or `{"name", "harness", "kind", "id", "state", "revision", "expires_at"}`, `state` being `signed in`, `needs sign-in`, `in use` or `signed in where it runs` (harness-login), never a credential. A page to open or a code to confirm is said on standard error (`Open this page: URL`, `Confirm this code there: CODE`), or with `--format json` printed as `{"say": TEXT}` and `{"open_url": {"url", "user_code", "expires_ms"}}`, then `{"account": {"name", "harness", "kind", "id", "label", "checked"}}` when `add` is done. A secret the method asks for is typed without echo at a terminal, or one line of standard input when that is not a terminal; it is never read from the command line. `--until-stdin-closes` cancels the sign-in when the front's input closes (the process-level `canoa` ends at once instead, so do not use it with a piped secret).
- `launch` is what `serve` runs for an agent entry with profiles: the environment on the first line of standard input, the rest to `PROGRAM`, whose status it exits with; a first line of another form is status 2.

- `--method` names the proof: this user's identity of that method, made when absent, and the home then records that method in `method`. Without it, the method the home records (the one named last, or its first identity's), or a new identity of the default method, Noise. An identity never changes method. A client of the other method is logged as `a client was refused: … the peer proves by X, and this endpoint only by Y`, with no word of `accept`: its key has to be paired again under this host's method. `--key-limit` and `--no-rekey` set the session, each method's default otherwise, and a non-default one is logged at `info`.
- **A running host keeps its home's method.** `id --method X` and `pair --method X`, with another method than the one the home records while a host holds the home (`Home::probe`, which takes nothing and writes nothing), are refused with status 3: `NAME (process PID) is running on this home, which proves by Y: already-paired clients would be refused after its next restart if the method became X. Stop it first, or add --force to change it anyway`. `--force` changes it and logs at `info` `--force: method is X while NAME (process PID) runs on this home, which proves by Y: …`; it is the person's own flag, and what it risks is a lockout of paired clients, not a looser proof. The method the home already records, no `--method`, and a home no host holds are never refused. `serve --method` is not guarded: it holds the home itself.
- `serve` prints one JSON object per entry on standard output, `{"name", "kind", "url", "harness"?, "title"?, "peer", "method"}`, `harness` being a harness entry's provider id (absent for any other entry) and `title` the entry's title (absent when it has none), `peer` this host's id and `method` its proof's, once every entry is bound, and then `{"ready": true, "peer", "method"}`, a line with no `name`; then it serves until stopped. It logs on standard error.
- **One host on a home.** `serve` takes the home's lock (see "One host on a home") before anything else of the home, or any address: a second one, the app's or another `canoa serve`, says `canoa: NAME (process PID) already hosts this home, DIR: ...`, binds nothing and ends with status 3. A holder that died holds nothing. `check`, `peers`, `accept`, `revoke`, `id` and `pair` take no lock, and `coqui-acp-bridge serve --paired` does not know of it.
- `pair` takes its PIN back however it ends, so a connection still pairing on its own thread when it is given up pairs nobody: in an application's process such a thread outlives the command.
- **The host's own sources** (`serve --local-sources`, `ServeOptions::with_local_sources`): `Claude` and `Codex` (built-in harness entries with Bypass offered), `Antigravity` (`agy acp`, usage read, also found in `%LOCALAPPDATA%/agy/bin`), `Grok` (`grok agent stdio`, also in `~/.grok/bin`) and `Pi` (a built-in `pi` harness entry: coqui-acp-pi's provider drives `pi --mode rpc` in this process, with no Node and no adapter; it is ready exactly when the Store's Pi extension finds `pi`, that is a managed copy, `PATH`, `~/.pi/agent/bin` or `~/.local/bin`; `missing` is `["pi"]` and nothing else). A managed copy (`current.json` the host wrote, every file as installed, its Node and script inside the copy) runs as the provider's `node` and `script` settings, never a `.cmd` shim; otherwise the `pi` found is its `executable`. `[permissions] protected`, when canoa.cfg has it, is the provider's `protected` setting, so the person's list replaces the provider's recommended one for this source (it is the one list the person already edits for what a session may not touch); absent, the provider's recommended list stands. The launch's `PI_SKIP_VERSION_CHECK` is not passed on yet: no environment reaches a source's provider but a profile's. Each is an entry with `Entry::local` set: no listener, no `/canoa.json` line, reachable through `Local::open` only. `serve` prints a line for each, `{"name", "kind": "agent", "local": true, "missing": [...], "harness"?, "title", "peer", "method"}` with no `url`; `missing` names the programs not found (on Windows only `.exe`, `.cmd`, `.bat` and `.com` count, so `npx` is `npx.cmd`), and the entry is served either way so a client can say what to install. A name canoa.cfg already uses is left to canoa.cfg.
- **A source of the host's own is launched with canoa.cfg's top level** (the host owns a session's profile), through the machinery of `[host.profiles]` ("An agent entry with `profiles`"), with one profile named `default`: `Config::local_profiles` holds `[profile]` (the person's layer), the names in `[overlays]` (read from `overlays/NAME` beside canoa.cfg, as for `[host.profiles]`, so a source that is a directory is not applied here), and for each agent `[agents.NAME.profile]` (a layer above the person's, `agent:NAME`), `[agents.NAME.hooks]` (an overlay's name and the tree digest its hooks run at: `allow_hooks`) and `[agents.NAME.mcp]`: `true` leaves a server of the person's on, `false` turns any server off, a `sha256:` digest allows an overlay's server at that definition (`allow_mcp`). `LocalProfiles::for_agent` is `None` when nothing is for that agent, which starts it bare. Only entries with `Entry::local` get this; an entry on an address offers only its `[host.profiles]`. An item for `model`, `effort` and `mode` carries `"value"`, since the host cannot set them and the session's options do. A project's profile (`.canoa/profile.toml`) is not read.
- **Approvals** (`profile::APPROVE_METHOD`, `_canoa/profile/approve`): a client that is this process's own link (`Local::open`, never a paired key) and whose `initialize` carries `_meta["dev.boriken.canoa/approve"] = true` (`profile::APPROVE_META`) is asked, before each `session/new`, `session/load` or `session/resume` is forwarded, which MCP servers of an overlay that the configuration does not allow this session may start: the request `{"profile", "harness", "servers": [{"name", "layer", "digest"}]}` with an unguessable text id (`canoa-approve-N-<24 hex>`), answered `{"allow": [names]}` (names it did not list are ignored). Those names are added to that session's `mcpServers` and shown `applied` in its report; nothing else changes, and nothing is kept: keeping a choice is `[agents.NAME.mcp]` in canoa.cfg. A paired key, however it opts in, and a link that does not opt in, answers with an error, takes over two minutes, or goes away gets the configuration's behaviour (off). Its other messages wait until the answer. A server the person turned off with `false` is never asked about. The scope removes a client's `_meta`, so the opt-in is read before it. An agent's own request or notification whose method starts with `_canoa/` or `_coqui_acp/secret_` is dropped before the client sees it (`scope::is_host_only`), on every entry.
- `serve --allow-empty`: a canoa.cfg with no entry, or none at all at that path, runs with nothing to serve, printing only the `ready` line, where without it that is status 2. It writes no file. A canoa.cfg that exists and cannot be read or parsed is still status 2.
- `serve --web [DIR]` serves the web app too (see "The web app"): from `DIR` (made absolute from the working directory), else `[host.web]`'s `dir`, else the site built in; on `[host.web]`'s settings, or the defaults without the table. `--web-listen` replaces `listen`, and needs `--web` or `[host.web]` (status 2). After the entries' lines it prints `{"web": URL}`, a line with no `name`. A site that cannot be served, no site, plain HTTP beyond loopback, a zero limit or an unusable certificate is status 2; an address that cannot be bound, status 1.
- **A link from inside** (`Local::open`) is served as a paired key's connection is: the entry's agent, scope, worktrees (`_canoa/worktree/*` before the first ACP message) and `max_peers`. It skips the proof, the trust store and the entry's `peers` list, since whoever holds a `Local` is this process; no network peer can obtain one. `Stopper::stop` closes every link. The peer it is logged as is the host's own id.
- `serve --until-stdin-closes`: once every entry is bound, standard input is read and discarded; when it closes, every listener and agent is ended (`Stopper::stop`) and the host exits 0. For a parent that runs the host as its child: the host ends with it, however the parent ends. Without the flag the input is never read.
- `pair` (`--listen` default `127.0.0.1:0`, another address logged at `info` as `non-default: pair --listen is ADDRESS, default loopback`; `--timeout-seconds` default 300) prints `{"pin", "peer", "method", "url"}`, a six-digit PIN, this host's id, its method and the address to dial, and pairs by that method; a native client joins with `coqui_agentic_peer::pairing::join` (`coqui-acp-bridge pair-dial URL`). The joiner's key is accepted in the store once the PIN confirms both keys, and `pair` prints `{"paired": ID}` and exits 0. One wrong try ends it (status 1), as does the timeout, and with `--until-stdin-closes` its input closing. A pairing connection serves no agent. A WebSocket upgrade that carries an `Origin` header, which every browser sends, is refused (403) before the handshake, so a web page cannot spend the PIN's one try; a browser pairs by connecting to `serve` once and being accepted.
- On `serve --until-stdin-closes` and `pair --until-stdin-closes`, nothing is logged between the input closing and the exit, and the process exits within 5 s whatever is left: a parent that died can leave standard error a pipe a write to which never returns (seen on Windows under a Dart parent).
- `accept` and `revoke` take `--` before the id.
- `tool` serves one tool entry on standard input and output, `JSON` being the entry as `ToolSpec` serializes it (without `listen`, `working_roots`, `peers`, `max_peers`): what `serve` runs per connection. A `JSON` that is not one, or that `check` refuses, is status 2.
- `check` prints `NAME KIND LISTEN` per entry, or `{"name", "kind", "listen"}` with `--format json`, and serves nothing.
- `peers` prints `accepted ID` and `waiting ID [from ADDRESS]`, or `{"peer", "state", "source"}`.
- canoa.cfg is `--config`, else `CANOA_CONFIG`, else `%APPDATA%\Canoa\canoa.cfg` on Windows, `~/Library/Application Support/Canoa/canoa.cfg` on macOS, `$XDG_CONFIG_HOME/Canoa/canoa.cfg` (else `~/.config/…`) elsewhere: where the app keeps it.
- `provider` takes `--library` and `--source` from where it was started, then runs in `--directory`; with `--settings-on-stdin` it reads a `HostLine` first, as `serve` writes it; with `--connect` it serves on that loopback socket after sending the line's token, rather than on standard input and output.
- Exit status: 0 success; 1 the operation failed; 2 the invocation or canoa.cfg was wrong, `serve` with nothing to serve, or a text that is not a peer id; 3 another host holds this home.

### A webpage's session folder

A client that is a webpage has no file system and names no path. Its `session/new` carries `_meta["dev.boriken.canoa/site"] = {"site": "<host name>"}`; the host ignores any `cwd` it sent and gives the agent `<Documents>/Canoa/scratch/<site>/<new UUID>` instead, made (`create_dir_all`, mode 0700 on Unix) only after the scope has passed the request, so a request that is refused makes nothing. The answer to `session/new` carries `_meta["dev.boriken.canoa/site"] = {"site", "id", "folder"}` (the host removes the key from any other answer, an agent's own included). The site is taken only as written, in the form a browser gives it: lower-case ASCII labels (an international name as `xn--` punycode), no trailing dot, port, separator, colon or IPv6, a label of 1 to 63 letters, digits and inner hyphens, 253 characters at most, a first label that is not a Windows device name (`con`, `prn`, `aux`, `nul`, `com1`..`com9`, `lpt1`..`lpt9`), and a name made of numbers only an IPv4 address; anything else fails the request with `-32602`. `session/load` and `session/resume` with the same key must carry the recorded folder as `cwd`: it is taken only when it is `<Documents>/Canoa/scratch/<site>/<id>` with `<id>` one segment of letters, digits and hyphens, and is made again when gone; anything else is `-32602`. An entry with `working_roots` refuses a folder outside them. The Documents folder is `XDG_DOCUMENTS_DIR`, else the system's; a host with none fails the request. The host makes no other folder for a client.

## Environment variables

| Variable | Effect | Default |
| --- | --- | --- |
| `CANOA_CONFIG` | canoa.cfg to read, below `--config` | the platform's place, above |
| `CANOA_LOG` | what to log, a `tracing` filter | `info` |
| `CANOA_WEB_DIST` | at build time, with feature `web`: the absolute directory of the site built in | none; the build stops without it |
| `COQUI_AGENTIC_PEER_HOME` | the directory of the identities (`identity` for WebCrypto, `identity-noise` for Noise) and trust store (`peers/`) | `coqui-agentic-peer` under `%LOCALAPPDATA%`, `$XDG_CONFIG_HOME` or `~/.config` |
| `XDG_DOCUMENTS_DIR` | the Documents folder a webpage session's scratch folder is made in (absolute, or from `$HOME`), on any system | the system's: the Documents known folder on Windows, `user-dirs.dirs` then `~/Documents` on Linux, `~/Documents` on macOS |
| `NO_COLOR` | no colour in the log | colour on a terminal |

## Gotchas

- Under Noise a client's handshake ends before the host decides, so a client the host refuses sees its link end rather than a failed proof.
- The identity and trust store are the ones `coqui-acp-bridge serve --paired` uses: a key paired for one is paired for the other, and `canoa revoke` removes it from both.
- A stdio agent inherits the host's environment and working directory. An agent can still start whatever its own tools start when the client allows them; the host stops only what a client could start without the agent asking.
- A profile's variable and MCP server values never reach the client from the host, but they reach the agent, which acts for that client and may show them: give a profile only what that entry's clients may use.
- A paired key reaches every session an agent keeps for this user: `session/list` and `session/load` pass to a stdio agent unchanged.
- `program = "canoa"`, `args = ["agent", "claude"]` with a profile that has skills needs `--allow-client-plugins` in `args`: without it Claude Code's translator refuses every session the host adds its plugin to. A `harness` entry needs nothing.
- On Windows the translators start `claude.exe` and `codex.exe`, as the native installers put them; an npm-installed `claude.cmd` or `codex.cmd` is untested, and a profile's context goes on the command line it is started with. Name the executable by its full path if it is elsewhere.
- `working_roots` compare paths as written: a link inside a root that points outside it is followed by the agent. Worktrees are made, read and removed only where every link is resolved, inside the roots resolved too.
- If the host itself is killed, on Windows the system ends its agents with it (each is in a job object closed with the host); on Unix each sees its standard input close, and most then exit. `serve --until-stdin-closes` ends them on every platform when the parent goes. A host inside an application has no parent to watch: on Unix, an agent that ignores its input closing outlives an application that crashes.
- Each connection is a fresh process, so a turn in progress ends with its link; a later connection resumes a session only if the agent kept it.
- `peers` is read when the host starts; a key removed from the trust store loses its link within about a second without a restart.
- A provider's process inherits the host's environment. For a Python source that includes, on Windows, `PATH`, which decides which `python3.dll` loads under `import_search = "path"` unless `import_dirs` holds one; and with `isolation = "inherit"`, `PYTHONPATH` and `PYTHONHOME`.
- Settings come from canoa.cfg only. A client reaches a provider through ACP, never through its settings; `check` does not load the provider, so a setting its schema refuses shows only when a connection loads it, on the host's standard error.
- A tool runs with the host's rights and does unasked whatever its rules do not turn into `ask`: the host shows its tool calls as they run, never before. `unasked` is the entry writer's account of that, shown, not checked.
- On Unix, ending one tool process (its session's cancel, a replaced `message` run) does not end what it started; that stays in the connection's process group and ends with the connection, unless it left the group.
- On Windows `program = "pi"` does not find npm's `pi.cmd`: name `node` with the script's absolute path, which keeps `cmd.exe` out, or `pi.cmd`.
- A tool entry is on `canoa tool`'s command line as JSON, which other users may read: nothing secret belongs in one.
- On Unix, what a tool starts in a session of its own (Node's `detached`) leaves the connection's process group and outlives it.
- Plain WebSocket carries only the proof and sealed records. Anyone who can reach the address can connect and make a proven key wait; only an accepted key gets further. On loopback, the default, that is anyone on this machine.
- The web app's page connects to the entries' addresses, which are loopback: a browser on another machine, opening it over HTTPS beyond loopback, reaches the page but not the entries until a proxy carries them. A non-empty `origins` must list the page's own origin for its connections to be let in.
- The site's files are served with this user's rights from what the directory holds when asked: whoever can write it chooses the page, and a link swapped in between the check and the open is followed (the check resolves the path, then opens it). Give it a directory only you can write. `serve` refuses a site holding canoa.cfg's directory or the identity home.
- The page's key lives in the browser under the page's origin, `http://127.0.0.1:7800` by default. While the host is not running, any program of any user on this machine that binds that port serves its own page in that origin, which can use the key paired with this host. On a machine others use, serve over HTTPS with a certificate the browser trusts and a key only you can read, or use the extension.
- The policy allows `'wasm-unsafe-eval'` and inline styles, which Flutter's web build needs, and no inline or evaluated script: build the app with `flutter build web --csp --no-web-resources-cdn`, or its code and CanvasKit are refused. The app carries its Roboto font (`app/assets/fonts`) and points the engine's fallback fonts at the site (`app/web/flutter_bootstrap.js`), so that build fetches nothing from outside the site.
