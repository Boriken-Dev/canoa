# `canoa-host` — API header

Header-file-style reference for the `canoa-host` crate: every public item with its signature, contract and gotchas, so the crate can be used without reading its source. It ships inside the package and is self-contained. Development documentation lives with the source at <https://github.com/Boriken-Dev/canoa>.

No Cargo features. MSRV Rust 1.89. Version `0.0.0-alpha.0`, not published (`publish = false`): its git dependencies are not released. It depends on `canoa`, `clap`, `coqui-acp`, `coqui-acp-bridge` and `coqui-acp-provider` (coqui-acp `d64ea1a`), `coqui-agentic-peer`, `coqui-peer`, `coqui-peer-webcrypto`, `coqui-web` (`websocket`), `guatiao` (`provider`, `load`), `guatiao-serde` (no format), `indexmap`, `serde`, `serde_json`, `toml`, `tracing` and `tracing-subscriber`.

The root re-exports `Config`, `ConfigError`, `Entry`, `EntryKind`, `Origin`, `Settings`, `Peer`, `PeerState`, `PeersError`, `HostLine`, `ProviderError`, `ProviderOptions`, `serve_stdio`, `Listening`, `ServeError`, `ServeOptions`, `Serving` and `Stopper`. Public modules: `config`, `peers`, `provider`, `serve`.

No `unsafe` code (`#![forbid(unsafe_code)]`). Loading a provider library or source runs its code; see `serve_stdio`.

## Canoa.cfg (`canoa_host::config`)

The host reads only `[host]`; the app's tables (`desktop`, `network`, `assets`, `appearance`, `agents`) are not read. Every table the host reads refuses a key it does not know.

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
paths = ["/opt/guatiao/python"]       # and what goes on sys.path; relative paths are from the file's directory

[host.providers.NAME.settings]        # the provider's configuration, for one that is built from one
prefix = "hi"                         # checked against the provider's schema when a connection loads it

# Either table, each optional:
listen = "127.0.0.1:7801"             # loopback only; default 127.0.0.1:0, any free port
working_roots = ["/home/me/src"]      # absolute, no . or ..; a session's cwd must be under one; default any
peers = ["sha256:…"]                  # paired keys that may use it; default every paired key
max_peers = 4                         # connections at once
```

Names are 1 to 64 letters, digits, `-` or `_`, unique across both tables. Two entries may not name the same non-zero address.

```rust
pub const DEFAULT_MAX_PEERS: usize = 4;
pub const DEFAULT_MAX_SESSIONS: usize = 8;
pub const MAX_NAME: usize = 64;

#[non_exhaustive] pub struct Config { pub origins: Vec<String>, pub entries: Vec<Entry> }   // Default, Clone, Eq
pub fn Config::load(path: &Path) -> Result<Config, ConfigError>
pub fn Config::from_str_in(text: &str, base: &Path) -> Result<Config, ConfigError>   // touches no file
pub fn Config::entry(&self, name: &str) -> Option<&Entry>

#[non_exhaustive] pub struct Entry { pub name: String, pub kind: EntryKind, pub listen: SocketAddr, pub working_roots: Vec<PathBuf>, pub peers: Option<Vec<PeerId>>, pub max_peers: usize }
#[non_exhaustive] pub enum EntryKind { Agent { program: OsString, args: Vec<OsString> }, Provider { origin: Origin, provider: String, settings: Option<Settings>, directory: PathBuf, max_sessions: usize } }
#[non_exhaustive] pub enum Origin { Library(PathBuf), Source { file: PathBuf, settings: Settings } }   // paths absolute
pub struct Settings(..)                         // Clone, Default, Eq; Debug shows keys, never values
pub fn Settings::new(map: serde_json::Map<String, Value>) -> Settings
pub fn Settings::as_map(&self) -> &serde_json::Map<String, Value>
```

- `entries` keeps the file's order, every agent before every provider. A file with no `[host]` gives no entries.
- `library` and `source` are made absolute against `base` (the file's directory for `load`); `directory` is `base` made absolute, where the provider's process runs.
- `source_settings` and `settings` are TOML tables held as JSON. A date, or a float that is not a number, is refused; nothing checks them further until a connection loads the provider, since checking means loading its code.
- A leading byte-order mark is ignored; anything else not UTF-8 is a parse error.

## Serving (`canoa_host::serve`)

```rust
pub const MAX_LINE: usize = 16 * 1024 * 1024;
pub const MAX_UNPROVEN: usize = 64;
pub const PROOF_LIMIT: Duration = 15 s;
pub const FIRST_MESSAGE: Duration = 60 s;
pub const ACP_CAPABILITY: &str = "acp/1";

#[non_exhaustive] pub struct ServeOptions { pub provider_program: PathBuf }
pub fn ServeOptions::new(provider_program: impl Into<PathBuf>) -> ServeOptions

pub fn Serving::bind(config: &Config, identity: Identity, store: TrustStore, options: &ServeOptions) -> Result<Serving, ServeError>
pub fn Serving::listening(&self) -> &[Listening]
pub fn Serving::stopper(&self) -> Stopper      // Clone, Send: for another thread
pub fn Serving::run(self)                      // takes connections until the process ends
pub fn Stopper::stop_agents(&self)             // ends every agent process running, and each later one before it serves
#[non_exhaustive] pub struct Listening { pub name: String, pub kind: &'static str, pub url: String }   // kind: "agent" or "provider"
```

- `stop_agents` ends each agent with what it started (job object on Windows, process group on Unix), and turns every later proven connection away before its agent serves it. The listeners stay bound: end the process to free them.

- `bind` binds every entry before any is served, so a failure leaves none listening. A plain socket beyond loopback is refused by `coqui-web` too.
- Each connection: the WebSocket handshake, then the `coqui-peer-webcrypto` proof within `PROOF_LIMIT`, with the trust store deciding (a stranger that proved its key is recorded as waiting). Then the entry's `peers`, through a `canoa::Registry` holding one verified `Service` per entry and a grant of `ACP_CAPABILITY` per listed key. Every later message is sealed in the proof's session, which is checked against the store every second: a removed key loses its link.
- The first message must be plain ACP: a link of streams (`coqui_agentic_peer::mux`) is refused. Then a fresh process serves that connection, relayed by `coqui_acp_bridge::relay`, and is stopped with its process tree when either side ends.
- An agent entry runs `program args`. Every message from the client to it is parsed and the host's own serialization forwarded: `params` other than an object is answered `-32602`, since a list would fill a request's fields in order; `_meta` is removed at every depth; a request whose `params.mcpServers` holds anything but `http` or `sse` over `https`, or `acp`, is answered `-32602` and never reaches the agent; with `working_roots`, so is one whose `cwd` or `additionalDirectories` is outside them; what is not one JSON object is answered `-32700` or `-32600`. A refused notification is dropped.
- A provider entry is not filtered: its facade reads the client's `_meta` launch values (`context`, `model`, `effort`, an opaque `meta`) and hands them to the provider, and refuses an environment name. What a provider does with them is the provider's.
- A provider entry runs `provider_program provider (--library L | --source S) --provider ID --max-sessions N --directory D --settings-on-stdin [--working-root R]...`, so a library or source is loaded only in that connection's process, which runs in `D`. The command line holds paths and names only: the host writes the entry's `source_settings` and `settings` as the first line of the process's standard input (`HostLine`), ahead of anything the client sent. Its facade refuses any `mcpServers` and checks `working_roots` itself.
- `max_peers` connections per entry at once; past it a proven peer is turned away. Past `MAX_UNPROVEN` handshakes at once across entries, a new connection is dropped.

```rust
#[non_exhaustive] pub enum ServeError { NothingToServe, Listen { entry: String, failure: Failure } }
```

## One provider on standard streams (`canoa_host::provider`)

```rust
#[non_exhaustive] pub struct ProviderOptions { pub origin: Origin, pub provider: String, pub settings: Option<Settings>, pub working_roots: Vec<PathBuf>, pub max_sessions: usize }
pub fn ProviderOptions::new(origin: Origin, provider: impl Into<String>) -> ProviderOptions   // no settings, any cwd, the facade's default max_sessions
pub fn serve_stdio(options: &ProviderOptions) -> Result<(), ProviderError>

#[non_exhaustive] pub struct HostLine { pub source_settings: Option<Settings>, pub settings: Option<Settings> }   // Default, Eq
pub fn HostLine::to_line(&self) -> Vec<u8>                                           // one line of JSON, no line break
pub fn HostLine::read(input: &mut impl BufRead, max: usize) -> Result<HostLine, ProviderError>   // one line of at most max bytes
```

`serve_stdio` loads the code, takes the provider, and runs `coqui_acp_provider::AcpFacade` on standard input and output until the client closes them. Nothing in this process may write to standard output.

- **A library** is mapped with Guatiao's default import search: the library's own directory, the executable's, the system's; not the working directory or `PATH`.
- **A source** is added with `settings` as a Guatiao map, then the registry is refreshed once; a library the source refuses is logged and passed over. On Windows the import search is `ImportSearch::Path`: an import the default search leaves missing (`python3.dll`, for Python) is looked for again from the source's own directory, the system directories, **the working directory (Canoa.cfg's, under `serve`) and every directory on `PATH`**, in that order. Elsewhere the search is the platform's (`LD_LIBRARY_PATH`, the file's run path, the system's).
- **The provider**: with no `settings`, its own instance; a provider built only from a configuration is `NeedsConfiguration`. With `settings`, an instance built from them: Guatiao checks them against the schema the provider declares, then the provider may refuse them; a provider that is its own one instance is `TakesNoSettings`.

**Loading runs code** with this process's rights, before any check, and trusts whoever can write any file it reads or the directories holding them. For a library, that is the file. For `guatiao_source_py` it is also: the Python whose shared library the loader finds (on Windows the first `python3.dll` the search above meets, then the first `python3X.dll` it names); that Python's standard library, `site-packages` and `.pth` files; the `PYTHON*` variables of this process's environment, which it inherits from the host (`PYTHONPATH`, `PYTHONHOME`); every directory in `paths`; every manifest in `dirs` and `manifests` and every directory its own `paths` names; and every module those import.

`HostLine::read` reads up to the line break and leaves the rest to the transport. The line is `{"canoa_host": 1, "source_settings": {...}, "settings": {...}}`; anything else, a client's ACP message included, is `ProviderError::HostLine`.

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
canoa-host serve [--config FILE] [--until-stdin-closes]
canoa-host check [--config FILE] [--format text|json]
canoa-host id
canoa-host peers [--format text|json]
canoa-host accept PEER
canoa-host revoke PEER
canoa-host pair [--listen ADDR] [--timeout-seconds N] [--until-stdin-closes]
canoa-host provider (--library FILE | --source FILE) --provider ID [--working-root DIR]... [--max-sessions N]
                    [--directory DIR] [--settings-on-stdin]
```

- `serve` prints one JSON object per entry on standard output, `{"name", "kind", "url", "peer"}`, `peer` being this host's id, once every entry is bound; then it serves until stopped. It logs on standard error.
- `serve --until-stdin-closes`: once every entry is bound, standard input is read and discarded; when it closes, every agent is ended (`Stopper::stop_agents`) and the host exits 0. For a parent that runs the host as its child: the host ends with it, however the parent ends. Without the flag the input is never read.
- `pair` (`--listen` default `127.0.0.1:0`, loopback only, else status 2; `--timeout-seconds` default 300) prints `{"pin", "peer", "url"}`, a six-digit PIN, this host's id and the address to dial; a native client joins with `coqui_agentic_peer::pairing::join` (`coqui-acp-bridge pair-dial URL`). The joiner's key is accepted in the store once the PIN confirms both keys, and `pair` prints `{"paired": ID}` and exits 0. One wrong try ends it (status 1), as does the timeout, and with `--until-stdin-closes` its input closing. A pairing connection serves no agent. A WebSocket upgrade that carries an `Origin` header, which every browser sends, is refused (403) before the handshake, so a web page cannot spend the PIN's one try; a browser pairs by connecting to `serve` once and being accepted.
- On `serve --until-stdin-closes` and `pair --until-stdin-closes`, nothing is logged between the input closing and the exit, and the process exits within 5 s whatever is left: a parent that died can leave standard error a pipe a write to which never returns (seen on Windows under a Dart parent).
- `accept` and `revoke` take `--` before the id.
- `check` prints `NAME KIND LISTEN` per entry, or `{"name", "kind", "listen"}` with `--format json`, and serves nothing.
- `peers` prints `accepted ID` and `waiting ID [from ADDRESS]`, or `{"peer", "state", "source"}`.
- Canoa.cfg is `--config`, else `CANOA_CONFIG`, else `%APPDATA%\Canoa\Canoa.cfg` on Windows, `~/Library/Application Support/Canoa/Canoa.cfg` on macOS, `$XDG_CONFIG_HOME/Canoa/Canoa.cfg` (else `~/.config/…`) elsewhere: where the app keeps it.
- `provider` takes `--library` and `--source` from where it was started, then runs in `--directory`; with `--settings-on-stdin` it reads a `HostLine` first, as `serve` writes it.
- Exit status: 0 success; 1 the operation failed; 2 the invocation or Canoa.cfg was wrong, `serve` with nothing to serve, or a text that is not a peer id.

## Environment variables

| Variable | Effect | Default |
| --- | --- | --- |
| `CANOA_CONFIG` | Canoa.cfg to read, below `--config` | the platform's place, above |
| `CANOA_LOG` | what to log, a `tracing` filter | `info` |
| `COQUI_AGENTIC_PEER_HOME` | the directory of the identity (`identity`) and trust store (`peers/`) | `coqui-agentic-peer` under `%LOCALAPPDATA%`, `$XDG_CONFIG_HOME` or `~/.config` |
| `NO_COLOR` | no colour in the log | colour on a terminal |

## Gotchas

- The identity and trust store are the ones `coqui-acp-bridge serve --paired` uses: a key paired for one is paired for the other, and `canoa-host revoke` removes it from both.
- A stdio agent inherits the host's environment and working directory. An agent can still start whatever its own tools start when the client allows them; the host stops only what a client could start without the agent asking.
- A paired key reaches every session an agent keeps for this user: `session/list` and `session/load` pass to a stdio agent unchanged.
- `working_roots` compare paths as written: a link inside a root that points outside it is followed by the agent.
- If the host itself is killed, on Windows the system ends its agents with it (each is in a job object closed with the host); on Unix each sees its standard input close, and most then exit. `serve --until-stdin-closes` ends them on every platform when the parent goes.
- Each connection is a fresh process, so a turn in progress ends with its link; a later connection resumes a session only if the agent kept it.
- `peers` is read when the host starts; a key removed from the trust store loses its link within about a second without a restart.
- A provider's process inherits the host's environment. For a Python source that includes `PYTHONPATH` and `PYTHONHOME`, and on Windows `PATH`, which decides which `python3.dll` loads.
- Settings come from Canoa.cfg only. A client reaches a provider through ACP, never through its settings; `check` does not load the provider, so a setting its schema refuses shows only when a connection loads it, on the host's standard error.
- Plain WebSocket on loopback carries only the proof and sealed records. Anyone on this machine can connect and make a proven key wait; only an accepted key gets further.
