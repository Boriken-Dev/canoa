# `canoa-host` CLI library, network, spawner, stores, and KB

## The command line as a library (`canoa_host::cli`)

```rust
pub const STATUS_HELD: u8 = 3;
pub const STATUS_RESTART: u8 = 75;   // `serve` ended because a key granted `host.restart` asked it to: start it again
pub trait Front: Send + Sync {
    fn print(&self, line: &str) -> io::Result<()>;     // a line of results: standard output for `canoa`
    fn say(&self, line: &str);                         // a line of diagnostics: standard error for `canoa`
    fn when_input_closes(&self, status: u8, then: Box<dyn FnOnce() + Send>) -> io::Result<()>;   // for --until-stdin-closes
    fn program(&self) -> io::Result<Program>;          // what runs provider, tool and launch for this host
    fn home(&self) -> Option<PathBuf>;                 // None: this user's
    fn holder(&self) -> &str;                          // what serve calls itself to another host
    fn ask(&self, prompt: &str, secret: bool) -> io::Result<Option<String>>;   // one typed line, a secret without echo; Ok(None): this front cannot take one (an application asks in its own forms)
    fn local(&self, local: Local) {}                   // default: ignored. serve hands over its way in, bound and before "ready"
    fn programs(&self) -> Option<(Arc<Programs>, Vec<String>)> { None }   // default: the built-in stores over this user's programs folder; a front with stores of its own serves those through store/*
}
#[non_exhaustive] pub struct Program { pub path: PathBuf, pub args: Vec<OsString> }   // args go before the subcommand
pub const fn Program::new(path: PathBuf, args: Vec<OsString>) -> Program
pub struct Cli;                                        // clap::Parser
pub const fn Cli::is_a_hosts_own(&self) -> bool        // provider, tool, agent, launch, mcp-bridge, helper, hook-forward or (Unix) spawner
pub fn run(cli: Cli, front: &dyn Front) -> Result<(), Failed>
pub struct Failed { pub status: u8, pub why: String }
```

Everything `canoa` does is here; `src/main.rs` is a `Front` on its own standard streams, and `canoa-ffi` another. `run` neither exits nor writes anywhere but through the front. With a `Front` that returns from `when_input_closes`, `serve --until-stdin-closes` returns once it has stopped, its lock given up.


## The network (`[host.network]`, `canoa::network`)

Every outbound HTTP(S) call the host makes builds its client with `canoa::network::Network::agent` (the process's network is `Network::current()`, installed by `Serving::bind` from `[host.network]`; `HttpTransport::on(network)` takes another, for a test).
The call sites: `store::tools::HttpTransport` (every Store download and release listing: `LocalTools`, the program, runtime and shell stores, the Foundry Local store; `registry/fetch`; `control::extension_install`), `kb_mcp` (`embedder::NetworkEmbedder` for `[host.kb] embeddings`, `rank::Ranker` for `llm_endpoint`: a search made for a connection whose profile has a proxy item goes through that session's network, `Access::network`, and an index is made once for every connection that applies the overlay, so its documents are embedded through the process's), `canoa_foundry::daemon::DaemonEngine` (the Foundry Local service, so the gateway's upstream and the Foundry account check; a chat call to an address the network sends direct, which the loopback service is unless `[host.network]` says otherwise, is made by a `ureq` agent of its own, since only that agent's sockets can be closed to end a call whose client went away), and the speech endpoints (`speech_endpoint`, `speech_say_endpoint`, in `canoa`).
Git for an overlay or a skill source (`control::overlays`, the one place git reaches a remote) starts with `-c http.sslBackend=schannel` on Windows (unless `git_tls = "git"`) and is given the proxy for the remote named in its arguments. The Foundry Local SDK engine (`canoa-foundry` feature `sdk`) keeps its own client, which reads the proxy variables of the process.

Every process the host starts through `spawn::Command` is given `HTTP_PROXY`, `HTTPS_PROXY` and `NO_PROXY` (lower case too off Windows) from the network when the environment it starts with holds no `http_proxy`, `https_proxy` or `all_proxy` and does not say `NO_PROXY=*` (what a session whose profile's proxy item is `none` is given): variables the person or a caller set are never replaced, and a name already set is left alone. A proxy chosen per URL (PAC, WPAD) is asked about the addresses the loaded libraries say (`network::program_endpoints`: every Store program's `endpoints` and `services`, and every agent vendor's `endpoints`, from the bundled libraries and the ones canoa.cfg loads) plus `[host.network] endpoints`, or about the URLs `Command::reaches(urls)` names; the variables are kept for a minute. `Command::network(network)` gives it another network. The variables carry the proxy login in the proxy URL.

A proxy (`network::session`, run for a profile's proxy item in `system` or `pac` mode) is shared: `Registry` (`registry()`) keeps one `Running` proxy for each distinct `ProxyKey` (mode, address, bypass, login, each pattern with its certificate, and the `[host.network]` installation it follows), with its own secret; `middleman` takes a `Lease` on the one that runs or starts it, a session's `SessionNetwork` holds the lease, and the proxy ends when the last lease is dropped (or `proxy_idle_seconds` after, a timer that an epoch keeps from ending one a later session took).
It ends TLS for the destinations the item's `client_certificate_hosts` entries match and for no others: `Intercept` compiles each entry (`canoa::profile_item::HostPattern`: a plain host, `*.name` or address as written, anything else a regular expression matched whole), `rule_for(host)` is the first entry that matches a CONNECT's host, and `ended_here` makes the TLS to the destination first with `Network::secure_tunnel_as` (the network's own trust and, per the entry, `system`'s choice or the certificate with its thumbprint; a failure is the program's `502`), answers the program's TLS with the certificate `ProxyCa::answer` made for that host and carries the decrypted bytes on one polling thread. `ProxyCa` (`network/proxy_ca.rs`) is the process's authority, held only in memory: its key is written nowhere and goes with the process, and nothing of it is kept in the home.
`ProxyCa::shared(hosts)` returns the one for that set of hosts (made the first time; `None`, which `Intercept::for_entries` asks for when any pattern is a regular expression, is the one with no limit), and `answer` makes a certificate for a destination when first asked and keeps it, by name, until a day before its 30 days end. An authority for hosts carries a critical `nameConstraints` that permits exactly them (a name or `*.name` as that DNS name, which a DNS constraint takes with everything under it, so a plain `name` also covers its subdomains; an address as a one-address `iPAddress`) and excludes every address family and DNS name the item does not name (`canoa.invalid` stands for "no DNS name"), so a certificate it signs for another host fails any client's validation, a leaked bundle included.
A session's programs are given the authority's certificate (alone, and with `rustls-native-certs`' roots) in files of the connection's private directory (`trust_files`), named by `SSL_CERT_FILE`, `NODE_EXTRA_CA_CERTS`, `REQUESTS_CA_BUNDLE`, `CURL_CA_BUNDLE`, `GIT_SSL_CAINFO`, `PIP_CERT`, `CARGO_HTTP_CAINFO` and `NPM_CONFIG_CAFILE`; the key is in none of them, and no store of the system's is written. `network::installed_settings()` is what `[host.network]` made the process's network from, which a session's own network (proxy, pac, command) starts from.

## Starting processes (`canoa_host::spawn`)

```rust
pub enum Stdio { Inherit /* default */, Null, Piped }
pub struct Command;   // std's, with what the host uses: new, arg(s), env, envs, env_remove, current_dir, stdin/stdout/stderr(Stdio), own_group(), reaches(urls), network(Arc<Network>), spawn(), output(); Windows: creation_flags
pub struct Child { pub stdin: Option<ChildStdin>, pub stdout: Option<ChildStdout>, pub stderr: Option<ChildStderr> }   // id, try_wait, peek, wait, kill, end_group
pub struct Agents;    // coqui_acp_bridge::agent::Spawner: how an agent's process is started
pub fn install(program: Program)           // what runs the starter; the latest named wins
pub fn install_default(program: Program)   // the same, unless one has been named
pub fn prepare_starter(prepare: fn(&mut std::process::Command))   // changes the command that starts the starter (canoa-ffi puts SIGCHLD back to the default in it)
pub fn serve() -> io::Result<()>   // Unix: what `canoa spawner` runs
```

Every process the host starts for itself (agents, providers, tools, `git`, the version of a program) is started through `spawn::Command`, which is std's unless `install` has been called, as `canoa-ffi` does for every job on Unix. Then each is started by `program args... spawner`, a hidden command that serves a socket on its standard input: the host sends a program, arguments, its whole environment, a directory and the three standard streams' descriptors (`SCM_RIGHTS`), and hears when a child ends, with its wait status. The reason: an application such as Flutter's collects every child of its process that ends, so the host's own wait fails with no child process and its status is lost. A child of the starter is collected only when the host asks (`wait`, a successful `try_wait`, dropping the `Child`), so `peek` and `end_group` still see an ended process and its group as its own.
`end_group` ends the process group a `own_group()` child leads and does nothing for one already collected, since the group's id may be anyone's then (`end_group_after_exit` is for the moment after `peek` or `try_wait` has said it ended, when the group's id is still held by what is left in it); `peek` leaves a child of the starter, and on Linux a child of this process, uncollected, and an error from it or from `try_wait` is a status that cannot be read, never a still-running process; `kill` ends the process, with no error for one that has ended. If the starter ends, the children it held read as an error (`the host's process starter has ended`), no new child starts from it, and the next one starts a new starter. When the host's end of the socket closes, however it closed (the application crashing included), the starter ends every child it still holds, each by its group (with what left the group by `setsid` on Linux, found by parent through `/proc`) or by itself, and any child it starts after that is ended as it starts: every kind of child ends with the host.
A frame that is not one (a count of descriptors that does not match, more than three, a body that stops short or passes 64 MiB, a kind unknown) ends the starter, and the host reads that as the starter lost; a spawn that gets no answer in 60 s fails alone with `TimedOut`, the link goes on, and the starter ends the child it starts for it late (the host asks for it to be ended if it hears of the child first). A starter whose thread for watching a child cannot start ends and collects the child and answers the spawn with the error. A child whose end the starter could not read (`SIGCHLD` ignored) reads as an error, and the starter signals no group of it. Windows and a process with no `install` start every child as std does, an agent in its job object (a child that has already ended when it would join has nothing to contain and starts as usual; any other refusal fails the start with the system's own error text). `Agents` is what `Agent::spawn_with` takes. Overlay hooks go through it too, and a hook whose status cannot be read is a failed run that refuses, never a timeout (`HookRun::on_failure` is `refuse` for it).
So do the processes `coqui-acp-harness` starts (an account's sign-in, the store's installers, a harness's model list, the macOS port search, the in-process Claude, Codex and Pi agents) and the commands `coqui-acp-secrets` runs (a secret in their environment; `install` also calls its `set_starter`): `install` also calls its `set_starter`, and they run without that crate's watcher shell and its search for descendants that left the group (the starter ends the group while it holds the process uncollected).

## Program stores (`canoa_host::store`)

The host's end of `canoa::store`'s `program_store` kind: it finds, checks, installs and switches the agent programs Canoa runs, through
stores that know each program. Nothing runs at startup, and the host holds no program name, command or URL: those are the stores'.

- `shelf::Shelf` (`<programs>/<program>/{lock, current.json, versions/<version>-<nonce>/}`): `Shelf::default_root()` is
  `%LOCALAPPDATA%\Canoa\programs`, `~/Library/Application Support/Canoa/programs` or `$XDG_DATA_HOME/Canoa/programs`. `ProgramDir::lock`
  takes the program's OS lock, refuses a folder another user owns or others can write (Unix; on Windows it inherits `%LOCALAPPDATA%`'s access
  list, which is not read: no vetted crate checks an owner or an access list, and this crate's only `unsafe` reads the process table, so on Windows the folder is only
  refused when it is a link) and removes the version folders `current.json` does not name. `Locked::stage`, `commit` (one rename of
  `current.json` is the atomic switch; the old copy becomes `previous`; folders other than current and previous are pruned, `keep_versions`
  default 2), `abandon`, `rollback`, `remove`. `commit` refuses a launch path that is not a regular file inside the staged folder.
- `footprint::{Watch, Footprint, uncovered}`: before and after snapshots of profile files, `PATH` directories, the home's top level
  (and, in the suite, the whole sandbox) and, on Windows, `HKCU\Environment`, `Run`, `RunOnce` and the `Uninstall` names. `uncovered` is
  what no declared `Change` covers; `restore_registry` puts back the values of `HKCU` `Environment`, `Run`, `RunOnce`, the policy `Run` and the command processor's key (Windows).
- `tools::LocalTools` (`InstallTools`): HTTPS only (loopback `http` under `loopback_http`, a test's setting); every redirect hop's origin must be
  one of the store's `endpoints`; `ToolsLimits` (5 redirects, 60 s to a response, 30 min a body, 100 000 files, 4 GiB unpack) each a
  setting; the digest is computed as the bytes are written. `Transport` is the network seam (`HttpTransport` is `ureq` through `canoa::network::Network`: the system's proxy and its login, the operating
  system's verifier and store, so a TLS-intercepting proxy the system trusts works, and the keystores' client certificate; `[host.network]` overrides each).
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
- `registry::built_in()`: `PiperStore` (in `canoa-stores`) and the bundled libraries registered from `bundled::ALL` and `bundled::INCLUDED` (`canoa-{node,python,git-bash,powershell,bitwarden,foundry,litellm,openai,whisper}`, `canoa::api_gateway`): `registry::agent_stores` lists the agent store ids and `store::runtimes` derives the OS-filtered runtime stores, `PATH`-exposed runtime programs (`!commands.is_empty()`), command-to-runtime offers and off-`PATH` extension programs (`secret_programs`) from registered `StoreInfo` and `StoreExtension` descriptions, so an agent or store added to `bundled.rs` needs no host table. The engine that runs a `Recipe` (`canoa::store::engine::Engine`), the process spawner
  `canoa::store::exec` and the protected-paths matcher `canoa::store::protect` are in `canoa`; `canoa::store_provider!` makes a provider of a recipe. Each vendor crate
  (`canoa-claude`, `canoa-codex`, `canoa-grok`, `canoa-antigravity`, `canoa-opencode`, `canoa-pi`) carries that agent's recipe and store provider, its source
  preset (`bundled.rs` lists them) and, for Claude, Codex and Pi, re-exports the coqui-acp provider `kind.rs` registers. `canoa::store::recipe::Recipe` and its
  helpers are the contract a recipe is written against. A recipe names the
  publisher's documents and the version's shape; the engine downloads, unpacks, runs the recipe's extra step and reads the version back.
  Claude: one binary, SHA-256 from the manifest (its signature is not checked). Codex: `codex-package` tar.gz, SHA-256 from the release's
  checksums and the channel's digest; no switch stops its updater (`self_updates`). Grok: one binary, no checksum published, so `Unverified`.
  OpenCode: one-file zip or tar.gz from GitHub's releases, SHA-256 from the release's own `digest` for the file; GitHub's newest release, or a named version from its tag.
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
- Downloads nothing checks (Grok's) are refused unless the program's scope in canoa.cfg has an enabled catalog entry that says `trusted = true` (`Programs::trust`); `current.json` records each file's `verified`. A file is checked as it is written by `Check::Sha256`, `Sha512` or `GitBlob` (the SHA-1 of `blob <size>`, a NUL and the file, computed over the written file: the id is believed because the catalog's address names the commit). Endpoints may be a prefix
  (`https://storage.googleapis.com/antigravity-public/antigravity-cli/`), which a URL and every redirect has to start with.
- `Programs::launch` reads `current.json` against a digest the host keeps in `<programs>/.digests/`, outside the program's folder, refuses a launch
  outside the copy or a copy outside the shelf, and, with `Options::verify_copies` (`[host.store] verify_copies`, off by default), checks every
  file of the copy (the files an archive unpacked too) against what was recorded at install: a changed, missing or added file refuses. Off, only
  the launch file must exist. `ProgramDir::verify` is that check and does not look at the option; it hashes the files on every core and, where
  the system keeps an inode's change time (Unix), leaves `.digests/<program>.checked` so a copy none of whose files has been written since is not read again. Records survive a rollback. A person with write access to the programs folder can rewrite
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

## The knowledge tools (`[host.kb]`, `[overlays.NAME] kb`)

```rust
pub const KB_META: &str = "dev.boriken.canoa/kb";   // the `_meta` key of `initialize` that asks for the tools
```

**Tools a program offers** (`client_tools`, `@tools`). A program on this computer, such as the browser extension (or an Office pane, through the web server's `/office/tools/` routes under `office_local`), opens `@tools` on a proven link and registers a name (`{"type":"register","provider":"browser"}`, a lowercase word of at most 32). From then on every session the host starts for any connection (every key that reaches the host is the person's own; a task another agent handed out is the one exception) is given one MCP server for that provider, `canoa-<name>` (`canoa-browser`), the way `canoa-kb` is given: over loopback HTTP behind a bearer minted for that session, or for an agent that takes only stdio through `PROGRAM mcp-bridge`; no `_meta` is needed from the client. The host opens a stream on the provider's link for each session and forwards that session's MCP requests (`tools/list`, `tools/call`) on it, so the provider's grants and windows belong to that session alone and end with it (its connection, a `session/close`, or the provider's link). `initialize` and `ping` are answered by the host, which says the tool list may change.
A `GET /mcp` with the bearer is the session's stream of server messages (`text/event-stream`), where the provider's `notifications/...` and the host's own `notifications/tools/list_changed` (a provider connected or left) reach an agent that takes HTTP servers; a stdio agent hears none and lists again when it chooses. Wire, one JSON object a message. Provider to host: `register`, `{"type":"mcp","stream":S,"message":M}` (an answer, or a `notifications/` message; a message for a stream this connection was not given is ignored) and `{"type":"close","stream":S}`. Host to provider: `registered` or `{"type":"refused","why"}`, `{"type":"open","stream":S,"session":{"agent":ENTRY,"via":"app"|"paired","peer"?}}` (what the extension's window tells the person: the entry the session runs on and whether it came from the app in this process or from a paired key), `{"type":"mcp","stream":S,"message":M}` and `{"type":"close","stream":S}`.
A session gets servers for the providers connected when it starts and for those seen since the host started, so one outlives a browser restart: with no provider connected `tools/list` is empty and a call says the provider is not connected, and a provider that returns is opened a new stream, with none of the old grants. A name held by one key is refused to another; the same key replaces its own connection. A session request a web page made (`_meta["dev.boriken.canoa/site"]` naming a site and not `own`) is given none: the connection's raw messages are read before the scope removes that mark (`ToolsPeek`), and `ToolsLayer` adds the servers after the scope. An answer waits up to 190 seconds for the provider, which may ask the person. Limits: 256 sessions a connection, 1024 streams and 16 waiting calls a stream, messages of at most 4 MiB, 16 provider names.

An agent can search and read the knowledge of the overlays its session has through an MCP server the host adds to its sessions, `canoa-kb`, offered the way `canoa-store` is: the client sets `_meta["dev.boriken.canoa/kb"] = true` in `initialize`, and each session request gets one server over loopback HTTP behind a bearer of that session (or, for an agent that takes only stdio, through `PROGRAM mcp-bridge`), which ends with the connection. A session that gets it also gets a built-in skill, `canoa-kb`, that says when to search. A session sees the overlays its connection applies (what its loadout gave it) and nothing else. The tools only read.

The tools, their arguments, answers, error classes, templates and limits are the Agent Overlay Specification's (`spec/v0/knowledge.md`) and coqui-aos's `KnowledgeSession` answers them; see its `rust/AGENTS.md`. The host adds:

- **Tools.** `kb_list`, `kb_read {path, from?, to?, overlay?, context?}` and `kb_search {query, top?, overlay?, mode?, path?}`; `kb_search` is offered because the host always has `words`. `mode` is `auto`, or one the host declares in `kb_list`: `words`, `meaning` (with `[host.kb.embedder]`) and `llm` (with `[host.kb.ranker]`). A call waits up to 20 seconds for an index still being made and then says so (`unavailable`).
- **Who is asking.** A template of a knowledge file is rendered with the connection's profile (`AOS_CALLER_PROFILE`), its loadout as the role (`AOS_CALLER_ROLE`), the session's own variables (`AOS_ENV_NAME`; AOS never gives a variable that changes how programs are found), the overlay's name and the person's option values, and the call's `context`. The searchable text is the served text with no caller.
- **Where the indexes are.** `<state>/kb/<overlay>/<tree digest>/<words | m-MODEL>/` (`coqui_aos::index`), made in the background the first time a version is asked for and shared by every connection that applies it. A new version starts from the last one's index and makes only the files that changed again; the versions before it are then removed. A change to `[host.kb]`'s sizes or model remakes the index. With no state directory the tools are off.
- **Meaning needs a backend the person names.** With no `[host.kb.embedder]` an index holds words only (BM25). `use` is a backend id of the host's `assist` service (`openai:work`, an account's API; `local`, an `[sdk.NAME]` server by address) or `default`, which is what `[assist.embed]` chooses (its `use`, then its `fallback`, else the first backend that embeds); `model` is the exact model (required; saved with each index as `BACKEND/MODEL`, with `@LENGTH` when `dimensions` is set, so vectors of two services never mix); `document_prefix` and `query_prefix` belong to the model. Every passage of the knowledge of every overlay a session applies goes to that backend, once per version and again for what changes, and each query too (the last 256 query embeddings are remembered, `query_cache`). A backend that fails while indexing leaves the version a words index with a note, and is left alone for `retry_seconds` (default 60) so a dead server costs one request. Vectors are asked through the process's network for the index and the connection profile's for a query.
- **The `llm` mode.** `[host.kb.ranker]`: `use` as the embedder's (`default` is `[assist.rerank]`'s choice; with no backend that reranks, `[assist.complete]`'s chat model is asked instead), `model` (required), `candidates` (default 10, 1 to 100), `timeout_seconds` (default 30) and `passage_chars` (default 600). The search finds candidates by `auto` (meaning where there is an index of it, else words) and the backend orders them: a backend that reranks is asked `{model, query, documents}`; a chat model is given the query and each passage as `[n] path > heading: text` cut to `passage_chars` and asked for "only a JSON array of the passage numbers", read as numbers only, so a passage that gives an instruction changes at most the order. What the model left out follows in the order found, and scores are replaced by rank. An ordering is remembered for five minutes (the same search from two sessions asks once). When it cannot be had, a search that named `llm` is refused (`mode_unavailable`, with the reason) and an `auto` one is answered by the mode that found the candidates, with the reason in `note`. `[host.kb.search] use` names a knowledge provider that does the whole search; none is loadable yet, so the answers say the host's own index searched.
- **Served through the tools.** `[overlays.NAME] kb = "index"` leaves that overlay's declared rules and context files out of the agent's context and puts one line in their place saying to search them with `kb_search`; `"files"` keeps them whole; with no key the overlay's own `[knowledge] serve` hint decides (`coqui_aos::knowledge_plan`). It applies only to a connection that was given the tools (a client that did not ask keeps the whole files). `HostProfile::kb_choice` holds the person's keys.
- **What an agent reads is data.** coqui-aos writes control, direction, zero-width and other hiding characters in passages, headings, paths and errors as `<U+XXXX>`.

### The releases a program store reads, and where its files may come from

`LocalTools::releases` answers a recipe's `install_tools.releases` with what `Programs`' reader makes of the program's catalogs: `Stores::trust_from` (run with the `[store]` section on every Store call) installs the reader, which `Stores::releases_of` implements by reading each enabled catalog of the scope (`store/catalog_run`: the cache, the shipped snapshot, else the entry's source) and merging them into one `canoa.releases/1` document (the first catalog that names a channel or lists a version wins; a catalog that cannot be read is left out, and why is the answer only when none could be). A program's catalogs are the ones its `StoreInfo::catalogs` registers, so its scope is its program id, which a Store type of the same id would take: Piper's program, which has no source, is the case.

A catalog entry that is `trusted` or pinned may name further `https` origins its files are downloaded from with `endpoints` (`Entry::endpoints`; any other entry that names some is listed with the reason): they are added to the program's endpoints for the downloads of that scope (`Programs::download_from`), and to a model row's for the models of a type (`FromCatalogs::endpoints`). A model row whose file is outside the type's endpoints and its entry's is refused by name before anything is fetched.
