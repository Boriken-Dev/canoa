//! A client identity: a certificate chain and the key that signs for it.

use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use std::time::SystemTime;

use rustls::SignatureScheme;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::sign::{CertifiedKey, SigningKey};
use sha2::{Digest, Sha256};
use x509_cert::Certificate;
use x509_cert::der::asn1::ObjectIdentifier;
use x509_cert::der::{Decode, Encode};
use x509_cert::ext::pkix::{ExtendedKeyUsage, KeyUsage};

use crate::error::{Error, Reason};

const RSA_ENCRYPTION: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.1");
const EC_PUBLIC_KEY: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.2.1");
const ED25519: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.101.112");
const SECP256R1: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.3.1.7");
const SECP384R1: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.132.0.34");
const SECP521R1: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.132.0.35");
const CLIENT_AUTH: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.6.1.5.5.7.3.2");
const ANY_EXTENDED_KEY_USAGE: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.29.37.0");

/// The kind of key a certificate holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum KeyType {
    /// An RSA key.
    Rsa,
    /// An elliptic-curve key on P-256.
    EcP256,
    /// An elliptic-curve key on P-384.
    EcP384,
    /// An elliptic-curve key on P-521.
    EcP521,
    /// An Ed25519 key.
    Ed25519,
}

impl KeyType {
    /// The signature schemes a key of this type can sign with, most preferred first.
    pub fn schemes(self) -> &'static [SignatureScheme] {
        match self {
            Self::Rsa => &[
                SignatureScheme::RSA_PSS_SHA256,
                SignatureScheme::RSA_PSS_SHA384,
                SignatureScheme::RSA_PSS_SHA512,
                SignatureScheme::RSA_PKCS1_SHA256,
                SignatureScheme::RSA_PKCS1_SHA384,
                SignatureScheme::RSA_PKCS1_SHA512,
            ],
            Self::EcP256 => &[SignatureScheme::ECDSA_NISTP256_SHA256],
            Self::EcP384 => &[SignatureScheme::ECDSA_NISTP384_SHA384],
            Self::EcP521 => &[SignatureScheme::ECDSA_NISTP521_SHA512],
            Self::Ed25519 => &[SignatureScheme::ED25519],
        }
    }

    /// The `rustls` signature algorithm of a key of this type.
    pub fn algorithm(self) -> rustls::SignatureAlgorithm {
        match self {
            Self::Rsa => rustls::SignatureAlgorithm::RSA,
            Self::EcP256 | Self::EcP384 | Self::EcP521 => rustls::SignatureAlgorithm::ECDSA,
            Self::Ed25519 => rustls::SignatureAlgorithm::ED25519,
        }
    }

    /// Whether a server that offers `offered` can verify a signature of this key.
    pub fn usable_with(self, offered: &[SignatureScheme]) -> bool {
        self.schemes().iter().any(|scheme| offered.contains(scheme))
    }
}

/// What a certificate says about itself, as a browser reads it to choose among several.
#[derive(Clone, PartialEq, Eq)]
pub struct CertificateInfo {
    subject: String,
    issuer: String,
    not_before: SystemTime,
    not_after: SystemTime,
    key_type: KeyType,
    client_auth: bool,
    fingerprint: [u8; 32],
    /// DER names the server may name as acceptable CAs: the leaf's issuer, and every subject and
    /// issuer in the rest of the chain.
    names: Vec<Vec<u8>>,
}

impl fmt::Debug for CertificateInfo {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CertificateInfo")
            .field("subject", &self.subject)
            .field("issuer", &self.issuer)
            .field("key_type", &self.key_type)
            .finish_non_exhaustive()
    }
}

impl CertificateInfo {
    /// Reads the leaf of `chain` (the first certificate); the rest give the CA names.
    ///
    /// # Errors
    ///
    /// [`Error::Tls`] when the chain is empty or a certificate does not decode.
    pub fn from_chain(chain: &[CertificateDer<'_>]) -> Result<Self, Error> {
        let bad = |what: &'static str| Error::Tls(Reason::new(what));
        let leaf_der = chain
            .first()
            .ok_or_else(|| bad("an identity needs a certificate"))?;
        let leaf = Certificate::from_der(leaf_der.as_ref())
            .map_err(|_| bad("a certificate does not decode"))?;
        let tbs = leaf.tbs_certificate();
        let key = tbs.subject_public_key_info();
        let key_type = if key.algorithm.oid == RSA_ENCRYPTION {
            KeyType::Rsa
        } else if key.algorithm.oid == ED25519 {
            KeyType::Ed25519
        } else if key.algorithm.oid == EC_PUBLIC_KEY {
            let curve = key
                .algorithm
                .parameters
                .as_ref()
                .and_then(|parameters| parameters.decode_as::<ObjectIdentifier>().ok())
                .ok_or_else(|| bad("an EC key names no curve"))?;
            match curve {
                SECP256R1 => KeyType::EcP256,
                SECP384R1 => KeyType::EcP384,
                SECP521R1 => KeyType::EcP521,
                _ => return Err(bad("an EC key is on an unsupported curve")),
            }
        } else {
            return Err(bad("a key is of an unsupported type"));
        };
        let (mut extended, mut usage) = (None, None);
        if let Some(extensions) = tbs.extensions() {
            for extension in extensions {
                if extension.extn_id
                    == <ExtendedKeyUsage as x509_cert::der::oid::AssociatedOid>::OID
                {
                    extended = ExtendedKeyUsage::from_der(extension.extn_value.as_bytes()).ok();
                } else if extension.extn_id == <KeyUsage as x509_cert::der::oid::AssociatedOid>::OID
                {
                    usage = KeyUsage::from_der(extension.extn_value.as_bytes()).ok();
                }
            }
        }
        // What a browser offers: no extended key usage, or clientAuth, or anyExtendedKeyUsage;
        // and digitalSignature when a key usage is present.
        let client_auth = extended.as_ref().is_none_or(|usages| {
            usages
                .0
                .iter()
                .any(|oid| *oid == CLIENT_AUTH || *oid == ANY_EXTENDED_KEY_USAGE)
        }) && usage.as_ref().is_none_or(KeyUsage::digital_signature);
        let mut names = Vec::new();
        names.push(
            tbs.issuer()
                .to_der()
                .map_err(|_| bad("a name does not encode"))?,
        );
        for der in chain.iter().skip(1) {
            if let Ok(certificate) = Certificate::from_der(der.as_ref()) {
                for name in [
                    certificate.tbs_certificate().subject(),
                    certificate.tbs_certificate().issuer(),
                ] {
                    if let Ok(encoded) = name.to_der() {
                        names.push(encoded);
                    }
                }
            }
        }
        names.sort();
        names.dedup();
        Ok(Self {
            subject: tbs.subject().to_string(),
            issuer: tbs.issuer().to_string(),
            not_before: tbs.validity().not_before.to_system_time(),
            not_after: tbs.validity().not_after.to_system_time(),
            key_type,
            client_auth,
            fingerprint: Sha256::digest(leaf_der.as_ref()).into(),
            names,
        })
    }

    /// The subject, as an RFC 4514 string.
    pub fn subject(&self) -> &str {
        &self.subject
    }

    /// The issuer, as an RFC 4514 string.
    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    /// The end of the validity period.
    pub fn not_after(&self) -> SystemTime {
        self.not_after
    }

    /// The kind of key.
    pub fn key_type(&self) -> KeyType {
        self.key_type
    }

    /// Whether the certificate may authenticate a client: no extended key usage, or `clientAuth`
    /// or `anyExtendedKeyUsage`; and `digitalSignature` when a key usage is present.
    pub fn client_auth(&self) -> bool {
        self.client_auth
    }

    /// The SHA-256 digest of the certificate, which identifies it.
    pub fn fingerprint(&self) -> &[u8; 32] {
        &self.fingerprint
    }

    /// Whether `now` is within the validity period.
    pub fn valid_at(&self, now: SystemTime) -> bool {
        self.not_before <= now && now <= self.not_after
    }

    /// Whether the certificate was issued under one of `authorities` (DER names): the leaf's issuer
    /// or any subject or issuer in its chain. An empty list names no restriction.
    pub fn issued_under(&self, authorities: &[Vec<u8>]) -> bool {
        authorities.is_empty() || authorities.iter().any(|name| self.names.contains(name))
    }
}

/// A certificate chain and the key that signs for it.
#[derive(Clone)]
pub struct Identity {
    chain: Vec<CertificateDer<'static>>,
    key: Arc<dyn SigningKey>,
    info: CertificateInfo,
    label: String,
}

impl fmt::Debug for Identity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Identity")
            .field("label", &self.label)
            .field("info", &self.info)
            .finish()
    }
}

impl Identity {
    /// An identity from a chain (the leaf first) and a key that signs for the leaf.
    ///
    /// # Errors
    ///
    /// [`Error::Tls`] when the chain is empty or the leaf does not decode.
    pub fn new(
        chain: Vec<CertificateDer<'static>>,
        key: Arc<dyn SigningKey>,
        label: impl Into<String>,
    ) -> Result<Self, Error> {
        let info = CertificateInfo::from_chain(&chain)?;
        Ok(Self {
            chain,
            key,
            info,
            label: label.into(),
        })
    }

    /// An identity from PEM text: the leaf and then its chain in `certificates`, and the private
    /// key (PKCS#8, PKCS#1 or SEC1, unencrypted) in `key`.
    ///
    /// # Errors
    ///
    /// [`Error::Tls`] for text that holds no certificate, no key, or a key `rustls` cannot sign with.
    pub fn from_pem(certificates: &[u8], key: &[u8]) -> Result<Self, Error> {
        let chain: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(certificates)
            .collect::<Result<_, _>>()
            .map_err(|_| Error::Tls(Reason::new("the certificate text is not PEM")))?;
        let key = PrivateKeyDer::from_pem_slice(key)
            .map_err(|_| Error::Tls(Reason::new("the key text is not PEM")))?;
        let key = rustls::crypto::ring::sign::any_supported_type(&key)
            .map_err(|_| Error::Tls(Reason::new("the key is not one rustls can sign with")))?;
        Self::new(chain, key, "pem")
    }

    /// What the leaf certificate says about itself.
    pub fn info(&self) -> &CertificateInfo {
        &self.info
    }

    /// A short name for where the identity came from, for diagnostics.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// The certificate chain, the leaf first.
    pub fn chain(&self) -> &[CertificateDer<'static>] {
        &self.chain
    }

    pub(crate) fn certified_key(&self) -> Arc<CertifiedKey> {
        Arc::new(CertifiedKey::new(self.chain.clone(), Arc::clone(&self.key)))
    }
}
