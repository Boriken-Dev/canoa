//! What a `coqui-transport` transport implements to become a provider of the kind.

use alloc::boxed::Box;
use alloc::string::String;
use core::fmt;
use core::time::Duration;

use coqui_builder::{Describe, Inspect};
use coqui_transport::{Channel, Failure};
use guatiao::Status;
use guatiao::library::ProviderError;

use crate::kind::TransportInfo;

/// A transport, as its own crate has it: settings it describes, and the two things it does.
///
/// Nothing here is guatiao's: the implementation names `coqui-transport` channels and the transport's own
/// types. [`transport_provider!`](crate::transport_provider) turns it into a provider of the kind, in the
/// crate that holds the implementation.
pub trait Driver: 'static {
    /// The transport's settings: every setting of every side, described by `coqui-builder`. They are the
    /// provider's configuration, whose schema the host checks before an instance is built.
    type Settings: Describe + Inspect + Default + Clone + fmt::Debug + Send + Sync + 'static;

    /// What the transport is, as `settings` configure it.
    fn describe(settings: &Self::Settings) -> TransportInfo;

    /// Connects to `endpoint`, waiting at most `timeout`.
    ///
    /// # Errors
    ///
    /// [`Refusal::Endpoint`] for text the transport cannot read, [`Refusal::Unsupported`] when it cannot dial,
    /// [`Refusal::Unreachable`] when nothing answered.
    fn dial(
        settings: &Self::Settings,
        endpoint: &str,
        timeout: Duration,
    ) -> Result<Box<dyn Channel>, DriverError>;

    /// Starts listening on `endpoint`.
    ///
    /// # Errors
    ///
    /// As [`dial`](Self::dial), and [`Refusal::Unreachable`] when the endpoint is taken.
    fn listen(settings: &Self::Settings, endpoint: &str) -> Result<Box<dyn Acceptor>, DriverError>;
}

/// A connection an [`Acceptor`] took, and where it came from: an address as text, empty when there is none.
pub type Accepted = (Box<dyn Channel>, String);

/// A bound endpoint that takes connections.
pub trait Acceptor: Send + 'static {
    /// The endpoint a peer dials to reach this one: the real port when `0` was asked.
    fn address(&self) -> String;

    /// The next connection and where it came from (empty when there is no address), waiting at most `timeout`;
    /// `None` when nobody came. A peer that fails its handshake is not an answer: the wait goes on or is
    /// reported as `None`.
    ///
    /// # Errors
    ///
    /// Why the endpoint can take no more connections.
    fn accept(&self, timeout: Duration) -> Result<Option<Accepted>, DriverError>;
}

/// Why a transport refused to dial or listen, or stopped listening.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Refusal {
    /// The endpoint text is not one the transport reads: `GUATIAO_ERR_BAD_VALUE`.
    Endpoint,
    /// The transport does not do this: `GUATIAO_ERR_UNSUPPORTED`.
    Unsupported,
    /// Nothing answered, the endpoint is taken, or the path is closed to this user: `GUATIAO_ERR_GONE`.
    Unreachable,
    /// Anything else: `GUATIAO_ERR_INTERNAL`.
    Failed,
}

impl Refusal {
    /// The status a provider answers with.
    #[must_use]
    pub const fn status(self) -> Status {
        match self {
            Self::Endpoint => Status::GUATIAO_ERR_BAD_VALUE,
            Self::Unsupported => Status::GUATIAO_ERR_UNSUPPORTED,
            Self::Unreachable => Status::GUATIAO_ERR_GONE,
            Self::Failed => Status::GUATIAO_ERR_INTERNAL,
        }
    }
}

/// A refusal and its reason, which holds no endpoint, address or credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriverError {
    refusal: Refusal,
    reason: String,
}

impl DriverError {
    /// A refusal for `reason`.
    #[must_use]
    pub fn new(refusal: Refusal, reason: impl Into<String>) -> Self {
        Self {
            refusal,
            reason: reason.into(),
        }
    }

    /// Why, as a kind.
    #[must_use]
    pub const fn refusal(&self) -> Refusal {
        self.refusal
    }

    /// Why, in words.
    #[must_use]
    pub fn reason(&self) -> &str {
        &self.reason
    }
}

impl fmt::Display for DriverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.reason)
    }
}

impl core::error::Error for DriverError {}

impl From<DriverError> for ProviderError {
    fn from(error: DriverError) -> Self {
        Self::new(error.refusal.status(), &error.reason)
    }
}

impl From<Failure> for DriverError {
    fn from(failure: Failure) -> Self {
        let refusal = match failure {
            Failure::Unsupported => Refusal::Unsupported,
            Failure::Timeout
            | Failure::PathLost
            | Failure::NoProgress
            | Failure::Certificate
            | Failure::PeerProof
            | Failure::Permission
            | Failure::Cancelled => Refusal::Unreachable,
            _ => Refusal::Failed,
        };
        Self::new(refusal, failure.to_string())
    }
}
