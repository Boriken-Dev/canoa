# Desktop and {{ config_file }}

{{ config_file }} is one file in TOML, YAML or JSON, chosen by its first line and read through [anycfg](https://github.com/jose-pr/anycfg-rs). The app and the `{{ program }}` host read it the same way and find it in the same places.

The default template is [examples/config/canoa.cfg](https://github.com/Boriken-Dev/canoa/blob/main/examples/config/canoa.cfg). [canoa-yaml.cfg](https://github.com/Boriken-Dev/canoa/blob/main/examples/config/canoa-yaml.cfg) and [canoa-json.cfg](https://github.com/Boriken-Dev/canoa/blob/main/examples/config/canoa-json.cfg) hold the same settings in the other two formats.

## The format

The first line names the format: `#!toml`, `#!yaml` or `#!json` (`#!yml` is YAML too, and a path such as `#!/usr/bin/env yaml` counts by its last word). A file with no such line is TOML. A file whose name ends in `.toml`, `.yaml`, `.yml` or `.json` is read as that format and its first line is not looked at; any other extension but `.cfg` is refused. A first line naming another format is refused, saying which formats are read.

Everything this page shows in TOML is the same in the other two: tables are maps, `[host.agents.claude]` is `host: {agents: {claude: {...}}}`. YAML and JSON have a null where TOML has none: a key holding one is the same as not being there, and one inside a list is refused. A TOML date or time is refused in every format; write it as text. The top level is always a table. A file that is malformed is refused naming the key (`host.agents.a.max_peers: expected usize, found a string`) or the line, and never quoting a value from the file.

The app writes the file back in the format and with the first line it was read with, keeping the tables Settings does not edit. A YAML or JSON file is written again as a whole, so its comments and layout are not kept. A TOML file keeps its own text when a host writes it, which is how the app saves while its host runs and how a page served by a host saves: one table is changed in place, and every other table, comment and the way you spelled a name stays. While no host runs, the app writes the whole file from what it holds, and a comment of yours is not kept; the tables it does not read (`[secrets]`, `[settings.ID]`, `[say.NAME]`, an extension's own) are written back as found. A file the app cannot read is shown, the app uses its defaults, and nothing is written over the file until it can be read.

## Where it is found

{{ config_file }} is read from up to three files, lowest layer first, and merged:

| Layer | File |
| --- | --- |
| the installed copy | `{{ config_file }}` beside the program (the folder of the `{{ program }}` or app executable; for the portable `{{ executable }}.exe`, the folder holding that file) |
| the system's | Windows `%PROGRAMDATA%\{{ folder }}\{{ config_file }}` (`C:\ProgramData` without it), elsewhere `/etc/{{ folder }}/{{ config_file }}` |
| your own | Windows `%APPDATA%\{{ folder }}\{{ config_file }}`, elsewhere `$XDG_CONFIG_HOME/{{ folder }}/{{ config_file }}` (an absolute path) or `~/.config/{{ folder }}/{{ config_file }}`; macOS uses `~/.config/{{ folder }}/{{ config_file }}` too |
| PWA / extension | {{ config_file }} alongside the deployed index.html, read-only |

A layer that does not exist is skipped, and each layer may be in any of the three formats. A layer that is malformed is refused, naming its file. The installed copy is where an application ships its defaults, the system's file is for a machine's administrator, and the last is yours.

`--config` (a `{{ program }}` option) or `{{ env_prefix }}_CONFIG` (used when `--config` is not given) replaces the third layer, or all three. Its value is a list of files, split at `;` on Windows and `:` elsewhere (no quoting), the last one highest:

- Without a leading `+`, only the listed files are read: no installed, system or user file.
- With a leading `+` (`+extra.cfg:more.cfg`), the listed files go on top of the installed, system and user files.
- An empty entry is refused, and so is a listed file that does not exist, saying which entry.

`{{ program }} layers` prints the files in the order they are read, lowest first, and the one that is written; the app finds the same ones.

The portable `{{ executable }}.exe` runs the app from an unpacked copy, so it sets `{{ env_prefix }}_INSTALLED_DIR` to its own folder; the installed copy is `{{ config_file }}` there. The variable can name that folder for any program.

### How layers merge

A higher layer is merged over the lower ones, key by key:

- A key in a higher layer replaces the same key below it, and tables merge by key.
- Arrays replace whole, never element by element.
- Named entries replace whole, by name, never key by key: `[host.agents.NAME]`, `[host.providers.NAME]`, `[host.tools.NAME]`, `[host.profiles.NAME]`, `[host.accounts.NAME]`, `[host.pools.NAME]`, `[host.environments.NAME]`, `[agents.NAME]`, `[profile.mcp.NAME]`, `[profile.skills.NAME]`, `[profile.env.NAME]`, `[overlays]` entries and each Store type's `[store.TYPE.catalogs.LABEL]`. A higher `[agents.Claude]` therefore drops a lower one's `[agents.Claude.profile]`.
- A layer cannot remove a key a lower one sets: a null in YAML or JSON is the same as leaving the key out.
- Relative `library` and `source` paths, and the `overlays/` folder, are taken from the directory of your own file.

Settings shows the merged values, says on a row which come from a lower layer ("From the system file") and which of yours override one ("Overrides the system file", with **Reset**, which takes the value out of your file so the lower one shows). The app writes your own file only, and only the keys you changed or already had there; a value from the installed or the system's file is never copied into it. If your file does not exist yet, the first change creates it.

### Everything in {{ config_file }} is managed in the app

You never have to edit the file by hand. Each table has a page: the tables of the app's own settings have theirs (General, Appearance, Voice, Sources, Permissions and the rest), and **Settings, Advanced** lists every other table the host reads (`[host.web]`, `[host.control]`, `[host.native]`, `[host.network]`, `[host.store]`, `[host.delegate]`, `[host.kb]`, `[host.mcp_list]`, `[host.usage]`, `[host.worktrees]`, `[host.history]`, `[host.tools.NAME]`, `[host.environments.NAME]`, `[[host.openers]]`, the settings directly under `[host]` and `[harnesses]`). Each is a form made from the settings the host reads, with add, change and remove, and the host refuses a change it would not read, saying why.

Every list in the app removes an entry the same way: it asks first, and says what goes out of {{ config_file }} and what stays. An entry that comes from the installed or the system's file is shown with where it comes from and how to change it there ("Defined in the system file. {{ name }} does not write that file: remove it there."), because your file holds nothing to take out; when your own entry replaces one of a lower layer, removing it brings the lower one back.

The host keeps the configuration in a *store* (the `config_store` kind): {{ config_file }}'s layers are the store it carries, and a store that keeps it somewhere else can replace it without the host or the app knowing. The app and the host never open the file themselves: the app asks the host it runs (or, before that host is up and where none starts, `{{ program }} control`, which answers the same requests over the same store), and the host reads and writes through the store.

Configuration tables:

- [desktop] start_hidden, close_to_tray, minimize_to_tray, start_at_login (absent until the first run turns it on; true registers the app to start hidden in the tray when you sign in, for your user only; false removes that), local_host (default true: the app runs its own host; false, or `--no-local-host`, leaves it off)
- [network] prefer_webtransport, connect_timeout (seconds, any number above 0)
- [appearance] look (`canoa` or `terminal`), brightness (`dark`, `light` or `system`), density (`normal`, `thinking` or `verbose`, default `normal`), open_in (`this_window`, the default, or `new_windows`: where a plain click on a conversation in the list, on a file in the Files panel and on a new shell in the panel's + menu shows it, here or in a window of its own, on a desktop and in a browser; "Open in new window" stays in each menu either way; Settings, Appearance, Open in), language (a code such as `en` or `es`; empty or absent follows the system's; [the words of the app](themes_and_brands.md)): density is how much of an agent's work a conversation shows. `verbose` draws every step as its own card; `thinking` folds each run of finished tool calls into one line ("Edited calc.js, ran a command +1 −1") and keeps the agent's thoughts; `normal` also hides thoughts and folds a finished turn into "Worked for 1m 29s", with what the agent said after its last step left outside. A step that failed, or waits for an answer, is never folded.
- [host] what `{{ program }}` serves: [host.agents.NAME] and [host.providers.NAME], described in [the host](host.md), and the logins an agent entry may run its sessions on, `[host.accounts.NAME]` and an entry's `accounts`, described in [accounts](host.md#accounts). A credential is never written here: `{{ program }} accounts add` keeps it in the provider `[secrets] accounts` names and writes only the account's kind and id; the account provider that offers the kind made it, and the agents that can run it are the ones that take a standard it provides. The app keeps this table as it is when it saves its settings.
- [sessions] the list of sessions kept across runs: keep (how many, default 200, 0 for none; the least recently used go first), keep_text (whether what was said is kept with each, default false), text_messages (how many of a session's latest messages, default 200), workspace (the folder of a session started with none chosen, a path template; see [a new session](#a-new-session)), while_working (`queue`, the default, or `steer`: what Enter does with a message written while a turn runs; Alt+Enter does the other, and the composer's Steer and Queue buttons are always there), queue_delivery (`merge`, the default, sends everything queued as one message when the turn ends, joined with a blank line; `each` sends one message with each turn)
- [notifications] permission, question, finished: which of these, in a conversation the person is not looking at, reach them as a notification outside the window; each on by default. See [below](#when-a-conversation-needs-you).
- [permissions] default_mode: the mode a new session starts in, one of `manual`, `accept_edits`, `auto`, `plan` and `bypass`, such as `default_mode = "plan"`; unset leaves each agent's own. A session keeps its own choice after that; see [modes](acp-client.md#permission-requests-in-the-chat). An older `full_access` list is read and dropped.
- Rules made by "Always allow here" are not in {{ config_file }}: they are `permissions.json` beside it (the origin's IndexedDB in a browser) with a `permissions-log.jsonl` beside it (desktop only), edited in Settings, Permissions; see [acp-client](acp-client.md#permission-requests-in-the-chat).
- [permissions] allow_once_only: `true` makes Bypass answer only with an answer that allows once, so a request that offers only an "always" is left to the person; default `false`.
- [permissions] protected: the paths a remembered rule ("Always allow here") never covers, in gitignore syntax, as text with a pattern a line or an array of such lines; absent, the recommended list applies and a list written replaces it. See [the ACP client](acp-client.md). `[host.store] protected` and `credential_variables` are the same kind of list for the program stores' installs, read by `{{ program }}` ([the host](host.md)). `[host.store] latest_ttl` (3600; it also keeps what the host found of each installed overlay's source, see [overlay updates](#overlay-updates)), `source_ttl` (3600, a catalog read through a source), `models_ttl` (300), `catalog_ttl` (300), `index_ttl` (300), `connect_timeout` (10) and `lookup_timeout` (15) and `refresh_every` (300; 0 never) are seconds (the last two timeouts are how long a lookup waits to connect and for an answer; a download waits 60): how long the Store answers from what it kept (a publisher's newest version, a type's models, a catalog's entries, the catalog index) and how often, while a client is connected, the host asks again what is past its time (see [what the Store keeps fresh](#what-the-store-keeps-fresh)).
- [harnesses] the harness each name an agent reports in `initialize` stands for, such as `"my-acp" = "My Harness"`. It extends and overrides the built-in names for Claude Code, Codex, Gemini, Antigravity, Pi, OpenCode and Grok; an agent named nowhere is labelled by its entry.
- [agents.NAME] program and args: a source of your own, an ACP agent the app starts on its standard streams; Settings → Sources adds and removes them. See [a new session](#a-new-session).
- [sources] hidden: the presets, by label, that Settings keeps out of a new session's sources, such as `hidden = ["Antigravity"]`. A preset is never changed, only hidden.
- [attachments] paste_as_file (characters, default 8000): a longer paste goes with the message as an attachment of its own; embed_limit (bytes, default 1048576): a larger file goes as a link to it rather than as the agent's embedded context.
- [voice] backend (default `"system"`, the operating system's speech service, which Windows and macOS have), model, language (a tag such as `"es-PR"`; absent is automatic), device (the input device the service names; absent is the system's default) mode (`"toggle"`, the default, or `"hold"` for push to talk: the microphone and Ctrl+Shift+Space listen while held and stop on release) and acknowledged (the services whose first-use notice about where the audio goes you accepted, which {{ name }} writes and does not ask about again): where speech is turned into text for dictation and how it is held. Settings, Voice says where the audio goes before anything is recorded; {{ name }} keeps no recording, and the microphone is on only while a dictation or the page's test runs. A service that fails is not replaced by another one. Three more keys are your own voice profile, which Settings, Voice writes and which goes to the service as hints and nowhere else: accent (in your words, such as `"Puerto Rican Spanish"`), voice (`"female"`, `"male"` or another word) and vocabulary (a list of names and words you say). Nothing is worked out from your audio.
- [speech.NAME] (in the host's {{ config_file }}): a speech provider the host runs, so a page and every platform can dictate. `provider` is `speech_command` (a program that reads the audio on its standard input, as a WAV stream or raw samples, and writes the text) or `speech_endpoint` (an HTTP speech-to-text address and model, such as an OpenAI-compatible `/audio/transcriptions`), or the id of a provider that a `library` (a Guatiao library file) or a `source` (a Guatiao source file, or `bundled:js` with `source_settings = { dirs = [...] }` for a folder of JavaScript providers) the table names offers, and the other keys are that provider's settings, which Settings, Voice draws as a form and writes back. A provider is listed in Settings, Voice beside the operating system's service, and `[voice] backend = "host:NAME"` chooses it; where the system has no service of its own (a browser, Linux) and `backend` was never set, a provider made from one of {{ name }}'s offers (below) is used. The program runs, and the address is called, as you; an endpoint's key is never shown again once saved. An argument of a command provider may be `{language}`, `{language_code}` (`es` for `es-PR`), `{vocabulary}`, `{accent}`, `{voice}`, `{rate}` or `{channels}`, each as `{name|fallback}` to say what stands in for an empty one.

  Settings, Voice also offers speech-to-text providers to start from, which the host sets up. **Whisper** (MIT; a library the host carries) is one Store type that lists OpenAI's Whisper models from Hugging Face's own listing, downloads each once as its folder (checked by the hash Hugging Face states for every file) and runs it on Candle as it is, or on whisper.cpp from the file the install writes in the form you pick; each model's tile shows a pill for each engine, and Set up writes a `[speech.NAME]` table that names the engine (`provider = "speech_whisper_candle"` or `"speech_whisper"`, `offer = "whisper"`). **whisper.cpp** (MIT; a library the host carries, so a build without it has no such offer, and the Store lists it as a type of its own) runs inside the host itself, with no program to install: the Store's Models page installs a model of its published list (Base in every language, Tiny, Small and Large v3 turbo, and English-only forms), each checked against its SHA-256, and the offer is ready once one is installed. Set up writes a `[speech.whisper]` table (`provider = "speech_whisper"`, `offer = "whisper"`, the model and the models folder): the audio is turned into text on the host and goes nowhere else. The model stays loaded for the next recording and is freed after five idle minutes or when another model is used. Another model is another install on the Models page. **Whisper (Candle)** (MIT; a library the host carries, so a build without it has no such offer) is the same Whisper written in Rust, with nothing to compile but Rust: the Store lists it as the type `whispercandle`, whose Models page installs Tiny, Base, Small and Large v3 turbo (three files each, the weights, tokenizer and configuration, from the publisher's repository at one pinned revision, each checked against its SHA-256), and Set up writes `[speech.whispercandle]` (`provider = "speech_whisper_candle"`, `offer = "whispercandle"`, the model and the models folder). It reads the recording in the language you said or the one it detects, with your vocabulary as the initial prompt, and searches greedily (no beam search, no voice-activity detection); it is slower than whisper.cpp on small models, and it reads the weights as 32-bit numbers, so a model takes about twice its size in memory. The person picks between the two. **Whisper in this browser** is the same engine in the web app's own page: it needs no host provider, runs in a web worker over WebAssembly on this device's processor, and the audio never leaves the page. The first dictation downloads the chosen model (Base by default, or Tiny or Small, never Large v3 turbo, which does not fit a page's memory) from Hugging Face at the pinned revision, checks each file against its SHA-256 and keeps it in the browser's cache. The host's page policy lets a page reach only its own origin, so name Hugging Face's addresses under `connect` in `[host.web]` (the error says which); choose it in Settings, under Voice (`[voice] backend = "web-whisper"`). **Foundry Local** (a library the host carries, so a build without it has no such offer) is offered when its service is running on the host and lists a Whisper model: Set up writes `[speech.foundry]` (`service = "foundry"`, `offer = "foundry"`, the model you chose; `service` is the id of the Store type whose library runs the model), and each recording is written to a temporary file on the host, handed to the service by its path and removed afterwards, so nothing leaves the computer. The service's address changes with each start, so it is found when you dictate (`service_address` in the table names one instead); {{ name }} never loads a model, so load it on the Foundry models page, where a Whisper model's page also has Use as speech provider. **OpenAI, account NAME** (the service is described by a library the host carries, `canoa-openai`, so a build without it offers none) is one offer for each OpenAI API key account you already have in {{ name }}: Add writes `[speech.openai-NAME]` with `account = "NAME"`, `offer = "openai:NAME"` and no key, and the key is read from the account's store when you dictate, so you never type it again and the provider's title says which key pays; the audio goes over the internet to OpenAI, and the page says so before anything is recorded. A ChatGPT sign-in is not an API key and is listed as not offered, with the reason. Add an API key as an account to use OpenAI's speech-to-text: a key is no longer typed into a form of its own. A page's key needs `voice` to use a provider, `voice.setup` to add one, and `store.programs` to install an engine; the page says which is missing.
- [say.NAME] (in the host's {{ config_file }}): a voice provider the host runs, so a page and every platform can read replies aloud. `provider` is `speech_say_system` (this computer's own voices: Windows' desktop voices, `say` on a Mac, `espeak-ng` elsewhere; the text goes nowhere), `speech_say_endpoint` (an HTTP text-to-speech address in the shape of OpenAI's `audio/speech`, with a `model`, a `voice`, a key and the `models`, `voices` and `styles` you list; the text goes to that address) or `speech_say_command` (a program that reads text on its standard input, or in an argument, and writes audio, with the `voices`, `models` and `styles` you list), or the id of a provider a `library` or a `source` the table names offers (as for `[speech.NAME]`), or one a library this build carries offers: `speech_say_piper` (Piper, a local neural engine, with a `program` and a `voices_dir` of `NAME.onnx` files; the text goes nowhere) and `speech_say_openai` (OpenAI's voices, with a key; the text goes to OpenAI). Two providers need no table: `system`, and `piper` once the Store installed it; accounts whose kind provides `openai-api` at OpenAI's own address (`OpenAI, account NAME`, via `canoa-openai`'s `account_speech_output`) also offer their voices directly without typing a key into `[say]`. Each provider lists its voices, models and speeds in Settings, Voice, and says where the text goes before anything is spoken.
- [voice] output (the voice provider that reads replies aloud, by the id Settings gives it: `"browser"` for the browser's own voices, `"host:NAME"` for a `[say.NAME]` provider of the host; absent is this page's own voices when it has them, else the host's operating system), output_voice, output_model, output_style, output_speed (a multiple of normal speech, above 0 and at most 10) and read_replies (`true` reads each reply aloud as it streams in; the switch beside the microphone sets it): how replies are read aloud. Each agent reply has a Play button beside Copy that reads its prose, a sentence at a time, without code blocks or a tool's output; typing or dictating stops voice mode's reading. A conversation popped out into a window of its own, and a child's conversation, have the same Play button, and voice mode reads a child's replies as they arrive while it is shown; a pop-out window plays nothing itself but asks the main window to, so its voice is the same. A provider that fails stops the reading and is never replaced by another. Audio a host provider sends back is played by the page's Web Audio, and on the desktop by Windows' waveOut, `afplay` on macOS and `paplay`, `pw-cat` or `aplay` on Linux, whichever is installed.
- [usage] warn_permille (thousandths, default 800): from this much of a usage bucket used, the usage control beside the mode shows a status; an exhausted bucket always shows as an error. See [usage](acp-client.md#usage).
- [usage] limit_phrases (a list of text; unset means `RESOURCE_EXHAUSTED`, `Resource has been exhausted`, `exhausted your capacity`, `Error 429`, `rate limit`, `usage limit`, `quota` and `throttling_error`): the words that make a failed turn of an agent that reports no usage a limit, read by `{{ program }}` for the stdio agents it serves (ASCII case does not matter). A list replaces the defaults rather than adding to them, and `[]` infers nothing. See [usage](acp-client.md#usage).
- `title` on a `[host.agents.NAME]` entry (text, 1 to 64 characters, not blank, no control characters; optional): what the app shows for the entry in its source picker instead of the entry's name. A provider or tool entry takes none. It is listed beside `harness` (the provider id) in the host's lines and in `/canoa.json`, which is how the app learns both.
- `usage` on a `[host.agents.NAME]` entry with `program`: the id of a usage reader, one a bundled agent offers (`"antigravity"`) or a library or source of this file offers (an extension written with the SDK's `UsageReader`; `{{ program }} serve` stops naming the entry when nothing offers it) makes `{{ program }}` read that agent's quota as `reported` usage buckets, the way [usage](acp-client.md#usage) describes. The app's Add an agent sets it when the command is `agy acp`.
- [overlays], [profile] and `[agents.NAME.profile]`: profiles, described [below](#profiles).
- [store]: the Store's limits and [the catalogs of each Store type](#catalogs-of-each-store-type), `[store.TYPE.catalogs.LABEL]`; the overlay store is described [below](#the-overlay-store), `[store.extensions.catalogs]` lists [extensions catalogs](#extensions-catalogs), and `[store] skills_search` turns on [searching skills.sh](#searching-skillssh).

The session list is `sessions.json` beside {{ config_file }} on a desktop, and this origin's IndexedDB in a web build, a PWA or the extension. Removing a session from it never deletes the agent's own session. It also keeps, per source, what its agent offered (model, effort, mode and its other choices) and took with a prompt when it last ran, and the last model, effort and mode the agent took from you.

### A new session

**New session**, Ctrl+N (⌘N on a Mac) or the palette opens a new session in the middle of the window: a choice of **Chat** or **Workspace** (and, for a workspace, its folder: recent folders, or **Open folder…**) above the message box, and in the row under the box `+`, the **source**, **Local** or **Worktree**, the agent's **mode**, **model** and **effort**, the prompt **cache**, the **overlays** it starts with, and Send. It is in the side list at once, as New, with nothing started: no agent, no connection and no worktree. One left untouched is shown again rather than another made, and a new one starts from the source chosen last, as the kind of session started last (a Chat before any); the recent folders are in the folder menu, and choosing one is for that session only. A chat has no worktree, so **Local** and **Worktree** appear for a workspace only.

A **Chat** is a session with no project: it works in a folder of its own that {{ name }} makes and never shows in the new-session view, the list or the header, on the first message and never before, and takes away again if the agent does not start. The details panel says Chat and keeps the folder under Technical details. A **Workspace** session works in the folder the person chooses, and cannot start without one. The folder of a chat comes from `[sessions] workspace` (Settings: Where chats keep their files), a path template whose `${name}` variables are put in when the folder is made (Settings shows it with what it makes now); the default is `${documents}/{{ folder }}/scratch/${session}`. The session list records which a session is; a chat opens again in its own folder, and the list grouped by workspace and Earlier conversations put chats under **Chats**.

A conversation's name is what the agent reports, else the first line of the first message. A name the person gives it (Rename) is kept, and no later name from the agent replaces it, including after the session is opened again.

| Variable | Is |
| --- | --- |
| `${documents}` | the person's Documents folder: on Windows the Documents known folder (which follows a redirected or OneDrive Documents), on Linux `$XDG_DOCUMENTS_DIR`, else `user-dirs.dirs`, else `~/Documents`, on macOS `~/Documents` |
| `${home}` | the home folder |
| `${session}` | {{ name }}'s own id for the session, a UUID made with the draft before any agent runs and kept in the session list beside the id the agent reports |
| `${date}`, `${time}`, `${datetime}` | local time when the session's draft was made, so the folder shown is the folder made: `2026-11-11`, `153005`, `2026-11-11T153005` |
| `${handler}` | the harness's `acp_harness` provider id, the same for a hosted and a preset source: `claude`, `codex`, `grok`, `antigravity`, `opencode`, `pi` or a hosted entry's own `harness`; else the source's label |
| `${source}` | the source's label |
| `${host}` | this computer's name |
| `${fqdn}` | a webpage's host name; empty on the desktop |

Only the template places separators (`/` or `\`, either spelling): every value except `${documents}` and `${home}` is made one path segment, its separators, `..`, control and reserved characters replaced by `_`, and a part of the template that a variable leaves empty (`${fqdn}` on the desktop) is dropped. {{ config_file }} is refused, naming `sessions.workspace`, when the template names a variable not in the table, leaves a `${` open, or has no `${session}`, so two sessions never share a folder by accident.

A session started from a webpage works in `<Documents>/{{ folder }}/scratch/<site>/<session>` on the machine its agent runs on: one folder for the site, one for each session. The page names the site, not a path: its `session/new` carries `{"site": "<host name>"}` under `_meta["dev.boriken.canoa/site"]` and no folder (the app sends `/`, which the host replaces), and `{{ program }}`, which serves the agent, resolves the folder itself, makes it (private to the user on Unix) and starts the agent there. The folder does not follow `[sessions] workspace`. The host's Documents folder is `XDG_DOCUMENTS_DIR` when the host's environment sets it, on any system, else the one `${documents}` names above. The site is the page's host name in lower case with an international name as punycode, no port, path or credentials, and the host takes it only as written: a name that is not a plain name fails the session with `-32602` and nothing is made. Refused: `..`, a separator, a colon (so a drive letter), a first label Windows reserves for a device (`con`, `nul`, `com1` and the rest, with or without an extension), an IPv6 address, a label that is empty, over 63 characters or has anything but letters, digits and inner hyphens, a name over 253 characters, an upper-case letter or a non-ASCII one; the app shows no panel for a page with such a name. The host's answer to `session/new` carries the same key with `site`, `id` and `folder`, and the session list records them. The extension's panel for the page in front lists that site's earlier sessions, newest first, with **Resume** and **New session**. **Resume** opens the session again, through the browser's provider (which has to hold the agent's grant), with `session/load` or `session/resume` naming the site and the recorded folder; the host takes that folder only when it is one session's own under that site's scratch folder, and makes it again if it is gone. An entry with `working_roots` refuses a site session whose folder is outside them. The host makes no other folder for a page.

A source is a program with its arguments and what else it needs: the host's own sources (Claude Code and Codex through the host's translators, Antigravity as `agy acp`, Grok as `grok agent stdio`, OpenCode as `opencode acp` through the built-in `opencode` harness, Pi; served by `{{ program }} serve --local-sources` to the app that runs it, with no address, and replaced by an entry of the same name in `[host.agents]`), the commands under `[agents.NAME]` (served by the host as its own sources: adding or removing one restarts the host, which ends the sessions open on it), and the agent entries the `{{ program }}` this app runs is serving, as `{{ program }} serve` lists them: an entry added to `[host.agents]`, a `harness` entry naming any provider included, is a source once the host serves it, with no change to the app (the app does not read the entry, and a draft kept on a hosted source gets it back when the host serves it). Settings → Sources lists them with where each program was found (on PATH, else where its installer puts it: `%LOCALAPPDATA%\agy\bin` for `agy` on Windows, `~/.grok/bin` for `grok`), hides a preset, and adds or removes a command. A source whose programs are not on PATH stays listed and says which it found and which it did not; sending to it says so instead of starting.

The first message starts it: the worktree is made, the profile is prepared (its dialog shown only when it has more than a model, effort or mode to approve), the agent started or the host reached, `session/new` sent with the folder, the profile and its MCP servers, the model, effort and mode chosen set, and then the message sent with what goes with it. A start that fails says why, takes back the worktree it made, and leaves the message to send again. Until then the session is kept in `drafts/` beside {{ config_file }}, with what is written in it and what goes with it, and comes back after a restart; closing it leaves nothing there or in the session list. The session list notes it once it has started.

Before its source has run, model and effort offer only **Default**, which sets nothing; once it has, they list what the agent offered then, with when. A new session's default is the profile's value, else your last choice for that source. After the start they list what the agent offers, a change is sent with `session/set_config_option` and shown once the agent takes it, and a value it refuses goes back, with its reason in the conversation. A change during a turn is sent before the next message. **Cache** is how long the agent's prompt cache lives, for this session: Short (about 5 minutes, for one-shot and background runs), Long (about an hour, for a session you are active in) or the harness's own Default. Unchosen, a chat session asks Long, or what its entry's `cache_lifetime` in {{ config_file }} says; your choice wins and is kept with the draft and the session. It shows only for a source whose provider said what it does with the choice, and that sentence is printed under the message box (Codex's reads "ignored", since its app server has no such setting). Full access shows in the mode control, in red, beside the conversation's banner. The overlays can be changed until the first message; after it the control shows what applied.

### A worktree per session

**Worktree** under a new session's message (only for a folder in a git repository), or **In a new git worktree, on its own branch** in a new session on a saved {{ name }} host, starts the session in a new worktree of the repository holding the folder, from that directory's current commit. For a new session it is made when the first message is sent, and named after its source. The branch is `canoa/` and the session's title (or its agent's name) in lower-case letters and digits joined by hyphens, then six random hexadecimal digits, such as `canoa/fix-the-tests-3fa91c`; nothing else is taken, so no branch reads as an option, a range or a path step. The worktree goes beside the repository, in `<repository>.worktrees/<branch without canoa/>`, and a `.worktrees` directory already there that is a link is refused. The session list shows the branch, and the session list kept across runs notes the worktree with the session.

- **An agent this app runs**: the app runs `git` itself, with an argument list, its hooks and fsmonitor off, and refuses a repository whose own settings name a filter, an include or an attributes file, or whose git directory is apart from its work tree, as a submodule's is. Filters in your global settings, such as Git LFS's, run as usual.
- **An agent `{{ program }}` serves**: the host makes the worktree, only inside its entry's working roots, as [the host](host.md#worktrees) describes; with no working root, or `worktrees = false`, it makes none. The browser's provider makes none.

Closing the session ends its agent first, then asks whether to keep the worktree. Everything there in no commit is counted: changed, new, and files git ignores. Removing them is a button of its own that names them, and a different set found before anything is deleted is told again; without that button nothing in no commit is removed. A HEAD left detached at commits no branch holds keeps the worktree. Its branch goes with it only when it holds no commit another branch, tag or remote does not have, and only if no commit came meanwhile; otherwise it stays, so no commit is lost. A session list entry is taken only when its worktree is where {{ name }} puts that branch beside that repository. Reopening a session from the list finds its worktree again, even with its HEAD moved, or says it is gone and does not start the agent.

Do not store identity private keys, tokens or passwords in {{ config_file }}.

## Profiles

A profile is what an agent is launched with: environment variables, an initial context, skills, MCP servers, and the
model, effort and mode it starts with. It comes in layers, merged lowest first:

1. **built-in**: {{ name }}'s own: the [standard access](#access), and nothing else;
2. **overlays**, each named in `[overlays]`: [AOS](https://github.com/Boriken-Dev/agent-overlay-spec) overlays, in their composition order (`priority`, then name);
3. **yours**: `[profile]` in {{ config_file }};
4. **the project's**: `{{ home_dir }}/profile.toml` in the working directory, only once you trust that directory;
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

What a profile may set is not a list in {{ name }}: each thing is an **item**, a provider of the `profile_item` kind that says its name, the
harnesses it is for and the form it asks for, and that turns the values you gave it into what a session gets (environment
variables, arguments for a program the host starts, values of the session's own settings, the network its programs use). The model,
effort, mode, starting instructions and access are items {{ name }} carries, kept in a profile under their own keys; a proxy and an
identity are items that ship with the host as scripts (JavaScript, offered as an extension's would be), and an extension of {{ config_file }}'s `[host.extensions]` (or a library or source
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
client_certificate_hosts = [                       # system and pac: hosts the proxy presents a client certificate to, first match wins
  { pattern = "api.corp.example" },                # a host, *.name, an address or a regular expression matched against the whole host
  { pattern = "(db|cache)[.]corp[.]example", identity = "system" },   # system (the default) or a thumbprint the host lists
  { pattern = ".*", identity = "3a7bd3e2360a3d29eea436fcfb7e44c735d117c42d1c1835420b6b9942dd4f1b" },   # every other host
]
```

A session applies each item its profile holds when it starts, and a started session's profile report lists each (`items.ID`) and what it became, by
the names of the variables and settings and never their values. A variable the profile's own `[profile.env]` already sets keeps that
value, and the report says so. An item for a harness other than the session's, an item whose values do not fit its form, and an item
no provider offers any more (its extension was removed) are not applied and say why; the profile keeps the values, and putting the
extension back applies them again.

**Identity** (`items.identity`): `name` and `email` become `GIT_AUTHOR_NAME`, `GIT_COMMITTER_NAME`, `GIT_AUTHOR_EMAIL` and
`GIT_COMMITTER_EMAIL`; `account` is `AGENTS_ACCOUNT`; `variables` are further `NAME=value` lines.

**Proxy** (`items.proxy`): how the session's programs reach the network, by `mode`:

- `system` (the default): the host runs a proxy on its loopback address (the one other sessions with these settings share) and gives the programs
  `HTTP_PROXY`, `HTTPS_PROXY` and `ALL_PROXY` for it (and `NO_PROXY` for this computer and `bypass`). It sends each connection
  the way the host's own network does: the system's proxy chosen per destination (PAC, WPAD), logged in to as the host logs in
  (Basic, Negotiate, NTLM) and carried through a CONNECT tunnel, with the hosts on `bypass` (in the `NO_PROXY` form) going
  direct, so a program that only knows a plain proxy behaves as the system does. The programs' address carries a secret of the
  proxy as its password, since the proxy answers with your own logon session: another process on this computer that
  lacks it is refused. The proxy ends with the last session that uses it.
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

**A client certificate to an origin** (mTLS) is carried only where you ask, for the hosts `client_certificate_hosts` names, and only by
the `system` and `pac` modes, whose proxy is the host's own. It is a list of entries, each a `pattern` and an `identity`; the first
entry whose pattern matches a destination decides, and a destination no entry matches is not touched. A `pattern` is a host name
(`api.corp.example`), `*.name` for a name and everything under it, an address, or any other regular expression, which must match
the whole host and ignores case (`(api|auth)[.]corp[.]example`; `.*` matches every host, so one entry can cover them all). It is
matched against the destination as the program names it: a name, or an address written out. The `identity` is `system` (the
default: what your keystores, or `[host.network] client_certificate`, choose by the authorities the server names, as a browser
would) or the thumbprint (SHA-256, in hex) of one certificate, picked from the list the host gives of the client certificates it
holds: the Windows certificate store, the macOS Keychain, a PKCS#11 module where one is set up, and the `[host.network]` file
(`network/client_certificates`: subject, issuer, expiry and thumbprint, never a key; the form shows it as a pick for each entry,
as a browser asks which certificate to send). A certificate that is not held here, has expired, or may not authenticate a client
is the program's `502`, and nothing is sent to the destination.

A CONNECT tunnel carries the TLS the program makes itself, so a proxy cannot add a certificate to it; for the destinations an entry
matches, and no others, the proxy ends the program's TLS instead. It makes its own TLS to the host first, through the network: the
server is verified as the operating system verifies it and the entry's certificate is presented, and a host that cannot be
verified, or that wants a certificate you have none for, is the program's `502` with the reason. It then answers the program's
TLS with a certificate for that host, made the first time a program connects to it and kept, by host name or address, until a
day before it expires, and carries the bytes between. Every other host stays a tunnel, untouched.

The session's programs are told to trust the authority that signs those certificates, and nothing else is: it is never added to
the operating system's store, your browser's or any other of your programs'. Its certificate, with the operating system's roots
beside it where a program replaces its roots by a file, is written in the session's private directory, which goes with the
session, and named by `SSL_CERT_FILE`, `REQUESTS_CA_BUNDLE`, `CURL_CA_BUNDLE`, `GIT_SSL_CAINFO`, `PIP_CERT`, `CARGO_HTTP_CAINFO`
and `NPM_CONFIG_CAFILE` (and, alone, by `NODE_EXTRA_CA_CERTS`, which adds to the roots Node carries). The authority is the host
process's: made in memory the first time a proxy needs it and shared by every session, its key is written to no file and goes with
the process. When every pattern is a plain host, `*.name` or address, the authority is limited to exactly those (a critical X.509
name constraint, which every TLS library checks: `*.name` becomes the name and what is under it, an address stays that address),
so a program that trusts it, or anyone who gets hold of the session's bundle files, cannot be shown a valid certificate for any
other host; a host named as `name` is permitted with its subdomains too, since a DNS constraint cannot be narrower, and a set of
hosts that differs is another authority. A regular expression cannot be said as a constraint, so a session that has one is
given the process's one authority that has no limit: it can vouch for any host to a program that trusts it, but only the host
process holds its key, which is why a pattern should be as narrow as the hosts need. A program that reads none of these, uses its
own trust (Windows' own tools, a browser) or pins the certificates it accepts fails for those hosts: nothing can be done about
that here. A host both matched here and on `bypass` is reached as the network chooses, not directly. What the session's report
says of the proxy names the patterns it ends TLS for.

**Sessions share a proxy.** The `system` and `pac` modes run one proxy for every session whose proxy item makes the same settings
(mode, address, bypass list, login, patterns and their certificates, and the `[host.network]` it follows), with a secret of its
own: it starts when the first such session does and ends when the last does. `[host.network] proxy_idle_seconds` keeps it that
long after its last session for the next one that asks (default 0: it ends with its last session).

A later layer wins per key. Environment variables, skills and MCP servers merge by name: a later layer's entry replaces a
lower one's whole and keeps its place, and `remove = true` takes it out. `unset = ["model"]` takes back a lower layer's
`model`, `effort`, `mode` or `context`. **Settings → Profiles** shows the effective profile of each agent, every value
beside the layer it came from, and what each layer removed. {{ program }} reads the same schema
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
mine = { dir = "profiles/mine" }                                   # a directory, relative to {{ config_file }}'s
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
directory: {{ config_file }}'s for yours and an agent's, the overlay's, or the working directory for a project.

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

An agent entry that names no profile (no `profiles`) starts with the default profile: {{ config_file }}'s top level (`[profile]`,
`[overlays]`, `[profiles.default]`), so its overlays, skills, hooks, access and variables, `<NAME>_OVERLAY_ROOT` included, reach
its sessions as they do for a named profile, whether the entry is a source of the host's own or written by hand under
`[host.agents.NAME]`. An entry that names a profile gets that one instead, and one that should run bare names an empty
profile (`profiles = ["bare"]` with an empty `[host.profiles.bare]`). `{{ program }} check` prints the profile each entry starts with
and why, and *Effective setup* lists it as `profile`. Without anything in {{ config_file }}'s top level there is no profile and no
overlay, and the agent is still given the identity variables above that do not depend on one (`AGENTS_HOST`, `AGENTS_HARNESS`,
`AGENT`, `AGENTS_VENDOR`, the account's).

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
started as its entry says: {{ name }} passes it no access setting.

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
such a file is refused (`reserved`). `{{ program }} agent ID` takes the same as `--sandbox none|workspace|read-only`,
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

Claude Code and Codex are run by `{{ program }} agent claude` and `{{ program }} agent codex`, the host's own translators of their stdio
protocols (coqui-acp's `coqui-acp-claude` and `coqui-acp-codex`), with no Node; the desktop app runs them from its own
executable, and a `[host.agents]` entry with `harness` runs them inside the host. Pi is run the same way by coqui-acp's
`coqui-acp-pi` (`harness = "pi"`), which needs only `pi`.

| Part | Claude Code (`{{ program }} agent claude`) | Codex (`{{ program }} agent codex`) | Gemini CLI (a command you add) | Pi (`coqui-acp-pi`) | Another agent |
| --- | --- | --- | --- | --- | --- |
| Environment | the process of `claude` | the process of `codex app-server` | the agent's process | the agent's process | the agent's process |
| Initial context | ACP `systemPrompt`, appended: `claude --append-system-prompt-file` | ACP `systemPrompt`, appended: the thread's developer instructions | not supported | ACP `systemPrompt`, appended: `pi --append-system-prompt` | ACP `systemPrompt` when it advertises it |
| Skills | a Claude Code plugin, `canoa-profile`, through `_meta.claudeCode.options.plugins`, which becomes `claude --plugin-dir`; a skill is `canoa-profile:<name>` | the Agent Skills directory of a new directory given as ACP `additionalDirectories`, which becomes one of Codex's skill roots | not supported | not supported | not supported |
| MCP servers | ACP `mcpServers`: stdio, HTTP, SSE (version 1), written to the file `claude --mcp-config` names | ACP `mcpServers`: stdio, HTTP, as the thread's `mcp_servers`; skipped by Codex when its own configuration names a server the same | ACP `mcpServers` | not supported: Pi reads its own from its configuration and takes none from a client | ACP `mcpServers`, as it advertises |
| Model, effort | the session's config options `model` and `effort`: the models Claude Code lists and their efforts | the same: the models Codex lists and their reasoning efforts | the session's config option offering that value | the config options `model` and `effort` (Thinking): the models Pi has credentials for | the same |
| Mode | not supported: the translator offers no mode | not supported: the translator offers no mode | the session's config option offering that value | the config option `mode`: Pi asks nobody, so the translator asks, and refuses protected paths | the same |

Plugins from a client are off in `{{ program }} agent claude` unless `--setting allow_client_plugins=true` is given, since a plugin's hooks and
MCP servers run with nothing asked. The app passes it, as it writes the only plugin, `canoa-profile`. A host entry allows
them only when it offers profiles, and then only its own: the host removes every `_meta` a client sends.

Which harness an agent is comes from what it says in `initialize`, else from the preset it was started from. A part a
harness does not take is listed as not supported, with why: Gemini CLI, Pi, OpenCode and Grok read context and skills only
from their own directories when they start. What the dialog says is what {{ name }} expects; the conversation says what the
agent itself advertised.

Antigravity (`agy acp`) takes skills like Codex: it reads `.agents/skills` inside each directory a session gives it as
`additionalDirectories`, so the skills go there. An Antigravity that does not advertise `additionalDirectories` is listed as
not supported, with that reason.

Skills and context are read once, when the session is prepared: an overlay's from the bytes its tree digest covered, never
from its files again; a project's only inside its directory once links are resolved, and a skill directory holding a link
is refused. Skills are written into a new directory under the system's temporary directory, private to you, removed when
the agent ends. The initial context, every layer's together, is at most 1 MiB; a skill directory at most 1024 files and
16 MiB.

A project's own agent context is handed to an agent that does not read it itself: `AGENTS.md` and `AGENTS.*.md` in the folder
the session opens in and in its `.agents` folder, `AGENTS.local.md` last, each with its path, in the system prompt where the
agent takes one and with the first prompt where it does not. A file the agent loads itself (Codex reads `AGENTS.md`) is left
out, and the report lists each file. `AGENTS_DIR` (default `.agents`) and `AGENTS_CONTEXT` (default `AGENTS.md` and
`AGENTS.*.md`) in the session's environment, each a list in the platform's path-list separator, change the folder and the
file names. With Claude Code, a directory the session enters brings its own files with the next prompt, once.

An MCP server's environment is resolved as the profile's is: an overlay's text is substituted against that overlay's own
environment, with `AOS_OVERLAY_ROOT` and `AOS_OVERLAY_NAME`; its `env_passthrough` and `headers_env` give a variable only
when one of your layers shares it, and otherwise leave it out and say so. A command with no directory is looked up on
`PATH`, since ACP takes an absolute one; one not found refuses that server. An overlay's server runs in the session's
working directory, not the overlay's, since ACP gives an MCP server none: the dialog says so.

A server from your own layers is on unless you turn it off; one from an overlay or a project is off until you allow it.
**Keep these choices** in the dialog writes them to {{ config_file }}, under the agent:

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
body, when it holds more than white space, is the overlay's context in place of `CONTEXT.md`, which {{ name }} then ignores
and lists among what it does not apply. `overlay.cfg` is TOML, or JSON or YAML after a `#!json` or `#!yaml` line. YAML is
read as a restricted subset: a tag, anchor, alias or merge key leaves the overlay out. {{ name }} reads the
manifest statically and validates it; one it cannot read or that is not valid leaves the overlay out, except that a
problem in `[about]`, store metadata, never does. {{ name }} applies:

- the environment: the overlay's env files (the file `env` read as dotenv, and `env_files`), `env`, and `paths`, each
  value substituted (`${NAME}` and its operators) when a session starts, in the overlays' order, after every overlay's
  `<NAME>_OVERLAY_ROOT`. A placeholder reads what overlays set and the variables your own layers share; any other variable
  of this computer it names is an error, so no session starts. A secret reference is read only for `provider = "env"`
  and a variable you share; {{ name }} holds no named or file secrets, so those stay unset when `optional`, and otherwise
  stop the session from starting. A substituted value is at most 1 MiB, expressions nest at most 32 deep, and a pattern
  may take at most ten million steps;
- skills, MCP servers (`env_passthrough` and `headers_env` only for variables you share; no header over plain `http` to
  another host), rules, routing lines and context as initial context, and the `model` and `effort` hints of
  `[harness.<id>]` for the agent's harness (`claude-code`, `codex`, `gemini-cli`, `pi` for the presets);
- `${NAME}` in the overlay's context files (and the body of `OVERLAY.md`) and in the body of each bundled skill, replaced
  when {{ name }} hands the text to the agent, never in the installed files: `AOS_OVERLAY_ROOT` (a private copy of the
  overlay's verified files, the one its hooks run from), `AOS_OVERLAY_NAME`, `AOS_PROJECT_ROOT` (the folder the session is
  opened in; a skill's body is written once for the connection, so it keeps this name as written) and the overlay's
  options. {{ name }} gives an overlay no state directory and its scopes no label, so `AOS_OVERLAY_STATE` and
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
ends it with the end of what it printed and leaves the record and the earlier environment as they were. {{ name }} gives every
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

{{ name }} does not apply yet, and **Settings → Profiles** lists as "not applied by {{ name }} yet": `mcp.skills`, an MCP server's `timeout` and its working directory (it runs in the session's), other
harnesses' hints. Each line names its overlay. An overlay's own `bin/` goes first on `PATH`, and its `lib/python/` and
`lib/node/` on `PYTHONPATH` and `NODE_PATH`, ahead of the host's own entries, for the agent and the programs it starts.
A `paths` entry that leaves the overlay's folder is refused, and a `paths` entry may set only those three of the variables
{{ name }} keeps to itself.

From git, the desktop app installs an overlay into `overlays/NAME/` beside {{ config_file }}, with `overlays/NAME.json`
recording the source, the commit, each file's git blob id and **the tree digest** of what it wrote, and its history in
`overlays/.cache/NAME.git`. **Install…** and **Check for changes…** in Settings → Profiles fetch the source and show
the commit, the tree digest, every file it adds, changes or removes, and its `OVERLAY.md` or `overlay.cfg`; nothing changes until you
apply, and what is applied is that commit's files, checked against the digest as they are written. Applying **pins**
the overlay in {{ config_file }} by its digest, `NAME = { source = "...", pin = "sha256:..." }`: a later check whose files
have another digest installs nothing until you change the pin. Each time profiles are built the installed copy is
digested again, and one whose files changed is
left out until synced. An overlay with no pin, or a `{ dir = "..." }` with none, is reported as `unpinned` each time.
A pin is a SHA-256 over every file's bytes as published, the same on every platform; the commit is kept beside it.
The ref is a branch, a tag, or a whole commit id, which names that commit even if a branch or tag carries the same
name; an abbreviated id is not taken. Only that ref is fetched, one commit deep. An overlay is upstream's copy,
replaced whole on each apply: put your own additions in `[profile]`, which wins over it. An overlay installed from
another source than the one {{ config_file }} names is left out until it is synced.

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
  **ambiguous**: both are reported with the overlay naming them, {{ name }} never picks one, and the overlay applies
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
  {{ name }}`, or `a named skill; no catalog resolved it here: install or sync the overlay in {{ name }}`), never silently missing.
- **The store's Skills section** lists each skill here with the overlays that name it (or that you chose it) and its
  source and pin, and the skills your catalogs offer, each installable by your own choice after the same review.
- **Removal.** Removing an overlay offers to remove the skills that only it named; a skill another overlay names stays,
  and one you chose is never offered. Declining leaves it listed as named by no overlay, to remove there.

`{{ program }}` itself fetches nothing: it delivers a skill the app fetched, after the same checks, and reports the others.

### The Store

**Store** in the left column (and **Store…** in Settings → Profiles) is where an agent's programs, overlays, skills and
MCP servers are found, installed and chosen. It has four types, **Agents** (where the host grants programs),
**Overlays**, **Skills** and **MCPs**, behind one toolbar: the type tabs with a count each (a badge on Agents when a
program has a newer version), **Installed** and **Available**, a search, **Refresh**, **Store settings** and a line that
says how old the catalogs' index is ("Cached · updated 3 min ago", "Checking…" while the host asks again, "Stale" with the reason only when a source could not be refreshed; see [What the Store keeps fresh](#what-the-store-keeps-fresh)). Each
type keeps its own query, switch and place in the list while you read an entry's details. Every card says where it comes
from (the catalog's label, the repository, the commit and digest it is pinned to), what it needs, and whether it is
installed, and opens its details: what it holds (skills, MCP servers), what it requires, the hooks and environment scripts
it carries (which stay off until you allow them at their digest), and technical details with the digests. Everything is
shown before it is installed. **Store settings** holds the catalogs: each one's source and what it said, and the form that
adds one.

- **Overlays.** Installed: each overlay's icon, summary, source, pin, what it applies (environment variables, skills, MCP
  servers, context) and what {{ name }} does not apply yet, a switch for whether **new sessions** start with it, an update
  check and Remove (for one the store installed). The host asks each overlay's source whether it holds something newer
  (see [updates](#overlay-updates)); one that does shows an **Update available** mark, its check becomes an **Update**
  button with the update icon, the same icon replaces the check on its tile in Available, and the Store's index (or the
  tab, on a page of its own) counts how many have an update. **Check for updates** asks every source now. Available: the overlays of the catalogs below, filterable by catalog,
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
  `registries`, `mcp`) is not read: the registries are `[store.mcps.catalogs]` and the notes on installed servers
  are `[store.mcp.NAME]` now.
- **A default loadout for each kind of client.** `[defaults.clients]` names, per kind (`browser`, `outlook`, `word`, `excel`,
  `powerpoint`), the loadout a session starts with when it names none and its client says which kind it is, in
  `initialize`'s `_meta["dev.boriken.canoa/client"]`: the browser extension's side panel and the Outlook, Word, Excel and
  PowerPoint task panes do. It sits between a session's own loadout (and the loadout of the profile it names) and
  `[defaults] loadout`, so a kind with no entry starts with the default. A kind that is not in that list refuses the
  file; a loadout `[loadouts]` no longer holds (deleted by hand) does not: the session gets the default, and its profile
  line says the kind's loadout is gone. The host resolves it when the session starts, so it also applies where the
  client cannot read {{ config_file }}, and the profile report names the loadout the session got (`loadout`). Deleting a loadout in
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
  extension's panel reads {{ config_file }} through its worker for the names; a panel that cannot shows no control and the host
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
  record names (and the pin in `[skill_pins]`, if any); otherwise it is left out and the session's report says why. Installed also lists every skill an installed overlay provides, marked with the overlay and with no Remove of its own: it goes when the overlay does. Available: the sources you read skills from (see [Skill sources](#skill-sources)) and a spec to paste; a skill an overlay's catalog names comes with its overlay and is not offered alone (`<repository>@<ref>#<skill directory>`). A
  skill is an [Agent Skills](https://agentskills.io/specification) directory: a `SKILL.md` with YAML front matter whose
  `name` is the directory's. It is fetched only on a yes, after its source and pin are shown, and checked against the
  pin before any file is read.
- **MCPs.** Installed: the servers `[profile.mcp]` and your overlays define; an overlay's is marked with the overlay and has no Remove of its own. Available: the
  [MCP Registry](https://github.com/modelcontextprotocol/registry) (`registry.modelcontextprotocol.io`, `GET
  /v0.1/servers`), each server a `server.json`. Installing shows the registry's claims, the digest of the `server.json`,
  each way to run it (an npm, PyPI or OCI package, or a remote address) with the command or address {{ name }} would write,
  and what it needs from your environment: a secret is a `from_env` reference, never written to {{ config_file }}. A package
  whose runtime (`npx`, `uvx`, `docker`) is not on this computer, a bundle, a package of another registry type, a
  remote over plain `http` and anything else {{ name }} does not run is refused with the reason. An installed server is
  written to `[profile.mcp.NAME]`, noted in `[store.mcp.NAME]` (`registry`, `server`, `version`, `digest`) and turned
  off for every agent in `[agents.NAME.mcp]`, so nothing starts until you allow it.
  A team's own servers can be listed by another registry that serves the same API, added by its `https` address (or
  `http` on the loopback address), or by a git repository holding a JSON file in the API's list shape, added as
  `<repository>@<ref>#<file>`; they are kept in `[store.mcps.catalogs]` and chosen beside the official one in Available.
  A git list is fetched at its ref like a catalog and read as one page.
- **Applying a change.** The host reads {{ config_file }}'s layers again when each session of its own sources starts, so a
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

[store.mcps.catalogs]          # a label of yours for each registry
team = "https://registry.team.example"
lists = "https://git.team.example/lists.git@main#servers.json"

[store.mcp.files]             # where an MCP server the store installed came from
registry = "https://registry.modelcontextprotocol.io"
server = "io.github.ana/files"
version = "1.2.0"
digest = "sha256:..."         # of the server.json that was shown
```

### The network

With no `[host.network]` table every fetch the host makes (the Store's downloads and release lists, `registry/fetch`, the servers of `[sdk.NAME]` and the endpoints of the speech providers, the Foundry Local service) and every program it starts follows the computer's own settings, as a browser on it would:

- the proxy: the `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY` and `NO_PROXY` variables of the host's environment, else the operating system's (Windows Internet Options through WinHTTP, `scutil --proxy` on a Mac, GNOME, KDE and NetworkManager settings elsewhere), PAC scripts and WPAD included, with its bypass list; loopback is never sent through a proxy;
- the proxy's login: Negotiate or NTLM with the logon session on Windows, when the proxy asks for it (`negotiate = false` turns it off); Basic with the login below, when one is given;
- who is trusted: the operating system's certificate verifier, so a TLS-intercepting proxy the computer trusts works;
- the client certificate: when a server asks for one, the one a browser would offer from the operating system's keystores (the Windows certificate store, the macOS Keychain, a PKCS #11 module through p11-kit) that the server's named authorities issued, with the policy Edge and Chrome read;
- git (overlay and skill sources): started with the proxy for the remote it is fetching and, on Windows, with `http.sslBackend=schannel`, so it uses the same certificate store; a git configuration of the person's own (`http.proxy`) still wins where the source is one they named.

The desktop app's own requests (the list a saved connection's host serves at `canoa.json`, its status probe, the terminal check) are sent by the host it runs through `network/fetch`, so they follow the same settings; the app's own library would use none of them. A served page's requests are its browser's.

The desktop app's WebSocket to a remote {{ name }} host follows all of them too: the app asks its host (`network/tunnel`) to open the connection, and for a `wss` address the host reaches the server through the proxy the system names (logged in as above) or straight, makes the TLS itself (the certificate verifier and client certificates above, with `ca_file` and `client_certificate` over them) and gives the app a loopback connection to the plain WebSocket, which only the app can use. A server whose certificate only the operating system trusts, such as a company or TLS-inspecting proxy's authority, is therefore trusted, and a server that asks for a client certificate is given the one a browser would offer; one the host does not trust is not connected to, and the error says why. A `ws` address is carried through a proxy the system names and opened by the app itself where it names none. This computer's own address is always opened by the app itself. WebTransport, which no HTTP proxy carries, is not used by the desktop app, which dials the WebSocket endpoint.

A program the host starts (an agent, Node or Python, `git`) is given `HTTP_PROXY`, `HTTPS_PROXY` and `NO_PROXY` for that proxy when its environment names none; variables the person or the entry set are never replaced. A proxy chosen per URL (PAC, WPAD) cannot be written as variables, so it is asked about the addresses the programs reach (the package registries, GitHub, the agents' own services; `endpoints` adds to them) and the proxy it names for those is the one given. Each key below is an override and none is needed:

```toml
[host.network]
proxy = "system"                 # system (the default), none, pac:LOCATION (a URL, a file: URL or a path), or a proxy for every URL:
                                 # [scheme://][user[:password]@]host[:port]
proxy_user = "alice"             # a login for every HTTP proxy that names none of its own
proxy_password_env = "PROXY_PW"  # the variable holding its password; the password is never in {{ config_file }}
no_proxy = ["corp.example", "*.lan", "10.0.0.0/8"]   # replaces the system's bypass list; "<-loopback>" sends loopback through the proxy too
negotiate = true                 # answer a proxy's Negotiate or NTLM with the logon session (Windows only)
client_certificate = "system"    # system (the default), none, or a PEM file holding a certificate chain and its unencrypted key
ca_file = "roots.pem"            # trust exactly these certificates, replacing the system's
git_tls = "system"               # system (Schannel on Windows) or git (git's own and the person's git configuration)
endpoints = ["https://tools.corp.example/"]   # addresses the programs reach, for a PAC script to choose their proxy by
proxy_idle_seconds = 0             # how long the proxy a profile's system or pac mode runs is kept after its last session
```

Paths are read from the folder of {{ config_file }}. The host starts only with a setting it can use: a proxy it cannot read, a `proxy_password_env` variable that is not set, or a certificate file that is missing is refused at start, with the key named. Git's client certificate is not chosen from the keystore as a browser's is: Git for Windows presents one only when its `http.sslCert` names it, which a source the person typed gets from their own git configuration.

### Catalogs of each Store type

Every Store type reads catalogs of its own kind, in its own format, and lists only those. They are written the same way for every
type, `[store.TYPE.catalogs.LABEL]`, where `TYPE` is `overlays`, `skills`, `mcps`, `extensions`, or the id of a type an extension
adds, and `LABEL` is a name of yours (a letter, then letters, digits, `-` or `_`, at most 64). A catalog added under one type is
never listed under another: Settings, from a type's page, shows and changes that type's catalogs only.

A catalog is read by a **catalog source**, a provider of the host (a `catalog_source` Guatiao kind, so an extension can add one).
A label's value is the source `read` reads, or a table naming the provider and what its form asks for, or `false`, which takes a default
catalog away:

```toml
[store.overlays.catalogs]
boriken = false                                                       # the default catalog, taken away
team = "https://github.com/example/overlays.git@main#catalog.toml"    # the same as { provider = "read", source = "..." }

[store.overlays.catalogs.generated]
provider = "command"                                                  # run on each refresh; what it prints is the catalog
command = ["python", "make_catalog.py", "--team"]
cwd = "C:/work/catalogs"                                              # optional: the folder it runs in
timeout = 60                                                          # optional: seconds it may run
```

#### Defaults, replacing, adding and turning off

A type's catalogs are its **defaults** (what the extension that adds the type registers, such as a pinned list of models it ships a copy of), then
your entries by label: an entry with a default's label **replaces** it whole, a new label **adds** a source after the defaults, and `false`
turns a default **off** (it stays listed in Settings, Catalog sources, off, so it can be turned on again).

```toml
[store.whisper.catalogs.openai]               # the default's label: this replaces it
provider = "whisper_hf"
author = "myorg"                              # a Hugging Face publisher whose Whisper repositories are listed
pin = "0f3c9d1a5b7e2c4d6f8091a2b3c4d5e6f7a8b9c0"   # a revision the source reads at, or "sha256:<hex>" of the catalog's text

[store.whisper.catalogs]
mine = { provider = "read", source = "D:/models/whisper.json" }   # a canoa.models/1 file of your own
other = false                                 # a default turned off
```

Four keys of an entry are the host's, whatever the provider: `pin` (a pinned catalog is read once and never goes stale by time; a
`sha256:` pin must match the text the source answers with), `ttl` (seconds before it is read again; `[host.store] source_ttl`,
3600, otherwise), `keys` (the publisher key fingerprints the entry demands a signature from) and `endpoints` (see below). A provider's
own form may not use these names. Type catalogs that list models answer in `canoa.models/1` and a program's releases in `canoa.releases/1`
(`crates/canoa/include/canoa.models.schema.json`, `canoa.releases.schema.json`); every item says which catalog it came from. A file's
`check` names the algorithm of the digest its publisher states (`sha256`, `sha512`, `md5`, `git_blob`), and the download is checked against
that one; an item whose files all have one carries a check mark in the Store, which says "Verified with SHA-256" (or the algorithm used),
and its page has a Verified field with the algorithm, or No. A file with no digest is offered all the same, without the mark. What was last read is kept on disk beside `latest.json`
(`cache/store/catalogs/<scope>/<label>.json`) and a catalog that cannot be reached shows its last answer with the reason; a pinned default
ships a copy and needs no network the first time. Settings, Catalog sources lists every scope's catalogs with their origin, pin, badge and last
read, and replaces, adds, turns off and removes them through `store/catalog_set`.

#### Where the files of a catalog come from

A Store type, and each program, downloads only from the places its extension names (the `endpoints` it declares), whatever a catalog
says: an item whose file is anywhere else is refused, with the address and the places it may have come from. A catalog that lives
elsewhere (a mirror, a fork's repository) names its own places with `endpoints`, a list of `https` origins or prefixes: whoever wrote it
has said where the files are, and each file is still checked by its digest.

```toml
[store.whisper.catalogs.openai]
provider = "whisper_hf"
author = "myorg"
pin = "0f3c9d1a5b7e2c4d6f8091a2b3c4d5e6f7a8b9c0"
endpoints = ["https://huggingface.co/myorg/whisper.cpp-models/resolve/"]   # where its files are
```

#### The releases of a program

A program the Store installs lists its releases from catalogs of its own, written the same way: `[store.PROGRAM.catalogs.LABEL]` where
`PROGRAM` is the program's id. Its extension registers one default, which follows the publisher, and nothing about a release (an
address, a version, a digest) is held in code:

| Program | Default label and source | Reads | Checked by |
| --- | --- | --- | --- |
| `claude` | `anthropic`, `claude_releases` | the channels (`latest`, `stable`) and each version's manifest | the manifest's SHA-256, after its signature by Anthropic's release key verifies |
| `codex` | `openai`, `codex_releases` | the newest release's channel document and checksums | SHA-256, which the two must agree on |
| `grok` | `xai`, `grok_releases` | the channels (`stable`, `alpha`, `enterprise`) | nothing: the publisher gives no checksum |
| `agy` | `google`, `agy_releases` | the updater's manifest for each platform | SHA-512 |
| `opencode`, `git-bash`, `powershell`, `bw` | `github`, `opencode_releases`, `git_bash_releases`, `powershell_releases`, `bitwarden_releases` | the stable releases of a GitHub repository (`repo = "owner/name"`) | the SHA-256 GitHub lists for each file |
| `node` | `nodejs`, `node_releases` | the index and each channel's checksums (`lts`, `current`) | SHA-256, read only from the text a signature by the Node.js release team verifies |
| `python` | `astral`, `python_releases` | the standalone builds' checksums (a channel for each minor series) | SHA-256 |
| `pi` | `npm`, `pi_releases` | the newest version in the npm registry, with Node's long-term-support build | SHA-256 for Node's archive; npm checks the package by the registry's integrity |

A version you name must be one a catalog lists. A signature that does not verify refuses the whole catalog; there is no fallback to the
digest alone. An entry's `keys` list the fingerprints a signature must come from, in place of the publisher's. A program with no
checksum is offered as not verified, and its plan says so:

```toml
[store.codex.catalogs.internal]                  # a mirror that serves the publisher's layout
provider = "codex_releases"
origin = "https://mirror.example.com/openai-releases"
endpoints = ["https://mirror.example.com/openai-releases/"]
```

#### Turning off a provider

`[host.disabled]` turns off a provider a library registers, by the kind's name and the provider's id, and leaves the library's other
providers alone:

```toml
[host.disabled]
account_provider = ["claude_accounts"]   # Claude's accounts are not offered; the claude harness still runs on any other provider's account
store_extension = ["whisper_extension"]  # the Whisper type leaves the Store
catalog_source = ["command"]             # entries that name this source show that it is off
mail_backend = ["ews"]                   # the mail service leaves this backend out; its other backends stay
```

A provider that is off is not listed or started (account makers, harnesses, program stores, Store types, catalog sources, session services,
settings sections, panel tabs, profile items, secret providers, the speech kinds the scripts offer, and the mail backends the mail service loads). The accounts of an account provider
that is off stay in {{ config_file }}, are shown as off in Settings, Accounts and `accounts/list` (`disabled`), and are back when it is turned on.
The host reads `[host]` when it starts; a kind or an id no library offers is a warning that lists the ones it knows, not a refusal.
Settings, Providers lists every registered provider by kind with a switch that writes this table (`providers/switch`).

Each source describes itself to the Store: its name, the types it serves (or all of them) and the form it asks for (a schema, and how to show it), which is what
Store settings shows when you add a catalog to a type. Two come with {{ name }}, as scripts the host carries. **`read`** reads what the type reads itself: a git source
(overlays and extensions), `owner/repo`, a git URL or a folder (skills), a registry's address (MCP servers). **`command`** runs a
program you name, as you and with your environment, each time the catalog is refreshed, and takes its standard output (or the file named
by `file`, in the folder it ran in) as the catalog in the type's own format, read exactly as a fetched one is. It serves every type;
a source an extension adds says in its description which types it serves (none listed means all), and Store settings offers it only
under those. A program that fails, runs past its time, prints nothing or prints more than 4 MiB leaves an overlays or extensions catalog as
the last good run made it, and shows why, with the end of what it wrote on its error stream; for Skills and MCP servers, which are read
when you open the page, it shows the same reason and lists nothing. An extension adds sources by offering `catalog_source` providers in
its library.

What a source that is not `read` prints is the type's own format:

| Type | Format of the text |
| --- | --- |
| `overlays` | an overlay catalog file (the AOS format), TOML or JSON |
| `extensions` | an extensions catalog file (below) |
| `skills` | a skills listing: JSON, `{"skills": [{"name", "description"?, "source", "path"?, "ref"?}]}`. `source` is where the skill is, written as a [skill source](#skill-sources) is (`owner/repo`, a git URL, a folder); `path` is the skill's folder from the repository's root when the source does not name it; `ref` pins a branch, tag or commit. Installing one reads the source it names, shows what was found and asks first, as for any skill source, and fetches it with none of your git settings, credentials or keys. An entry with no name or source, or a second of a name, is left out and counted; one whose name or source is not valid is listed with why it cannot be installed. At most 1024 are listed |
| `mcps` | a page of the MCP Registry's list shape: JSON, `{"servers": [{"server": {...}, "_meta": {...}}], "metadata": {"nextCursor"}}`, each `server` a `server.json`. An item that names nothing or that the registry marks as not its latest version is left out, and a search narrows the rest by name, title and description. The servers are the source's claim, as any registry's are |

`read` is also what the type reads itself, and the host reads it for every type the same way: the Skills type scans the repository or
folder for `SKILL.md` as `npx skills` does, and the MCP servers type reads a registry's list endpoint (`<address>/v0.1/servers`) or a
git file in that list shape. The MCP servers type always offers the official registry, `registry.modelcontextprotocol.io`, first under
the label `official`: it cannot be removed or replaced, and no label of yours may take its name.

Naming a source other than `read`, in a file a page writes through the host, takes the `hosting` grant: choosing one starts a program as
you or code an extension supplied, so it is the same trust as naming a library.

### Secrets

When an agent needs a password or a key it asks for it by name, and {{ name }} answers from what it already holds before it asks you. Where a secret comes from is a provider: **the form** (you type it in the app), **the system store** (Windows' Credential Manager, the Secret Service on Linux, the Keychain on a Mac: besides what {{ name }} keeps there, the Secrets section lists the other entries your account can read, such as the credentials of other programs, marks them as not {{ name }}'s and shows one only when you press Show; Forget on one of those asks first and deletes it from the store) and any a library adds, such as a Bitwarden one written as a script. The form that asks you also asks how long to keep what you type: this once, for this session, until {{ name }} closes, for 15 minutes, an hour, 8 hours or a day, or **Remember** in the system store. An agent never sees the value, only a handle, and using it still asks you as any command does.

```toml
[secrets]
default = "form"          # who answers a name nothing else names
store = "system"          # where Remember keeps it; "none" takes the choice away
keep = "session"          # what the form offers first
max_wait = 240            # the most seconds an agent's question waits for you (never more than the question lasts)
accounts = "auto"         # who keeps the credentials of your accounts and your mail passwords: the system store when it answers, else sealed files, or a setup of yours that keeps what it is given

[secrets.names]
github_token = "work"     # this name is answered by the entry below

[secrets.providers.work]
provider = "bitwarden"
keep = "close"            # what this entry gives stays until {{ name }} closes, or less if it says so itself
command = "bw"
[secrets.providers.work.from]
session = { entry = "form", name = "bw_session" }   # the key that opens the vault is a secret you gave {{ name }}, never written here
```

**Bitwarden** is a provider that ships in {{ name }}, written as a script that {{ name }} runs itself, so there is nothing to build. It reads an item of your vault by name through Bitwarden's own `bw` command, which the Store installs if you have none (Store, Secret providers), so `bw` has to be signed in (`bw login`). Point a name or the default at it and take the key `bw unlock --raw` prints from another provider, such as the form, as the secret `bw_session`:

```toml
[secrets.providers.work]
provider = "bitwarden"
[secrets.providers.work.from]
session = { entry = "form", name = "bw_session" }   # the key is a secret you gave {{ name }}, so it is not written here
```

Store, Secret providers lists Bitwarden's command line (`bw`), which it installs for you with a plan you confirm (the zip of your system, checked against the SHA-256 GitHub lists for it); {{ name }} runs it itself and no agent finds it by name.

A script of your own is added as an extension that names the JavaScript {{ name }} carries: `source = "bundled:js"`, with `source_settings` of `dirs` (the folders of its `*.guatiao` manifests) and `kinds` (the description of the kind it serves, `canoa.secret.kinds.json`).

The vault is never unlocked for you: until the key is given, a request for a name Bitwarden answers falls back to asking you, and a wrong or expired key does the same. The key goes to `bw` in its environment, never on a command line.

**Set up a vault** in the app: Secrets, Set up a vault, at the foot of its index (or, after the Store installs `bw`, it opens by itself) draws the provider's settings from the form the host describes. A setting that is a secret, such as Bitwarden's session key, is never a box to type in: you choose where it comes from, another setup or the form (you type it when {{ name }} asks, and the form that asks opens next), and what you choose is saved in {{ config_file }} as a reference, never as the secret.

The **Secrets** section of the app has an index like the Store's: All vaults, then one row per vault (the form, the system store and each setup you made) with how many secrets it holds. All vaults lists what {{ name }} holds, each secret's vault and name, its value hidden until you press Show, and how long it is kept. Forget takes one out (of the system store too, when it was kept there), and Set a secret asks for a value in a form only the app and the host can read.

An agent can say how long it can wait for your answer; {{ name }} never waits longer than `max_wait`, and when you are not connected at all it tells the agent so at once instead of waiting. A question nobody answered in time disappears from your screen.

The credentials of your agents' accounts and your mail passwords are kept the same way, as secrets named `accounts/…` in the provider `accounts` names, so they are in the Secrets section too (each home's system store is its own: two homes never see each other's) and can be kept in any setup that keeps what it is given. A host that kept them before (in the operating system's store under its own name, or in sealed files) moves each account into the new place the first time it is used, and takes the old copy away only once the new one reads back the same. `[host] account_store` is gone: `account_store = "file"` is now `accounts = "file"` in `[secrets]`.

A value that comes with an end of its own (a token that expires) is kept no longer than that end, whatever you chose. Nothing is written to a log.

### Skill sources

Skills are looked for the way `npx skills add` looks, in their own list, `[store.skills.catalogs]`, a label of yours
for each source written as that command takes it. In Store, Skills, Available the sources are chips; choosing one is what
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

{{ name }} offers `anthropics` (`anthropics/skills`) first. It is fetched with none of your git settings, credentials or keys; a source you
add is yours, so it is fetched as you would from a shell, which reads a private repository with your credentials. To remove the
default, use Remove this source or set `anthropics = false`; giving the label a source of yours replaces it.

```toml
[store.skills.catalogs]
anthropics = false                                  # leave out the source {{ name }} offers by default
web = "addyosmani/web-quality-skills"               # a label of yours = a source as above
```

### Searching skills.sh

[skills.sh](https://skills.sh) ranks skills by how often `npx skills add` installed them, and has a search. {{ name }} can search it for you, but
it is off until you turn it on, because skills.sh is a third party and its search address is not documented. Turn it on in Store settings
(Skills, Search skills.sh) or in {{ config_file }}:

```toml
[store]
skills_search = true                       # skills.sh; remove the line (or write false) to turn it off
# skills_search = "https://skills.example"  # another directory that answers /api/search?q=...&limit=... the same way
```

With it on, Store, Skills, Available has a Search skills.sh chip. Only the words you type are sent (at least two characters), through the
same proxy and certificate settings as every fetch the host makes, with no redirect followed. A result names a repository
(`owner/repo`) and a skill in it, with how many installs skills.sh counted; nothing it sends is a file. Installing one reads that
repository the way any source is read (fetched with none of your git settings, credentials or keys, its `SKILL.md` files listing what it
holds), shows the commit that was read, asks, and fetches exactly that commit. A result that does not name a GitHub `owner/repo`
is dropped, and one whose repository has no `SKILL.md` for it says so. Turning the search off removes the chip and the results.

### Extensions catalogs

An extension is code the host loads to add to {{ name }}: a library that adds store types, or a [package of scripts](script_providers.md)
that serves any kind of provider. Store, Extensions lists the extensions of
the **extensions catalogs** you name in `[store.extensions.catalogs]`, a label of yours for each, written as a catalog is in
`[store.overlays.catalogs]` (`<repository>@<ref>#<file>`) but a list of its own: none is offered until you add one, and a catalog of overlays is
never read as one of extensions. {{ name }}'s own format, described here, is not the overlay catalog's.

```toml
[store.extensions.catalogs]
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

[extensions.team-notes]                  # a package of scripts: a folder with a manifest, the same on every platform
source = "./packages/team_notes"
pin = "sha256:8280aaf5de7c4f61342f648586d534e481ae095cef223dded70f5b1db5bd84a1"
runtime = "js"
provider = "team_notes_notes"
```

| Key | Meaning |
| --- | --- |
| `source` | Required. A git repository wherever git reads one (a folder of this computer by its path, `file:`, `ssh`, `git@host:path`, `https` with the credentials the address holds), written `<repository>` with an optional `@<ref>` and `#<folder>` (a `git+` before it names git when the address does not end in `.git`); a folder of the catalog's own repository (`./kits/x`, `../x`), at the catalog's ref and from the catalog file's folder; or an `http` or `https` URL of one file, with any query or credentials it needs |
| `ref` | The branch, tag or commit of a git `source` written without `@<ref>` (a branch may hold `/`). The repository's head when neither is given |
| `pin` | Required with a git or relative `source`, refused with a URL: the tree digest of the folder (`sha256:` and 64 hexadecimal digits), the same digest an overlay's `pin` is |
| `sha256` | Required with a URL, refused with a git `source`: the SHA-256 of the file, 64 hexadecimal digits |
| `load` | The library file in the tree the host loads (a `/`-separated path of plain names). Required for a tree by this host, refused with a URL, whose file is the library |
| `runtime` | Optional, for a git or relative `source` with no `load`: `js` or `python`, a package of scripts whose folder is the library (it has a `*.guatiao` manifest at its root). It needs no build and no `platforms`; the host runs `js` with the QuickJS it carries, and a host with no Python source refuses `python` at the plan |
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
| a tree with no `load` and no `runtime` | this host loads one library file; with no name it cannot choose |
| a `runtime` of any language but `js` and `python`, or one beside a `load` or a URL | a package of scripts is a folder the host reads whole: it names no file, and a lone file is no package |
| a file larger than 4 MiB for the catalog, more than 2000 entries, a download over 256 MiB | so a hostile or broken source cannot take the host's memory or disk |

Installing needs the `hosting` grant, which already names the libraries the host loads. The host shows a plan first: the source, the
digest, the provider, where the files will be kept (`<home>/extensions/NAME`) and the `[host.extensions.NAME]` it will write, and the
notice that an extension runs with your access. Only after you confirm does it fetch, and it keeps nothing unless what it fetched has the
tree digest or the SHA-256 the entry gives. It then writes `library`, `provider` and `pin` and `root` (or `sha256`) into
`[host.extensions.NAME]` (for a package of scripts, `source = "bundled:js"` and `source_settings = { dirs = [...] }` in place of
`library`), and the host restarts to load it. The digest is checked again each time the host loads the extension, before
the file is opened, and an extension whose bytes changed is skipped and logged. Settings, Extensions lists what is installed (marking
what the Store installed) and removes an extension with its files; one you name there by hand has no digest and is loaded as it is.

### What the Store keeps fresh

The host keeps what the Store lists up to date, so opening it asks nobody. Each thing has a time in `[host.store]`, in seconds
(0 asks every time), and one thread asks again whatever is past its time, every `refresh_every` seconds while a client is
connected. It asks again only what was asked for before, and only what is past its time, so `refresh_every` decides how late
a renewal can be and never how often a publisher or a repository is asked: that is the time of each thing.

| Key | Default | What it keeps |
| --- | --- | --- |
| `index_ttl` | 300 | The index of each overlay, skill and extension catalog (a catalog in git: `ls-remote` for the ref, a fetch only when it moved) |
| `latest_ttl` | 3600 | A publisher's newest version of each program, and whether an installed overlay's source holds a newer commit |
| `models_ttl` | 300 | A Store type's list of models (an install, update or removal empties it) |
| `catalog_ttl` | 300 | The entries of a skills or MCP servers catalog, as the app lists them |
| `source_ttl` | 3600 | A catalog read through a source provider (an entry's own `ttl` replaces it) |
| `refresh_every` | 300 | How often the renewal runs while a client is connected; 0 never |
| `connect_timeout`, `lookup_timeout` | 10, 15 | How long one lookup waits to connect and for the answer |

The line under the Store's toolbar and each catalog say where the answer stands, from what the host reports:

- **Cached · updated 3 min ago**: the last refresh worked, then.
- **Checking… · updated 5 h ago**: the answer is past its time and the host is asking again. The app reads the host again
  every few seconds until it is done, and every minute otherwise, so the line follows the host.
- **Stale · it cannot be fetched: offline · updated 2 h ago**: the last refresh failed, with the reason; the last answer that
  worked is still listed. Only a failed refresh makes a catalog stale, and "updated" is always when one last worked, never when
  it was last tried. The next renewal tries again.

The default `refresh_every` is five minutes, the shortest time of the things it renews: a longer one only makes a
renewal late, and a shorter one finds nothing past its time.

### Overlay updates

The host tells the app which installed overlays have something newer (`overlay/updates`). It asks each overlay's source, as
the record of its install names it: a source that follows a branch has an update when the branch's head is not the
installed commit **and** the overlay's folder holds other files there (a commit elsewhere in a shared repository is no
update); a source that names a tag has one when the tag names another commit; a source named by a whole commit id has
none, and says it is pinned. A source that cannot be asked is reported as unreachable with the reason, never as an update.
An overlay chosen from a catalog is asked as it is fetched, with none of your git settings, credentials or keys; one you
named is asked as you would from a shell, through the same proxy and certificate settings as every other fetch.

What the host found is kept for `[host.store] latest_ttl` seconds (3600), the same time the Store keeps a program's newest
version; a source that could not be asked is tried again after a minute at most. The host asks in the background when it
starts and, while a client is connected, every `[host.store] refresh_every` seconds (300; 0 never) for what has gone
past its time, so opening the Store waits for nobody. **Update** shows the same plan an install shows (what changes, from and to)
and pins the new digest in {{ config_file }} on the yes.

### The overlay store

Store, Overlays, Available lists the overlays of the catalogs `[store.overlays.catalogs]` names, to search, read about and
install. A catalog is an [AOS catalog](https://github.com/Boriken-Dev/agent-overlay-spec/blob/main/spec/v0/catalog.md):
a file in a git repository, TOML, JSON (or, with any other name, `overlay.cfg`'s rule), listing overlays by name and
source, with a summary, an icon, tags and the like for each. {{ name }} fetches it as it fetches an overlay (git objects at
the ref named, one commit deep, never checked out, with the same limits), and reads YAML in neither. A `#path` that is
a directory of the repository is a catalog too: each subdirectory is an overlay of that name, as `dotagents` reads a
repository of overlays (`https://github.com/example/overlays.git@main#overlays`).

{{ name }} offers one catalog of its own, first, with nothing configured: `boriken`, Boriken Dev's overlays
(`https://github.com/Boriken-Dev/agent-overlays.git@main#catalog.toml`). It is fetched like any catalog, by the host,
so a desktop app and a served page list the same. Yours add beside it. To remove it, use Remove on the Overlays settings page in the Store, or
set `boriken = false` in your own {{ config_file }}: that is kept across restarts and never edits a lower layer. Giving the label
`boriken` a source of yours replaces it, and the Overlays settings page's Add, or the Store's own offer brings it back.

```toml
[store.overlays.catalogs]
boriken = false   # leave out the catalog {{ name }} offers by default
team = "https://github.com/example/overlays.git@main#catalog.toml"   # a label of yours = <repository>@<ref>#<catalog file>

[store]
catalog_max_bytes = 4194304   # a catalog file's size, 4 MiB
catalog_max_depth = 64        # how deeply it nests
catalog_max_entries = 2000    # entries listed from one catalog, by name; the rest are left out and counted
catalog_max_icons = 256       # distinct icon files read for one catalog, each once
icon_max_bytes = 524288       # an icon's size, 512 KiB
icon_max_width = 1024         # the width and height its header may give
icon_max_height = 1024
contents_ttl_seconds = 300    # how long what was read of an entry stays current in a catalog the host gave no commit for (a catalog at the commit it was read at needs no time); 0 reads every time an entry opens
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
  what {{ name }} does not apply (setup, dependencies, commands, the `bin`/`lib` defaults), each setup and
  whether it says it needs the network, the overlays it requires, what none of your layers shares, and its non-default settings.
  A tag or a summary is never taken as evidence of any of it.
- **A catalog's pin** is checked against the tree fetched: one that does not match is shown as a refusal and the
  overlay cannot be installed from that entry. An entry with no pin is shown as unpinned.
- **Install and pin** installs exactly what was shown, through the path **Install…** takes, and names the overlay in
  `[overlays]` as `{ source, pin }`, pinned to the digest checked. **Update** shows what an update changes first. A pin
  {{ config_file }} already gives the overlay holds as it does for **Check for changes…**: a tree of another digest installs
  only once you choose **Update and move the pin…** and confirm the move, naming both digests. **Remove…** takes the
  overlay out of `[overlays]` and deletes its files and fetched history. The store records the catalog it installed an overlay
  from; an overlay of the same name that {{ config_file }} names from another source, or from the same source by hand, is
  neither installed over, updated nor removed. Nothing of an overlay runs, from the store or
  otherwise. Catalogs are fetched into `overlays/.catalogs/LABEL.git`; what a page fetched and did not install is
  deleted when the page closes.
- **Text from a catalog or a manifest** is shown as plain text, with every control, invisible or bidirectional
  character named as `<U+XXXX>`; a line break is named too wherever one line is shown, and a description is set
  apart, after the source, pin, commit and digest. A source is shown without user information or query, and one that
  holds a password is refused, in `[store.overlays.catalogs]` and from a catalog.

Only git sources with a branch, tag or commit install: an entry whose source is a directory, a URL that is not git, or
git with no ref is listed, and its page says why it does not install. `homepage` and `repository` are shown whole as links that open in the system's browser when tapped, never fetched by
{{ name }}; one that is not a plain ASCII `http` or `https` address stays text. A web build lists no catalog.

### Saved connections

The **Hosts** section lists this computer first (Local: the host inside the app, with Hosting in its overview), then each connection saved in `[connections.NAME]`. A name is 1 to 64 characters. Each holds where to connect and which key to expect, never a key, a password or a token: an address with a user name or password is refused.

```toml
[connections."Work desk"]
kind = "canoa"                  # "canoa" (a {{ name }} host, whose agents and abilities it lists) or "acp" (one agent at an endpoint)
address = "https://desk.example:7800/"   # a {{ name }} host's web address; for "acp", a wss:// or ws:// endpoint (or https:// with transport = "webtransport")
peer = "sha256:..."             # the pinned peer id, 64 hex digits
method = "noise"                # the proof the other end runs: "noise" or "webcrypto"
```

A {{ name }} host is read from the list it serves at its address when the section opens, which connects to nothing; a direct agent is connected to only when the person asks it to be checked. What either lists of itself is its claim: connecting proves that it holds the pinned id. A direct endpoint offers a session and nothing else, so the section shows no folder browsing, files or management for it, and a workspace is typed as a path on that machine. Removing a connection removes only this app's record of it and names the sessions that were on it; nothing is stopped on the other end. A page served by a host saves its connections in {{ config_file }} on that host, through the control channel, when its key holds `config.write`; the host refuses an address with a user name or password and any key but the five above.

### Trusting a project

A project profile is ignored until you trust its directory in Settings → Profiles, which shows the file first, every
invisible or control character in it named as `<U+XXXX>`. The
trust is kept per directory, links resolved, in `trust.json` beside {{ config_file }}, with the SHA-256 of the file you were
shown: if the file changes, it is ignored again until you trust it again. **Stop trusting** revokes it for every later
session. A web build reads no working directory and installs no overlay.

### Settings sections from extensions

An extension can add to Settings the way it adds to the Store. A provider of the `settings_section` kind says a section's name, icon
and the form it asks for (a guatiao schema and the guatiao-intake form that shows it). It either sits in Settings' list under a heading
of its own (`Extensions` when it names none) or is a titled group at the end of an existing section's page (`under`). Its values are
kept in `[settings.ID]` of {{ config_file }}, or by the provider itself; a field the schema marks sensitive is never shown again, only said to
be set, and is not written to {{ config_file }}: it is kept as the secret `settings/ID/MEMBER` by the provider `[secrets] accounts` names (the
operating system's store unless you chose another), where the Secrets section lists it. A secret an older {{ config_file }} holds in
`[settings.ID]` is moved there the first time the section is opened. The host lists the sections it loaded with `settings/sections`. A section disappears from Settings with the extension that
offered it, and the values it kept stay in {{ config_file }} until you remove the table. The pages {{ name }} draws itself (General, Appearance,
Accounts and the rest) are listed by the host the same way, so the app learns what Settings has from the host and not from a list of its own.

### Panel tabs from extensions

The bottom panel of a conversation is one bar of tabs. Its + menu offers Terminal (and Terminal in each shell the host lists), the host's
Logs and what an extension adds: a provider of the `panel_tab` kind says a tab's name, icon and whether it shows a shell (the host's
own, or one it names), a log whose latest lines the provider answers, or a page the extension serves on this computer's loopback
(`http` or `https` on `127.0.0.1`, `[::1]` or `localhost`; any other address is refused). The host lists the kinds with `panel_tabs/list`.
Opening Logs and an extension's logs needs the key to hold `hosting`. A page's tab names its address and has a button that opens it in
your browser; {{ name }} embeds no web view, and a page on a host that runs on another computer is not offered, since its loopback is not yours.

### Copy and paste in a terminal

Every terminal screen (the Terminal section, the panel's shell tabs, a host's shell) copies and pastes as the terminals you know do, in the
desktop app, the web app and the browser extension's page. Copy takes the selected text: Ctrl+Shift+C or Ctrl+Insert (⌘C on a Mac), and on
Windows Ctrl+C while text is selected, which also drops the selection, so the next Ctrl+C is the interrupt; with nothing selected Ctrl+C goes
to the shell as it always did, and on Linux and macOS it always does. Paste sends the clipboard to the shell: Ctrl+Shift+V, Ctrl+V or
Shift+Insert (⌘V on a Mac). Ctrl+Shift+A (⌘A) selects everything the screen holds; Ctrl+A is the shell's. A right click opens Copy, Paste and
Select all, and the Terminal section's bar has Copy and Paste buttons. Selecting does not copy. A paste is one write to the shell, with each
line end sent as Enter, and wrapped in the bracketed-paste markers when the shell asked for them (a pasted end marker is dropped, so the text
cannot end its own paste); nothing limits its size or asks about several lines.

A browser hands a page a keyboard paste as its `paste` event, which asks for no permission, so the keys above paste through it; only the
menu's and the bar's Paste have to read the clipboard, which a browser lets a page do only after you allow it and only over HTTPS or on
localhost, and when it refuses, the terminal says so and names the keyboard paste. A copy writes from your key or click, with the browser's
copy command where the clipboard API is not there.

## Limits and warnings

{{ name }} does not refuse a choice because it is weak or unusual, and a limit is a setting with a default and no ceiling of its own. Where
something is told instead of refused, it is in the log as `non-default:`, in `{{ program }} check`, and where the app shows the account.

- **Plain http.** Any `http` or `https` address is accepted for an account's address (the API-gateway and LiteLLM kinds, and any
  kind's endpoint setting), a mail account's `server` and `autodiscover`, `[store] skills_search`, an MCP registry, the `address` of an `[sdk.NAME]` server,
  and the address of an MCP server a paired client hands a session. One that goes over plain `http`
  to somewhere other than this computer is told of: `{{ program }} check` and `{{ program }} accounts list` print it, the host logs it when it
  starts, and the account's row in Settings shows it. An address that holds a login is still refused.
- **Messages.** `[host] max_message_bytes` (16 MiB) is the longest ACP message, as one line, either way on a connection: a prompt
  carrying an attachment is one message, and base64 makes a file a third larger. There is no ceiling of {{ name }}'s own, so raise it to
  send a larger file. A message over it is refused with the setting named (the link's error, the host's log), and the connection
  carries on.
- **Listing MCP servers.** `[host.mcp_list]` `query_seconds` (5, all of one server's lists), `max_items_per_server` (1000) and
  `max_reply_bytes` (4 MiB, an HTTP body or a line of output) are what listing a session's tools and resources is held to; each is at
  least 1 and has no ceiling of {{ name }}'s own.
- **Worktrees.** `[host.worktrees]` `max_per_repository` (32) and `git_seconds` (120).
- **Delegation.** `[host.delegate]` `wait_seconds` (300, the longest one `task_status` waits) and `dial_seconds` (20, reaching an agent
  on another machine).
- **History and mail.** `[host.history]` `max_session_bytes` and a mail account's `timeout_secs` have no ceiling.
- **Foundry Local.** An account's `answer_seconds` (1800) is how long one answer may take.
- **Files.** The host answers `fs/list` and `fs/read` a page at a time and says where the next one starts, so a long folder or a large
  file is reachable whole.
- **Writing files.** `fs/write` (grants `files` and `files.write`) has no limit of its own beyond `[host] max_message_bytes`, which bounds the
  one request that carries the text; it never writes a place on your protected list, nor {{ config_file }} or the host's state folder without
  `files.write.host`.
- **Keep awake.** Settings > General > Power (`[settings.power] keep_awake`, off by default) keeps this computer from idle-sleeping while an
  agent turn runs, for as long as the turn does; it lets go when the turn ends, the connection closes or the host stops, however it
  stops. Read when the host starts.
- **Secrets.** A secret provider lists every name it holds; a name or a value has no length limit of {{ name }}'s.
- **The Store.** A program is offered for any platform its publisher has a build for.

## Windows, macOS and Linux tray behavior

{{ name }} uses a desktop tray controller and the shared Flutter app under root app/. The app can start without a visible main window, remain alive when its window closes, and expose Open {{ name }}, Hosting, Settings, Hide and Exit commands via the notification area. Closing the window leaves the app, its conversations, its own agents and `{{ program }}` running; Exit, or **Quit {{ name }}** in Settings, ends them.

What has been run, by platform:

- **Windows 11 (arm64)**: the icon is in the notification area (in its overflow, until the person pins it), named {{ name }}. Closing the window hides it while the host keeps serving, and the real extension in Edge ran a turn through that host with the window closed. Two clicks on the icon within half a second, Windows' default double-click time, open the window; that decision is tested through the plugin's events, and a double click by mouse was not run.
- **Linux (Fedora 44 under WSLg, arm64)**: tray_manager draws the icon as a StatusNotifier item, which passes no clicks on, so **Open {{ name }}** in its menu opens the chat. The app looks for a StatusNotifier host on the session bus (`org.kde.StatusNotifierWatcher`). With one, closing the window hides it, and picking Open {{ name }} from the menu over D-Bus, as a panel does, brings it back. WSLg's desktop has none, so there the app shows no icon, and closing the window minimizes it instead of hiding it where nothing could bring it back; WSLg does not report the window as minimized.
- **macOS**: not run.

The Settings action brings up the main window and opens Settings, a page (Ctrl+, from anywhere in the app) with a list of sections at its left: App (General, Appearance, Notifications, Voice, Shortcuts), Agents (Sources, Profiles, MCP servers, Permissions, Accounts, Shells, Usage), Store (Catalog sources, Advanced) and Extensions (Libraries, Providers), then About, and a search over their keywords. A value applies as it is changed and is written at once to the same {{ config_file }} file, not an unrelated preferences database; there is no Save or Cancel, and every section keeps the tables it does not edit. A value kept only on this computer is marked. Quitting is the tray's and the menu's, not Settings'. A system-hosted web application cannot overwrite its deployment {{ config_file }}.

Where the host grants the key `usage`, the app also has a Usage section of its own (its selector icon, with an index of All accounts and each account under its provider). Its Limits view shows each account's allowances as the host last heard them: how much is left, a bar, the reset, the pace and when the figure was read, with a warning in words once a reading is half an hour old. An account that several hosts report under the same identity is one allowance, the newest reading; Host, Provider and Account filters share the index's selection. The section holds the `[usage] warn_permille` choice, so Settings does not list Usage then. Accounts are still managed in Settings.

The default launch is visible, with close-to-tray enabled. The start-hidden option takes effect on the next launch.

## When a conversation needs you

A conversation needs the person when its agent asks leave to use a tool, asks a question, or finishes a turn (a turn the person
stopped does not count). If that conversation is not the one shown, or the window is hidden, minimized or not focused, the
desktop shows a notification, and choosing it opens that conversation. A notification names the conversation and its agent and
says which of the three it is, nothing else: never the command, the path or anything else the agent sent, which a lock screen or
a shared screen could show. `[notifications]` in {{ config_file }}, or Settings → **Notifications**, turns each kind off: `permission`,
`question` and `finished`, each on by default.

In the side list, a conversation waiting for an answer is marked **Needs you**, one whose turn is running shows that it works,
and one whose turn ended since it was last chosen has a dot. The tray says how many conversations wait for an answer: its
tooltip on Windows, the first item of its menu everywhere.

- **Windows**: a toast, through leanflutter's `local_notifier`. The first one creates a Start menu shortcut named {{ name }} to
  the running program (`%APPDATA%\Microsoft\Windows\Start Menu\Programs\{{ name }}.lnk`), which Windows requires to show a toast
  for a program not installed from a package; nothing is created until a notification is shown.
- **Linux**: a freedesktop notification through libnotify, which needs a notification service on the session bus.
- **Web, a PWA and the extension**: the browser's Notification API, once the person allows it from Settings → Notifications →
  **Allow notifications**; nothing registers a service worker. A page shows them while it is open.

`app/integration_test/notification_test.dart` has a conversation behind the chosen one finish its turn in the app's own window,
and holds the window open for `CANOA_TEST_HOLD` seconds. What has been run:

- **Windows 11 (arm64)**: Windows' own notification history for {{ name }} held the toast, titled with the conversation and saying
  its agent finished its turn. Choosing it was not run.
- **Linux (Fedora 44 under WSLg, arm64)**: WSLg has no notification service of its own; with `dunst` running, it received the
  notification from {{ name }} with the same words. Choosing it was not run.
- **Web and macOS**: not run.

## Building desktop runners

This repository keeps the canonical Flutter code in app/. `python app/tool/build.py desktop` does everything below for this machine's platform and prints the built executable (`--run` runs it, `--debug` builds debug). By hand, Flutter's platform runner directories must be generated before native desktop builds:

    cd app
    flutter create --org dev.boriken --platforms=windows,linux,macos --project-name canoa .
    sh tool/name_runner.sh
    flutter pub get
    flutter build windows  # use linux or macos on that platform

The generated runner names the executable `{{ program }}`, the command-line program's name, which Windows and macOS read as the
same file. `app/tool/name_runner.sh` renames it before the first build, as above: the executable becomes `{{ executable }}`
(`{{ executable }}.exe` on Windows), the window and the file's description are titled {{ name }}, and on macOS the bundle is `{{ name }}.app`
with the executable `{{ executable }}` (not yet run on a Mac). People see {{ name }} everywhere; only the file is `{{ executable }}`.

On Linux the tray and window packages (leanflutter's `tray_manager` and `window_manager`) link system libraries that no Dart package carries, so a build needs them installed, offline too:

    sudo apt-get install libayatana-appindicator3-dev libgtk-3-dev libx11-dev libxi-dev libnotify-dev

`libnotify-dev` is for notifications (`local_notifier`). On Fedora the packages are `libayatana-appindicator-gtk3-devel` and
`libnotify-devel`. Its version 0.6 marks a function tray_manager calls as
deprecated, and the generated runner builds with `-Werror`, so build with
`CXXFLAGS=-Wno-error=deprecated-declarations` there (Fedora 44).

A desktop build carries the host as a library, with no `{{ program }}` program: the app's own plugin, `app/native`, builds
`crates/canoa-ffi` with cargo as part of `flutter build`, so `cargo` has to be on PATH or in `~/.cargo/bin`, and the runner
has to be generated in `app/`, whose workspace the plugin finds from there. The result is `canoa_ffi.dll` beside
`{{ executable }}.exe`, or `lib/libcanoa_ffi.so` in a Linux bundle; a macOS build is written and has not been run. With
`CANOA_WEB_DIST` set to the absolute path of a web build, the library has the web app built in.

`app/integration_test/desktop_test.dart` runs the app on a real desktop. On a Linux session with no tray, run
`python3 integration_test/sni_watcher.py` first to have one: it is a minimal StatusNotifier watcher that draws nothing,
and the test picks the menu's entries over D-Bus.

The `deps` branch mirrors those packages' Dart side and names these in its `manifest.json`. The desktop CI workflow generates the appropriate runner on each OS with the app's identifier and builds it. Windows, macOS and Linux compilation have been verified in GitHub Actions; the tray on a full Linux desktop panel (KDE, GNOME with its AppIndicator extension) has not been run. Platform-native plugins and tray host support are required. A standard Flutter Web/PWA build does not create a system notification icon.

## Limitations

{{ name }} is still an early-stage app. The tray controller preserves the process, in-memory sessions and the host while its window is hidden; it does not replace a native background service or automatically install a startup entry. After a restart, sessions are listed from `sessions.json` and open again only through their agents, which have to offer `session/load` or `session/resume`.
