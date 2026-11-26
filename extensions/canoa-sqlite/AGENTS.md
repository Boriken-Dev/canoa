# `canoa-sqlite` — API header

Header-file-style reference for `canoa-sqlite`: the `history_store` provider that keeps a Canoa host's recorded conversations, the list of them each client shows and the properties of every turn in one SQLite database file, so it can be used without reading its source. It ships inside the package and is self-contained. Development documentation lives with the source at <https://github.com/Boriken-Dev/canoa>.

MSRV Rust 1.89. Version `0.0.0-alpha.0`, not published (`publish = false`). `#![forbid(unsafe_code)]`. Depends on `canoa` (feature `history-store`), `coqui-acp-api`, `guatiao`, `guatiao-intake`, `serde_json` and `rusqlite` (feature `bundled`: SQLite is compiled into the binary; no other crate of the workspace names `rusqlite`).

## Exports

```rust
pub const LIBRARY: &str = "canoa-sqlite";   // the name `library` is registered under
pub const ID: &str = "sqlite";              // the provider's id, as canoa.cfg names it
pub const FILE: &str = "sessions.sqlite3";  // the file in the host's home folder when the settings name none

pub fn library(host: guatiao::library::Host) -> Option<LibraryParts>;   // one provider, SqliteStore

pub struct SqliteStore;   // the `history_store` provider: default, read-write, capabilities `sql` (dialect sqlite), `blobs`, `compact`
pub struct SqliteDb;      // an opened store (`HistoryDb`); `SqliteDb::open(&Options)`
pub fn options(request: &OpenRequest) -> Result<Options, ProviderError>;   // the settings read, defaults applied
pub struct Options { path, busy_timeout_ms, journal: Journal, synchronous: Synchronous, read_only, create, reply_bytes }
```

## Settings

A JSON object, every key optional, no other key accepted; they live in `canoa.cfg` only.

| Key | Default | Meaning |
| --- | --- | --- |
| `path` | `<home>/sessions.sqlite3` | the file; a relative path is under the host's home folder |
| `busy_timeout_ms` | 5000 | how long a writer waits for another before `Busy` |
| `journal` | `wal` | `wal` or `delete` |
| `synchronous` | `normal` | `off`, `normal` or `full` |
| `read_only` | false | open without the right to change it; every write and migration answers `ReadOnly` |

## Behaviour

- A new file is created owner-only on Unix; on Windows the home folder's permissions protect it. Nothing is encrypted.
- `migrate` runs its typed steps in one transaction and records each table's description in `canoa_tables` and the schema version in `PRAGMA user_version`; `schema()` answers from there, so `bool`, `json`, keys and `requires` survive a reopen. A table is `PRIMARY KEY (key columns)`; a `bool` is an integer, `json` text.
- `write` is one `BEGIN IMMEDIATE` transaction: every change or none. A key or unique-index duplicate is `Conflict`; `upsert` is `ON CONFLICT (key) DO UPDATE`. `{add: n}` is `col = COALESCE(col, 0) + n`.
- Every value is bound; names are checked against the schema and quoted. A text test (`prefix`, `contains`, `suffix`) is literal and case-sensitive.
- `select` is keyset-paged (the key's columns end the order); nulls sort lowest. A reply ends at `limit` rows or `reply_bytes`.
- `sql` runs one `SELECT` or `WITH` statement on a second, read-only connection (`query_only`); anything that would change the database, or a second statement, is `Bad`.
- `revision()` moves when any connection changes data or schema. `compact()` checkpoints the log and runs `VACUUM`.
- Not yet: the `search` capability (full-text over `events.text`).
