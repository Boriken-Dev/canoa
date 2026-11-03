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

### Changed

- `chrome.storage.local` answers only the extension's own pages, so a content script can no longer read the agents' secret.
- The transport and the ACP client moved out: `coqui-web` and `coqui-acp` replace `crates/coqui-net-transport`, `bindings/dart/coqui_net_transport`, `app/lib/transport` and most of `app/lib/acp`. Canoa now also connects to ACP v1 agents.

### Fixed

- A failed WebSocket or WebTransport connection now releases its socket. Before, `close()` did nothing once the stream had ended, so a peer that sent a frame over the limit left the socket open.

### Changed

- Text files are LF on every platform (`* text=auto eol=lf`). A Windows checkout made
  before this needs `git add --renormalize .` or a fresh clone.
