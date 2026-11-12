#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

extern crate alloc;

mod accounts;
mod child;
pub mod cli;
pub mod config;
pub mod control;
pub mod harness;
pub mod home;
mod inproc;
mod kind;
mod local;
pub mod options;
pub mod peers;
pub mod pipe;
pub mod profile;
pub mod provider;
mod scope;
pub mod serve;
mod sources;
pub mod store;
pub mod tool;
mod usage_splice;
pub mod web;
mod worktree;

pub use accounts::ACCOUNT_META;
pub use config::{
    AccountConfig, AccountStore, CACHE_LIFETIMES, Config, ConfigError, Entry, EntryKind,
    HarnessOptions, Listen, Origin, Settings, UsageSource, WebConfig, cache_lifetime_name,
    cache_lifetime_named,
};
pub use harness::Harness;
pub use home::{Holder, Home, HomeLock, LockError};
pub use kind::CACHE_META;
pub use local::{Link, LinkError};
pub use peers::{Peer, PeerState, PeersError};
pub use provider::{HostLine, ProviderError, ProviderOptions, serve_stdio};
pub use serve::{Listening, Local, LocalError, ServeError, ServeOptions, Serving, Stopper};
pub use tool::ToolSpec;
pub use web::{WebError, WebServer, WebStopper};
