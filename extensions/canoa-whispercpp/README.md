# canoa-whispercpp

Canoa's package for whisper.cpp: the Store type that lists its models (each installed on its own, checked against its publisher's
SHA-256) and the speech provider that runs a model inside the host's own process, so dictation runs on the host's processor and
the audio goes nowhere else. There is no program to install.

A build that does not carry it has no whisper.cpp models or offer. Compiling it compiles whisper.cpp: cmake, a C++ compiler and
libclang are needed (see [`AGENTS.md`](AGENTS.md), "Building"). Not published; `canoa-host` carries it. Every export is listed in
[`AGENTS.md`](AGENTS.md).

## Development

```sh
cargo test -p canoa-whispercpp
cargo test -p canoa-whispercpp -- --ignored   # downloads the tiny model and a recording
```

## License

MPL-2.0; see [LICENSE](../../LICENSE).
