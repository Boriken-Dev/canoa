# Canoa — repository orientation

Canoa uses a Rust workspace with focused crates, public contracts, cross-platform tests and a `clients/` directory for the browser extension and other chat clients of the host.

| Shipped header | Covers |
| --- | --- |
| [`crates/canoa/AGENTS.md`](crates/canoa/AGENTS.md) | the `canoa` crate's public API |
| [`crates/canoa-foundry/AGENTS.md`](crates/canoa-foundry/AGENTS.md) | `canoa-foundry`: the OpenAI and Anthropic APIs over a local Foundry Local model, and its account kind |
| [`crates/canoa-litellm/AGENTS.md`](crates/canoa-litellm/AGENTS.md) | `canoa-litellm`: the account kind of a LiteLLM proxy, one key for the OpenAI and Anthropic APIs, signed in through the proxy's SSO or set, with the key's budget |
| [`crates/canoa-openai/AGENTS.md`](crates/canoa-openai/AGENTS.md) | `canoa-openai`: the speech-to-text service the accounts that hold an OpenAI API key can call |
| [`crates/canoa-piper/AGENTS.md`](crates/canoa-piper/AGENTS.md) | `canoa-piper`: Piper's program and voices as a Store type, and the speech-output provider that reads text aloud with the installed copy |
| [`crates/canoa-whisper/AGENTS.md`](crates/canoa-whisper/AGENTS.md) | `canoa-whisper`: whisper.cpp's program and models as a Store type, and the speech offer made of the installed copy |
| [`crates/canoa-microsoft-mail/AGENTS.md`](crates/canoa-microsoft-mail/AGENTS.md) | `canoa-microsoft-mail`: Exchange Server, Microsoft Graph, and classic Outlook `mail_backend` providers |
| [`crates/canoa-host/AGENTS.md`](crates/canoa-host/AGENTS.md) | `canoa-host`: the `canoa` program, canoa.cfg's `[host]`, its command line and library |
| [`crates/canoa-ffi/AGENTS.md`](crates/canoa-ffi/AGENTS.md) | `canoa-ffi`: the C ABI that runs the host inside an application |
| [`crates/canoa-launch/AGENTS.md`](crates/canoa-launch/AGENTS.md) | `canoa-launch`: the portable `CanoaUI.exe` and `canoa-pack` |
| [`crates/canoa-office/AGENTS.md`](crates/canoa-office/AGENTS.md) | `canoa-office`: the COM add-in DLL for classic desktop Office, and its per-user registration script |
| [`crates/canoa-claude/AGENTS.md`](crates/canoa-claude/AGENTS.md) | `canoa-claude`: Claude Code's provider re-export, source preset and Store recipe |
| [`crates/canoa-codex/AGENTS.md`](crates/canoa-codex/AGENTS.md) | `canoa-codex`: the same for Codex |
| [`crates/canoa-grok/AGENTS.md`](crates/canoa-grok/AGENTS.md) | `canoa-grok`: Grok's source preset and Store recipe |
| [`crates/canoa-node/AGENTS.md`](crates/canoa-node/AGENTS.md) | `canoa-node`: Node.js as a Store program |
| [`crates/canoa-python/AGENTS.md`](crates/canoa-python/AGENTS.md) | `canoa-python`: CPython as a Store program |
| [`crates/canoa-git-bash/AGENTS.md`](crates/canoa-git-bash/AGENTS.md) | `canoa-git-bash`: Git for Windows' portable Bash as a Store program |
| [`crates/canoa-powershell/AGENTS.md`](crates/canoa-powershell/AGENTS.md) | `canoa-powershell`: PowerShell 7 as a Store program |
| [`crates/canoa-bitwarden/AGENTS.md`](crates/canoa-bitwarden/AGENTS.md) | `canoa-bitwarden`: Bitwarden's command line as a Store program and the `secrets` Store type |
| [`crates/canoa-pi/AGENTS.md`](crates/canoa-pi/AGENTS.md) | `canoa-pi`: Pi's provider re-export, source preset and Store recipe |
| [`crates/canoa-antigravity/AGENTS.md`](crates/canoa-antigravity/AGENTS.md) | `canoa-antigravity`: Antigravity's source preset and Store recipe |
| [`crates/canoa-opencode/AGENTS.md`](crates/canoa-opencode/AGENTS.md) | `canoa-opencode`: OpenCode's source preset and Store recipe |

## Layout

| Path | Responsibility |
| --- | --- |
| `app/` | The Flutter chat client: web, desktop and the extension's UI |
| `app/native/` | The app's own plugin: builds `crates/canoa-ffi` with the desktop runner and declares its C ABI |
| `crates/canoa/` | Registry, service descriptors and deny-by-default authorization, which the host uses |
| `crates/canoa-host/` | The host: serves stdio ACP agents and Guatiao providers from canoa.cfg to paired keys, on loopback, and the web app; `examples/` holds the programs the tests run (`stdio_agent`, also scripted with `--scripted`; `web_peer`, the page's peer; `mcp_server`; `harness_standin`; `store_control`, the control channel over standard streams with a stand-in program as its only Store) |
| `crates/canoa-{claude,codex,grok,pi,antigravity}/` | One agent each: its source preset, its Store recipe and, for Claude, Codex and Pi, the coqui-acp provider re-exported; `canoa-host` depends on them and wraps each recipe as a Store provider |
| `crates/canoa-foundry/` | Foundry Local: Anthropic Messages, OpenAI Responses and chat completions translated to one chat completion, the engines that run it (the person's Foundry Local service, or its Rust SDK) and the `foundry-local` account kind; the host's `gateway` serves it on loopback |
| `crates/canoa-litellm/` | A LiteLLM proxy as an account: one address and one virtual key that provide `openai-api` and `anthropic-api`, made by the proxy's SSO (the browser, the key handed back to a loopback address) or by a key you set, listing the proxy's models and reporting the key's budget and limits; offered through its library like `canoa-foundry`, so `canoa-host` names it in one line of `bundled.rs` |
| `crates/canoa-openai/` | OpenAI's speech for accounts (and its voices as the `speech_say_openai` provider): the speech-to-text an `account_speech` description (the `openai-api` standard at OpenAI's own address, where a recording is posted, the credential that holds the key, the models and their end dates), offered through its library like `canoa-foundry`, so `canoa-host` names it in one line of `bundled.rs` |
| `crates/canoa-piper/` | Piper: its text-to-speech program (the publishers' releases, each file checked against its SHA-256) and two voices as the Store type `piper`, the voices page of that type and the `speech_say_piper` provider that speaks with the installed copy; offered through its library like `canoa-whisper`, so `canoa-host` names it in one line of `bundled.rs` |
| `crates/canoa-whisper/` | whisper.cpp: its speech-to-text program (the publishers' releases, each file checked against its SHA-256) and eight models as the Store type `whisper`, the models page of that type and the speech offer, which says how the installed copy is run; offered through its library like `canoa-foundry`, so `canoa-host` names it in one line of `bundled.rs` |
| `crates/canoa-{node,python,git-bash,powershell,bitwarden}/` | Store libraries for Node.js, CPython, Git Bash, PowerShell 7 and Bitwarden CLI (`bitwarden_extension` adds the `secrets` Store type); each offered through its `library` in `crates/canoa-host/src/bundled.rs`, while `canoa-host` derives runtime, PATH, command and secret-program lists from registered `StoreInfo` and `StoreExtension` descriptions |
| `crates/canoa-microsoft-mail/` | Microsoft mail backends: Exchange Server (`ews`), Microsoft Graph (`graph`), and classic Outlook (`outlook`) as `mail_backend` providers (`canoa::mail_backend`), offered through its library like `canoa-foundry`, so `canoa-host` names it in one line of `bundled.rs` |
| `crates/canoa-ffi/` | The host as a library inside an application, behind a small C ABI; the only crate with `unsafe`, bar five modules of `canoa-host` (`usage_splice/members/win32.rs`, the Windows process table; `replace.rs`'s `native`, `ReplaceFileW`; `awake.rs`'s `win32`, `SetThreadExecutionState`; `spawn/pty.rs`, the child's setup between fork and exec; `terminal/pty_windows.rs`, the pseudo-console calls) and one Windows test module of `canoa-microsoft-mail` (`sspi_server.rs`, test code that plays a server for Windows' security package); `crates/canoa-host/tests/unsafe_modules.rs` fails when one is added without being written down here and in that crate's header |
| `crates/canoa-launch/` | The Windows portable launcher: carries the built app folder, unpacks it once per version, verifies and starts it; `app/tool/portable.py` builds it and the Linux AppImage |
| `crates/canoa-office/` | The COM add-in for classic desktop Office on Windows (Windows-only `cdylib`): the interfaces Office calls, a ribbon button and a task pane, loaded in the Office process; `tools/office_addin.ps1` registers it under HKCU. `unsafe` is confined to its eight COM-glue files, which a test lists |
| `clients/browser/chromium/` | MV3 browser extension code and permission policy |
| `clients/office/` | The Office add-in: manifests for Outlook and for Word, Excel and PowerPoint, rendered for an origin by `render.mjs`; the task pane page that frames the chat-only app for the open item; `canoa-host` carries the pages and serves them with `[host.web] office = true` |
| `bindings/python/canoa-sdk/`, `bindings/js/canoa-sdk/` | The SDKs for writing a provider in Python or JavaScript: one base class per kind, a settings form from a dataclass or `s` schema; their kinds are written from `crates/canoa/include/canoa.*.kinds.json` (`tools/write_kinds.{py,mjs}`, `--check`); their `AGENTS.md` is the API |
| `examples/providers/` | One provider per kind in each language, written with the SDKs; `crates/canoa-host/tests/script_providers.rs` loads them through the host |
| `docs/` | Architecture and interoperability contracts; `docs/roadmap.md` is the one status table |
| `tools/` | Offline dependency mirroring |

## Project identity

- Publisher and primary domain: **Boriken Dev** (`boriken.dev`); GitHub organization `Boriken-Dev`.
- Use `dev.boriken.*` for reverse-DNS identifiers (Android application IDs, Apple bundle IDs, native messaging hosts). Canoa uses `dev.boriken.canoa` as its base identifier.
- Keep ecosystem-native package naming: Rust crates use hyphens (`coqui-transport-web`), Dart packages underscores, and scoped npm packages must use a verified scope. Do not invent a claimed publishing scope.
- Documentation remains at GitHub Pages until DNS is configured for a custom domain. Do not claim hypothetical URLs are live.

## Rules

- Canoa is one implementation of an interoperable concept. No proprietary agent gateway is required.
- Prefer WebTransport over HTTPS, with explicit WSS fallback on connection/path timeout or failure; plain WS is fine where the link is sealed in-band (the paired Noise or WebCrypto channel), and unsealed plain WS stays a loopback-only development exception. Do not retry unacknowledged agent operations automatically.
- Browser clients must authenticate peer identity through an independently implementable versioned wire profile; a claimed peer ID or TLS hostname alone is not proof of pairing.
- The extension owns its permissions for webpages and agents independently of the native host.
- Permissions are scoped by verified peer, client origin, tab/document and capability, and default to deny. Navigation and revocation close the access.
- Agent permissions and browser permissions are distinct: neither can override the other's rejection.
- Peer identity and key continuity should reuse coqui-peer primitives but not require clients to use the Rust crate.
- ACP, MCP, and WebMCP are different protocols/APIs; never assert a webpage's WebMCP tool is an HTTP MCP server.
- Add crates only for genuine public contracts. No forwarding-only crates.

## Commands

```sh
export CANOA_WEB_DIST="$PWD/crates/canoa-host/tests/site"   # --all-features builds canoa's web feature, which needs a site
export CANOA_DEPS=<a checkout of the deps branch>           # the host carries Office.js from its cdn/office-js/; CANOA_OFFICE_JS=cdn instead names Microsoft's address and needs none
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features
CANOA_TEST_PYTHON=python3 cargo test -p canoa-host --test python_source   # a Python with its shared library; ignored without
CANOA_TEST_PYTHON=python3 CANOA_TEST_NODE=node cargo test -p canoa-host --lib profile -- --ignored   # real pip and npm against a folder of wheels and a loopback registry; ignored, with the variable they need as the reason, without them
CANOA_TEST_STDIO_PROVIDER=<libcoqui_acp_harness-stdio-provider> CANOA_TEST_FAKE_STDIO=<fake_stdio_cli> cargo test -p canoa-host --test history_replay -- --ignored   # coqui-acp builds both with `--features kind --examples`
# A test that needs what a machine may not have is `#[ignore = "<what it needs>"]`, never a pass that did not run; `cargo test` reports the count. Run the ones this machine can:
cargo test -p canoa-host --lib -- --ignored a_skill_directory_that_is_a_link a_link_to_a_file_outside a_dot_name_is_refused_by_its_short_name   # Windows: symbolic links (Developer Mode or elevated) and 8.3 short names
cargo test -p canoa-launch --lib -- --ignored a_base_folder_cannot_vouch   # Windows: elevated, for the restore privilege
cargo test -p canoa-host --all-features -- --ignored discover_nearby_reports a_connection_from_another_user   # Linux: passwordless sudo and setpriv
cargo test -p canoa-host --lib -- --ignored operating_systems_store two_homes_with_the_same_account_id   # Linux: a Secret Service session; the other platforms run these without --ignored
(cd app && flutter analyze && flutter test)
(cd app && flutter test integration_test/desktop_test.dart -d windows)   # or -d linux; needs CANOA_TEST_HOST
# the chat's connection code against the real host
cargo build -p canoa-host --bin canoa --examples
(cd app && CANOA_TEST_HOST=../target/debug/canoa flutter test test/host_test.dart)
# the app's reading of `settings/sections`, `accounts/catalog` and `store/models` against a real `canoa serve` whose vendor homes, data folder and PATH are the test's own (needs the examples built)
(cd app && CANOA_TEST_HOST=../target/debug/canoa flutter test test/control_wire_host_test.dart)
# the app's Store code against the real control channel and a stand-in program (needs the examples built)
(cd app && CANOA_TEST_HOST=../target/debug/canoa flutter test test/program_store_host_test.dart)
# the host as a library in a test process, and a standalone canoa on the same home
cargo build -p canoa-ffi --lib
(cd app && CANOA_TEST_LIBRARY=../target/debug/canoa_ffi.dll CANOA_TEST_HOST=../target/debug/canoa flutter test test/host_library_test.dart)   # libcanoa_ffi.so on Linux
# earlier Claude and Codex sessions brought in through the host library, from fixture folders the programs' own variables name (the person's are never read)
(cd app && CANOA_TEST_LIBRARY=../target/debug/canoa_ffi.dll CANOA_TEST_HOST=../target/debug/canoa CLAUDE_CONFIG_DIR=test/fixtures/import_homes/projects_home CODEX_HOME=test/fixtures/import_homes/sessions_home flutter test test/session_import_host_test.dart)
# the built app hosting the scripted agent from its library, with no canoa on disk; needs a runner and the examples built
(cd app && flutter test integration_test/embedded_host_test.dart -d windows)   # or -d linux
# main.dart's own main in the built app, one step a run on a new empty DIR: first (no canoa.cfg), flag, config, hosted
(cd app && CANOA_TEST_LAUNCH_STEP=first CANOA_CONFIG=DIR/canoa.cfg COQUI_AGENTIC_PEER_HOME=DIR/home flutter test integration_test/launch_test.dart -d windows)
(cd clients/browser/chromium && npm test)
(cd clients/office && npm test)
(cd bindings/js/canoa-sdk && npm test && node tools/write_kinds.mjs --check)   # typecheck, unit tests, and whether the kinds are current
(cd bindings/python/canoa-sdk && python tools/write_kinds.py --check)
# the built web app in a real browser: no service worker, every request to its own origin, symbols, emoji and CJK drawn from the fonts `build.py` copies in
(cd clients/browser/chromium && CANOA_E2E_BROWSER=<Edge> CANOA_E2E_HOST=<canoa> CANOA_E2E_AGENT=<stdio_agent> CANOA_E2E_WEB=../../../app/build/web [CANOA_E2E_SHOT=<png>] node e2e/web_fonts.mjs)
# a real voice's audio (Piper's `--output_raw`, 22050 Hz) played by the web app's own Web Audio player in a real browser; CANOA_E2E_TELL_RATE=44100 is the control that must fail
(cd app && dart compile js -O1 -o <dir>/page.js tool/voice_page.dart)
(cd clients/browser/chromium && CANOA_E2E_BROWSER=<Edge> CANOA_E2E_PAGE=<dir> CANOA_E2E_PCM=<voice.pcm> node e2e/voice_pcm.mjs)
# copy and paste in the terminal of the web app, in a real browser: key events through the engine, the browser's paste event with no clipboard permission, Ctrl+Shift+C and Ctrl+C
(cd app && flutter build web --release --no-web-resources-cdn -t tool/terminal_page.dart -o <dir>)
(cd clients/browser/chromium && CANOA_E2E_BROWSER=<Edge> CANOA_E2E_PAGE=<dir> node e2e/terminal_clipboard.mjs)
# providers written with the SDKs, loaded through the host (Python needs its shared library; JavaScript needs a C compiler for QuickJS)
CANOA_TEST_PYTHON=python3 CANOA_TEST_JS=1 cargo test -p canoa-host --test script_providers   # needs `cargo build -p canoa-host --example stdio_agent`
(cd bindings/python/canoa-sdk && GUATIAO_LIBRARY=<guatiao_source_py built by the test> PYTHONPATH=<guatiao>/bindings/python/guatiao/src pytest)
mkdocs build --strict
```

`flutter pub get` with an older Flutter rewrites `app/pubspec.lock`: restore it before committing.

A coding task through the desktop window, its widgets only (fix a failing test, stop and continue, attach, refuse one question,
close and reopen from Earlier conversations), then checked on disk. It needs the examples built (the real-Claude run also needs `node` on PATH) and a runner from `flutter create
--platforms=windows .` (or `linux`) renamed by `sh app/tool/name_runner.sh app`; screenshots go to `CANOA_TEST_SHOTS`, each distinct update's kind and shape to
`CANOA_TEST_UPDATES`:

```sh
(cd app && flutter test integration_test/coding_task_test.dart -d windows)   # a scripted agent, nothing spent
(cd app && CANOA_TEST_TASK_AGENT=Claude flutter test integration_test/coding_task_test.dart -d windows)   # spends the subscription, no user settings
```

`CANOA_EVENT_LOG=<file>` makes the app append the kind, shape and rendering of each distinct ACP update it receives, never its
text. `test/fixtures/coding_task_updates.jsonl` is that record from a real run, plus the v2 kinds it lacked from a scripted run;
`chat_test.dart` fails when any kind in it would reach the fallback, which shows nothing.

## CI and release

Checks run on a developer's own machines, by the commands above. `test.yml`, `flutter.yml`, `browser.yml`, `desktop.yml`,
`offline.yml` and `deps.yml` run only when started by hand. `docs.yml` publishes Pages from `main`.
`release.yml` gates a `v*` tag, running `test.yml`, and publishes no package yet. Pushing a `v*` tag needs the owner's consent for that release.
