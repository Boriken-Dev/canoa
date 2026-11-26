//! What the session layer reports.

use alloc::boxed::Box;
use alloc::string::String;
use std::io;

/// Any error a hook or an auth provider reports.
pub type BoxError = Box<dyn core::error::Error + Send + Sync>;

/// What can go wrong in a session request.
///
/// A message names the setting, the key or the URL's scheme, host and path, and never a secret,
/// a query string or a header value. A caller's mistake (a bad argument, two options that
/// contradict) is [`Error::InvalidRequest`] or [`Error::Configuration`], reported before anything
/// is sent.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A setting or a stored credential that cannot be used: a malformed environment variable, a
    /// token with a line break in it. `variable` names the variable or the key, never the value.
    #[error("{variable}: {reason}")]
    Configuration {
        /// The variable, setting or jar key at fault.
        variable: String,
        /// What is wrong with it, free of its value.
        reason: String,
    },
    /// A hook failed; `source` is the hook's own error.
    #[error("the hook {key} failed for {request}: {source}")]
    Hook {
        /// The hook's key (upper case).
        key: String,
        /// The request the hook ran for: method, scheme, host, port and path.
        request: String,
        /// What the hook reported.
        #[source]
        source: BoxError,
    },
    /// A reply whose body is not what a helper was asked for (JSON that is not JSON).
    #[error("response was not valid JSON (HTTP {status})")]
    Response {
        /// The status of the reply.
        status: u16,
    },
    /// A request that cannot be sent: a URL, header, method or body that fails the transport's
    /// checks.
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    /// A reply longer than the limit asked for.
    #[error("the reply for {what} is larger than {limit} bytes")]
    TooLarge {
        /// The request, as a message may show it.
        what: String,
        /// The limit that was exceeded, in bytes.
        limit: usize,
    },
    /// A redirect that is not followed, and why.
    #[error("{0}")]
    Redirect(String),
    /// A network or file error, a reply that is not valid HTTP or ends early.
    #[error("{0}")]
    Io(#[from] io::Error),
    /// A wait or the deadline ran out.
    #[error("{0}")]
    Timeout(String),
    /// A TLS configuration, a keystore or a certificate could not be used.
    #[error(transparent)]
    Tls(#[from] coqui_sslcontext::Error),
    /// Resolving a proxy, connecting through one or logging in failed.
    #[error(transparent)]
    Proxy(#[from] coqui_proxy::Error),
}

impl Error {
    pub(crate) fn config(variable: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::Configuration {
            variable: variable.into(),
            reason: reason.into(),
        }
    }

    /// Whether the failure is a connection that did not work (refused, reset, closed, no route),
    /// which a retry may fix; a timeout, a refused route or a malformed reply is not.
    #[must_use]
    pub fn is_connection_failure(&self) -> bool {
        let kind = match self {
            Self::Io(error) => error.kind(),
            Self::Proxy(coqui_proxy::Error::Io(error)) => error.kind(),
            Self::Proxy(coqui_proxy::Error::Refused(_)) => return true,
            _ => return false,
        };
        matches!(
            kind,
            io::ErrorKind::ConnectionRefused
                | io::ErrorKind::ConnectionReset
                | io::ErrorKind::ConnectionAborted
                | io::ErrorKind::NotConnected
                | io::ErrorKind::BrokenPipe
                | io::ErrorKind::UnexpectedEof
                | io::ErrorKind::HostUnreachable
                | io::ErrorKind::NetworkUnreachable
                | io::ErrorKind::NetworkDown
                | io::ErrorKind::Other
        )
    }

    /// Whether the failure is a timeout or the deadline.
    #[must_use]
    pub fn is_timeout(&self) -> bool {
        match self {
            Self::Timeout(_) => true,
            Self::Io(error) => error.kind() == io::ErrorKind::TimedOut,
            _ => false,
        }
    }
}
