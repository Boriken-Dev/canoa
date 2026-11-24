//! The crate's error type.

use alloc::string::String;
use core::fmt;

/// What can go wrong building a TLS configuration or reading a keystore.
///
/// A message names what failed and never quotes the input: a key, a PIN or a certificate may be
/// a secret.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A TLS configuration, a keystore or a certificate could not be used.
    #[error("tls: {0}")]
    Tls(Reason),
    /// A certificate, a key or a password could not be read or does not fit its use.
    #[error("credential: {0}")]
    Credential(Reason),
    /// No identity matched a query.
    #[error("not found: {0}")]
    NotFound(Reason),
    /// A keystore refused or failed an operation. `code` is the platform's status when it gave one
    /// (an `HRESULT`, a PKCS#11 `CKR_` value, an `OSStatus`).
    #[error("keystore: {reason}")]
    Keystore {
        /// The platform's status code, if any.
        code: Option<i64>,
        /// What failed.
        reason: Reason,
    },
    /// The keystore rejected the PIN. This crate does not try again: another attempt could lock
    /// the card.
    #[error("the keystore rejected the PIN")]
    WrongPin,
    /// A key could not sign.
    #[error("signing: {0}")]
    Signing(Reason),
    /// The request is valid but this build or platform cannot do it.
    #[error("unsupported: {0}")]
    Unsupported(Reason),
    /// A file operation failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
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
