//! Whisper for Canoa: one Store type for OpenAI's Whisper models, read from Hugging Face, whose one download both engines run.
//!
//! A model is the Hugging Face folder of `openai/whisper-*` (`config.json`, `tokenizer.json`, `model.safetensors`), downloaded once
//! and checked against the digests Hugging Face lists. Candle runs the folder as it is; whisper.cpp reads a GGML file the install writes
//! from it, in the form the person picks. [`library`] is what a host loads. The crate keeps no model name and no digest.
//!
//! Without the `host` feature only the [`catalog`] is built: it reads a Hugging Face listing and has no thread, file or clock in it, so
//! it compiles to WebAssembly, where the web app reads the same listing.
#![forbid(unsafe_code)]

extern crate alloc;

pub mod catalog;
#[cfg(feature = "host")]
mod model_store;
#[cfg(feature = "host")]
mod providers;
#[cfg(feature = "host")]
mod source;

#[cfg(feature = "host")]
pub use model_store::WhisperModels;
#[cfg(feature = "host")]
pub use providers::{WhisperExtension, library};
#[cfg(feature = "host")]
pub use source::{SOURCE, WhisperHf, WhisperHfSettings, catalog_of, default_catalog};

/// The name this crate's library is registered under.
pub const LIBRARY: &str = "canoa-whisper";

/// What the type is called to the person: its page, its facts and its speech offer.
pub const NAME: &str = "Whisper";

/// The id of the Store type, and of its speech offer.
pub const STORE_TYPE: &str = "whisper";

/// The id of the Candle engine on a model's pill, and in the install dialog's choices.
pub const CANDLE: &str = "candle";

/// The id of the whisper.cpp engine, which a build with the `whispercpp` feature offers.
pub const WHISPERCPP: &str = "whispercpp";

/// The action the install asks for once the folder is downloaded, which writes whisper.cpp's GGML file from it.
pub const WRITE_GGML: &str = "write_ggml";
