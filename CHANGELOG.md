# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- An MCP client outside any ACP session, such as an IDE or a CLI agent, can ask for a browser tab: a person turns this on for one paired machine under **Canoa browser permissions**, and `coqui-acp-bridge mcp-config` prints one stable entry for it. Off by default, and out of a page's reach.

### Changed

- `chrome.storage.local` answers only the extension's own pages, so a content script can no longer read the agents' secret.
- The transport and the ACP client moved out: `coqui-web` and `coqui-acp` replace `crates/coqui-net-transport`, `bindings/dart/coqui_net_transport`, `app/lib/transport` and most of `app/lib/acp`. Canoa now also connects to ACP v1 agents.

### Fixed

- A failed WebSocket or WebTransport connection now releases its socket. Before, `close()` did nothing once the stream had ended, so a peer that sent a frame over the limit left the socket open.

### Changed

- Text files are LF on every platform (`* text=auto eol=lf`). A Windows checkout made
  before this needs `git add --renormalize .` or a fresh clone.
