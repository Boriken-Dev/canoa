//! Piper for Canoa: the text-to-speech program and its voices, and the provider that reads text aloud with them.
//!
//! [`store`] is the Store program that installs Piper with a voice for English and one for Spanish. [`library`] is what a Canoa host
//! loads: that program, the Store type that lists it, the `model_store` that lists the voices and says which provider speaks with
//! the installed copy, and that `speech_output` provider ([`PiperSpeech`]). The host names none of them.
#![forbid(unsafe_code)]

extern crate alloc;

pub mod catalog;
mod model_store;
mod providers;
mod source;
mod speech;
pub mod store;

pub use model_store::PiperModels;
pub use providers::{PiperExtension, PiperStore, library};
pub use source::{PiperVoices, PiperVoicesSettings, SOURCE, catalog_of};
pub use speech::{PROVIDER, PiperSettings, PiperSpeech};

/// The name this crate's library is registered under.
pub const LIBRARY: &str = "canoa-piper";

/// The id of the Store type that lists Piper.
pub const STORE_TYPE: &str = "piper";
