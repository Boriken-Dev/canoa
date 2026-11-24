//! Identities with software keys that count their signatures, for testing code that selects and
//! uses client certificates without a smart card.

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use core::sync::atomic::{AtomicUsize, Ordering};

use rustls::SignatureScheme;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::sign::{Signer, SigningKey};

use crate::error::{Error, Reason};
use crate::identity::Identity;
use crate::password::KeyPassword;

/// A software key that counts how many signatures it made, so a test can tell the key was used
/// (or was not).
#[derive(Clone)]
pub struct SoftwareSigningKey {
    inner: Arc<dyn SigningKey>,
    calls: Arc<AtomicUsize>,
}

impl fmt::Debug for SoftwareSigningKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SoftwareSigningKey")
            .field("calls", &self.calls())
            .finish_non_exhaustive()
    }
}

impl SoftwareSigningKey {
    /// A key from PEM (an encrypted PKCS#8 key is opened with `password`) or DER.
    ///
    /// # Errors
    ///
    /// [`Error::Credential`] for a key `rustls` cannot sign with, or a missing or wrong password.
    pub fn new(key: &[u8], password: &KeyPassword) -> Result<Self, Error> {
        let key = if key.windows(11).any(|window| window == b"-----BEGIN ") {
            crate::load::pem_private_key(key, password)?
        } else {
            PrivateKeyDer::try_from(key)
                .map_err(|_| Error::Credential(Reason::new("the key is not DER")))?
        };
        let inner = rustls::crypto::ring::sign::any_supported_type(&key).map_err(|_| {
            Error::Credential(Reason::new("the key is not one rustls can sign with"))
        })?;
        Ok(Self {
            inner,
            calls: Arc::new(AtomicUsize::new(0)),
        })
    }

    /// How many signatures the key has made.
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl SigningKey for SoftwareSigningKey {
    fn choose_scheme(&self, offered: &[SignatureScheme]) -> Option<Box<dyn Signer>> {
        Some(Box::new(CountingSigner {
            inner: self.inner.choose_scheme(offered)?,
            calls: Arc::clone(&self.calls),
        }))
    }

    fn algorithm(&self) -> rustls::SignatureAlgorithm {
        self.inner.algorithm()
    }
}

struct CountingSigner {
    inner: Box<dyn Signer>,
    calls: Arc<AtomicUsize>,
}

impl fmt::Debug for CountingSigner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CountingSigner")
            .finish_non_exhaustive()
    }
}

impl Signer for CountingSigner {
    fn sign(&self, message: &[u8]) -> Result<Vec<u8>, rustls::Error> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.sign(message)
    }

    fn scheme(&self) -> SignatureScheme {
        self.inner.scheme()
    }
}

/// An identity from PEM text whose key counts its signatures, with a handle on that count.
///
/// # Errors
///
/// [`Error::Credential`] for text that holds no certificate or a key that cannot sign.
pub fn software_identity(
    certificate: &[u8],
    key: &[u8],
    password: &KeyPassword,
) -> Result<(Identity, SoftwareSigningKey), Error> {
    let chain: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(certificate)
        .collect::<Result<_, _>>()
        .map_err(|_| Error::Credential(Reason::new("the certificate text is not PEM")))?;
    let signing = SoftwareSigningKey::new(key, password)?;
    let identity = Identity::new(chain, Arc::new(signing.clone()), "software")?;
    Ok((identity, signing))
}
