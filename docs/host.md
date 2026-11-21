# The host

`canoa` serves agents to Canoa's chat on this machine. Each entry of canoa.cfg's `[host]` table is one agent, on a
WebSocket of its own, on loopback unless its `listen` says otherwise (or on a [pipe or Unix socket](#a-pipe-or-unix-socket)):

- **Claude Code or Codex**: the installed `claude` or `codex`, served by the host's own translators of their stdio
  protocols (coqui-acp's `coqui-acp-claude` and `coqui-acp-codex`), inside the host, with no Node. See
  [Claude and ChatGPT](#claude-and-chatgpt).
- **A stdio ACP agent**: any program that speaks the Agent Client Protocol on its standard streams, such as
  `agy acp`, or `canoa agent claude` itself (with `--setting allow_client_plugins=true` when a profile gives it skills).
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
harness = "claude"                # or "codex"; instead of program and args
listen = "127.0.0.1:7801"
working_roots = ["/home/me/src"]
worktrees = true                  # a git worktree per session, inside working_roots; see Worktrees

[host.agents.agy]
program = "agy"
args = ["acp"]
usage = "antigravity"             # read agy's quota for the usage control; see Usage in the app's guide

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

With `inherit`, whoever sets the host's environment chooses the code a library runs. `canoa serve` lists it in its
log when it starts, once per entry, as configuration: `non-default: host.providers.NAME.source_settings.isolation is
inherit, default shipped`. The first source a process adds decides `isolation` and `home`; each connection is a process of its own.

What a provider prints, to standard output or standard error, goes to the host's log, up to 1 MiB per connection, and
never to the client: the host and the provider's process speak ACP over a loopback socket of their own, which the process
proves with a one-time token. Its standard input is closed once the settings are read.

On Windows the Python's `python3.dll` has to be found: name its directory (`sys.base_prefix`) in `import_dirs`. The
entry's `import_search` says where else it may come from: `library` looks only in `import_dirs` (after the source's own
directory, `canoa`'s and the system's); `path` also in the Windows directory, the provider's working directory
(canoa.cfg's) and every directory on `PATH`, so whoever can write any of those chooses the Python that loads. Without
`import_search` an entry whose `import_dirs` names a directory takes `library`, and one that names none takes `path`,
which keeps a Python on `PATH` working. `canoa serve` logs every entry on `path` when it starts: as configuration
(`non-default: host.providers.NAME.import_search is path, default library`) when the file says `path`, as a warning
when the host took it because `import_dirs` names no directory, with how to narrow it. On Linux and macOS the
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

The host's own **Pi** source is the better choice: `coqui-acp-pi`, coqui-acp's translator of Pi's RPC mode, runs inside the
host as the `pi` harness, so it needs only `pi` (no Node, no `npx`, no adapter) and offers more of Pi than these rules do
(its models and thinking levels, modes, images, and reopening sessions). A `[host.agents]` entry with `harness = "pi"` runs
it too, with `settings` for `executable`, or `node` with `script`, and `protected`. The tool entry is for a tool like Pi with
no translator.

`canoa serve` reads the same canoa.cfg as the app, from the same layers (the installed copy, the system's file and yours, or the files `--config` or `CANOA_CONFIG` list; see [the configuration](configuration.md#where-it-is-found)), and
prints one JSON line per entry: its name, kind, address, its provider id (a `harness` entry) and `title` when it has them, the host's peer id and the method it proves by. The app keeps `[host]` as it is when it
saves its own settings.

## Profiles

A stdio agent entry can be launched with a profile, the same schema as the app's `[profile]` (see
[Desktop and canoa.cfg](configuration.md)): environment variables, an initial context, skills, MCP servers and model,
effort and mode defaults.

```toml
[host.agents.claude]
harness = "claude"
profiles = ["review", "plain"]        # a client chooses among these; the first when it names none

[host.profiles.review]
context = { text = "Review before you change anything." }
overlays = ["team"]                   # installed by the app in overlays/ beside canoa.cfg
allow_mcp = { team = "sha256:…" }     # an overlay's MCP server, at the digest of its definition
allow_hooks = { team = "sha256:…" }   # an overlay's hooks, at the digest of its files

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
  profile's contents: every value comes from canoa.cfg. It may narrow which of the profile's overlays one connection
  starts with, as a list of names in `_meta["dev.boriken.canoa/overlays"]`: a name the profile does not list adds
  nothing, and the report says which overlays were not chosen.
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
- **An overlay's hooks stay off** until `allow_hooks` names the overlay's tree digest, or the person allows them for one
  session when the host asks; a changed overlay is off again. A hook runs outside the agent's sandbox with the person's
  access, from a copy of the files the digest covered, in the session's folder and with the session's environment (every
  overlay's, stacked), and can add context, block a prompt, deny, ask about or allow a tool call, or refuse a session.
  An allow answers the question the person would have been asked, with the allow-once option only, and never for a path on
  the person's protected list (`[permissions] protected`), which still goes to them; a deny beats an ask and an ask beats
  an allow. A command is not read for the paths it touches, so allowing a command call can mean writes to protected paths. An overlay whose hooks fail closed is not used while they are off. The report lists each hook as
  `applied`, `not supported` or `off`; `crates/canoa-host/AGENTS.md`, "Overlay hooks", has what fires on which event.
- **An overlay's env scripts** (`env.<ext>` and `env_scripts`) are off by the same rule: they run only while
  `allow_hooks` names the overlay's tree digest (the allowance of its hooks covers both), once when a connection starts,
  never when the person is asked, and the overlay is otherwise applied. A script runs as its `interpreter` says (found on
  the host's `PATH`, never through a shell), from a copy of the files the digest covered, with the environment the overlays
  before it left, and is stopped at its `timeout` (60 seconds unless the overlay says; 0 for none). Its output, JSON
  objects on standard output, is applied unsubstituted over the layers before it; a script that fails, times out or prints
  anything else applies nothing, is reported with the end of its standard error, and the session still starts. The report
  lists each script as `applied`, `refused` or `off`.
- **A skill an overlay names from elsewhere** is delivered only from the copy the app fetched (`overlays/.skills/`), at
  its record's digest, the overlay's pin and any pin a top-level `[skill_pins]` table in canoa.cfg gives its name
  (`review = "sha256:…"`). It never takes a name another overlay's skill holds; one kept out, one not fetched and one
  from plain `http` or a private network address are reported.
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
harness = "claude"
working_roots = ["/home/me/src"]   # a repository at /home/me/src/proj gets its worktrees in /home/me/src/proj.worktrees/
worktrees = true                   # the default
```

- **The client names a directory and a branch, never where the worktree goes.** The directory must be under a working
  root as written, and again once every link in it is resolved, against the roots resolved too; so must the repository's
  main worktree, and the directory beside it that worktrees go in. A `.worktrees` directory that is a link, a branch
  other than `canoa/` and lower-case words joined by single hyphens, and a share or device path are refused before git
  runs, and so is a `.git` file or `commondir` naming a share. A worktree is made on a new branch from the directory's
  current commit, then checked again where it landed. A repository holds at most 32 worktrees beside it from the host unless `[host.worktrees] max_per_repository` says another number, and a git command may run 120 seconds unless `git_seconds` says otherwise.
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
| `_canoa/worktree/diff` | `path`, `base` (a commit's hex id, default HEAD) | `{"exists": false}`, or `head`, `branch`, `base`, `added`, `removed`, and `files` (each `path`, `status`, `added`, `removed`, `binary`, `patch`, `cut`, `print`), `more`: what the folder holds against `base`, read only |

A link swapped in, between the host's check and git's work, by someone who can already write inside a root, is not
guarded against.

## Accounts

A harness entry (`harness = "claude"`, `"codex"`, or an outside provider's id) can offer several logins of the same
harness, so that one session runs on a work subscription and another on a personal one, or on an API key with an
endpoint of its own. The provider knows how its harness takes each kind of login (the variables, the files, the sign-in
command); the host keeps who the accounts are, their credentials, who holds one, and what a renewal does.

Each kind of account says the standards it provides (a Claude subscription provides `claude-code`, an Anthropic API key
`anthropic-api`, a Codex login `codex`, an OpenAI key `openai-api`) and each harness the standards it accepts. An account
can run a session on any agent that accepts one of them, whichever provider made it: an Anthropic key can run Pi, and a
local model served through the host can run Claude Code, Codex or Pi. An agent never lists an account it cannot use, and
naming one is refused with what each side says.

```toml
[host]
shell = "git-bash"              # the shell a terminal starts and every agent is told to use: an id `shell/list` gives (git-bash, wsl, msys2, bash, zsh, pwsh, powershell, canoa-git-bash, canoa-pwsh) or a program; absent, your own ($SHELL, else pwsh or powershell on Windows)

# One agent's own shell, in place of [host] shell. Each harness is told as its documentation allows; the host's log says when one cannot be.
# [host.agents.Claude]
# harness = "claude"
# shell = "canoa-git-bash"

# What a program store's install is held to, as gitignore-style lists (see the host's AGENTS.md); absent, the recommended lists apply.
[host.store]
protected = ["~/.ssh", "~/.gnupg", "~/.config/autostart"]   # places a declared change may not be at, under or hold
credential_variables = ["*TOKEN", "*_API_KEY"]               # variables an install is never given

# The tools that search the knowledge an overlay ships (`canoa-kb`). Without `embeddings` they match words; with it, meaning too, by the
# server you name. `[overlays.NAME] kb = "index"` serves that overlay's rules and context as a search instead of in the agent's context.
[host.kb]
embeddings = "http://127.0.0.1:60570"   # an OpenAI-compatible embeddings server; optional
model = "qwen3-embedding-0.6b-generic-cpu:1"   # the exact id it loaded; required with embeddings
llm_endpoint = "http://127.0.0.1:60570"       # a chat model that puts the best passages first (the llm search mode); optional
llm_model = "qwen2.5-1.5b-instruct-generic-cpu:4"   # the exact id it loaded; required with llm_endpoint

# The person's mail as a source (`canoa-mail`): see "Mail as a source".
# [host.mail.accounts.work]
# backend = "ews"
# server = "https://mail.example.com/EWS/Exchange.asmx"
# user = "EXAMPLE\\ana"
# id = "..."                             # `canoa mail add` writes it: the password is in the account store

[host.agents.claude]
harness = "claude"
accounts = ["work", "personal"] # a session runs on the first unless its client names another

[host.accounts.work]            # written by `canoa accounts add`; holds no credential
kind = "claude-login"
id = "0b7d3c1e-5a52-4f0e-9d0f-6c1f5d5b8e21"

[host.pools.logins]             # accounts that provide one credential standard, chosen as one
standard = "claude-code"
accounts = ["local", "work"]
```

A session runs on one account, with a model that account lists, or on a pool: a named set of accounts that provide the same standard (the kind of credential: a Claude Code login, an Anthropic API key, an OpenAI API key, whatever the providers name). The Accounts section makes, edits and removes pools: pick the standard and it offers every account that provides it. Canoa also makes an `any-<standard>` pool for each standard an account provides, holding all of them, which follows the accounts as they are added and removed and is written nowhere. It is listed (Settings, the pickers) only while two or more accounts hold it, since a pool of one cannot rotate; a session or canoa.cfg that names it still gets it. An entry offers a pool by naming it among its `accounts`. A pool takes each new session on the next of its accounts that lists the model and has usage left, in turn; a session keeps its account while its prompt cache is warm, moves to the next account when the cache has lapsed or when its account has run out (the conversation goes with it, and the chat says which account and why), and a turn that no account of the pool can take is refused with the time the soonest allowance refills. A session on one account is never moved: when that account runs out, the chat offers the move.

```sh
canoa accounts providers                                         # the account providers: each agent's own and each maker (a proxy, a gateway, a model service), with its own ways to sign in
canoa accounts add work --provider claude_accounts --method subscription   # signs in; asks for the code the page shows, with no echo
canoa accounts list                                              # name, provider, kind, state; never a credential. canoa.cfg's accounts, then `local` of each harness that has a login of its own
canoa accounts login work                                        # sign in again when a login was lost
canoa accounts remove work                                       # forgets the stored credential and the account's directory
```

- **A credential is never in canoa.cfg.** A file that holds one under `[host.accounts]` is refused, naming the key and not
  quoting the value. It rests in the operating system's credential store (Windows Credential Manager, macOS Keychain,
  Secret Service), and a typed key is read from the terminal without echo, never from the command line. A host with no
  such store, a Linux service with no session bus for one, keeps a file per account in the host's directory instead,
  sealed to a key beside it, with nothing to set: `[secrets] accounts = "auto"` is the default, probes the operating system's
  store once and falls back to files when it does not answer. That keeps a copied file closed and nothing from anyone who
  can read the directory, so the host logs it as a weakening with the reason, and `canoa accounts list` prints the same
  note. Once files hold an account, `auto` stays on files, so a store that answers later does not hide them. `"file"`
  chooses files outright; `"os"` insists on the operating system's store and fails, naming `auto` and `file`, where there
  is none.
- **One copy of a login that rotates.** Claude's and Codex's logins replace their refresh token each time they renew, so a
  second copy would be dead the moment the first renewed. A session on such an account holds it from its start until its
  connection ends, a second connection that chooses it is turned away saying who holds it, and so is a second session on
  the same connection. Sessions on an API key share it.
- **What the harness renews is kept.** While a session runs the harness may refresh its login in its files. The host stores
  the newer copy, ignores an older report, and when the harness lost the login (a refused refresh blanks or deletes its
  file) marks the account as needing a sign-in without replacing the last copy it holds.
- **The harness gets only the account.** Every credential variable the provider knows is removed from what it starts, the
  account's is set, and a login kind's files are written into a directory of the account's own under the host's directory,
  taken away when the last session on it ends. A credential variable in the host's own environment is logged as replaced.
- A client chooses the account in `initialize`'s `_meta["dev.boriken.canoa/account"]`; the answer's `_meta` names the
  account and its kind.
- The app adds, signs in to and removes accounts over the host's control channel, on the desktop and in a browser alike: a
  sign-in that asks for a typed secret (Claude's code, an API key) puts a form to the app, and the answer comes back sealed to the
  key that asked, so the host opens it and nothing else sees it. The same holds for a paired key whose host grants `accounts`.
  The app also sets which accounts an agent entry offers (`accounts/offer`), which writes canoa.cfg. The host does not start again
  by itself: the offers apply from its next start, and the app asks for one (`host/restart`, which a host run with `--supervised`
  answers by ending so that its supervisor starts it).

### A local model as an account

An account of kind `foundry-local` runs a session on a model Foundry Local serves on this computer, whichever of Claude Code,
Codex or Pi the entry runs. It holds no credential: the model's id (`model`, as `foundry model list` shows it), and optionally
the service's address (`endpoint`; the running service's own is read from `~/.foundry/daemon.json`), are the account.

```sh
canoa accounts add local --provider foundry_accounts --method foundry-local --input model=qwen3-14b-generic-gpu:2
```

- **Loading a model is the person's.** Load it first, from the Store's Foundry models page (Foundry Local itself is in the
  Store's Foundry Local type) or with `foundry model load qwen3-14b-generic-gpu:2`, and unload it when done, since it holds gigabytes other work
  needs. The gateway never loads or unloads one; a session on a model that is not loaded is refused with that said.
- **A gateway per session.** When a session starts on the account the host listens on loopback (a port of the system's choosing)
  and hands the harness an address and a bearer minted for that session, through the harness's own account kind: a gateway
  token for Claude Code (`anthropic-auth-token`), an API key and `openai_base_url` for Codex (`codex-api-key`, with a
  `config.toml` in its folder that turns off its web search and its multi-agent and goal tools), and an `OPENAI_API_KEY` with a
  `models.json` in its folder for Pi (`pi-api-key`). The bearer ends with the session, the gateway with its last session, and
  a request is answered only when it shows the bearer, names this socket in `Host` and carries no `Origin` (and, on Linux, comes from a process of this user).
- **`answer_seconds`** is how long one answer may take (1800, 30 minutes, with the `service` engine); a large prompt on a slow model may need
  longer, and there is no ceiling.
- **The three APIs.** The gateway answers `/v1/messages` (Anthropic: Claude Code), `/v1/responses` (OpenAI Responses: Codex and
  Pi) and `/v1/chat/completions`, each translated to one chat completion, with `/v1/models` and `/v1/messages/count_tokens`.
  Any model name a client sends is served by the account's model. The model is called once, whole, and a stream is replayed from
  its answer, with a ping or comment while it works: the service's own streaming delivers no tokens in 0.14.0.
- **What the model cannot do is said, not faked.** Tools offered to a model that cannot call them, an image to one that cannot
  read it, `previous_response_id` and a tool the gateway cannot run (web search, deferred tools) are refused or dropped with the
  reason, in the API's own error shape and in an `x-canoa-dropped` header. The service's model list overstates tool calling
  (only `qwen3-14b-generic-gpu:2` returned structured calls when measured), so the account offers tools and the service's own
  refusal is passed on as it said it.
- **`engine`** is `service` (the default: the model lives in the Foundry Local service) or `sdk` (the model lives in a process
  of the host's own, through Foundry Local's Rust SDK, which needs a build with the `foundry` feature and `library`, a copy of its
  native library made outside the installed package: Windows does not let a program outside the package load one from it).
- **The `sdk` engine runs in its own process.** The gateway starts `canoa helper canoa-foundry` (the program the host starts its
  other commands with) through the host's spawner for the account, which opens the SDK, loads the model and answers over its standard input and output. The native library
  crashes on some models and prompts (a segmentation fault on a prompt of about two thousand tokens was measured on
  `gpt-oss-20b-generic-gpu:1`), and a crash there ends only that process: the request under way fails with how it ended, and the
  next request starts the process again, which loads the model again (seconds to a minute). A process that has not stopped its
  work 30 seconds after a request was cancelled is ended. The model is unloaded when the account's last session ends.
- **The model's folder.** The SDK keeps the models registered with it in a catalog file inside the models folder it is given, so
  the `sdk` engine is given a models folder of the host's own, `foundry-models/` in its home, and links the model into it from
  Foundry Local's cache (hard links, the weights never copied): Foundry Local's own folder and its catalog file are not
  written. On a drive that cannot link the weights (the host's home and Foundry Local's cache on different drives) the engine
  falls back to Foundry Local's folder, changes its catalog file while the model runs, restores it when the engine ends, and the
  host's log says so.
- **Tool calls on a model whose package has no tool-call markers.** Foundry Local refuses a request that carries tools to a
  model whose package publishes no opening and closing tool markers (`qwen3.5-9b-generic-gpu:4` is one), although the model
  writes `<tool_call>` and `</tool_call>`. With the `sdk` engine the model is registered as a copy of the host's own that carries
  those markers (the `qwen3.5` family; no other model is changed), so it takes tools; the `service` engine cannot do this.
- **A client that goes away stops the model's work.** The gateway watches each request's connection: when the client hangs up
  (Claude Code killed, a timeout) the engine's call is ended, so the model is free for the next request. With the `service`
  engine that closes the connection to the service, and with the `sdk` engine it cancels the native request. Through a proxy
  the host's network puts between the host and the service, the call is abandoned but the service cannot be told.
- **A tool call cut by the token limit is a stop for length.** When the answer runs out of tokens while the model writes a tool
  call, Foundry Local answers HTTP 500 (`ThrowInvalidToolCallWithoutRecovery ... Tool-call output was interrupted`). The gateway
  answers it as the API's own stop for length (`max_tokens` in Anthropic's, `length` in chat completions, `incomplete` in
  Responses) with the call dropped and a line saying so, which the agent knows how to meet; it is no longer an error.
- **A model's window.** The window is how many tokens a prompt and its answer may take together: a model's own
  `genai_config.json` says what it was built for (`model.context_length`, the most it can be set to) and what a session over
  it allows as cached (`search.max_length`). The Store's page of a Foundry model shows both and offers a setting, kept in
  canoa.cfg:

  ```toml
  [foundry.models."qwen3-1.7b-generic-gpu:2"]
  window = 16384
  ```

  A number above the model's context length is lowered to it, and the page says so. Foundry Local's cache is never edited
  (a re-download would undo that): with `engine = "sdk"` the host makes a folder of its own for the model under
  `foundry-windows/` in its home, with a hard link to each of the cached model's files and its own `genai_config.json` whose
  `search.max_length` is the window, and registers it in the SDK's local catalog (the one in `foundry-models/`). The weights are linked, never copied: on a
  drive that cannot link them (Canoa's home and Foundry Local's cache on different drives) the account's gateway is refused
  with that said, since copying gigabytes silently is not a choice to make for the person. The setting is read when a gateway
  starts. **The `service` engine ignores it**: the Foundry Local service loads the models of its own cache and has no call to
  load another folder (its routes as measured on 0.14.0 include `/catalogs/local/v1/`, which listed no model, and none that registers one), so
  the host logs that the window was not used. The NPU builds are compiled for 4096 tokens and cannot be raised.

## Environments

An environment is where an agent works, which need not be the computer the host runs on: a machine reached over a link, a container, a virtual machine. Each kind of environment is a provider of the `environment` kind that a library adds, so the host names none; the host's own computer is the environment `here`, which every host has and which needs no entry. An environment says what it is and what it can do as a list of capability names (`exec`, `terminal`, `fs.read`, `fs.write`, `fs.list`, and any name of its own, such as `snapshot` or `ui`), each with its facts; the host does nothing the environment does not offer, and a refusal names the capability. What an agent may do in an environment is what the environment grants, and the session's permission mode asks about the rest as it does anywhere.

```toml
[host.environments.build-box]          # made by the provider `kind` names; the other keys are its settings
kind = "some_environment"             # an id a library offers
library = "/path/to/library.so"       # or `source`, as a harness entry names its provider; none for a bundled one

[host.agents.worker]
program = "worker-agent"
environment = "build-box"             # this entry's agent runs there; `here` (the default) runs it on this computer
```

A session uses an environment three ways, which can be combined, and a client asks for them in `initialize`'s `_meta["dev.boriken.canoa/environments"]`: `{"run": NAME, "serve": NAME, "tools": [NAME, ...]}`, each part optional. A name this host does not have turns the connection away.

- **The agent runs in it** (`run`, or the entry's `environment`). The agent's program is started through the environment's spawner, so its own tools, files and shell are the environment's: where the agent thinks it is. A program entry's session identity reaches it in its environment; an entry with a profile cannot run elsewhere, since a profile writes its skills and settings on this computer, and the host says so.
- **Its standard tools are served from it** (`serve`). The agent runs where it does and the host, as the ACP client, answers its `fs/read_text_file`, `fs/write_text_file` and `terminal/*` calls from the environment, advertising in `clientCapabilities` only what the environment offers. A call for a capability it lacks is refused with the capability's name.
- **It is offered as tools** (`tools`). Each environment is one MCP server, `env_<name>`, with a tool for each capability it offers (`exec`, `terminal_open`, `terminal_send`, `terminal_read`, `terminal_close`, `fs_read`, `fs_write`, `fs_list`, and `call_<name>` for a capability of its own), so one agent reaches several environments at once.

The first prompt of each session also tells the agent which environments it has, how each is attached, what each offers and what it lacks ("no interactive shell here", "read-only files here"), so it does not try what is not possible. A terminal in the app's panel opens in an environment with `terminal/open`'s `environment`.

Which agents ask their client for files and commands, so that serving them changes anything, was read from each translator's source and its recorded sessions:

| Agent | Its file and shell tools | Through the client's `fs/*` and `terminal/*` | Use |
| --- | --- | --- | --- |
| Claude Code | run inside the program (recorded sessions: `Read`, `Edit`, `Write`, `Bash`, `Grep`, `Task*`); the host answers only its permission questions | no | run it in the environment, or offer the environment as tools |
| Codex | run inside the program (its `commandExecution` and file-change items) | no | the same |
| Pi | run inside the program (`edit`, `write`, `bash` tool executions) | no | the same |
| OpenCode, Grok, Antigravity | their own ACP agents; whether they ask the client when it offers `fs` and `terminal` was not measured, since that needs the real program and a login | not measured | run in the environment, or offer it as tools |

When a client asks for the tools of Claude Code, Codex or Pi to be served, the host tells the agent that doing so changes nothing it does.

## Granting a key the Store and accounts

`[host.control] grants` names, per key, what that key may do. Two of the grants run programs as the user: **`store.programs` lets
the key download and run any program as the user** (it installs, updates and removes the programs of the host's Store and
runs a version probe of what it installs), and `accounts` lets it sign in to a provider, which runs that provider's program and
keeps a credential on the host. Write them only for a key whose holder you would give a shell. A host run for a portal gives both
to the person's own browser, because reaching that host needs the person's own sign-in.

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

**Hosting**, in Local's overview in the app's Hosts section and in its tray menu, shows the host's state and its peer id, lists its entries with
their addresses, adds a stdio ACP agent by its command on a loopback port of its own, after `canoa check` accepts the
change, and stops hosting one. It lists the keys that may connect, pairs a waiting one and removes one, and shows a PIN for
a program to pair with.

## Pairing

The host proves itself and its clients by the method of this host's identity, Noise or WebCrypto, kept with its trust store
in `.canoa` in the user's home folder (`%USERPROFILE%\.canoa` on Windows; `COQUI_AGENTIC_PEER_HOME` names another). `canoa serve --program --paired` reads the
same layout. A home keeps each method's identity in a file of its own:
a home paired before Noise keeps its WebCrypto identity and its paired keys, and `--method` names one when a home has
both. A key is of one method, and a client of the other is refused, which the host's log says. The home records the
method named last, or its first identity's, so the app's `serve` and `pair`, which name none, use it. **Canoa's app proves
by WebCrypto today:** a home it reaches must prove by WebCrypto (`canoa serve --method webcrypto` once) until the app
proves by Noise. A home with no identity gets a Noise one; to move a WebCrypto home, run `canoa serve --method noise`
once and pair each browser again: its Noise key is a new key.

While a host runs on a home, `canoa id --method X` and `canoa pair --method X` refuse to change the home's method: the
host's next restart (it restarts on every `canoa.cfg` change) would come up with the other identity and refuse every
client already paired. The refusal names the running host and ends with status 3; `--force` changes the method anyway and
logs it at `info`. Naming the method the home already has is never refused, and a home no host runs on changes freely.

A browser pairs by connecting once:

1. In the browser, give the entry's address (`ws://127.0.0.1:7801`), the host's peer id, which Hosting shows (and
   `canoa id` prints), and the method it proves by, which `serve` prints: in Canoa's Add connection dialog, or by
   adding the agent by hand in the extension's window.
2. The first connection is turned away, and the browser's key waits on the host.
3. Hosting lists it as waiting, as does `canoa peers`. Compare its whole id with the one the browser shows (Canoa's
   Add connection dialog, or the extension's permissions page), then pair it: **Pair…** and **Pair** in Hosting, or
   `canoa accept <id>`.
4. Connect again. Removing the key in Hosting, or `canoa revoke <id>`, ends its links within about a second.

A program with a native key, such as another `canoa`, can pair by PIN instead: **Show a PIN** in Hosting, or
`canoa pair`, shows six digits and an address, and `canoa pair --join ADDRESS` with those digits pairs it.
One wrong try ends the PIN, and it pairs by the host's method. `canoa pair` refuses a browser, so a page cannot spend
the PIN, unless `--origin` names it; a browser pairs with it by connecting once. The extension and Canoa's web Add connection dialog pair by PIN with
`canoa pair --method noise --origin ORIGIN`, the origin each of them shows.

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
mode = "local"               # "local" or "remote": where the person sits (see below)
```

`serve` prints `{"web": "http://127.0.0.1:7800"}` after its entries. Open that address: the Hosts section lists that host, named as the host that served the page and never as Local, with
its peer id, its method and its entries, read from `/canoa.json`. That is only what the host says of itself. Connecting still proves the host holds that id's key, and the host turns the
browser away until its key is paired, as in [Pairing](#pairing): connect once, then accept the browser's id in Hosting
or with `canoa accept`.

`mode` says where the person is in front of the page. **`local`**: the browser is at this computer, so Open in starts VS Code or the file
manager here (a key holding `workspace.open`; `workspace/openers` says what is installed). **`remote`**: the browser is only the interface to
a computer elsewhere, so Open in VS Code is a `vscode://vscode-remote/ssh-remote+HOST/PATH` link the browser opens (it needs VS Code's Remote -
SSH extension and an SSH route to this computer under the name the page was reached at) and the file manager is not offered: the Files panel
shows the folder. Unset, a page reached as `localhost` or a loopback address is local and any other name is remote. The host puts the
answer in `/canoa.json` as `web`, and the page reads it there.

What it serves and refuses:

- **Only the site's files.** A path that could step outside the directory (`..`, an encoded separator, a Windows drive
  or device name, a name starting with a dot) is refused, and so is a file a link leads to outside it. A directory is
  never listed. A path with no extension that names no file is one of the app's routes, answered with `index.html`.
- **A strict Content-Security-Policy** on every answer: scripts, styles, images and fonts from the site alone, and
  connections to the site and the host's entries alone. Inline and evaluated script are refused, so the site must be
  built with `flutter build web --csp --no-web-resources-cdn`. The app carries its Roboto font and looks for any other
  beside the page, so that build fetches nothing from outside the site.
- **Not the host's own files.** A site directory that holds canoa.cfg's directory or the host's identity and trust store
  is refused.
- **Requests for this host only.** A request whose `Host` is not `localhost`, an IP address or a name in `[host.web]
  hosts` is refused, so another site cannot reach this one by pointing a name of its own at this machine.
- **Plain HTTP on loopback only.** Beyond loopback it serves HTTPS, from `cert` and `key` (PEM files) in `[host.web]`,
  and refuses to start without them. The page it serves still connects to the entries' loopback addresses, so a browser
  on another machine needs a proxy carrying those as well.
- **Limits**: connections at once and from one address, how long a connection lasts, the size and time of a request,
  requests per connection, how long a write may stall, each a setting of `[host.web]` with a default; one above
  the default is logged as configuration. A file whose name holds a hash is cached for a year, any other file is checked
  each time.

**On a machine others use**, the browser keeps the page's key under the page's origin, `http://127.0.0.1:7800` by
default. While the host is not running, anyone on the machine can listen on that port and serve a page of their own in
that origin, which can then use the key you paired. Serve over HTTPS there, with a certificate the browser trusts and a key only
you can read, or reach the host from the browser extension.

**The Office add-in** is off unless `[host.web] office = true`. It makes the host serve the add-in's task pane under `/office/`
(the page that loads Office.js and frames the chat-only app) and relaxes the policy of those routes only: Office.js is served
by the host itself (`/office/js/`: the build carries it, so a pane asks Microsoft for nothing; a build made with `CANOA_OFFICE_JS=cdn`
names Microsoft's address instead and the policy of the pane and the dialog allows that one origin), and Outlook on the web may frame
the pane and the app's page. Every other path keeps the
policy above. Office on the web needs HTTPS, so it is used behind a certificate the browser trusts. The add-in's folder in the
repository, `clients/office/`, says how to render the manifests for your address and sideload them; `office_frames` names further
origins that may frame the pages (Word, Excel and PowerPoint on the web, once you have seen which ones your account uses). A
portal serves the same pages with `office = true` in its own settings, and signs in through Office's dialog: see
[The portal](portal.md).

**The Office add-in on this computer** adds `office_local = true`, with `listen` on loopback and `cert` and `key` the browser engine
inside Office trusts (HTTPS is what Microsoft recommends for every add-in, and Office on the web needs it). The task pane then needs no
sign-in and no pairing: its page asks the host that served it to accept its key, which the host does for a request from this computer,
at a loopback name, from the page's own origin, for a key that has connected once. What it can do is what the browser extension's key
can do (`[host.native] control`), and the keys it was given are kept, so a restart of the host needs no new request. The page's key
also holds the `mail` grant when `[host.mail]` names an account (and its `tools` are not turned off), so the pane's chat can search,
read and draft the mail you set up there without a step of its own; `office_secret` is what keeps another account on a shared
computer from being admitted. Do not set it behind a proxy.

Three more things belong to the same setting. **The pane's tools:** while the pane is open in Outlook, Word or Excel, every session
of this Canoa can use that application's tools (`canoa-outlook`, `canoa-word`, `canoa-excel`: the open item, the document, the
workbook), and a change is asked in the pane first. **A secret**, for a computer other accounts use: `office_secret = true` makes the
host keep one in its home and refuse the page's requests that do not carry it; the manifest is rendered with it. **Starting Canoa
from the pane:** `canoa office-launcher`, a small program you leave running, holds the pane's address while no Canoa runs and shows a
page with **Start Canoa**, which starts it; `canoa office-launcher --install` makes it start when you sign in (`--uninstall` stops that, `--status` says
whether it does and whether a launcher answers). `clients/office/README.md` in the repository says how to set each up.

The API header lists every setting.

## The browser extension on this computer

`canoa native install` lets the Canoa extension in Chrome, Edge or Chromium use the Canoa on this computer; the desktop app does it each time it starts. The extension then lists the host's entries, its own sources and your own agent commands among them, and its sessions and settings, once you pair it with a PIN: **Hosting, then Pair the browser extension** in the app, or `canoa pair --native`, shows the PIN and you type it in the extension. When Canoa is not running the extension says so and offers **Start Canoa**.

```toml
[host.native]
enabled = true                                 # false closes the door the browser's native host reaches
control = ["ledger.read", "ledger.write", "worktree", "store.read", "folders", "config.read", "config.write", "discover"]   # the default
```

`control` is what a browser key may do on the control channel; it cannot hold `config.write.sessions`, `overlay.write`, `host.restart` or `upload`. A key paired this way works only from the extension that paired it. `canoa native status` shows what is registered and what Start would run, and `canoa native uninstall` removes it. Chrome, Edge and Chromium installed as Flatpak or Snap cannot start a native host. See [browser integration](browser.md#canoa-on-this-computer).

## A pipe or Unix socket

A native client on the same machine can reach an entry without a port: set its `listen` to `pipe:NAME` on Windows, a named
pipe `\\.\pipe\NAME`, or to `unix:PATH` on Unix, a socket at an absolute path (`pipe:NAME` also works on Unix, and means
`NAME.sock` in `$XDG_RUNTIME_DIR`, else `$TMPDIR`).

```toml
[host.agents.claude]
harness = "claude"
listen = "pipe:canoa-claude"          # Windows
# listen = "unix:/run/user/1000/canoa/claude.sock"   # Linux and macOS
```

The proof is the same as on an address: the entry serves a key only once it is paired (`canoa accept`), `peers` and
`max_peers` apply, and every message after the proof is sealed. What is added is the operating system's access control
before the proof, from [`coqui-transport-pipe`](https://github.com/Boriken-Dev/coqui-transport-pipe) with its defaults:

- **Windows**: the pipe is open to the current user alone (and, in an elevated host, not to a process of that user at a
  lower integrity level), is created as the first of its name so no other program can already hold it, and refuses
  clients from other machines. A second host on the same name does not start.
- **Unix**: the socket is `0600` in a directory that only its owner (or root) can write, which the host refuses to use
  otherwise; a leftover socket nothing listens on is replaced, and one that something answers on is never taken.

That decides who can reach the entry, not who is let in: only a paired key is. A pipe name is visible to every user on
Windows, so a client that connects before any host listens would reach whoever listens first, and only the key proof tells
it from your host; pin the host's id as for an address. The web app never lists a pipe entry in `/canoa.json` or lets the
page connect to one, since a browser cannot reach it. `canoa check` prints the entry and refuses a name the system cannot
use, and a `unix:` path is refused on Windows.

The app dials such an entry when the host it runs lists one (`app/lib/acp/pipe_channel.dart`: on Windows a pipe opened for overlapped I/O through `dart:ffi`, on Unix a socket from `dart:io`), proving the same pinned key, and so can a Rust client with `canoa_host::pipe::PipeChannel::connect`.
## A transport provider

An entry can listen through a transport provider instead of an address, a pipe or a Unix socket: name it with `transport`, and
give the endpoint it listens on as `listen`. `pipe` and `websocket` are built in; any other id is loaded from a library or a
source, as a harness provider is, with the entry's `transport_library` or `transport_source` (and `transport_source_settings`
and `transport_import_search` for a source). Its own configuration is `transport_settings`, which the host checks against the
provider's schema before it listens.

```toml
[host.agents.claude]
harness = "claude"
transport = "websocket"
listen = "ws://127.0.0.1:7801"
transport_settings = { listen = { frames = "binary" } }   # the proof sends bytes: binary frames

[host.agents.other]
program = "my-agent"
transport = "my_transport_id"
listen = "tcp://127.0.0.1:7802"
transport_source = "guatiao_source_py.dll"
[host.agents.other.transport_source_settings]
dirs = ["transports"]
```

The proof is the same as on an address, and so is everything after it. What the provider decides is the wire: how messages
are framed (the built-in `pipe` frames with a big-endian length, the entry's own `pipe:NAME` with a little-endian one, so a
client of one does not talk to the other), who can reach it, and whether plain WebSocket may leave loopback (only with
`listen.plain_beyond_loopback` in `transport_settings`, which the host logs). The host's `[host] origins` are passed to a built-in `websocket` as its `listen.origins` (logged) unless the entry lists its own; the built-ins also cap a message at 1 MiB, raised in `transport_settings` (`channel.max_message` of `pipe`, `listen.limits.max_message` of `websocket`). An outside provider checks origins by its own settings, if at all.
An outside provider is code that runs
in the host's process, and the host logs that at every start. The web app never lists such an entry. Dialling through a
provider, `wss://` and WebTransport are not offered yet.

## What the host allows

- **Loopback by default.** An entry listens on loopback unless its `listen` names another address, which `serve` logs as
  configuration: the proof does not depend on TLS, and every message after it is sealed, so a connection that is carried
  further can neither be read nor changed. The web app is refused beyond loopback unless it serves HTTPS, since its
  page is not sealed and whoever is on the path could rewrite the code that holds your key.
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
  directory, and logged as a warning), the Windows directory, the provider's working directory (canoa.cfg's) and every
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
| Entry | `harness = "claude"`, whose provider runs `claude` in stream-json mode inside the host (`executable` names another) | `harness = "codex"`, whose provider starts `codex app-server` inside the host (`executable` names another) |
| On standard streams | `canoa agent claude` | `canoa agent codex` |
| Sign-in | `claude` signed in on this machine: a subscription, or an API key | `codex` signed in with a ChatGPT plan, or an API key |
| Protocol it drives | `claude -p --input-format stream-json --output-format stream-json`, `--resume` | `codex app-server`'s JSON-RPC |

**A harness entry.** `harness` names an `acp_harness` provider by its id; `claude` and `codex` are the ones the host
carries, registered in its own process, and any other id is an outside provider that the entry's `library` or `source`
loads (see below). The host runs the chosen provider in its own process for each connection and gives it a spawner, which
starts the command line as a child of the host, ended with everything it started when the connection or the host ends (a
job object on Windows, a process group elsewhere). The scope above applies before it: a client's `cwd` and more
directories are held to `working_roots`, its `_meta` removed, and its stdio MCP servers refused. The roots go to the
provider too, which holds a resumed Codex thread's own directory, which no request shows the host, to them. With a profile
they go with one more directory, the one the connection's skills are written under (a `canoa-profile-` directory of the
system's temporary one, removed with the connection), since the provider is told to work there too. An entry with no
`working_roots` takes any directory, with or without a profile. What the command line may do and asks before it acts is
the profile's `access`, the same for every agent ([configuration](configuration.md#access)): with nothing set, no sandbox, the
network on and everything asked, whatever the configuration of the command line says. Claude Code's own allow rules in the
settings it loads still apply. Permission prompts reach the
client as `session/request_permission` with the command line's own options (Claude Code: allow once, always allow with
the rule it suggests, reject; Codex: its own decisions).

```toml
[host.agents.claude]
harness = "claude"
setting_sources = ["project"]     # of user, project, local; unset is Claude Code's own
executable = "C:/Tools/claude.exe"  # optional: a name on PATH, or a path from canoa.cfg's directory

[host.agents.codex]
harness = "codex"
profiles = ["strict"]             # optional: the profile's `access` is this entry's access
[host.profiles.strict]
access = { sandbox = "workspace", network = false, approvals = "edits" }
```

An entry with no profile runs with the standard access. A paired client is offered every mode the agent has (Plan, Manual,
Accept edits, Auto where there is one, Bypass) and may choose any of them: the host runs as its person, and a key paired to it
is theirs. A paired key can therefore put a session in Bypass, which asks for nothing and, with the standard access, has no
sandbox: pair only keys you would give your own account to. The keys `permission_mode`, `approval_policy`, `sandbox`,
`bypass` and `modes` are gone: the access replaces the first three, and no key gates a mode.

`cache_lifetime = "default" | "off" | "short" | "long"` sets how long the provider's prompt cache lives for the entry's sessions, where
the harness has a control for it. Claude Code has: `short` is a 5-minute cache and `long` a 1-hour one (it sets
`CLAUDE_CODE_PROMPT_CACHE_TTL` and `CLAUDE_CODE_SUBAGENT_PROMPT_CACHE_TTL`), `off` sets `DISABLE_PROMPT_CACHING`, and `default` leaves the environment's. On a
subscription a 1-hour cache write costs twice the input price against 1.25 times for 5 minutes, so a one-shot or background
run is better with `short`, and a session someone is active in with `long`. Codex's app server has no such setting, and
`canoa options NAME --entry` prints each provider's own sentence on it as `cache_lifetime` and the lifetimes it honours as `cache_choices`; `off` goes only to a provider that lists it (a client's `off` for Codex is the default). The key is the entry's default: the
chat asks `long` for a session when the entry says nothing, and the person can choose another for one session in the
new-session view, which wins. A value other than the four is refused naming `cache_lifetime`. `canoa agent ID`
takes the same as `--cache-lifetime`.

On Windows the providers start `claude.exe` and `codex.exe`, as the native installers put them; an npm-installed `.cmd`
is untested. With `profiles`, the profile's environment is the command line's process
environment, never its command line; its skills are the host's own Claude Code plugin, the only plugin the provider
takes, since the host removes every client's `_meta`; context, MCP servers and Codex's skills go as the
[profile table](configuration.md#applying-a-profile) says. The providers are tested with Claude Code 2.1.318 and Codex
0.164.0 and 0.166.0; another version runs, with a notice.

**An outside harness.** Any provider of the `acp_harness` kind, in any language Guatiao loads, is chosen the same way, by
its id, and needs no rebuild of Canoa. The entry names where its code is, as a provider entry does, and the provider's own
configuration:

```toml
[host.agents.echo]
harness = "acp_echo_harness_echo_harness"   # the provider's id
source = "guatiao_source_py.dll"            # or library = "..."; relative paths are from canoa.cfg's directory
settings = { title = "Echo" }               # optional: the provider's configuration, checked against its schema at start
[host.agents.echo.source_settings]          # as for a provider entry
dirs = ["harness"]
```

`executable` and `setting_sources` are the `claude` and `codex` providers' settings and belong to the built-in ids; another id
takes `settings` alone. Loading runs the provider's code with the host's rights,
in the host's process, so a provider that crashes takes the host with it. Canoa passes no account, so the provider runs on
the harness's own sign-in, and the prompt cache lifetime is the harness's default.

**What the host hears.** A provider tells the host what is no message of the client's through the spawner the host hands
it: a version it was not tested with, a program that exited mid-session (how, never what the program last wrote), a part of
its own plumbing that failed. The host logs each at `info`, with the entry's name and the notice's kind (`notice`, `exited`,
`fault`). A notice holds no client text and no secret value. A provider that ends a connection with a failure has its status
and reason logged at `info` too. Each connection works on at most 64 requests at once, as before; the entry's `settings` can say
`max_requests` for another number. Each entry loads its provider into a registry of its own, so an id is resolved from that
entry's file and no other, and `claude` and `codex` cannot be replaced by a `library` or `source`. How an instance
departs from its provider's defaults (`settings = { allow_bypass = false }`, `max_requests`) is logged at
`info` as configuration when the host starts.

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

## Mail as a source

With a mail account in canoa.cfg the host gives an agent the person's own mail as it gives it an overlay's knowledge: search it,
read it, find what is related to a message, and draft into it. Gather what was said before writing a new message, or sort an
inbox, from any session on this computer.

```toml
[host.mail]
tools = true                             # false turns the tools off; they are offered only once an account is named

[host.mail.accounts.work]                # a name of lower-case letters, digits, - and _
backend = "ews"                          # Exchange Server's web services
email = "ana@example.com"                # Autodiscover finds the server from this ...
server = "https://mail.example.com/EWS/Exchange.asmx"   # ... or name it, and nothing is discovered
user = "EXAMPLE\\ana"
# The password is not here. `canoa mail add` keeps it in the host's account store and writes an `id = "..."` line that names it;
# or name a variable that holds it, which is read before the store's copy:
# password_env = "WORK_MAIL_PASSWORD"

[host.mail.accounts.cloud]
backend = "graph"                        # Exchange Online through Microsoft Graph, signed in with the Windows account
client_id = "11111111-2222-3333-4444-555555555555"   # your app registration in Entra ID (required)
# tenant = "example.com"                 # the registration's tenant; "organizations" otherwise
# email = "ana@example.com"              # which Windows account to ask, when it holds several

[host.mail.accounts.desktop]
backend = "outlook"                      # classic Outlook for Windows, every account it holds, signed in as Outlook is
# profile = "Work"                       # the Outlook profile to log on to; the one running otherwise
```

- **Who gets the tools.** A session of a connection whose key holds the **`mail`** grant: the application on this computer holds
  it, a paired key only if `[host.control] grants` or `[host.native] control` lists `mail`, the Office add-in's pane on this
  computer (see `office_local`) whenever a mail account is named and the tools are on, and a host run for a portal gives it to
  no one. A task another agent handed out, and a session a web page started, never get it. Every session of such a connection gets
  the `canoa-mail` server, whether or not its client asked, along with a built-in skill that says when to use it.
- **What the agent can do.** `mail_list` names the accounts and their folders. `mail_search` filters by sender, recipient, subject,
  dates, folder, attachments, unread and flagged, and by words (an Exchange account takes Exchange's own query syntax, so
  `budget AND (Q3 OR Q4)` works). `mail_read` reads a message or its whole conversation, and takes the message by its Internet message id when a mail program is showing it (the Outlook add-in knows that id, not the tools' own). `mail_related` finds the conversation, the
  same subject and the same people. `mail_draft` saves a new message or a reply in Drafts. Nothing in the host sends mail: there is
  no tool that does, and the Exchange requests are all saved without sending.
- **What it must ask.** `mail_move`, `mail_archive` and `mail_update` (read or unread, follow-up flag, categories) are put to the
  person as a permission request showing the messages and the change, and nothing happens until they allow it. A session the
  person put in a mode that asks for nothing is not asked, and says so in the conversation, as for any other tool.
- **Mail is data.** A subject, a body and an attachment name were written by whoever sent the message. Every result says so, strips
  what could hide or reorder text, and cuts long text; the skill tells the agent to weigh a message and never to obey it.
- **Exchange Server (`ews`).** Requests go through the host's network settings (`[host.network]`) like every other fetch. `server`
  or `email` is required; a plain `http` address is accepted only on this computer, since a password would cross the network in the
  clear. Basic sign-in needs `user` and a password: the one `canoa mail add` keeps in the account store (the operating
  system's credential store, or sealed files where it has none: the same store `canoa accounts` uses), or the variable `password_env`
  names, which wins when both are given. `canoa mail add NAME --backend ews --server URL --user U` asks for the password without
  echo (from standard input when that is not a terminal, never from the command line) and writes the account's table with an `id`
  and no password; `canoa mail list` shows each account's name, backend, server and whether its password is there, never the
  password; `canoa mail remove NAME` takes the table out and forgets the stored password. A `password` key in canoa.cfg is refused. Settings, Mail in the app does the same (add, Test sign-in, Remove) through the control channel's `mail.setup` grant, which the app's own link holds and a paired key only when canoa.cfg lists it, and the mail tools of a running host follow each change at once. `auth = "windows"` signs in with the person's own Windows logon
  instead, with no user or password: when the server answers `401` offering Negotiate or NTLM, the host answers with the logon
  session (Kerberos, then NTLM) on the same connection, and sends the request again with it. A server that offers only Basic is
  left at its `401`, and the error names what it offers. It needs Windows: elsewhere the sign-in fails with a message. Exchange Online
  turned EWS off for tenants from 1 October 2026 (it ends for all on 1 April 2027): an Exchange Online mailbox belongs to
  Microsoft Graph (`graph`) or to classic Outlook (`outlook`).
- **Exchange Online through Microsoft Graph (`graph`).** `client_id` is required: the application (client) id of an app
  registration of your own in Entra ID, registered as a public client with the delegated permission `Mail.ReadWrite` (the host
  asks for that and nothing else, so the registration cannot send mail) allowed for you or by an administrator. The host never
  uses a Microsoft application's id. It takes the token silently from the Windows (or Office) sign-in for that registration, through
  the Web Account Manager, by Windows PowerShell with a helper script that speaks JSON (so the host itself holds no `unsafe` for
  it), and sends it as a bearer token over the host's network settings. `tenant` (default `organizations`) names the registration's
  tenant, `email` says which Windows account to ask when it holds several, `server` replaces Graph's address
  (`https://graph.microsoft.com/v1.0`; plain `http` only on this computer), and `command` replaces the PowerShell command. When
  Windows would have to ask you to sign in or to allow the app, the account says so and names the two ways out: register the
  app, or use `outlook`. A search with words, a sender, a recipient or a subject goes to Graph's `$search` (KQL, newest first; Graph
  gives no count for it, and the unread and flag filters are applied to what it returns); any other search is a `$filter` with a
  count. Graph is not asked to send anything: drafts are saved and a reply is created, never posted.
- **Classic Outlook (`outlook`).** Windows only. The host runs Windows PowerShell with a helper script that drives Outlook's object
  model (COM) and speaks JSON, so the host itself holds no `unsafe` for it. It reaches every account Outlook holds (Exchange, IMAP,
  Gmail, a PST) as Outlook is signed in to them. Outlook may ask the person to allow programmatic access (an antivirus product or a
  policy decides), and the call waits for the answer up to `timeout_secs` (default 60). `command` replaces the PowerShell command.

## Delegating a task to another agent

With `[host.delegate]` on (the default) and a client that asks for it, an agent is given four tools, `delegate_targets`, `delegate_task`, `task_status` and `cancel`, on a loopback server behind a bearer made for that one session. An agent that takes HTTP MCP servers reaches it directly; any other reaches it through a small stdio server, `canoa mcp-bridge`, that the host starts for it with the address and the bearer in its environment, never on its command line. A task runs as a session of another source in a folder at or inside the delegating session's own, in the delegating session's permission mode or one that asks for more, and everything it asks the person goes to the person on the delegating session. `max_depth`, `max_children`, `max_running` and `sources` bound it; see the host's API header for every setting. A client that says it can show tasks, answer a task's permission request and stop it is told what becomes of each task (`_canoa/delegate/event`) and can ask for a task's updates and stop it; the app keeps each conversation's task list from these, shows a task's request with its title and source, and an "allow for the session" given there covers that task only.

A task may be handed more than its text: `context` `messages` gives it the newest turns of the delegating conversation and `files` the files that conversation's tools touched, bounded by `context_bytes` and never for a task that crosses machines. The person's own Delegate task shows that text in full before anything is sent and gives the task exactly it; an agent's own call may attach either only when the person allowed it in the session's Delegation setup, which also narrows the agents, sets the default and the limit on tasks running at once.

A task may also go to an agent on another machine. `[agents.NAME] a2a = { url = "ws://HOST:PORT", peer = "PEERID", source = "SOURCE" }` names it, and it is a source only when `[host.delegate] sources` lists it, since the task's text leaves this machine and only the person's own sessions may send it. The host proves its identity to that key alone, sends the text and nothing else (no folder, no history), and shows the other agent's task as its own: its questions reach the person on the delegating session as any task's do, and a link that is lost ends the task as failed. The other host decides what its agent may do, by what it grants this host's key. To serve such a task, a host lists the keys granted in `[host.delegate] a2a = ["PEERID"]`: they reach `a2a/1` over an agent entry's address, see the sources they may use, and run them in the source's first working root, asking the key before each tool the agent wants to use. A grant reaches only the sources whose entry lists that key in `peers` (an entry with no `peers` is open to every paired key, and the grant does not widen that) and that have a working root. It lets the key run tasks on them and answer their questions (the one-time allow or a refusal, never an "always"), and granted keys together never take the last of `max_running`. When the key's link ends, its tasks are cancelled.
