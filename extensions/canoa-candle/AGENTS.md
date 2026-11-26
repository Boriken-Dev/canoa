# `canoa-candle` — API header

Header-file-style reference for `canoa-candle`: OpenAI's Whisper written in Rust over Hugging Face's Candle, for a Canoa host (the engine, and the `speech_provider` that transcribes with a model inside the host's own process), so it can be used without reading its source. It ships inside the package and is self-contained. Development documentation lives with the source at <https://github.com/Boriken-Dev/canoa>.

MSRV Rust 1.89. Version `0.0.0-alpha.0`, not published (`publish = false`). `#![forbid(unsafe_code)]`. Depends on `candle-core`, `candle-nn`, `candle-transformers` 0.11 (the `whisper` module), `tokenizers` 0.23 (its regular expressions in Rust: feature `unstable_wasm`) and `serde_json`; with the default feature `host` also `canoa` (features `store`, `speech`), `coqui-acp-api`, `guatiao`, `guatiao-intake` and `rayon`. There is no C or C++ to build and nothing is fetched at build time.

It is offered beside `canoa-whispercpp` (whisper.cpp): the person picks. What a host loads is `library` (below); the host names none of it. The models are the Whisper type's (`canoa-whisper`, whose `catalog` reads Hugging Face's listing of them, also in the web app); this crate has no Store type, catalog or model list of its own.

## Features

- `host` (default): the speech provider.
- Without it: [`Engine`] and the names of a model's files alone. It reads bytes and samples, and starts no thread, reads no file and keeps no clock, so the crate compiles to `wasm32-unknown-unknown` (the web app's module is `app/web_whisper`, which links it that way; its build needs `RUSTFLAGS` `-C target-feature=+simd128` and, for the random generator Candle's dependencies use, `--cfg getrandom_backend="wasm_js"`).
- A debug build on Windows on ARM needs the workspace's `[profile.dev.package.gemm-common]` and `gemm-f16` at `opt-level = 1`: Candle's matrix multiply writes half-precision instructions as inline assembly that assembles only once it is inlined.

## Exports

```rust
pub const LIBRARY: &str = "canoa-candle";    // the name `library` is registered under
pub const CONFIG: &str = "config.json";      // the names of the files of a model's folder that `Engine::load` reads
pub const TOKENIZER: &str = "tokenizer.json";
pub const WEIGHTS: &str = "model.safetensors";
pub const FILE_NAMES: [&str; 3] = [CONFIG, TOKENIZER, WEIGHTS];
pub const PROVIDER: &str = "speech_whisper_candle";  // the id of the speech provider (host)
pub const IDLE: Duration;                            // 5 minutes: how long a loaded model is kept without a recording (host)

/// Whisper over bytes.
pub struct Engine;
impl Engine {
    /// `config.json`, `tokenizer.json` and the `model.safetensors` weights (any float type, read as f32).
    pub fn load(config: &[u8], tokenizer: &[u8], weights: Vec<u8>) -> Result<Engine, Error>;
    pub fn knows(&self, code: &str) -> bool;          // a language code the tokenizer has (`es`)
    /// 16 kHz mono samples in -1..1. `language`: a code `knows` accepts, or None to detect it on the first window.
    /// `prompt`: the initial prompt (the person's vocabulary), empty for none. Clips over 30 s are read a window at a time.
    pub fn transcribe(&mut self, samples: &[f32], language: Option<&str>, prompt: &str) -> Result<Transcription, Error>;
}
pub struct Transcription { pub text: String, pub language: Option<String> }
pub struct Error;                                     // Display, std::error::Error

/// What a host links: handed to `Registry::register_local`.
pub fn library(host: guatiao::library::Host) -> Option<LibraryParts>;   // WhisperCandleSpeech  (host)

pub struct WhisperCandleSpeech;     // a `speech_provider`, id "speech_whisper_candle", built `From<WhisperCandleSettings>`
pub struct WhisperCandleSettings { pub model: String, pub models_dir: String, pub language: Option<String>, pub threads: Option<u32> }   // `new(model, models_dir)`
pub fn unload();                    // frees the model held
pub fn is_loaded() -> bool;         // whether a model is held

```

## The models

The crate lists and downloads nothing: a model is a folder `<models_dir>/<id>/` of the three files, which the Whisper type (`canoa-whisper`) installs, and whose speech offer names `speech_whisper_candle` and the settings (`model`, `models_dir`) for a model whose three files are there.

## The provider

`WhisperCandleSpeech` takes `pcm_s16le` at any rate and channel count (averaged to mono and resampled to 16 kHz by linear interpolation), keeps the recording until `finish`, and runs the engine on it with the language hint's primary subtag (`es-PR` is `es`; else the setting `language`; else the model's detection), the vocabulary hint joined with `, ` as the initial prompt, and no timestamps. A language the model does not know is detected, not refused. The transcript's front matter holds `language` and `duration_ms`. Decoding is greedy, with OpenAI's rules for suppressed tokens and for silence (no-speech probability above 0.6 and average log-probability below -1 gives no text); there is no beam search, temperature fallback or voice-activity detection. `threads` is the processors, up to eight, unless set; a rayon pool of that size runs the model.

`open` refuses a model whose three files are not in `<models_dir>/<model>/` (`the model X is not installed: ...`) and starts loading it on a thread. The loaded model is kept for the next recording and freed when another is asked for, after `IDLE`, or by `unload()`. Weights are read as 32-bit numbers, so a model takes about twice its size in memory (`large-v3-turbo` about 3.3 GB).

## Gotchas

- The mel spectrogram is this crate's own (`mel.rs`: Slaney filters computed, a mixed-radix FFT), single-threaded: Candle's own one spawns threads, which WebAssembly lacks.
- A clip is read at its real length, not padded to 30 seconds.
- The tests that run a model (`tests/speech.rs`) download `openai/whisper-tiny`'s three files and a recording with `curl` into a temporary folder and are ignored without the network: `cargo test -p canoa-candle -- --ignored` (use `--release`: Candle is slow unoptimized).
