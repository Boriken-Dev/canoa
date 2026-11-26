# `canoa-whispercpp` — API header

Header-file-style reference for `canoa-whispercpp`: whisper.cpp for a Canoa host (the `speech_provider` that transcribes with a model inside the host's own process, and the converter that writes the file it reads from a Hugging Face Whisper folder), so it can be used without reading its source. It ships inside the package and is self-contained. Development documentation lives with the source at <https://github.com/Boriken-Dev/canoa>.

MSRV Rust 1.89. Version `0.0.0-alpha.0`, not published (`publish = false`). `#![deny(unsafe_code)]`, with one module that allows it: `src/native.rs` holds the two foreign calls the converter makes (a read-only memory map of `model.safetensors`, and `ggml_quantize_chunk` with `ggml_row_size`), each behind a safe function. Depends on `canoa` (feature `speech`), `coqui-acp-api`, `guatiao`, `guatiao-intake`, `serde_json`, `safetensors`, `half`, `memmap2`, `whisper-rs-sys` and `whisper-rs` 0.16.0, whose `whisper-rs-sys` is replaced (`[patch.crates-io]`) by the fork `Boriken-Dev/whisper-rs-sys` at its `main` (the crate's 0.15.0 as the first commit, then whisper.cpp v1.9.6 in place of the 1.8.3 it carries, the Windows on ARM build and the CPU target option as commits on top; the deps branch vendors it like the other git crates). whisper-rs 0.16's safe API compiles against 1.9.6 unchanged. cmake compiles the sources with the crate.

What a host loads is `library` (below). The host names none of it: it carries the library as it carries any other. The models are the Whisper type's (`canoa-whisper`), whose install calls `convert`; this crate has no Store type, catalog or model list of its own.

## Building

Compiling the crate compiles whisper.cpp (C and C++), so a build needs a C++ toolchain, **cmake** and **libclang** (bindgen). Nothing is fetched: the sources are in the crate, and `deps` carries them. OpenMP is not used (measured: no gain on Windows on ARM or Linux x86-64, and it would need a runtime beside the program).

- **Linux**: `g++`, `cmake`, `libclang` (Debian: `build-essential cmake libclang-dev`). The container's `host` stage installs them. ggml is compiled for the build machine's own CPU (`GGML_NATIVE`, its default), so a binary built on one machine may not run on an older one; the container's host stage names a baseline instead (AVX2, FMA, F16C, BMI2 on x86-64).
- **Windows on ARM**: ggml refuses `cl.exe`, so the fork's build script uses LLVM's `clang-cl` and Ninja for that target by itself: it takes them from `PATH`, else from LLVM's and Visual Studio's own folders (Visual Studio's copy of ninja is in `Common7/IDE/CommonExtensions/Microsoft/CMake/Ninja`), and needs no environment variable. libclang is LLVM's `libclang.dll`.
- **The CPU the code is built for** is the fork's `CANOA_WHISPER_CPU` (unset: a portable baseline, `x86-64-v3` on x86-64, `armv8.6-a+dotprod+i8mm` on Windows on ARM, `armv8.2-a+dotprod+fp16` on other AArch64; `native` or a named target by choice; ggml's `GGML_*` switches still win). The native build on a Snapdragon X2 Elite used SVE and SME and ran 2.4 times slower, and would not run on a processor without them. The speech provider's description reports `whisper_rs_sys::CPP_VERSION` and `CPU_TARGET` as `SpeechInfo::version` and `built_for`, and Settings > About shows them.
- **Windows x64**: Visual Studio's C++ tools and cmake; untested.

## Exports

```rust
pub const LIBRARY: &str = "canoa-whispercpp";   // the name `library` is registered under
pub const GGML_FILE: &str = "ggml.bin";      // the file `canoa-whisper`'s install writes in a model's folder: <models>/<id>/ggml.bin
pub const GGML_RECORD: &str = "ggml.json";   // what it recorded of it: `precision`, `bytes`, `sha256`
pub const PROVIDER: &str = "speech_whisper"; // the id of the speech provider
pub const IDLE: Duration;                    // 5 minutes: how long a loaded model is kept without a recording

/// What a host links: handed to `Registry::register_local`.
pub fn library(host: guatiao::library::Host) -> Option<LibraryParts>;   // WhisperSpeech

pub struct WhisperSpeech;      // a `speech_provider`, id "speech_whisper", built `From<WhisperSettings>`
pub struct WhisperSettings { pub model: String, pub models_dir: String, pub threads: Option<u32>, pub beam_size: u32, pub flash_attention: bool }   // `WhisperSettings::new(model, models_dir)`; beam_size defaults to 5, flash_attention to true
pub fn unload();               // frees the model held
pub fn is_loaded() -> bool;    // whether a model is held

/// A Hugging Face Whisper folder as whisper.cpp's GGML file.
pub enum Precision { F16, Q5_0, Q5_1, Q8_0 }   // `Precision::ALL`; `suffix()` is "", "-q5_0", "-q5_1", "-q8_0" (as in `ggml-small-q5_1.bin`); `parse("q5_1")`
pub enum ConvertError { Io(std::io::Error), Unsupported(String) }   // Display says what is missing or unread
pub fn convert(folder: &Path, output: &Path, precision: Precision) -> Result<(), ConvertError>;
```

## The models

The crate lists and downloads nothing: a model is a folder `<models_dir>/<id>/` of the Whisper type, whose install writes `ggml.bin` (and `ggml.json`) in it with `convert`, and whose speech offer (`canoa-whisper`) names `speech_whisper` and the settings (`model`, `models_dir`) for a model that has the file. No file named `ggml-<id>.bin` is read.

## The provider

`WhisperSpeech` takes `pcm_s16le` at any rate and channel count (averaged to mono and resampled to 16 kHz by linear interpolation), keeps the recording until `finish`, and runs whisper.cpp on it with the language hint's primary subtag (`es-PR` is `es`) or its own detection, the vocabulary hint joined with `, ` as the initial prompt, and no timestamps. The transcript's front matter holds `language` (detected or given) and `duration_ms`. Beam search is 5 wide by default, as whisper.cpp's own program does; `beam_size: 1` is greedy. `threads` is the processors, up to eight, unless set. `flash_attention` is on, as whisper.cpp's own program has it: without it the same model took 1.4 to 1.5 times as long on `base` and `large-v3-turbo-q5_0` (whisper-rs leaves it off by default); changing it loads the model again.

`open` refuses a model whose file is not in `models_dir` (`the model X is not installed: ...`), and starts loading the model on a thread so it is ready when the person stops. The loaded model is kept for the next recording and freed when a recording asks for another model, after `IDLE` (five minutes) without one, or by `unload()`; a recording running keeps what it uses until it ends. One recording runs at a time on a model.

## Gotchas

- Logs from whisper.cpp and ggml are discarded (`whisper_rs::install_logging_hooks`), so nothing is written to the host's standard streams.
- A model is the file `<models_dir>/<id>/ggml.bin`, which `canoa-whisper`'s install (or its action that adds whisper.cpp to a model already installed) writes; `open` refuses a model without it.
- The provider's tests that run a model (`tests/speech.rs`) download `openai/whisper-tiny`'s folder and a recording with `curl` into a temporary folder, convert it to f16 and are ignored without the network: `cargo test -p canoa-whispercpp -- --ignored`.
- When a `whisper-rs-sys` that bundles whisper.cpp 1.9 or later and the CPU option is published, drop the `[patch.crates-io]` entry.

## The converter

`convert(folder, output, precision)` reads the Hugging Face folder of a Whisper model (`config.json`, `tokenizer.json`, `model.safetensors`; f32, f16 or bf16 weights) and writes whisper.cpp's GGML file, in pure Rust, with no Python and no whisper.cpp program. The sizes come from `config.json`, the vocabulary from `tokenizer.json`'s `model.vocab` (the byte-level tokens whose id is below 50257: for an English-only model that ends with `<|endoftext|>`), the mel filters are computed (Slaney scale and norm, 80 or 128 bands as `num_mel_bins` says), and the tensors are written in the order of the files whisper.cpp publishes. The weights are read from a memory map one tensor at a time, so a large model is never held whole (the largest tensor, as f32, is the peak). Output goes to `<output>.part` and is renamed when whole; a failure removes it.

- `F16` writes the same tensors as the published `ggml-<model>.bin`, byte for byte for tiny and tiny.en; for large-v3, large-v3-turbo and distil-small.en (whose published files list the tensors in another order) every tensor is identical. `Q8_0`, `Q5_0` and `Q5_1` are quantized by the ggml this crate links, from the f16-rounded values, as whisper.cpp's own `quantize` does: two-dimensional tensors other than `encoder.conv1.bias`, `encoder.conv2.bias` and the two positional embeddings; every other tensor is written as in the f16 file. The header's `ftype` carries ggml's quantization version (2), so a published file made by an older ggml differs from ours in those two bytes alone.
- A quantized precision needs each weight row to divide into blocks of 32; a model whose rows do not is refused with the tensor's name, and f16 still works.
- A tensor the folder lacks is named in the error (`model.safetensors has no tensor for decoder.blocks.0.mlp.2.weight`). Tensors whisper.cpp has no use for (`proj_out.weight`, tied to the embedding) are left out. Weights in a type other than f32, f16 or bf16, and folders with only a `pytorch_model.bin` or a sharded `model.safetensors.index.json`, are not read.
- `tests/convert.rs` makes a small model of its own and reads the file back (always run), and its ignored tests download `openai/whisper-tiny` at `main` into a temporary folder, compare f16, q8_0 and q5_1 with whisper.cpp's published tiny files (q5_1 may differ in bytes 44 and 45, the quantization version) and transcribe a recording with every precision.
