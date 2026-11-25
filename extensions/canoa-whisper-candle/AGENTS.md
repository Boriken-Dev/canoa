# `canoa-whisper-candle` — API header

Header-file-style reference for `canoa-whisper-candle`: OpenAI's Whisper written in Rust over Hugging Face's Candle, for a Canoa host (the Store type that lists its models, the models page of that type, and the `speech_provider` that transcribes with a model inside the host's own process), so it can be used without reading its source. It ships inside the package and is self-contained. Development documentation lives with the source at <https://github.com/Boriken-Dev/canoa>.

MSRV Rust 1.89. Version `0.0.0-alpha.0`, not published (`publish = false`). `#![forbid(unsafe_code)]`. Depends on `candle-core`, `candle-nn`, `candle-transformers` 0.11 (the `whisper` module), `tokenizers` 0.23 (its regular expressions in Rust: feature `unstable_wasm`) and `serde_json`; with the default feature `host` also `canoa` (features `store`, `speech`), `coqui-acp-api`, `guatiao`, `guatiao-intake` and `rayon`. There is no C or C++ to build and nothing is fetched at build time.

It is offered beside `canoa-whisper` (whisper.cpp): the person picks. What a host loads is `library` (below); the host names none of it.

## Features

- `host` (default): the Store type, the models page, the provider and `catalog`.
- Without it: [`Engine`] alone. It reads bytes and samples, and starts no thread, reads no file and keeps no clock, so the crate compiles to `wasm32-unknown-unknown` (the web app's module is `app/web_whisper`, which links it that way; its build needs `RUSTFLAGS` `-C target-feature=+simd128` and, for the random generator Candle's dependencies use, `--cfg getrandom_backend="wasm_js"`).
- A debug build on Windows on ARM needs the workspace's `[profile.dev.package.gemm-common]` and `gemm-f16` at `opt-level = 1`: Candle's matrix multiply writes half-precision instructions as inline assembly that assembles only once it is inlined.

## Exports

```rust
pub const LIBRARY: &str = "canoa-whisper-candle";    // the name `library` is registered under
pub const NAME: &str = "Whisper (Candle)";           // what it is called to the person
pub const STORE_TYPE: &str = "whispercandle";        // the id of the Store type, and of its speech offer
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
pub fn library(host: guatiao::library::Host) -> Option<LibraryParts>;   // WhisperCandleExtension, WhisperCandleModels, WhisperCandleSpeech  (host)

pub struct WhisperCandleExtension;  // a `store_extension`, id "whispercandle_extension": the type `whispercandle` ("Whisper (Candle)", icon `chip`, no program store, models `whispercandle_models`)
pub struct WhisperCandleModels;     // a `model_store`, id "whispercandle_models"; `WhisperCandleModels::over(&'static [Model])` lists another set, which a test serves
pub struct WhisperCandleSpeech;     // a `speech_provider`, id "speech_whisper_candle", built `From<WhisperCandleSettings>`
pub struct WhisperCandleSettings { pub model: String, pub models_dir: String, pub language: Option<String>, pub threads: Option<u32> }   // `new(model, models_dir)`
pub fn unload();                    // frees the model held
pub fn is_loaded() -> bool;         // whether a model is held

pub mod catalog {                   // (host)
    pub const ENDPOINTS: [&str; 6];  // where the files are downloaded from: Hugging Face's `openai/` and the hosts it sends a download on to
    pub const CONFIG: &str; pub const TOKENIZER: &str; pub const WEIGHTS: &str; pub const FILE_NAMES: [&str; 3];
    pub struct File { pub name: &'static str, pub bytes: u64, pub sha256: &'static str }
    pub struct Model { pub id, pub label, pub repository, pub revision: &'static str, pub files: [File; 3] }   // `url(&File)`, `bytes()`, `file(name)`
    pub const MODELS: [Model; 4];    // `base` (the default), `tiny`, `small`, `large-v3-turbo`
    pub fn model(id: &str) -> Option<&'static Model>;
}
```

## The models page and the speech offer

The type has no program: the host keeps its files under its own id in the shelf (`<shelf>/whispercandle/models`). Each model is a folder `<id>/` of three files: `openai/whisper-<id>`'s `config.json`, `tokenizer.json` and `model.safetensors` at one pinned revision, each checked against the SHA-256 listed for it. A model is installed when all three are there; its row offers `fetch_files` until then and `delete_files` once they are. `act` and `keep` refuse `UNSUPPORTED`. `describe().speech` is `SpeechService::in_process(.., "speech_whisper_candle")`; `service` lists the installed models and answers `settings` for the model asked for, else the first installed, or `unavailable` in its own words.

## The provider

`WhisperCandleSpeech` takes `pcm_s16le` at any rate and channel count (averaged to mono and resampled to 16 kHz by linear interpolation), keeps the recording until `finish`, and runs the engine on it with the language hint's primary subtag (`es-PR` is `es`; else the setting `language`; else the model's detection), the vocabulary hint joined with `, ` as the initial prompt, and no timestamps. A language the model does not know is detected, not refused. The transcript's front matter holds `language` and `duration_ms`. Decoding is greedy, with OpenAI's rules for suppressed tokens and for silence (no-speech probability above 0.6 and average log-probability below -1 gives no text); there is no beam search, temperature fallback or voice-activity detection. `threads` is the processors, up to eight, unless set; a rayon pool of that size runs the model.

`open` refuses a model whose three files are not in `<models_dir>/<model>/` (`the model X is not installed: ...`) and starts loading it on a thread. The loaded model is kept for the next recording and freed when another is asked for, after `IDLE`, or by `unload()`. Weights are read as 32-bit numbers, so a model takes about twice its size in memory (`large-v3-turbo` about 3.3 GB).

## Gotchas

- The mel spectrogram is this crate's own (`mel.rs`: Slaney filters computed, a mixed-radix FFT), single-threaded: Candle's own one spawns threads, which WebAssembly lacks.
- A clip is read at its real length, not padded to 30 seconds.
- The tests that run a model (`tests/speech.rs`) download `tiny` and a recording with `curl` into a temporary folder, check each file's SHA-256, and are ignored without the network: `cargo test -p canoa-whisper-candle -- --ignored` (use `--release`: Candle is slow unoptimized).
