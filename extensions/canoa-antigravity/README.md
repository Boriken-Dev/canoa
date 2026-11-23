# canoa-antigravity

Canoa's package for Antigravity: the Store recipe that finds, checks for updates and installs it, and the source preset the host
starts it from when `canoa.cfg` does not name it. Not published; `canoa-host` depends on it.

Every export is listed in [`AGENTS.md`](AGENTS.md).

## Development

```sh
cargo test -p canoa-antigravity --all-features
```

## License

MPL-2.0; see [LICENSE](../../LICENSE).
