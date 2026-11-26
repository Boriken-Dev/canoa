//! whisper.cpp for Canoa: the speech-to-text provider that runs a model inside the host's process, and the converter that writes the
//! GGML file it reads from a Hugging Face Whisper folder.
//!
//! [`library`] is what a Canoa host loads: [`WhisperSpeech`], the `speech_provider` that loads a model and transcribes with it. The
//! models themselves are the Whisper Store type's (`canoa-whisper`), which calls [`convert`] at install. There is no program to
//! install: whisper.cpp is compiled into this crate (through `whisper-rs`), and the host names none of it.
#![deny(unsafe_code)]

extern crate alloc;

mod conversion;
mod mel;
mod names;
mod native;
mod providers;
mod speech;

pub use conversion::{ConvertError, Precision, convert};
pub use providers::library;
pub use speech::{IDLE, PROVIDER, WhisperSettings, WhisperSpeech, is_loaded, unload};

/// The file a model's folder holds when the Hugging Face folder of the model was converted at install: `<models>/<id>/ggml.bin`.
pub const GGML_FILE: &str = "ggml.bin";

/// What was recorded of that conversion, beside it: a JSON object with `precision`, `bytes` and the file's `sha256`.
pub const GGML_RECORD: &str = "ggml.json";

/// The name this crate's library is registered under.
pub const LIBRARY: &str = "canoa-whispercpp";
