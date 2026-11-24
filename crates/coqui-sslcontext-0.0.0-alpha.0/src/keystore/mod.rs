//! The operating system's keystores as sources of client identities.

use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::pin::Pin;
use crate::select::IdentitySource;

#[cfg(all(windows, feature = "system"))]
mod cng;
#[cfg(all(windows, feature = "system"))]
#[allow(
    unsafe_code,
    reason = "NCryptSetProperty with the PIN property; see the file"
)]
mod cng_pin;
#[cfg(all(target_os = "macos", feature = "system"))]
mod keychain;
#[cfg(feature = "pkcs11")]
mod pkcs11;

#[cfg(all(windows, feature = "system"))]
pub use cng::CngSource;
#[cfg(all(target_os = "macos", feature = "system"))]
pub use keychain::KeychainSource;
#[cfg(feature = "pkcs11")]
pub use pkcs11::{Pkcs11Source, Pkcs11SourceBuilder};

/// The operating system's keystores, as sources for [`Selector::system`](crate::Selector::system):
/// Windows CNG's `MY` store of the current user, the macOS Keychain, and, with the `pkcs11`
/// feature, PKCS#11's default module where there is one. Each asks `pin` for a smart card's PIN.
pub fn system_sources(pin: Pin) -> Vec<Box<dyn IdentitySource>> {
    let _ = pin;
    #[cfg_attr(
        not(any(
            all(windows, feature = "system"),
            all(target_os = "macos", feature = "system"),
            all(feature = "pkcs11", unix)
        )),
        expect(unused_mut, reason = "no keystore is built in for this platform")
    )]
    let mut sources: Vec<Box<dyn IdentitySource>> = Vec::new();
    #[cfg(all(windows, feature = "system"))]
    match CngSource::current_user("MY") {
        Ok(source) => sources.push(Box::new(source.pin(pin.clone()))),
        Err(error) => tracing::debug!(%error, "the certificate store could not be opened"),
    }
    #[cfg(all(target_os = "macos", feature = "system"))]
    sources.push(Box::new(KeychainSource::new().pin(pin.clone())));
    #[cfg(all(feature = "pkcs11", unix, not(target_os = "macos")))]
    if let Some(source) = pkcs11::Pkcs11Source::system_default() {
        sources.push(Box::new(source));
    }
    sources
}
