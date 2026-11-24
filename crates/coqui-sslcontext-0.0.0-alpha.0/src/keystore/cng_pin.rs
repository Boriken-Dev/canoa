//! A smart card's PIN set on a CNG key.
//!
//! `rustls-cng`'s `NCryptKey::set_pin` passes the PIN's UTF-8 length as the byte count of a UTF-16
//! buffer, so the PIN reaches the card cut short and counts as a wrong attempt against its retry
//! limit. This file sets the property with the full length.

use alloc::vec::Vec;

use rustls_cng::key::NCryptKey;
use windows_sys::Win32::Security::Cryptography::{NCRYPT_PIN_PROPERTY, NCryptSetProperty};
use zeroize::Zeroizing;

use crate::error::{Error, Reason};
use crate::secret::Secret;

/// Gives `key` the PIN for its next operations (`NCRYPT_PIN_PROPERTY`).
///
/// # Errors
///
/// [`Error::Keystore`] with the `HRESULT` when the provider refuses the property.
pub(super) fn set_pin(key: &NCryptKey, pin: &Secret) -> Result<(), Error> {
    if pin.expose().contains('\0') {
        return Err(Error::Credential(Reason::new(
            "the PIN holds a NUL character",
        )));
    }
    // Sized up front so the buffer is never reallocated, which would leave a copy behind; it is
    // wiped when it goes out of scope, on every path.
    let mut wide: Zeroizing<Vec<u16>> = Zeroizing::new(Vec::with_capacity(pin.expose().len() + 1));
    wide.extend(pin.expose().encode_utf16());
    wide.push(0);
    let bytes = u32::try_from(wide.len() * 2)
        .map_err(|_| Error::Credential(Reason::new("the PIN is too long")))?;
    // SAFETY: `key.inner()` is a live key handle for as long as `key` is borrowed; `wide` is a
    // NUL-terminated UTF-16 buffer of exactly `bytes` bytes that outlives the call; the property
    // name is a static NUL-terminated wide string.
    let status = unsafe {
        NCryptSetProperty(
            key.inner(),
            NCRYPT_PIN_PROPERTY,
            wide.as_ptr().cast(),
            bytes,
            0,
        )
    };
    if status == 0 {
        Ok(())
    } else {
        Err(Error::Keystore {
            code: Some(i64::from(status)),
            reason: Reason::new("the key store did not accept the PIN property"),
        })
    }
}
