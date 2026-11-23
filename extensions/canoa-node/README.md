# canoa-node

Node.js as a Store program: `node_store`, nodejs.org's own builds (channels `lts` and `current`), each file checked against the `SHASUMS256.txt` of its release and unpacked into the Store's folder as `node`.

Not published; `canoa-host` depends on it and offers its library from `bundled.rs`.

Every export is listed in [`AGENTS.md`](AGENTS.md).

## Development

```sh
cargo test -p canoa-node
```

## License

MPL-2.0; see [LICENSE](../../LICENSE).
