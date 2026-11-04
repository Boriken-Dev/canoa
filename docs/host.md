# The host

`canoa-host` serves agents to Canoa's chat on this machine. Each entry of Canoa.cfg's `[host]` table is one agent, on a
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

[host.providers.python.source_settings]
dirs = ["python/echo"]                                     # directories of manifests, each <name>.guatiao
paths = ["python/guatiao", "python/coqui_acp_provider"]    # what goes first on sys.path: Guatiao's binding, the kinds
isolation = "shipped"                                      # optional: shipped (the default), isolated or inherit
home = "python/runtime"                                    # optional: the Python installation it runs from
import_dirs = ["python/runtime"]                           # optional, Windows: where python3.dll is found
```

`library` and `source` exclude each other, and `source_settings` goes with `source` alone. Relative paths in `library`,
`source`, `home` and the `dirs`, `manifests`, `paths` and `import_dirs` lists are made absolute from Canoa.cfg's directory
when the file is read; the provider's process runs in that directory, so any other relative path in `source_settings` is
taken from there too. The source reads the rest; it refuses a value it does not take when a connection loads it.

A manifest is named `<name>.guatiao`, or `<name>.guatiao.<major>[.<minor>]`; a file named `*.guatiao.toml` is not read. It
is TOML unless its first line is `#!json` or `#!yaml`.

`isolation` decides how much the interpreter takes from outside these settings:

| `isolation` | What it reads | What it ignores |
| --- | --- | --- |
| `shipped` (the default) | what ships with the program: a `._pth` or `pyvenv.cfg` beside `canoa-host`, on Windows `canoa-host`'s own directory, `home`, the installation's `site-packages`, `paths` | `PYTHONPATH`, `PYTHONHOME` and every other `PYTHON*` variable, the user's own `site-packages` |
| `isolated` | the standard library of `home` (else of the installation `python3.dll` came from) and `paths`, nothing else | everything beside `canoa-host`, the installation's `site-packages`, the environment |
| `inherit` | the environment as well, as a `python` started from the host would | nothing |

`inherit` is weaker: whoever sets the host's environment chooses the code a library runs. `canoa-host serve` says so in its
log when it starts, once per entry: `weakened: host.providers.NAME.source_settings.isolation is inherit, looser than
shipped`. The first source a process adds decides `isolation` and `home`; each connection is a process of its own.

What a provider prints, to standard output or standard error, goes to the host's log, up to 1 MiB per connection, and
never to the client: the host and the provider's process speak ACP over a loopback socket of their own, which the process
proves with a one-time token. Its standard input is closed once the settings are read.

On Windows the Python's `python3.dll` has to be found: name its directory (`sys.base_prefix`) in `import_dirs`, or put it
on the host's `PATH`. On Linux and macOS the source links that Python's shared library, found where the loader looks, or
through `LD_LIBRARY_PATH` or `DYLD_FALLBACK_LIBRARY_PATH`; `import_dirs` is ignored there.

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

Settings are read from Canoa.cfg only, never from a client. They reach the provider's process on its standard input, not
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
- **On a command line**: `serve` gives the entry, as JSON, to `canoa-host tool` on its command line, which other users of
  the machine may be able to read. Put nothing secret in a tool entry; a key belongs in the host's environment, which the
  tool inherits. An entry is at most 16 KiB as JSON and holds no NUL, or Canoa.cfg is refused.
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

`canoa-host serve` reads the same Canoa.cfg as the app (`--config`, then `CANOA_CONFIG`, then the platform's place) and
prints one JSON line per entry: its name, kind, address, the host's peer id and the method it proves by. The app keeps `[host]` as it is when it
saves its own settings.

## In the desktop app

Canoa's desktop app runs the host as its own child, so nothing else has to be started:

- It runs the `canoa-host` named by `CANOA_HOST`, else the one beside its own executable, where a desktop build puts it.
  Without one, the app hosts nothing and says so.
- It starts `canoa-host serve --until-stdin-closes` when Canoa.cfg has a `[host]` entry, and again whenever its hosting
  dialog changes one. If the host ends on its own, the app starts it again after a pause that doubles from one second up
  to a minute, and back to one second after a run of thirty seconds. A Canoa.cfg the host refuses (status 2) is shown, and
  not retried until the person asks.
- Closing the window leaves the host and its agents running. Quitting ends them: the app closes the host's input, and the
  host ends every agent it started, then itself. If the app itself dies, its end closes that input too.

**Hosting**, in the app's side rail and its tray menu, shows the host's state and its peer id, lists its entries with
their addresses, adds a stdio ACP agent by its command on a loopback port of its own, after `canoa-host check` accepts the
change, and stops hosting one. It lists the keys that may connect, pairs a waiting one and removes one, and shows a PIN for
a program to pair with.

## Pairing

The host proves itself and its clients by the method of this user's `coqui-agentic-peer` identity, Noise or WebCrypto,
with the trust store `coqui-acp-bridge serve --paired` uses. A home keeps each method's identity in a file of its own:
a home paired before Noise keeps its WebCrypto identity and its paired keys, and `--method` names one when a home has
both. A key is of one method, and a client of the other is refused, which the host's log says. The home records the
method named last, or its first identity's, so the app's `serve` and `pair`, which name none, use it. **Canoa's app proves
by WebCrypto today:** a home it reaches must prove by WebCrypto (`canoa-host serve --method webcrypto` once) until the app
proves by Noise. A home with no identity gets a Noise one; to move a WebCrypto home, run `canoa-host serve --method noise`
once and pair each browser again: its Noise key is a new key.

A browser pairs by connecting once:

1. In the browser, give the entry's address (`ws://127.0.0.1:7801`), the host's peer id, which Hosting shows (and
   `canoa-host id` prints), and the method it proves by, which `serve` prints: in Canoa's web connect dialog, or by
   adding the agent by hand in the extension's window.
2. The first connection is turned away, and the browser's key waits on the host.
3. Hosting lists it as waiting, as does `canoa-host peers`. Compare its whole id with the one the browser shows (Canoa's
   connect dialog, or the extension's permissions page), then pair it: **Pair…** and **Pair** in Hosting, or
   `canoa-host accept <id>`.
4. Connect again. Removing the key in Hosting, or `canoa-host revoke <id>`, ends its links within about a second.

A program with a native key, such as `coqui-acp-bridge`, can pair by PIN instead: **Show a PIN** in Hosting, or
`canoa-host pair`, shows six digits and an address, and `coqui-acp-bridge pair-dial ADDRESS` with those digits pairs it.
One wrong try ends the PIN, and it pairs by the host's method. `canoa-host pair` refuses a browser, so a page cannot spend
the PIN; a browser pairs with it by connecting once. The extension and Canoa's web connect dialog pair by PIN with
`coqui-acp-bridge pair-accept --method noise --origin ORIGIN`, the origin each of them shows.

A key's first use is always the person's decision: nothing is accepted automatically, and the app pairs or removes a key
only from a dialog that shows its whole id.

## What the host allows

- **Nothing beyond loopback.** An address that is not loopback is refused. The proof does not depend on TLS, so a proxy
  that ends TLS can carry a connection further and can neither read nor change it.
- **Nothing without a paired, proven key.** Every message after the proof is sealed in the session it ended in. An
  entry's `peers` narrows it to some paired keys.
- **No program a client names.** ACP's `session/new` can list a stdio MCP server, which an agent starts as a program
  on this machine without asking anyone. The host applies [the page scope](https://github.com/Boriken-Dev/coqui-acp/blob/main/docs/browser.md#scope)'s
  rule to a stdio agent's clients: such an entry is refused whole, an `http` or `sse` server must be `https`, and `_meta`,
  which an agent may read as launch options, is removed. A provider's facade takes no MCP server at all.
- **Only the working roots it names**, when an entry names any, compared as written: a link inside a root that points
  outside it is followed.

## What it trusts

- **A paired key** gets the agents its entries offer, which act with this user's rights as far as each agent lets them.
  An agent's own permission requests still go to the chat.
- **A provider library is native code.** Loading it runs it, with this user's rights, before anything can check what it
  does. It is mapped only in the process serving one connection, so a crash ends that connection alone, but it is
  trusted as fully as a program you run, and so is whoever can write it or its directory.
- **A provider source runs code too, and loads more.** Python's source trusts, besides its own file: the Python whose
  shared library `import_dirs` or the system finds; that Python's standard library, and unless `isolation` is
  `isolated` its `site-packages` and their `.pth` files and, on Windows, `canoa-host`'s own directory with a
  `canoa-host._pth` or `pyvenv.cfg` there; `GUATIAO_LIBRARY`, which its process inherits; every directory in `paths`;
  every manifest in `dirs` and the directories it names; and every module those import. With `isolation = "inherit"` it
  also trusts your own site-packages and the host's `PYTHONPATH` and `PYTHONHOME`. Whoever can write any of them runs code
  as you.
- **On Windows a source's imports are looked for more widely.** A library's imports are found beside it, beside the host
  and in the system's directories only. For a source, an import it names itself that those leave missing, such as
  `python3.dll`, is looked for in `import_dirs`, the Windows directory, the provider's working directory (Canoa.cfg's) and
  every directory on `PATH`, in that order: the first `python3.dll` found there is the one that loads. The source is then
  opened with the narrow search again, so its other imports come from where they did.
- **A stdio agent** is the program the file names, with the host's environment and working directory.
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
