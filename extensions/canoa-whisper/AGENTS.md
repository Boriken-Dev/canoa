# `canoa-whisper` — API header

Header-file-style reference for `canoa-whisper`: whisper.cpp for a Canoa host (the Store type that lists its models, the models page of that type, and the `speech_provider` that transcribes with a model inside the host's own process), so it can be used without reading its source. It ships inside the package and is self-contained. Development documentation lives with the source at <https://github.com/Boriken-Dev/canoa>.

MSRV Rust 1.89. Version `0.0.0-alpha.0`, not published (`publish = false`). `#![forbid(unsafe_code)]`. Depends on `canoa` (features `store`, `speech`), `coqui-acp-api`, `guatiao`, `guatiao-intake` and `whisper-rs` 0.16.0, whose `whisper-rs-sys` is replaced (`[patch.crates-io]`) by the vendored copy in `third_party/whisper-rs-sys`: the same crate with whisper.cpp v1.9.6 (release tarball, commit 96ec4d9823e9d0211944172da35f9f29134af990, SHA-256 7ab15dc88022d2425ad94dcffd8abbf1b6d7018089b4e48d6b40780738e0b445) in place of the 1.8.3 it carries, reduced to the CPU and BLAS sources (no models, tests, examples, GPU backends or pre-generated bindings; its build script reads 1.9's `WHISPER_VERSION_*` lines). whisper-rs 0.16's safe API compiles against 1.9.6 unchanged. cmake compiles the sources with the crate.

What a host loads is `library` (below). The host names none of it: it carries the library as it carries any other, lists the type and its models through the generic Store calls and makes the speech offer from what `WhisperModels::service` answers.

## Building

Compiling the crate compiles whisper.cpp (C and C++), so a build needs a C++ toolchain, **cmake** and **libclang** (bindgen). Nothing is fetched: the sources are in the crate, and `deps` carries them. OpenMP is not used (measured: no gain on Windows on ARM or Linux x86-64, and it would need a runtime beside the program).

- **Linux**: `g++`, `cmake`, `libclang` (Debian: `build-essential cmake libclang-dev`). The container's `host` stage installs them.
- **Windows on ARM**: ggml refuses `cl.exe`, so the build uses LLVM's `clang-cl` with Ninja. `.cargo/config.toml` names them for the `aarch64-pc-windows-msvc` target alone (`CC`, `CXX`, `CXXFLAGS=-EHsc`, `CMAKE_GENERATOR=Ninja`, and a `CFLAGS` that keeps clang 22 from stopping on libffi's locks); `clang-cl` (LLVM for Windows) and `ninja` have to be on `PATH`. Visual Studio's own is in `Common7/IDE/CommonExtensions/Microsoft/CMake/Ninja`. libclang is LLVM's `libclang.dll`.
- **Windows x64**: Visual Studio's C++ tools and cmake; untested.

## Exports

```rust
pub const LIBRARY: &str = "canoa-whisper";   // the name `library` is registered under
pub const NAME: &str = "whisper.cpp";        // what it is called to the person: its rows, facts and speech offer
pub const STORE_TYPE: &str = "whisper";      // the id of the Store type, and of its speech offer
pub const PROVIDER: &str = "speech_whisper"; // the id of the speech provider
pub const IDLE: Duration;                    // 5 minutes: how long a loaded model is kept without a recording

/// What a host links: handed to `Registry::register_local`.
pub fn library(host: guatiao::library::Host) -> Option<LibraryParts>;   // WhisperExtension, WhisperModels, WhisperSpeech

pub struct WhisperExtension;   // a `store_extension`, id "whisper_extension": the type `whisper` ("whisper.cpp", icon `chip`, no program store, models `whisper_models`)
pub struct WhisperModels;      // a `model_store`, id "whisper_models"; `WhisperModels::over(&'static [Model])` lists another set of models, which a test serves
pub struct WhisperSpeech;      // a `speech_provider`, id "speech_whisper", built `From<WhisperSettings>`
pub struct WhisperSettings { pub model: String, pub models_dir: String, pub threads: Option<u32>, pub beam_size: u32, pub flash_attention: bool }   // `WhisperSettings::new(model, models_dir)`; beam_size defaults to 5, flash_attention to true
pub fn unload();               // frees the model held
pub fn is_loaded() -> bool;    // whether a model is held

pub mod catalog {
    pub const MODELS_ORIGIN: &str;      // the publisher's Hugging Face folder at the revision the digests were read from
    pub const ENDPOINTS: [&str; 6];     // where the files are downloaded from: that folder and the hosts Hugging Face sends a download on to
    pub struct Model { pub id: &'static str, pub label: &'static str, pub file: &'static str, pub bytes: u64, pub sha256: &'static str }
    pub const MODELS: [Model; 8];       // the default (`base`) first
    pub fn model(id: &str) -> Option<&'static Model>;
}
```

## The models page and the speech offer

The type has no program: `WhisperExtension` names no store, and the host keeps the type's files under its own id in the shelf (`<shelf>/whisper/models`). `WhisperModels::list` answers the eight models (`base`, `base.en`, `tiny`, `tiny.en`, `small-q5_1`, `small`, `small.en`, `large-v3-turbo-q5_0`: the `ggml-*.bin` files of the publisher's Hugging Face repository at one pinned revision, checked against the SHA-256 that revision lists) as rows (runtime, size, file and digest as facts; `speech-to-text` and `English only` or `Every language` as the tags), each with its file. A model is its own install: its row offers `fetch_files` while it is not in the host's folder (`query.folder`) and `delete_files` once it is, both performed by the host from the origins `describe().endpoints` names. `act` and `keep` refuse `UNSUPPORTED`.

`describe().speech` is `SpeechService::in_process(.., "speech_whisper")`. `service` lists, with `query.models`, the models installed in the folder (the default first) and answers `settings`: the `speech_whisper` settings for `query.model` when that is installed, else for the first that is (`model`, `models_dir`); with no folder or no model it answers `unavailable` in its own words. The host writes the person's provider from those settings.

## The provider

`WhisperSpeech` takes `pcm_s16le` at any rate and channel count (averaged to mono and resampled to 16 kHz by linear interpolation), keeps the recording until `finish`, and runs whisper.cpp on it with the language hint's primary subtag (`es-PR` is `es`) or its own detection, the vocabulary hint joined with `, ` as the initial prompt, and no timestamps. The transcript's front matter holds `language` (detected or given) and `duration_ms`. Beam search is 5 wide by default, as whisper.cpp's own program does; `beam_size: 1` is greedy. `threads` is the processors, up to eight, unless set. `flash_attention` is on, as whisper.cpp's own program has it: without it the same model took 1.4 to 1.5 times as long on `base` and `large-v3-turbo-q5_0` (whisper-rs leaves it off by default); changing it loads the model again.

`open` refuses a model whose file is not in `models_dir` (`the model X is not installed: ...`), and starts loading the model on a thread so it is ready when the person stops. The loaded model is kept for the next recording and freed when a recording asks for another model, after `IDLE` (five minutes) without one, or by `unload()`; a recording running keeps what it uses until it ends. One recording runs at a time on a model.

## Gotchas

- Logs from whisper.cpp and ggml are discarded (`whisper_rs::install_logging_hooks`), so nothing is written to the host's standard streams.
- A model id the catalog does not list is read from `ggml-<id>.bin` in the folder, so a file the person drops there runs.
- The provider's tests that run a model (`tests/speech.rs`) download `tiny` and a recording with `curl` into a temporary folder, check the model's SHA-256, and are ignored without the network: `cargo test -p canoa-whisper -- --ignored`.
- When a `whisper-rs-sys` that bundles whisper.cpp 1.9 or later is published, drop the `[patch.crates-io]` entry and `third_party/whisper-rs-sys`.
