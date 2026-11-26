# canoa-whisper

Canoa's package for OpenAI's Whisper as one Store type. The models are read from Hugging Face's own listing (no model is named in the
crate), each is downloaded once as the Hugging Face folder and checked against the digests the listing states, and both engines run
it: Candle (pure Rust) reads the folder as it is, and whisper.cpp reads a GGML file the install writes from it, in the form the person
picks. Each model's tile shows an engine pill for each way it runs.

Not published; `canoa-host` carries it. The `whispercpp` feature (on by default) is the second engine and needs a C++ toolchain, cmake
and libclang; without it the models run on Candle alone. Every export is listed in [`AGENTS.md`](AGENTS.md).

## Development

```sh
cargo test -p canoa-whisper
cargo clippy -p canoa-whisper --all-targets -- -D warnings
```

The tests read recorded Hugging Face listings from `tests/fixtures/` and install a model of a few kilobytes made on the spot.
