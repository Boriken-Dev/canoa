#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod config;
pub mod peers;
pub mod provider;
mod scope;
pub mod serve;

pub use config::{Config, ConfigError, Entry, EntryKind, Origin, Settings};
pub use peers::{Peer, PeerState, PeersError};
pub use provider::{HostLine, ProviderError, ProviderOptions, serve_stdio};
pub use serve::{Listening, ServeError, ServeOptions, Serving, Stopper};
