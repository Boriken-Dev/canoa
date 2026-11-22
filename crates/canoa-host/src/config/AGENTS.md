# `canoa-host` configuration and shells

## canoa.cfg (`canoa_host::config`)

canoa.cfg is one file in TOML, YAML (`#!yaml` first line) or JSON (`#!json`), read through anycfg (revision `bf71f70`) into `toml::Value` by `Config::load`, `Config::load_layers(&Layers)` and `Config::from_str_in`; the examples below are TOML. The Cargo features `yaml` and `json`, on by default, register the other two; without them `#!yaml` or `#!json` is refused naming the formats read. A null in YAML or JSON leaves its key unset; a TOML date or time is refused. `ConfigError::Parse` has `file`, `line` and `column` where known, and for a setting of the wrong shape the key's path (`host.agents.a.max_peers: expected usize, found a string`), never a value.

The host reads `[host]`, `[usage] limit_phrases` (`Config::limit_phrases`: `None` when unset, which means coqui-acp-api's `DEFAULT_LIMIT_PHRASES`; a list replaces them, `[]` infers nothing, an empty or blank phrase is refused) and `[permissions] protected` (`Config::protected_paths`: a string with a pattern a line, or a list of strings; `None` when unset; it is handed to the built-in Pi source and to nothing else) and `[settings.power] keep_awake` (`Config::keep_awake`, default `false`: the Power group of General, a `settings_section` the host offers itself; while it is on, the host holds the operating system's idle-sleep inhibition for as long as at least one `session/prompt` turn is in flight, `awake.rs`: Windows' `SetThreadExecutionState` on a thread of its own, macOS's `caffeinate -w` on the host's pid, Linux's `systemd-inhibit` running `cat` on a pipe the host holds, so the hold ends with the last turn, a closed connection or a host that dies; logged at `info` when on); the app's tables (`desktop`, `network`, `assets`, `appearance`, `agents`, `overlays`, `profile`, and `[usage] warn_permille`) are not read. Every table the host reads refuses a key it does not know.

```toml
[host]
origins = ["https://canoa.example"]  # optional: browser origins that may connect; empty or absent lets any, and the proof decides
shell = "git-bash"                    # optional: the shell a terminal starts and each harness entry is told to use: an id `shell/list` gives (see Shells) or a program, an absolute path or a name found on PATH, started with no arguments; absent, $SHELL, else pwsh or powershell on Windows, else /bin/sh

[host.agents.NAME]                    # a program speaking ACP on its standard streams
program = "gemini"                    # this or harness, one of the two; found as the OS finds a command
args = ["--acp"]
profiles = ["review", "plain"]        # optional: [host.profiles] a client may choose among, the first by default; an entry that names none gets the default profile (canoa.cfg's top level), and one that should run bare names an empty profile
usage = "antigravity"                 # optional, with program: read the agent's quota too, by the `usage_reader` a bundled agent offers (`antigravity`, for agy acp); see Usage
title = "Claude for review"             # optional: what a client shows for this entry instead of its name; 1 to 64 characters, not blank, no control characters (MAX_TITLE)
shared = false                        # optional: a session outlives its connection and any key the entry admits attaches to it; see Shared sessions
shared_idle_seconds = 86400           # optional, with shared: how long a session nobody is attached to is kept; 0 keeps it until a client ends it
shared_permission_window_seconds = 0  # optional, with shared: log a permission request nobody has answered for this long (it stays open; nothing is answered for anyone); 0 or unset logs nothing
shared_keep_log = true                # optional, with shared: keep each session's log of updates in the home so it survives the host's restart (see Shared sessions); false keeps nothing
shared_log_entries = 100000           # optional, with shared: updates kept for replay, the oldest going first
shared_log_bytes = 67108864           # optional, with shared: serialized bytes of updates kept for replay

[host.agents.OTHER]                   # an acp_harness provider chosen by id, run by the host itself; see Serving
harness = "claude"                    # the id of a bundled agent's harness (`providers/list` names them) or of an outside provider that library or source loads; takes no args
executable = "/opt/claude/bin/claude" # claude and codex: a name found as the OS finds a command, or a path from the file's directory
setting_sources = ["project"]         # claude only: of user, project, local; [] loads none; default Claude Code's own
# the access (sandbox, network, approvals) is the profile's, not a key here: `profiles = [...]` with `[host.profiles.NAME] access = { ... }`; every mode the agent has is offered, Bypass included
shell = "canoa-pwsh"                  # claude, codex and pi (a built-in harness): this entry's shell, in place of [host] shell; see Shells
cache_lifetime = "short"              # optional, with harness: how long the provider's prompt cache lives, one of CACHE_LIFETIMES ("default", "off", "short", "long"; off only where the provider lists it, else it is the default); the entry's default for a session whose client chooses none; absent leaves the client's (the chat asks "long")
account_settings = { renew_poll_ms = 30000 }  # optional, with harness: the configuration of the account provider the same library offers as `<harness>_accounts` (`claude_accounts`, `codex_accounts`, `pi_accounts`, `opencode_accounts`, `grok_accounts`, `antigravity_accounts`, an outside library's `stdio_accounts`), which signs in, renews, watches and reads plan limits; its `executable` (and `harness`, for `stdio_accounts`) default to the entry's. `renew_poll_ms` (30000; 0 looks only when a watch begins and ends) and `command_timeout_ms` (60000) are its settings, no longer the harness's. `antigravity_accounts` and Antigravity's usage reader share theirs (`canoa_antigravity::AgySettings`): those two, `login_args` (`["acp", "--login"]`), `login_poll_ms` (1000), `login_hosts` (`["accounts.google.com"]`), and where its stored login is read: `home`, `gemini_dir`, `keyring` (true), `keyring_timeout_ms` (5000), `client_program`, `token_url`, `userinfo_url`, `service`, `consumer_service`, `gcp_service` (Google's own addresses), `user_agent`, `timeout_ms` (15000), `max_answer_bytes` (4194304)
accounts = ["work", "personal", "keys"]   # optional, with harness: the [host.accounts] a client may run a session on, and the [host.pools] it offers (a pool takes the place of the accounts it holds, and an `any-<standard>` pool the host makes is named without being in the file), the first by default; absent offers the automatic `local` account (the harness on the login it has where it runs, when its provider has one); `local` may be named among them to place it

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

[host.pools.keys]                     # a name for accounts that provide one standard, which an entry offers and a session may run on (written by the app's Accounts section or `pools/set`)
standard = "anthropic-api"            # any standard id: lower-case letters, digits and single hyphens; each account has to provide it
accounts = ["work", "personal"]       # [host.accounts] names, or the automatic `local`; the order is the order they are taken in

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

[host.extensions.NAME]                # a Guatiao store_extension: adds types to the Store; see "Store extensions" below
library = "..."                       # or source = "..." (with source_settings, import_search, as a provider)
provider = "..."                      # the store_extension provider id
pin = "sha256:..."                    # the Store wrote these when it installed the extension; each is checked again every time the host loads it
root = "..."                          # with pin: the folder the tree digest covers, which holds the library
sha256 = "..."                        # or, instead of pin and root, the library file's SHA-256 (with library)
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
listen = "127.0.0.1:7801"             # default 127.0.0.1:0, any free port; another address is configuration, logged; or "pipe:NAME" (Windows) or "unix:PATH" (Unix) for a native client on this machine, see "A pipe or Unix socket"; with `transport`, the endpoint that provider listens on, required
transport = "websocket"               # optional: carry this entry's connections through a `transport` provider by id: "pipe" and "websocket" are built in, any other id needs transport_library or transport_source; see "A transport provider"
transport_library = "t.dll"           # with transport: an outside provider's library, or transport_source; exactly one, paths from the file's directory; never with a built-in id
transport_source = "guatiao_source_py.dll"   # with transport: as an entry's `source`, with transport_source_settings and transport_import_search as source_settings and import_search
transport_settings = { listen = { frames = "binary" } }   # with transport: the provider's own configuration, checked against its schema when the host starts
working_roots = ["/home/me/src"]      # absolute, no . or ..; a session's cwd must be under one; default any
                                      # a Windows share (\\server\share\src) is a root and a cwd like any path; \\?\UNC\ is compared and forwarded plain; a device path is refused
worktrees = true                      # agents and tools: a git worktree per session inside working_roots; see "Worktrees"
peers = ["sha256:…"]                  # paired keys that may use it; default every paired key
max_peers = 4                         # connections at once

[host]
max_message_bytes = 16777216          # the longest ACP message, one line, either way on a connection (a prompt with an attachment is one); no ceiling here, 0 is refused; `Config::max_message_bytes`, default `DEFAULT_MAX_MESSAGE_BYTES`. A message over it is refused naming the setting (`LinkError::TooLong` says it; a network client sees its connection end and the host's log says it). The host's link to another machine's agent uses it too; `canoa agent` takes `--max-message-bytes`, and `canoa launch`, `mcp-bridge` and a tool's own lines keep the default

[host.mcp_list]                     # optional: what listing a session's MCP servers (`_canoa/mcp/list`) is held to; each at least 1, no ceiling here
query_seconds = 5                     # how long one server may take to answer all its lists
max_items_per_server = 1000           # the tools, and the resources, read from one server
max_reply_bytes = 4194304             # the longest answer read from one server: an HTTP body or a line of its output

[host.web]                            # optional: the web app on a listener of its own; see "The web app" below
dir = "site"                          # the site, relative to the file's directory; without it the site built in (feature web)
listen = "127.0.0.1:7800"             # the default; beyond loopback only with cert and key
cert = "web.crt"                      # PEM: the certificate, then any intermediates; with key, HTTPS
key = "web.key"                       # PEM: its private key
hosts = ["canoa.example"]             # Host names a request may carry besides localhost and an IP address
mode = "local"                        # "local" or "remote": where the person sits, told to the page in /canoa.json as `web`; absent, local for a page reached as localhost or a loopback address and remote for any other name
connect = ["https://fonts.gstatic.com"]   # origins the page may connect to besides the entries; each logged as configuration
office = false                        # serve the Office add-in's pages under /office/ with a policy of their own (see "The Office add-in's routes"); off by default, logged as configuration when on
office_local = false                  # with office and a loopback listen: the add-in runs in Office on this computer, so the page's own key is accepted when it asks (see "The Office add-in's routes"); off by default, logged as configuration when on
office_secret = false                 # with office_local: the page's requests (admission, tools, starting a host) carry the secret in <home>/office/secret, so another account on a shared computer cannot make them; off by default, logged as configuration when on
office_frames = ["https://*.officeapps.live.com"]   # with office: origins besides the built-in ones that may frame the add-in's pages; https, a host that may start with *. and holds a dot more, an optional port; each logged as configuration
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

#[non_exhaustive] pub struct Config { pub origins: Vec<String>, pub entries: Vec<Entry>, pub profiles: IndexMap<String, HostProfile>, pub web: Option<WebConfig>, pub limit_phrases: Option<Vec<String>>, pub protected_paths: Option<Vec<String>>, pub accounts: IndexMap<String, AccountConfig>, pub pools: IndexMap<String, PoolConfig>, pub secrets: Option<serde_json::Value>, pub shell: Option<String>, pub store: StoreConfig, pub skill_pins: BTreeMap<String, String> }   // Default, Clone, Eq
#[non_exhaustive] pub struct WebConfig { pub dir: Option<PathBuf>, pub listen: SocketAddr, pub tls: Option<WebTls>, pub hosts: Vec<String>, pub connect: Vec<String>, pub limits: web::Limits, pub keep_out: Vec<PathBuf>, pub mode: Option<WebMode>, pub office: bool, pub office_frames: Vec<String>, pub office_local: bool, pub office_secret: bool }   // Default, Clone, Eq
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
#[non_exhaustive] pub struct HostProfile { pub profile: Profile, pub overlays: Vec<String>, pub allow_mcp: BTreeMap<String, String>, pub allow_hooks: BTreeMap<String, String>, pub directory: PathBuf, pub skill_pins: BTreeMap<String, String>, pub options: BTreeMap<String, Document>, pub protected: Option<Vec<String>> }   // protected: `[permissions] protected`, which a hook's allow never answers for; options: `[overlays.NAME] options`, by overlay, for the host's own profile and every `[host.profiles]` one
#[non_exhaustive] pub struct Entry { pub name: String, pub kind: EntryKind, pub listen: Listen, pub working_roots: Vec<PathBuf>, pub worktrees: bool, pub peers: Option<Vec<PeerId>>, pub max_peers: usize, pub title: Option<String>, pub local: bool }   // local: served over Local only, no address; only the host's own sources are
#[non_exhaustive] pub enum EntryKind { Agent { program: OsString, args: Vec<OsString>, profiles: Vec<String>, usage: Option<String> /* the id of a bundled agent's usage_reader */ }, Harness { harness: String, origin: Option<Origin>, executable: Option<OsString>, options: HarnessOptions, settings: Option<Settings>, account_settings: Option<Settings>, profiles: Vec<String>, accounts: Vec<String> }, Provider { origin: Origin, provider: String, settings: Option<Settings>, directory: PathBuf, max_sessions: usize }, Tool { spec: ToolSpec } }
#[non_exhaustive] pub struct AccountConfig { pub harness: String, pub kind: String, pub id: String, pub settings: Option<Settings> }   // Clone, Eq: one [host.accounts.NAME]
#[non_exhaustive] pub struct PoolConfig { pub standard: String, pub accounts: Vec<String> }   // Clone, Eq: one [host.pools.NAME]; `Config::pools` is by name in the file's order
#[non_exhaustive] pub struct KbConfig { pub tools: Option<bool>, pub embeddings: Option<String>, pub model: Option<String>, pub api_key_env: Option<String>, pub document_prefix: Option<String>, pub query_prefix: Option<String>, pub chunk_bytes: Option<usize>, pub overlap_bytes: Option<usize>, pub llm_endpoint: Option<String>, pub llm_model: Option<String>, pub llm_api_key_env: Option<String>, pub llm_candidates: Option<usize>, pub llm_timeout_secs: Option<u64> }   // Default, Clone, Eq: `[host.kb]`, as `Config::kb`
#[non_exhaustive] pub struct StoreConfig { pub protected: Option<Vec<String>>, pub credential_variables: Option<Vec<String>>, pub skill_folders: Option<Vec<String>>, pub index_ttl: Option<u64>, pub index_cache_bytes: Option<u64>, pub tools: Option<bool> }   // Default, Clone, Eq: `[host.store]`; None is the recommended list, or for the index and the tools their default
#[non_exhaustive] pub struct NetworkConfig { pub proxy: ProxyChoice, pub proxy_user: Option<String>, pub proxy_password_env: Option<String>, pub no_proxy: Option<Vec<String>>, pub negotiate: Option<bool>, pub client_certificate: ClientCertificate, pub ca_file: Option<PathBuf>, pub git_tls: GitTls, pub endpoints: Vec<String>, pub proxy_idle_seconds: Option<u64> }   // Default, Clone, Eq: `[host.network]`; `settings()` reads the password from its variable (`canoa::network` has the types)
pub const CACHE_META: &str = "dev.boriken.canoa/cache_lifetime";   // the `_meta` key of `initialize`: {"lifetime": "default"|"off"|"short"|"long", "chosen"?: bool}; `chosen` (true when absent) says the person chose it for this session and it beats the entry's `cache_lifetime`; false is the client's default for its kind of session, which the entry's setting replaces; no key leaves the entry's, else the provider's default; any other value turns the client away (-32602)
pub const SITE_META: &str = "dev.boriken.canoa/site";   // the `_meta` key of a webpage's `session/new`, `session/load` and `session/resume`, in the request ({"site": "<host name>"}) and in the answer to `session/new` ({"site", "id", "folder"}); see "A webpage's session folder"
pub const MAX_MESSAGE_META: &str = "dev.boriken.canoa/maxMessageBytes";   // the `_meta` key of the answer to `initialize` on every connection of a paired key or the app: `[host] max_message_bytes`, a number, so a client that frames its own messages opens its limit to the host's
pub const ACCOUNT_META: &str = "dev.boriken.canoa/account";   // the `_meta` key of `initialize`, in the request (the account's name) and in the answer ({"name", "kind"})
pub const MODEL_META: &str = "dev.boriken.canoa/model";   // the `_meta` key of `initialize` naming the model the session starts on, by the id the account lists (text); a model that account does not list refuses the start with `-32602`, naming the account and the model, and a provider that cannot list before a session leaves the model to the session's own options
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
- An agent names `program` (with optional `args`) or `harness` (a provider id, not empty), one of the two; a `harness` other than `claude`, `codex`, `pi` or `opencode` with no `library` or `source`, `args` with `harness`, and `executable`, `setting_sources`, `cache_lifetime`, `library`, `source`, `source_settings`, `import_search` or `settings` with `program` are refused, and so is another harness's key or a value not in its list. `executable` and `setting_sources` are the `claude`, `codex` and `pi` providers' settings (`setting_sources` is Claude Code's alone; `pi` has `settings` for the rest: `node` with `script`, `protected`, `isolated`; `opencode` takes `executable` and `settings` for the stdio provider's limits, `max_requests`, `max_message_bytes` and the `replay_*` ones) and are refused for any other id, which takes `settings`; one the entry's `settings` also holds is refused as set twice. An entry has no key for the access (`permission_mode`, `approval_policy`, `sandbox`, `bypass`, `modes`), and a `settings` table that names `access`, `permission_mode`, `approval_policy` or `sandbox` is refused: the access is the profile's.
  `settings = { modes = [...] }` and `{ allow_bypass = ... }` stay the person's own and are logged at `info` when the host starts. The `library` or `source`, `source_settings` and `import_search` keys read as a provider entry's. An `executable` holding `/` or `\` is made absolute against the file's directory, since the command line runs in a session's directory, which a client chooses. A harness entry is an `agent` in `check`'s and `serve`'s lines.

```rust
#[non_exhaustive] pub struct HarnessOptions { pub setting_sources: Option<Vec<String>>, pub cache_lifetime: Option<CacheLifetime> }   // Default, Eq
pub fn built_in_harnesses() -> impl Iterator<Item = &'static str>;  // the acp_harness providers the bundled agents' libraries register (`bundled.rs`)
pub fn is_built_in_harness(id: &str) -> bool;
pub const SETTING_SOURCES: &[&str];    // user, project, local
```
- `[[host.openers]]` (`Config::openers`, `OpenerConfig`): the editors and file managers `workspace/openers` offers and `workspace/open` starts. Each has `id` (1 to 64 lower-case letters, digits, `-` or `_`, each once), `label` (the text of the Open in menu), `program` (a name on the `PATH` or a path), `args` (default `["{path}"]`; at least one holds `{path}` or `{parent}`, and none holds a NUL, CR or LF) and `item` (`true` for items only, `false` for folders only, absent for either). Absent, the host seeds VS Code (when found), the system's file manager and Reveal; a table of its own replaces that list whole, so a person who wants none of them lists only theirs. Any other key, or a value outside these, is refused naming `host.openers`.
- `[host.kb]`: `tools` (default true), `embeddings`, `model`, `api_key_env`, `document_prefix`, `query_prefix`, `chunk_bytes`, `overlap_bytes` and the `llm_*` keys; see "The knowledge tools". `[overlays.NAME] kb = "index"` or `"files"` (absent: the overlay's own hint) is read from the app's table by `Config::local_profiles` into `kb_choice` (name to whether it is searched), and each `HostProfile::kb_choice`; any other value is refused naming the overlay.
- `[host.network]`: how every fetch and every program the host starts reaches the network; with no table, the system's proxy (environment, then the operating system's, PAC and WPAD included), proxy login, certificate verifier and keystore client certificates. `proxy` (`system`, `none`, `pac:LOCATION` or a proxy), `proxy_user` with `proxy_password_env` (a variable's name, never the password), `no_proxy` (a string split on commas and whitespace, or a list; replaces the system's), `negotiate`, `client_certificate` (`system`, `none` or a PEM file of a chain and its unencrypted key), `ca_file` (trust exactly these), `git_tls` (`system`: Schannel on Windows; `git`), `endpoints` (URLs added to the list a PAC script is asked about for the programs' proxy variables) and `proxy_idle_seconds` (how long the proxy a profile's `system` or `pac` mode runs is kept after its last session; default 0); paths are from canoa.cfg's folder, any other key or value is refused naming the key. `Serving::bind` installs it (`ServeError::Network` for one it cannot use: an unreadable proxy, a certificate file that is missing or empty, a password variable that is not set). See "The network".
- `[host.store]`: `protected` and `credential_variables`, each a string with a pattern a line or an array of such strings, in gitignore syntax (a leading `!` allows what an earlier line protected, the last matching line wins, `#` comments, `~/` the home, a drive or `/` the root, otherwise anywhere, a folder's pattern covers what is under it, names compare whatever their case). Absent, the recommended list applies (`store::protect::RECOMMENDED_PLACES`, `RECOMMENDED_VARIABLES`); a list written replaces it, an empty one protects nothing. `protected` is the places a program store's declared change may not be at, under or hold (`~/.ssh`, `~/.config/autostart`, ...) and the shared folders it may enter only by a place of its own (`~/.config` then `!~/.config/*`); `credential_variables` is the variable names an install is never given and a launch never sets (`*TOKEN`, `*_API_KEY`, `SSH_AUTH_SOCK`, ...). `skill_folders` is the common folders searched for `SKILL.md` directories when scanning a skills source (same string or array form; absent, `control/overlays/skill_folders.txt` applies; loaded agents' folders are still included). It is `[host.store]` and not `[permissions] protected` because the question differs: a state folder `~/.name` is a program's own place here,
and the app's `.*` would protect it. `index_ttl` (seconds, default 300; 0 asks on every request) and `index_cache_bytes` (default 64 MiB) are the catalog index's ("The catalog index" below). `tools` (default true) is whether the host offers the `canoa-store` tools ("The store tools" below). Any other key is refused by name. `Options::from_config` applies the lists.
  `index_ttl` (seconds, default 300; 0 asks on every request) and `index_cache_bytes` (default 64 MiB) are the catalog index's ("The catalog index" below). `tools` (default true) is whether the host offers the `canoa-store` tools ("The store tools" below). Any other key is refused by name. `Options::from_config` applies the lists.
- `[host.accounts.NAME]`: a name of 1 to 64 characters with no control character (it is a key, written quoted where it has to be, and never part of an address), with `harness`, `kind` and `id` (1 to 128 bytes, no control character, another account's refused) required and `settings` optional. Any other key is refused by name (`api_key is not a key of an account`), never quoting its value: **a credential never goes in canoa.cfg**. The credential rests in the provider `[secrets] accounts` names (`[host] account_store`, the key's old name, is refused). `local` is taken: the automatic account every harness that has a login of its own offers (`AccountOffer::local`) is called that, so `[host.accounts.local]` is refused. An agent entry's `accounts` names accounts the file holds (and may name `local`), none twice; with `program` it is refused. When `serve` starts, each account an entry offers is checked against the provider's catalog: its harness is the entry's, its kind is listed, its `settings` fit the kind's schema (a key the kind marks as a credential is refused as one, naming the key) and an endpoint setting is an `https` address or an `http` one to this machine.
- `[host.pools.NAME]`: a name as an account's, `standard` (a standard id as `coqui_acp_api::standard::is_standard` takes it: any, the set is open) and `accounts` (names in `[host.accounts]`, or `local`, each once, possibly none). Any other key is refused by name. A pool's name is not an account's, not `local`, and not `any-<standard>` for any standard (that is the host's own pool of it, and an account may not be named so either); an entry's `accounts` may name an account, `local`, a pool, or such an `any-<standard>`. That every member provides the pool's standard is checked where the account providers' kinds are known (`pools/set`, `accounts/offer`, and an entry that names the pool when the host starts): the refusal names the account, its kind's standards and the pool's. `pools` is in `ENTRIES`: a higher layer's pool of a name replaces a lower one's whole.
- `[host.profiles.NAME]`: a name as an entry's; `overlays` valid AOS overlay names; `allow_mcp` and `allow_hooks` values `sha256:` and 64 lower-case hex digits (`allow_hooks` keys valid overlay names); the rest `Profile::from_table` as `LayerKind::User`, its relative paths from the file's directory. An agent's `profiles` must name profiles the file holds, each once. Errors say `profile NAME:` and a setting, never a value.
- `[skill_pins]` (top level, read whether or not there is a `[host]`): each key a skill name (a letter or digit, then letters, digits, `_`, `.`, `-`; no device name), each value `sha256:` and 64 hex digits; two keys equal without regard to ASCII case, or anything else, refuse the file with `[skill_pins]` and what is wrong. Kept by lower-case name in `Config::skill_pins` and in each `HostProfile::skill_pins`.
- An entry's `listen` is `ADDRESS:PORT`, `pipe:NAME` or `unix:PATH` (`Listen::parse`). `pipe:NAME` is a Windows named pipe `\\.\pipe\NAME`, and on Unix the socket `$XDG_RUNTIME_DIR/NAME.sock` (else `$TMPDIR/NAME.sock`); `unix:PATH` is an absolute socket path, and is refused on Windows. A name or path `coqui-transport-pipe` cannot use is refused naming `listen`, never quoting it. Two entries cannot share a pipe or an address with a port; a pipe never conflicts with `[host.web]`'s address.
- An entry that names a `transport` listens through that provider (`Listen::Transport`): `listen` is then the provider's endpoint, not read by the host and required (a blank one is refused), `transport_settings` and the origin keys follow the rules of an entry's `settings`, `library` and `source` (an id that is not `pipe` or `websocket` needs `transport_library` or `transport_source`, a built-in one takes neither, the two exclude each other, and each `transport_*` key without `transport` is refused). Two entries cannot name one endpoint. `transport` and its keys are read in the same way in an agent, provider and tool entry; the entry's own `library`, `source` and `settings` stay the entry's provider's.
- A tool entry's `listen`, `working_roots`, `worktrees`, `peers` and `max_peers` are read as in the other tables; the rest is a `ToolSpec`, refused as a whole when `ToolSpec::check` refuses it. Its errors name a setting and a rule's index, never a value.
- `library` and `source` are made absolute against `base` (the file's directory for `load`); `directory` is `base` made absolute, where the provider's process runs.
- In `source_settings`, every text in a `dirs`, `manifests`, `paths` or `import_dirs` list, and `home` when it is text, is made absolute against the directory, since a source may otherwise resolve it against a later working directory or its own file's; other keys, `isolation` included, pass as written, for the source to refuse.
- `source_settings` and `settings` are TOML tables held as JSON. A date, or a float that is not a number, is refused; nothing checks them further until a connection loads the provider, since checking means loading its code.
- A leading byte-order mark is ignored; anything else not UTF-8 is a parse error.
- `[host.web]`: `mode` is `local` or `remote` (anything else is refused, `[host]: web: mode is local or remote`); `dir`, `cert` and `key` are made absolute against the directory; `cert` and `key` go together; `listen` beyond loopback without them is refused; `hosts` are names of letters, digits, `-` and `.`, kept in lower case; each `connect` source is `http`, `https`, `ws` or `wss`, `://`, a host of letters, digits, `-` and `.`, an optional port and an optional path of letters, digits, `/`, `-`, `.`, `_` and `~`, nothing else; every limit but `immutable_seconds` must be at least 1; `office_local` needs `office = true` and a loopback `listen` (`[host]: web: office_local needs ...`; behind a proxy every request looks local), `office_secret` needs `office_local = true`, and with `office_local` a `[host.control]` that lists an address needs no key under `grants`, since the page's key is not known to canoa.cfg; `office_frames` needs `office = true` and each source is `https://`, a host of letters, digits, `-` and `.` that may start with `*.` (and then holds another dot), and an optional port. Errors say `[host]: web:` and a setting, never a value.
  The table read is not checked for a site: `WebServer::bind` does that.

## Shells (`canoa_host::shells`, `[host] shell`, an entry's `shell`)

`shells::detect(&Lookup)` lists the shells on this computer, Canoa's own copies and everyone else's alike, each a `Shell {id, label, path, family, origin}`: on Windows Git Bash (`canoa-git-bash` for the Store's copy; `git-bash` for Git for Windows' own, found under `%ProgramFiles%`, `%LocalAppData%\Programs\Git` and beside a `git` on the `PATH`), MSYS2's bash (`msys2`), WSL's `bash.exe` (`wsl`), any other `bash.exe` on the `PATH` (`bash`), PowerShell 7 (`pwsh`, and `canoa-pwsh`) and Windows PowerShell (`powershell`); elsewhere `bash`, `zsh`, `pwsh` and the login shell (`login`) when it is none of those. A second of a kind has `-2` after its id. `Lookup::here()` is this computer; a test gives its own folders. Nothing is run to find one.

A setting (`[host] shell`, an entry's `shell`) is an id of that list, else a program: an absolute path (`.exe` added on Windows) or a name found only in an absolute folder of the `PATH`, never the folder the host is in. A setting that names none stops an entry that has its own `shell` from starting (`host entry NAME: ...`) and is logged for the host's, which every other entry then runs without.

One setting, translated for each harness by `shells::for_harness(harness, &Shell, windows)` into `Applied {env, notice}`: only what the harness documents is passed, and nothing writes a program's own settings file. Claude Code gets `CLAUDE_CODE_GIT_BASH_PATH` on Windows for a `bash.exe` or `sh.exe` that is not WSL's launcher (its documentation: "path to the Git Bash executable (`bash.exe`)", ignored unless the file is named `bash.exe`, `sh.exe`, `bash` or `sh`), and `CLAUDE_CODE_SHELL` elsewhere for a `bash` or `zsh` ("a path to a `bash` or `zsh` binary"); a PowerShell is not passed (its PowerShell tool runs the first `pwsh` on the `PATH`, which a Store copy is on). Codex has no setting for its shell (it looks for `pwsh` and `powershell` on the `PATH` on Windows and takes the account's login shell elsewhere), Pi reads `shellPath` only from its own `settings.json` (and otherwise finds Git Bash in `%ProgramFiles%`, then `bash.exe` on the `PATH`, which a Store copy of Git Bash is on), and Grok and Antigravity document none: each is logged at `info` as "shell X is not passed: why".
The variables reach the harness the way the cache-lifetime ones do (`kind::Harness::with_shell_env`): added to each connection's environment, a profile's own variable of the same name winning.

`shell/list` (grant `terminal`, or `store.read`) answers `{"shells": [Shell...], "setting", "default"?: Shell, "problem"?, "agents": [{"entry", "harness", "shell", "own", "id"?, "env"?: [names], "notice"?, "problem"?}]}`: the shells found, `[host] shell` as written and the shell it names (or why none), and for each harness entry which setting applies, the variables its harness is told (names only) and why it cannot be told.
