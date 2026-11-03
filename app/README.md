# Canoa Flutter app

The canonical Canoa chat UI lives at the repository root under `app/`. The
standalone PWA, desktop builds and Chrome extension side panel/full-page
window share this Dart/Flutter application; the extension retains privileged
browser operations and browser-owned permission policy in
`bindings/browser/chromium/`.

## Development

```sh
cd app
flutter pub get
flutter run -d chrome
flutter test
flutter analyze
```

## Output choices

```sh
cd app
flutter build web --release
flutter build web --release --wasm
```

A platform-independent packager at `tool/build.py` can build a standalone
PWA or copy a locally bundled Flutter output into the Chromium extension.
The `--wasm` output is appropriate for deployment on a server that can set
cross-origin isolation headers; its use inside a Chromium Manifest V3 extension
must also be checked against the extension's CSP and renderer requirements.

## App state

The current UI offers a responsive session list, independent transcripts and
a composer. It starts in an **explicit offline preview**. Messages typed into
a preview conversation are kept only in memory and are never sent to an agent.
The WebTransport-first/WSS-fallback transport is connected to an ACP v2 controller **through an injected verified-peer interface**. The Flutter session store and composer can process streaming text, insertion acknowledgments and state transitions from a verified client. The experimental coqui-peer WebCrypto method and explicit pinned-peer dialog are available. Production Coqui pairing/host authorization and remote MCP browsing are still missing; the normal app remains in offline preview. The list of sessions is kept across runs: see [Sessions by harness](#sessions-by-harness).

`lib/session_store.dart` is the shared view model. Transport and browser
permissions must remain independent of it. A real transport must perform the
interoperable peer-auth handshake and bind a verified peer before attaching
an ACP session.

## ACP v2 integration

The [ACP client](lib/acp/client.dart) and [connection orchestrator](lib/acp/connect.dart) intentionally cannot send ACP methods before a supplied verifier proves the expected peer identity. The `SessionStore.attachVerifiedClient` API binds that initialized client to one local/remote session and routes updates by the agent-issued Session ID. See [protocol documentation](../docs/acp-client.md).

## Shared dependencies

The browser cryptographic methods are owned by [coqui-peer-noise](https://github.com/Boriken-Dev/coqui-peer-noise) and [coqui-peer-webcrypto](https://github.com/Boriken-Dev/coqui-peer-webcrypto) and vendored in `app/web/vendor` at the exact recorded Git blobs in `COQUI-SOURCE.json`, one entry per file. The app and the extension each prove an agent by the method its record names, with a key of each method's own, and pair by PIN under Noise. A browser whose WebCrypto has no X25519 shows a Noise agent as unavailable, with the reason, and never tries the other method. A native build proves by neither: its connect dialog says so. Browser-side UI code never implements its own ECDSA handshake. The early-message handoff is provided by `coqui_web` as a local Dart package, not a Canoa authentication class.

To upgrade the browser method, update its canonical Coqui source and tests first, then replace the exact vendored source and source metadata together; the vendor tests will reject an unrecorded change.

## Smoke testing remote chat

Use the [authenticated ACP echo fixture](../examples/acp-echo/README.md) to verify the WebSocket path from the connection dialog. It requires trusted TLS, explicit Coqui peer allowlisting, and a pinned server key. This is diagnostic only, not a deployed agent service.

## Agents on this computer

The desktop app runs any program that speaks ACP on its standard streams, with no bridge and no pairing, since it is the person's own program. The connect dialog offers Claude, Codex, Gemini and Pi when what each needs is installed: Claude and Codex through their ACP adapters, fetched with `npx`, driving the tool you have already signed in to, so its subscription applies and no API key is involved; Pi through `pi-acp`, also fetched with `npx`, with whatever model and key Pi itself is set up with. Any other agent runs by typing its command in the dialog, or by naming it in `Canoa.cfg`, under `[agents.NAME]` with `program` and `args`. With no folder named, it starts in your home directory. The first run downloads the adapter. Each permission request waits in the conversation until you answer, and closing the session ends the agent and anything it started.

Canoa can only show what an agent asks. An adapter drives your installed tool under that tool's own settings, so what you have already allowed there, such as Claude's `permissions.allow` rules, runs without a question here. Tool calls still appear in the conversation as they happen.

An agent's Markdown is untrusted: images are not fetched, and a link is copied rather than opened.

## Sessions by harness

Every session the app opens or resumes is noted in a ledger (`lib/ledger/`): its harness, the version the agent reported, the agent entry it came from, where it runs (this computer, the app's `canoa-host`, a paired peer with its id, or the browser's provider), its working directory, the agent's own session id, its title, and when it was created and last used. The side list shows those not open now, grouped by harness, with a chip per harness to show only that one. **Reopen** starts the entry's agent again, or reaches the peer again, and asks it for the session by `session/load` or `session/resume`, whichever its capabilities offer; where it offers neither, or the session came through the browser's provider, the row says so instead. Removing a row removes Canoa's note only, never the agent's own session, and the confirmation says so.

The harness comes from what `initialize` reports. Claude's, Codex's and Pi's adapters report their own package names and Grok reports none, so a map gives each its harness: Claude Code, Codex, Gemini, Pi, OpenCode and Grok are built in, `[harnesses]` in Canoa.cfg adds or overrides names, and an agent the map does not know is labelled by its entry or preset name. The version shown is the reporter's, named after it when that is an adapter (`claude-agent-acp 0.99.1`).

On a desktop the ledger is `sessions.json` beside Canoa.cfg; on the web, in the PWA and in the extension it is this origin's IndexedDB (`canoa-sessions`). `[sessions] keep` bounds it (200 by default; the least recently used go first). What was said is not kept unless **Keep what was said with each session** is turned on in Settings (`keep_text`, at most `text_messages` per session, each message cut at 64 KiB); turning it off, or **Clear what was said**, removes what was kept. Kept text is shown when a reopened agent does not send the conversation again, or, for a session that cannot be reopened, from **Show what was kept**.

`integration_test/session_ledger_test.dart` proves it across a real restart, in two runs of the app sharing a directory:

```sh
cd app
CANOA_TEST_LEDGER_DIR=<empty dir> CANOA_TEST_LEDGER_STEP=record flutter test integration_test/session_ledger_test.dart -d windows
CANOA_TEST_LEDGER_DIR=<same dir>  CANOA_TEST_LEDGER_STEP=reopen flutter test integration_test/session_ledger_test.dart -d windows
```

The scripted stand-in plays Claude's adapter, a name Canoa.cfg maps to Pi, and an agent that names nothing and cannot reopen. The second run checks it is another process, finds each harness's group, filters by Pi, reopens the Claude Code session and gets an answer in it, and removes Pi's row while Pi's own session stays. `test/ledger_web_test.dart` runs the IndexedDB backend in a browser: `flutter test --platform chrome test/ledger_web_test.dart`.

## Desktop tray and Canoa.cfg

The app reads a TOML Canoa.cfg just like AI6's OO.cfg. Closing the window keeps the app, its conversations and what it hosts running in the tray, whose menu has Open, Hosting, Settings, Hide and Exit; a double click on the icon opens the chat on Windows, and on Linux the menu's Open does. Where Linux shows no tray icons at all, closing minimizes the window instead, and Settings has **Quit Canoa**. Refer to the [configuration guide](../docs/configuration.md). Build platform runners with flutter create --org dev.boriken --platforms=windows,linux,macos . in app/ before flutter build windows/linux/macos.

The desktop app runs `canoa-host` as its child (`lib/desktop/host_supervisor.dart`): the one `CANOA_HOST` names, else the one beside its executable. **Hosting** adds an agent, pairs and removes keys and shows a PIN; see [the host](../docs/host.md#in-the-desktop-app).

`integration_test/desktop_test.dart` runs the app on a real desktop (`flutter test integration_test/desktop_test.dart -d windows`, or `-d linux`) with `CANOA_TEST_HOST`, `CANOA_TEST_NODE` and `CANOA_TEST_APP` as `test/host_test.dart` describes.

## A coding task, end to end

`integration_test/coding_task_test.dart` does what a person would, in the desktop window and through its widgets only: it makes a throwaway Node project with a failing test, runs an agent there, has it fix the test, add one and rename a function, allows every question but one, stops a turn and continues it, attaches a file, closes the conversation and opens it again from **Earlier conversations**. Then it checks the project on disk, and that every kind of update the agent sent has a rendering of its own. Anything reaching outside the project's temporary directory fails the test: a question about it is refused and counted, and a tool that ran there unasked is counted too.

```sh
cd app
flutter test integration_test/coding_task_test.dart -d windows                              # scripted agent, nothing spent
CANOA_TEST_TASK_AGENT=Claude flutter test integration_test/coding_task_test.dart -d windows  # Claude, on your sign-in
```

With `Claude`, the test runs the preset's adapter through `integration_test/without_user_settings.mjs`, which sets `_meta.claudeCode.options.settingSources` to project and local on each session it opens: Claude Code then starts without your user-level settings, `~/.claude/CLAUDE.md` and user skills, while your login still applies. The preset people use is unchanged.

`node` has to be on PATH. `CANOA_TEST_SHOTS=<dir>` keeps a screenshot of each step, and `CANOA_TEST_UPDATES=<file>` the kind and shape of each distinct update. The file dialog is the one thing the test answers for the person: it picks the attached file itself. Reopening here is done after closing a conversation; after quitting, `integration_test/session_ledger_test.dart` does it.

Outside the test, `CANOA_EVENT_LOG=<file>` makes the app append the kind, shape and rendering of each distinct ACP update it receives, as a JSON line. A shape keeps the protocol's own words, such as a tool's `kind`, and replaces every other string with `"string"`, so nothing anyone wrote is kept. `test/fixtures/coding_task_updates.jsonl` is that record from a real Claude run; `test/chat_test.dart` replays it and fails if any kind would reach the fallback, which shows nothing.
