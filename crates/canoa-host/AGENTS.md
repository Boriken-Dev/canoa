# `canoa-host` — API header

Header-file-style reference for the `canoa-host` crate: every public item with its signature, contract and gotchas, so the crate can be used without reading its source. It ships inside the package and is self-contained. Development documentation lives with the source at <https://github.com/Boriken-Dev/canoa>.

No Cargo features. MSRV Rust 1.89. Version `0.0.0-alpha.0`, not published (`publish = false`): its git dependencies are not released. It depends on `canoa`, `clap`, `coqui-acp`, `coqui-acp-bridge` and `coqui-acp-provider` (coqui-acp `ee19118`), `coqui-agentic-peer` (`9b1fee4`, which brings `coqui-peer-noise`), `coqui-peer`, `coqui-peer-webcrypto`, `coqui-web` (`websocket`), `guatiao` (`provider`, `load`), `guatiao-serde` (no format), `indexmap`, `serde`, `serde_json`, `toml`, `tracing` and `tracing-subscriber`.

The root re-exports `Config`, `ConfigError`, `Entry`, `EntryKind`, `Origin`, `Settings`, `Peer`, `PeerState`, `PeersError`, `HostLine`, `ProviderError`, `ProviderOptions`, `serve_stdio`, `Listening`, `ServeError`, `ServeOptions`, `Serving`, `Stopper` and `ToolSpec`. Public modules: `aos`, `config`, `peers`, `profile`, `provider`, `serve`, `tool`.

No `unsafe` code (`#![forbid(unsafe_code)]`). Loading a provider library or source runs its code; see `serve_stdio`.

## Canoa.cfg (`canoa_host::config`)

The host reads only `[host]`; the app's tables (`desktop`, `network`, `assets`, `appearance`, `agents`, `overlays`, `profile`) are not read. Every table the host reads refuses a key it does not know.

```toml
[host]
origins = ["https://canoa.example"]  # optional: browser origins that may connect; empty or absent lets any, and the proof decides

[host.agents.NAME]                    # a program speaking ACP on its standard streams
program = "npx"                       # required; found as the OS finds a command
args = ["-y", "@agentclientprotocol/claude-agent-acp@0.99.1"]
profiles = ["review", "plain"]        # optional: [host.profiles] a client may choose among, the first by default

[host.profiles.NAME]                  # a profile (see Profiles below), read as the person's own layer
overlays = ["team"]                   # optional: AOS overlays the app installed in overlays/ beside this file
allow_mcp = { team = "sha256:…" }     # optional: an overlay's MCP server and the definition digest it may start at
context = { text = "..." }            # and every key of the profile schema

[host.providers.NAME]                 # a Guatiao agent_provider, from a library or a source
library = "echo_provider.dll"         # a library file, relative to the file's directory; or:
source = "guatiao_source_py.dll"      # a source file, the same way; exactly one of the two
provider = "echo_provider"            # required; the provider's id
max_sessions = 8                      # sessions one connection may hold
import_search = "library"            # with source only, Windows: "library" or "path"; default library when import_dirs names a directory, else path, logged as weakened

[host.providers.NAME.source_settings]  # with source only; what the source is added with
dirs = ["python"]                     # for guatiao_source_py: directories of manifests, each <name>.guatiao[.<major>[.<minor>]]
paths = ["/opt/guatiao/python"]       # and what goes on sys.path; relative paths are made absolute from the file's directory
isolation = "shipped"                 # optional: "shipped" (the source's default), "isolated" or "inherit", which is logged as weakened
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
listen = "127.0.0.1:7801"             # loopback only; default 127.0.0.1:0, any free port
working_roots = ["/home/me/src"]      # absolute, no . or ..; a session's cwd must be under one; default any
worktrees = true                      # agents and tools: a git worktree per session inside working_roots; see "Worktrees"
peers = ["sha256:…"]                  # paired keys that may use it; default every paired key
max_peers = 4                         # connections at once
```

Names are 1 to 64 letters, digits, `-` or `_`, unique across the three tables. Two entries may not name the same non-zero address.

```rust
pub const DEFAULT_MAX_PEERS: usize = 4;
pub const DEFAULT_MAX_SESSIONS: usize = 8;
pub const MAX_NAME: usize = 64;

#[non_exhaustive] pub struct Config { pub origins: Vec<String>, pub entries: Vec<Entry>, pub profiles: IndexMap<String, HostProfile> }   // Default, Clone, Eq
pub fn Config::load(path: &Path) -> Result<Config, ConfigError>
pub fn Config::from_str_in(text: &str, base: &Path) -> Result<Config, ConfigError>   // touches no file
pub fn Config::entry(&self, name: &str) -> Option<&Entry>

#[non_exhaustive] pub struct HostProfile { pub profile: Profile, pub overlays: Vec<String>, pub allow_mcp: BTreeMap<String, String>, pub directory: PathBuf }
#[non_exhaustive] pub struct Entry { pub name: String, pub kind: EntryKind, pub listen: SocketAddr, pub working_roots: Vec<PathBuf>, pub worktrees: bool, pub peers: Option<Vec<PeerId>>, pub max_peers: usize }
#[non_exhaustive] pub enum EntryKind { Agent { program: OsString, args: Vec<OsString>, profiles: Vec<String> }, Provider { origin: Origin, provider: String, settings: Option<Settings>, directory: PathBuf, max_sessions: usize }, Tool { spec: ToolSpec } }
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
- `[host.profiles.NAME]`: a name as an entry's; `overlays` valid AOS overlay names; `allow_mcp` values `sha256:` and 64 lower-case hex digits; the rest `Profile::from_table` as `LayerKind::User`, its relative paths from the file's directory. An agent's `profiles` must name profiles the file holds, each once. Errors say `profile NAME:` and a setting, never a value.
- A tool entry's `listen`, `working_roots`, `worktrees`, `peers` and `max_peers` are read as in the other tables; the rest is a `ToolSpec`, refused as a whole when `ToolSpec::check` refuses it. Its errors name a setting and a rule's index, never a value.
- `library` and `source` are made absolute against `base` (the file's directory for `load`); `directory` is `base` made absolute, where the provider's process runs.
- In `source_settings`, every text in a `dirs`, `manifests`, `paths` or `import_dirs` list, and `home` when it is text, is made absolute against the directory, since a source may otherwise resolve it against a later working directory or its own file's; other keys, `isolation` included, pass as written, for the source to refuse.
- `source_settings` and `settings` are TOML tables held as JSON. A date, or a float that is not a number, is refused; nothing checks them further until a connection loads the provider, since checking means loading its code.
- A leading byte-order mark is ignored; anything else not UTF-8 is a parse error.

## Serving (`canoa_host::serve`)

```rust
pub const MAX_LINE: usize = 16 * 1024 * 1024;
pub const MAX_UNPROVEN: usize = 64;
pub const PROOF_LIMIT: Duration = 15 s;
pub const FIRST_MESSAGE: Duration = 60 s;
pub const ACP_CAPABILITY: &str = "acp/1";
pub const OUTPUT_LIMIT: usize = 1024 * 1024;   // what a provider's process may print per connection
pub const START_LIMIT: Duration = 30 s;        // for a provider's process to connect back

#[non_exhaustive] pub struct ServeOptions { pub provider_program: PathBuf, pub key_limit: Option<u64>, pub rekey: bool }
pub fn ServeOptions::new(provider_program: impl Into<PathBuf>) -> ServeOptions   // each method's defaults: key limit 2^38, rekey on
pub const fn ServeOptions::with_key_limit(self, bytes: u64) -> ServeOptions      // no floor; above the default the log says weakened
pub const fn ServeOptions::with_rekey(self, rekey: bool) -> ServeOptions

pub fn Serving::bind(config: &Config, identity: coqui_agentic_peer::auth::Identity, store: TrustStore, options: &ServeOptions) -> Result<Serving, ServeError>
pub fn Serving::method(&self) -> coqui_agentic_peer::auth::Method   // the identity's: every entry proves by it
pub fn Serving::listening(&self) -> &[Listening]
pub fn Serving::stopper(&self) -> Stopper      // Clone, Send: for another thread
pub fn Serving::run(self)                      // takes connections until the process ends
pub fn Stopper::stop_agents(&self)             // ends every agent process running, and each later one before it serves
#[non_exhaustive] pub struct Listening { pub name: String, pub kind: &'static str, pub url: String }   // kind: "agent", "provider" or "tool"
```

- `stop_agents` ends each agent and each provider process with what it started (job object on Windows, process group on Unix), and turns every later proven connection away before its agent serves it. The listeners stay bound: end the process to free them.

- `bind` binds every entry before any is served, so a failure leaves none listening. A plain socket beyond loopback is refused by `coqui-web` too.
- Each connection: the WebSocket handshake, then the proof of the identity's method, Noise (`coqui-peer-noise`) or WebCrypto (`coqui-peer-webcrypto`), within `PROOF_LIMIT`; a client of the other method fails it, and the log names both methods. Then the trust store decides (a stranger that proved its key is recorded as waiting), then the entry's `peers`, through a `canoa::Registry` holding one verified `Service` per entry and a grant of `ACP_CAPABILITY` per listed key. Every later message is sealed in the proof's session, which is checked against the store every second: a removed key loses its link.
- On an agent or tool entry, worktree requests (see "Worktrees") are answered by the host before anything else, up to 16 per connection, then the connection ends; once a first other message has come, one is answered `-32600`.
- The first message must be plain ACP: a link of streams (`coqui_agentic_peer::mux`) is refused. Then a fresh process serves that connection, relayed by `coqui_acp_bridge::relay`, and is stopped with its process tree when either side ends.
- An agent entry runs `program args`. Every message from the client to it is parsed and the host's own serialization forwarded: `params` other than an object is answered `-32602`, since a list would fill a request's fields in order; `_meta` is removed at every depth; a request whose `params.mcpServers` holds anything but `http` or `sse` over `https`, or `acp`, is answered `-32602` and never reaches the agent; with `working_roots`, so is one whose `cwd` or `additionalDirectories` is outside them; what is not one JSON object is answered `-32700` or `-32600`. A refused notification is dropped.
- **An agent entry with `profiles`** runs `provider_program launch -- PROGRAM ARGS...` instead, whose first line of standard input the host writes: the profile's environment (see `profile::launch`). The client chooses a profile by name in its first message, `initialize`, under `params._meta["dev.boriken.canoa/profile"]` (`profile::PROFILE_META`); none takes the first. A first message that is not `initialize` is answered `-32600` (`-32602` when it names a profile), a name the entry does not offer or a value that is not text `-32602`, each logged, and then no reference is read and no process starts; a choice in any later message is answered `-32602` and logged. A reference that cannot be resolved (`from_env` unset, `from_file` unreadable) answers `-32603` and starts nothing. Each overlay the profile names is read from `overlays/NAME` beside Canoa.cfg (4096 files, 64 MiB at most), with no link or excluded name, and must have the digest `overlays/NAME.json` records; else it is left out and reported. The agent's `initialize` answer decides the harness (from `agentInfo.name`, else an entry named `Claude`, `Codex`, `Gemini` or `Pi`) and what it takes (`systemPrompt`, `additionalDirectories`, MCP transports, by ACP v1 or v2 capability paths); then every `session/new`, `session/load` and `session/resume`, after the scope above, gets the context as `systemPrompt` (`append`, before a client's own blocks), the MCP servers in `mcpServers` (v1's stdio form untyped; a client's server of a name the profile holds, started or not, is `-32602`), skills written once per connection to a private temporary directory (removed after the agent ends) as the Claude plugin `canoa-profile` in `_meta.claudeCode.options.plugins` or a directory Codex reads (`.agents/skills` inside) in `additionalDirectories`. A session request before `initialize` is answered `-32600`. A stdio server's `command` goes absolute: as written, from its layer's directory when it holds a separator, else the first match on the host's `PATH` (absolute entries only; `PATHEXT` on Windows), refused when none is found. An overlay's MCP server starts only when `allow_mcp` names it at its digest (`sha256` of `{"layer", "server"}` as the view writes them, and `"program"`, the stdio program found); off, it is logged with that digest. The answer to `initialize` and to each session request carries `_meta[PROFILE_META]` = `{"name", "harness", "summary", "items": [{"key", "layer", "state", "how", "digest"?}]}`, `state` being `applied`, `not supported`, `off` or `refused`: names, never values. No project profile is read.
- A tool entry is served as an agent entry whose program is `provider_program` itself: `provider_program tool --name NAME --spec JSON`, the entry as `ToolSpec` serializes it. So the scope above applies to its clients, and the process, with every tool process it starts, ends with the connection.
- A provider entry is not filtered: its facade reads the client's `_meta` launch values (`context`, `model`, `effort`, an opaque `meta`) and hands them to the provider, and refuses an environment name. What a provider does with them is the provider's.
- A provider entry runs `provider_program provider (--library L | --source S) --provider ID --max-sessions N --directory D --settings-on-stdin [--import-search library|path] [--working-root R]... --connect A`, so a library or source is loaded only in that connection's process, which runs in `D`. The command line holds paths, names and the address only. For each connection the host listens on a loopback port of its own, `A`; writes the entry's `source_settings` and `settings` and a one-time token of 32 random bytes as the first line of the process's standard input (`HostLine`), then closes it; and takes the first connection to `A` that sends the token within `START_LIMIT`, up to 8 that do not. ACP runs on that socket alone. What the process writes to standard output or error goes to the host's standard error, `OUTPUT_LIMIT` bytes per connection in all, the rest read and dropped after one warning; none of it reaches the client. The process and what it starts are ended with the connection, by job object on Windows and process group elsewhere. Its facade refuses any `mcpServers` and checks `working_roots` itself.
- On Windows a source entry whose `import_search` is `path`, named or by default, is logged as `bind` binds it: `weakened: host.providers.NAME.import_search is path, looser than library: …`, saying to set `import_dirs` or `import_search = "library"`.
- A provider entry whose `source_settings` hold `isolation = "inherit"` is logged as `bind` binds it: `weakened: host.providers.NAME.source_settings.isolation is inherit, looser than shipped`. No other source setting is judged.
- `max_peers` connections per entry at once; past it a proven peer is turned away. Past `MAX_UNPROVEN` handshakes at once across entries, a new connection is dropped.

```rust
#[non_exhaustive] pub enum ServeError { NothingToServe, Listen { entry: String, failure: Failure } }
```

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
- **A source** is added with `settings` as a Guatiao map, then the registry is refreshed once; a library the source refuses is logged and passed over. On Windows the import search is the entry's `import_search`: an import the source names itself that the default search leaves missing (`python3.dll`, for Python) is mapped from the first of `import_dirs` holding it, and with `ImportSearch::Path` else the Windows directory, else **the working directory (Canoa.cfg's, under `serve`) or a directory on `PATH`**, in that order; the source is then opened with the default search, so its other imports come from where they did. An import of an import is not looked for. Elsewhere the search is the platform's (`LD_LIBRARY_PATH`, the file's run path, the system's).
- **The provider**: with no `settings`, its own instance; a provider built only from a configuration is `NeedsConfiguration`. With `settings`, an instance built from them: Guatiao checks them against the schema the provider declares, then the provider may refuse them; a provider that is its own one instance is `TakesNoSettings`.

**Loading runs code** with this process's rights, before any check, and trusts whoever can write any file it reads or the directories holding them. For a library, that is the file. For `guatiao_source_py` it is also: the Python whose shared library the loader finds (on Windows the first `python3.dll` in `import_dirs` or the search above, then the `python3X.dll` it names); `GUATIAO_LIBRARY` in this process's environment, which picks the native library Guatiao's Python binding loads; every directory in `paths`; every manifest in `dirs` and `manifests` and every directory its own `paths` names; every module those import; and what the source's `isolation` lets the interpreter read:

| `isolation` | The interpreter also reads |
| --- | --- |
| `shipped` (the source's default) | the standard library and `site-packages` of `home` or the Python found, with their `.pth` files; a `canoa-host._pth` or `pyvenv.cfg` beside the `canoa-host` executable, and on Windows that executable's directory, which CPython puts on `sys.path`, so whoever can write that directory chooses where modules come from. Not the `PYTHON*` variables or the user's own site-packages. |
| `isolated` | the standard library of `home` or the Python found, and nothing beside the executable, in `site-packages` or in the environment |
| `inherit` | all of `shipped`'s, the user's own site-packages, and the `PYTHON*` variables this process inherits from the host (`PYTHONPATH`, `PYTHONHOME`). `serve` logs it as weakened. |

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
- **`ask`** sends `session/request_permission` (v1: `toolCall` `{"toolCallId": "canoa-host-ask-N", "title", "status": "pending", "rawInput": {"text": the whole question, cut to MAX_TEXT}}`, options `allow`/`allow_once` and `reject`/`reject_once`) and writes `allow` only for an answer selecting `allow` while the turn that asked runs uncancelled; any other answer and an error write `reject`. A request that comes with no turn running, cancelled, or `MAX_ASKS` waiting is refused at once, without asking. When a turn is cancelled or ends, every question of its session still waiting gets `reject` then and is forgotten, so a later answer reaches nothing; so is one whose process was replaced or ended.
- **Processes** run `program` (with `args`; in `message` mode `resume_args` first once a `session` rule has recorded one) in the session's `cwd`, with this process's environment. A line to one goes through a queue of 64; a full queue ends it. Its standard error goes to this process's, `OUTPUT_LIMIT` bytes per process. A line of output over `serve::MAX_LINE` ends it. On Windows each process is in a job object of its own, ended with it, and runs with `NoDefaultCurrentDirectoryInExePath` set, so a `.cmd` tool does not take a program it names bare from the client's `cwd`; on Unix it stays in this process's group, and ending it ends only it. When `input` ends, each process's input is closed, and it is ended 1 s later; under `serve` that does not happen, since the host ends the whole connection's process tree at once.

## Profiles (`canoa_host::profile`)

The schema of a profile, what an agent is launched with, and its merge in layers. The app reads the same schema; `tests/vectors/profile.json` holds both to the same views and refusals. `serve` applies a `[host.profiles]` profile to an agent entry that offers it (see Serving).

```rust
pub const PROFILE_META: &str = "dev.boriken.canoa/profile";
pub fn launch(program: &OsStr, args: &[OsString], max: usize) -> io::Error
```

`launch` reads one line of at most `max` bytes, `{"canoa_host_launch": 1, "env": [[NAME, VALUE], ...]}`, runs `program` with those variables added, passes the rest of standard input to it, and exits this process with its status (1 for a signal); it returns only an error: `InvalidData` for a line of another form, or why the program did not start. It is `canoa-host launch`.

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
pub fn is_reserved_env(name: &str) -> bool  // any of those, without case, and every name aos::is_sensitive names
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
#[non_exhaustive] pub struct AosLayer { pub identity, root, os, digest: String, pub pinned: bool, pub manifest: Map<String, Value>, pub files: BTreeSet<String>, pub texts: BTreeMap<String, Vec<u8>>, pub references: Vec<(String, String, bool)>, pub not_applied: Vec<String> }
#[non_exhaustive] pub struct OverlayReport { pub layer: LayerId, pub digest: String, pub not_applied, ungranted, unresolved: Vec<String>, pub weakenings: Vec<aos::Weakening> }   pub fn OverlayReport::view(&self) -> Value
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
- **`parse_git`** reads an AOS source string (`aos::parse_source`): the path follows the first `#`, without leading or trailing `/`, and must be an AOS relative path; the ref follows the last `@` when it holds no `/`, `:` or `\`. Refused besides what AOS refuses: a source that is not Git, or names no ref; a repository holding a space or control character, or a scheme other than `https`, `http`, `ssh`, `git`, `file` in lowercase; `file://` with a host, or a path opening with `\\` or `//` (another host's share); `http://` or `git://` at a ref that is not a `full_commit_id`, since nothing else checks what a plaintext transport carries; a ref `git check-ref-format` refuses, or one opening with `-`. `Dir` is never parsed here.
- **`overlay_layer`** makes the layer of an AOS overlay from its tree, whose digest it computes: the root variable `<NAME>_OVERLAY_ROOT` (a `Value`), then each env file's and `env`'s variables as written, then path additions per variable, merged as any layer's; skills `skills/<name>`; MCP servers (a command with a separator made absolute under the root; `env_passthrough` and `headers_env` as optional `env:` references; no header over plain `http` to another host); `contexts` from rules, routing lines, `context` and the harness's `context`; `model` and `effort` from `[harness.<input.harness>]`. Mapping entries are taken in key byte order. Refused whole: a name AOS refuses, a tree it cannot digest, an unreadable or invalid `overlay.cfg` (problems under `/about` alone never refuse), a missing or unreadable env file, a variable `is_reserved_env` names. Not applied, and listed in `not_applied`: the `bin` and `lib` defaults, env scripts, setup, dependencies, `runtime`, `scripts`, `mcp.skills`, an MCP `timeout` and working directory, other harnesses' hints and the harness's own keys.
- **An AOS layer in a merge** adds an `OverlayReport` (grants: the merged `share_env`, as `env` provider keys) to `overlays`, and its `AosLayer` to `aos`. `unset = ["context"]` also takes back every `contexts` entry below. `Effective::environment` builds the overlays' environment first, lowest first, over this process's environment through `lookup`: every root variable, then each overlay's env files, `env` and path additions with substitution. A placeholder reads what overlays set and the shared variables only; any other variable is seen by a path addition and `if_unset`, never a placeholder. A variable an overlay won takes the built value (an optional reference not resolved leaves it out); an error is `Resolve` with key `overlay:<identity>.env.<subject>`. The view adds `contexts` and `overlays` when there are any.

## AOS overlays (`canoa_host::aos`)

The Agent Overlay Specification v0 (edition `0.1.0-draft`, vectors of agent-overlay-spec `061b8f5` in `tests/vectors/aos/`): manifest reader (TOML, and JSON after `#!json`; YAML and other formats `unsupported-format`), validator, integrity, resolver and environment. No env script, setup or package manager is run. Every function is static except `read_tree`, which reads a directory.

```rust
pub struct AosError { pub code: &'static str, pub subject: String }   // the vectors' class; subject: a variable, path or format, never a value
pub struct Limits { pub max_bytes: usize, pub max_depth: usize }       // Default: 1 MiB, 64
pub fn read_document(bytes: &[u8], limits: Limits) -> Result<Map<String, Value>, AosError>
pub fn parse_dotenv(bytes: &[u8]) -> Result<Vec<DotenvLine>, AosError>     DotenvLine { name, text, literal }
pub fn read_dotenv(bytes: &[u8], env: &mut Environment) -> Result<IndexMap<String, String>, AosError>
pub fn substitute(text: &str, lookup: &dyn Fn(&str) -> Result<Option<String>, AosError>) -> Result<String, AosError>   // too-large, too-deep, too-complex past SubstLimits
pub fn substitute_with(text, lookup, limits: SubstLimits) -> Result<String, AosError>   SubstLimits { max_value: 1 MiB, max_depth: 32, max_steps: 10_000_000 }
pub fn is_insecure_url(url: &str) -> bool   // plain http to a host other than localhost, [::1] or an IPv4 address in 127.0.0.0/8
pub fn validate(doc: &Map<String, Value>, files: Option<&BTreeSet<String>>) -> Vec<Issue>   Issue { code, at }   // sorted by pointer
pub fn applies(issues: &[Issue]) -> bool                // only problems under /about
pub fn valid_overlay_name, normalize_name, root_variable, state_variable, is_relative_path, is_reserved_variable, is_sensitive, is_env_name
pub fn parse_source(text: &str) -> Result<Source, AosError>   Source { kind: SourceKind (Git, Url, Directory), location, reference, path }
pub fn digest_entries(entries: &[TreeEntry]) -> Result<TreeDigest, AosError>   TreeEntry { components: Vec<Vec<u8>>, kind: EntryKind, data }, TreeDigest { digest, lines }
pub fn read_tree(root: &Path, max_files: usize, max_bytes: u64) -> Result<Vec<TreeEntry>, AosError>   // never follows a link or junction
pub fn sha256_digest(data: &[u8]) -> String     pub fn same_digest(a: &str, b: &str) -> bool
pub fn resolve(overlays: &BTreeMap<String, Map<String, Value>>, requested: &[String]) -> Result<Resolved, Vec<String>>   // Err: the cycle
pub fn composition_order(overlays, active: &[String]) -> Vec<String>   // (priority, name, identity), by code point
pub fn rules_of, context_of, skills_of(manifest, files) -> Vec<String>;  paths_of -> Vec<PathAddition>;  env_files_of -> Vec<EnvFile>
pub struct Environment<'a>   new(windows), with_base(windows, base: &dyn Fn(&str) -> Option<(String, bool)>), get, set(name, value, secret), lookup, is_secret, values()
pub struct Facts { os, grants, host_env, secrets, secret_files, providers }   Facts::new(os): only the env provider
pub struct Overlay<'a> { identity, manifest, files, root, state, read: &dyn Fn(&str) -> Option<Vec<u8>> }
pub fn apply_overlay(env: &mut Environment, overlay: &Overlay, facts: &Facts, weakenings: &mut Vec<Weakening>) -> Result<(), AosError>
pub fn grants_report(manifest, facts, weakenings) -> Vec<String>   // not granted: <provider>:<key>, secret:<name>, env:<variable>
```

- `[about]` is validated (its own pointer), never applied; Canoa's app shows a valid `summary` and `icon` only.
- Mapping order carries no meaning: `serde_json::Map` iteration is what a caller gets.
## Worktrees

No public API: what `serve` answers on an agent or tool entry with `working_roots` and `worktrees` on (the default), before its agent starts. Off, or with no root, each is `-32602`.

| Method | Params | Result |
| --- | --- | --- |
| `_canoa/worktree/new` | `cwd`, `branch` | `{"path", "branch", "repo"}`: `path` and `repo` written under the root as configured |
| `_canoa/worktree/status` | `path` | `{"exists": false}`, or `{"exists": true, "branch", "changed", "commits"}` |
| `_canoa/worktree/remove` | `path`, `discard` (bool, default false) | `{"removed": true, "branch", "branchKept", "commits"}` |

- **A branch** is `canoa/` then runs of `a-z0-9` joined by single hyphens, 70 bytes at most; any other is `-32602` before git runs.
- **A path** (`cwd`, `path`) must be plain absolute, on a drive on Windows (a share or device path is refused unlooked at), under a root as written, and, resolved by `canonicalize`, under a root resolved the same way. A missing `path` is `{"exists": false}` for `status`.
- **`new`**: the repository's main worktree (`git worktree list --porcelain -z`, its first entry; a bare one refused) resolved must be under a resolved root, and so must its parent; the worktree goes in `<parent>/<name>.worktrees/<branch less canoa/>`. An existing `.worktrees` that is a link or not a directory, or a worktree already there, is refused. Then `git worktree add -b BRANCH -- PATH HEAD` from `cwd`; a result whose resolved path is not itself is removed again and refused.
- **`status` and `remove`** take only a worktree `new` would have made: its own top level (`rev-parse --show-toplevel`), not the main worktree, on a branch `new` takes, at the place `new` puts that branch; else `-32602`. `changed` counts what `git status --porcelain -z --untracked-files=all` lists, `commits` what `rev-list BRANCH --not --exclude=BRANCH --branches --remotes --tags` counts. `remove` runs `git worktree remove [--force] -- PATH` (`--force` only with `discard`, so a worktree holding changes or untracked files is git's to refuse), then `git branch -D -- BRANCH` only when `commits` is 0.
- **Git** is `git` on the host's `PATH`, run as `git -C DIR -c core.hooksPath=NUL|/dev/null -c core.fsmonitor=false ...` with every `GIT_` variable of the host's removed and `GIT_TERMINAL_PROMPT=0`; a failure is `-32603` `git failed: ` and up to 1 KiB of git's own words, control characters made spaces.
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

## Errors

- `ConfigError` (`#[non_exhaustive]`): `Read { path, source }`; `Parse { line, column, what }`, 1-based, `what` in the schema's words; `Invalid { entry, what }`. No variant quotes a value from the file; an entry's name is quoted.
- `ProviderError` (`#[non_exhaustive]`): `Load(String)`, `Source(String)`, `Missing`, `WrongKind`, `NeedsConfiguration`, `TakesNoSettings`, `Settings(String)` (names a setting, never its value, unless the provider's own words do), `HostLine(&'static str)`, `Transport(io::Error)`, `Facade(String)`.
- `PeersError` (`#[non_exhaustive]`): `NotAnId`, `NotWaiting`, `AlreadyAccepted`, `Store(String)`.

## Command line

```text
canoa-host serve [--config FILE] [--until-stdin-closes] [--method noise|webcrypto] [--key-limit BYTES] [--no-rekey]
canoa-host check [--config FILE] [--format text|json]
canoa-host id [--method noise|webcrypto]
canoa-host peers [--format text|json]
canoa-host accept PEER
canoa-host revoke PEER
canoa-host pair [--listen ADDR] [--timeout-seconds N] [--until-stdin-closes] [--method noise|webcrypto]
canoa-host provider (--library FILE | --source FILE [--import-search library|path]) --provider ID [--working-root DIR]... [--max-sessions N]
                    [--directory DIR] [--settings-on-stdin [--connect ADDRESS]]
canoa-host tool --name NAME --spec JSON
canoa-host launch PROGRAM [ARGS]...
```

- `launch` is what `serve` runs for an agent entry with profiles: the environment on the first line of standard input, the rest to `PROGRAM`, whose status it exits with; a first line of another form is status 2.

- `--method` names the proof: this user's identity of that method, made when absent, and the home then records that method in `method`. Without it, the method the home records (the one named last, or its first identity's), or a new identity of the default method, Noise. An identity never changes method. A client of the other method is logged as `a client was refused: … the peer proves by X, and this endpoint only by Y`, with no word of `accept`: its key has to be paired again under this host's method. `--key-limit` and `--no-rekey` set the session, each method's default otherwise, and a looser one is logged as `weakened`.
- `serve` prints one JSON object per entry on standard output, `{"name", "kind", "url", "peer", "method"}`, `peer` being this host's id and `method` its proof's, once every entry is bound; then it serves until stopped. It logs on standard error.
- `serve --until-stdin-closes`: once every entry is bound, standard input is read and discarded; when it closes, every agent is ended (`Stopper::stop_agents`) and the host exits 0. For a parent that runs the host as its child: the host ends with it, however the parent ends. Without the flag the input is never read.
- `pair` (`--listen` default `127.0.0.1:0`, loopback only, else status 2; `--timeout-seconds` default 300) prints `{"pin", "peer", "method", "url"}`, a six-digit PIN, this host's id, its method and the address to dial, and pairs by that method; a native client joins with `coqui_agentic_peer::pairing::join` (`coqui-acp-bridge pair-dial URL`). The joiner's key is accepted in the store once the PIN confirms both keys, and `pair` prints `{"paired": ID}` and exits 0. One wrong try ends it (status 1), as does the timeout, and with `--until-stdin-closes` its input closing. A pairing connection serves no agent. A WebSocket upgrade that carries an `Origin` header, which every browser sends, is refused (403) before the handshake, so a web page cannot spend the PIN's one try; a browser pairs by connecting to `serve` once and being accepted.
- On `serve --until-stdin-closes` and `pair --until-stdin-closes`, nothing is logged between the input closing and the exit, and the process exits within 5 s whatever is left: a parent that died can leave standard error a pipe a write to which never returns (seen on Windows under a Dart parent).
- `accept` and `revoke` take `--` before the id.
- `tool` serves one tool entry on standard input and output, `JSON` being the entry as `ToolSpec` serializes it (without `listen`, `working_roots`, `peers`, `max_peers`): what `serve` runs per connection. A `JSON` that is not one, or that `check` refuses, is status 2.
- `check` prints `NAME KIND LISTEN` per entry, or `{"name", "kind", "listen"}` with `--format json`, and serves nothing.
- `peers` prints `accepted ID` and `waiting ID [from ADDRESS]`, or `{"peer", "state", "source"}`.
- Canoa.cfg is `--config`, else `CANOA_CONFIG`, else `%APPDATA%\Canoa\Canoa.cfg` on Windows, `~/Library/Application Support/Canoa/Canoa.cfg` on macOS, `$XDG_CONFIG_HOME/Canoa/Canoa.cfg` (else `~/.config/…`) elsewhere: where the app keeps it.
- `provider` takes `--library` and `--source` from where it was started, then runs in `--directory`; with `--settings-on-stdin` it reads a `HostLine` first, as `serve` writes it; with `--connect` it serves on that loopback socket after sending the line's token, rather than on standard input and output.
- Exit status: 0 success; 1 the operation failed; 2 the invocation or Canoa.cfg was wrong, `serve` with nothing to serve, or a text that is not a peer id.

## Environment variables

| Variable | Effect | Default |
| --- | --- | --- |
| `CANOA_CONFIG` | Canoa.cfg to read, below `--config` | the platform's place, above |
| `CANOA_LOG` | what to log, a `tracing` filter | `info` |
| `COQUI_AGENTIC_PEER_HOME` | the directory of the identities (`identity` for WebCrypto, `identity-noise` for Noise) and trust store (`peers/`) | `coqui-agentic-peer` under `%LOCALAPPDATA%`, `$XDG_CONFIG_HOME` or `~/.config` |
| `NO_COLOR` | no colour in the log | colour on a terminal |

## Gotchas

- Under Noise a client's handshake ends before the host decides, so a client the host refuses sees its link end rather than a failed proof.
- The identity and trust store are the ones `coqui-acp-bridge serve --paired` uses: a key paired for one is paired for the other, and `canoa-host revoke` removes it from both.
- A stdio agent inherits the host's environment and working directory. An agent can still start whatever its own tools start when the client allows them; the host stops only what a client could start without the agent asking.
- A profile's variable and MCP server values never reach the client from the host, but they reach the agent, which acts for that client and may show them: give a profile only what that entry's clients may use.
- A paired key reaches every session an agent keeps for this user: `session/list` and `session/load` pass to a stdio agent unchanged.
- `working_roots` compare paths as written: a link inside a root that points outside it is followed by the agent. Worktrees are made, read and removed only where every link is resolved, inside the roots resolved too.
- If the host itself is killed, on Windows the system ends its agents with it (each is in a job object closed with the host); on Unix each sees its standard input close, and most then exit. `serve --until-stdin-closes` ends them on every platform when the parent goes.
- Each connection is a fresh process, so a turn in progress ends with its link; a later connection resumes a session only if the agent kept it.
- `peers` is read when the host starts; a key removed from the trust store loses its link within about a second without a restart.
- A provider's process inherits the host's environment. For a Python source that includes, on Windows, `PATH`, which decides which `python3.dll` loads under `import_search = "path"` unless `import_dirs` holds one; and with `isolation = "inherit"`, `PYTHONPATH` and `PYTHONHOME`.
- Settings come from Canoa.cfg only. A client reaches a provider through ACP, never through its settings; `check` does not load the provider, so a setting its schema refuses shows only when a connection loads it, on the host's standard error.
- A tool runs with the host's rights and does unasked whatever its rules do not turn into `ask`: the host shows its tool calls as they run, never before. `unasked` is the entry writer's account of that, shown, not checked.
- On Unix, ending one tool process (its session's cancel, a replaced `message` run) does not end what it started; that stays in the connection's process group and ends with the connection, unless it left the group.
- On Windows `program = "pi"` does not find npm's `pi.cmd`: name `node` with the script's absolute path, which keeps `cmd.exe` out, or `pi.cmd`.
- A tool entry is on `canoa-host tool`'s command line as JSON, which other users may read: nothing secret belongs in one.
- On Unix, what a tool starts in a session of its own (Node's `detached`) leaves the connection's process group and outlives it.
- Plain WebSocket on loopback carries only the proof and sealed records. Anyone on this machine can connect and make a proven key wait; only an accepted key gets further.
