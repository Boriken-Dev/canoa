//! Client certificates held by a PKCS #11 module: smart cards, hardware tokens, SoftHSM.

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use cryptoki::context::{CInitializeArgs, CInitializeFlags, Pkcs11};
use cryptoki::error::{Error as CryptokiError, RvError};
use cryptoki::mechanism::rsa::{PkcsMgfType, PkcsPssParams};
use cryptoki::mechanism::{Mechanism, MechanismType};
use cryptoki::object::{Attribute, AttributeType, CertificateType, ObjectClass, ObjectHandle};
use cryptoki::session::{Session, UserType};
use cryptoki::types::{AuthPin, Ulong};
use rustls::SignatureScheme;
use rustls::pki_types::CertificateDer;
use rustls::sign::{Signer, SigningKey};
use sha2::{Digest, Sha256, Sha384, Sha512};
use x509_cert::Certificate;
use x509_cert::der::{Decode, Encode};

use super::identity::{CertificateInfo, Identity, KeyType};
use super::select::IdentitySource;
use crate::error::{Error, Reason};
use crate::proxy::Secret;

fn tls_error(error: impl fmt::Display) -> Error {
    Error::Tls(Reason::new(alloc::format!("pkcs11: {error}")))
}

/// Where p11-kit's proxy module is installed on the common distributions.
const SYSTEM_MODULES: [&str; 6] = [
    "/usr/lib64/pkcs11/p11-kit-proxy.so",
    "/usr/lib/x86_64-linux-gnu/pkcs11/p11-kit-proxy.so",
    "/usr/lib/aarch64-linux-gnu/pkcs11/p11-kit-proxy.so",
    "/usr/lib/pkcs11/p11-kit-proxy.so",
    "/usr/lib64/p11-kit-proxy.so",
    "/usr/lib/p11-kit-proxy.so",
];

/// The certificates of a PKCS #11 module's tokens, with the private keys that go with them.
///
/// Listing opens a read-only session on each token (logging in when a PIN is given) and finds
/// the X.509 certificates that have a private key of the same `CKA_ID`. A key that needs a login
/// and no PIN was given is not listed. The key itself never leaves the token: each signature is
/// made by the module.
pub struct Pkcs11Source {
    module: PathBuf,
    pin: Option<Secret>,
    token_label: Option<String>,
    context: Mutex<Option<Pkcs11>>,
}

impl fmt::Debug for Pkcs11Source {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Pkcs11Source")
            .field("module", &self.module)
            .field("token_label", &self.token_label)
            .finish_non_exhaustive()
    }
}

/// Builds a [`Pkcs11Source`].
#[derive(Debug)]
#[must_use]
pub struct Pkcs11SourceBuilder {
    source: Pkcs11Source,
}

impl Pkcs11SourceBuilder {
    /// The user PIN, to log in to each token.
    pub fn pin(mut self, pin: Secret) -> Self {
        self.source.pin = Some(pin);
        self
    }

    /// Only the token with this label.
    pub fn token_label(mut self, label: impl Into<String>) -> Self {
        self.source.token_label = Some(label.into());
        self
    }

    /// The source. The module is loaded at the first listing.
    pub fn build(self) -> Pkcs11Source {
        self.source
    }
}

impl Pkcs11Source {
    /// Starts a source over the PKCS #11 module at `module`.
    pub fn builder(module: impl Into<PathBuf>) -> Pkcs11SourceBuilder {
        Pkcs11SourceBuilder {
            source: Self {
                module: module.into(),
                pin: None,
                token_label: None,
                context: Mutex::new(None),
            },
        }
    }

    /// The system's default module, p11-kit's proxy, when it is installed.
    pub fn system_default() -> Option<Self> {
        SYSTEM_MODULES
            .iter()
            .find(|path| Path::new(path).exists())
            .map(|path| Self::builder(*path).build())
    }

    fn context(&self) -> Result<Pkcs11, Error> {
        let mut slot = self
            .context
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(context) = slot.as_ref() {
            return Ok(context.clone());
        }
        let context = Pkcs11::new(&self.module).map_err(tls_error)?;
        match context.initialize(CInitializeArgs::new(CInitializeFlags::OS_LOCKING_OK)) {
            Ok(()) | Err(CryptokiError::Pkcs11(RvError::CryptokiAlreadyInitialized, _)) => {}
            Err(error) => return Err(tls_error(error)),
        }
        *slot = Some(context.clone());
        Ok(context)
    }
}

impl IdentitySource for Pkcs11Source {
    fn identities(&self) -> Result<Vec<Identity>, Error> {
        let context = self.context()?;
        let mut found = Vec::new();
        for slot in context.get_slots_with_token().map_err(tls_error)? {
            let token = context.get_token_info(slot).map_err(tls_error)?;
            if self
                .token_label
                .as_deref()
                .is_some_and(|wanted| wanted != token.label().trim())
            {
                continue;
            }
            let session = context.open_ro_session(slot).map_err(tls_error)?;
            if let Some(pin) = &self.pin {
                match session.login(UserType::User, Some(&AuthPin::new(pin.expose().into()))) {
                    Ok(()) | Err(CryptokiError::Pkcs11(RvError::UserAlreadyLoggedIn, _)) => {}
                    Err(error) => return Err(tls_error(error)),
                }
            }
            let label = alloc::format!("pkcs11:{}", token.label().trim());
            let session = Arc::new(Mutex::new(session));
            found.extend(token_identities(&session, &label));
        }
        Ok(found)
    }
}

fn token_identities(session: &Arc<Mutex<Session>>, label: &str) -> Vec<Identity> {
    let guard = session
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Ok(handles) = guard.find_objects(&[
        Attribute::Class(ObjectClass::CERTIFICATE),
        Attribute::CertificateType(CertificateType::X_509),
    ]) else {
        return Vec::new();
    };
    // Every certificate of the token: the leaves, and the CAs a chain is built from.
    let mut certificates: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    for handle in handles {
        let Ok(attributes) =
            guard.get_attributes(handle, &[AttributeType::Value, AttributeType::Id])
        else {
            continue;
        };
        let (mut value, mut id) = (None, Vec::new());
        for attribute in attributes {
            match attribute {
                Attribute::Value(bytes) => value = Some(bytes),
                Attribute::Id(bytes) => id = bytes,
                _ => {}
            }
        }
        if let Some(value) = value {
            certificates.push((value, id));
        }
    }
    let mut identities = Vec::new();
    for (der, id) in &certificates {
        if id.is_empty() {
            continue;
        }
        let Ok(keys) = guard.find_objects(&[
            Attribute::Class(ObjectClass::PRIVATE_KEY),
            Attribute::Id(id.clone()),
        ]) else {
            continue;
        };
        let Some(key_handle) = keys.first().copied() else {
            continue;
        };
        let chain = build_chain(der, &certificates);
        let Ok(info) = CertificateInfo::from_chain(&chain) else {
            continue;
        };
        let key = Arc::new(Pkcs11Key {
            session: Arc::clone(session),
            handle: key_handle,
            key_type: info.key_type(),
        });
        if let Ok(identity) = Identity::new(chain, key, label) {
            identities.push(identity);
        }
    }
    identities
}

/// `leaf` followed by the certificates of `all` that issued it, one above the other.
fn build_chain(leaf: &[u8], all: &[(Vec<u8>, Vec<u8>)]) -> Vec<CertificateDer<'static>> {
    let mut chain = alloc::vec![CertificateDer::from(leaf.to_vec())];
    let mut current = leaf.to_vec();
    for _ in 0..8 {
        let Ok(certificate) = Certificate::from_der(&current) else {
            break;
        };
        let (Ok(issuer), Ok(subject)) = (
            certificate.tbs_certificate().issuer().to_der(),
            certificate.tbs_certificate().subject().to_der(),
        ) else {
            break;
        };
        if issuer == subject {
            break;
        }
        let parent = all.iter().map(|(der, _)| der).find(|candidate| {
            Certificate::from_der(candidate)
                .ok()
                .and_then(|parent| parent.tbs_certificate().subject().to_der().ok())
                .is_some_and(|name| name == issuer)
        });
        let Some(parent) = parent else { break };
        chain.push(CertificateDer::from(parent.clone()));
        current.clone_from(parent);
    }
    chain
}

/// A private key on a token, signing through the module's session.
struct Pkcs11Key {
    session: Arc<Mutex<Session>>,
    handle: ObjectHandle,
    key_type: KeyType,
}

impl fmt::Debug for Pkcs11Key {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Pkcs11Key")
            .field("key_type", &self.key_type)
            .finish_non_exhaustive()
    }
}

impl SigningKey for Pkcs11Key {
    fn choose_scheme(&self, offered: &[SignatureScheme]) -> Option<Box<dyn Signer>> {
        let scheme = self
            .key_type
            .schemes()
            .iter()
            .copied()
            .find(|scheme| offered.contains(scheme) && supported(*scheme))?;
        Some(Box::new(Pkcs11Signer {
            session: Arc::clone(&self.session),
            handle: self.handle,
            scheme,
        }))
    }

    fn algorithm(&self) -> rustls::SignatureAlgorithm {
        self.key_type.algorithm()
    }
}

fn supported(scheme: SignatureScheme) -> bool {
    !matches!(scheme, SignatureScheme::ED25519)
}

struct Pkcs11Signer {
    session: Arc<Mutex<Session>>,
    handle: ObjectHandle,
    scheme: SignatureScheme,
}

impl fmt::Debug for Pkcs11Signer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Pkcs11Signer")
            .field("scheme", &self.scheme)
            .finish_non_exhaustive()
    }
}

impl Signer for Pkcs11Signer {
    fn sign(&self, message: &[u8]) -> Result<Vec<u8>, rustls::Error> {
        let pss = |hash: MechanismType, mgf: PkcsMgfType, bytes: u8| PkcsPssParams {
            hash_alg: hash,
            mgf,
            s_len: Ulong::new(bytes.into()),
        };
        let (mechanism, data, ecdsa): (Mechanism<'_>, Vec<u8>, bool) = match self.scheme {
            SignatureScheme::RSA_PKCS1_SHA256 => {
                (Mechanism::Sha256RsaPkcs, message.to_vec(), false)
            }
            SignatureScheme::RSA_PKCS1_SHA384 => {
                (Mechanism::Sha384RsaPkcs, message.to_vec(), false)
            }
            SignatureScheme::RSA_PKCS1_SHA512 => {
                (Mechanism::Sha512RsaPkcs, message.to_vec(), false)
            }
            SignatureScheme::RSA_PSS_SHA256 => (
                Mechanism::Sha256RsaPkcsPss(pss(
                    MechanismType::SHA256,
                    PkcsMgfType::MGF1_SHA256,
                    32,
                )),
                message.to_vec(),
                false,
            ),
            SignatureScheme::RSA_PSS_SHA384 => (
                Mechanism::Sha384RsaPkcsPss(pss(
                    MechanismType::SHA384,
                    PkcsMgfType::MGF1_SHA384,
                    48,
                )),
                message.to_vec(),
                false,
            ),
            SignatureScheme::RSA_PSS_SHA512 => (
                Mechanism::Sha512RsaPkcsPss(pss(
                    MechanismType::SHA512,
                    PkcsMgfType::MGF1_SHA512,
                    64,
                )),
                message.to_vec(),
                false,
            ),
            SignatureScheme::ECDSA_NISTP256_SHA256 => {
                (Mechanism::Ecdsa, Sha256::digest(message).to_vec(), true)
            }
            SignatureScheme::ECDSA_NISTP384_SHA384 => {
                (Mechanism::Ecdsa, Sha384::digest(message).to_vec(), true)
            }
            SignatureScheme::ECDSA_NISTP521_SHA512 => {
                (Mechanism::Ecdsa, Sha512::digest(message).to_vec(), true)
            }
            _ => {
                return Err(rustls::Error::General(
                    "a signature scheme this token cannot make".into(),
                ));
            }
        };
        let session = self
            .session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let signature = session
            .sign(&mechanism, self.handle, &data)
            .map_err(|error| {
                rustls::Error::General(alloc::format!("the token refused to sign: {error}"))
            })?;
        if ecdsa {
            p1363_to_der(&signature)
                .ok_or_else(|| rustls::Error::General("the token's signature is malformed".into()))
        } else {
            Ok(signature)
        }
    }

    fn scheme(&self) -> SignatureScheme {
        self.scheme
    }
}

/// An ECDSA signature as the token makes it, `r || s`, in the DER `SEQUENCE` TLS carries.
fn p1363_to_der(signature: &[u8]) -> Option<Vec<u8>> {
    if signature.is_empty() || !signature.len().is_multiple_of(2) {
        return None;
    }
    let (r, s) = signature.split_at(signature.len() / 2);
    fn integer(bytes: &[u8]) -> Vec<u8> {
        let trimmed = &bytes[bytes
            .iter()
            .position(|byte| *byte != 0)
            .unwrap_or(bytes.len() - 1)..];
        let mut body = Vec::with_capacity(trimmed.len() + 1);
        if trimmed[0] & 0x80 != 0 {
            body.push(0);
        }
        body.extend_from_slice(trimmed);
        let mut out = alloc::vec![0x02];
        out.extend(length(body.len()));
        out.extend(body);
        out
    }
    fn length(length: usize) -> Vec<u8> {
        if length < 0x80 {
            alloc::vec![length as u8]
        } else {
            alloc::vec![0x81, length as u8]
        }
    }
    let mut body = integer(r);
    body.extend(integer(s));
    let mut out = alloc::vec![0x30];
    out.extend(length(body.len()));
    out.extend(body);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_raw_ecdsa_signature_becomes_a_der_sequence_of_two_integers() {
        // r = 0x01..., s = 0x80... (needs a leading zero), both 4 bytes.
        let der = p1363_to_der(&[0x01, 0x02, 0x03, 0x04, 0x80, 0x00, 0x00, 0x01]).unwrap();
        assert_eq!(
            der,
            [
                0x30, 0x0d, 0x02, 0x04, 0x01, 0x02, 0x03, 0x04, 0x02, 0x05, 0x00, 0x80, 0x00, 0x00,
                0x01
            ]
        );
    }

    #[test]
    fn leading_zeros_of_an_integer_are_trimmed_and_an_odd_length_is_refused() {
        let der = p1363_to_der(&[0x00, 0x00, 0x05, 0x00, 0x00, 0x07]).unwrap();
        assert_eq!(der, [0x30, 0x06, 0x02, 0x01, 0x05, 0x02, 0x01, 0x07]);
        assert!(p1363_to_der(&[1, 2, 3]).is_none());
        assert!(p1363_to_der(&[]).is_none());
    }

    #[test]
    fn a_module_that_does_not_exist_is_an_error_not_a_panic() {
        let source = Pkcs11Source::builder("/no/such/module.so").build();
        assert!(source.identities().is_err());
    }

    #[test]
    fn the_secret_token_pin_is_not_in_debug() {
        let source = Pkcs11Source::builder("/m.so")
            .pin(Secret::new("1234"))
            .build();
        assert!(!alloc::format!("{source:?}").contains("1234"));
    }
}
