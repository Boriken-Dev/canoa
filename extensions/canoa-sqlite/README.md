# canoa-sqlite

Canoa's default history store: the conversations a host records, the list of them each client shows, and the properties of every
turn, kept in one SQLite database file (`sessions.sqlite3` in the host's home folder unless the settings name another). SQLite is
built into the binary, so there is nothing to install.

A build that does not carry it falls back to the in-memory store, which forgets everything when the host stops. Not published;
`canoa-host` carries it. Every export is listed in [`AGENTS.md`](AGENTS.md).

## Development

```sh
cargo test -p canoa-sqlite
```

## License

MPL-2.0; see [LICENSE](../../LICENSE).
