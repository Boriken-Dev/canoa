//! The macOS Keychain: identities in the user's keychains and on smart cards.

use alloc::vec::Vec;
use core::fmt;

use rustls::SignatureScheme;
use rustls::pki_types::CertificateDer;
use rustls::sign::{Signer, SigningKey};
use security_framework::item::{ItemClass, ItemSearchOptions, Limit, Reference, SearchResult};
use security_framework::key::{Algorithm, SecKey};

use super::identity::{CertificateInfo, Identity, KeyType};
use super::select::IdentitySource;
use crate::error::{Error, Reason};

/// The identities (certificate and private key) the Keychain holds.
///
/// The key stays in the Keychain, the Secure Enclave or on the card: each signature is made by the
/// Security framework, which asks the person for permission where the key's access control says so.
/// Only the leaf certificate is sent; a server that needs an intermediate must have it.
#[derive(Debug, Default)]
pub struct KeychainSource {
    _private: (),
}

impl KeychainSource {
    /// A source over every keychain the user's session searches.
    pub fn new() -> Self {
        Self::default()
    }
}

impl IdentitySource for KeychainSource {
    fn identities(&self) -> Result<Vec<Identity>, Error> {
        // errSecItemNotFound: a keychain with no identity is an empty list, not a failure.
        const NOT_FOUND: i32 = -25300;
        let results = match ItemSearchOptions::new()
            .class(ItemClass::identity())
            .load_refs(true)
            .limit(Limit::All)
            .search()
        {
            Ok(results) => results,
            Err(error) if error.code() == NOT_FOUND => Vec::new(),
            Err(error) => return Err(Error::Tls(Reason::new(alloc::format!("keychain: {error}")))),
        };
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
