//! whisper.cpp for Canoa: the speech-to-text program, the models it runs and the speech offer made of them.
//!
//! [`store`] is the Store program that installs `whisper-cli` and one model, the model being the install's channel. [`library`] is
//! what a Canoa host loads: that program, the Store type that lists it, and the `model_store` that lists the models and says how
//! the installed copy is run for the person's speech-to-text. The host names none of them.
#![forbid(unsafe_code)]

extern crate alloc;

mod model_store;
mod providers;
pub mod store;

pub use model_store::{WhisperModels, arguments, run_of};
pub use providers::{WhisperExtension, WhisperStore, library};

/// The name this crate's library is registered under.
pub const LIBRARY: &str = "canoa-whisper";

/// What the program is called to the person: the name on its Store rows, its facts and its speech offer.
pub const NAME: &str = "whisper.cpp";

/// The id of the Store type that lists whisper.cpp, and so the id of its speech offer.
pub const STORE_TYPE: &str = "whisper";
