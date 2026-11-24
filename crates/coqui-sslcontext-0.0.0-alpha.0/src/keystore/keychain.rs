//! The macOS Keychain: identities in the user's keychains and on smart cards.

use alloc::string::ToString;
use alloc::vec::Vec;
use core::fmt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rustls::SignatureScheme;
use rustls::pki_types::CertificateDer;
use rustls::sign::{Signer, SigningKey};
use security_framework::item::{ItemClass, ItemSearchOptions, Limit, Reference, SearchResult};
use security_framework::key::{Algorithm, SecKey};
use security_framework::os::macos::keychain::SecKeychain;

use crate::error::{Error, Reason};
use crate::identity::{CertificateInfo, Identity, KeyType};
use crate::pin::{KeystoreKind, Pin, PinAnswer, PinPurpose, PinRequest};
use crate::secret::Secret;
use crate::select::IdentitySource;

/// The identities (certificate and private key) the Keychain holds.
///
/// The key stays in the Keychain, the Secure Enclave or on the card: each signature is made by the
/// Security framework, which asks the person for permission where the key's access control says so.
/// Only the leaf certificate is sent; a server that needs an intermediate must have it.
///
/// A keychain file given with [`keychain`](Self::keychain) is unlocked with the source's [`Pin`]
/// (its password): a password the keychain rejected is not sent again.
#[derive(Debug, Default)]
pub struct KeychainSource {
    keychain: Option<PathBuf>,
    pin: Pin,
    rejected: Mutex<bool>,
}

impl KeychainSource {
    /// A source over every keychain the user's session searches.
    pub fn new() -> Self {
        Self::default()
    }

    /// Only the keychain file at `path`, unlocked when it is locked.
    pub fn keychain(mut self, path: impl Into<PathBuf>) -> Self {
        self.keychain = Some(path.into());
        self
    }

    /// Where the keychain's password comes from (default [`Pin::None`]: a locked keychain stays
    /// closed and is not listed; [`Pin::Keystore`] lets the system ask).
    pub fn pin(mut self, pin: impl Into<Pin>) -> Self {
        self.pin = pin.into();
        self
    }

    /// Opens the keychain file; `true` when no password was given, so it may still be locked.
    fn open(&self, path: &Path) -> Result<(SecKeychain, bool), Error> {
        // errSecAuthFailed
        const AUTH_FAILED: i32 = -25293;
        let mut keychain = SecKeychain::open(path)
            .map_err(|error| Error::Tls(Reason::new(alloc::format!("keychain: {error}"))))?;
        // Held across the unlock, so two threads cannot each send a wrong password.
        let mut rejected = self
            .rejected
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *rejected {
            return Err(Error::WrongPin);
        }
        let request = PinRequest::new(
            KeystoreKind::Keychain,
            path.display().to_string(),
            None,
            PinPurpose::List,
            true,
        );
        let password = match self.pin.resolve(&request) {
            PinAnswer::Decline => return Ok((keychain, true)),
            PinAnswer::Keystore => None,
            PinAnswer::Secret(secret) => Some(secret),
        };
        match keychain.unlock(password.as_ref().map(Secret::expose)) {
            Ok(()) => Ok((keychain, false)),
            Err(error) if error.code() == AUTH_FAILED => {
                *rejected = true;
                Err(Error::WrongPin)
            }
            Err(error) => Err(Error::Keystore {
                code: Some(i64::from(error.code())),
                reason: Reason::new("the keychain could not be unlocked"),
            }),
        }
    }
}

impl IdentitySource for KeychainSource {
    fn identities(&self) -> Result<Vec<Identity>, Error> {
        // errSecItemNotFound: a keychain with no identity is an empty list, not a failure.
        const NOT_FOUND: i32 = -25300;
        let mut options = ItemSearchOptions::new();
        options
            .class(ItemClass::identity())
            .load_refs(true)
            .limit(Limit::All);
        // A keychain left locked because no password was given must not make the system ask:
        // the search is made with user interaction off.
        let mut quiet = None;
        if let Some(path) = &self.keychain {
            let (keychain, declined) = self.open(path)?;
            if declined {
                quiet = SecKeychain::disable_user_interaction().ok();
            }
            options.keychains(&[keychain]);
        }
        let results = match options.search() {
            Ok(results) => results,
            Err(error) if error.code() == NOT_FOUND => Vec::new(),
            Err(error) => return Err(Error::Tls(Reason::new(alloc::format!("keychain: {error}")))),
        };
        drop(quiet);
        let mut found = Vec::new();
        for result in results {
            let SearchResult::Ref(Reference::Identity(identity)) = result else {
                continue;
            };
            let (Ok(certificate), Ok(key)) = (identity.certificate(), identity.private_key())
            else {
                continue;
            };
            let chain = alloc::vec![CertificateDer::from(certificate.to_der())];
            let Ok(info) = CertificateInfo::from_chain(&chain) else {
                continue;
            };
            let key = alloc::sync::Arc::new(KeychainKey {
                key,
                key_type: info.key_type(),
            });
            if let Ok(identity) = Identity::new(chain, key, "keychain") {
                found.push(identity);
            }
        }
        Ok(found)
    }
}

struct KeychainKey {
    key: SecKey,
    key_type: KeyType,
}

impl fmt::Debug for KeychainKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("KeychainKey")
            .field("key_type", &self.key_type)
            .finish_non_exhaustive()
    }
}

fn algorithm(scheme: SignatureScheme) -> Option<Algorithm> {
    Some(match scheme {
        SignatureScheme::RSA_PKCS1_SHA256 => Algorithm::RSASignatureMessagePKCS1v15SHA256,
        SignatureScheme::RSA_PKCS1_SHA384 => Algorithm::RSASignatureMessagePKCS1v15SHA384,
        SignatureScheme::RSA_PKCS1_SHA512 => Algorithm::RSASignatureMessagePKCS1v15SHA512,
        SignatureScheme::RSA_PSS_SHA256 => Algorithm::RSASignatureMessagePSSSHA256,
        SignatureScheme::RSA_PSS_SHA384 => Algorithm::RSASignatureMessagePSSSHA384,
        SignatureScheme::RSA_PSS_SHA512 => Algorithm::RSASignatureMessagePSSSHA512,
        SignatureScheme::ECDSA_NISTP256_SHA256 => Algorithm::ECDSASignatureMessageX962SHA256,
        SignatureScheme::ECDSA_NISTP384_SHA384 => Algorithm::ECDSASignatureMessageX962SHA384,
        SignatureScheme::ECDSA_NISTP521_SHA512 => Algorithm::ECDSASignatureMessageX962SHA512,
        _ => return None,
    })
}

impl SigningKey for KeychainKey {
    fn choose_scheme(&self, offered: &[SignatureScheme]) -> Option<alloc::boxed::Box<dyn Signer>> {
        let scheme = self
            .key_type
            .schemes()
            .iter()
            .copied()
            .find(|scheme| offered.contains(scheme) && algorithm(*scheme).is_some())?;
        Some(alloc::boxed::Box::new(KeychainSigner {
            key: self.key.clone(),
            scheme,
        }))
    }

    fn algorithm(&self) -> rustls::SignatureAlgorithm {
        self.key_type.algorithm()
    }
}

struct KeychainSigner {
    key: SecKey,
    scheme: SignatureScheme,
}

impl fmt::Debug for KeychainSigner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("KeychainSigner")
            .field("scheme", &self.scheme)
            .finish_non_exhaustive()
    }
}

impl Signer for KeychainSigner {
    fn sign(&self, message: &[u8]) -> Result<Vec<u8>, rustls::Error> {
        let algorithm = algorithm(self.scheme)
            .ok_or_else(|| rustls::Error::General("an unsupported signature scheme".into()))?;
        // The framework hashes the message and, for ECDSA, answers in the DER form TLS carries.
        self.key
            .create_signature(algorithm, message)
            .map_err(|error| {
                rustls::Error::General(alloc::format!("the keychain refused to sign: {error}"))
            })
    }

    fn scheme(&self) -> SignatureScheme {
        self.scheme
    }
}
