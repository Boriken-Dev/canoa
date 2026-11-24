//! The crate's error type.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use crate::proxy::Proxy;

/// What can go wrong resolving a proxy, logging in to one or building a TLS configuration.
///
/// A message names what failed and never quotes the input: a proxy URL, a PAC script or a
/// credential may be a secret.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Text that should name a proxy, a PAC result or a URL could not be read.
    #[error("malformed {0}")]
    Parse(&'static str),
    /// A PAC script failed to load, raised, overran its limits or answered with something unusable.
    #[error("pac script failed: {0}")]
    Pac(Reason),
    /// A PAC location was configured, but this build has no JavaScript engine to run it
    /// (the `pac` feature) and the operating system does not run it for this crate.
    #[error("a pac script needs the `pac` feature")]
    MissingEngine,
    /// The operating system's proxy settings could not be read or applied.
    #[error("system proxy settings: {0}")]
    Settings(Reason),
    /// No proxy the resolver offered accepted a connection.
    #[error("no reachable proxy")]
    NoReachableProxy,
    /// A proxy answered a `CONNECT` with a refusal other than a request for a login.
    #[error("proxy refused the connection: {0}")]
    Refused(Reason),
    /// A proxy or a server refused the credentials, or a Negotiate or NTLM exchange failed.
    #[error("login failed: {0}")]
    Login(Reason),
    /// Every proxy offered speaks a protocol this client cannot use (an `https:` proxy without the
    /// `tls` feature, or an IPv6 address through SOCKS 4).
    #[error("no usable proxy: {0}")]
    Unsupported(Reason),
    /// An HTTP client cannot use any of the proxies it was offered, and every one is listed with
    /// the reason.
    #[error("{0}")]
    UnsupportedProxy(Box<UnsupportedProxy>),
    /// A TLS configuration, a keystore or a certificate could not be used.
    #[error("tls: {0}")]
    Tls(Reason),
    /// A network or file operation failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl Error {
    /// The Cargo feature to enable when this is [`Error::MissingEngine`] (`pac`).
    #[must_use]
    pub fn missing_feature(&self) -> Option<&'static str> {
        matches!(self, Self::MissingEngine).then_some("pac")
    }
}

/// Why an HTTP client cannot use any proxy of an answer: the client, and each proxy with its
/// reason. Nothing in it quotes a credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsupportedProxy {
    client: String,
    refused: Vec<(Proxy, String)>,
}

impl UnsupportedProxy {
    /// Names the `client` and the `refused` proxies with the reason each was.
    pub fn new(client: impl Into<String>, refused: Vec<(Proxy, String)>) -> Self {
        Self {
            client: client.into(),
            refused,
        }
    }

    /// The HTTP client that cannot use them.
    #[must_use]
    pub fn client(&self) -> &str {
        &self.client
    }

    /// The proxies, without credentials in their text.
    #[must_use]
    pub fn proxies(&self) -> Vec<&Proxy> {
        self.refused.iter().map(|(proxy, _)| proxy).collect()
    }

    /// The reason for each proxy, in the order of [`UnsupportedProxy::proxies`].
    #[must_use]
    pub fn reasons(&self) -> Vec<&str> {
        self.refused
            .iter()
            .map(|(_, reason)| reason.as_str())
            .collect()
    }
}

impl fmt::Display for UnsupportedProxy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} cannot use the proxies offered", self.client)?;
        for (proxy, reason) in &self.refused {
            write!(formatter, "; {proxy}: {reason}")?;
        }
        Ok(())
    }
}

/// A short, fixed or derived description of why something failed, free of input text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reason(String);

impl Reason {
    pub(crate) fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }
}

impl fmt::Display for Reason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}
