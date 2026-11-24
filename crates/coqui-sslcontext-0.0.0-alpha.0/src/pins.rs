//! Certificate pins: SHA-256 digests of a server's public key (its `SubjectPublicKeyInfo`).

use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use std::time::SystemTime;

use base64::Engine;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::WebPkiSupportedAlgorithms;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use sha2::{Digest, Sha256};
use x509_cert::Certificate;
use x509_cert::der::{Decode, Encode};

use crate::error::{Error, Reason};

fn credential(what: &'static str) -> Error {
    Error::Credential(Reason::new(what))
}

/// The SHA-256 digest of the `SubjectPublicKeyInfo` of a DER certificate.
fn spki_digest(certificate: &[u8]) -> Option<[u8; 32]> {
    let certificate = Certificate::from_der(certificate).ok()?;
    let spki = certificate
        .tbs_certificate()
        .subject_public_key_info()
        .to_der()
        .ok()?;
    Some(Sha256::digest(spki).into())
}

/// A set of public key pins. A certificate matches when its public key's SHA-256 digest is one
/// of them; a pin survives the certificate's renewal as long as the key does.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Pins {
    digests: Vec<[u8; 32]>,
}

impl Pins {
    /// No pins.
    pub fn new() -> Self {
        Self::default()
    }

    /// Pins from text: entries separated by commas, semicolons or white space, each
    /// `sha256/<base64>` (curl writes `sha256//<base64>`), `sha256:<hex>` or a bare 64-digit hex
    /// digest.
    ///
    /// # Errors
    ///
    /// [`Error::Credential`] for an entry that is none of these or text with no entry.
    pub fn parse(text: &str) -> Result<Self, Error> {
        let mut pins = Self::new();
        for entry in text
            .split(|character: char| {
                character == ',' || character == ';' || character.is_whitespace()
            })
            .filter(|entry| !entry.is_empty())
        {
            pins.digests.push(parse_entry(entry)?);
        }
        if pins.digests.is_empty() {
            return Err(credential("no pin was given"));
        }
        Ok(pins)
    }

    /// Adds the pin of the public key in the DER certificate `certificate`.
    ///
    /// # Errors
    ///
    /// [`Error::Credential`] when the certificate does not decode.
    pub fn with_certificate(mut self, certificate: &[u8]) -> Result<Self, Error> {
        let digest =
            spki_digest(certificate).ok_or_else(|| credential("a certificate does not decode"))?;
        self.digests.push(digest);
        Ok(self)
    }

    /// Adds a pin from its SHA-256 digest.
    #[must_use]
    pub fn with_digest(mut self, digest: [u8; 32]) -> Self {
        self.digests.push(digest);
        self
    }

    /// The pins as `sha256/<base64>` entries, which [`parse`](Self::parse) reads back.
    pub fn to_entries(&self) -> Vec<String> {
        self.digests
            .iter()
            .map(|digest| {
                alloc::format!(
                    "sha256/{}",
                    base64::engine::general_purpose::STANDARD.encode(digest)
                )
            })
            .collect()
    }

    /// Whether there are no pins.
    pub fn is_empty(&self) -> bool {
        self.digests.is_empty()
    }

    /// Whether the public key of the DER certificate is one of the pins.
    pub fn matches(&self, certificate: &[u8]) -> bool {
        spki_digest(certificate).is_some_and(|digest| self.digests.contains(&digest))
    }
}

fn parse_entry(entry: &str) -> Result<[u8; 32], Error> {
    let bad = || credential("a pin is not sha256/<base64> or a hex digest");
    let lowered = entry.to_ascii_lowercase();
    if let Some(rest) = lowered.strip_prefix("sha256:") {
        return hex_digest(rest).ok_or_else(bad);
    }
    if let Some(rest) = entry
        .get(..7)
        .filter(|prefix| prefix.eq_ignore_ascii_case("sha256/"))
        .map(|_| entry[7..].trim_start_matches('/'))
    {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(rest)
            .map_err(|_| bad())?;
        return <[u8; 32]>::try_from(bytes.as_slice()).map_err(|_| bad());
    }
    hex_digest(&lowered).ok_or_else(bad)
}

fn hex_digest(text: &str) -> Option<[u8; 32]> {
    let text: String = text.chars().filter(|character| *character != ':').collect();
    if text.len() != 64 || !text.is_ascii() {
        return None;
    }
    let mut digest = [0u8; 32];
    for (index, byte) in digest.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(digest)
}

/// Verifies as `inner` does, then requires one of the presented certificates to carry a pinned
/// key.
pub(crate) struct PinningVerifier {
    pub(crate) inner: Arc<dyn ServerCertVerifier>,
    pub(crate) pins: Pins,
    pub(crate) provider: Arc<rustls::crypto::CryptoProvider>,
}

impl fmt::Debug for PinningVerifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PinningVerifier")
            .field("pins", &self.pins.digests.len())
            .finish_non_exhaustive()
    }
}

impl PinningVerifier {
    /// Whether `end_entity` verifies with `anchor` as the only trust anchor.
    fn chains_to(
        &self,
        anchor: &CertificateDer<'_>,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        now: UnixTime,
    ) -> bool {
        let mut store = rustls::RootCertStore::empty();
        if store.add(anchor.clone().into_owned()).is_err() {
            return false;
        }
        rustls::client::WebPkiServerVerifier::builder_with_provider(
            Arc::new(store),
            Arc::clone(&self.provider),
        )
        .build()
        .and_then(|verifier| {
            verifier
                .verify_server_cert(end_entity, intermediates, server_name, &[], now)
                .map_err(|_| rustls::client::VerifierBuilderError::NoRootAnchors)
        })
        .is_ok()
    }
}

impl ServerCertVerifier for PinningVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let verified = self.inner.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        )?;
        // A pinned certificate counts when it is the leaf, or when the leaf chains to it: an
        // unrelated pinned certificate appended to the chain proves nothing.
        let pinned = self.pins.matches(end_entity.as_ref())
            || intermediates
                .iter()
                .filter(|certificate| self.pins.matches(certificate.as_ref()))
                .any(|certificate| {
                    self.chains_to(certificate, end_entity, intermediates, server_name, now)
                });
        if pinned {
            Ok(verified)
        } else {
            Err(rustls::Error::General(
                "no certificate of the server carries a pinned key".to_string(),
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

/// Accepts the server's own certificate when its key is pinned, with no chain to a root: for a
/// self-signed peer. The name and the validity period are still checked.
pub(crate) struct PinnedLeafVerifier {
    pub(crate) pins: Pins,
    pub(crate) algorithms: WebPkiSupportedAlgorithms,
}

impl fmt::Debug for PinnedLeafVerifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PinnedLeafVerifier")
            .field("pins", &self.pins.digests.len())
            .finish_non_exhaustive()
    }
}

impl ServerCertVerifier for PinnedLeafVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if !self.pins.matches(end_entity.as_ref()) {
            return Err(rustls::Error::General(
                "the server's key is not pinned".to_string(),
            ));
        }
        let parsed = rustls::server::ParsedCertificate::try_from(end_entity)?;
        rustls::client::verify_server_name(&parsed, server_name)?;
        let certificate = Certificate::from_der(end_entity.as_ref())
            .map_err(|_| rustls::Error::General("the certificate does not decode".to_string()))?;
        let validity = certificate.tbs_certificate().validity();
        let at = SystemTime::UNIX_EPOCH + core::time::Duration::from_secs(now.as_secs());
        if at < validity.not_before.to_system_time() || at > validity.not_after.to_system_time() {
            return Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::Expired,
            ));
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}
