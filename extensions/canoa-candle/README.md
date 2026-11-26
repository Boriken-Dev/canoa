# canoa-candle

Canoa's package for Whisper written in Rust (Hugging Face's Candle): the Store type that lists its models (each installed on its own,
every file checked against its publisher's SHA-256) and the speech provider that runs a model inside the host's own process, so
dictation runs on the host's processor and the audio goes nowhere else. There is no program to install, and no C or C++ to build.

It is offered beside `canoa-whispercpp` (whisper.cpp): the person picks. Without its `host` feature the crate is the engine alone, which
compiles to WebAssembly. Not published; `canoa-host` carries it. Every export is listed in [`AGENTS.md`](AGENTS.md).

## Development

```sh
cargo test -p canoa-candle
cargo test -p canoa-candle -- --ignored   # downloads the tiny model and a recording
```

## License

MPL-2.0; see [LICENSE](../../LICENSE).
