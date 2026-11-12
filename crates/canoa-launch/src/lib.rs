#![doc = include_str!("../README.md")]

mod error;
mod guard;
mod manifest;
mod names;
mod pack;
mod store;

pub use error::{Error, Result};
pub use manifest::{Entry, MARKER, MAX_BYTES, MAX_FILES, Manifest, sha256_hex};
pub use pack::pack;
pub use store::{Installed, KEEP_VERSIONS, Payload, differs, install, prune};

/// The program the launcher starts from the unpacked folder.
pub const APP_PROGRAM: &str = "CanoaUI.exe";
