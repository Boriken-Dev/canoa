# canoa-bitwarden

Bitwarden's command line (`bw`) as a Store program, `bitwarden_store`, and the Store type `secrets` it belongs to (`BitwardenExtension`): the `cli-vYYYY.M.P` release of the clients repository whose zip lists a digest, checked against that digest. Nothing here reads a vault: the Bitwarden secret provider runs `bw`, which is on no search path, only found by name by the spawner a secret provider is handed.

Not published; `canoa-host` depends on it and offers its library from `bundled.rs`.

Every export is listed in [`AGENTS.md`](AGENTS.md).

## Development

```sh
cargo test -p canoa-bitwarden
```

## License

MPL-2.0; see [LICENSE](../../LICENSE).
