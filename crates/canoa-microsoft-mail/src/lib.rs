//! Microsoft mail backends for Canoa (`mail_backend` Guatiao providers): Exchange Server (`ews`), Microsoft Graph (`graph`),
//! and classic Outlook for Windows (`outlook`).
#![deny(unsafe_code)]

extern crate alloc;

pub mod ews;
pub mod graph;
pub mod outlook;
mod providers;
#[cfg(all(test, windows))]
#[allow(unsafe_code)]
mod sspi_server;
#[cfg(any(test, feature = "standin"))]
pub mod standin;
#[cfg(test)]
mod tests;
pub(crate) mod xml;

pub use providers::{EwsBackend, GraphBackend, OutlookBackend, library};

/// The name this crate's library is registered under.
pub const LIBRARY: &str = "canoa-microsoft-mail";
