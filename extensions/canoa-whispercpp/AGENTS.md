# `canoa-whispercpp` — API header

Header-file-style reference for `canoa-whispercpp`: whisper.cpp for a Canoa host (the Store type that lists its models, the models page of that type, and the `speech_provider` that transcribes with a model inside the host's own process), so it can be used without reading its source. It ships inside the package and is self-contained. Development documentation lives with the source at <https://github.com/Boriken-Dev/canoa>.

MSRV Rust 1.89. Version `0.0.0-alpha.0`, not published (`publish = false`). `#![deny(unsafe_code)]`, with one module that allows it: `src/native.rs` holds the two foreign calls the converter makes (a read-only memory map of `model.safetensors`, and `ggml_quantize_chunk` with `ggml_row_size`), each behind a safe function. Depends on `canoa` (features `store`, `speech`), `coqui-acp-api`, `guatiao`, `guatiao-intake`, `serde_json`, `safetensors`, `half`, `memmap2`, `whisper-rs-sys` and `whisper-rs` 0.16.0, whose `whisper-rs-sys` is replaced (`[patch.crates-io]`) by the fork `Boriken-Dev/whisper-rs-sys` at its `main` (the crate's 0.15.0 as the first commit, then whisper.cpp v1.9.6 in place of the 1.8.3 it carries, the Windows on ARM build and the CPU target option as commits on top; the deps branch vendors it like the other git crates). whisper-rs 0.16's safe API compiles against 1.9.6 unchanged. cmake compiles the sources with the crate.

What a host loads is `library` (below). The host names none of it: it carries the library as it carries any other, lists the type and its models through the generic Store calls and makes the speech offer from what `WhisperModels::service` answers.

## Building

Compiling the crate compiles whisper.cpp (C and C++), so a build needs a C++ toolchain, **cmake** and **libclang** (bindgen). Nothing is fetched: the sources are in the crate, and `deps` carries them. OpenMP is not used (measured: no gain on Windows on ARM or Linux x86-64, and it would need a runtime beside the program).

- **Linux**: `g++`, `cmake`, `libclang` (Debian: `build-essential cmake libclang-dev`). The container's `host` stage installs them. ggml is compiled for the build machine's own CPU (`GGML_NATIVE`, its default), so a binary built on one machine may not run on an older one; the container's host stage names a baseline instead (AVX2, FMA, F16C, BMI2 on x86-64).
- **Windows on ARM**: ggml refuses `cl.exe`, so the fork's build script uses LLVM's `clang-cl` and Ninja for that target by itself: it takes them from `PATH`, else from LLVM's and Visual Studio's own folders (Visual Studio's copy of ninja is in `Common7/IDE/CommonExtensions/Microsoft/CMake/Ninja`), and needs no environment variable. libclang is LLVM's `libclang.dll`.
- **The CPU the code is built for** is the fork's `CANOA_WHISPER_CPU` (unset: a portable baseline, `x86-64-v3` on x86-64, `armv8.6-a+dotprod+i8mm` on Windows on ARM, `armv8.2-a+dotprod+fp16` on other AArch64; `native` or a named target by choice; ggml's `GGML_*` switches still win). The native build on a Snapdragon X2 Elite used SVE and SME and ran 2.4 times slower, and would not run on a processor without them. The speech provider's description reports `whisper_rs_sys::CPP_VERSION` and `CPU_TARGET` as `SpeechInfo::version` and `built_for`, and Settings > About shows them.
- **Windows x64**: Visual Studio's C++ tools and cmake; untested.

## Exports

```rust
pub const LIBRARY: &str = "canoa-whispercpp";   // the name `library` is registered under
pub const NAME: &str = "whisper.cpp";        // what it is called to the person: its rows, facts and speech offer
pub const STORE_TYPE: &str = "whispercpp";   // the id of the Store type, and of its speech offer
pub const GGML_FILE: &str = "ggml.bin";      // the file `canoa-whisper`'s install writes in a model's folder: <models>/<id>/ggml.bin
pub const GGML_RECORD: &str = "ggml.json";   // what it recorded of it: `precision`, `bytes`, `sha256`
pub const PROVIDER: &str = "speech_whisper"; // the id of the speech provider
pub const IDLE: Duration;                    // 5 minutes: how long a loaded model is kept without a recording

/// What a host links: handed to `Registry::register_local`.
pub fn library(host: guatiao::library::Host) -> Option<LibraryParts>;   // WhisperExtension, WhisperModels, WhisperHf, WhisperSpeech

pub struct WhisperExtension;   // a `store_extension`, id "whispercpp_extension": the type `whispercpp` ("whisper.cpp", icon `chip`, no program store, models `whisper_models`)
pub struct WhisperModels;      // a `model_store`, id "whispercpp_models": the speech offer's side of the type (its endpoints, `service`); the page's rows are the catalogs'
pub struct WhisperSpeech;      // a `speech_provider`, id "speech_whisper", built `From<WhisperSettings>`
pub struct WhisperSettings { pub model: String, pub models_dir: String, pub threads: Option<u32>, pub beam_size: u32, pub flash_attention: bool }   // `WhisperSettings::new(model, models_dir)`; beam_size defaults to 5, flash_attention to true
pub fn unload();               // frees the model held
pub fn is_loaded() -> bool;    // whether a model is held

pub struct WhisperHf;          // a `catalog_source`, id "whispercpp_hf", serving the type `whispercpp`: the `ggml-*.bin` files of a Hugging Face repository, from its file listing at a revision
pub struct WhisperHfSettings { pub repo: String, pub include: Vec<String>, pub exclude: Vec<String> }   // repo `ggerganov/whisper.cpp`; patterns with `*` on file names, none meaning every `ggml-*.bin`
pub const SOURCE: &str = "whispercpp_hf";
pub fn catalog_of(repo: &str, revision: &str, listing: &str, include: &[String], exclude: &[String]) -> Result<ModelsCatalog, String>;   // what the source makes of a listing

/// A Hugging Face Whisper folder as whisper.cpp's GGML file.
pub enum Precision { F16, Q5_0, Q5_1, Q8_0 }   // `Precision::ALL`; `suffix()` is "", "-q5_0", "-q5_1", "-q8_0" (as in `ggml-small-q5_1.bin`); `parse("q5_1")`
pub enum ConvertError { Io(std::io::Error), Unsupported(String) }   // Display says what is missing or unread
pub fn convert(folder: &Path, output: &Path, precision: Precision) -> Result<(), ConvertError>;

pub mod catalog {
    pub const LABEL: &str = "ggerganov";    // the default catalog's label: `[store.whispercpp.catalogs.ggerganov]`
    pub const DEFAULT_REPO: &str;           // ggerganov/whisper.cpp
    pub const ENDPOINTS: [&str; 6];         // where the files are downloaded from: the repository's files and the hosts Hugging Face sends a download on to
    pub fn default_catalog() -> DefaultCatalog;
}
```

## The models page and the speech offer

The type has no program: `WhisperExtension` names no store, and the host keeps the type's files under its own id in the shelf (`<shelf>/whispercpp/models`). It holds no model, size or digest: the type registers no catalog (its page is empty: the Whisper type of `canoa-whisper` lists the models, and this one stays for its speech offer and for a catalog canoa.cfg adds); the catalog `catalog::default_catalog()` describes, `ggerganov` (`CatalogWords` in `canoa.models/1`), has a source that `whisper_hf` reads Hugging Face's file listing of the repository at the entry's `pin`, or `main` when it has none (`GET /api/models/<repo>/tree/<revision>`), takes each large file's SHA-256 from `lfs.oid`, and names a model from its file name (`ggml-small-q5_1.bin` is `small-q5_1`, "Small, every language, compressed"). The default lists every `ggml-*.bin` file of the listing and ships no list: with no network and nothing read before, the page is empty, with the reason. A person's entry replaces the catalog by its label, adds another repository, or narrows `include`. The host lists the catalog's models on the page with their badge and downloads a row's file checked by that digest. `WhisperModels::list` answers no row of its own.

`describe().speech` is `SpeechService::in_process(.., "speech_whisper")`. `service` lists, with `query.models`, the models installed in the folder (every `ggml-<id>.bin`, the default catalog's first) and answers `settings`: the `speech_whisper` settings for `query.model` when that is installed, else for the first that is (`model`, `models_dir`); with no folder or no model it answers `unavailable` in its own words. The host writes the person's provider from those settings.

## The provider

`WhisperSpeech` takes `pcm_s16le` at any rate and channel count (averaged to mono and resampled to 16 kHz by linear interpolation), keeps the recording until `finish`, and runs whisper.cpp on it with the language hint's primary subtag (`es-PR` is `es`) or its own detection, the vocabulary hint joined with `, ` as the initial prompt, and no timestamps. The transcript's front matter holds `language` (detected or given) and `duration_ms`. Beam search is 5 wide by default, as whisper.cpp's own program does; `beam_size: 1` is greedy. `threads` is the processors, up to eight, unless set. `flash_attention` is on, as whisper.cpp's own program has it: without it the same model took 1.4 to 1.5 times as long on `base` and `large-v3-turbo-q5_0` (whisper-rs leaves it off by default); changing it loads the model again.

`open` refuses a model whose file is not in `models_dir` (`the model X is not installed: ...`), and starts loading the model on a thread so it is ready when the person stops. The loaded model is kept for the next recording and freed when a recording asks for another model, after `IDLE` (five minutes) without one, or by `unload()`; a recording running keeps what it uses until it ends. One recording runs at a time on a model.

## Gotchas

- Logs from whisper.cpp and ggml are discarded (`whisper_rs::install_logging_hooks`), so nothing is written to the host's standard streams.
- A model is the file `<models_dir>/<id>/ggml.bin` when that exists (what `canoa-whisper`'s install wrote), else `ggml-<id>.bin` in the folder, whatever catalog it came from, so a file the person drops there runs.
- The provider's tests that run a model (`tests/speech.rs`) download `tiny` (from the address a recorded listing gives) and a recording with `curl` into a temporary folder, check the model's SHA-256, and are ignored without the network: `cargo test -p canoa-whispercpp -- --ignored`.
- When a `whisper-rs-sys` that bundles whisper.cpp 1.9 or later and the CPU option is published, drop the `[patch.crates-io]` entry.

## The converter

`convert(folder, output, precision)` reads the Hugging Face folder of a Whisper model (`config.json`, `tokenizer.json`, `model.safetensors`; f32, f16 or bf16 weights) and writes whisper.cpp's GGML file, in pure Rust, with no Python and no whisper.cpp program. The sizes come from `config.json`, the vocabulary from `tokenizer.json`'s `model.vocab` (the byte-level tokens whose id is below 50257: for an English-only model that ends with `<|endoftext|>`), the mel filters are computed (Slaney scale and norm, 80 or 128 bands as `num_mel_bins` says), and the tensors are written in the order of the files whisper.cpp publishes. The weights are read from a memory map one tensor at a time, so a large model is never held whole (the largest tensor, as f32, is the peak). Output goes to `<output>.part` and is renamed when whole; a failure removes it.

- `F16` writes the same tensors as the published `ggml-<model>.bin`, byte for byte for tiny and tiny.en; for large-v3, large-v3-turbo and distil-small.en (whose published files list the tensors in another order) every tensor is identical. `Q8_0`, `Q5_0` and `Q5_1` are quantized by the ggml this crate links, from the f16-rounded values, as whisper.cpp's own `quantize` does: two-dimensional tensors other than `encoder.conv1.bias`, `encoder.conv2.bias` and the two positional embeddings; every other tensor is written as in the f16 file. The header's `ftype` carries ggml's quantization version (2), so a published file made by an older ggml differs from ours in those two bytes alone.
- A quantized precision needs each weight row to divide into blocks of 32; a model whose rows do not is refused with the tensor's name, and f16 still works.
- A tensor the folder lacks is named in the error (`model.safetensors has no tensor for decoder.blocks.0.mlp.2.weight`). Tensors whisper.cpp has no use for (`proj_out.weight`, tied to the embedding) are left out. Weights in a type other than f32, f16 or bf16, and folders with only a `pytorch_model.bin` or a sharded `model.safetensors.index.json`, are not read.
- `tests/convert.rs` makes a small model of its own and reads the file back (always run), and its ignored tests download `openai/whisper-tiny` at `main` into a temporary folder, compare f16, q8_0 and q5_1 with whisper.cpp's published tiny files (q5_1 may differ in bytes 44 and 45, the quantization version) and transcribe a recording with every precision.
