# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Fixed

- A failed WebSocket or WebTransport connection now releases its socket. Before, `close()` did nothing once the stream had ended, so a peer that sent a frame over the limit left the socket open.

### Changed

- Text files are LF on every platform (`* text=auto eol=lf`). A Windows checkout made
  before this needs `git add --renormalize .` or a fresh clone.
