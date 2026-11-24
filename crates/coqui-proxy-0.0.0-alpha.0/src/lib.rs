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
mod netutil;
#[cfg(feature = "pac")]
pub mod pac;
mod proxy;
mod resolver;
pub mod settings;
pub mod source;
pub mod system;
#[cfg(feature = "tls")]
mod tls;

pub use auth::{Negotiate, NegotiateBuilder, OriginAuth, OriginAuthBuilder, OriginLogin};
pub use bypass::{BypassRules, Dialect};
pub use connect::{Connection, ProxyConnector, ProxyConnectorBuilder};
pub use env::{EnvSource, EnvSourceBuilder, Environment, ProxyVariables};
pub use error::{Error, Reason, UnsupportedProxy};
pub use proxy::{Credentials, Proxy, ProxyBuilder, ProxyScheme, Route, Secret};
pub use resolver::{ChildEnvironment, OnMissingEngine, ProxySetting, Resolver, ResolverBuilder};
pub use settings::{ConnectorSettings, PacSettings, ReaderSettings, ResolverSettings};
pub use source::{ChainSource, ProxySource};

/// This crate's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
