# canoa-whispercpp

Canoa's package for whisper.cpp: the Store type that lists its models (each installed on its own, checked against its publisher's
SHA-256) and the speech provider that runs a model inside the host's own process, so dictation runs on the host's processor and
the audio goes nowhere else. There is no program to install.

A build that does not carry it has no whisper.cpp models or offer. Compiling it compiles whisper.cpp: cmake, a C++ compiler and
libclang are needed (see [`AGENTS.md`](AGENTS.md), "Building"). Not published; `canoa-host` carries it. Every export is listed in
[`AGENTS.md`](AGENTS.md).

`convert` writes whisper.cpp's GGML file (f16, or q5_0, q5_1 or q8_0) from a Hugging Face Whisper folder in pure Rust, so one download of the folder can serve whisper.cpp and Candle's engine. It memory-maps the weights, and its quantizer is the ggml this crate links.

## Development

```sh
cargo test -p canoa-whispercpp
cargo test -p canoa-whispercpp -- --ignored   # downloads the tiny model, its Hugging Face folder and a recording
```

## License

MPL-2.0; see [LICENSE](../../LICENSE).
