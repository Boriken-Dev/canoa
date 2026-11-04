#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod aos;
mod child;
pub mod config;
pub mod peers;
pub mod profile;
pub mod provider;
mod scope;
pub mod serve;
pub mod tool;
pub mod web;
mod worktree;

pub use config::{Config, ConfigError, Entry, EntryKind, Origin, Settings, WebConfig};
pub use peers::{Peer, PeerState, PeersError};
pub use provider::{HostLine, ProviderError, ProviderOptions, serve_stdio};
pub use serve::{Listening, ServeError, ServeOptions, Serving, Stopper};
pub use tool::ToolSpec;
pub use web::{WebError, WebServer};
