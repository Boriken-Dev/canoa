//! Whisper in Rust for Canoa: the models Candle runs and the speech-to-text provider that runs them inside the host's process.
//!
//! [`library`] is what a Canoa host loads: the Store type that lists the models, the `model_store` that lists them and says how the
//! person's speech-to-text calls them, and [`WhisperCandleSpeech`], the `speech_provider` that loads a model and transcribes with it.
//! There is no program to install and nothing to compile but Rust: [`Engine`] is Whisper over Candle, and the host names none of it.
//!
//! Without the `host` feature only [`Engine`] is built, which has no thread, file or clock in it and compiles to WebAssembly.
#![forbid(unsafe_code)]

extern crate alloc;

#[cfg(feature = "host")]
pub mod catalog;
mod engine;
mod mel;
#[cfg(feature = "host")]
mod model_store;
#[cfg(feature = "host")]
mod providers;
#[cfg(feature = "host")]
mod speech;

pub use engine::{Engine, Error, Transcription};
#[cfg(feature = "host")]
pub use model_store::WhisperCandleModels;
#[cfg(feature = "host")]
pub use providers::{WhisperCandleExtension, library};
#[cfg(feature = "host")]
pub use speech::{IDLE, PROVIDER, WhisperCandleSettings, WhisperCandleSpeech, is_loaded, unload};

/// The name this crate's library is registered under.
pub const LIBRARY: &str = "canoa-whisper-candle";

/// What this runtime is called to the person: the name on its Store rows, its facts and its speech offer.
pub const NAME: &str = "Whisper (Candle)";

/// The id of the Store type that lists the models, and so the id of its speech offer.
pub const STORE_TYPE: &str = "whispercandle";
