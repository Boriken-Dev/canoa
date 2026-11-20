# CDN files

Files the builds would otherwise take from a CDN or a repository, mirrored here so no build or run needs the network for them.
`manifest.json` records what is here; `tools/mirror_deps.py` fills this folder, refuses a file whose digest differs from the one
pinned on the source branch, and is the only thing that writes it. No build fetches from a CDN.

| Folder | What | Read by |
| --- | --- | --- |
| `fonts.gstatic.com/s/` | The fonts Flutter's web engine falls back to for a glyph Roboto lacks (symbols, emoji, CJK, every script), pinned by `tools/cdn_files.json` and `tools/fallback_fonts.sha256` | `app/tool/build.py`, which copies them into the web build |
| `office-js/` | The whole tree of one commit of github.com/OfficeDev/office-js (its commit and every file's SHA-256 are in `manifest.json`), unmodified, with its `LICENSE.md`. Nothing is renamed, filtered or rewritten, and nothing is trimmed to what compiles: the owner asked for the library as published, which Microsoft's license terms require of a copy | `crates/canoa-host/build.rs`, which carries the files the add-in's pane loads, and `LICENSE.md`, in the host |

Flutter's CanvasKit is not here: the web build bundles it (`--no-web-resources-cdn`).
