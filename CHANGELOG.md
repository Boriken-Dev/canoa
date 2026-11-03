# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- `canoa-host serve --key-limit` and `--no-rekey`, and `ServeOptions::with_key_limit` and `with_rekey`; a setting looser than the default is logged. A client of the other method is logged as a method mismatch, never as a key to accept. A home records the method named last, so a host the app runs naming none keeps working after a move.
- Hosting shows the method beside the host's id, and the app's and the extension's add-by-hand forms ask for the agent's method instead of assuming one.
- The extension reports a link that ends without the agent's close record as cut, sizes its frame limit for the longest record a Noise sender can make, refuses a known key named under another method, shows `mcp-config` with its key for the chosen machine's method, and says the `pair-accept --origin` its own origin needs.

- The extension proves an agent by the method stored with its key, Noise or WebCrypto, and pairs with a machine by the PIN it shows under Noise (**Pair with a code**). Each method has a key of its own. An agent stored before methods is WebCrypto's; a browser whose WebCrypto has no X25519 shows a Noise agent as unreachable, and says why, rather than trying another method. Discovered agents carry the method their record names.
- The app's web build proves an agent by the method named with its key in the connect dialog, Noise or WebCrypto, and pairs by the PIN a `coqui-acp-bridge pair-accept --method noise --origin ORIGIN` shows, the dialog naming this page's origin. An agent typed in by hand has no method until the person picks one; one named by a record or a host comes prefilled, and a PIN pairing fills in Noise. Each method has a key of its own, and the dialog shows this browser's id under the method chosen. A method this browser cannot run is shown with the reason, and connecting by it is refused before anything is dialled; no other method is tried. `coqui_web` moves to the revision whose `StagedChannel` gives a setup message as bytes.
- `app/web/vendor` holds `coqui-peer-noise` beside `coqui-peer-webcrypto`, each pinned by blob in `COQUI-SOURCE.json`, now one entry per file.

- `canoa-host` proves by Noise or by WebCrypto, the method of this user's identity: `serve`, `id` and `pair` take `--method noise|webcrypto`, and `serve` and `pair` print the method beside the peer id. A home keeps each method's identity in a file of its own and keeps the method it has; one with both has to name one. A client of the other method is refused, and the log says why.

- An MCP client outside any ACP session, such as an IDE or a CLI agent, can ask for a browser tab: a person turns this on for one paired machine under **Canoa browser permissions**, and `coqui-acp-bridge mcp-config` prints one stable entry for it. Off by default, and out of a page's reach.
- `canoa-host`, a new crate and command: serves each `[host]` entry of Canoa.cfg, a stdio ACP agent or a Guatiao agent provider, to keys you paired, on loopback only. `peers`, `accept` and `revoke` manage the keys; a removed key loses its link within about a second. A client cannot have it start a program: a stdio MCP server in a request and `_meta` never reach a stdio agent.
- The chat dials `ws://` on loopback, where the host listens. Everything after the peer proof is sealed, so the transport carries nothing it could read.
- The app keeps Canoa.cfg's `[host]` table when it saves its settings.
- `canoa-host serve --until-stdin-closes`: the host ends every agent it started, then itself, when its input closes, so it ends with the program that runs it. `Serving::stopper` and `Stopper::stop_agents` do the same from the library.
- The desktop app runs `canoa-host` as its child: the one `CANOA_HOST` names, else the one beside its executable, which the desktop build now puts there. It is started again after an unplanned end, with a pause that doubles up to a minute, and ended with its agents on quit or when the app dies.
- **Hosting**, in the side rail and the tray menu: the host's state and id, its agents, adding one by its command, the keys that may connect, pairing or removing one only from a dialog that shows its whole id, and a PIN for a native program.
- The tray menu opens Hosting, a double click on the tray icon opens the chat, and where Linux shows no tray icons, closing the window minimizes it rather than hiding it out of reach. Settings has **Quit Canoa**.
- `canoa-host pair`: shows a PIN that a native client uses to pair, such as `coqui-acp-bridge pair-dial`. One wrong try ends it.
- A `canoa-host` tool entry, `[host.tools.NAME]`, serves a command-line tool that has no ACP as an ACP agent, with no code for that tool: Canoa.cfg says how it keeps a conversation (one process per session, or one per message), how a prompt goes in, and how each JSON line it writes becomes text, thinking, a tool call, the end of a turn, or a question to the person, answered back to the tool only when they allow. Every session opens by saying what the tool does without asking, in the entry's own words, which an entry must give, and whether it remembers earlier messages; the app shows these in the conversation. `examples/config/pi.cfg` serves Pi through its RPC mode, with Pi's permission-gate extension's questions reaching the person.
- The connect dialog offers **Pi** through its ACP adapter, `pi-acp` 0.0.34, when `pi` is installed.
- A `canoa-host` provider entry can name a Guatiao source instead of a library, with `[host.providers.NAME.source_settings]`: a provider written in Python, Lua, JavaScript, Rhai or WebAssembly is served from one entry. `[host.providers.NAME.settings]` gives a provider the configuration it declares, checked against its schema. Settings reach the provider's process on its standard input, never its command line, and the process runs in Canoa.cfg's directory. The host and a provider's process speak ACP over a loopback socket of their own, so what a provider prints goes to the host's log, 1 MiB per connection at most, and never to the client. `Stopper::stop_agents` and `--until-stdin-closes` end provider processes too.

### Changed

- `chrome.storage.local` answers only the extension's own pages, so a content script can no longer read the agents' secret.
- The transport and the ACP client moved out: `coqui-web` and `coqui-acp` replace `crates/coqui-net-transport`, `bindings/dart/coqui_net_transport`, `app/lib/transport` and most of `app/lib/acp`. Canoa now also connects to ACP v1 agents.

### Fixed

- Settings opens from the side rail: its dialog, like Hosting's, now opens on the app's own navigator.
- A failed WebSocket or WebTransport connection now releases its socket. Before, `close()` did nothing once the stream had ended, so a peer that sent a frame over the limit left the socket open.

### Changed

- The app builds on `coqui_acp` `d64ea1a`. A turn the agent ends as cancelled shows "Stopped." and ends; a permission request the app cannot read is answered cancelled and said in the conversation, while the turn goes on; output a bridge dropped while nobody was connected is said as a notice; and a client takes only the traffic of the sessions it opened.
- `canoa-host` builds on coqui-acp `d64ea1a`, whose bridge stops an agent with everything it started: by job object on Windows, by process group on Unix.
- Text files are LF on every platform (`* text=auto eol=lf`). A Windows checkout made
  before this needs `git add --renormalize .` or a fresh clone.
