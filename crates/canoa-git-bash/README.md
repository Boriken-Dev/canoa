# canoa-git-bash

Git for Windows' portable release as a Store program, `git_bash_store` (Windows only): `bin/bash.exe`, `sh.exe` and `git.exe` kept as `git-bash`, each file checked against the SHA-256 GitHub lists for it.

Not published; `canoa-host` depends on it and offers its library from `bundled.rs`.

Every export is listed in [`AGENTS.md`](AGENTS.md).

## Development

```sh
cargo test -p canoa-git-bash
```

## License

MPL-2.0; see [LICENSE](../../LICENSE).
