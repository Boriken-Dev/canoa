# `canoa-host` — API header

Header-file-style reference for the `canoa-host` crate: every public item with its signature, contract and gotchas, so the crate can be used without reading its source. It ships inside the package and is self-contained. Development documentation lives with the source at <https://github.com/Boriken-Dev/canoa>.

No Cargo features. MSRV Rust 1.89. Version `0.0.0-alpha.0`, not published (`publish = false`): its git dependencies are not released. It depends on `canoa`, `clap`, `coqui-acp`, `coqui-acp-bridge` and `coqui-acp-provider` (coqui-acp `5397c7d`), `coqui-agentic-peer` (`79a9f39`, which brings `coqui-peer-noise`), `coqui-peer`, `coqui-peer-webcrypto`, `coqui-web` (`websocket`), `guatiao` (`provider`, `load`), `guatiao-serde` (no format), `indexmap`, `serde`, `serde_json`, `toml`, `tracing` and `tracing-subscriber`.

The root re-exports `Config`, `ConfigError`, `Entry`, `EntryKind`, `Origin`, `Settings`, `Peer`, `PeerState`, `PeersError`, `HostLine`, `ProviderError`, `ProviderOptions`, `serve_stdio`, `Listening`, `ServeError`, `ServeOptions`, `Serving`, `Stopper` and `ToolSpec`. Public modules: `config`, `peers`, `profile`, `provider`, `serve`, `tool`.

No `unsafe` code (`#![forbid(unsafe_code)]`). Loading a provider library or source runs its code; see `serve_stdio`.

## Canoa.cfg (`canoa_host::config`)

The host reads only `[host]`; the app's tables (`desktop`, `network`, `assets`, `appearance`, `agents`, `overlays`, `profile`) are not read. Every table the host reads refuses a key it does not know.

```toml
[host]
origins = ["https://canoa.example"]  # optional: browser origins that may connect; empty or absent lets any, and the proof decides

[host.agents.NAME]                    # a program speaking ACP on its standard streams
program = "npx"                       # required; found as the OS finds a command
args = ["-y", "@agentclientprotocol/claude-agent-acp@0.99.1"]

[host.providers.NAME]                 # a Guatiao agent_provider, from a library or a source
library = "echo_provider.dll"         # a library file, relative to the file's directory; or:
source = "guatiao_source_py.dll"      # a source file, the same way; exactly one of the two
provider = "echo_provider"            # required; the provider's id
max_sessions = 8                      # sessions one connection may hold

[host.providers.NAME.source_settings]  # with source only; what the source is added with
dirs = ["python"]                     # for guatiao_source_py: directories of *.guatiao.toml manifests
paths = ["/opt/guatiao/python"]       # and what goes on sys.path; relative paths are made absolute from the file's directory

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
peers = ["sha256:…"]                  # paired keys that may use it; default every paired key
max_peers = 4                         # connections at once
```

Names are 1 to 64 letters, digits, `-` or `_`, unique across the three tables. Two entries may not name the same non-zero address.

```rust
pub const DEFAULT_MAX_PEERS: usize = 4;
pub const DEFAULT_MAX_SESSIONS: usize = 8;
pub const MAX_NAME: usize = 64;

#[non_exhaustive] pub struct Config { pub origins: Vec<String>, pub entries: Vec<Entry> }   // Default, Clone, Eq
pub fn Config::load(path: &Path) -> Result<Config, ConfigError>
pub fn Config::from_str_in(text: &str, base: &Path) -> Result<Config, ConfigError>   // touches no file
pub fn Config::entry(&self, name: &str) -> Option<&Entry>

#[non_exhaustive] pub struct Entry { pub name: String, pub kind: EntryKind, pub listen: SocketAddr, pub working_roots: Vec<PathBuf>, pub peers: Option<Vec<PeerId>>, pub max_peers: usize }
#[non_exhaustive] pub enum EntryKind { Agent { program: OsString, args: Vec<OsString> }, Provider { origin: Origin, provider: String, settings: Option<Settings>, directory: PathBuf, max_sessions: usize }, Tool { spec: ToolSpec } }
#[non_exhaustive] pub enum Origin { Library(PathBuf), Source { file: PathBuf, settings: Settings } }   // paths absolute
pub struct Settings(..)                         // Clone, Default, Eq; Debug shows keys, never values
pub fn Settings::new(map: serde_json::Map<String, Value>) -> Settings
pub fn Settings::as_map(&self) -> &serde_json::Map<String, Value>
```

- `entries` keeps the file's order, every agent before every provider, every provider before every tool. A file with no `[host]` gives no entries.
- A tool entry's `listen`, `working_roots`, `peers` and `max_peers` are read as in the other tables; the rest is a `ToolSpec`, refused as a whole when `ToolSpec::check` refuses it. Its errors name a setting and a rule's index, never a value.
- `library` and `source` are made absolute against `base` (the file's directory for `load`); `directory` is `base` made absolute, where the provider's process runs.
- In `source_settings`, every text in a `dirs`, `manifests` or `paths` list is made absolute against the directory, since a source may otherwise resolve it against a later working directory; other keys pass as written.
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
- The first message must be plain ACP: a link of streams (`coqui_agentic_peer::mux`) is refused. Then a fresh process serves that connection, relayed by `coqui_acp_bridge::relay`, and is stopped with its process tree when either side ends.
- An agent entry runs `program args`. Every message from the client to it is parsed and the host's own serialization forwarded: `params` other than an object is answered `-32602`, since a list would fill a request's fields in order; `_meta` is removed at every depth; a request whose `params.mcpServers` holds anything but `http` or `sse` over `https`, or `acp`, is answered `-32602` and never reaches the agent; with `working_roots`, so is one whose `cwd` or `additionalDirectories` is outside them; what is not one JSON object is answered `-32700` or `-32600`. A refused notification is dropped.
- A tool entry is served as an agent entry whose program is `provider_program` itself: `provider_program tool --name NAME --spec JSON`, the entry as `ToolSpec` serializes it. So the scope above applies to its clients, and the process, with every tool process it starts, ends with the connection.
- A provider entry is not filtered: its facade reads the client's `_meta` launch values (`context`, `model`, `effort`, an opaque `meta`) and hands them to the provider, and refuses an environment name. What a provider does with them is the provider's.
- A provider entry runs `provider_program provider (--library L | --source S) --provider ID --max-sessions N --directory D --settings-on-stdin [--working-root R]... --connect A`, so a library or source is loaded only in that connection's process, which runs in `D`. The command line holds paths, names and the address only. For each connection the host listens on a loopback port of its own, `A`; writes the entry's `source_settings` and `settings` and a one-time token of 32 random bytes as the first line of the process's standard input (`HostLine`), then closes it; and takes the first connection to `A` that sends the token within `START_LIMIT`, up to 8 that do not. ACP runs on that socket alone. What the process writes to standard output or error goes to the host's standard error, `OUTPUT_LIMIT` bytes per connection in all, the rest read and dropped after one warning; none of it reaches the client. The process and what it starts are ended with the connection, by job object on Windows and process group elsewhere. Its facade refuses any `mcpServers` and checks `working_roots` itself.
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
- **A source** is added with `settings` as a Guatiao map, then the registry is refreshed once; a library the source refuses is logged and passed over. On Windows the import search is `ImportSearch::Path`: when the default search leaves an import missing (`python3.dll`, for Python), the source is loaded again with Windows' standard search, and then every import it has, not only the missing one, is looked for in the source's own directory, the system directories, **the working directory (Canoa.cfg's, under `serve`) and every directory on `PATH`**, in that order. Elsewhere the search is the platform's (`LD_LIBRARY_PATH`, the file's run path, the system's).
- **The provider**: with no `settings`, its own instance; a provider built only from a configuration is `NeedsConfiguration`. With `settings`, an instance built from them: Guatiao checks them against the schema the provider declares, then the provider may refuse them; a provider that is its own one instance is `TakesNoSettings`.

**Loading runs code** with this process's rights, before any check, and trusts whoever can write any file it reads or the directories holding them. For a library, that is the file. For `guatiao_source_py` it is also: the Python whose shared library the loader finds (on Windows the first `python3.dll` the search above meets, then the first `python3X.dll` it names); that Python's standard library, `site-packages`, the user's own site-packages, and their `.pth` files, since the interpreter is not isolated; the `PYTHON*` variables of this process's environment, which it inherits from the host (`PYTHONPATH`, `PYTHONHOME`), and `GUATIAO_LIBRARY`, which picks the native library Guatiao's Python binding loads; on Windows, a `canoa-host._pth` or `pyvenv.cfg` beside the `canoa-host` executable, which the interpreter reads, so whoever can write that directory chooses where its modules come from; every directory in `paths`; every manifest in `dirs` and `manifests` and every directory its own `paths` names; and every module those import.

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

The schema of a profile, what an agent is launched with, and its merge in layers. The app reads the same schema; `tests/vectors/profile.json` holds both to the same views and refusals. `serve` does not apply profiles yet.

```toml
model = "m"                      # effort, mode likewise: text, not empty, no control character
context = { text = "..." }       # or { file = "CONTEXT.md" }: exactly one
unset = ["model"]                # model, effort, mode or context of a lower layer; not beside that key
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

#[non_exhaustive] pub enum LayerKind { BuiltIn, Overlay, User, Project, Agent, Session }   // Copy, Ord: lowest first
pub const fn LayerKind::as_str(self) -> &'static str      // "built-in", "overlay", "user", "project", "agent", "session"
pub const fn LayerKind::is_confined(self) -> bool         // Overlay and Project
pub struct LayerId { pub kind: LayerKind, pub name: String }   // Display: "user", "overlay:team", "agent:Claude"
pub fn LayerId::new(kind: LayerKind, name: impl Into<String>) -> LayerId

#[non_exhaustive] pub struct Profile { pub model, effort, mode: Option<String>, pub context: Option<Context>, pub unset: Vec<Scalar>, pub env: IndexMap<String, Change<EnvValue>>, pub skills: IndexMap<String, Change<Skill>>, pub mcp: IndexMap<String, Change<McpServer>> }   // Default
pub fn Profile::from_str(text: &str, kind: LayerKind) -> Result<Profile, ProfileError>    // touches no file, reads no variable
pub fn Profile::from_table(table: toml::Table, kind: LayerKind) -> Result<Profile, ProfileError>
#[non_exhaustive] pub enum EnvValue { Value(String), FromEnv(String), FromFile(String) }   // Debug never shows a Value's text
#[non_exhaustive] pub enum Context { Text(String), File(String) }
#[non_exhaustive] pub struct Skill { pub path: String }   pub fn Skill::new(path) -> Skill
#[non_exhaustive] pub enum McpServer { Stdio { command, args, env }, Http { url, headers }, Sse { url, headers } }
#[non_exhaustive] pub enum Change<T> { Set(T), Remove }
#[non_exhaustive] pub enum Scalar { Model, Effort, Mode, Context }

#[non_exhaustive] pub struct Layer { pub id: LayerId, pub dir: Option<PathBuf>, pub profile: Profile }
pub fn Layer::new(id: LayerId, dir: Option<PathBuf>, profile: Profile) -> Layer
pub fn merge(layers: &[Layer]) -> Effective
#[non_exhaustive] pub struct Effective { pub model, effort, mode: Option<Sourced<String>>, pub context: Option<Sourced<Context>>, pub env, skills, mcp: IndexMap<String, Sourced<_>>, pub removed: Vec<Removed> }
#[non_exhaustive] pub struct Sourced<T> { pub value: T, pub layer: LayerId, pub dir: Option<PathBuf> }
#[non_exhaustive] pub struct Removed { pub key: String, pub layer: LayerId, pub was: LayerId }
pub fn Effective::view(&self) -> serde_json::Value          // references as written, never resolved
pub fn Effective::environment(&self, lookup: impl Fn(&str) -> Option<String>) -> Result<Vec<(String, Secret)>, ProfileError>
pub fn resolve(key: &str, value: &EnvValue, dir: Option<&Path>, confined: bool, lookup: impl Fn(&str) -> Option<String>) -> Result<Secret, ProfileError>
pub struct Secret(..)   pub fn Secret::expose(&self) -> &str   // Debug "Secret(..)", Display "(hidden)"

#[non_exhaustive] pub enum OverlaySource { Git { repo: String, reference: String, path: String }, Dir(PathBuf) }
pub fn OverlaySource::parse_git(source: &str) -> Result<OverlaySource, ProfileError>   // <repository>@<ref>[#<path>]

#[non_exhaustive] pub enum ProfileError { Parse { line, column }, Invalid { key: String, rule: Rule }, Resolve { key: String, what: &'static str } }
#[non_exhaustive] pub enum Rule { Name, Path, Form, Value }
pub const fn ProfileError::code(&self) -> &'static str     // "parse", "name", "path", "form", "value", "resolve"
```

- **Merge**: lowest first. A scalar a later layer sets replaces the lower one; `unset` takes it away first. A named entry a later layer sets replaces the lower one whole and keeps its place; `Change::Remove` takes it out, and set again later it comes last. `removed` lists only what was there to remove, in order.
- **Paths** are judged on the text, the same on every platform. In a confined layer (overlay, project) a path is relative and stays below its directory: no leading `/`, `\` or `~`, no `:`, no `..` segment. Elsewhere any non-empty path without NUL.
- **Errors** never quote a value: `Invalid.key` names the setting (`env.A`, `mcp.a.headers.T`), or for a refused name the table holding it (`env`, `mcp.a.env`). The first refusal in a document is, in order: anything serde refuses (an unknown key, a wrong type, a key twice) as `Parse`; then `model`, `effort`, `mode`, `context`, `unset`, then `env`, `skills` and `mcp` entries in the document's order.
- **`resolve`** reads `FromEnv` through `lookup` and `FromFile` from `dir` (an absolute path only outside a confined layer), less one final `\n` or `\r\n`. For a confined layer the file and `dir` are canonicalized and the file must be under `dir`, so a link out of the project is refused. Call it only when a session starts.
- **`parse_git`**: the ref follows the last `@`, the path the last `#`. Refused: no ref; a repository that opens with `-`, holds a space or control character, names a transport helper (`::`), or a scheme other than `https`, `http`, `ssh`, `git`, `file`; a ref `git check-ref-format` refuses, or one opening with `-`; a path empty after `#`, absolute, holding `\` or `:`, or with a `..` segment. `Dir` is never parsed here.

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
canoa-host provider (--library FILE | --source FILE) --provider ID [--working-root DIR]... [--max-sessions N]
                    [--directory DIR] [--settings-on-stdin [--connect ADDRESS]]
canoa-host tool --name NAME --spec JSON
```

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
- A paired key reaches every session an agent keeps for this user: `session/list` and `session/load` pass to a stdio agent unchanged.
- `working_roots` compare paths as written: a link inside a root that points outside it is followed by the agent.
- If the host itself is killed, on Windows the system ends its agents with it (each is in a job object closed with the host); on Unix each sees its standard input close, and most then exit. `serve --until-stdin-closes` ends them on every platform when the parent goes.
- Each connection is a fresh process, so a turn in progress ends with its link; a later connection resumes a session only if the agent kept it.
- `peers` is read when the host starts; a key removed from the trust store loses its link within about a second without a restart.
- A provider's process inherits the host's environment. For a Python source that includes `PYTHONPATH` and `PYTHONHOME`, and on Windows `PATH`, which decides which `python3.dll` loads.
- Settings come from Canoa.cfg only. A client reaches a provider through ACP, never through its settings; `check` does not load the provider, so a setting its schema refuses shows only when a connection loads it, on the host's standard error.
- A tool runs with the host's rights and does unasked whatever its rules do not turn into `ask`: the host shows its tool calls as they run, never before. `unasked` is the entry writer's account of that, shown, not checked.
- On Unix, ending one tool process (its session's cancel, a replaced `message` run) does not end what it started; that stays in the connection's process group and ends with the connection, unless it left the group.
- On Windows `program = "pi"` does not find npm's `pi.cmd`: name `node` with the script's absolute path, which keeps `cmd.exe` out, or `pi.cmd`.
- A tool entry is on `canoa-host tool`'s command line as JSON, which other users may read: nothing secret belongs in one.
- On Unix, what a tool starts in a session of its own (Node's `detached`) leaves the connection's process group and outlives it.
- Plain WebSocket on loopback carries only the proof and sealed records. Anyone on this machine can connect and make a proven key wait; only an accepted key gets further.
