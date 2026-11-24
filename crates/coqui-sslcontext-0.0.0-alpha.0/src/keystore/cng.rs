//! Windows certificate stores: keys in CNG key storage providers (software, TPM, smart cards).

use alloc::boxed::Box;
use alloc::collections::BTreeSet;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use std::sync::Mutex;

use rustls::SignatureScheme;
use rustls::pki_types::CertificateDer;
use rustls::sign::{Signer, SigningKey};
use rustls_cng::cert::CertContext;
use rustls_cng::signer::CngSigningKey;
use rustls_cng::store::{CertStore, CertStoreType};

use super::cng_pin;
use crate::error::{Error, Reason};
use crate::identity::{CertificateInfo, Identity};
use crate::pin::{KeystoreKind, Pin, PinAnswer, PinPurpose, PinRequest};
use crate::select::IdentitySource;

/// The certificates of a Windows store that have a private key.
///
/// A key is opened silently first. When it needs a PIN (a smart card), the source's [`Pin`] decides:
/// a PIN is set on the key, [`Pin::Keystore`] lets Windows show its own dialog, and no PIN fails
/// the signature at once. A PIN the key rejected is never sent again.
#[derive(Debug)]
pub struct CngSource {
    store: Arc<CertStore>,
    label: String,
    pin: Pin,
    rejected: Arc<Mutex<BTreeSet<[u8; 32]>>>,
}

impl CngSource {
    /// The current user's store `name` (`MY`, the personal store, is where a person's client
    /// certificates are; smart-card certificates appear there while the card is inserted).
    ///
    /// # Errors
    ///
    /// [`Error::Tls`] when the store cannot be opened.
    pub fn current_user(name: &str) -> Result<Self, Error> {
        Self::open(CertStoreType::CurrentUser, name)
    }

    /// The local machine's store `name`.
    ///
    /// # Errors
    ///
    /// [`Error::Tls`] when the store cannot be opened.
    pub fn local_machine(name: &str) -> Result<Self, Error> {
        Self::open(CertStoreType::LocalMachine, name)
    }

    fn open(kind: CertStoreType, name: &str) -> Result<Self, Error> {
        let store = CertStore::open(kind, name)
            .map_err(|error| Error::Tls(Reason::new(error.to_string())))?;
        Ok(Self::from_store(
            Arc::new(store),
            alloc::format!("cng:{name}"),
        ))
    }

    /// Certificates in a store opened elsewhere, such as one imported from a PKCS #12 file.
    pub fn from_store(store: Arc<CertStore>, label: impl Into<String>) -> Self {
        Self {
            store,
            label: label.into(),
            pin: Pin::Keystore,
            rejected: Arc::new(Mutex::new(BTreeSet::new())),
        }
    }

    /// Where a smart card key's PIN comes from (default [`Pin::Keystore`]: Windows asks).
    pub fn pin(mut self, pin: impl Into<Pin>) -> Self {
        self.pin = pin.into();
        self
    }
}

impl IdentitySource for CngSource {
    fn identities(&self) -> Result<Vec<Identity>, Error> {
        let contexts = self
            .store
            .find_all()
            .map_err(|error| Error::Tls(Reason::new(error.to_string())))?;
        let mut found = Vec::new();
        for context in contexts {
            // Listing opens each key silently: a certificate with no key, or whose key needs the
            // user to open it, is not listed. A key is opened again, silent first, to sign.
            if context.acquire_key(true).is_err() {
                continue;
            }
            let Ok(chain) = context.as_chain_der() else {
                continue;
            };
            let chain: Vec<CertificateDer<'static>> =
                chain.into_iter().map(CertificateDer::from).collect();
            let Ok(info) = CertificateInfo::from_chain(&chain) else {
                continue;
            };
            let key = Arc::new(LazyCngKey {
                context,
                info,
                store: self.label.clone(),
                pin: self.pin.clone(),
                rejected: Arc::clone(&self.rejected),
                signing: Mutex::new(None),
            });
            if let Ok(identity) = Identity::new(chain, key, self.label.clone()) {
                found.push(identity);
            }
        }
        Ok(found)
    }
}

/// A CNG key opened when the first signature is asked for, so listing the store never touches a
/// smart card.
struct LazyCngKey {
    context: CertContext,
    info: CertificateInfo,
    store: String,
    pin: Pin,
    rejected: Arc<Mutex<BTreeSet<[u8; 32]>>>,
    signing: Mutex<Option<Arc<CngSigningKey>>>,
}

impl fmt::Debug for LazyCngKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LazyCngKey")
            .field("key_type", &self.info.key_type())
            .finish_non_exhaustive()
    }
}

impl LazyCngKey {
    /// Signs `message` after the silent attempt failed, with the PIN the source's policy gives.
    fn sign_with_pin(
        &self,
        scheme: SignatureScheme,
        message: &[u8],
    ) -> Result<Vec<u8>, rustls::Error> {
        let refused = |text: &str| rustls::Error::General(text.to_string());
        // Held until the signature is made, so two threads cannot each send a wrong PIN.
        let mut rejected = self
            .rejected
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let fingerprint = *self.info.fingerprint();
        if rejected.contains(&fingerprint) {
            return Err(refused("the key store rejected the PIN; it is not retried"));
        }
        let request = PinRequest::new(
            KeystoreKind::Cng,
            self.store.clone(),
            Some(self.info.clone()),
            PinPurpose::Sign,
            true,
        );
        let mut sent = false;
        let key = match self.pin.resolve(&request) {
            PinAnswer::Decline => return Err(refused("the key needs a PIN and none was given")),
            PinAnswer::Keystore => self
                .context
                .acquire_key(false)
                .map_err(|_| refused("the key could not be opened"))?,
            PinAnswer::Secret(secret) => {
                let key = self
                    .context
                    .acquire_key(true)
                    .map_err(|_| refused("the key could not be opened"))?;
                cng_pin::set_pin(&key, &secret)
                    .map_err(|_| refused("the key store did not take the PIN"))?;
                sent = true;
                key
            }
        };
        let signer = CngSigningKey::new(key)
            .map_err(|_| refused("the key could not be used"))?
            .choose_scheme(&[scheme])
            .ok_or_else(|| refused("the key cannot sign with this scheme"))?;
        match signer.sign(message) {
            Ok(signature) => {
                rejected.remove(&fingerprint);
                Ok(signature)
            }
            Err(error) => {
                // The provider reports a wrong PIN as an ordinary signing failure, so a failure
                // after a PIN was sent counts as a rejection: it is not sent again for this key.
                if sent {
                    rejected.insert(fingerprint);
                }
                Err(error)
            }
        }
    }
}

impl SigningKey for LazyCngKey {
    fn choose_scheme(&self, offered: &[SignatureScheme]) -> Option<Box<dyn Signer>> {
        let mut signing = self
            .signing
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if signing.is_none() {
            let key = self.context.acquire_key(true).ok()?;
            *signing = Some(Arc::new(CngSigningKey::new(key).ok()?));
        }
        let inner = signing.as_ref()?.choose_scheme(offered)?;
        let scheme = inner.scheme();
        Some(Box::new(CngSigner {
            inner,
            scheme,
            key: Arc::new(LazyCngKey {
                context: self.context.clone(),
                info: self.info.clone(),
                store: self.store.clone(),
                pin: self.pin.clone(),
                rejected: Arc::clone(&self.rejected),
                signing: Mutex::new(None),
            }),
        }))
    }

    fn algorithm(&self) -> rustls::SignatureAlgorithm {
        self.info.key_type().algorithm()
    }
}

/// Signs silently, and with the PIN policy when the silent attempt failed.
struct CngSigner {
    inner: Box<dyn Signer>,
    scheme: SignatureScheme,
    key: Arc<LazyCngKey>,
}

impl fmt::Debug for CngSigner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CngSigner")
            .field("scheme", &self.scheme)
            .finish_non_exhaustive()
    }
}

impl Signer for CngSigner {
    fn sign(&self, message: &[u8]) -> Result<Vec<u8>, rustls::Error> {
        match self.inner.sign(message) {
            Ok(signature) => Ok(signature),
            Err(_) => self.key.sign_with_pin(self.scheme, message),
        }
    }

    fn scheme(&self) -> SignatureScheme {
        self.scheme
    }
}
