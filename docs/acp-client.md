# ACP v2 Flutter session controller

The shared Canoa Flutter interface has a tested **ACP v2 JSON-RPC client** and an explicit pinned-key connection dialog, using the experimental WebCrypto method from coqui-peer. The default PWA and extension chat start on a welcome page, and a new session with no agent source to run it is an offline preview. A remote connection works only when the agent endpoint implements the same method and ACP v2. There is no production pairing workflow or complete agent backend yet; see the [roadmap](roadmap.md).

## Components

| File | Responsibility |
| --- | --- |
| [coqui-transport-web](https://github.com/Boriken-Dev/coqui-transport-web) | Prefer WebTransport and fall back to an explicitly advertised WSS endpoint on transport-opening failure |
| [`app/lib/acp/connect.dart`](https://github.com/Boriken-Dev/canoa/blob/main/app/lib/acp/connect.dart) | Verify expected peer ID, initialize ACP v2 and close a rejected link without retrying on WSS |
| [coqui-acp](https://github.com/Boriken-Dev/coqui-acp) | Correlated JSON-RPC requests and updates, cancellation, session creation and resume, version 1 and 2 |
| [`app/lib/acp/client.dart`](https://github.com/Boriken-Dev/canoa/blob/main/app/lib/acp/client.dart) | No client until the peer is proven |
| [`app/lib/session_store.dart`](https://github.com/Boriken-Dev/canoa/blob/main/app/lib/session_store.dart) | Bind a verified agent to the correct local/remote session, stream transcript fragments and show foreground state |
| [`app/test/acp_v2_test.dart`](https://github.com/Boriken-Dev/canoa/blob/main/app/test/acp_v2_test.dart) | In-memory agent tests, peer-proof rejection, transport downgrade rules and message deduplication |

## ACP v2 semantics

After independently verified peer authentication, the client sends `initialize` using protocol version 2, `capabilities: {}`, and Boriken Dev's Canoa client information. It rejects agents not advertising v2 session support.

Session creation requires an absolute workspace path and sends `session/new`. Existing remote sessions can be attached via `session/resume`, optionally asking for a full transcript with `replayFrom: { "type": "start" }`.

`session/prompt` returns the **ID of an inserted user message**. This does not mean the agent has finished the turn. The UI consumes `session/update` notifications, including `agent_message_chunk`, full `agent_message` replacement, and `state_update` with `running`, `requires_action`, or `idle`. The agent-issued message ID is used to prevent duplicate transcript entries across chunk and full-message updates.

Every `sessionUpdate` kind in the v2 schema has a place on screen. A `plan_update` of `items` replaces the plan panel, a `tool_call_content_chunk` adds one item to its tool's card, a `notice` shows in the conversation as the agent's word with its severity, and `compaction_update` and `compaction_summary_chunk` build one entry per compaction with its summary folded away. A message or thought sent without `content` patches the one with its ID and leaves its text. A replayed user message's `resource`, `resource_link`, image or audio blocks are named in a notice under it, as a message sent with attachments is.

`session/cancel` is a notification, not an RPC expecting a turn-completion response. Agent completion is still delivered as a later idle state update.

## Peer authentication boundary

A verified transport must be produced by a callback that **cryptographically proves the expected peer identity** and answers with a channel tied to that proof, since a plain channel after a proof belongs to whoever carries it. A claimed `peerId` string, TLS hostname, or an unverified server hello is never sufficient. The ordinary ACP tests inject a fake verifier for protocol testing only. The browser method is now sourced from [coqui-peer](https://github.com/Boriken-Dev/coqui-peer/tree/main/methods/webcrypto), bundled unchanged with a source-blob hash test, and driven via a transport-level staged message queue.

Canoa's implementation intentionally **does not** connect the standard UI to unverified remote endpoints. A rejected peer proof closes the link and cannot trigger WebTransport-to-WebSocket downgrade. The native Rust responder and JavaScript↔Rust handshake tests now live in coqui-peer, with Coqui TrustStore-based acceptance and revocation. `canoa serve` with the `stdio_agent` example exercises a real socket, Coqui proof and v2 session messages. Missing production work is agent-host pairing UI, persistent keys/sessions, production agent adapters and real remote MCP tools. The echo fixture does not execute or represent an AI agent.

The client negotiates: it requests v2 and accepts v1, whichever the agent selects.

## Permission requests in the chat

A `session/request_permission` shows as a card in its conversation with the tool, what it would run or touch, and one button per
option the agent offered, under the agent's own name for it: refusals first, then kinds Canoa does not know, then what allows,
once before always. An option sent without a name is named by its kind. The answer selects that option's `optionId`. What an
`allow_always` or `reject_always` answer remembers belongs to the agent, which the card says when it offers one: Canoa keeps
nothing of it and cannot undo it. A request offering no option Canoa can read can only be cancelled, and one whose options repeat
an id is answered cancelled without being shown, since the agent could take that id for either. A conversation holds at most 64
questions waiting; the agent's further ones are answered cancelled, and the conversation says so.

While a card waits, the composer's placeholder reads "Send a message to decline and redirect the agent": sending refuses that one
request with the agent's `reject_once` option (any other refusal, else a plain one) and sends the text, which goes at once or, while
the turn is still open, when it ends. A refusal chosen on the card itself stops the turn with `session/cancel`; a subagent's request
is refused alone.

**Modes.** The mode a session is in shows at the composer's bottom left, as Claude Code's and Codex's own do, and is changed
from its menu or with Shift+Tab in the message, which goes on to the next mode and round. Nothing is asked: choosing a mode is
the whole act. The control offers the agent's own modes (the `mode` option, or an agent's `modes`), read from the agent and
never assumed; `canoa agent claude` and `canoa agent codex` offer Canoa's five where their tool has them: Manual, Accept edits,
Auto, Plan and Bypass (the table of what each sets is in the translators' headers, in coqui-acp). A session keeps its own mode
in the session list, so a session opened again, or after the app restarts, is put back in it before anything else is sent; Settings'
**Mode of a new session** (`[permissions] default_mode`) is where a new one starts, and a profile's mode wins over it.

Shift+Tab goes on from the mode last asked for, so quick presses do not repeat one, skips a mode the agent refused, leaves Bypass
like any other mode, and does nothing while the session connects or is being opened again.

**Bypass** asks for nothing. Where the agent has a mode that asks for nothing (`bypass`, `bypassPermissions`), that is the one
chosen. Where it has none, Canoa provides it by answering the agent's requests itself: every request that offers an allow is
answered, with `allow_once` when offered, else `allow_always` (Settings' **Bypass answers with allow once only**, `[permissions] allow_once_only`, leaves a request that offers only an "always" to the person), and each answer is said in the conversation. Canoa's Bypass sits on the agent's own mode, which stays as it was: leaving Bypass returns to it, and the session list keeps both,
so a reopened session is put back in that mode and in Bypass. When the agent is itself in a mode that asks for nothing, or says
so by itself, Canoa answers nothing: what the agent still asks is the person's. A request that offers no allow, a question
and a plan are still the person's, and are shown with that said. A request already waiting when Bypass is chosen is
answered too. A request reaches only the conversation of the connection that sent it, whatever session id the agent chose. While
Bypass is on, the mode control shows in the accent colour and a banner and a mark on the session's row say so; the banner's
**Leave Bypass** ends it in one action. It is the chat's answer to the agent's requests and nothing else: it never reaches the
browser extension's grants for pages, tabs or tools, which stay [their own](acp-permissions.md#independent-browser-grants).

**The card.** A request waiting is a card in the conversation under a "Needs your input" chip. Its title asks the question
("Allow Claude to edit calc.js?") with the reason the agent gave under it, and what it would do sits below at the card's full
width: an edit as a diff, a command or other input as text. **Deny** and a split primary button end it: the primary is the
agent's "always" answer where it offers one (else its allow-once), and its menu holds the allow-once answer and anything else
the agent offers. Ctrl+Shift+A allows once and Ctrl+Shift+R denies from anywhere in the window; the hints show on a wide card.

**Always allow here.** A card for a command, a change to one file or an MCP tool also offers a button that names its scope,
`Always allow here: <folder>`, beside the agent's own answers (and only when the agent offers an answer that allows once). It
opens the rule to read before anything is kept:

- A command is allowed exactly, that command and no other: there are no prefix rules. Only a command of printable ASCII is
  remembered, so nothing that looks like another character (a curly quote, a typographic dash, a Unicode space, a hidden or tag
  character) and no control or line break gets a rule. A command runs whatever the files in the folder make it do, which the
  dialog says.
- An MCP tool is allowed as that tool of that server with any input, by the structured tool name the agent sends
  (`_meta.claudeCode.toolName`, `mcp__<server>__<tool>`); a title is never read as one. It is offered only for a tool whose name
  begins with a verb that reads (`readVerbs`: read, get, list, search, find, view, show, describe, status, stat, info) and has no
  later word that writes or joins another action (`writeOrJoinWords`: and, or, then, set, create, update, upsert, delete, replace,
  write, put, post, send, push, drop, edit, move, rename, add, apply, commit, merge, save, store, upload, run, start, stop, reset,
  clear, grant, install and the like, so `get_or_create_issue` and `search_replace` get none), and never
  for one of the `execute` kind or whose server or tool name says it runs things (run, exec, process, shell, terminal, eval, bash,
  command, cmd, spawn, script, invoke, powershell, pwsh).
- Commands and tools apply only while the session is in a mode Canoa can see asks about commands (the agent's Manual or Plan);
  in Accept edits, in Bypass, in the mode an agent's own "always" switches to, in any other mode or before the agent has said, a
  request asks as it would.
- A change to a file is allowed for that file or a folder under the session's own, by an absolute path with no `.` or `..` in it
  (the system resolves `..` after links), and only for a session on this computer (`SessionPlace.local`) in a desktop build,
  which can follow links. Nothing is offered for a hosted, peer or browser-provided session, in a browser, or where the session
  folder is the person's home folder, holds it, or touches Canoa's configuration folder (read as written and after links are
  resolved). A path that is relative, starts with `~`, holds a `:` other than a drive letter's (a stream, `C:x`) or is a network
  path is never offered a rule. Below the session folder nothing on the **protected list** is ever covered. The list is `[permissions] protected` in
  canoa.cfg (text with a pattern a line, or an array of such lines), edited in Settings, Permissions with a **Restore
  recommended** action; lines that are also in the recommended list show in the accent colour. When the key is absent the
  recommended list applies (`recommendedProtected` in `protected_paths.dart`): every name that starts with a dot (`.*`: `.git`,
  `.claude`, `.envrc`, ...), build and test scripts and manifests (`package.json`, `makefile`, `pyproject.toml`, `setup.py`,
  `cargo.toml`, `go.mod`, `pom.xml`, `build.gradle*`, `requirements*.txt`, `*.csproj`, `dockerfile`, ...), tool configuration
  (`*.config.*`, `gulpfile.*`, `gruntfile.*`), agent instruction files (`agents.md`, `claude.md`, `gemini.md`, ...) and code a
  build runs (`node_modules`, `vendor`, `venv`, `env`, `__pycache__`, `*.pth`). A list the person writes replaces the
  recommended one entirely, so removing a line takes effect for the rules already made as well as new ones. The syntax is the one
  of coqui-acp-pi's `protected` setting: a line is a pattern; a leading `!` allows what an earlier line protected and the last
  matching line wins; `#` starts a comment; `*` and `?` stand for part of a name and `**` for any number of names; a pattern that
  matches a folder covers what is under it; `~/` starts at the home folder, a drive (`C:/x`) or `/` at the file system, and any
  other pattern matches anywhere (`.env` every file of that name, `a/b` every `b` under an `a`). Names compare whatever their
  case. A pattern without a start counts only below the session folder, so a dot above the folder a person works in is theirs;
  one with a start (`~/.ssh`) also protects a session folder inside it. What stays in code is the mechanism, whatever the list
  says: each segment is read the way Windows reads it first (trailing dots and spaces dropped, so `.claude.` and `package.json.`
  are the files they look like; an 8.3 name with `~1` is refused; a Windows device name such as `CON`, `NUL` or `COM1`, with or
  without an extension, is refused), one with anything but printable ASCII is refused rather than normalised, and `.` and `..`
  are never covered. Links are resolved and the result has to satisfy the same rules inside the
  resolved folder and scope. An `edits` list is read only as entries of `old_string`, `new_string` and `replace_all`; a request
  carrying a patch, a diff or any other key is not covered.

A folder keeps one kind of rule: commands, tools and file changes can each write what another runs, so a second kind is refused for
any source or peer, in one folder, in two where one holds the other once links are resolved, and in two folders of one repository
(the nearest `.git`; a linked worktree or a submodule counts as its main repository). A rule has to cover the very request it was made from (the one the dialog
was opened for), and is kept for one source (the key the session list remembers it under: a preset's or command's label, and
`canoa/<name>` for an entry the host serves), the peer it ran on, one folder (exactly the session's, not what is under it) and the
tool the agent named (`Bash` is not `PowerShell`). A later request that matches is answered with the agent's `allow_once` answer,
never an `allow_always`, and never when the agent offers no `allow_once`; anything else, including input that says more than a
command or a path (a background run, a second path), asks as before.

A request that needs the rules is matched after the rules file is read again, in order with the requests before it, and only if
nothing changed meanwhile: a request that was waiting when the person pressed Stop, closed the conversation, or the connection
ended is answered cancelled, and one that finds the mode, Bypass or the cards different is decided on what they are now. Each rule
made and each use is said in the conversation. Rules are kept in `permissions.json` beside canoa.cfg (in the browser, the origin's
IndexedDB). On a desktop the file is replaced through a temp file named for that write, under a lock that the read-merge-write holds,
so another instance cannot write between; a rule another program added is kept and shown as not made in this app, and a rule
another instance revoked is dropped. A rename another process's open file refuses is tried again for up to two seconds; the read and the
write wait for the lock for as long, and a read that gives up applies no rule (the request is asked). A file that cannot be read or
does not parse applies no rule at all and is not written over, and is read again before the next request, so the rules come back
when it reads again; Settings says so. A write that fails is shown in Settings, together with "Not yet saved": the change, a revoke
among them, is in effect here (a revoked rule is not matched) and is written at the next change or read; if the app quits before
it is, a revoke is lost and the rule is back at the next start. A rule added while the file cannot be read or is not parseable is not
made, so none is kept. A temp file a killed
writer left is taken away by the next write once it is a minute old. Stop answers a request still waiting on the file cancelled at
once. On a desktop `permissions-log.jsonl` beside it gets one JSON line
appended for each rule made, used and revoked: the time, the rule's id, source and folder, and for a command only its program
and only when the command begins with a plain program or path (otherwise `(command)`, which covers an assignment such as `NAME=value`
or `$name=...` before it, a quoted value, and a URL), never an argument; for a path or tool the thing itself. It holds no hash of the text, only grows, and a
browser keeps none. A rule an earlier version wrote that this one does not understand (the prefix rules it once had) is kept,
shown as no longer supported and never applied, until it is revoked. Settings lists the rules under **Permissions** with when each
was made and how often it answered, and **Revoke** takes one back at once. Canoa adds no rule when the person picks the agent's own
"always": that stays the agent's, and when the agent then changes the session's mode, the conversation says it did. Rules are kept
by this app, per client, for every session it shows, wherever the agent runs; they move to the host with the session state when
that moves.

Known limits, said in the dialog too: a hard link to a file outside the folder is not detected; the list of names that are never
covered, and the list of tool names that execute, cannot be complete; and what the agent has itself been set to allow (its own permission rules, rules it remembered) is
outside what Canoa can see, and can make its Manual mode silent.

## The changes pane

**Changes** in a conversation's header, once a tool has changed a file, lists every file the session's completed tools
changed, each once, in the order it was first changed: beside the conversation on a wide window, as a sheet on a phone. A file
changed by before-and-after text shows the change from the first text the agent showed to the last; a patch is shown as it
came, numbered from its hunks, under the file its `changes` (ACP v2) or `+++` line names. A refused, failed or cancelled tool
changed nothing and is left out. Each file has its additions and removals, and a mark of **viewed** that holds until the file
changes again; a viewed file starts folded. Choosing a line asks for a comment and adds `path:line`, the line and the comment to
the message being written, for the person to send.

All of it is what the agent sent, never read from disk, and bounded as the transcript is: a change the agent's text was cut
from says so. A line number counts in the text the agent sent, which is the whole file only when it sent the whole file.

## An agent's questions as forms

Canoa advertises ACP's form elicitation, so an agent can ask the person something (Claude's adapter does for its
`AskUserQuestion` tool). `elicitation/create` shows as a card in its conversation, built from the request's schema: text, one
choice, several choices, a number or a whole number, yes or no, each with the agent's title and description, required ones
marked. **Send answer** is offered only once the answer fits the form; **Decline** refuses to answer and **Cancel** dismisses
it. A question of a kind Canoa does not know is shown as one it cannot answer, and when it is required only Decline and Cancel
remain. Everything the agent wrote is shown as text, with nothing that could reorder or hide what surrounds it, and no link in
it is followed. A question counts as needing the person, with its own notification (`[notifications] question`). Stop, closing
the conversation, or the agent going away answers a waiting question cancelled. URL elicitation is not offered: Canoa opens no
address an agent names. `coqui_acp` reads the form and checks the answer, as `vectors/elicitation.json` in coqui-acp says.

## A new session, started by its first message

A new session is the conversation's empty view: the folder above the message box, and under it the source, Local or
Worktree, the agent's mode, model and effort, the overlays, `+` and Send ([a new session](configuration.md#a-new-session)).
Nothing starts until the first message. Then the app starts the agent, or reaches `canoa` for a hosted one, sends
`initialize`, then `session/new` with the folder, the profile's `systemPrompt`, `additionalDirectories`, `_meta` and MCP
servers, sets each choice made beforehand with `session/set_config_option`, and sends the message. A choice the agent does not
offer by then is said in the conversation and not sent. A start that fails leaves the session as it was, with the message.

## Mode, model and effort beside Send

The agent's `configOptions` (v1's `id`, v2's `configId`) are shown by their ACP `category`: `mode`, `model` and `thought_level`
each have a control under the message, an option without a category is placed by its id (`mode`, `model`, `effort`,
`reasoning_effort`, `thought_level`), and every other select option is behind **More**; an agent with `modes` and no options
has its modes there. A change goes to the agent with `session/set_config_option` (in v2 with `type: "id"`, as v2 requires)
and shows once the agent's answer or a `config_option_update` says so. A value the agent refuses stays as it was, and its
reason is said in the conversation. A change made while a turn runs is sent before the next prompt, and says so meanwhile.
Before an agent has started, the controls list what its source's agent offered when it last ran, kept with the session list,
else only Default. The mode control is at the composer's bottom left (see Modes above), with Bypass beside the agent's own modes.

## Usage

An agent that advertises coqui-acp's usage extension (`dev.boriken.coqui.acp/usage` in its capabilities' `_meta`) gets a control right after the mode: the account's allowance that matters most, with how it refills. It is hidden for an agent that advertises nothing or answers that it reports none, and says **No usage yet** when the agent reports but has nothing yet. The app asks to be told of changes when it initializes the connection, reads the report when a session starts and when a turn ends, and keeps the newest by revision.

- The control shows the bucket with the highest fraction used, an exhausted one first: its name, then `62%`, `120k / 400k tokens` or `$1.20 / $5.00`, then `resets in 1 h 20 m` for a fixed window or `40k tokens/min` for a token bucket. A window whose end has passed says **reset, not yet reported**, since its figures are of the window before.
- It shows in the accent of the scheme from `[usage] warn_permille` used (800 by default) or when the agent itself says `warning`, and in the colour of an error once the bucket is exhausted.
- Its menu lists every bucket, the most pressing first, with the figure and refill, **inferred** where the agent gave only the words of an error, which models it covers, how long ago it was observed, the account, and the last refusal with its retry-after.
- A session's spend (tokens, and cost when the agent reported one; an absent cost stays absent) joins the context figure in the conversation's header.
- The account is the host's: `canoa` tells the agent the entry's name and sets it on every report it relays.
- An agent that reports nothing (a generic ACP agent, a gateway, `agy` without `usage = "antigravity"`) still gets the control once the `canoa` that serves it has seen it refused for a limit. For a stdio agent whose `initialize` answer does not advertise the extension, `canoa` advertises it with the source `inferred`, reads each failed prompt (the error's message and data) and each refusal for `[usage] limit_phrases` (`RESOURCE_EXHAUSTED`, `Error 429`, `rate limit`, `usage limit`, `quota` and a few more when unset; a list replaces them, `[]` turns it off), and keeps one report for the entry: a single exhausted bucket marked **inferred**, which models it covers unknown, with a reset only when the text gave one. It answers `_coqui_acp/usage` itself and tells a client that asked for notices when the report changes. The bucket ends at the reset the text named, else the retry-after it named counted from when the host saw it, else when a turn of that agent ends well; a good turn does not clear one that still has a reset to wait for. An agent that advertises the extension is passed through as it is, and a capacity error such as a 503 is not a limit. The host reads the agent's own words, which may echo what a client sent: a client whose prompt makes the agent fail with its own text can set the entry's shared limit. While the host speaks for the agent it also drops the agent's own `_coqui_acp/usage_update`, and it answers `_coqui_acp/usage` only once the agent's `initialize` answer has shown the agent does not advertise it; a request sent before then goes to the agent.
- An Antigravity entry (`usage = "antigravity"` on its `[host.agents]` entry, which the app sets for `agy acp`) is **reported**, not only inferred: `canoa` reads agy's quota from the loopback port agy's own process opens once a session is open, so the control shows a bucket for each allowance (agy's 5-hour and 7-day windows) with the share of each model's allowance left, spending no turn. `canoa` advertises the usage capability for it with the sources `reported` and `inferred` and with `needs_session`, and the report is the buckets read beside any inferred refusal. It reads when a session opens (again after 2, 4, 8 and 16 seconds while agy has not opened its port), after each answered prompt, and when a client asks and the last reading is over 15 seconds old; a request waits for its reading, and the host's relay never does. A client that asks with no session open makes `canoa` start an `agy acp` of its own in an empty directory under its state directory, open an empty session (no prompt, nothing spent; agy keeps that session on disk but does not list it), read, and end it, at most one at a time and not within 15 seconds of the last. An entry with profiles starts none of these, since the profile's environment is not applied to it. Only the quota is read: the account's name and email, which the same answer holds, are never kept, sent or logged. A reading whose port no longer belongs to the agent's processes is dropped, and so is one whose answer is not agy's.

## What goes with a message

`+` adds files or images, a folder reference, or what the clipboard holds; Ctrl+V (⌘V on a Mac) pastes the same way, and
files dropped on the conversation are added too. An image on the clipboard becomes an image, copied files become files, and
text stays in the message unless it is longer than `[attachments] paste_as_file`, when it goes as an attachment of its own.
Each attachment shows as a chip above the message, an image with its thumbnail, with its size and a way to take it back.

**`@` names files.** In a session on this computer, an `@` at the start of the message or after a space lists files and folders
under the session's folder, by name or path, nearest first; a pick writes `@path` in the message and adds a chip. A mention
always goes as a `resource_link` to the file's path, never its text, and only while the message still holds its `@path`. Nothing
outside the folder is listed or taken: a path that climbs out of it, is absolute, or reaches outside through a link is refused,
and names with control or direction characters are not offered. In a git repository the list is what `git ls-files` gives by the
repository's own rules (tracked, and untracked that no ignore rule covers; git is started with no shell and no `core.fsmonitor`);
elsewhere the folder is walked, at most 8 levels deep and 5000 entries, with no list of names left out. A session on
another computer has no `@`, since the agent could not read its links. The placeholder names what the message takes ("@ files,
/ commands") and is one line on a phone.

**`/` lists two groups.** The agent's commands, then Canoa's own actions (the next mode, Settings, the source settings,
Loadout, and what the session was given), each under its title. An action runs when chosen and sends nothing. `+` has Slash
commands, which starts the list from an empty message.

An image goes as ACP `image` content when the agent's `promptCapabilities.image` says it takes images; a file goes as an
embedded `resource` when `embeddedContext` allows it, it is text and no larger than `[attachments] embed_limit`, and as a
`resource_link` to its `file:` URI otherwise; a folder always goes as a `resource_link`, which ACP has every agent take. An
image with a file behind it goes as a link to an agent that takes no images. One that cannot go at all, such as a pasted
image for an agent without images, is refused when it is added, and a chip says how its attachment will go when that is not
as it is: nothing is left out of a message without saying so.

## Selecting and copying

The conversation is one selection, across messages, tool cards, code and diffs; the plan and the changes pane are selections
of their own. Ctrl+A (⌘A) in the conversation selects all of it and Ctrl+C copies it, as text in reading order: an agent's
Markdown as the text it shows, a code block as its code, a folded thought or summary as its heading. The context menu has
Copy and Select all. Each message and each code block has a copy button. In the message being written, Ctrl+A selects only
its text, and the text field keeps its own Cut, Copy and Paste.

Everything else is selectable too: the side list, the header, the settings, every dialog and sheet, and each page opened over
the screen (the overlay store) is a selection of its own, so Select all inside a dialog never reaches the screen behind it.
A drag never runs from one selection into another (the screen, the conversation, the plan, the changes pane). Not selectable
on purpose: menus (a menu item is a command) and tooltips.

## A queue while the agent works

A message written while a turn runs (Enter, or **Queue** beside Stop) waits in a queue under the conversation, with what goes
with it, instead of reaching the agent mid-turn: ACP has no way to add to a running turn. Each queued message can be edited or
removed. When a turn ends as it should, the first is sent, and the next waits for that turn to end. When a turn is stopped or
fails, the queue is held, not dropped, and says why: **Send next** goes on with it and **Clear queue** removes it. A
conversation holds at most 20 queued messages.

## Keyboard and the command palette

Ctrl (⌘ on a Mac) with: **K** the command palette, **N** a new session's view, **1** to **9** that session in the list, **Tab** and
**Shift+Tab** the next and previous session, **L** the message being written, **.** Stop, **Shift+A** and **Shift+R** the
agent's own allow-once and refuse-once options for the question shown (never an "always" one), **Shift+D** the changes,
**A** and **C** select all and copy, **V** pastes into the message, and **/** a sheet of all of these. Up in an empty message brings back the last message sent in that conversation. The palette lists
what the app can do, the other sessions to go to and the agent's own commands; typing narrows it, and Enter runs the first
left. A command chosen there is written into the message, not sent.

## Verification

```sh
cd app
flutter analyze
flutter test test/acp_v2_test.dart
flutter test
```

References: [ACP v2 initialization](https://agentclientprotocol.com/protocol/v2/initialization), [session setup](https://agentclientprotocol.com/protocol/v2/session-setup) and [v2 migration](https://agentclientprotocol.com/protocol/v2/migration).

## Message handoff

`coqui_web` owns the bounded queue of complete messages between connection setup and application use. Its `StagedChannel` holds early messages. Neither authenticates the peer or interprets the WebCrypto proof. The authenticated connection is handed to ACP **only after** Coqui succeeds.
