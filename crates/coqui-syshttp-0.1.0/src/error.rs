//! The facade's error type.

use alloc::string::String;
use core::fmt;

/// What can go wrong in a request that follows the system.
///
/// A message names what failed and never quotes the input.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Resolving a proxy, connecting through one or logging in failed.
    #[error(transparent)]
    Proxy(#[from] coqui_proxy::Error),
    /// A TLS configuration, a keystore or a certificate could not be used.
    #[cfg(feature = "tls")]
    #[error(transparent)]
    Tls(#[from] coqui_sslcontext::Error),
    /// The HTTP client refused or failed the exchange.
    #[error("http: {0}")]
    Http(Reason),
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Proxy(coqui_proxy::Error::Io(error))
    }
}

/// A short description of why something failed, free of input text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reason(String);

impl Reason {
    /// A reason with the given text.
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }
}

impl fmt::Display for Reason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}
