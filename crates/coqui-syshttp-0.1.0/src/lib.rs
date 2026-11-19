#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(unsafe_code)]
#![warn(missing_docs)]
#![warn(missing_debug_implementations)]
#![warn(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    clippy::alloc_instead_of_core
)]

extern crate alloc;

pub mod auth;
mod bypass;
#[cfg(feature = "ureq")]
pub mod client;
mod connect;
mod env;
mod error;
mod netutil;
#[cfg(feature = "pac")]
pub mod pac;
mod proxy;
mod resolver;
mod socks;
pub mod system;
#[cfg(feature = "tls")]
pub mod tls;

pub use auth::{Negotiate, NegotiateBuilder, OriginAuth, OriginAuthBuilder, OriginLogin};
pub use bypass::{BypassRules, Dialect};
pub use connect::{Connection, ProxyConnector, ProxyConnectorBuilder};
pub use env::{Environment, ProxyVariables};
pub use error::{Error, Reason};
pub use proxy::{Credentials, Proxy, ProxyBuilder, ProxyScheme, Route, Secret};
pub use resolver::{ChildEnvironment, OnMissingEngine, ProxySetting, Resolver, ResolverBuilder};
