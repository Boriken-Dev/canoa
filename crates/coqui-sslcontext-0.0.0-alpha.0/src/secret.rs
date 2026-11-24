//! A secret text.

use alloc::string::String;
use core::fmt;

use zeroize::Zeroize;

/// A secret text (a PIN, a password): it prints as `***`, is wiped from memory when dropped and
/// is read only through [`Secret::expose`].
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Secret(String);

impl Secret {
    /// Wraps `value`.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The secret itself.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("***")
    }
}

/// `Display` masks too, so a secret formatted by mistake does not leak.
impl fmt::Display for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("***")
    }
}

impl From<&str> for Secret {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for Secret {
    fn from(value: String) -> Self {
        Self(value)
    }
}
