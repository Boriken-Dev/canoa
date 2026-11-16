#![doc = include_str!("../README.md")]
#![deny(unsafe_code)]

extern crate alloc;

mod accounts;
mod child;
pub mod cli;
pub mod config;
pub mod control;
mod delegate;
mod format;
pub mod harness;
mod history;
pub mod home;
mod inproc;
mod kb_mcp;
mod kind;
mod local;
mod loopback;
pub mod native;
pub mod options;
pub mod peers;
pub mod pipe;
#[cfg(all(unix, feature = "portal"))]
pub mod portal;
pub mod portal_link;
pub mod profile;
pub mod provider;
mod replace;
mod scope;
mod secrets;
pub mod serve;
mod share;
mod site;
mod sources;
pub mod spawn;
pub mod store;
mod terminal;
pub mod tool;
mod transport;
mod usage_log;
mod usage_splice;
pub mod web;
mod worktree;

pub use accounts::ACCOUNT_META;
pub use config::{
    AccountConfig, AccountStore, CACHE_LIFETIMES, Config, ConfigError, ControlConfig,
    DelegateConfig, ENTRIES, Entry, EntryKind, HarnessOptions, INSTALLED_DIR_VARIABLE, KbConfig,
    Layers, Listen, Loadout, NativeConfig, Origin, ResolvedLoadout, Settings, StoreConfig,
    TransportListen, UnknownLoadout, UsageSource, WebConfig, cache_lifetime_name,
    cache_lifetime_named, merge_into,
};
/// The `_meta` key of `initialize` that asks a host for the delegation tools, and that a task's permission request carries
/// with its task id, title and source.
pub const DELEGATE_META: &str = delegate::META;
/// The `_meta` key of `initialize` that asks a host for the `canoa-store` tools (search and inspect the Store).
pub const STORE_META: &str = store::mcp::META;
/// The `_meta` key of `initialize` that asks for the `canoa-kb` tools.
pub const KB_META: &str = kb_mcp::META;
pub use harness::Harness;
pub use home::{Holder, Home, HomeLock, LockError};
pub use kind::CACHE_META;
pub use local::{Link, LinkError};
pub use peers::{Peer, PeerState, PeersError};
pub use provider::{HostLine, ProviderError, ProviderOptions, serve_stdio};
pub use serve::{Listening, Local, LocalError, ServeError, ServeOptions, Serving, Stopper};
pub use site::SITE_META;
pub use tool::ToolSpec;
pub use web::{WebError, WebServer, WebStopper};
