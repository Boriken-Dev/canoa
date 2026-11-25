//! whisper.cpp for Canoa: the models it runs and the speech-to-text provider that runs them inside the host's process.
//!
//! [`library`] is what a Canoa host loads: the Store type that lists the models, the `model_store` that lists them and says how the
//! person's speech-to-text calls them, and [`WhisperSpeech`], the `speech_provider` that loads a model and transcribes with it.
//! There is no program to install: whisper.cpp is compiled into this crate (through `whisper-rs`), and the host names none of it.
#![forbid(unsafe_code)]

extern crate alloc;

pub mod catalog;
mod model_store;
mod providers;
mod source;
mod speech;

pub use model_store::WhisperModels;
pub use providers::{WhisperExtension, library};
pub use source::{SOURCE, WhisperHf, WhisperHfSettings, catalog_of};
pub use speech::{IDLE, PROVIDER, WhisperSettings, WhisperSpeech, is_loaded, unload};

/// The name this crate's library is registered under.
pub const LIBRARY: &str = "canoa-whisper";

/// What whisper.cpp is called to the person: the name on its Store rows, its facts and its speech offer.
pub const NAME: &str = "whisper.cpp";

/// The id of the Store type that lists whisper.cpp's models, and so the id of its speech offer.
pub const STORE_TYPE: &str = "whisper";
