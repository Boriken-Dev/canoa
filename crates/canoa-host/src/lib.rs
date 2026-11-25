#![doc = include_str!("../README.md")]
#![deny(unsafe_code)]

extern crate alloc;

mod accounts;
mod addresses;
mod assist;
mod awake;
mod bundled;
mod cfg_edit;
mod child;
pub mod cli;
mod client_tools;
pub mod config;
mod config_store;
pub mod control;
mod delegate;
mod descriptions;
pub mod disabled;
mod environments;
mod format;
mod forms;
mod gateways;
mod harness;
mod history;
pub mod home;
pub mod host_log;
mod inproc;
mod kb_mcp;
mod kind;
mod launcher;
mod local;
mod loopback;
mod makers;
mod mcp_list;
pub mod native;
mod network;
pub mod options;
mod pair_run;
mod panel_tabs;
pub mod peers;
pub mod pipe;
mod pools;
#[cfg(all(unix, feature = "portal"))]
pub mod portal;
pub mod portal_link;
pub mod profile;
pub mod provider;
mod replace;
mod scope;
mod scripts;
mod secrets;
pub mod serve;
mod services;
mod settings_sections;
mod share;
pub mod shells;
mod site;
mod sources;
pub mod spawn;
mod speech_scripts;
#[cfg(test)]
mod statics_tests;
pub mod store;
mod terminal;
pub mod tool;
mod transport;
mod usage_log;
mod usage_splice;
mod utc;
mod vendors;
pub mod web;
mod worktree;

pub use accounts::ACCOUNT_META;
pub use config::{
    AccountConfig, CACHE_LIFETIMES, Config, ConfigError, ControlConfig, DEFAULT_PRESET,
    DelegateConfig, ENTRIES, Entry, EntryKind, Extension, ExtensionCheck, HarnessOptions,
    INSTALLED_DIR_VARIABLE, KbConfig, Layers, Listen, Loadout, NativeConfig, NetworkConfig,
    OpenerConfig, Origin, PoolConfig, Preset, ResolvedLoadout, Settings, StoreConfig,
    TransportListen, UnknownLoadout, UnknownPreset, WebConfig, WebMode, cache_lifetime_label,
    cache_lifetime_name, cache_lifetime_named, merge_into,
};
pub use serve::MAX_MESSAGE_META;
/// The `_meta` key of `initialize` that asks a host for the delegation tools, and that a task's permission request carries
/// with its task id, title and source.
pub const DELEGATE_META: &str = delegate::META;
/// The `_meta` key of `initialize` that asks a host for the `canoa-store` tools (search and inspect the Store).
pub const STORE_META: &str = store::mcp::META;
/// The `_meta` key of `initialize` that asks for the `canoa-kb` tools.
pub const KB_META: &str = kb_mcp::META;
pub use home::{Holder, Home, HomeLock, LockError};
pub use kind::{CACHE_META, MODEL_META};
pub use local::{Link, LinkError};
pub use peers::{Peer, PeerState, PeersError};
pub use provider::{HostLine, ProviderError, ProviderOptions, serve_stdio};
pub use serve::{Listening, Local, LocalError, ServeError, ServeOptions, Serving, Stopper};
pub use site::SITE_META;
pub use tool::ToolSpec;
pub use web::{WebError, WebServer, WebStopper};
