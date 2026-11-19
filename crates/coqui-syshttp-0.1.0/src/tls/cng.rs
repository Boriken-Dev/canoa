//! Windows certificate stores: keys in CNG key storage providers (software, TPM, smart cards).

use alloc::string::ToString;
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

use super::identity::{CertificateInfo, Identity, KeyType};
use super::select::IdentitySource;
use crate::error::{Error, Reason};

/// The certificates of a Windows store that have a private key.
#[derive(Debug)]
pub struct CngSource {
    store: Arc<CertStore>,
    label: String,
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
        }
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
            // A certificate with no key, or whose key needs the user to open, is not an identity.
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
                key_type: info.key_type(),
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
    key_type: KeyType,
    signing: Mutex<Option<Arc<CngSigningKey>>>,
}

impl fmt::Debug for LazyCngKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LazyCngKey")
            .field("key_type", &self.key_type)
            .finish_non_exhaustive()
    }
}

impl SigningKey for LazyCngKey {
    fn choose_scheme(&self, offered: &[SignatureScheme]) -> Option<Box<dyn Signer>> {
        let mut signing = self
            .signing
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if signing.is_none() {
            let key = self.context.acquire_key(false).ok()?;
            *signing = Some(Arc::new(CngSigningKey::new(key).ok()?));
        }
        signing.as_ref()?.choose_scheme(offered)
    }

    fn algorithm(&self) -> rustls::SignatureAlgorithm {
        self.key_type.algorithm()
    }
}
