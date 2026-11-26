#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(
    missing_docs,
    missing_debug_implementations,
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    clippy::alloc_instead_of_core
)]
#![deny(unsafe_code)]

extern crate alloc;

pub mod auth;
mod bypass;
pub mod connect;
mod dns;
mod env;
mod error;
#[cfg(feature = "hyper")]
#[cfg_attr(docsrs, doc(cfg(feature = "hyper")))]
pub mod hyper;
mod netutil;
#[cfg(feature = "pac")]
pub mod pac;
mod proxy;
#[cfg(any(feature = "relay", feature = "relay-blocking"))]
#[cfg_attr(docsrs, doc(cfg(any(feature = "relay", feature = "relay-blocking"))))]
pub mod relay;
mod resolver;
pub mod settings;
pub mod source;
pub mod system;
#[cfg(feature = "tls")]
mod tls;

pub use auth::{Negotiate, NegotiateBuilder, OriginAuth, OriginAuthBuilder, OriginLogin};
pub use bypass::{BypassRules, Dialect};
#[cfg(feature = "tokio")]
#[cfg_attr(docsrs, doc(cfg(feature = "tokio")))]
pub use connect::asynch;
pub use connect::{Connection, ProxyConnector, ProxyConnectorBuilder};
pub use env::{EnvSource, EnvSourceBuilder, Environment, ProxyVariables};
pub use error::{Error, Reason, UnsupportedProxy};
pub use proxy::{Credentials, Proxy, ProxyBuilder, ProxyScheme, Route, Secret};
pub use resolver::{ChildEnvironment, OnMissingEngine, ProxySetting, Resolver, ResolverBuilder};
pub use settings::{
    ConnectorSettings, PacSettings, ReaderSettings, RelaySettings, ResolverSettings,
};
pub use source::{ChainSource, ProxySource};

/// This crate's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
