# canoa-python

CPython as a Store program: `python_store`, the standalone builds of the python-build-standalone releases (channels `3.11` to `3.14`), each file checked against the `SHA256SUMS` of the newest release and unpacked as `python`.

Not published; `canoa-host` depends on it and offers its library from `bundled.rs`.

Every export is listed in [`AGENTS.md`](AGENTS.md).

## Development

```sh
cargo test -p canoa-python
```

## License

MPL-2.0; see [LICENSE](../../LICENSE).
