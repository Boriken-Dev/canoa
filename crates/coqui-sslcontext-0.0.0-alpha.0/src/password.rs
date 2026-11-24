//! Where the password of an encrypted key or a PKCS#12 file comes from.

use alloc::sync::Arc;
#[cfg(feature = "encrypted-keys")]
use alloc::vec::Vec;
use core::fmt;
use std::path::PathBuf;

use zeroize::Zeroizing;

use crate::error::{Error, Reason};
use crate::secret::Secret;

/// Where a password comes from. It is consulted only when the key or file turns out to be
/// encrypted.
#[derive(Clone, Default)]
#[non_exhaustive]
pub enum KeyPassword {
    /// No password: an encrypted key is an error.
    #[default]
    None,
    /// This password.
    Secret(Secret),
    /// The first line of this file, without its line ending.
    File(PathBuf),
    /// Ask this function; `None` declines.
    Callback(Arc<dyn Fn() -> Option<Secret> + Send + Sync>),
}

impl KeyPassword {
    /// A password callback from a closure.
    pub fn callback(ask: impl Fn() -> Option<Secret> + Send + Sync + 'static) -> Self {
        Self::Callback(Arc::new(ask))
    }

    /// The password, or `None` when there is none to give.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] when the password file cannot be read; [`Error::Credential`] when it is not
    /// UTF-8 text.
    pub fn resolve(&self) -> Result<Option<Secret>, Error> {
        match self {
            Self::None => Ok(None),
            Self::Secret(secret) => Ok(Some(secret.clone())),
            Self::File(path) => {
                let bytes = Zeroizing::new(std::fs::read(path)?);
                let line = bytes.split(|byte| *byte == b'\n').next().unwrap_or(&[]);
                let line = line.strip_suffix(b"\r").unwrap_or(line);
                let text = core::str::from_utf8(line)
                    .map_err(|_| Error::Credential(Reason::new("the password file is not text")))?;
                Ok(Some(Secret::new(text)))
            }
            Self::Callback(ask) => Ok(ask()),
        }
    }

    /// The password as bytes, for formats that take bytes.
    #[cfg(feature = "encrypted-keys")]
    pub(crate) fn resolve_bytes(&self) -> Result<Option<Zeroizing<Vec<u8>>>, Error> {
        Ok(self
            .resolve()?
            .map(|secret| Zeroizing::new(secret.expose().as_bytes().to_vec())))
    }
}

impl fmt::Debug for KeyPassword {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => formatter.write_str("KeyPassword::None"),
            Self::Secret(_) => formatter.write_str("KeyPassword::Secret(***)"),
            Self::File(path) => formatter
                .debug_tuple("KeyPassword::File")
                .field(path)
                .finish(),
            Self::Callback(_) => formatter.write_str("KeyPassword::Callback(..)"),
        }
    }
}

impl From<Secret> for KeyPassword {
    fn from(secret: Secret) -> Self {
        Self::Secret(secret)
    }
}

impl From<&str> for KeyPassword {
    fn from(password: &str) -> Self {
        Self::Secret(Secret::new(password))
    }
}
