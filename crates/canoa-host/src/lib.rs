#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

extern crate alloc;

mod accounts;
mod child;
pub mod cli;
pub mod config;
pub mod harness;
pub mod home;
mod inproc;
mod kind;
pub mod options;
pub mod peers;
pub mod profile;
pub mod provider;
mod scope;
pub mod serve;
pub mod tool;
mod usage_splice;
pub mod web;
mod worktree;

pub use accounts::ACCOUNT_META;
pub use config::{
    AccountConfig, AccountStore, Config, ConfigError, Entry, EntryKind, HarnessOptions, Origin,
    Settings, UsageSource, WebConfig,
};
pub use harness::Harness;
pub use home::{Holder, Home, HomeLock, LockError};
pub use peers::{Peer, PeerState, PeersError};
pub use provider::{HostLine, ProviderError, ProviderOptions, serve_stdio};
pub use serve::{Listening, ServeError, ServeOptions, Serving, Stopper};
pub use tool::ToolSpec;
pub use web::{WebError, WebServer, WebStopper};
