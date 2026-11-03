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
- [harnesses] the harness each name an agent reports in `initialize` stands for, such as `"my-acp" = "My Harness"`. It extends and overrides the built-in names for Claude Code, Codex, Gemini, Pi, OpenCode and Grok; an agent named nowhere is labelled by its entry.
- [overlays], [profile] and `[agents.NAME.profile]`: profiles, described [below](#profiles).

The session list is `sessions.json` beside Canoa.cfg on a desktop, and this origin's IndexedDB in a web build, a PWA or the extension. Removing a session from it never deletes the agent's own session.

Do not store identity private keys, tokens or passwords in Canoa.cfg.

## Profiles

A profile is what an agent is launched with: environment variables, an initial context, skills, MCP servers, and the
model, effort and mode it starts with. It comes in layers, merged lowest first:

1. **built-in**: Canoa's own, which sets nothing;
2. **overlays**, each named in `[overlays]`, in the file's order;
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
Which of these each harness can take, and when, is in [the roadmap](roadmap.md); applying a profile to a harness is
not done yet.

```toml
[overlays]
team = "https://github.com/example/canoa-profiles.git@main#team"   # <repository>@<ref>#<path>, as dotagents writes it
mine = { dir = "profiles/mine" }                                   # a directory, relative to Canoa.cfg's

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

A profile file has the same keys at its top level (`model = ...`, `[env.NAME]`). Each entry takes exactly one form, and
a key the schema does not know is refused. Names: environment variables a letter or `_` then letters, digits or `_`;
skills, MCP servers and headers 1 to 64 letters, digits, `-` or `_`. A relative path is read from its layer's
directory: Canoa.cfg's for yours and an agent's, the overlay's, or the working directory for a project. **An overlay
or a project comes from elsewhere**, so its paths must be relative and stay inside its directory: no root, drive, `~`,
`:` or `..`, and a file it names for a secret is read only if, its links followed, it is still inside.

**Secrets** come from a reference, `from_env` or `from_file`, and are never written into a profile. A reference is
resolved only when a session starts; the view shows the reference, and nothing logs the value. Errors name a key, never
a value.

### Overlays

An overlay is a directory holding `profile.toml` and whatever it names, such as skills. From git, the desktop app
installs it into `overlays/NAME/` beside Canoa.cfg, with `overlays/NAME.json` recording the source, **the commit** and
each file's git blob id, and its history in `overlays/.cache/NAME.git`. **Install…** and **Check for changes…** in
Settings → Profiles fetch the source and show the commit it would move to and every file it adds, changes or removes;
nothing changes until you apply, and what is applied is that commit, wherever the ref has moved since. The ref is a
branch, a tag or a commit id. An overlay is upstream's copy, replaced whole on each apply: put your own additions in
`[profile]`, which wins over it. An overlay installed from another source than the one Canoa.cfg names is left out
until it is synced.

Nothing from a source runs: its files are read out of git's object store, never checked out, so no hook, filter or
attribute of the source applies; links and submodules are listed and never installed, and a file name that is not safe
on every platform (`..`, `\`, `:`, a Windows device name) refuses the whole overlay. Git may fetch only over `https`,
`http`, `ssh`, `git` and `file`, never a transport helper such as `ext::`, and never prompts. An overlay holds at most
4096 files and 64 MiB. A local directory, `{ dir = "..." }`, is read in place, as it is when an agent starts.

### Trusting a project

A project profile is ignored until you trust its directory in Settings → Profiles, which shows the file first. The
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

## Building desktop runners

This repository keeps the canonical Flutter code in app/. Flutter's platform runner directories must be generated before native desktop builds:

    cd app
    flutter create --org dev.boriken --platforms=windows,linux,macos .
    flutter pub get
    flutter build windows  # use linux or macos on that platform

On Linux the tray and window packages (leanflutter's `tray_manager` and `window_manager`) link system libraries that no Dart package carries, so a build needs them installed, offline too:

    sudo apt-get install libayatana-appindicator3-dev libgtk-3-dev libx11-dev libxi-dev

On Fedora the package is `libayatana-appindicator-gtk3-devel`. Its version 0.6 marks a function tray_manager calls as
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
