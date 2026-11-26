#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(missing_debug_implementations)]
#![warn(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    clippy::alloc_instead_of_core
)]

extern crate alloc;

mod adapter;
mod error;
#[cfg(feature = "hyper")]
#[cfg_attr(docsrs, doc(cfg(feature = "hyper")))]
pub mod hyper;
#[cfg(feature = "reqwest")]
#[cfg_attr(docsrs, doc(cfg(feature = "reqwest")))]
pub mod reqwest;
#[cfg(feature = "session")]
#[cfg_attr(docsrs, doc(cfg(feature = "session")))]
pub mod session;
#[cfg(feature = "ureq")]
pub mod ureq;

pub use adapter::AdapterSettings;
pub use coqui_proxy as proxy;
#[cfg(feature = "pac")]
#[cfg_attr(docsrs, doc(cfg(feature = "pac")))]
pub use coqui_proxy::pac;
pub use coqui_proxy::{
    BypassRules, ChildEnvironment, Connection, Credentials, Dialect, Environment, Negotiate,
    NegotiateBuilder, OnMissingEngine, OriginAuth, OriginAuthBuilder, OriginLogin, Proxy,
    ProxyBuilder, ProxyConnector, ProxyConnectorBuilder, ProxyScheme, ProxySetting, ProxyVariables,
    Resolver, ResolverBuilder, Route, Secret, auth, system,
};
#[cfg(feature = "tls")]
#[cfg_attr(docsrs, doc(cfg(feature = "tls")))]
pub use coqui_sslcontext as tls;
pub use error::{Error, Reason};

/// A `ureq` agent that follows the system: the system's proxy, proxy login and the operating
/// system's TLS. Shorthand for `ureq::AgentBuilder::new().build()`.
///
/// # Errors
///
/// [`Error::Tls`] when the operating system's verifier cannot start.
#[cfg(feature = "ureq")]
#[cfg_attr(docsrs, doc(cfg(feature = "ureq")))]
pub fn agent() -> Result<::ureq::Agent, Error> {
    ureq::AgentBuilder::new().build()
}

/// The version of this crate, `User-Agent` of the session layer included.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
