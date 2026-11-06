# The host

`canoa` serves agents to Canoa's chat on this machine. Each entry of canoa.cfg's `[host]` table is one agent, on a
loopback WebSocket of its own:

- **A stdio ACP agent**: any program that speaks the Agent Client Protocol on its standard streams, such as Claude's
  adapter, Codex's adapter or `gemini --acp`.
- **A Guatiao provider**: coqui-acp-provider's `agent_provider` kind, served through its ACP facade. It comes from a
  library file, written once in any language Guatiao builds a library from, or from a Guatiao source: a file that runs
  libraries written in another language, such as Python, Lua, JavaScript, Rhai or WebAssembly.
- **A command-line tool with no ACP**: a program that takes a prompt and writes JSON lines, such as Pi. The entry says how
  a prompt goes in and what each line becomes in ACP; the host has no code for any one tool. See
  [A command-line tool](#a-command-line-tool).

The API and every setting are in the crate's [API header](https://github.com/Boriken-Dev/canoa/blob/main/crates/canoa-host/AGENTS.md).

## Configuration

```toml
[host.agents.claude]
program = "npx"
args = ["-y", "@agentclientprotocol/claude-agent-acp@0.99.1"]
listen = "127.0.0.1:7801"
working_roots = ["/home/me/src"]
worktrees = true                  # a git worktree per session, inside working_roots; see Worktrees

[host.providers.echo]
library = "echo_provider.dll"
provider = "echo_provider"
listen = "127.0.0.1:7802"
```

### A provider written in Python

Guatiao's `guatiao_source_py`, built against a Python, serves Python modules as libraries. One entry names the source
file, the provider, and the source's settings:

```toml
[host.providers.python]
source = "guatiao_source_py.dll"
provider = "acp_echo_echo"
listen = "127.0.0.1:7803"
import_search = "library"                                   # optional, Windows: library or path; see below

[host.providers.python.source_settings]
dirs = ["python/echo"]                                     # directories of manifests, each <name>.guatiao
paths = ["python/guatiao", "python/coqui_acp_provider"]    # what goes first on sys.path: Guatiao's binding, the kinds
isolation = "shipped"                                      # optional: shipped (the default), isolated or inherit
home = "python/runtime"                                    # optional: the Python installation it runs from
import_dirs = ["python/runtime"]                           # optional, Windows: where python3.dll is found
```

`library` and `source` exclude each other, and `source_settings` goes with `source` alone. Relative paths in `library`,
`source`, `home` and the `dirs`, `manifests`, `paths` and `import_dirs` lists are made absolute from canoa.cfg's directory
when the file is read; the provider's process runs in that directory, so any other relative path in `source_settings` is
taken from there too. The source reads the rest; it refuses a value it does not take when a connection loads it.

A manifest is named `<name>.guatiao`, or `<name>.guatiao.<major>[.<minor>]`; a file named `*.guatiao.toml` is not read. It
is TOML unless its first line is `#!json` or `#!yaml`.

`isolation` decides how much the interpreter takes from outside these settings:

| `isolation` | What it reads | What it ignores |
| --- | --- | --- |
| `shipped` (the default) | what ships with the program: a `._pth` or `pyvenv.cfg` beside `canoa`, on Windows `canoa`'s own directory, `home`, the installation's `site-packages`, `paths` | `PYTHONPATH`, `PYTHONHOME` and every other `PYTHON*` variable, the user's own `site-packages` |
| `isolated` | the standard library of `home` (else of the installation `python3.dll` came from) and `paths`, nothing else | everything beside `canoa`, the installation's `site-packages`, the environment |
| `inherit` | the environment as well, as a `python` started from the host would | nothing |

`inherit` is weaker: whoever sets the host's environment chooses the code a library runs. `canoa serve` says so in its
log when it starts, once per entry: `weakened: host.providers.NAME.source_settings.isolation is inherit, looser than
shipped`. The first source a process adds decides `isolation` and `home`; each connection is a process of its own.

What a provider prints, to standard output or standard error, goes to the host's log, up to 1 MiB per connection, and
never to the client: the host and the provider's process speak ACP over a loopback socket of their own, which the process
proves with a one-time token. Its standard input is closed once the settings are read.

On Windows the Python's `python3.dll` has to be found: name its directory (`sys.base_prefix`) in `import_dirs`. The
entry's `import_search` says where else it may come from: `library` looks only in `import_dirs` (after the source's own
directory, `canoa`'s and the system's); `path` also in the Windows directory, the provider's working directory
(canoa.cfg's) and every directory on `PATH`, so whoever can write any of those chooses the Python that loads. Without
`import_search` an entry whose `import_dirs` names a directory takes `library`, and one that names none takes `path`,
which keeps a Python on `PATH` working. `canoa serve` logs every entry on `path` when it starts:
`weakened: host.providers.NAME.import_search is path, looser than library`, with how to narrow it. On Linux and macOS the
source links that Python's shared library, found where the loader looks, or through `LD_LIBRARY_PATH` or
`DYLD_FALLBACK_LIBRARY_PATH`; `import_dirs` and `import_search` change nothing there.

### A provider built from settings

A provider that declares a configuration has no instance until it is given one. `settings` gives it:

```toml
[host.providers.prefixed]
library = "echo_provider.dll"
provider = "prefixed_echo"

[host.providers.prefixed.settings]
prefix = "hi"
```

The table is checked against the schema the provider declares when a connection loads it, since checking means running
its code; a misspelled or ill-typed setting fails that connection, and the host's log names the setting, never its value.
A provider that is its own instance refuses `settings`.

Settings are read from canoa.cfg only, never from a client. They reach the provider's process on its standard input, not
on its command line, which other users of the machine may be able to read.

### A command-line tool

A tool that has no ACP of its own, and no adapter, is an entry of `[host.tools]`: its program, how it keeps a
conversation, how a prompt reaches it and a list of rules that turn each JSON line it writes into ACP. Use an ACP agent or
an adapter when there is one: it knows the tool, and these rules only know the lines.

`examples/config/pi.cfg` is a whole working entry, for [Pi](https://pi.dev) (`@earendil-works/pi-coding-agent`, MIT), the
one the tests run:

```toml
[host.tools.pi]
program = "pi"                              # on Windows pi.cmd, or node with Pi's dist/bundle/cli.js first in args
args = ["--mode", "rpc", "--no-session", "-e", "/abs/path/to/examples/extensions/permission-gate.ts"]
runs = "session"                            # one process holds the conversation
unasked = "Pi runs its read, bash, edit and write tools with your rights. ..."
prompt = { type = "prompt", message = "{prompt}" }
cancel = { type = "abort" }

[host.tools.pi.kinds]
bash = "execute"

[[host.tools.pi.events]]
when = { type = "message_update", "assistantMessageEvent.type" = "text_delta" }
send = "text"
text = "{assistantMessageEvent.delta}"

[[host.tools.pi.events]]
when = { type = "agent_settled" }
send = "end"

[[host.tools.pi.events]]
when = { type = "extension_ui_request", method = "select", "options.0" = "Yes", "options.1" = "No" }
send = "ask"
title = "{title}"
allow = { type = "extension_ui_response", id = "{id}", value = "Yes" }
reject = { type = "extension_ui_response", id = "{id}", cancelled = true }
```

- **`runs`**: `session` starts one process when a session opens, and writes each prompt to its standard input as the
  `prompt` line; `cancel`, when given, is written on `session/cancel`, and a turn not ended `CANCEL_GRACE` (10 s) later
  has its process ended. Without `cancel`, cancelling ends the process. Either way, once its process ends the session takes
  no more prompts. `message` starts a process per prompt: the prompt is `{prompt}` in `args`, or else its standard input,
  which then closes; an argument that is `{prompt}` alone must come right after `--`, so a prompt beginning with `-` is
  not read as an option. The turn ends when a rule says so or when the process exits. A `message` tool remembers nothing
  unless `resume_args` names the session it reported (a `session` rule), put before `args` from the second prompt on.
- **Rules**: each line the tool writes is taken by the first rule whose `when` paths all hold their values; a line that is
  not JSON, or that no rule takes, is passed over. `send` is `text` or `thought` (`agent_message_chunk`,
  `agent_thought_chunk`), `tool` and `tool_done` (`tool_call`, `tool_call_update`), `failure` (the turn, once it ends,
  fails with this text), `end` (with `stop`, or `error`), `session`, `ask`, `reply` (a line written back at once) or `ignore`.
  In text, `{path}` is the line's value at a dotted path, such as `{args.command}` or `{result.content.0.text}`. In the
  JSON of `prompt`, `cancel`, `allow`, `reject` and `line`, a string that is a placeholder alone keeps the value's type,
  and only values are filled: a prompt cannot add a key or end the line, since it is never parsed as JSON, and U+2028 and
  U+2029, at which some line readers split, are escaped.
- **On a command line**: `serve` gives the entry, as JSON, to `canoa tool` on its command line, which other users of
  the machine may be able to read. Put nothing secret in a tool entry; a key belongs in the host's environment, which the
  tool inherits. An entry is at most 16 KiB as JSON and holds no NUL, or canoa.cfg is refused.
- **The person is told** what they would not otherwise know: every session opens with the notices the entry implies,
  listed under `_meta["dev.boriken.canoa/notices"]` in the answer to `session/new`, which the app shows in the
  conversation as the agent's words (any agent can send that key, so the app never shows it as its own). One says what the tool does without asking, in the entry's own `unasked` words, which every entry must give;
  a `message` tool's also says that each message runs it anew.
- **Permissions**: an `ask` rule sends `session/request_permission` with Allow and Refuse, the whole question as the call's
  input besides its title, and writes `allow` back only when the person picks Allow during the turn that asked. Anything
  else, an error or a cancelled request included, writes `reject`; so does a request that comes when no turn is running or
  16 are already waiting, and every question still waiting when its turn ends or is cancelled, at that moment, so an answer
  after that reaches nothing. What the tool does without such a line is not asked about: the
  host cannot see it before it happens, only show it as it runs, as a tool call.

**What Pi does without asking.** Pi has no approval of its own: its documentation says it "does not ask for approval before
every tool call", and runs `read`, `bash`, `edit` and `write` by default with the rights of the account that started it.
Its RPC protocol lets an extension ask the client a question, and its own example extension `permission-gate.ts` asks
before a `bash` command holding `rm -rf`, `sudo`, or `chmod` or `chown` with `777`. That question is what reaches the
person; everything else Pi does, every other command and every file it reads or writes, it does unasked, and the entry's
notice says so. Without the extension Pi asks nothing.

**What Pi cannot do through this entry**: no images or embedded files in a prompt (text and resource links only); no
session list or reopening (with `--no-session` nothing is kept, and the entry offers no `session/load`); no model or
thinking-level choice from the chat, since Pi's are set by its own `--model` and settings; no MCP servers from the
client; no slash commands listed; and no diff view, since Pi's `edit` result is text. Its model and credentials are Pi's
own: a `models.json` in `PI_CODING_AGENT_DIR`, whose key it reads from an environment variable the host passes on.

[`pi-acp`](https://www.npmjs.com/package/pi-acp) is an ACP adapter for Pi, MIT, in Zed's ACP registry, which runs `pi
--mode rpc` itself. Version 0.0.34 with Pi 1.5.0 ran a turn as a `[host.agents]` entry (`program = "npx"`, `args = ["-y",
"pi-acp@0.0.34"]`), and the app offers it as the **Pi** preset. It offers more of Pi than these rules do (its slash
commands and usage arrived in that turn; its README also lists diffs for `edit` and reopening sessions) and is the better
choice; the tool entry is for a Pi, or a tool like it, with no adapter.

`canoa serve` reads the same canoa.cfg as the app (`--config`, then `CANOA_CONFIG`, then the platform's place) and
prints one JSON line per entry: its name, kind, address, the host's peer id and the method it proves by. The app keeps `[host]` as it is when it
saves its own settings.

## Profiles

A stdio agent entry can be launched with a profile, the same schema as the app's `[profile]` (see
[Desktop and canoa.cfg](configuration.md)): environment variables, an initial context, skills, MCP servers and model,
effort and mode defaults.

```toml
[host.agents.claude]
program = "npx"
args = ["-y", "@agentclientprotocol/claude-agent-acp@0.99.1"]
profiles = ["review", "plain"]        # a client chooses among these; the first when it names none

[host.profiles.review]
context = { text = "Review before you change anything." }
share_env = ["GITHUB_TOKEN"]          # what the overlays below may read
overlays = ["team"]                   # installed by the app in overlays/ beside canoa.cfg
allow_mcp = { team = "sha256:…" }     # an overlay's MCP server, at the digest of its definition

[host.profiles.review.env.GITHUB_TOKEN]
from_env = "GITHUB_TOKEN"             # read from the host's own environment when the agent starts

[host.profiles.review.mcp.lint]
type = "stdio"
command = "/usr/local/bin/lint-mcp"

[host.profiles.plain]
```

- **A client chooses only by name, only among the entry's `profiles`.** It names one in `initialize`'s
  `_meta["dev.boriken.canoa/profile"]`; anything else there, a name the entry does not offer, or a choice in any later
  message is refused (`-32602`) and logged, and no agent starts for a refused `initialize`. A first message that is not
  `initialize` is refused too, before any secret is read or any agent starts; only [worktree requests](#worktrees),
  which the host answers itself with git, may come before it. A client never sends a
  profile's contents: every value comes from canoa.cfg.
- **The environment** reaches the agent's process when it starts, on the first line of its standard input through
  `canoa launch`, never on a command line. A reference (`from_env`, `from_file`) is read from the host's own
  environment and files when the connection starts; one that cannot be read starts no agent.
- **Context, skills and MCP servers** go with each `session/new`, `session/load` and `session/resume`, where the
  agent's harness takes them, as the app does: the context through ACP `systemPrompt` (before a client's own), skills as the
  Claude Code plugin `canoa-profile` or a directory Codex reads, MCP servers through `mcpServers`. A client's MCP server
  may not take one of the profile's names. A stdio server named by a bare command is found on the host's `PATH`, in
  absolute directories only, and sent to the agent as that absolute path.
- **An overlay's MCP server stays off** until `allow_mcp` names it at the digest of its definition; a changed definition
  is off again. The report, and the host's log, give the digest of each server that is off. A client cannot turn one on.
  An overlay is read from the copy the app installed and checked against its record's digest; a changed copy is left out.
- **The client is told what applied.** The answer to `initialize` and to each session request carries, under
  `_meta["dev.boriken.canoa/profile"]`, the profile's name, the harness, a one-line summary and each part with its layer,
  `applied`, `not supported`, `off` or `refused`, and why: names only, never a value.
- The host reads no project's `.canoa/profile.toml`: the directory is the client's to name.

## Worktrees

An agent or tool entry with `working_roots` makes a git worktree for a session when a client asks, so two sessions on
one repository each work on a branch of their own. `worktrees = false` on the entry turns it off; an entry with no
working root, and a provider, makes none.

```toml
[host.agents.claude]
program = "npx"
args = ["-y", "@agentclientprotocol/claude-agent-acp@0.99.1"]
working_roots = ["/home/me/src"]   # a repository at /home/me/src/proj gets its worktrees in /home/me/src/proj.worktrees/
worktrees = true                   # the default
```

- **The client names a directory and a branch, never where the worktree goes.** The directory must be under a working
  root as written, and again once every link in it is resolved, against the roots resolved too; so must the repository's
  main worktree, and the directory beside it that worktrees go in. A `.worktrees` directory that is a link, a branch
  other than `canoa/` and lower-case words joined by single hyphens, and a share or device path are refused before git
  runs, and so is a `.git` file or `commondir` naming a share. A worktree is made on a new branch from the directory's
  current commit, then checked again where it landed. A repository holds at most 32 worktrees beside it from the host.
- **A repository that is itself a working root has nowhere beside it** inside the roots: name the directory holding it.
- **Refused before git reads more than its settings**: a repository whose own settings (`.git/config`, a worktree's
  `config.worktree`) name a filter, an include or an attributes file, which would run a program or read files the host
  does not check; one whose git directory is not its work tree's own `.git`, as a submodule's or a
  `--separate-git-dir`'s is, or is outside the roots; and one that borrows objects from another (alternates).
- **Git runs from an argument list**, never a shell, with `--` before every path, its hooks and fsmonitor off, no
  repository looked for above the roots (`GIT_CEILING_DIRECTORIES`), none of the host's `GIT_` variables, and two minutes
  at most per command. A repository's hooks do not run for the host: a post-checkout hook such as Git LFS's does not
  either. Filters in your own global settings, such as Git LFS's, do.
- **Only a worktree the host would have made is read or removed**: a linked worktree at its own top level, where the
  branch its directory is named for goes; found there even when its HEAD was detached or moved to another branch.
- **Nothing in no commit goes unasked.** `status` counts what `git status` lists, files git ignores and repositories
  inside included, and a digest of that list. Removing a worktree that holds any of it takes the client's `discard`, which
  the host checks itself since git removes ignored files without being forced. A HEAD detached at commits no branch
  holds is never removed. The branch is deleted only when no other branch, tag or remote has a commit of it, and only at
  the commit counted: one that moved meanwhile stays.
- **Before the agent starts.** The requests come first on a connection, before `initialize`, and no agent starts for a
  connection that sends only these; once the agent runs, one is refused. So a worktree is removed with no agent of that
  connection working in it. An agent of another connection, or a process one left behind, may still be.

The requests, each answered by the host, never the agent:

| Method | Params | Result |
| --- | --- | --- |
| `_canoa/worktree/new` | `cwd`, `branch` | `path` (under the root as written), `branch`, `repo` |
| `_canoa/worktree/status` | `path` | `{"exists": false}`, or `exists`, `branch` (null when detached), `detached`, `changed`, `commits` (reached by HEAD or the branch alone), `listing` (SHA-256 of what `changed` counts) |
| `_canoa/worktree/remove` | `path`, `discard` (default false) | `removed`, `branch`, `branchKept`, `commits` |

A link swapped in, between the host's check and git's work, by someone who can already write inside a root, is not
guarded against.

## In the desktop app

Canoa's desktop app carries the host as a library and runs it in its own process, on the host's own threads, so there is
no other program to install or start. Agents stay child processes of the host.

- It starts its host when it starts, whatever canoa.cfg names and whether or not there is one yet, and again whenever its
  hosting dialog changes it. With nothing to host, the host is up and idle. If it ends on its own, the app starts it again after a pause that doubles from
  one second up to a minute, and back to one second after a run of thirty seconds. A canoa.cfg the host refuses (status 2)
  is shown, and not retried until the person asks.
- `local_host = false` under `[desktop]` in canoa.cfg, or starting the app with `--no-local-host`, leaves it off. Hosting
  says so; the keys can still be managed, and sessions on other hosts work as before.
- **One host on a home.** The app's host and a `canoa serve` on the same `COQUI_AGENTIC_PEER_HOME` never run together: the
  second says which is running (`canoa (process 12) already hosts this home`, or `CanoaUI ...`) and does not start. The app
  shows it in Hosting and tries again only when asked, so the two never take turns. A lock in the home decides, which a
  holder gives up however it ends.
- Closing the window leaves the host and its agents running. Quitting stops the host, which ends every agent it started.
  If the app dies, agents on Windows end with it; on Unix each sees its standard input close, and most then exit.
- A provider, a tool, and an agent with a profile need a process of the host's own. The app starts itself for those, with
  `--canoa-child` first, and that process runs the command and never reaches Flutter or a window.
- `canoa` stays for servers and for use with no desktop: the same library with a command line in front. `CANOA_HOST`
  names a `canoa` for the app to run as a child instead, which is for tests.

**Hosting**, in the app's side rail and its tray menu, shows the host's state and its peer id, lists its entries with
their addresses, adds a stdio ACP agent by its command on a loopback port of its own, after `canoa check` accepts the
change, and stops hosting one. It lists the keys that may connect, pairs a waiting one and removes one, and shows a PIN for
a program to pair with.

## Pairing

The host proves itself and its clients by the method of this user's `coqui-agentic-peer` identity, Noise or WebCrypto,
with the trust store `coqui-acp-bridge serve --paired` uses. A home keeps each method's identity in a file of its own:
a home paired before Noise keeps its WebCrypto identity and its paired keys, and `--method` names one when a home has
both. A key is of one method, and a client of the other is refused, which the host's log says. The home records the
method named last, or its first identity's, so the app's `serve` and `pair`, which name none, use it. **Canoa's app proves
by WebCrypto today:** a home it reaches must prove by WebCrypto (`canoa serve --method webcrypto` once) until the app
proves by Noise. A home with no identity gets a Noise one; to move a WebCrypto home, run `canoa serve --method noise`
once and pair each browser again: its Noise key is a new key.

A browser pairs by connecting once:

1. In the browser, give the entry's address (`ws://127.0.0.1:7801`), the host's peer id, which Hosting shows (and
   `canoa id` prints), and the method it proves by, which `serve` prints: in Canoa's web connect dialog, or by
   adding the agent by hand in the extension's window.
2. The first connection is turned away, and the browser's key waits on the host.
3. Hosting lists it as waiting, as does `canoa peers`. Compare its whole id with the one the browser shows (Canoa's
   connect dialog, or the extension's permissions page), then pair it: **Pair…** and **Pair** in Hosting, or
   `canoa accept <id>`.
4. Connect again. Removing the key in Hosting, or `canoa revoke <id>`, ends its links within about a second.

A program with a native key, such as `coqui-acp-bridge`, can pair by PIN instead: **Show a PIN** in Hosting, or
`canoa pair`, shows six digits and an address, and `coqui-acp-bridge pair-dial ADDRESS` with those digits pairs it.
One wrong try ends the PIN, and it pairs by the host's method. `canoa pair` refuses a browser, so a page cannot spend
the PIN; a browser pairs with it by connecting once. The extension and Canoa's web connect dialog pair by PIN with
`coqui-acp-bridge pair-accept --method noise --origin ORIGIN`, the origin each of them shows.

A key's first use is always the person's decision: nothing is accepted automatically, and the app pairs or removes a key
only from a dialog that shows its whole id.

## The web app

The host can serve Canoa's web build itself, so a browser on this machine opens the chat with no other web server:

```sh
canoa serve --web app/build/web     # a directory, or [host.web] dir in canoa.cfg
canoa serve --web                   # the site built in, in a build with the web feature
```

```toml
[host.web]
dir = "site"                 # relative to canoa.cfg; without it, the site built in
listen = "127.0.0.1:7800"    # the default
```

`serve` prints `{"web": "http://127.0.0.1:7800"}` after its entries. Open that address: the connect dialog starts filled
with the host's first entry, its peer id and its method, read from `/canoa.json`, and lists its other entries to
choose from. That is only a filling. Connecting still proves the host holds that id's key, and the host turns the
browser away until its key is paired, as in [Pairing](#pairing): connect once, then accept the browser's id in Hosting
or with `canoa accept`.

What it serves and refuses:

- **Only the site's files.** A path that could step outside the directory (`..`, an encoded separator, a Windows drive
  or device name, a name starting with a dot) is refused, and so is a file a link leads to outside it. A directory is
  never listed. A path with no extension that names no file is one of the app's routes, answered with `index.html`.
- **A strict Content-Security-Policy** on every answer: scripts, styles, images and fonts from the site alone, and
  connections to the site and the host's entries alone. Inline and evaluated script are refused, so the site must be
  built with `flutter build web --csp --no-web-resources-cdn`. That build still fetches its fonts from
  `https://fonts.gstatic.com`, and without them Flutter draws no text: until the app carries its own fonts, add
  `connect = ["https://fonts.gstatic.com"]` to `[host.web]`, which the host logs as a weakening.
- **Not the host's own files.** A site directory that holds canoa.cfg's directory or the host's identity and trust store
  is refused.
- **Requests for this host only.** A request whose `Host` is not `localhost`, an IP address or a name in `[host.web]
  hosts` is refused, so another site cannot reach this one by pointing a name of its own at this machine.
- **Plain HTTP on loopback only.** Beyond loopback it serves HTTPS, from `cert` and `key` (PEM files) in `[host.web]`,
  and refuses to start without them. The page it serves still connects to the entries' loopback addresses, so a browser
  on another machine needs a proxy carrying those as well.
- **Limits**: connections at once and from one address, how long a connection lasts, the size and time of a request,
  requests per connection, how long a write may stall, each a setting of `[host.web]` with a default; a looser one than
  the default is logged as weakened. A file whose name holds a hash is cached for a year, any other file is checked
  each time.

**On a machine others use**, the browser keeps the page's key under the page's origin, `http://127.0.0.1:7800` by
default. While the host is not running, anyone on the machine can listen on that port and serve a page of their own in
that origin, which can then use the key you paired. Serve over HTTPS there, with a certificate the browser trusts and a key only
you can read, or reach the host from the browser extension.

The API header lists every setting.

## What the host allows

- **Nothing beyond loopback.** An entry's address that is not loopback is refused, and so is the web app's, unless it
  serves HTTPS. The proof does not depend on TLS, so a proxy that ends TLS can carry a connection further and can neither
  read nor change it.
- **Nothing without a paired, proven key.** Every message after the proof is sealed in the session it ended in. An
  entry's `peers` narrows it to some paired keys.
- **No program a client names.** ACP's `session/new` can list a stdio MCP server, which an agent starts as a program
  on this machine without asking anyone. The host applies [the page scope](https://github.com/Boriken-Dev/coqui-acp/blob/main/docs/browser.md#scope)'s
  rule to a stdio agent's clients: such an entry is refused whole, an `http` or `sse` server must be `https`, and `_meta`,
  which an agent may read as launch options, is removed. A provider's facade takes no MCP server at all.
- **Only the working roots it names**, when an entry names any, compared as written: a link inside a root that points
  outside it is followed. A worktree is made, read or removed only where links are resolved too; see
  [Worktrees](#worktrees).
- **Only the profiles an entry offers, by name.** What a profile holds, its secrets and its MCP servers included, comes
  from canoa.cfg alone; see [Profiles](#profiles).

## What it trusts

- **A paired key** gets the agents its entries offer, which act with this user's rights as far as each agent lets them.
  An agent's own permission requests still go to the chat.
- **A provider library is native code.** Loading it runs it, with this user's rights, before anything can check what it
  does. It is mapped only in the process serving one connection, so a crash ends that connection alone, but it is
  trusted as fully as a program you run, and so is whoever can write it or its directory.
- **A provider source runs code too, and loads more.** Python's source trusts, besides its own file: the Python whose
  shared library `import_dirs` or the system finds; that Python's standard library, and unless `isolation` is
  `isolated` its `site-packages` and their `.pth` files and, on Windows, `canoa`'s own directory with a
  `canoa._pth` or `pyvenv.cfg` there; `GUATIAO_LIBRARY`, which its process inherits; every directory in `paths`;
  every manifest in `dirs` and the directories it names; and every module those import. With `isolation = "inherit"` it
  also trusts your own site-packages and the host's `PYTHONPATH` and `PYTHONHOME`. Whoever can write any of them runs code
  as you.
- **On Windows a source's imports are looked for more widely.** A library's imports are found beside it, beside the host
  and in the system's directories only. For a source, an import it names itself that those leave missing, such as
  `python3.dll`, is looked for in `import_dirs` and, with `import_search = "path"` (the default when `import_dirs` names no
  directory, and logged as weakened), the Windows directory, the provider's working directory (canoa.cfg's) and every
  directory on `PATH`, in that order: the first `python3.dll` found there is the one that loads. The source is then
  opened with the narrow search again, so its other imports come from where they did.
- **A stdio agent** is the program the file names, with the host's environment and working directory, and its profile's
  variables. A profile's stdio MCP server is a program the agent starts as it is told, with the variables the profile
  gives it: the values reach the agent, which can show them to whoever drives it, so give a profile only secrets you
  would give that agent's clients the use of.
- **A tool entry** runs its program with the host's environment, in the session's working directory, with the rights of
  the account that runs the host. The client names that directory: set `working_roots` on a tool entry, or any directory
  of this user's will do. A network share or device path is refused, since even looking at one reaches another machine
  with this user's credentials. On Windows the program finds nothing in that directory by a bare name
  (`NoDefaultCurrentDirectoryInExePath` is set for it), which matters for a `.cmd` such as npm's `pi.cmd`; naming `node`
  and Pi's script by absolute path avoids `cmd.exe` altogether. The host takes what the tool writes as untrusted: a title, an id or a failure loses its
  control and format characters and is cut to 4 KiB, text is split into updates of at most 1 MiB, and a line over 16 MiB
  ends the process. What the tool does is the tool's: only what its rules turn into `ask` reaches the person first. A
  process ends with its session's end, its cancel or its connection: on Windows with everything it started, by a job object
  of its own; on Unix, what it started stays in the host's process group for that connection and ends with the connection,
  unless it left the group: a command a tool starts in a session of its own (Node's `detached`) outlives the connection.
- **A provider gets what a client asks of it**: the facade passes a session's `context`, `model`, `effort` and opaque
  `meta` through, as a provider's own interface defines them.

## Claude and ChatGPT

Each is reached through the vendor's own command-line program, signed in by the person through the vendor's own flow.
The host never sees, stores or forwards a vendor credential, and never reuses a consumer app's session.

| | Claude | ChatGPT |
| --- | --- | --- |
| Entry | `npx -y @agentclientprotocol/claude-agent-acp@0.99.1`, which drives Claude Code (`CLAUDE_CODE_EXECUTABLE` names it) | `npx -y @agentclientprotocol/codex-acp@2.5.0`, which starts Codex's app server |
| Sign-in | `claude` signed in on this machine: a subscription, or an API key | `codex` signed in with a ChatGPT plan, or an API key |
| Without the adapter | `claude -p --output-format stream-json`, `--resume` | `codex exec --json`, `codex app-server` |

**What the terms say.** Anthropic's [legal and compliance page](https://code.claude.com/docs/en/legal-and-compliance)
does not prevent "an end user from signing in to the unmodified Claude Code binary with their own Claude subscription",
says subscription limits "assume ordinary, individual usage of Claude Code and the Agent SDK", and does not permit
third parties to "route requests through Free, Pro, Max, or Ultra plan credentials on behalf of their users". So an entry
running Claude on a subscription is for its owner's own keys: pairing someone else's key would make the account available
to them. OpenAI documents ChatGPT sign-in for Codex and its programmatic modes; its Terms of Use could not be read for this
page, so check its sharing rules before pairing anyone else.

**What they cannot do here.**

- Nothing of the consumer apps: no claude.ai or ChatGPT conversation history, projects, connectors, custom GPTs or image
  generation. A session exists only in the agent that made it.
- The program has to be installed and signed in on the host's machine. A headless machine signs in with
  `claude setup-token` or Codex's device-code login.
- The plan's usage limits apply, shared with the person's other use.
- An `ANTHROPIC_API_KEY` in the host's environment takes precedence over Claude's subscription sign-in, so it changes who
  pays. The agent inherits the host's environment.
- A turn in progress ends with its connection: each connection runs a fresh agent process.
