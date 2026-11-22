# canoa-powershell

PowerShell 7 as a Store program, `powershell_store` (Windows and Linux): the zip or `tar.gz` of the newest release that is no prerelease, kept as `powershell` with `pwsh` on the `PATH`, checked against the SHA-256 GitHub lists for the file.

Not published; `canoa-host` depends on it and offers its library from `bundled.rs`.

Every export is listed in [`AGENTS.md`](AGENTS.md).

## Development

```sh
cargo test -p canoa-powershell
```

## License

MPL-2.0; see [LICENSE](../../LICENSE).
