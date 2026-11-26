//! Whisper in Rust for Canoa: the engine that runs a model's files over Candle, and the speech-to-text provider that runs it inside
//! the host's process.
//!
//! [`library`] is what a Canoa host loads: [`WhisperCandleSpeech`], the `speech_provider` that loads a model and transcribes with
//! it. The models are the Whisper Store type's (`canoa-whisper`). There is no program to install and nothing to compile but Rust:
//! [`Engine`] is Whisper over Candle, and the host names none of it.
//!
//! Without the `host` feature only [`Engine`] and the names of a model's files are built: neither has a thread, file or clock in it,
//! so they compile to WebAssembly.
#![forbid(unsafe_code)]

extern crate alloc;

mod engine;
mod mel;
#[cfg(feature = "host")]
mod providers;
#[cfg(feature = "host")]
mod speech;

pub use engine::{Engine, Error, Transcription};
#[cfg(feature = "host")]
pub use providers::library;
#[cfg(feature = "host")]
pub use speech::{IDLE, PROVIDER, WhisperCandleSettings, WhisperCandleSpeech, is_loaded, unload};

/// The name this crate's library is registered under.
pub const LIBRARY: &str = "canoa-candle";

/// The names of the files of a model's folder: the `config.json`, the `tokenizer.json` and the `model.safetensors` weights that
/// [`Engine::load`] reads, as Hugging Face's `openai/whisper-*` repositories name them.
pub const CONFIG: &str = "config.json";
/// The tokenizer's file.
pub const TOKENIZER: &str = "tokenizer.json";
/// The weights' file.
pub const WEIGHTS: &str = "model.safetensors";
/// The three together.
pub const FILE_NAMES: [&str; 3] = [CONFIG, TOKENIZER, WEIGHTS];
