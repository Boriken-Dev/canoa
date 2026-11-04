# Desktop and Canoa.cfg

Canoa uses the **same deployment configuration convention** as AI6's Outer Office: a user-level TOML file ending in .cfg, with the environment override taking priority.

The default template is [examples/config/Canoa.cfg](https://github.com/Boriken-Dev/canoa/blob/main/examples/config/Canoa.cfg).

| Platform | Config location |
| --- | --- |
| Windows | %APPDATA%\Canoa\Canoa.cfg |
| macOS | ~/Library/Application Support/Canoa/Canoa.cfg |
| Linux | $XDG_CONFIG_HOME/Canoa/Canoa.cfg or ~/.config/Canoa/Canoa.cfg |
| Override | CANOA_CONFIG |
| PWA / extension | Canoa.cfg alongside the deployed index.html, read-only |

Configuration tables:

- [desktop] start_hidden, close_to_tray, minimize_to_tray
- [network] prefer_webtransport, connect_timeout (seconds, between 1 and 120)
- [assets] cdn = "none" or "local" (deployment option; font hosting remains a separate build concern)
- [host] what `canoa-host` serves: [host.agents.NAME] and [host.providers.NAME], described in [the host](host.md). The app keeps this table as it is when it saves its settings.
- [sessions] the list of sessions kept across runs: keep (how many, default 200, 0 for none; the least recently used go first), keep_text (whether what was said is kept with each, default false), text_messages (how many of a session's latest messages, default 200)
- [notifications] permission, question, finished: which of these, in a conversation the person is not looking at, reach them as a notification outside the window; each on by default. See [below](#when-a-conversation-needs-you).
- [permissions] full_access: the agents, by the label the app shows, whose sessions start with full access, such as `full_access = ["Claude"]`. Empty by default. Full access answers each permission request that offers to allow once with that option and no question, and is shown on the conversation and its row while it lasts; see [permission requests in the chat](acp-client.md#permission-requests-in-the-chat).
- [harnesses] the harness each name an agent reports in `initialize` stands for, such as `"my-acp" = "My Harness"`. It extends and overrides the built-in names for Claude Code, Codex, Gemini, Pi, OpenCode and Grok; an agent named nowhere is labelled by its entry.
- [overlays], [profile] and `[agents.NAME.profile]`: profiles, described [below](#profiles).
- [catalogs] and [store]: the overlay store's catalogs and limits, described [below](#the-overlay-store).

The session list is `sessions.json` beside Canoa.cfg on a desktop, and this origin's IndexedDB in a web build, a PWA or the extension. Removing a session from it never deletes the agent's own session.

### A worktree per session

**In a new git worktree, on its own branch** in the connect dialog starts the session in a new worktree of the repository holding the workspace, from that directory's current commit. The branch is `canoa/` and the session's title (or its agent's name) in lower-case letters and digits joined by hyphens, then six random hexadecimal digits, such as `canoa/fix-the-tests-3fa91c`; nothing else is taken, so no branch reads as an option, a range or a path step. The worktree goes beside the repository, in `<repository>.worktrees/<branch without canoa/>`, and a `.worktrees` directory already there that is a link is refused. The session list shows the branch, and the session list kept across runs notes the worktree with the session.

- **An agent this app runs**: the app runs `git` itself, with an argument list, as you would in your own repository: its hooks run.
- **An agent `canoa-host` serves**: the host makes the worktree, only inside its entry's working roots, as [the host](host.md#worktrees) describes; with no working root, or `worktrees = false`, it makes none. The browser's provider makes none.

Closing the session ends its agent first, then asks whether to keep the worktree. Files with changes not committed, untracked ones included, are counted, and removing them is a button of its own that names them; a change made after you were told is told again before anything is deleted. Without that, git refuses to remove a worktree holding changes. Files git ignores go with the worktree. Its branch goes with it only when it holds no commit another branch, tag or remote does not have; otherwise it stays, so no commit is lost. Reopening a session from the list finds its worktree again, or says it is gone and does not start the agent.

Do not store identity private keys, tokens or passwords in Canoa.cfg.

## Profiles

A profile is what an agent is launched with: environment variables, an initial context, skills, MCP servers, and the
model, effort and mode it starts with. It comes in layers, merged lowest first:

1. **built-in**: Canoa's own, which sets nothing;
2. **overlays**, each named in `[overlays]`: [AOS](https://github.com/Boriken-Dev/agent-overlay-spec) overlays, in their composition order (`priority`, then name);
3. **yours**: `[profile]` in Canoa.cfg;
4. **the project's**: `.canoa/profile.toml` in the working directory, only once you trust that directory;
5. **the agent's entry**: `[agents.NAME.profile]`, where `NAME` is an agent's label (an entry with only a `profile` names
   a preset, such as `Claude`);
6. **the session** being started.

A later layer wins per key. Environment variables, skills and MCP servers merge by name: a later layer's entry replaces a
lower one's whole and keeps its place, and `remove = true` takes it out. `unset = ["model"]` takes back a lower layer's
`model`, `effort`, `mode` or `context`. **Settings → Profiles** shows the effective profile of each agent, every value
beside the layer it came from, and what each layer removed. canoa-host reads the same schema
(`canoa_host::profile`); `crates/canoa-host/tests/vectors/profile.json` holds the app and the host to the same merge.
What each harness takes of it, and how, is [below](#applying-a-profile).

```toml
[overlays]
team = "https://github.com/example/canoa-profiles.git@main#team"   # an AOS source string, <repository>@<ref>#<path>
mine = { dir = "profiles/mine" }                                   # a directory, relative to Canoa.cfg's
# pinned = { source = "...", pin = "sha256:..." }                 # what installing writes: the tree digest it checks

[profile]
model = "claude-sonnet-4-5"
effort = "high"
mode = "plan"
context = { text = "Answer in Spanish." }      # or { file = "CONTEXT.md" }
unset = ["mode"]                               # not beside a value it takes back
share_env = ["GITHUB_TOKEN"]                   # variables overlays and projects may read; only in your own layers

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
directory: Canoa.cfg's for yours and an agent's, the overlay's, or the working directory for a project.

**An overlay or a project comes from elsewhere**, so it is held to more:

- Its paths must be relative and stay inside its directory: no root, drive, `~`, `:` or `..`, and a file it names for
  a secret is read only if, its links followed, it is still inside.
- It reads only the variables of this computer that one of your own layers lists in `share_env`, such as
  `share_env = ["GITHUB_TOKEN"]` in `[profile]`. An environment variable or an MCP server of its that reads any other
  is left out, the lower layer's entry stays, and Settings → Profiles lists it as left out and what it reads.
- It may not set the variables that decide what a process runs or loads, where an agent's own requests go, where a
  harness keeps its configuration, or which certificates are trusted: `PATH`, `NODE_OPTIONS`, `HOME`, `TMP`,
  `CLAUDE_CONFIG_DIR`, `CODEX_HOME` and the like, anything starting `LD_`, `DYLD_`, `PYTHON`, `GIT_`, `NPM_CONFIG_`,
  `CANOA_` or `COQUI_`, and anything ending `_BASE_URL` or `_PROXY`, in any case. Your own layers may set them. The
  whole list is `RESERVED_ENV` in `canoa_host::profile`, with every variable the AOS specification names sensitive.
  An overlay that sets one anywhere (`env`, an env file, `paths`, an MCP server) is left out whole.

**Secrets** come from a reference, `from_env` or `from_file`, and are never written into a profile. A reference is
resolved only when a session starts; the view shows the reference and the variable each MCP header or variable comes
from, and nothing logs the value. A `from_file` reads a regular file of at most 1 MiB. Errors name a key, never a
value.

### Applying a profile

The desktop app applies a profile to each agent it starts on this computer, its own way for each harness. Before the
session starts, a dialog shows every part of the profile and what that harness takes of it, every layer left out and
why, and each MCP server: what it runs or the URL it reaches, the variables or headers it receives by name (never their
values), what is left out of them, and a switch. Once the session is open, the conversation says in one line what
applied and what did not, and **Profile** in its header lists each part, its layer, what became of it and through what.
Nothing is applied that is not listed there. An agent reached on another machine, through the host or a browser, is
given none of it, and its conversation says so; serving profiles from `canoa-host` is not done yet.

| Part | Claude Code (`claude-agent-acp`) | Codex (`codex-acp`) | Gemini CLI | Pi (`pi-acp`) | Another agent |
| --- | --- | --- | --- | --- | --- |
| Environment | the agent's process | the agent's process | the agent's process | the agent's process | the agent's process |
| Initial context | ACP `systemPrompt`, appended | ACP `systemPrompt`, appended | not supported | not supported | ACP `systemPrompt` when it advertises it |
| Skills | a Claude Code plugin, `canoa-profile`, through the adapter's `_meta.claudeCode.options.plugins`; a skill is `canoa-profile:<name>` | the Agent Skills directory of a new directory given as ACP `additionalDirectories` | not supported | not supported | not supported |
| MCP servers | ACP `mcpServers`: stdio, HTTP, SSE | ACP `mcpServers`: stdio, HTTP; skipped by Codex when its own configuration names a server the same | ACP `mcpServers` | not supported: pi-acp accepts them and starts none | ACP `mcpServers`, as it advertises |
| Model, effort, mode | the session's config option offering that value | the same | the same | the same | the same |

Which harness an agent is comes from what it says in `initialize`, else from the preset it was started from. A part a
harness does not take is listed as not supported, with why: Gemini CLI, Pi, OpenCode and Grok read context and skills only
from their own directories when they start. What the dialog says is what Canoa expects; the conversation says what the
agent itself advertised.

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
**Keep these choices** in the dialog writes them to Canoa.cfg, under the agent:

```toml
[agents.Claude.mcp]
files = true                 # on
docs = false                 # off
team = "sha256:..."          # an overlay's or a project's server, allowed at this definition: one that changes is off again
```

### Overlays

An overlay is an [Agent Overlay Specification](https://github.com/Boriken-Dev/agent-overlay-spec) (AOS) v0 overlay: a
directory whose files take effect by convention (`skills/<name>/SKILL.md`, `CONTEXT.md`, `rules/<name>.md`, the file
`env`), with an optional `overlay.cfg` manifest, TOML, or JSON after a `#!json` line; YAML is refused. Canoa reads the
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
- `requires`, among the overlays `[overlays]` names: a missing one is noted and the overlay applies without it, and
  overlays in a cycle are left out.

Canoa does not apply yet, and **Settings → Profiles** lists as "not applied by Canoa yet": env scripts (`env.<ext>`,
`env_scripts`), `setup`, dependencies (`lib/python/requirements.txt`, `lib/node/package.json`, `runtime`), `runtime`
probes, `scripts`, `mcp.skills`, an MCP server's `timeout` and its working directory (it runs in the session's), other
harnesses' hints, and the
`bin/` and `lib/` defaults, which would change `PATH`, `PYTHONPATH` and `NODE_PATH`. Nothing of an overlay ever runs.

From git, the desktop app installs an overlay into `overlays/NAME/` beside Canoa.cfg, with `overlays/NAME.json`
recording the source, the commit, each file's git blob id and **the tree digest** of what it wrote, and its history in
`overlays/.cache/NAME.git`. **Install…** and **Check for changes…** in Settings → Profiles fetch the source and show
the commit, the tree digest, every file it adds, changes or removes, and its `overlay.cfg`; nothing changes until you
apply, and what is applied is that commit's files, checked against the digest as they are written. Applying **pins**
the overlay in Canoa.cfg by its digest, `NAME = { source = "...", pin = "sha256:..." }`: a later check whose files
have another digest installs nothing until you change the pin. Each time profiles are built the installed copy is
digested again, and one whose files changed, or that holds an entry no overlay has (`.git`, `node_modules`, ...), is
left out until synced. An overlay with no pin, or a `{ dir = "..." }` with none, is reported as `unpinned` each time.
A pin is a SHA-256 over every file's bytes as published, the same on every platform; the commit is kept beside it.
The ref is a branch, a tag, or a whole commit id, which names that commit even if a branch or tag carries the same
name; an abbreviated id is not taken. Only that ref is fetched, one commit deep. An overlay is upstream's copy,
replaced whole on each apply: put your own additions in `[profile]`, which wins over it. An overlay installed from
another source than the one Canoa.cfg names is left out until it is synced.

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

### The overlay store

**Store…** in Settings → Profiles lists the overlays of the catalogs `[catalogs]` names, to search, read about and
install. A catalog is an [AOS catalog](https://github.com/Boriken-Dev/agent-overlay-spec/blob/main/spec/v0/catalog.md):
a file in a git repository, TOML, JSON (or, with any other name, `overlay.cfg`'s rule), listing overlays by name and
source, with a summary, an icon, tags and the like for each. Canoa fetches it as it fetches an overlay (git objects at
the ref named, one commit deep, never checked out, with the same limits), and reads YAML in neither.

```toml
[catalogs]
team = "https://github.com/example/overlays.git@main#catalog.toml"   # a label of yours = <repository>@<ref>#<catalog file>

[store]
catalog_max_bytes = 4194304   # a catalog file's size, 4 MiB
catalog_max_depth = 64        # how deeply it nests
catalog_max_entries = 2000    # entries listed from one catalog, by name; the rest are left out and counted
catalog_max_icons = 256       # distinct icon files read for one catalog, each once
icon_max_bytes = 524288       # an icon's size, 512 KiB
icon_max_width = 1024         # the width and height its header may give
icon_max_height = 1024
```

A label is yours, a valid overlay name unique among your catalogs; it never comes from the catalog. Each `[store]`
limit is a setting with the specification's default and no floor; one set above its default is shown in the store as
looser than the default.

- **A listing** shows each overlay's icon, name, summary, tags, its catalog's label and its source (with no user
  information or query). It searches by words and filters by tags. A name two catalogs offer is shown once per catalog,
  as `label/name`; entries are never merged. A source relative to the catalog is the catalog's repository at its ref;
  a catalog from a repository may not name a path on this computer, and such an entry is refused. Each refused entry,
  each field left out and each refused icon is listed under its catalog.
- **What a catalog says is its author's claim.** An overlay's page shows it under "What the catalog says", with the
  source and the pin it claims. **Fetch and show what it would do** fetches the overlay, installing nothing, and shows
  what the overlay itself says (its name, description, `[about]` and icon, from its own files), the commit and tree
  digest, and what it would do as Settings → Profiles would apply it: its environment, skills, MCP servers and context,
  what Canoa does not apply (setup, env scripts, dependencies, commands, the `bin`/`lib` defaults), each setup and
  whether it says it needs the network, the overlays it requires, what none of your layers shares, and its weakenings.
  A tag or a summary is never taken as evidence of any of it.
- **A catalog's pin** is checked against the tree fetched: one that does not match is shown as a refusal and the
  overlay cannot be installed from that entry. An entry with no pin is shown as unpinned.
- **Install and pin** installs exactly what was shown, through the path **Install…** takes, and names the overlay in
  `[overlays]` as `{ source, pin }`, pinned to the digest checked. **Update** shows what an update changes first. A pin
  Canoa.cfg already gives the overlay holds as it does for **Check for changes…**: a tree of another digest installs
  only once you choose **Update and move the pin…** and confirm the move, naming both digests. **Remove…** takes the
  overlay out of `[overlays]` and deletes its files and fetched history. An overlay of the same name that Canoa.cfg
  names from another source is neither installed over nor removed. Nothing of an overlay runs, from the store or
  otherwise. Catalogs are fetched into `overlays/.catalogs/LABEL.git`; what a page fetched and did not install is
  deleted when the page closes.
- **Text from a catalog or a manifest** is shown as plain text, with every control, invisible or bidirectional
  character named as `<U+XXXX>`; a line break is named too wherever one line is shown, and a description is set
  apart, after the source, pin, commit and digest. A source is shown without user information or query, and one that
  holds a password is refused, in `[catalogs]` and from a catalog.

Only git sources with a branch, tag or commit install: an entry whose source is a directory, a URL that is not git, or
git with no ref is listed, and its page says why it does not install. `homepage` and `repository` are shown as text to
copy, never fetched. A web build lists no catalog.

### Trusting a project

A project profile is ignored until you trust its directory in Settings → Profiles, which shows the file first, every
invisible or control character in it named as `<U+XXXX>`. The
trust is kept per directory, links resolved, in `trust.json` beside Canoa.cfg, with the SHA-256 of the file you were
shown: if the file changes, it is ignored again until you trust it again. **Stop trusting** revokes it for every later
session. A web build reads no working directory and installs no overlay.

## Windows, macOS and Linux tray behavior

Canoa uses a desktop tray controller and the shared Flutter app under root app/. The app can start without a visible main window, remain alive when its window closes, and expose Open Canoa, Hosting, Settings, Hide and Exit commands via the notification area. Closing the window leaves the app, its conversations, its own agents and `canoa-host` running; Exit, or **Quit Canoa** in Settings, ends them.

What has been run, by platform:

- **Windows 11 (arm64)**: the icon is in the notification area (in its overflow, until the person pins it), named Canoa. Closing the window hides it while the host keeps serving, and the real extension in Edge ran a turn through that host with the window closed. Two clicks on the icon within half a second, Windows' default double-click time, open the window; that decision is tested through the plugin's events, and a double click by mouse was not run.
- **Linux (Fedora 44 under WSLg, arm64)**: tray_manager draws the icon as a StatusNotifier item, which passes no clicks on, so **Open Canoa** in its menu opens the chat. The app looks for a StatusNotifier host on the session bus (`org.kde.StatusNotifierWatcher`). With one, closing the window hides it, and picking Open Canoa from the menu over D-Bus, as a panel does, brings it back. WSLg's desktop has none, so there the app shows no icon, and closing the window minimizes it instead of hiding it where nothing could bring it back; WSLg does not report the window as minimized.
- **macOS**: not run.

The Settings action brings up the main window and opens the Canoa settings dialog. Saving writes the same Canoa.cfg file, not an unrelated preferences database. A system-hosted web application cannot overwrite its deployment Canoa.cfg.

The default launch is visible, with close-to-tray enabled. The start-hidden option takes effect on the next launch.

## When a conversation needs you

A conversation needs the person when its agent asks leave to use a tool, asks a question, or finishes a turn (a turn the person
stopped does not count). If that conversation is not the one shown, or the window is hidden, minimized or not focused, the
desktop shows a notification, and choosing it opens that conversation. A notification names the conversation and its agent and
says which of the three it is, nothing else: never the command, the path or anything else the agent sent, which a lock screen or
a shared screen could show. `[notifications]` in Canoa.cfg, or Settings → **Notifications**, turns each kind off: `permission`,
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

This repository keeps the canonical Flutter code in app/. Flutter's platform runner directories must be generated before native desktop builds:

    cd app
    flutter create --org dev.boriken --platforms=windows,linux,macos .
    flutter pub get
    flutter build windows  # use linux or macos on that platform

On Linux the tray and window packages (leanflutter's `tray_manager` and `window_manager`) link system libraries that no Dart package carries, so a build needs them installed, offline too:

    sudo apt-get install libayatana-appindicator3-dev libgtk-3-dev libx11-dev libxi-dev libnotify-dev

`libnotify-dev` is for notifications (`local_notifier`). On Fedora the packages are `libayatana-appindicator-gtk3-devel` and
`libnotify-devel`. Its version 0.6 marks a function tray_manager calls as
deprecated, and the generated runner builds with `-Werror`, so build with
`CXXFLAGS=-Wno-error=deprecated-declarations` there (Fedora 44).

A desktop build carries `canoa-host` beside the app's executable, where the app finds it: `cargo build --release -p
canoa-host`, then copy `target/release/canoa-host` (`.exe` on Windows) into the bundle's directory, as the desktop
workflow does.

`app/integration_test/desktop_test.dart` runs the app on a real desktop. On a Linux session with no tray, run
`python3 integration_test/sni_watcher.py` first to have one: it is a minimal StatusNotifier watcher that draws nothing,
and the test picks the menu's entries over D-Bus.

The `deps` branch mirrors those packages' Dart side and names these in its `manifest.json`. The desktop CI workflow generates the appropriate runner on each OS with Boriken Dev's reverse-DNS namespace and builds it. Windows, macOS and Linux compilation have been verified in GitHub Actions; the tray on a full Linux desktop panel (KDE, GNOME with its AppIndicator extension) has not been run. Platform-native plugins and tray host support are required. A standard Flutter Web/PWA build does not create a system notification icon.

## Limitations

Canoa is still an early-stage app. The tray controller preserves the process, in-memory sessions and the host while its window is hidden; it does not replace a native background service or automatically install a startup entry. After a restart, sessions are listed from `sessions.json` and open again only through their agents, which have to offer `session/load` or `session/resume`.
