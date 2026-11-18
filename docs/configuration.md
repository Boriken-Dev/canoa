# Desktop and canoa.cfg

canoa.cfg is one file in TOML, YAML or JSON, chosen by its first line and read through [anycfg](https://github.com/jose-pr/anycfg-rs). The app and the `canoa` host read it the same way and find it in the same places.

The default template is [examples/config/canoa.cfg](https://github.com/Boriken-Dev/canoa/blob/main/examples/config/canoa.cfg). [canoa-yaml.cfg](https://github.com/Boriken-Dev/canoa/blob/main/examples/config/canoa-yaml.cfg) and [canoa-json.cfg](https://github.com/Boriken-Dev/canoa/blob/main/examples/config/canoa-json.cfg) hold the same settings in the other two formats.

## The format

The first line names the format: `#!toml`, `#!yaml` or `#!json` (`#!yml` is YAML too, and a path such as `#!/usr/bin/env yaml` counts by its last word). A file with no such line is TOML. A file whose name ends in `.toml`, `.yaml`, `.yml` or `.json` is read as that format and its first line is not looked at; any other extension but `.cfg` is refused. A first line naming another format is refused, saying which formats are read.

Everything this page shows in TOML is the same in the other two: tables are maps, `[host.agents.claude]` is `host: {agents: {claude: {...}}}`. YAML and JSON have a null where TOML has none: a key holding one is the same as not being there, and one inside a list is refused. A TOML date or time is refused in every format; write it as text. The top level is always a table. A file that is malformed is refused naming the key (`host.agents.a.max_peers: expected usize, found a string`) or the line, and never quoting a value from the file.

The app writes the file back in the format and with the first line it was read with, keeping the tables Settings does not edit. A YAML or JSON file is written again as a whole, so its comments and layout are not kept; a TOML file keeps its own text.

## Where it is found

canoa.cfg is read from up to three files, lowest layer first, and merged:

| Layer | File |
| --- | --- |
| the installed copy | `canoa.cfg` beside the program (the folder of the `canoa` or app executable; for the portable `CanoaUI.exe`, the folder holding that file) |
| the system's | Windows `%PROGRAMDATA%\Canoa\canoa.cfg` (`C:\ProgramData` without it), elsewhere `/etc/Canoa/canoa.cfg` |
| your own | Windows `%APPDATA%\Canoa\canoa.cfg`, elsewhere `$XDG_CONFIG_HOME/Canoa/canoa.cfg` (an absolute path) or `~/.config/Canoa/canoa.cfg`; macOS uses `~/.config/Canoa/canoa.cfg` too |
| PWA / extension | canoa.cfg alongside the deployed index.html, read-only |

A layer that does not exist is skipped, and each layer may be in any of the three formats. A layer that is malformed is refused, naming its file. The installed copy is where an application ships its defaults, the system's file is for a machine's administrator, and the last is yours.

`--config` (a `canoa` option) or `CANOA_CONFIG` (used when `--config` is not given) replaces the third layer, or all three. Its value is a list of files, split at `;` on Windows and `:` elsewhere (no quoting), the last one highest:

- Without a leading `+`, only the listed files are read: no installed, system or user file.
- With a leading `+` (`+extra.cfg:more.cfg`), the listed files go on top of the installed, system and user files.
- An empty entry is refused, and so is a listed file that does not exist, saying which entry.

`canoa layers` prints the files in the order they are read, lowest first, and the one that is written; the app finds the same ones.

The portable `CanoaUI.exe` runs the app from an unpacked copy, so it sets `CANOA_INSTALLED_DIR` to its own folder; the installed copy is `canoa.cfg` there. The variable can name that folder for any program.

### How layers merge

A higher layer is merged over the lower ones, key by key:

- A key in a higher layer replaces the same key below it, and tables merge by key.
- Arrays replace whole, never element by element.
- Named entries replace whole, by name, never key by key: `[host.agents.NAME]`, `[host.providers.NAME]`, `[host.tools.NAME]`, `[host.profiles.NAME]`, `[host.accounts.NAME]`, `[agents.NAME]`, `[profile.mcp.NAME]`, `[profile.skills.NAME]`, `[profile.env.NAME]`, `[overlays]` and `[catalogs]` entries. A higher `[agents.Claude]` therefore drops a lower one's `[agents.Claude.profile]`.
- A layer cannot remove a key a lower one sets: a null in YAML or JSON is the same as leaving the key out.
- Relative `library` and `source` paths, and the `overlays/` folder, are taken from the directory of your own file.

Settings shows the merged values, says on a row which come from a lower layer ("From the system file") and which of yours override one ("Overrides the system file", with **Reset**, which takes the value out of your file so the lower one shows). The app writes your own file only, and only the keys you changed or already had there; a value from the installed or the system's file is never copied into it. If your file does not exist yet, the first change creates it.

Configuration tables:

- [desktop] start_hidden, close_to_tray, minimize_to_tray, local_host (default true: the app runs its own host; false, or `--no-local-host`, leaves it off)
- [network] prefer_webtransport, connect_timeout (seconds, between 1 and 120)
- [assets] cdn = "none" or "local" (deployment option; font hosting remains a separate build concern)
- [appearance] look (`canoa` or `terminal`), brightness (`dark`, `light` or `system`), density (`normal`, `thinking` or `verbose`, default `normal`): how much of an agent's work a conversation shows. `verbose` draws every step as its own card; `thinking` folds each run of finished tool calls into one line ("Edited calc.js, ran a command +1 −1") and keeps the agent's thoughts; `normal` also hides thoughts and folds a finished turn into "Worked for 1m 29s", with what the agent said after its last step left outside. A step that failed, or waits for an answer, is never folded.
- [host] what `canoa` serves: [host.agents.NAME] and [host.providers.NAME], described in [the host](host.md), and the logins an agent entry may run its sessions on, `[host.accounts.NAME]` with `account_store` and an entry's `accounts`, described in [accounts](host.md#accounts). A credential is never written here: `canoa accounts add` keeps it in the account store and writes only the account's harness, kind and id. The app keeps this table as it is when it saves its settings.
- [sessions] the list of sessions kept across runs: keep (how many, default 200, 0 for none; the least recently used go first), keep_text (whether what was said is kept with each, default false), text_messages (how many of a session's latest messages, default 200), workspace (the folder of a session started with none chosen, a path template; see [a new session](#a-new-session)), while_working (`queue`, the default, or `steer`: what Enter does with a message written while a turn runs; Alt+Enter does the other, and the composer's Steer and Queue buttons are always there), queue_delivery (`merge`, the default, sends everything queued as one message when the turn ends, joined with a blank line; `each` sends one message with each turn)
- [notifications] permission, question, finished: which of these, in a conversation the person is not looking at, reach them as a notification outside the window; each on by default. See [below](#when-a-conversation-needs-you).
- [permissions] default_mode: the mode a new session starts in, one of `manual`, `accept_edits`, `auto`, `plan` and `bypass`, such as `default_mode = "plan"`; unset leaves each agent's own. A session keeps its own choice after that; see [modes](acp-client.md#permission-requests-in-the-chat). An older `full_access` list is read and dropped.
- Rules made by "Always allow here" are not in canoa.cfg: they are `permissions.json` beside it (the origin's IndexedDB in a browser) with a `permissions-log.jsonl` beside it (desktop only), edited in Settings, Permissions; see [acp-client](acp-client.md#permission-requests-in-the-chat).
- [permissions] allow_once_only: `true` makes Bypass answer only with an answer that allows once, so a request that offers only an "always" is left to the person; default `false`.
- [permissions] protected: the paths a remembered rule ("Always allow here") never covers, in gitignore syntax, as text with a pattern a line or an array of such lines; absent, the recommended list applies and a list written replaces it. See [the ACP client](acp-client.md). `[host.store] protected` and `credential_variables` are the same kind of list for the program stores' installs, read by `canoa` ([the host](host.md)).
- [harnesses] the harness each name an agent reports in `initialize` stands for, such as `"my-acp" = "My Harness"`. It extends and overrides the built-in names for Claude Code, Codex, Gemini, Antigravity, Pi, OpenCode and Grok; an agent named nowhere is labelled by its entry.
- [agents.NAME] program and args: a source of your own, an ACP agent the app starts on its standard streams; Settings → Sources adds and removes them. See [a new session](#a-new-session).
- [sources] hidden: the presets, by label, that Settings keeps out of a new session's sources, such as `hidden = ["Antigravity"]`. A preset is never changed, only hidden.
- [attachments] paste_as_file (characters, default 8000): a longer paste goes with the message as an attachment of its own; embed_limit (bytes, default 1048576): a larger file goes as a link to it rather than as the agent's embedded context.
- [voice] backend (default `"system"`, the operating system's speech service, which Windows and macOS have), model, language (a tag such as `"es-PR"`; absent is automatic), device (the input device the service names; absent is the system's default) mode (`"toggle"`, the default, or `"hold"` for push to talk: the microphone and Ctrl+Shift+Space listen while held and stop on release) and acknowledged (the services whose first-use notice about where the audio goes you accepted, which Canoa writes and does not ask about again): where speech is turned into text for dictation and how it is held. Settings, Voice says where the audio goes before anything is recorded; Canoa keeps no recording, and the microphone is on only while a dictation or the page's test runs. A service that fails is not replaced by another one. Three more keys are your own voice profile, which Settings, Voice writes and which goes to the service as hints and nowhere else: accent (in your words, such as `"Puerto Rican Spanish"`), voice (`"female"`, `"male"` or another word) and vocabulary (a list of names and words you say). Nothing is worked out from your audio.
- [speech.NAME] (in the host's canoa.cfg): a speech provider the host runs, so a page and every platform can dictate. `provider` is `speech_command` (a program that reads the audio on its standard input, as a WAV stream or raw samples, and writes the text) or `speech_endpoint` (an HTTP speech-to-text address and model, such as an OpenAI-compatible `/audio/transcriptions`), and the other keys are that kind's settings, which Settings, Voice draws as a form and writes back. A provider is listed in Settings, Voice beside the operating system's service, and `[voice] backend = "host:NAME"` chooses it; where the system has no service of its own (a browser, Linux) and `backend` was never set, a provider made from one of Canoa's offers (below) is used. The program runs, and the address is called, as you; an endpoint's key is never shown again once saved. An argument of a command provider may be `{language}`, `{language_code}` (`es` for `es-PR`), `{vocabulary}`, `{accent}`, `{voice}`, `{rate}` or `{channels}`, each as `{name|fallback}` to say what stands in for an empty one.

  Settings, Voice also offers speech-to-text providers to start from, which the host sets up. **whisper.cpp** (MIT) is a local engine: Install asks the host's Store for the publisher's program (Windows or Linux, x86-64 or ARM64) and one model of its published list (Base in every language is the default; Tiny, Small and Large v3 turbo are there too, and English-only forms), each checked against its SHA-256, shows the files before fetching them, then writes a `[speech.whisper]` table (`offer = "whisper"`) that runs `whisper-cli` on the audio: it is turned into text on the host and goes nowhere else. Another model is another install. **OpenAI** is a named service: Add opens a form for `[speech.openai]` with its address and the model `gpt-transcribe` (the others the service takes are listed to choose from), and your own key; the audio goes over the internet to OpenAI, and the page says so before anything is recorded. **Foundry Local** is offered when its service is running on the host and lists a Whisper model: Set up writes `[speech.foundry]` (`service = "foundry_local"`, `offer = "foundry"`, the model you chose), and each recording is written to a temporary file on the host, handed to the service by its path and removed afterwards, so nothing leaves the computer. The service's address changes with each start, so it is found when you dictate (`service_address` in the table names one instead); Canoa never loads a model, so load it on the Foundry models page, where a Whisper model's page also has Use as speech provider. **OpenAI, account NAME** is one offer for each OpenAI API key account you already have in Canoa: Add writes `[speech.openai-NAME]` with `account = "NAME"` and no key, and the key is read from the account's store when you dictate, so you never type it again and the provider's title says which key pays. A ChatGPT sign-in is not an API key and is listed as not offered, with the reason. A page's key needs `voice` to use a provider, `voice.setup` to add one, and `store.programs` to install an engine; the page says which is missing.
- [say.NAME] (in the host's canoa.cfg): a voice provider the host runs, so a page and every platform can read replies aloud. `provider` is `speech_say_system` (this computer's own voices: Windows' desktop voices, `say` on a Mac, `espeak-ng` elsewhere; the text goes nowhere), `speech_say_piper` (Piper, a local neural engine, with a `program` and a `voices_dir` of `NAME.onnx` files; empty, they are the Store's copy of Piper and its voices, and the text goes nowhere), `speech_say_endpoint` (an HTTP text-to-speech address in the shape of OpenAI's `audio/speech`, with a `model`, a `voice` and a key; the text goes to that address, and for `api.openai.com` its documented models, voices and styles are listed until you give others) or `speech_say_command` (a program that reads text on its standard input, or in an argument, and writes audio, with the `voices`, `models` and `styles` you list). Two providers need no table: `system`, and `piper` once the Store installed it. Each provider lists its voices, models and speeds in Settings, Voice, and says where the text goes before anything is spoken.
- [voice] output (the voice provider that reads replies aloud, by the id Settings gives it: `"browser"` for the browser's own voices, `"host:NAME"` for a `[say.NAME]` provider of the host; absent is this page's own voices when it has them, else the host's operating system), output_voice, output_model, output_style, output_speed (a multiple of normal speech, above 0 and at most 10) and read_replies (`true` reads each reply aloud as it streams in; the switch beside the microphone sets it): how replies are read aloud. Each agent reply has a Play button beside Copy that reads its prose, a sentence at a time, without code blocks or a tool's output; typing or dictating stops voice mode's reading. A conversation popped out into a window of its own, and a child's conversation, have the same Play button, and voice mode reads a child's replies as they arrive while it is shown; a pop-out window plays nothing itself but asks the main window to, so its voice is the same. A provider that fails stops the reading and is never replaced by another. Audio a host provider sends back is played by the page's Web Audio, and on the desktop by Windows' waveOut, `afplay` on macOS and `paplay`, `pw-cat` or `aplay` on Linux, whichever is installed.
- [usage] warn_permille (thousandths, default 800): from this much of a usage bucket used, the usage control beside the mode shows a status; an exhausted bucket always shows as an error. See [usage](acp-client.md#usage).
- [usage] limit_phrases (a list of text; unset means `RESOURCE_EXHAUSTED`, `Resource has been exhausted`, `exhausted your capacity`, `Error 429`, `rate limit`, `usage limit`, `quota` and `throttling_error`): the words that make a failed turn of an agent that reports no usage a limit, read by `canoa` for the stdio agents it serves (ASCII case does not matter). A list replaces the defaults rather than adding to them, and `[]` infers nothing. See [usage](acp-client.md#usage).
- `title` on a `[host.agents.NAME]` entry (text, 1 to 64 characters, not blank, no control characters; optional): what the app shows for the entry in its source picker instead of the entry's name. A provider or tool entry takes none. It is listed beside `harness` (the provider id) in the host's lines and in `/canoa.json`, which is how the app learns both.
- `usage` on a `[host.agents.NAME]` entry with `program`: `"antigravity"` (the only value) makes `canoa` read that agent's quota as `reported` usage buckets, the way [usage](acp-client.md#usage) describes. The app's Add an agent sets it when the command is `agy acp`.
- [overlays], [profile] and `[agents.NAME.profile]`: profiles, described [below](#profiles).
- [catalogs] and [store]: the overlay store's catalogs and limits, described [below](#the-overlay-store); `[store.extension_catalogs]` lists [extensions catalogs](#extensions-catalogs); `[store] skills_search` turns on [searching skills.sh](#searching-skillssh).

The session list is `sessions.json` beside canoa.cfg on a desktop, and this origin's IndexedDB in a web build, a PWA or the extension. Removing a session from it never deletes the agent's own session. It also keeps, per source, what its agent offered (model, effort, mode and its other choices) and took with a prompt when it last ran, and the last model, effort and mode the agent took from you.

### A new session

**New session**, Ctrl+N (⌘N on a Mac) or the palette opens a new session in the middle of the window: a choice of **Chat** or **Workspace** (and, for a workspace, its folder: recent folders, or **Open folder…**) above the message box, and in the row under the box `+`, the **source**, **Local** or **Worktree**, the agent's **mode**, **model** and **effort**, the prompt **cache**, the **overlays** it starts with, and Send. It is in the side list at once, as New, with nothing started: no agent, no connection and no worktree. One left untouched is shown again rather than another made, and a new one starts from the source chosen last, as the kind of session started last (a Chat before any); the recent folders are in the folder menu, and choosing one is for that session only. A chat has no worktree, so **Local** and **Worktree** appear for a workspace only.

A **Chat** is a session with no project: it works in a folder of its own that Canoa makes and never shows in the new-session view, the list or the header, on the first message and never before, and takes away again if the agent does not start. The details panel says Chat and keeps the folder under Technical details. A **Workspace** session works in the folder the person chooses, and cannot start without one. The folder of a chat comes from `[sessions] workspace` (Settings: Where chats keep their files), a path template whose `${name}` variables are put in when the folder is made (Settings shows it with what it makes now); the default is `${documents}/Canoa/scratch/${session}`. The session list records which a session is; a chat opens again in its own folder, and the list grouped by workspace and Earlier conversations put chats under **Chats**.

A conversation's name is what the agent reports, else the first line of the first message. A name the person gives it (Rename) is kept, and no later name from the agent replaces it, including after the session is opened again.

| Variable | Is |
| --- | --- |
| `${documents}` | the person's Documents folder: on Windows the Documents known folder (which follows a redirected or OneDrive Documents), on Linux `$XDG_DOCUMENTS_DIR`, else `user-dirs.dirs`, else `~/Documents`, on macOS `~/Documents` |
| `${home}` | the home folder |
| `${session}` | Canoa's own id for the session, a UUID made with the draft before any agent runs and kept in the session list beside the id the agent reports |
| `${date}`, `${time}`, `${datetime}` | local time when the session's draft was made, so the folder shown is the folder made: `2026-11-11`, `153005`, `2026-11-11T153005` |
| `${handler}` | the harness's `acp_harness` provider id, the same for a hosted and a preset source: `claude`, `codex`, `grok`, `antigravity`, `opencode`, `pi` or a hosted entry's own `harness`; else the source's label |
| `${source}` | the source's label |
| `${host}` | this computer's name |
| `${fqdn}` | a webpage's host name; empty on the desktop |

Only the template places separators (`/` or `\`, either spelling): every value except `${documents}` and `${home}` is made one path segment, its separators, `..`, control and reserved characters replaced by `_`, and a part of the template that a variable leaves empty (`${fqdn}` on the desktop) is dropped. canoa.cfg is refused, naming `sessions.workspace`, when the template names a variable not in the table, leaves a `${` open, or has no `${session}`, so two sessions never share a folder by accident.

A session started from a webpage works in `<Documents>/Canoa/scratch/<site>/<session>` on the machine its agent runs on: one folder for the site, one for each session. The page names the site, not a path: its `session/new` carries `{"site": "<host name>"}` under `_meta["dev.boriken.canoa/site"]` and no folder (the app sends `/`, which the host replaces), and `canoa`, which serves the agent, resolves the folder itself, makes it (private to the user on Unix) and starts the agent there. The folder does not follow `[sessions] workspace`. The host's Documents folder is `XDG_DOCUMENTS_DIR` when the host's environment sets it, on any system, else the one `${documents}` names above. The site is the page's host name in lower case with an international name as punycode, no port, path or credentials, and the host takes it only as written: a name that is not a plain name fails the session with `-32602` and nothing is made. Refused: `..`, a separator, a colon (so a drive letter), a first label Windows reserves for a device (`con`, `nul`, `com1` and the rest, with or without an extension), an IPv6 address, a label that is empty, over 63 characters or has anything but letters, digits and inner hyphens, a name over 253 characters, an upper-case letter or a non-ASCII one; the app shows no panel for a page with such a name. The host's answer to `session/new` carries the same key with `site`, `id` and `folder`, and the session list records them. The extension's panel for the page in front lists that site's earlier sessions, newest first, with **Resume** and **New session**. **Resume** opens the session again, through the browser's provider (which has to hold the agent's grant), with `session/load` or `session/resume` naming the site and the recorded folder; the host takes that folder only when it is one session's own under that site's scratch folder, and makes it again if it is gone. An entry with `working_roots` refuses a site session whose folder is outside them. The host makes no other folder for a page.

A source is a program with its arguments and what else it needs: the host's own sources (Claude Code and Codex through the host's translators, Antigravity as `agy acp`, Grok as `grok agent stdio`, OpenCode as `opencode acp` through the built-in `opencode` harness, Pi; served by `canoa serve --local-sources` to the app that runs it, with no address, and replaced by an entry of the same name in `[host.agents]`), the commands under `[agents.NAME]` (served by the host as its own sources: adding or removing one restarts the host, which ends the sessions open on it), and the agent entries the `canoa` this app runs is serving, as `canoa serve` lists them: an entry added to `[host.agents]`, a `harness` entry naming any provider included, is a source once the host serves it, with no change to the app (the app does not read the entry, and a draft kept on a hosted source gets it back when the host serves it). Settings → Sources lists them with where each program was found (on PATH, else where its installer puts it: `%LOCALAPPDATA%\agy\bin` for `agy` on Windows, `~/.grok/bin` for `grok`), hides a preset, and adds or removes a command. A source whose programs are not on PATH stays listed and says which it found and which it did not; sending to it says so instead of starting.

The first message starts it: the worktree is made, the profile is prepared (its dialog shown only when it has more than a model, effort or mode to approve), the agent started or the host reached, `session/new` sent with the folder, the profile and its MCP servers, the model, effort and mode chosen set, and then the message sent with what goes with it. A start that fails says why, takes back the worktree it made, and leaves the message to send again. Until then the session is kept in `drafts/` beside canoa.cfg, with what is written in it and what goes with it, and comes back after a restart; closing it leaves nothing there or in the session list. The session list notes it once it has started.

Before its source has run, model and effort offer only **Default**, which sets nothing; once it has, they list what the agent offered then, with when. A new session's default is the profile's value, else your last choice for that source. After the start they list what the agent offers, a change is sent with `session/set_config_option` and shown once the agent takes it, and a value it refuses goes back, with its reason in the conversation. A change during a turn is sent before the next message. **Cache** is how long the agent's prompt cache lives, for this session: Short (about 5 minutes, for one-shot and background runs), Long (about an hour, for a session you are active in) or the harness's own Default. Unchosen, a chat session asks Long, or what its entry's `cache_lifetime` in canoa.cfg says; your choice wins and is kept with the draft and the session. It shows only for a source whose provider said what it does with the choice, and that sentence is printed under the message box (Codex's reads "ignored", since its app server has no such setting). Full access shows in the mode control, in red, beside the conversation's banner. The overlays can be changed until the first message; after it the control shows what applied.

### A worktree per session

**Worktree** under a new session's message (only for a folder in a git repository), or **In a new git worktree, on its own branch** in a new session on a saved Canoa host, starts the session in a new worktree of the repository holding the folder, from that directory's current commit. For a new session it is made when the first message is sent, and named after its source. The branch is `canoa/` and the session's title (or its agent's name) in lower-case letters and digits joined by hyphens, then six random hexadecimal digits, such as `canoa/fix-the-tests-3fa91c`; nothing else is taken, so no branch reads as an option, a range or a path step. The worktree goes beside the repository, in `<repository>.worktrees/<branch without canoa/>`, and a `.worktrees` directory already there that is a link is refused. The session list shows the branch, and the session list kept across runs notes the worktree with the session.

- **An agent this app runs**: the app runs `git` itself, with an argument list, its hooks and fsmonitor off, and refuses a repository whose own settings name a filter, an include or an attributes file, or whose git directory is apart from its work tree, as a submodule's is. Filters in your global settings, such as Git LFS's, run as usual.
- **An agent `canoa` serves**: the host makes the worktree, only inside its entry's working roots, as [the host](host.md#worktrees) describes; with no working root, or `worktrees = false`, it makes none. The browser's provider makes none.

Closing the session ends its agent first, then asks whether to keep the worktree. Everything there in no commit is counted: changed, new, and files git ignores. Removing them is a button of its own that names them, and a different set found before anything is deleted is told again; without that button nothing in no commit is removed. A HEAD left detached at commits no branch holds keeps the worktree. Its branch goes with it only when it holds no commit another branch, tag or remote does not have, and only if no commit came meanwhile; otherwise it stays, so no commit is lost. A session list entry is taken only when its worktree is where Canoa puts that branch beside that repository. Reopening a session from the list finds its worktree again, even with its HEAD moved, or says it is gone and does not start the agent.

Do not store identity private keys, tokens or passwords in canoa.cfg.

## Profiles

A profile is what an agent is launched with: environment variables, an initial context, skills, MCP servers, and the
model, effort and mode it starts with. It comes in layers, merged lowest first:

1. **built-in**: Canoa's own: the [standard access](#access), and nothing else;
2. **overlays**, each named in `[overlays]`: [AOS](https://github.com/Boriken-Dev/agent-overlay-spec) overlays, in their composition order (`priority`, then name);
3. **yours**: `[profile]` in canoa.cfg;
4. **the project's**: `.canoa/profile.toml` in the working directory, only once you trust that directory;
5. **the agent's entry**: `[agents.NAME.profile]`, where `NAME` is an agent's label (an entry with only a `profile` names
   a preset, such as `Claude`);
6. **the session** being started: the profile chosen for it in its footer (`[profiles.NAME]`, below), and what was changed
   from it.

`[profile]` is **yours**: every session gets it, whatever profile it is given. A named profile is the session's own values
and sits above it. It holds `source`, `model`, `effort`, `mode`, `access`, `context` (shown as "Starting instructions", text added to
the initial context after every other layer's), `cache_lifetime` and `loadout`, all optional:

```toml
[profiles."Deep review"]
source = "Claude"
model = "opus"
effort = "high"
context = "Read the tests before the code."
loadout = "review"
```

It is chosen in a new session's footer. A value changed after choosing it reads "Deep review · Modified", with *Save changes*
and *Save as new profile*; *Effective setup* lists every value beside its layer and what the agent does not offer. The
account and the permissions are chosen apart from it. Settings → Profiles creates, edits, copies and deletes them.

**The default profile** is `[profiles.default]`. It is always there, it cannot be removed, and a session that names no other
profile starts with it: what you set in it (the model, the access, the starting instructions, whatever it holds) is what every new
session gets unless it is given another. Without the table it sets nothing, and naming `default` is never an error. The footer
lists it first, as *Default*; Settings → Profiles edits and copies it but offers no delete, and the host refuses a write of
`[profiles]` that leaves `default` out. A session that names another profile gets that profile's values, not the default's: a
profile is the whole of what it sets, above `[profile]`.

```toml
[profiles.default]
effort = "medium"
access = { sandbox = "none", network = true, approvals = "ask" }
context = "Answer briefly."
```

### Profile items

What a profile may set is not a list in Canoa: each thing is an **item**, a provider of the `profile_item` kind that says its name, the
harnesses it is for and the form it asks for, and that turns the values you gave it into what a session gets (environment
variables, arguments for a program the host starts, values of the session's own settings, the network its programs use). The model,
effort, mode, starting instructions and access are items Canoa carries, kept in a profile under their own keys; a proxy and an
identity are items that ship with the host as extensions, and an extension of canoa.cfg's `[host.extensions]` (or a library or source
an agent entry loads) adds more the same way. Settings → Profiles draws a form for each item the host lists
(`profile_items/list`), and a profile keeps what you entered for an item under `items`:

```toml
[profiles."Behind the gateway"]
model = "opus"

[profiles."Behind the gateway".items.identity]
name = "Ana Ruiz"
email = "ana@example.test"
account = "work"                       # what AGENTS_ACCOUNT says to the session's programs
variables = ["TEAM=core"]              # any other NAME=value

[profiles."Behind the gateway".items.proxy]
mode = "system"                        # system (the default), none, proxy, pac or command
client_certificate_hosts = ["api.corp.example", "*.mtls.example"]   # system and pac: hosts the proxy presents your client certificate to
```

A session applies each item its profile holds when it starts, and a started session's profile report lists each (`items.ID`) and what it became, by
the names of the variables and settings and never their values. A variable the profile's own `[profile.env]` already sets keeps that
value, and the report says so. An item for a harness other than the session's, an item whose values do not fit its form, and an item
no provider offers any more (its extension was removed) are not applied and say why; the profile keeps the values, and putting the
extension back applies them again.

**Identity** (`items.identity`): `name` and `email` become `GIT_AUTHOR_NAME`, `GIT_COMMITTER_NAME`, `GIT_AUTHOR_EMAIL` and
`GIT_COMMITTER_EMAIL`; `account` is `AGENTS_ACCOUNT`; `variables` are further `NAME=value` lines.

**Proxy** (`items.proxy`): how the session's programs reach the network, by `mode`:

- `system` (the default): the host runs a proxy on its loopback address for this session alone and gives the programs
  `HTTP_PROXY`, `HTTPS_PROXY` and `ALL_PROXY` for it (and `NO_PROXY` for this computer and `bypass`). It sends each connection
  the way the host's own network does: the system's proxy chosen per destination (PAC, WPAD), logged in to as the host logs in
  (Basic, Negotiate, NTLM) and carried through a CONNECT tunnel, with the hosts on `bypass` (in the `NO_PROXY` form) going
  direct, so a program that only knows a plain proxy behaves as the system does. The programs' address carries a secret of the
  session as the proxy's password, since the proxy answers with your own logon session: another process on this computer that
  lacks it is refused. The proxy ends with the session.
- `none`: the programs are given `NO_PROXY=*` and no proxy: the host adds no system proxy to them, and a proxy variable they
  inherit from you is overridden by it for a program that reads `NO_PROXY`. The host's own requests for the session (the
  knowledge tools' search by meaning and by a model) go direct too.
- `proxy`: the proxy at `url`, `[scheme://][user[:password]@]host[:port]`, given to the programs as it is; `user` and
  `password_variable` (the name of a variable of the host that holds the password) log in to it.
- `pac`: as `system`, choosing the route by the PAC script at `url` (an address, or a file), with `user` and `password_variable`
  for a proxy it names.
- `command`: a program of your own, `command` (the program first, then its arguments, in which `{listen}` is the `host:port` it
  must listen on); the host starts it for the session, waits until it listens, gives its address to the programs and ends it with
  the session.

**A client certificate to an origin** (mTLS) is carried only where you ask, for the hosts `client_certificate_hosts` names (a host,
`*.name` for a name and everything under it, or an address) and only by the `system` and `pac` modes, whose proxy is the host's
own. A CONNECT tunnel carries the TLS the program makes itself, so a proxy cannot add a certificate to it; for these hosts alone
the proxy ends the program's TLS instead. It makes its own TLS to the host first, through the network: the server is verified as
the operating system verifies it and the client certificate your keystores (or `[host.network] client_certificate`) choose is
presented, and a host that cannot be verified, or that wants a certificate you have none for, is the program's `502` with the
reason. It then answers the program's TLS with a certificate for that host signed by an authority the host keeps in its home
(`network/session-ca/ca.pem`, one per home, readable by you alone, made again each year), a certificate of this session's own,
and carries the bytes between. Every other host stays a tunnel, untouched.

The session's programs are told to trust that authority, and nothing else is: it is never added to the operating system's store,
your browser's or any other of your programs'. Its certificate, with the operating system's roots beside it where a program
replaces its roots by a file, is written in the session's private directory, which goes with the session, and named by
`SSL_CERT_FILE`, `REQUESTS_CA_BUNDLE`, `CURL_CA_BUNDLE`, `GIT_SSL_CAINFO`, `PIP_CERT`, `CARGO_HTTP_CAINFO` and
`NPM_CONFIG_CAFILE` (and, alone, by `NODE_EXTRA_CA_CERTS`, which adds to the roots Node carries). Its key is in no variable, no
report and no file a program is given. A program that reads none of these, uses its own trust (Windows' own tools, a browser)
or pins the certificates it accepts fails for those hosts: nothing can be done about that here. A host both named here and on
`bypass` is reached as the network chooses, not directly. What the session's report says of the proxy names the hosts it ends TLS
for.

A later layer wins per key. Environment variables, skills and MCP servers merge by name: a later layer's entry replaces a
lower one's whole and keeps its place, and `remove = true` takes it out. `unset = ["model"]` takes back a lower layer's
`model`, `effort`, `mode` or `context`. **Settings → Profiles** shows the effective profile of each agent, every value
beside the layer it came from, and what each layer removed. canoa reads the same schema
(`canoa_host::profile`); `crates/canoa-host/tests/vectors/profile.json` holds the app and the host to the same merge.
What each harness takes of it, and how, is [below](#applying-a-profile).

A layer may also suggest how a session delegates, with `[delegate]` (an overlay does it with `[harness.ID.delegate]` for the
harness it is read for): `default` is the agent a task goes to when none is named, `sources` the agents a task may name, and
`max_running` how many tasks of the session run at once. It is only a suggestion. The default is used only when the connection
may use that agent, `sources` can only narrow what `[host.delegate] sources` and the connection already allow, and
`max_running` can only be below the host's own limits. What the person sets in the session's Delegation setup replaces the
suggestion key by key, and the setup and the Delegate task dialog say which layer a value in effect came from.

```toml
[profile.delegate]
default = "Claude"
sources = ["Claude", "Pi"]
max_running = 3
```

```toml
[overlays]
team = "https://github.com/example/canoa-profiles.git@main#team"   # an AOS source string, <repository>@<ref>#<path>
mine = { dir = "profiles/mine" }                                   # a directory, relative to canoa.cfg's
# pinned = { source = "...", pin = "sha256:..." }                 # what installing writes: the tree digest it checks
# tuned = { dir = "profiles/tuned", options = { depth = "deep" } } # values for the options the overlay's manifest declares

[profile]
model = "claude-sonnet-4-5"
effort = "high"
mode = "plan"
context = { text = "Answer in Spanish." }      # or { file = "CONTEXT.md" }
unset = ["mode"]                               # not beside a value it takes back

[profile.env.RUST_LOG]
value = "debug"                                # written here: never a secret
[profile.env.GITHUB_TOKEN]
from_env = "GITHUB_TOKEN"                      # this process's variable, read when a session starts
[profile.env.API_KEY]
from_file = "keys/api"                         # a file's text, less one final line break, read when a session starts
[profile.env.OLD]
remove = true

[profile.skills.review]
path = "skills/review"                         # a directory holding SKILL.md

[profile.mcp.files]
type = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "."]
[profile.mcp.files.env.ROOT]
value = "."
[profile.mcp.docs]
type = "http"                                  # or "sse"; url is http:// or https://
url = "https://docs.example/mcp"
[profile.mcp.docs.headers.Authorization]
from_env = "DOCS_MCP_AUTH"

[agents.Claude.profile]
model = "claude-opus-4-1"
```

A project's profile file has the same keys at its top level (`model = ...`, `[env.NAME]`). Each entry takes exactly one form, and
a key the schema does not know is refused. Names: environment variables a letter or `_` then letters, digits or `_`;
skills, MCP servers and headers 1 to 64 letters, digits, `-` or `_`. A relative path is read from its layer's
directory: canoa.cfg's for yours and an agent's, the overlay's, or the working directory for a project.

**An overlay or a project comes from elsewhere**, so it is held to more:

- Its paths must be relative and stay inside its directory: no root, drive, `~`, `:` or `..`, and a file it names for
  a secret is read only if, its links followed, it is still inside.
- It reads any variable of this computer, by `${NAME}` in its text or by a `from_env` or `env` provider reference, as
  your own layers do. An overlay's report lists each such read as `env-passthrough`, and an MCP server it reaches over
  plain `http` as `insecure-transport`; its headers are still sent.
- It may set any variable, including `PATH`, `NODE_OPTIONS`, `LD_PRELOAD`, `HTTPS_PROXY`, a harness's home or an
  endpoint: you chose the overlay, and what it sets is applied. The one exception is the format's own: a name the AOS
  specification reserves for the host, `AOS_*`, `*_OVERLAY_ROOT` and `*_OVERLAY_STATE`, leaves the overlay out whole.
  A variable that changes how programs find code, trust certificates or reach the network (`PATH`, `LD_*`,
  `NODE_OPTIONS`, `SSL_*`, `*_PROXY`, `*_BASE_URL` and the like) is applied and listed in the overlay's report as
  `env-sensitive`; so is one a `pre-env` hook exports.

### Variables every session carries

Before any layer applies, each session starts from a base environment that every layer's `${...}` reads, and an
overlay may set its own variables from them. A layer's own value for one replaces it. Each is set only when it has a
value, and never empty; a value already set in the host's own environment is kept.

| Variable | Value |
| --- | --- |
| `AGENTS_HOST` | `canoa` |
| `AGENTS_HARNESS`, `AGENT` | the harness id: `claude-code`, `codex`, `pi`, `antigravity`, `gemini-cli`, `grok`, `opencode` |
| `AGENTS_VENDOR` | `anthropic`, `openai`, `google` or `xai`; none for Pi |
| `AGENTS_MODEL`, `AGENTS_EFFORT` | the model and effort the session starts on |
| `AGENTS_HARNESS_VERSION` | the version of the program the Store installed for the harness |
| `AGENTS_ACCOUNT` | the account's label, never anything of its credentials |
| `AGENTS_ACCOUNT_KIND` | `subscription`, `api-key`, `gateway` or `local` |
| `AGENTS_ACCOUNT_PROVIDER` | `anthropic`, `openai`, `xai`, `google` or `microsoft-foundry` |
| `AGENTS_PROFILE`, `AGENTS_LOADOUT` | the profile the connection runs and the loadout it started with |
| `AGENTS_SANDBOX`, `AGENTS_NETWORK`, `AGENTS_APPROVALS` | the session's [access](#access): `none`, `workspace` or `read-only`; `true` or `false`; `ask`, `edits` or `never` |
| `AGENTS_ROLE` | `delegate` when another agent started the session as a task |
| `AGENTS_PROJECT_ROOT`, `AGENTS_SESSION_ID` | the session's folder and id, for the hooks that run for it |

The names and the harness ids and vendors are dotagents'. One overlay then adapts to every harness:

```toml
[env]
TEAM_NOTES = "${AOS_OVERLAY_ROOT}/notes/${AGENTS_HARNESS}"
```

reads `notes/claude-code` for Claude Code and `notes/codex` for Codex. A variable the host itself sets for a session is
read the same way, and so are the ones the account the session runs on gives the program: its state folder
(`CLAUDE_CONFIG_DIR`, `CODEX_HOME`) and endpoint (`ANTHROPIC_BASE_URL`), never a credential. The provider applies the
account's after the profile's own variables, so a layer that sets one of them is told in the session's **Profile** line
that the account's value is used.

**Secrets** come from a reference, `from_env` or `from_file`, and are never written into a profile. A reference is
resolved only when a session starts; the view shows the reference and the variable each MCP header or variable comes
from, and nothing logs the value. A `from_file` reads a regular file of at most 1 MiB. Errors name a key, never a
value.

### Access

`access` is the one setting that says how much an agent may do, the same for every agent: how much of the machine a command
may reach, whether it may use the network, and what is asked first. It is set in any of your layers (`[profile]`, a named
profile's `access = { ... }`, `[agents.NAME.profile]`, the session's), each part on its own, and a later layer wins per part:

```toml
[profile]
access = { sandbox = "workspace", network = false, approvals = "edits" }   # any part may be left out
```

| Part | Values | Standard access (the built-in layer) |
| --- | --- | --- |
| `sandbox` | `none`: nothing holds a command back but your own account; `workspace`: it reads widely and writes only in the session's folders; `read-only`: it changes nothing | `none` |
| `network` | `true`, `false` | `true` |
| `approvals` | `ask`: everything that is not read-only is asked first; `edits`: edits run on their own, the rest is asked; `never`: nothing is asked | `ask` |

The names follow what the three command lines share: a sandbox that is off, one that keeps writes to the folder, one that
writes nothing; a network on or off; and the three steps from asking about everything to asking about nothing. Each
harness's own words are in the tables below, and a part a harness has no way to say is
listed as not supported in the session's **Profile** line, with why. An agent that is none of these (a command you add) is
started as its entry says: Canoa passes it no access setting.

| Access | Claude Code | Codex | Pi |
| --- | --- | --- | --- |
| `sandbox = "none"` | no sandbox of its own | `sandbox` `danger-full-access` | no sandbox |
| `sandbox = "workspace"` | its sandbox on, no retry outside it (`--settings`: `sandbox.enabled`, `sandbox.allowUnsandboxedCommands = false`) | `workspace-write` | not supported: the permission gate asks about paths outside the session's folders in every mode but Bypass |
| `sandbox = "read-only"` | the `plan` permission mode (its sandbox has no switch that forbids every write) | `read-only` | the gate's Plan mode, which refuses every change and command |
| `network = false` | its sandbox's network with no allowed host and no asking (`sandbox.network.strictAllowlist`; with no sandbox, `sandbox.filesystem.disabled` leaves only that part); its WebFetch and MCP tools are not covered | `sandbox_workspace_write.network_access = false`; not supported with `sandbox = "none"`, and a read-only sandbox has none already | not supported |
| `approvals = "ask"` | permission mode `default` | approval policy `untrusted` | the gate's Manual mode |
| `approvals = "edits"` | `acceptEdits` | `on-request` | Accept edits |
| `approvals = "never"` | `bypassPermissions` | `never` | Bypass |

| Access | Grok | Antigravity |
| --- | --- | --- |
| `sandbox = "none"` | `GROK_SANDBOX=off` in its environment | `agy acp` without `--sandbox` |
| `sandbox = "workspace"` | `GROK_SANDBOX=workspace` | `agy acp --sandbox`, which restricts its terminal |
| `sandbox = "read-only"` | `GROK_SANDBOX=read-only` | `--sandbox` too; Antigravity does not say its restrictions forbid every write |
| `network = false` | its `read-only` profile blocks its commands' network (on Linux, by its documentation); its other built-in profiles have no network switch, so not supported | not supported: it has no network setting |
| `approvals = "ask"` | its default, which asks | its default, which asks |
| `approvals = "edits"` | not supported: its agent mode has no accept-edits setting | not supported |
| `approvals = "never"` | always-approve for each session (`_meta` `yoloMode`) | `--dangerously-skip-permissions` |

OpenCode has no sandbox and takes the access as one variable, `OPENCODE_PERMISSION`, a JSON object of tool to `allow`, `ask` or
`deny` (a tool set to `deny` is not offered to the model at all):

| Access | OpenCode |
| --- | --- |
| `sandbox = "none"` | no sandbox |
| `sandbox = "workspace"` | not supported: its `external_directory` rule asks about paths outside the session's folder, unless `approvals` is `never` |
| `sandbox = "read-only"` | `edit` set to `deny`, which takes its edit and write tools away; its commands are not restricted |
| `network = false` | not supported: it has no network setting |
| `approvals = "ask"` | `edit` and `bash` set to `ask` (OpenCode's own default allows both) |
| `approvals = "edits"` | `edit` `allow`, `bash` `ask` |
| `approvals = "never"` | everything `allow`, so nothing asks, a path outside the folder and a `.env` file included |

The variable wins over the `permission` of OpenCode's own `opencode.json` for the session.

A variable one of your layers already sets for the program (an `env` entry named `GROK_SANDBOX` or `OPENCODE_PERMISSION`) keeps its value, and the
session's Profile line says the access did not set it.

With nothing set, every agent starts with the standard access, whatever the configuration of its own command line says: no
sandbox, the network on, everything asked. Every mode the agent has is offered in the mode control, Bypass included, and any
paired key may choose any of them. An overlay or a project may not set `access`: a layer from elsewhere could loosen it, so
such a file is refused (`reserved`). `canoa agent claude|codex` takes the same as `--sandbox none|workspace|read-only`,
`--no-network` and `--approvals ask|edits|never`.

### Applying a profile

The host applies a profile to each session of its own sources (the ones in Settings → Sources), its way for each harness;
the app only reports. Once the session is open, the conversation says in one line what applied and what did not, and
**Profile** in its header lists each part, its layer, what became of it and through what. Nothing is applied that is not
listed there. An MCP server of an overlay or a project is off until `[agents.NAME.mcp]` allows it. Before a session starts the host
asks the app which of those servers this session may start, and the app shows a dialog (**Keep these choices** writes
`[agents.NAME.mcp]`, used when the host next starts). An overlay's hooks are off the same way: the dialog lists them beside the
servers, with the events they run for and the digest of the overlay's files, and **Keep these choices** writes
`[agents.NAME.hooks]`. The per-session overlay choice and project trust are not done in the host yet. An agent reached on another machine is
given what that host's `[host.profiles]` offers; with none, its conversation says so.

Claude Code and Codex are run by `canoa agent claude` and `canoa agent codex`, the host's own translators of their stdio
protocols (coqui-acp's `coqui-acp-claude` and `coqui-acp-codex`), with no Node; the desktop app runs them from its own
executable, and a `[host.agents]` entry with `harness` runs them inside the host. Pi is run the same way by coqui-acp's
`coqui-acp-pi` (`harness = "pi"`), which needs only `pi`.

| Part | Claude Code (`canoa agent claude`) | Codex (`canoa agent codex`) | Gemini CLI (a command you add) | Pi (`coqui-acp-pi`) | Another agent |
| --- | --- | --- | --- | --- | --- |
| Environment | the process of `claude` | the process of `codex app-server` | the agent's process | the agent's process | the agent's process |
| Initial context | ACP `systemPrompt`, appended: `claude --append-system-prompt` | ACP `systemPrompt`, appended: the thread's developer instructions | not supported | ACP `systemPrompt`, appended: `pi --append-system-prompt` | ACP `systemPrompt` when it advertises it |
| Skills | a Claude Code plugin, `canoa-profile`, through `_meta.claudeCode.options.plugins`, which becomes `claude --plugin-dir`; a skill is `canoa-profile:<name>` | the Agent Skills directory of a new directory given as ACP `additionalDirectories`, which becomes one of Codex's skill roots | not supported | not supported | not supported |
| MCP servers | ACP `mcpServers`: stdio, HTTP, SSE (version 1), written to the file `claude --mcp-config` names | ACP `mcpServers`: stdio, HTTP, as the thread's `mcp_servers`; skipped by Codex when its own configuration names a server the same | ACP `mcpServers` | not supported: Pi reads its own from its configuration and takes none from a client | ACP `mcpServers`, as it advertises |
| Model, effort | the session's config options `model` and `effort`: the models Claude Code lists and their efforts | the same: the models Codex lists and their reasoning efforts | the session's config option offering that value | the config options `model` and `effort` (Thinking): the models Pi has credentials for | the same |
| Mode | not supported: the translator offers no mode | not supported: the translator offers no mode | the session's config option offering that value | the config option `mode`: Pi asks nobody, so the translator asks, and refuses protected paths | the same |

Plugins from a client are off in `canoa agent claude` unless `--allow-client-plugins` is given, since a plugin's hooks and
MCP servers run with nothing asked. The app passes it, as it writes the only plugin, `canoa-profile`. A host entry allows
them only when it offers profiles, and then only its own: the host removes every `_meta` a client sends.

Which harness an agent is comes from what it says in `initialize`, else from the preset it was started from. A part a
harness does not take is listed as not supported, with why: Gemini CLI, Pi, OpenCode and Grok read context and skills only
from their own directories when they start. What the dialog says is what Canoa expects; the conversation says what the
agent itself advertised.

Antigravity (`agy acp`) takes skills like Codex: it reads `.agents/skills` inside each directory a session gives it as
`additionalDirectories`, so the skills go there. An Antigravity that does not advertise `additionalDirectories` is listed as
not supported, with that reason.

Skills and context are read once, when the session is prepared: an overlay's from the bytes its tree digest covered, never
from its files again; a project's only inside its directory once links are resolved, and a skill directory holding a link
is refused. Skills are written into a new directory under the system's temporary directory, private to you, removed when
the agent ends. The initial context, every layer's together, is at most 1 MiB; a skill directory at most 1024 files and
16 MiB.

An MCP server's environment is resolved as the profile's is: an overlay's text is substituted against that overlay's own
environment, with `AOS_OVERLAY_ROOT` and `AOS_OVERLAY_NAME`; its `env_passthrough` and `headers_env` give a variable only
when one of your layers shares it, and otherwise leave it out and say so. A command with no directory is looked up on
`PATH`, since ACP takes an absolute one; one not found refuses that server. An overlay's server runs in the session's
working directory, not the overlay's, since ACP gives an MCP server none: the dialog says so.

A server from your own layers is on unless you turn it off; one from an overlay or a project is off until you allow it.
**Keep these choices** in the dialog writes them to canoa.cfg, under the agent:

```toml
[agents.Claude.mcp]
files = true                 # on
docs = false                 # off
team = "sha256:..."          # an overlay's or a project's server, allowed at this definition: one that changes is off again

[agents.Claude.hooks]
team = "sha256:..."          # an overlay's hooks, allowed at the digest of its files: any change to them is off again
```

### Overlays

An overlay is an [Agent Overlay Specification](https://github.com/Boriken-Dev/agent-overlay-spec) (AOS) v0 overlay: a
directory whose files take effect by convention (`skills/<name>/SKILL.md`, `CONTEXT.md`, `rules/<name>.md`, the file
`env`), with an optional manifest in one of two forms, never both (an overlay with both is left out). `OVERLAY.md` holds
the manifest as front matter between `---` lines, YAML unless its first line is a format line such as `#!toml`, and its
body, when it holds more than white space, is the overlay's context in place of `CONTEXT.md`, which Canoa then ignores
and lists among what it does not apply. `overlay.cfg` is TOML, or JSON or YAML after a `#!json` or `#!yaml` line. YAML is
read as a restricted subset: a tag, anchor, alias or merge key leaves the overlay out. Canoa reads the
manifest statically and validates it; one it cannot read or that is not valid leaves the overlay out, except that a
problem in `[about]`, store metadata, never does. Canoa applies:

- the environment: the overlay's env files (the file `env` read as dotenv, and `env_files`), `env`, and `paths`, each
  value substituted (`${NAME}` and its operators) when a session starts, in the overlays' order, after every overlay's
  `<NAME>_OVERLAY_ROOT`. A placeholder reads what overlays set and the variables your own layers share; any other variable
  of this computer it names is an error, so no session starts. A secret reference is read only for `provider = "env"`
  and a variable you share; Canoa holds no named or file secrets, so those stay unset when `optional`, and otherwise
  stop the session from starting. A substituted value is at most 1 MiB, expressions nest at most 32 deep, and a pattern
  may take at most ten million steps;
- skills, MCP servers (`env_passthrough` and `headers_env` only for variables you share; no header over plain `http` to
  another host), rules, routing lines and context as initial context, and the `model` and `effort` hints of
  `[harness.<id>]` for the agent's harness (`claude-code`, `codex`, `gemini-cli`, `pi` for the presets);
- `${NAME}` in the overlay's context files (and the body of `OVERLAY.md`) and in the body of each bundled skill, replaced
  when Canoa hands the text to the agent, never in the installed files: `AOS_OVERLAY_ROOT` (a private copy of the
  overlay's verified files, the one its hooks run from), `AOS_OVERLAY_NAME`, `AOS_PROJECT_ROOT` (the folder the session is
  opened in; a skill's body is written once for the connection, so it keeps this name as written) and the overlay's
  options. Canoa gives an overlay no state directory and its scopes no label, so `AOS_OVERLAY_STATE` and
  `AOS_OVERLAY_SCOPE` stay as written. Anything else, `$${` aside, is left as written, and nothing is replaced in `rules`,
  `routing` or a skill fetched from elsewhere. An option's value is the one in `[overlays.NAME] options`, else the
  manifest's `default`; a **sensitive** option is never put in text, and a value of another type than the option declares
  refuses the text that reads it (the report names the option, never the value);
- `requires`, among the overlays `[overlays]` names: a missing one is noted and the overlay applies without it, and
  overlays in a cycle are left out.

What an overlay needs of the computer it runs on (`runtime`: the operating systems it supports, a Python or Node.js of a
minimum version, commands, and the interpreters its scripts name) is checked when a session starts, against the `PATH` the
session starts from and then the Store's runtimes. A satisfied need counts as applied; a missing one is listed as `off`
with why and the Store runtime that installs it (Runtimes: Python, Node.js, Git Bash, PowerShell), and an installed
overlay's card in the Store and **Effective setup** show it with **Install from the Store**, which shows the host's plan
for that runtime (what it downloads, from where, how it is checked) and installs it on your yes, then asks again what the
overlay needs. Nothing is installed without you, and an overlay that needs a program the computer lacks still applies.

An overlay's dependencies and setup are prepared when you install or sync it, on the yes you give on its page, which lists
what will run first: each dependency file (`lib/python/requirements.txt`, `lib/node/package.json`, or the ones `runtime`
names) with the package manager, whether it is pinned and that it fetches from the network, and the setup with its
interpreter, time limit and what it says of the network. Preparing installs the dependencies into an environment of the
overlay's own (`overlays/.environments/NAME/`, a virtual environment for Python and a `node_modules` folder for Node.js,
made beside the overlay and never inside it, so the overlay's digest never covers it), then runs its setup as the
specification's phases say (`install` the first time, `sync` after, with the state directory `overlays/.state/NAME/`), and
records the digest it prepared in `overlays/.prepared/NAME.json`. It runs outside any agent's session, and a step that fails
ends it with the end of what it printed and leaves the record and the earlier environment as they were. Canoa gives every
overlay an environment of its own, so a shared or named `environment` is not shared. A setup that says it needs no network
is not kept from it, and the plan says so. A session puts the packages after the overlay's own `lib/python/` and
`lib/node/`, and says per dependency file and setup whether it is prepared for the version installed; an overlay installed
or updated since then reports them as not done, and its card in the Store has **Prepare…**, which lists what runs and asks
again. Removing an overlay runs its setup as `remove` and takes its environment, state and record away.

The commands an overlay's `scripts` name are exposed as launchers in `overlays/.environments/NAME/commands/`, a folder first
on the `PATH` of the agent's processes (never before the agent's own program, which is found on the host's `PATH`). A
launcher runs the script under its interpreter, found on the host's own `PATH` (Python, Node.js, `sh`, `bash`, PowerShell 7),
with the arguments it was given, in the caller's directory and environment, with no time limit, and ends with the script's
status; on Windows it is a `.cmd` file, so `cmd.exe` reads its arguments before the script does. Two commands whose names
differ only by the case of ASCII letters, in one overlay or two, are exposed by neither, and the report says so. An MCP
server whose `command` has no separator is looked for in the overlay's own `bin/` (and its other `PATH` additions) as well as
on the host's `PATH`, and the agent is given the program's absolute path.

The agent's own program is always the one on the host's `PATH` (and in the Store's runtimes): a folder a profile or an
overlay puts first on the `PATH` of the agent's processes never decides which program the agent is, on any system.

Canoa does not apply yet, and **Settings → Profiles** lists as "not applied by Canoa yet": `mcp.skills`, an MCP server's `timeout` and its working directory (it runs in the session's), other
harnesses' hints. Each line names its overlay. An overlay's own `bin/` goes first on `PATH`, and its `lib/python/` and
`lib/node/` on `PYTHONPATH` and `NODE_PATH`, ahead of the host's own entries, for the agent and the programs it starts.
A `paths` entry that leaves the overlay's folder is refused, and a `paths` entry may set only those three of the variables
Canoa keeps to itself.

From git, the desktop app installs an overlay into `overlays/NAME/` beside canoa.cfg, with `overlays/NAME.json`
recording the source, the commit, each file's git blob id and **the tree digest** of what it wrote, and its history in
`overlays/.cache/NAME.git`. **Install…** and **Check for changes…** in Settings → Profiles fetch the source and show
the commit, the tree digest, every file it adds, changes or removes, and its `OVERLAY.md` or `overlay.cfg`; nothing changes until you
apply, and what is applied is that commit's files, checked against the digest as they are written. Applying **pins**
the overlay in canoa.cfg by its digest, `NAME = { source = "...", pin = "sha256:..." }`: a later check whose files
have another digest installs nothing until you change the pin. Each time profiles are built the installed copy is
digested again, and one whose files changed is
left out until synced. An overlay with no pin, or a `{ dir = "..." }` with none, is reported as `unpinned` each time.
A pin is a SHA-256 over every file's bytes as published, the same on every platform; the commit is kept beside it.
The ref is a branch, a tag, or a whole commit id, which names that commit even if a branch or tag carries the same
name; an abbreviated id is not taken. Only that ref is fetched, one commit deep. An overlay is upstream's copy,
replaced whole on each apply: put your own additions in `[profile]`, which wins over it. An overlay installed from
another source than the one canoa.cfg names is left out until it is synced.

Nothing from a source runs: its files are read out of git's object store, never checked out, so no hook, filter or
attribute of the source applies. A link or a submodule refuses the whole overlay, and so does a file name that is not
an AOS relative path: `..`, `\`, `:`, `"`, `*`, `<`, `>`, `?`, `|`, a control or invisible character, a Windows device
name, `.git` or another excluded name in another case, `~` followed by a digit (a Windows short name), or two names that
differ only in case. Entries under an excluded name (`.git`, `node_modules`, `__pycache__`, ...) are no part of an
overlay and are never installed. Git may fetch only over `https`, `ssh`, `file` with no host, and `http` or `git` at a
whole commit id; never a transport helper such as `ext::`, nor another host's share. It asks nothing in a terminal and
tells Git Credential Manager not to ask; another credential helper you configured may still ask. Git is run from an
absolute directory of `PATH`, never from the current directory. An overlay holds at most 4096 files and 64 MiB, and its
source's listing at most 16384 entries; listing stops as soon as it passes any of them, a fetch that writes more than
256 MiB is ended, and each git command is ended after 5 minutes. A local directory,
`{ dir = "..." }`, is read in place, digested each time, as it is when an agent starts.

An overlay's `[about]` `summary` shows beside it, as plain text. Its `icon`, a PNG or WebP file of the overlay (never
SVG), shows only after its size (512 KiB) and the dimensions its header gives (1024 by 1024) are checked, before it is
decoded; otherwise a placeholder shows. Those limits are the `[store]` settings below. An icon is shown as the still
pixels of its first frame.

#### Skills from elsewhere

An overlay's `skills` may name a skill it does not bundle (the specification's
[skills](https://github.com/Boriken-Dev/agent-overlay-spec/blob/main/spec/v0/skills.md)): by a table with a `source`
(and an optional `ref` and `pin`), or by a bare name that your catalogs' `[skills]` tables resolve. The desktop app
installs them with the overlay:

- **The review names them first.** An overlay's page in the store, and **Install…** and **Check for changes…** in
  Settings → Profiles, list each skill with its source (no user information, no query), its pin or that it has none,
  the catalog that gave the source, and what installing does: fetched, fetched again because the source or pin changed
  from the one you accepted, or kept. A name no catalog offers is **missing** and one two catalogs offer is
  **ambiguous**: both are reported with the overlay naming them, Canoa never picks one, and the overlay applies
  without them. Nothing of a skill is fetched until you apply, and the one yes covers the overlay and its skills.
- **A skill is fetched like an overlay**, from git objects at its ref (its `HEAD` with none), never checked out, with a
  Git that gets none of your credential helpers, SSH agent, askpass programs or settings, follows no redirect to
  another host and checks every object it receives. Over ssh it uses an empty configuration file and an empty identity
  (`-F` and `IdentityFile` both an empty file, `IdentitiesOnly`, `IdentityAgent=none`, `BatchMode`), since OpenSSH takes
  your home directory from the password database, not from `HOME`, and would otherwise offer the keys in `~/.ssh`. Its files are digested before any is read and must match the pin; its `SKILL.md` must begin with YAML
  front matter whose `name` is the entry's. A skill with no pin is fetched and reported `unpinned`. A skill whose
  front matter holds a key other than `name`, `description`, `license`, `compatibility` and `metadata` is reported with
  that key, since a harness may take it as a grant. A pin that does not match refuses that skill only: nothing of it
  is kept, and the overlay still installs. A link or submodule in it refuses it. It holds at most 1024 files and 16 MiB.
  Nothing in it runs, and nothing it names (a manifest inside it, a dependency file) is fetched or installed.
- **Where it is kept.** `overlays/.skills/NAME/` holds the files its digest covers and `overlays/.skills/NAME.json`
  records the source, the ref, the pin, the digest fetched, the catalog, the overlays naming it and whether you chose
  it, under the skill's name in lower case. A later check that resolves to another source or pin is an update, shown
  as one, never applied because a catalog changed. A change of the catalog that gave the name is such a change. A copy
  whose files are no longer those recorded is shown the same way and fetched again only on your yes. A skill another
  overlay already has from another source, or one you chose from another source, is refused, the holder keeping the
  name. A record under `.skills` is read only when it names the skill its own file is named for, and nothing outside
  `.skills` is read or removed for it.
- **A name has one holder.** Skills are planned across every installed overlay and the incoming one: a skill from
  elsewhere never takes a name another overlay's skill holds, a bundled skill of a later overlay takes it from a fetched
  one, and the review shows a skill that would not be delivered, and by whom the name is held; it is not fetched. The
  profile built for a session drops the fetched skill that lost its name, and says so. A source over plain `http` or
  `git://`, or at a private network address, is shown in the review as such.
- **`[skill_pins]`** pins a skill by its name (compared without regard to ASCII case) to a tree digest:
  `[skill_pins]\nreview = "sha256:..."`. What is fetched must equal it and the pin of the entry or the catalog; two
  that differ refuse the skill before any fetch. It anchors a skill named without a pin, which is then not reported
  unpinned, and a copy whose digest differs is not delivered.
- **It is delivered like a bundled skill**, to the agents that take one, after the same checks: the record must list
  the overlay and the files must still have the recorded digest. A skill that fails them, or was never fetched, is
  listed in **Settings → Profiles** as not applied (`skills.NAME: from SOURCE, not fetched: install or sync the overlay in
  Canoa`, or `a named skill; no catalog resolved it here: install or sync the overlay in Canoa`), never silently missing.
- **The store's Skills section** lists each skill here with the overlays that name it (or that you chose it) and its
  source and pin, and the skills your catalogs offer, each installable by your own choice after the same review.
- **Removal.** Removing an overlay offers to remove the skills that only it named; a skill another overlay names stays,
  and one you chose is never offered. Declining leaves it listed as named by no overlay, to remove there.

`canoa` itself fetches nothing: it delivers a skill the app fetched, after the same checks, and reports the others.

### The Store

**Store** in the left column (and **Store…** in Settings → Profiles) is where an agent's programs, overlays, skills and
MCP servers are found, installed and chosen. It has four types, **Agents** (where the host grants programs),
**Overlays**, **Skills** and **MCPs**, behind one toolbar: the type tabs with a count each (a badge on Agents when a
program has a newer version), **Installed** and **Discover**, a search, **Refresh**, **Store settings** and a line that
says how old the catalogs' index is ("Cached · updated 3 min ago", "Stale" when a source could not be refreshed). Each
type keeps its own query, switch and place in the list while you read an entry's details. Every card says where it comes
from (the catalog's label, the repository, the commit and digest it is pinned to), what it needs, and whether it is
installed, and opens its details: what it holds (skills, MCP servers), what it requires, the hooks and environment scripts
it carries (which stay off until you allow them at their digest), and technical details with the digests. Everything is
shown before it is installed. **Store settings** holds the catalogs: each one's source and what it said, and the form that
adds one.

- **Overlays.** Installed: each overlay's icon, summary, source, pin, what it applies (environment variables, skills, MCP
  servers, context) and what Canoa does not apply yet, a switch for whether **new sessions** start with it, an update
  check and Remove (for one the store installed). Discover: the overlays of the catalogs below, filterable by catalog,
  tag and text. With no catalog it offers a known one and a spec to paste.
- **Loadouts.** A loadout is a named collection a session starts with: whole overlays, skills you chose, and MCP
  servers of `[profile.mcp]`, written as `[loadouts.NAME]` (`description`, `overlays`, `skills`, `mcp`; a name has 1
  to 64 letters, digits, `-` or `_`). `[defaults] loadout` names the one a session that names none starts with. A
  session names its own in `initialize`'s `_meta["dev.boriken.canoa/loadout"]`; the host resolves it once, when the
  session starts, so editing a loadout reaches new sessions only and ends none that is open. A name that
  `[loadouts]` does not hold is refused, with the name. Overlays apply whole, in the order `[overlays]` composes
  them: a loadout cannot reorder them, and one it names that `[overlays]` no longer lists is reported, not applied.
  The person's own `[profile.mcp]` servers a loadout does not name are off for the session (an overlay's servers come
  with the overlay); a name it gives that `[profile.mcp]` lacks is reported. With no `[defaults] loadout`, a session
  starts with every overlay and every server, as before loadouts: the first time the app changes what new sessions
  start with, it writes a loadout named `Default` that holds that. The old `[loadout]` table (`off`, `skills`,
  `registries`, `mcp`) is not read: `registries` and the notes on installed servers are `[store] registries` and
  `[store.mcp.NAME]` now.
- **A default loadout for each kind of client.** `[defaults.clients]` names, per kind (`browser`, `outlook`, `word`, `excel`,
  `powerpoint`), the loadout a session starts with when it names none and its client says which kind it is, in
  `initialize`'s `_meta["dev.boriken.canoa/client"]`: the browser extension's side panel and the Outlook, Word, Excel and
  PowerPoint task panes do. It sits between a session's own loadout (and the loadout of the profile it names) and
  `[defaults] loadout`, so a kind with no entry starts with the default. A kind that is not in that list refuses the
  file; a loadout `[loadouts]` no longer holds (deleted by hand) does not: the session gets the default, and its profile
  line says the kind's loadout is gone. The host resolves it when the session starts, so it also applies where the
  client cannot read canoa.cfg, and the profile report names the loadout the session got (`loadout`). Deleting a loadout in
  the app drops the entries that name it, and renaming one carries them.
- **Editing loadouts.** The Loadouts section lists them in the index beside the main area (the default marked, a search when there are many, New loadout at the top) and shows the chosen one there: what it holds, whether it can start, and what a new session gets from it. The choice is kept across a restart. It edits, duplicates and deletes
  them, makes one the default and starts a session with one. Its editor adds installed overlays, skills and servers from a
  searchable list, and leads to the Store for more without losing what was typed. Overlays stay whole: nothing there switches
  an item inside one.
- **The choice in the chat-only app.** The extension's side panel and the Office task panes start a session in one click,
  with no draft to choose on, so the loadout and profile controls sit beside New session: the same radio lists as the
  composer's, the kind's default marked and chosen until the person picks another. What is chosen is sent as
  `_meta["dev.boriken.canoa/loadout"]` and `["dev.boriken.canoa/preset"]` with the client's kind, applies to the next session
  only, and the conversation notes which loadout it started with when the link does not carry the host's report. The
  extension's panel reads canoa.cfg through its worker for the names; a panel that cannot shows no control and the host
  still applies the kind's default.
- **The choice per session.** The composer's loadout control names the loadout a session starts with, sent as
  `_meta["dev.boriken.canoa/loadout"]`, and changes its overlays for this session alone.
  What a session started with is kept with it in the session list and asked for again when it is opened, so turning a
  default off reaches new sessions only. The app tells its host through `initialize`'s
  `_meta["dev.boriken.canoa/overlays"]`, which takes the place of the loadout's overlays; the host applies only those
  of the profile's overlays, and its report names each one left out ("not chosen for this session").
- **Skills.** Installed: the skills fetched, with the overlays that name each or that you chose it, source and pin; one
  you chose has a switch for new sessions, which adds or takes away its name in the starting loadout's `skills`. The host delivers
  such a skill from its record at each session, only when the record says you chose it and its files have the digest the
  record names (and the pin in `[skill_pins]`, if any); otherwise it is left out and the session's report says why. Discover: the skills your catalogs list, the sources you read skills from (see [Skill sources](#skill-sources)), and a spec to paste (`<repository>@<ref>#<skill directory>`). A
  skill is an [Agent Skills](https://agentskills.io/specification) directory: a `SKILL.md` with YAML front matter whose
  `name` is the directory's. It is fetched only on a yes, after its source and pin are shown, and checked against the
  pin before any file is read.
- **MCPs.** Installed: the servers `[profile.mcp]` and your overlays define. Discover: the
  [MCP Registry](https://github.com/modelcontextprotocol/registry) (`registry.modelcontextprotocol.io`, `GET
  /v0.1/servers`), each server a `server.json`. Installing shows the registry's claims, the digest of the `server.json`,
  each way to run it (an npm, PyPI or OCI package, or a remote address) with the command or address Canoa would write,
  and what it needs from your environment: a secret is a `from_env` reference, never written to canoa.cfg. A package
  whose runtime (`npx`, `uvx`, `docker`) is not on this computer, a bundle, a package of another registry type, a
  remote over plain `http` and anything else Canoa does not run is refused with the reason. An installed server is
  written to `[profile.mcp.NAME]`, noted in `[store.mcp.NAME]` (`registry`, `server`, `version`, `digest`) and turned
  off for every agent in `[agents.NAME.mcp]`, so nothing starts until you allow it.
  A team's own servers can be listed by another registry that serves the same API, added by its `https` address (or
  `http` on the loopback address), or by a git repository holding a JSON file in the API's list shape, added as
  `<repository>@<ref>#<file>`; they are kept in `[store] registries` and chosen beside the official one in Discover.
  A git list is fetched at its ref like a catalog and read as one page.
- **Applying a change.** The host reads canoa.cfg's layers again when each session of its own sources starts, so a
  change to `[overlays]`, `[profile]` or `[agents.*.mcp]` reaches the next session and ends none that is open. A file
  that no longer parses is refused and the profile stays as the last good file gave it.

```toml
[loadouts.review]
description = "Reading and reviewing code"
overlays = ["review"]         # whole overlays, applied in the order [overlays] composes them
skills = ["audit"]            # skills you chose
mcp = ["files"]               # servers of [profile.mcp]

[defaults]
loadout = "review"            # what a session that names no loadout starts with

[defaults.clients]            # ... when its client is of this kind: browser, outlook, word, excel or powerpoint
outlook = "mail"
word = "writing"

[store]
registries = ["https://registry.team.example", "https://git.team.example/lists.git@main#servers.json"]

[store.mcp.files]             # where an MCP server the store installed came from
registry = "https://registry.modelcontextprotocol.io"
server = "io.github.ana/files"
version = "1.2.0"
digest = "sha256:..."         # of the server.json that was shown
```

### The network

With no `[host.network]` table every fetch the host makes (the Store's downloads and release lists, `registry/fetch`, the endpoints of `[host.kb]` and of the speech providers, the Foundry Local service) and every program it starts follows the computer's own settings, as a browser on it would:

- the proxy: the `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY` and `NO_PROXY` variables of the host's environment, else the operating system's (Windows Internet Options through WinHTTP, `scutil --proxy` on a Mac, GNOME, KDE and NetworkManager settings elsewhere), PAC scripts and WPAD included, with its bypass list; loopback is never sent through a proxy;
- the proxy's login: Negotiate or NTLM with the logon session on Windows, when the proxy asks for it (`negotiate = false` turns it off); Basic with the login below, when one is given;
- who is trusted: the operating system's certificate verifier, so a TLS-intercepting proxy the computer trusts works;
- the client certificate: when a server asks for one, the one a browser would offer from the operating system's keystores (the Windows certificate store, the macOS Keychain, a PKCS #11 module through p11-kit) that the server's named authorities issued, with the policy Edge and Chrome read;
- git (overlay and skill sources): started with the proxy for the remote it is fetching and, on Windows, with `http.sslBackend=schannel`, so it uses the same certificate store; a git configuration of the person's own (`http.proxy`) still wins where the source is one they named.

The desktop app's own requests (the list a saved connection's host serves at `canoa.json`, its status probe, the terminal check) are sent by the host it runs through `network/fetch`, so they follow the same settings; the app's own library would use none of them. A served page's requests are its browser's.

The desktop app's WebSocket to a remote Canoa host follows all of them too: the app asks its host (`network/tunnel`) to open the connection, and for a `wss` address the host reaches the server through the proxy the system names (logged in as above) or straight, makes the TLS itself (the certificate verifier and client certificates above, with `ca_file` and `client_certificate` over them) and gives the app a loopback connection to the plain WebSocket, which only the app can use. A server whose certificate only the operating system trusts, such as a company or TLS-inspecting proxy's authority, is therefore trusted, and a server that asks for a client certificate is given the one a browser would offer; one the host does not trust is not connected to, and the error says why. A `ws` address is carried through a proxy the system names and opened by the app itself where it names none. This computer's own address is always opened by the app itself. WebTransport, which no HTTP proxy carries, is not used by the desktop app, which dials the WebSocket endpoint.

A program the host starts (an agent, Node or Python, `git`) is given `HTTP_PROXY`, `HTTPS_PROXY` and `NO_PROXY` for that proxy when its environment names none; variables the person or the entry set are never replaced. A proxy chosen per URL (PAC, WPAD) cannot be written as variables, so it is asked about the addresses the programs reach (the package registries, GitHub, the agents' own services; `endpoints` adds to them) and the proxy it names for those is the one given. Each key below is an override and none is needed:

```toml
[host.network]
proxy = "system"                 # system (the default), none, pac:LOCATION (a URL, a file: URL or a path), or a proxy for every URL:
                                 # [scheme://][user[:password]@]host[:port]
proxy_user = "alice"             # a login for every HTTP proxy that names none of its own
proxy_password_env = "PROXY_PW"  # the variable holding its password; the password is never in canoa.cfg
no_proxy = ["corp.example", "*.lan", "10.0.0.0/8"]   # replaces the system's bypass list; "<-loopback>" sends loopback through the proxy too
negotiate = true                 # answer a proxy's Negotiate or NTLM with the logon session (Windows only)
client_certificate = "system"    # system (the default), none, or a PEM file holding a certificate chain and its unencrypted key
ca_file = "roots.pem"            # trust exactly these certificates, replacing the system's
git_tls = "system"               # system (Schannel on Windows) or git (git's own and the person's git configuration)
endpoints = ["https://tools.corp.example/"]   # addresses the programs reach, for a PAC script to choose their proxy by
```

Paths are read from the folder of canoa.cfg. The host starts only with a setting it can use: a proxy it cannot read, a `proxy_password_env` variable that is not set, or a certificate file that is missing is refused at start, with the key named. Git's client certificate is not chosen from the keystore as a browser's is: Git for Windows presents one only when its `http.sslCert` names it, which a source the person typed gets from their own git configuration.

### Skill sources

Skills are looked for the way `npx skills add` looks, in their own list beside `[catalogs]`: `[store.skill_sources]`, a label of yours
for each source written as that command takes it. In Store, Skills, Discover the sources are chips beside Catalogs; choosing one is what
fetches its repository (nothing is fetched by listing it), reads which skills it holds and lists them, and installing one asks first, with
the commit that was read shown, and fetches exactly that commit.

| Written | Means |
| --- | --- |
| `anthropics/skills` | GitHub `owner/repo` |
| `owner/repo/skills/pdf` | a folder in it |
| `owner/repo#dev`, `owner/repo@v1.2.3` | a branch, tag or whole commit (`@` names a skill instead when it is not a version or a commit: `owner/repo@pdf`) |
| `https://github.com/owner/repo`, `.../tree/main/skills` | a URL, or a GitHub or GitLab tree URL |
| `git@host:owner/repo.git`, `ssh://`, `git://`, `https://host/x.git` | any git source |
| `/home/ana/skills`, `C:\skills`, `file:///...` | a folder on this computer |

A link to one `SKILL.md`, an archive, and a site's `/.well-known/` list are not read yet: name the git repository. A source is searched
for `SKILL.md` as `npx skills` searches: the source's own, else the folders under `skills/` and the ones agents read
(`.claude/skills`, `.agents/skills`, ...), else any within five folders. A skill's name has to be its folder's, as the
[Agent Skills specification](https://agentskills.io/specification) says, and one that is not is listed with why it cannot be installed.

Canoa offers `anthropics` (`anthropics/skills`) first. It is fetched with none of your git settings, credentials or keys; a source you
add is yours, so it is fetched as you would from a shell, which reads a private repository with your credentials. To remove the
default, use Remove this source or set `anthropics = false`; giving the label a source of yours replaces it.

```toml
[store.skill_sources]
anthropics = false                                  # leave out the source Canoa offers by default
web = "addyosmani/web-quality-skills"               # a label of yours = a source as above
```

### Searching skills.sh

[skills.sh](https://skills.sh) ranks skills by how often `npx skills add` installed them, and has a search. Canoa can search it for you, but
it is off until you turn it on, because skills.sh is a third party and its search address is not documented. Turn it on in Store settings
(Skills, Search skills.sh) or in canoa.cfg:

```toml
[store]
skills_search = true                       # skills.sh; remove the line (or write false) to turn it off
# skills_search = "https://skills.example"  # another directory that answers /api/search?q=...&limit=... the same way
```

With it on, Store, Skills, Discover has a Search skills.sh chip. Only the words you type are sent (at least two characters), through the
same proxy and certificate settings as every fetch the host makes, with no redirect followed. A result names a repository
(`owner/repo`) and a skill in it, with how many installs skills.sh counted; nothing it sends is a file. Installing one reads that
repository the way any source is read (fetched with none of your git settings, credentials or keys, its `SKILL.md` files listing what it
holds), shows the commit that was read, asks, and fetches exactly that commit. A result that does not name a GitHub `owner/repo`
is dropped, and one whose repository has no `SKILL.md` for it says so. Turning the search off removes the chip and the results.

### Extensions catalogs

An extension is code the host loads to add to Canoa: today a library that adds store types. Store, Extensions lists the extensions of
the **extensions catalogs** you name in `[store.extension_catalogs]`, a label of yours for each, written as a catalog is in
`[catalogs]` (`<repository>@<ref>#<file>`) but a list of its own: none is offered until you add one, and a catalog of overlays is
never read as one of extensions. Canoa's own format, described here, is not the overlay catalog's.

```toml
[store.extension_catalogs]
team = "https://github.com/example/canoa-extensions.git@main#extensions.toml"
```

The file is TOML in a git repository (at most 4 MiB). Its `[extensions.NAME]` tables are the entries; `name` and `description` at the top
are shown as the catalog's own, and any other top-level key is ignored. A `NAME` is 1 to 64 letters, digits, hyphens or underscores
(it becomes `[host.extensions.NAME]`), and two that differ only in case are both refused.

```toml
name = "Team extensions"
description = "What the team installs."

[extensions.speech-kit]                  # a tree of files in a git repository, pinned by its digest
source = "https://github.com/example/extensions.git"
ref = "4b825dc642cb6eb9a060e54bf8d69288fbee4904"
pin = "sha256:8280aaf5de7c4f61342f648586d534e481ae095cef223dded70f5b1db5bd84a1"
load = "bin/speech_kit.dll"              # the library file in the tree the host loads
provider = "example_speech"
description = "Adds a speech engine."

[extensions.speech-kit.needs]
kind = "store_extension"
version = "1"
platforms = ["windows-x86_64"]

[extensions.speech-kit.about]
summary = "Speech engines, as an extension."
tags = ["speech"]

[extensions.speech-lib]                  # one file at a URL, pinned by its SHA-256
source = "https://github.com/example/extensions/releases/download/v1.2.0/speech_lib.so"
sha256 = "<64 hexadecimal digits>"
provider = "example_speech"

[extensions.local-kit]                   # a folder of this catalog's own repository, at its ref
source = "./kits/local"
pin = "sha256:8280aaf5de7c4f61342f648586d534e481ae095cef223dded70f5b1db5bd84a1"
load = "local.dll"
provider = "example_local"
```

| Key | Meaning |
| --- | --- |
| `source` | Required. A git repository wherever git reads one (a folder of this computer by its path, `file:`, `ssh`, `git@host:path`, `https` with the credentials the address holds), written `<repository>` with an optional `@<ref>` and `#<folder>` (a `git+` before it names git when the address does not end in `.git`); a folder of the catalog's own repository (`./kits/x`, `../x`), at the catalog's ref and from the catalog file's folder; or an `http` or `https` URL of one file, with any query or credentials it needs |
| `ref` | The branch, tag or commit of a git `source` written without `@<ref>` (a branch may hold `/`). The repository's head when neither is given |
| `pin` | Required with a git or relative `source`, refused with a URL: the tree digest of the folder (`sha256:` and 64 hexadecimal digits), the same digest an overlay's `pin` is |
| `sha256` | Required with a URL, refused with a git `source`: the SHA-256 of the file, 64 hexadecimal digits |
| `load` | The library file in the tree the host loads (a `/`-separated path of plain names). Required for a tree by this host, refused with a URL, whose file is the library |
| `provider` | Required. The id of the `store_extension` provider the library offers: letters, digits, `_`, `.`, `-` |
| `needs` | Optional. `kind` (a name; this host offers `store_extension`), `version` (compared as written; this host offers `1`; needs a `kind`) and `platforms` (a list of `windows`, `linux`, `macos` or the same with an architecture, `linux-aarch64`; the host's own OS, or its OS and architecture, must be one) |
| `description`, `about` | Informational: `about` takes `summary`, `icon` (a PNG or WebP file beside the catalog), `homepage`, `repository` (web addresses), `license`, `authors` and `tags`; one that is not valid is left out and the entry is still listed |

Where an extension comes from is yours to choose: the catalog may be a folder of this computer, a `file:` address, `ssh` or an
address that needs your own git credentials, and it is fetched with your own git settings, as an overlay catalog is. A source the
catalog names is not restricted either, and is fetched as follows: one in the catalog's own repository (a relative one) with your own git
settings, one anywhere else with none of them, no credential helper, ssh key or agent, since its author, not you, chose where it points
(the plan says which). What the format refuses, each for a reason it cannot work without:

| Refused | Why |
| --- | --- |
| a name that is not 1 to 64 letters, digits, hyphens or underscores, or two that differ only in case (`invalid-name`, `duplicate-name`) | the name becomes the key `[host.extensions.NAME]` and a folder; two names would share both |
| an entry that is not a table, has a key this list does not, or a value of the wrong form (`invalid-entry`) | a misspelt `pin` or `load` would otherwise be read as no pin or no file |
| a source that is empty, holds a control character, names a scheme neither git nor the download speaks (`s3:`, `ftp:`), or a folder with an empty, `.` or `..` name (`invalid-source`) | it cannot be handed to git or the download, or names nothing |
| a relative source that leaves the repository (`leaves-repository`) | there is nothing there to fetch |
| a `ref` beside a relative source, a URL, or an `@<ref>` of the source's own (`ref-not-allowed`), or a `ref` git cannot be asked for (`:`, `\`, `@`, `#`, `?`, `*`, `[`, `^`, `~`, `..`, white space, a leading `-`) | the source would have two refs or none to put it on, and git would read the others as a refspec or an option |
| a `pin` or `sha256` that is not a digest (`invalid-pin`), a `pin` with a URL or a `sha256` with a git source, a `load` with a URL (`invalid-entry`), no digest (`unpinned`) | the digest is the install's integrity, not a restriction: what is fetched is checked against it before anything is kept, so an entry that gives none, or one its source cannot be checked by, has nothing to check |
| a `needs` this host does not offer: another `kind` or `version`, or other `platforms` | the library cannot run here |
| a tree with no `load` | this host loads one library file; with no name it cannot choose |
| a file larger than 4 MiB for the catalog, more than 2000 entries, a download over 256 MiB | so a hostile or broken source cannot take the host's memory or disk |

Installing needs the `hosting` grant, which already names the libraries the host loads. The host shows a plan first: the source, the
digest, the provider, where the files will be kept (`<home>/extensions/NAME`) and the `[host.extensions.NAME]` it will write, and the
notice that an extension runs with your access. Only after you confirm does it fetch, and it keeps nothing unless what it fetched has the
tree digest or the SHA-256 the entry gives. It then writes `library`, `provider` and `pin` and `root` (or `sha256`) into
`[host.extensions.NAME]`, and the host restarts to load it. The digest is checked again each time the host loads the extension, before
the file is opened, and an extension whose bytes changed is skipped and logged. Settings, Extensions lists what is installed (marking
what the Store installed) and removes an extension with its files; one you name there by hand has no digest and is loaded as it is.

### The overlay store

Store, Overlays, Discover lists the overlays of the catalogs `[catalogs]` names, to search, read about and
install. A catalog is an [AOS catalog](https://github.com/Boriken-Dev/agent-overlay-spec/blob/main/spec/v0/catalog.md):
a file in a git repository, TOML, JSON (or, with any other name, `overlay.cfg`'s rule), listing overlays by name and
source, with a summary, an icon, tags and the like for each. Canoa fetches it as it fetches an overlay (git objects at
the ref named, one commit deep, never checked out, with the same limits), and reads YAML in neither. A `#path` that is
a directory of the repository is a catalog too: each subdirectory is an overlay of that name, as `dotagents` reads a
repository of overlays (`https://github.com/example/overlays.git@main#overlays`).

Canoa offers one catalog of its own, first, with nothing configured: `boriken`, Boriken Dev's overlays
(`https://github.com/Boriken-Dev/agent-overlays.git@main#catalog.toml`). It is fetched like any catalog, by the host,
so a desktop app and a served page list the same. Yours add beside it. To remove it, use Remove in Settings, Catalogs, or
set `boriken = false` in your own canoa.cfg: that is kept across restarts and never edits a lower layer. Giving the label
`boriken` a source of yours replaces it, and Settings, Add, or the Store's own offer brings it back.

```toml
[catalogs]
boriken = false   # leave out the catalog Canoa offers by default
team = "https://github.com/example/overlays.git@main#catalog.toml"   # a label of yours = <repository>@<ref>#<catalog file>

[store]
catalog_max_bytes = 4194304   # a catalog file's size, 4 MiB
catalog_max_depth = 64        # how deeply it nests
catalog_max_entries = 2000    # entries listed from one catalog, by name; the rest are left out and counted
catalog_max_icons = 256       # distinct icon files read for one catalog, each once
icon_max_bytes = 524288       # an icon's size, 512 KiB
icon_max_width = 1024         # the width and height its header may give
icon_max_height = 1024
contents_ttl_seconds = 300    # how long what was read of every entry stays current; the Store's refresh forces it, 0 reads at every refresh
```

A label is yours, a valid overlay name unique among your catalogs; it never comes from the catalog. Each `[store]`
limit is a setting with the specification's default and no floor; one set above its default is listed in the store with
its default.

- **A listing** shows each overlay's icon, name, summary, tags, its catalog's label and its source (with no user
  information or query). It searches by words and filters by tags. A name two catalogs offer is shown once per catalog,
  as `label/name`; entries are never merged. A source relative to the catalog is the catalog's repository at its ref;
  a catalog from a repository may not name a path on this computer, and such an entry is refused. Each refused entry,
  each field left out and each refused icon is listed under its catalog.
- **What a catalog says is its author's claim.** An overlay's page shows it under "What the catalog says", with the
  source and the pin it claims. **Fetch and show what it would do** fetches the overlay, installing nothing, and shows
  what the overlay itself says (its name, description, `[about]` and icon, from its own files), the commit and tree
  digest, and what it would do as Settings → Profiles would apply it: its environment, skills, MCP servers and context,
  what Canoa does not apply (setup, dependencies, commands, the `bin`/`lib` defaults), each setup and
  whether it says it needs the network, the overlays it requires, what none of your layers shares, and its non-default settings.
  A tag or a summary is never taken as evidence of any of it.
- **A catalog's pin** is checked against the tree fetched: one that does not match is shown as a refusal and the
  overlay cannot be installed from that entry. An entry with no pin is shown as unpinned.
- **Install and pin** installs exactly what was shown, through the path **Install…** takes, and names the overlay in
  `[overlays]` as `{ source, pin }`, pinned to the digest checked. **Update** shows what an update changes first. A pin
  canoa.cfg already gives the overlay holds as it does for **Check for changes…**: a tree of another digest installs
  only once you choose **Update and move the pin…** and confirm the move, naming both digests. **Remove…** takes the
  overlay out of `[overlays]` and deletes its files and fetched history. The store records the catalog it installed an overlay
  from; an overlay of the same name that canoa.cfg names from another source, or from the same source by hand, is
  neither installed over, updated nor removed. Nothing of an overlay runs, from the store or
  otherwise. Catalogs are fetched into `overlays/.catalogs/LABEL.git`; what a page fetched and did not install is
  deleted when the page closes.
- **Text from a catalog or a manifest** is shown as plain text, with every control, invisible or bidirectional
  character named as `<U+XXXX>`; a line break is named too wherever one line is shown, and a description is set
  apart, after the source, pin, commit and digest. A source is shown without user information or query, and one that
  holds a password is refused, in `[catalogs]` and from a catalog.

Only git sources with a branch, tag or commit install: an entry whose source is a directory, a URL that is not git, or
git with no ref is listed, and its page says why it does not install. `homepage` and `repository` are shown whole as links that open in the system's browser when tapped, never fetched by
Canoa; one that is not a plain ASCII `http` or `https` address stays text. A web build lists no catalog.

### Saved connections

The **Hosts** section lists this computer first (Local: the host inside the app, with Hosting in its overview), then each connection saved in `[connections.NAME]`. A name is 1 to 64 characters. Each holds where to connect and which key to expect, never a key, a password or a token: an address with a user name or password is refused.

```toml
[connections."Work desk"]
kind = "canoa"                  # "canoa" (a Canoa host, whose agents and abilities it lists) or "acp" (one agent at an endpoint)
address = "https://desk.example:7800/"   # a Canoa host's web address; for "acp", a wss:// or ws:// endpoint (or https:// with transport = "webtransport")
peer = "sha256:..."             # the pinned peer id, 64 hex digits
method = "noise"                # the proof the other end runs: "noise" or "webcrypto"
```

A Canoa host is read from the list it serves at its address when the section opens, which connects to nothing; a direct agent is connected to only when the person asks it to be checked. What either lists of itself is its claim: connecting proves that it holds the pinned id. A direct endpoint offers a session and nothing else, so the section shows no folder browsing, files or management for it, and a workspace is typed as a path on that machine. Removing a connection removes only this app's record of it and names the sessions that were on it; nothing is stopped on the other end. A page served by a host saves its connections in canoa.cfg on that host, through the control channel, when its key holds `config.write`; the host refuses an address with a user name or password and any key but the five above.

### Trusting a project

A project profile is ignored until you trust its directory in Settings → Profiles, which shows the file first, every
invisible or control character in it named as `<U+XXXX>`. The
trust is kept per directory, links resolved, in `trust.json` beside canoa.cfg, with the SHA-256 of the file you were
shown: if the file changes, it is ignored again until you trust it again. **Stop trusting** revokes it for every later
session. A web build reads no working directory and installs no overlay.

## Windows, macOS and Linux tray behavior

Canoa uses a desktop tray controller and the shared Flutter app under root app/. The app can start without a visible main window, remain alive when its window closes, and expose Open Canoa, Hosting, Settings, Hide and Exit commands via the notification area. Closing the window leaves the app, its conversations, its own agents and `canoa` running; Exit, or **Quit Canoa** in Settings, ends them.

What has been run, by platform:

- **Windows 11 (arm64)**: the icon is in the notification area (in its overflow, until the person pins it), named Canoa. Closing the window hides it while the host keeps serving, and the real extension in Edge ran a turn through that host with the window closed. Two clicks on the icon within half a second, Windows' default double-click time, open the window; that decision is tested through the plugin's events, and a double click by mouse was not run.
- **Linux (Fedora 44 under WSLg, arm64)**: tray_manager draws the icon as a StatusNotifier item, which passes no clicks on, so **Open Canoa** in its menu opens the chat. The app looks for a StatusNotifier host on the session bus (`org.kde.StatusNotifierWatcher`). With one, closing the window hides it, and picking Open Canoa from the menu over D-Bus, as a panel does, brings it back. WSLg's desktop has none, so there the app shows no icon, and closing the window minimizes it instead of hiding it where nothing could bring it back; WSLg does not report the window as minimized.
- **macOS**: not run.

The Settings action brings up the main window and opens Settings, a page (Ctrl+, from anywhere in the app) with a list of sections at its left: App (General, Appearance, Notifications, Shortcuts), Agents (Sources, Profiles, MCP servers, Permissions, Usage) and Overlays (Catalogs), then About, and a search over their keywords. A value applies as it is changed and is written at once to the same canoa.cfg file, not an unrelated preferences database; there is no Save or Cancel, and every section keeps the tables it does not edit. A value kept only on this computer is marked. Quitting is the tray's and the menu's, not Settings'. A system-hosted web application cannot overwrite its deployment canoa.cfg.

Where the host grants the key `usage`, the app also has a Usage section of its own (its selector icon, with an index of All accounts and each account under its provider). Its Limits view shows each account's allowances as the host last heard them: how much is left, a bar, the reset, the pace and when the figure was read, with a warning in words once a reading is half an hour old. An account that several hosts report under the same identity is one allowance, the newest reading; Host, Provider and Account filters share the index's selection. The section holds the `[usage] warn_permille` choice, so Settings does not list Usage then. Accounts are still managed in Settings.

The default launch is visible, with close-to-tray enabled. The start-hidden option takes effect on the next launch.

## When a conversation needs you

A conversation needs the person when its agent asks leave to use a tool, asks a question, or finishes a turn (a turn the person
stopped does not count). If that conversation is not the one shown, or the window is hidden, minimized or not focused, the
desktop shows a notification, and choosing it opens that conversation. A notification names the conversation and its agent and
says which of the three it is, nothing else: never the command, the path or anything else the agent sent, which a lock screen or
a shared screen could show. `[notifications]` in canoa.cfg, or Settings → **Notifications**, turns each kind off: `permission`,
`question` and `finished`, each on by default.

In the side list, a conversation waiting for an answer is marked **Needs you**, one whose turn is running shows that it works,
and one whose turn ended since it was last chosen has a dot. The tray says how many conversations wait for an answer: its
tooltip on Windows, the first item of its menu everywhere.

- **Windows**: a toast, through leanflutter's `local_notifier`. The first one creates a Start menu shortcut named Canoa to
  the running program (`%APPDATA%\Microsoft\Windows\Start Menu\Programs\Canoa.lnk`), which Windows requires to show a toast
  for a program not installed from a package; nothing is created until a notification is shown.
- **Linux**: a freedesktop notification through libnotify, which needs a notification service on the session bus.
- **Web, a PWA and the extension**: the browser's Notification API, once the person allows it from Settings → Notifications →
  **Allow notifications**; nothing registers a service worker. A page shows them while it is open.

`app/integration_test/notification_test.dart` has a conversation behind the chosen one finish its turn in the app's own window,
and holds the window open for `CANOA_TEST_HOLD` seconds. What has been run:

- **Windows 11 (arm64)**: Windows' own notification history for Canoa held the toast, titled with the conversation and saying
  its agent finished its turn. Choosing it was not run.
- **Linux (Fedora 44 under WSLg, arm64)**: WSLg has no notification service of its own; with `dunst` running, it received the
  notification from Canoa with the same words. Choosing it was not run.
- **Web and macOS**: not run.

## Building desktop runners

This repository keeps the canonical Flutter code in app/. `python app/tool/build.py desktop` does everything below for this machine's platform and prints the built executable (`--run` runs it, `--debug` builds debug). By hand, Flutter's platform runner directories must be generated before native desktop builds:

    cd app
    flutter create --org dev.boriken --platforms=windows,linux,macos --project-name canoa .
    sh tool/name_runner.sh
    flutter pub get
    flutter build windows  # use linux or macos on that platform

The generated runner names the executable `canoa`, the command-line program's name, which Windows and macOS read as the
same file. `app/tool/name_runner.sh` renames it before the first build, as above: the executable becomes `CanoaUI`
(`CanoaUI.exe` on Windows), the window and the file's description are titled Canoa, and on macOS the bundle is `Canoa.app`
with the executable `CanoaUI` (not yet run on a Mac). People see Canoa everywhere; only the file is `CanoaUI`.

On Linux the tray and window packages (leanflutter's `tray_manager` and `window_manager`) link system libraries that no Dart package carries, so a build needs them installed, offline too:

    sudo apt-get install libayatana-appindicator3-dev libgtk-3-dev libx11-dev libxi-dev libnotify-dev

`libnotify-dev` is for notifications (`local_notifier`). On Fedora the packages are `libayatana-appindicator-gtk3-devel` and
`libnotify-devel`. Its version 0.6 marks a function tray_manager calls as
deprecated, and the generated runner builds with `-Werror`, so build with
`CXXFLAGS=-Wno-error=deprecated-declarations` there (Fedora 44).

A desktop build carries the host as a library, with no `canoa` program: the app's own plugin, `app/native`, builds
`crates/canoa-ffi` with cargo as part of `flutter build`, so `cargo` has to be on PATH or in `~/.cargo/bin`, and the runner
has to be generated in `app/`, whose workspace the plugin finds from there. The result is `canoa_ffi.dll` beside
`CanoaUI.exe`, or `lib/libcanoa_ffi.so` in a Linux bundle; a macOS build is written and has not been run. With
`CANOA_WEB_DIST` set to the absolute path of a web build, the library has the web app built in.

`app/integration_test/desktop_test.dart` runs the app on a real desktop. On a Linux session with no tray, run
`python3 integration_test/sni_watcher.py` first to have one: it is a minimal StatusNotifier watcher that draws nothing,
and the test picks the menu's entries over D-Bus.

The `deps` branch mirrors those packages' Dart side and names these in its `manifest.json`. The desktop CI workflow generates the appropriate runner on each OS with the app's identifier and builds it. Windows, macOS and Linux compilation have been verified in GitHub Actions; the tray on a full Linux desktop panel (KDE, GNOME with its AppIndicator extension) has not been run. Platform-native plugins and tray host support are required. A standard Flutter Web/PWA build does not create a system notification icon.

## Limitations

Canoa is still an early-stage app. The tray controller preserves the process, in-memory sessions and the host while its window is hidden; it does not replace a native background service or automatically install a startup entry. After a restart, sessions are listed from `sessions.json` and open again only through their agents, which have to offer `session/load` or `session/resume`.
