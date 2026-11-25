//! Shared [`ProviderError`] constructors for Canoa's Guatiao kind implementations.

use coqui_acp_api::guatiao::Status;
use coqui_acp_api::guatiao::library::ProviderError;

/// Error with status `GUATIAO_ERR_UNSUPPORTED`: the provider does not do what was asked.
#[must_use]
pub fn unsupported(why: impl AsRef<str>) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_UNSUPPORTED, why.as_ref())
}

/// Error with status `GUATIAO_ERR_GONE`: what the provider needs is locked, offline or unreachable.
#[must_use]
pub fn unavailable(why: impl AsRef<str>) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_GONE, why.as_ref())
}

/// Error with status `GUATIAO_ERR_BAD_VALUE`: a name, value or setting the provider cannot use.
#[must_use]
pub fn bad_value(why: impl AsRef<str>) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, why.as_ref())
}

/// Error with status `GUATIAO_ERR_INTERNAL`: the provider failed internally or read malformed data.
#[must_use]
pub fn internal(why: impl AsRef<str>) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_INTERNAL, why.as_ref())
}

/// Error with status `GUATIAO_ERR_BUSY`: what the caller read has changed since, so its write was not made. Read again and retry.
#[must_use]
pub fn changed(why: impl AsRef<str>) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_BUSY, why.as_ref())
}
