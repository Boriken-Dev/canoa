//! TLS that follows the system: servers verified by the operating system, and a client
//! certificate from an operating-system keystore chosen as a browser would.

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use std::sync::Mutex;

use rustls::client::ResolvesClientCert;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::sign::CertifiedKey;
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};

use crate::error::{Error, Reason};

#[cfg(windows)]
mod cng;
mod identity;
#[cfg(target_os = "macos")]
mod keychain;
#[cfg(feature = "pkcs11")]
mod pkcs11;
mod policy;
mod select;

#[cfg(windows)]
pub use cng::CngSource;
pub use identity::{CertificateInfo, Identity, KeyType};
#[cfg(target_os = "macos")]
pub use keychain::KeychainSource;
#[cfg(feature = "pkcs11")]
pub use pkcs11::{Pkcs11Source, Pkcs11SourceBuilder};
pub use policy::{
    AutoSelectRule, AutoSelectRuleBuilder, NameFilter, browser_policies, rules_from_policy,
};
pub use select::{CertificateRequest, Chooser, IdentitySource, Selector, SelectorBuilder};

/// Who is trusted to vouch for a server.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub enum Trust {
    /// The operating system verifies: CryptoAPI on Windows, the Security framework on macOS,
    /// and the system's root store on Linux and elsewhere.
    #[default]
    System,
    /// Only these root certificates, replacing the system's trust exactly.
    Roots(Vec<CertificateDer<'static>>),
    /// Nobody: no server is verified. Reported by [`Tls::non_defaults`].
    Disabled,
}

/// What a client does when a server asks for a certificate.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub enum ClientCertificates {
    /// Send none.
    None,
    /// What a browser on this system would send: [`Selector::system`].
    #[default]
    System,
    /// A selector of your own.
    Selector(Arc<Selector>),
    /// This identity, whatever the server asks.
    Identity(Identity),
}

/// A setting that is not the default, as data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NonDefault {
    /// The setting's name.
    pub setting: &'static str,
    /// What it is set to.
    pub value: String,
    /// What it is by default.
    pub default: &'static str,
}

/// Builds a [`Tls`].
#[derive(Debug, Clone, Default)]
#[must_use]
pub struct TlsBuilder {
    trust: Trust,
    client_certificates: ClientCertificates,
    alpn: Vec<Vec<u8>>,
}

impl TlsBuilder {
    /// Who is trusted (default: the system).
    pub fn trust(mut self, trust: Trust) -> Self {
        self.trust = trust;
        self
    }

    /// What to send when a server asks for a certificate (default: what a browser would).
    pub fn client_certificates(mut self, certificates: ClientCertificates) -> Self {
        self.client_certificates = certificates;
        self
    }

    /// The application protocols offered, in order (default: none).
    pub fn alpn<I, P>(mut self, protocols: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: Into<Vec<u8>>,
    {
        self.alpn = protocols.into_iter().map(Into::into).collect();
        self
    }

    /// The TLS configuration source.
    ///
    /// # Errors
    ///
    /// [`Error::Tls`] when the operating system's verifier cannot start.
    pub fn build(self) -> Result<Tls, Error> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let verifier: Arc<dyn ServerCertVerifier> = match &self.trust {
            Trust::System => Arc::new(
                rustls_platform_verifier::Verifier::new(Arc::clone(&provider))
                    .map_err(|error| Error::Tls(Reason::new(error.to_string())))?,
            ),
            Trust::Roots(roots) => {
                let mut store = RootCertStore::empty();
                let (added, _ignored) = store.add_parsable_certificates(roots.iter().cloned());
                if added == 0 {
                    return Err(Error::Tls(Reason::new("no root certificate could be read")));
                }
                rustls::client::WebPkiServerVerifier::builder_with_provider(
                    Arc::new(store),
                    Arc::clone(&provider),
                )
                .build()
                .map_err(|error| Error::Tls(Reason::new(error.to_string())))?
            }
            Trust::Disabled => {
                Arc::new(DisabledVerifier(provider.signature_verification_algorithms))
            }
        };
        let selector = match &self.client_certificates {
            ClientCertificates::System => Some(Arc::new(Selector::system())),
            ClientCertificates::Selector(selector) => Some(Arc::clone(selector)),
            ClientCertificates::None | ClientCertificates::Identity(_) => None,
        };
        Ok(Tls {
            provider,
            verifier,
            trust: self.trust,
            client_certificates: self.client_certificates,
            selector,
            alpn: self.alpn,
            configs: Mutex::new(BTreeMap::new()),
        })
    }
}

/// Makes TLS client configurations that verify servers as the system does and answer certificate
/// requests from a keystore.
///
/// A configuration is made per server name, because a certificate chosen for a host is remembered
/// for that host; the configurations are cached.
pub struct Tls {
    provider: Arc<CryptoProvider>,
    verifier: Arc<dyn ServerCertVerifier>,
    trust: Trust,
    client_certificates: ClientCertificates,
    selector: Option<Arc<Selector>>,
    alpn: Vec<Vec<u8>>,
    configs: Mutex<BTreeMap<String, Arc<ClientConfig>>>,
}

impl fmt::Debug for Tls {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Tls")
            .field("trust", &self.trust)
            .field("client_certificates", &self.client_certificates)
            .finish_non_exhaustive()
    }
}

impl Tls {
    /// Starts the settings: the system's trust and a browser's choice of client certificate.
    pub fn builder() -> TlsBuilder {
        TlsBuilder::default()
    }

    /// The settings that are not the default, as data.
    pub fn non_defaults(&self) -> Vec<NonDefault> {
        let mut found = Vec::new();
        if matches!(self.trust, Trust::Disabled) {
            found.push(NonDefault {
                setting: "trust",
                value: "disabled".into(),
                default: "system",
            });
        }
        found
    }

    /// The selector answering certificate requests, if any.
    pub fn selector(&self) -> Option<&Arc<Selector>> {
        self.selector.as_ref()
    }

    /// The client configuration for connections to `host`.
    ///
    /// # Errors
    ///
    /// [`Error::Tls`] when the configuration cannot be built.
    pub fn client_config(&self, host: &str) -> Result<Arc<ClientConfig>, Error> {
        let key = host.to_lowercase();
        let mut configs = self
            .configs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(config) = configs.get(&key) {
            return Ok(Arc::clone(config));
        }
        let builder = ClientConfig::builder_with_provider(Arc::clone(&self.provider))
            .with_safe_default_protocol_versions()
            .map_err(|error| Error::Tls(Reason::new(error.to_string())))?
            .dangerous()
            .with_custom_certificate_verifier(Arc::clone(&self.verifier));
        let mut config = match (&self.client_certificates, &self.selector) {
            (ClientCertificates::Identity(identity), _) => {
                builder.with_client_cert_resolver(Arc::new(OneIdentity {
                    key: identity.certified_key(),
                }))
            }
            (_, Some(selector)) => {
                let name = Some(key.clone()).filter(|name| !name.is_empty());
                builder.with_client_cert_resolver(selector.resolver(name))
            }
            _ => builder.with_no_client_auth(),
        };
        config.alpn_protocols.clone_from(&self.alpn);
        let config = Arc::new(config);
        configs.insert(key, Arc::clone(&config));
        Ok(config)
    }
}

/// Answers every certificate request with one identity.
struct OneIdentity {
    key: Arc<CertifiedKey>,
}

impl fmt::Debug for OneIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OneIdentity")
            .finish_non_exhaustive()
    }
}

impl ResolvesClientCert for OneIdentity {
    fn resolve(
        &self,
        _root_hint_subjects: &[&[u8]],
        _sigschemes: &[SignatureScheme],
    ) -> Option<Arc<CertifiedKey>> {
        Some(Arc::clone(&self.key))
    }

    fn has_certs(&self) -> bool {
        true
    }
}

/// Accepts every server certificate: the verifier of [`Trust::Disabled`].
#[derive(Debug)]
struct DisabledVerifier(rustls::crypto::WebPkiSupportedAlgorithms);

impl ServerCertVerifier for DisabledVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.supported_schemes()
    }
}

/// The operating system's keystores, as sources for [`Selector::system`]: Windows CNG's `MY` store
/// of the current user, the macOS Keychain, and, with the `pkcs11` feature, PKCS#11's default
/// module where there is one.
pub(crate) fn system_sources() -> Vec<Box<dyn IdentitySource>> {
    #[cfg_attr(
        not(any(windows, target_os = "macos", all(feature = "pkcs11", unix))),
        expect(unused_mut, reason = "no keystore is built in for this platform")
    )]
    let mut sources: Vec<Box<dyn IdentitySource>> = Vec::new();
    #[cfg(windows)]
    match CngSource::current_user("MY") {
        Ok(source) => sources.push(Box::new(source)),
        Err(error) => tracing::debug!(%error, "the certificate store could not be opened"),
    }
    #[cfg(target_os = "macos")]
    sources.push(Box::new(KeychainSource::new()));
    #[cfg(all(feature = "pkcs11", unix, not(target_os = "macos")))]
    if let Some(source) = Pkcs11Source::system_default() {
        sources.push(Box::new(source));
    }
    sources
}
