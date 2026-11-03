# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- An MCP client outside any ACP session, such as an IDE or a CLI agent, can ask for a browser tab: a person turns this on for one paired machine under **Canoa browser permissions**, and `coqui-acp-bridge mcp-config` prints one stable entry for it. Off by default, and out of a page's reach.
- `canoa-host`, a new crate and command: serves each `[host]` entry of Canoa.cfg, a stdio ACP agent or a Guatiao agent provider, to keys you paired, on loopback only. `peers`, `accept` and `revoke` manage the keys; a removed key loses its link within about a second. A client cannot have it start a program: a stdio MCP server in a request and `_meta` never reach a stdio agent.
- The chat dials `ws://` on loopback, where the host listens. Everything after the peer proof is sealed, so the transport carries nothing it could read.
- The app keeps Canoa.cfg's `[host]` table when it saves its settings.
- `canoa-host serve --until-stdin-closes`: the host ends every agent it started, then itself, when its input closes, so it ends with the program that runs it. `Serving::stopper` and `Stopper::stop_agents` do the same from the library.
- The desktop app runs `canoa-host` as its child: the one `CANOA_HOST` names, else the one beside its executable, which the desktop build now puts there. It is started again after an unplanned end, with a pause that doubles up to a minute, and ended with its agents on quit or when the app dies.
- **Hosting**, in the side rail and the tray menu: the host's state and id, its agents, adding one by its command, the keys that may connect, pairing or removing one only from a dialog that shows its whole id, and a PIN for a native program.
- The tray menu opens Hosting, a double click on the tray icon opens the chat, and where Linux shows no tray icons, closing the window minimizes it rather than hiding it out of reach. Settings has **Quit Canoa**.
- `canoa-host pair`: shows a PIN that a native client uses to pair, such as `coqui-acp-bridge pair-dial`. One wrong try ends it.

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
