//! TLS that follows the system: servers verified by the operating system, and a client
//! certificate from an operating-system keystore chosen as a browser would.

use alloc::collections::VecDeque;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rustls::client::ResolvesClientCert;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::sign::CertifiedKey;
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};

use crate::error::{Error, Reason};
use crate::identity::Identity;
use crate::keylog::key_log;
use crate::pin::Pin;
use crate::pins::{PinnedLeafVerifier, PinningVerifier, Pins};
use crate::select::{CertificatePolicy, IdentityQuery, Selector, sole_candidate_non_default};
use crate::settings::{KeyLogging, TlsSettings, TlsVersion};

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
    /// Only the root certificates in this PEM file, or in every file of this directory, read when
    /// the configuration is built.
    Bundle(PathBuf),
    /// Nobody: no server is verified. Reported by [`Tls::non_defaults`].
    Disabled,
    /// Only a server whose certificate carries one of these public keys, with no chain to a root:
    /// for a self-signed peer. The name and the validity period are still checked.
    Pinned(Pins),
}

impl Trust {
    /// [`Trust::Roots`] from PEM text holding certificates.
    ///
    /// # Errors
    ///
    /// [`Error::Credential`] when the text holds no certificate.
    pub fn roots_from_pem(pem: &[u8]) -> Result<Self, Error> {
        let roots: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(pem)
            .collect::<Result<_, _>>()
            .map_err(|_| Error::Credential(Reason::new("the certificate text is not PEM")))?;
        if roots.is_empty() {
            return Err(Error::Credential(Reason::new(
                "the text holds no certificate",
            )));
        }
        Ok(Self::Roots(roots))
    }
}

/// The certificates of a PEM file, or of every file in a directory.
fn bundle_roots(path: &Path) -> Result<Vec<CertificateDer<'static>>, Error> {
    let mut files = Vec::new();
    if path.is_dir() {
        for entry in std::fs::read_dir(path)? {
            let entry = entry?.path();
            if entry.is_file() {
                files.push(entry);
            }
        }
        files.sort();
    } else {
        files.push(path.to_path_buf());
    }
    let mut roots = Vec::new();
    for file in files {
        let bytes = std::fs::read(&file)?;
        roots.extend(CertificateDer::pem_slice_iter(&bytes).filter_map(Result::ok));
    }
    Ok(roots)
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
    /// These identities, the first whose key the server's signature schemes allow (one per key
    /// type can coexist: an RSA and an EC certificate).
    Identities(Vec<Identity>),
    /// The best match for this query among the system's keystores, looked up at the first request.
    Query(IdentityQuery),
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
#[derive(Clone)]
#[must_use]
pub struct TlsBuilder {
    trust: Trust,
    client_certificates: ClientCertificates,
    alpn: Vec<Vec<u8>>,
    pin: Pin,
    fallback: Option<Identity>,
    pins: Option<Pins>,
    settings: TlsSettings,
    policy: Option<CertificatePolicy>,
    send_sole_candidate: bool,
}

impl Default for TlsBuilder {
    fn default() -> Self {
        Self {
            trust: Trust::default(),
            client_certificates: ClientCertificates::default(),
            alpn: Vec::new(),
            pin: Pin::Keystore,
            fallback: None,
            pins: None,
            settings: TlsSettings::default(),
            policy: None,
            send_sole_candidate: false,
        }
    }
}

impl TlsBuilder {
    /// The caller's rule per server host for the system's certificates (`ClientCertificates::System`
    /// and `Query`; default none, so the selector's own choice applies). A `Selector` given outright
    /// carries its own policy ([`SelectorBuilder::policy`](crate::SelectorBuilder::policy)).
    pub fn certificate_policy(mut self, policy: CertificatePolicy) -> Self {
        self.policy = Some(policy);
        self
    }

    /// Whether the system's only fitting certificate is sent although no rule, policy or chooser
    /// picked it (default `false`; on is listed by `non_defaults()`).
    pub fn send_sole_candidate(mut self, send: bool) -> Self {
        self.send_sole_candidate = send;
        self
    }

    /// Who is trusted (default: the system).
    pub fn trust(mut self, trust: Trust) -> Self {
        self.trust = trust;
        self
    }

    /// What to send when a server asks for a certificate (default: what a browser would choose by a
    /// rule, browser policy or chooser; with none of them, no certificate).
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

    /// Where a smart card key's PIN comes from, for the system's keystores (default
    /// [`Pin::Keystore`], as a browser: the system asks). [`Pin::None`] never waits on a person.
    pub fn pin(mut self, pin: impl Into<Pin>) -> Self {
        self.pin = pin.into();
        self
    }

    /// An identity sent when the keystores offer none that fits (default: none).
    pub fn fallback_identity(mut self, identity: Identity) -> Self {
        self.fallback = Some(identity);
        self
    }

    /// Public key pins, checked in addition to the trust: a server is accepted only if the trust
    /// vouches for it and one of the certificates it sends carries a pinned key (default: none).
    pub fn pins(mut self, pins: Pins) -> Self {
        self.pins = Some(pins);
        self
    }

    /// The oldest protocol version offered (default TLS 1.2).
    pub fn minimum_version(mut self, version: TlsVersion) -> Self {
        self.settings.minimum_version = version;
        self
    }

    /// The newest protocol version offered (default TLS 1.3).
    pub fn maximum_version(mut self, version: TlsVersion) -> Self {
        self.settings.maximum_version = version;
        self
    }

    /// Where session secrets are logged for a traffic analyser (default nowhere).
    pub fn key_log(mut self, logging: KeyLogging) -> Self {
        self.settings.key_log = logging;
        self
    }

    /// All the plain settings at once.
    pub fn settings(mut self, settings: TlsSettings) -> Self {
        self.settings = settings;
        self
    }

    /// The settings that are not the default, as data (see [`Tls::non_defaults`]).
    pub fn non_defaults(&self) -> Vec<NonDefault> {
        let sole = match &self.client_certificates {
            ClientCertificates::Selector(selector) => selector.non_defaults(),
            _ => sole_candidate_non_default(self.send_sole_candidate),
        };
        non_defaults(&self.trust, &self.settings, sole)
    }

    /// The TLS configuration source.
    ///
    /// # Errors
    ///
    /// [`Error::Tls`] when the operating system's verifier cannot start, no root certificate can
    /// be read or the versions leave none to offer; [`Error::Io`] when a bundle or key log file
    /// cannot be read or opened.
    pub fn build(self) -> Result<Tls, Error> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let versions = protocol_versions(&self.settings)?;
        let verifier: Arc<dyn ServerCertVerifier> = match &self.trust {
            Trust::System => Arc::new(
                rustls_platform_verifier::Verifier::new(Arc::clone(&provider))
                    .map_err(|error| Error::Tls(Reason::new(error.to_string())))?,
            ),
            Trust::Roots(roots) => roots_verifier(roots, &provider)?,
            Trust::Bundle(path) => roots_verifier(&bundle_roots(path)?, &provider)?,
            Trust::Disabled => {
                Arc::new(DisabledVerifier(provider.signature_verification_algorithms))
            }
            Trust::Pinned(pins) => {
                if pins.is_empty() {
                    return Err(Error::Tls(Reason::new("no pin was given")));
                }
                Arc::new(PinnedLeafVerifier {
                    pins: pins.clone(),
                    algorithms: provider.signature_verification_algorithms,
                })
            }
        };
        let verifier: Arc<dyn ServerCertVerifier> = match self.pins.clone() {
            Some(pins) if pins.is_empty() => {
                return Err(Error::Tls(Reason::new("no pin was given")));
            }
            Some(_) if matches!(self.trust, Trust::Disabled) => {
                // Handshake signatures are not checked when nothing is trusted, so a pin would
                // protect nothing.
                return Err(Error::Tls(Reason::new(
                    "pins need a trust that verifies the server; use Trust::Pinned alone",
                )));
            }
            Some(pins) => Arc::new(PinningVerifier {
                inner: verifier,
                pins,
                provider: Arc::clone(&provider),
            }),
            _ => verifier,
        };
        let selector = match &self.client_certificates {
            ClientCertificates::System | ClientCertificates::Query(_) => {
                let mut builder = Selector::system_builder(self.pin.clone())
                    .send_sole_candidate(self.send_sole_candidate);
                if let Some(policy) = &self.policy {
                    builder = builder.policy(Arc::clone(policy));
                }
                Some(Arc::new(builder.build()))
            }
            ClientCertificates::Selector(selector) => Some(Arc::clone(selector)),
            ClientCertificates::None
            | ClientCertificates::Identity(_)
            | ClientCertificates::Identities(_) => None,
        };
        let key_log = key_log(&self.settings.key_log)?;
        Ok(Tls {
            provider,
            verifier,
            versions,
            trust: self.trust,
            client_certificates: self.client_certificates,
            selector,
            alpn: self.alpn,
            fallback: self.fallback.map(|identity| identity.certified_key()),
            settings: self.settings,
            key_log,
            configs: Mutex::new(VecDeque::new()),
        })
    }
}

/// The root certificates a trust names, for verifying the other side's certificate.
pub(crate) fn trust_roots(trust: &Trust) -> Result<RootCertStore, Error> {
    let roots: Vec<CertificateDer<'static>> = match trust {
        Trust::Roots(roots) => roots.clone(),
        Trust::Bundle(path) => bundle_roots(path)?,
        Trust::System => rustls_native_certs::load_native_certs().certs,
        Trust::Disabled | Trust::Pinned(_) => Vec::new(),
    };
    let mut store = RootCertStore::empty();
    let (added, _ignored) = store.add_parsable_certificates(roots);
    if added == 0 {
        return Err(Error::Tls(Reason::new("no root certificate could be read")));
    }
    Ok(store)
}

fn roots_verifier(
    roots: &[CertificateDer<'static>],
    provider: &Arc<CryptoProvider>,
) -> Result<Arc<dyn ServerCertVerifier>, Error> {
    let mut store = RootCertStore::empty();
    let (added, _ignored) = store.add_parsable_certificates(roots.iter().cloned());
    if added == 0 {
        return Err(Error::Tls(Reason::new("no root certificate could be read")));
    }
    Ok(rustls::client::WebPkiServerVerifier::builder_with_provider(
        Arc::new(store),
        Arc::clone(provider),
    )
    .build()
    .map_err(|error| Error::Tls(Reason::new(error.to_string())))?)
}

pub(crate) fn protocol_versions(
    settings: &TlsSettings,
) -> Result<Vec<&'static rustls::SupportedProtocolVersion>, Error> {
    let mut versions = Vec::new();
    if settings.minimum_version <= TlsVersion::Tls12
        && TlsVersion::Tls12 <= settings.maximum_version
    {
        versions.push(&rustls::version::TLS12);
    }
    if settings.minimum_version <= TlsVersion::Tls13
        && TlsVersion::Tls13 <= settings.maximum_version
    {
        versions.push(&rustls::version::TLS13);
    }
    if versions.is_empty() {
        return Err(Error::Tls(Reason::new(
            "the minimum TLS version is above the maximum",
        )));
    }
    Ok(versions)
}

fn non_defaults(trust: &Trust, settings: &TlsSettings, sole: Vec<NonDefault>) -> Vec<NonDefault> {
    let mut found = sole;
    if matches!(trust, Trust::Disabled) {
        found.push(NonDefault {
            setting: "trust",
            value: "disabled".into(),
            default: "system",
        });
    }
    if settings.key_log != KeyLogging::Off {
        found.push(NonDefault {
            setting: "key_log",
            value: "on".into(),
            default: "off",
        });
    }
    found
}

/// Makes TLS client configurations that verify servers as the system does and answer certificate
/// requests from a keystore.
///
/// A configuration is made per server name, because a certificate chosen for a host is remembered
/// for that host; the configurations are cached.
pub struct Tls {
    provider: Arc<CryptoProvider>,
    verifier: Arc<dyn ServerCertVerifier>,
    versions: Vec<&'static rustls::SupportedProtocolVersion>,
    trust: Trust,
    client_certificates: ClientCertificates,
    selector: Option<Arc<Selector>>,
    alpn: Vec<Vec<u8>>,
    fallback: Option<Arc<CertifiedKey>>,
    settings: TlsSettings,
    key_log: Option<Arc<dyn rustls::KeyLog>>,
    configs: Mutex<VecDeque<(String, Arc<ClientConfig>)>>,
}

impl fmt::Debug for Tls {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Tls")
            .field("trust", &self.trust)
            .field("client_certificates", &self.client_certificates)
            .field("settings", &self.settings)
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
        let sole = self
            .selector
            .as_ref()
            .map(|selector| selector.non_defaults())
            .unwrap_or_default();
        non_defaults(&self.trust, &self.settings, sole)
    }

    /// The plain settings this source was built with.
    pub fn settings(&self) -> &TlsSettings {
        &self.settings
    }

    /// The selector answering certificate requests, if any.
    pub fn selector(&self) -> Option<&Arc<Selector>> {
        self.selector.as_ref()
    }

    /// The client configuration for connections to `host`; an empty `host` makes the host-blind
    /// configuration, where the choice of certificate is made from the server's request alone.
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
        if let Some((_, config)) = configs.iter().find(|(name, _)| *name == key) {
            return Ok(Arc::clone(config));
        }
        let builder = ClientConfig::builder_with_provider(Arc::clone(&self.provider))
            .with_protocol_versions(&self.versions)
            .map_err(|error| Error::Tls(Reason::new(error.to_string())))?
            .dangerous()
            .with_custom_certificate_verifier(Arc::clone(&self.verifier));
        let name = Some(key.clone()).filter(|name| !name.is_empty());
        let only_tls13 = self.versions.len() == 1
            && self.versions[0].version == rustls::ProtocolVersion::TLSv1_3;
        let version = only_tls13.then_some(TlsVersion::Tls13);
        let primary: Option<Arc<dyn ResolvesClientCert>> =
            match (&self.client_certificates, &self.selector) {
                (ClientCertificates::Identity(identity), _) => Some(Arc::new(IdentityResolver {
                    identities: alloc::vec![identity.clone()],
                })),
                (ClientCertificates::Identities(identities), _) => {
                    Some(Arc::new(IdentityResolver {
                        identities: identities.clone(),
                    }))
                }
                (ClientCertificates::Query(query), Some(selector)) => {
                    Some(Arc::new(QueryResolver {
                        selector: Arc::clone(selector),
                        query: query.clone(),
                    }))
                }
                (_, Some(selector)) => Some(selector.resolver_for(name, version)),
                _ => None,
            };
        let mut config = match (primary, &self.fallback) {
            (None, None) => builder.with_no_client_auth(),
            (primary, fallback) => builder.with_client_cert_resolver(Arc::new(WithFallback {
                primary,
                fallback: fallback.clone(),
            })),
        };
        config.alpn_protocols.clone_from(&self.alpn);
        if let Some(log) = &self.key_log {
            config.key_log = Arc::clone(log);
        }
        let config = Arc::new(config);
        while configs.len() >= self.settings.config_cache_capacity.max(1) {
            configs.pop_front();
        }
        configs.push_back((key, Arc::clone(&config)));
        Ok(config)
    }
}

/// The client configuration of the system: the operating system's trust and the certificate a
/// browser on this machine would send, chosen from the server's request alone (rustls does not
/// tell a resolver the server's name; [`Tls::client_config`] with a host also remembers choices
/// per host).
///
/// # Errors
///
/// [`Error::Tls`] when the operating system's verifier cannot start.
pub fn client_config() -> Result<Arc<ClientConfig>, Error> {
    Tls::builder().build()?.client_config("")
}

/// Answers a certificate request with the first identity whose key the server can verify.
struct IdentityResolver {
    identities: Vec<Identity>,
}

impl fmt::Debug for IdentityResolver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IdentityResolver")
            .field("identities", &self.identities.len())
            .finish()
    }
}

impl ResolvesClientCert for IdentityResolver {
    fn resolve(
        &self,
        root_hint_subjects: &[&[u8]],
        sigschemes: &[SignatureScheme],
    ) -> Option<Arc<CertifiedKey>> {
        let authorities: Vec<Vec<u8>> = root_hint_subjects
            .iter()
            .map(|name| name.to_vec())
            .collect();
        let fits = |identity: &&Identity| identity.info().key_type().usable_with(sigschemes);
        let usable: Vec<&Identity> = self.identities.iter().filter(fits).collect();
        // One identity is sent whatever the server names; several are told apart by the CAs.
        let chosen = if let [only] = self.identities.as_slice() {
            Some(only)
        } else {
            usable
                .iter()
                .copied()
                .find(|identity| {
                    !authorities.is_empty() && identity.info().issued_under(&authorities)
                })
                .or_else(|| usable.first().copied())
        };
        chosen.map(Identity::certified_key)
    }

    fn has_certs(&self) -> bool {
        !self.identities.is_empty()
    }
}

/// Answers with the best match of a query among a selector's sources.
#[derive(Debug)]
struct QueryResolver {
    selector: Arc<Selector>,
    query: IdentityQuery,
}

impl ResolvesClientCert for QueryResolver {
    fn resolve(
        &self,
        root_hint_subjects: &[&[u8]],
        sigschemes: &[SignatureScheme],
    ) -> Option<Arc<CertifiedKey>> {
        let authorities: Vec<Vec<u8>> = root_hint_subjects
            .iter()
            .map(|name| name.to_vec())
            .collect();
        self.selector
            .find(&self.query)
            .into_iter()
            .find(|identity| {
                let info = identity.info();
                info.client_auth()
                    && info.key_type().usable_with(sigschemes)
                    && info.issued_under(&authorities)
            })
            .map(|identity| identity.certified_key())
    }

    fn has_certs(&self) -> bool {
        true
    }
}

/// The primary resolver's answer, or the fallback identity when it has none that the server can
/// verify.
struct WithFallback {
    primary: Option<Arc<dyn ResolvesClientCert>>,
    fallback: Option<Arc<CertifiedKey>>,
}

impl fmt::Debug for WithFallback {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WithFallback")
            .field("fallback", &self.fallback.is_some())
            .finish_non_exhaustive()
    }
}

impl ResolvesClientCert for WithFallback {
    fn resolve(
        &self,
        root_hint_subjects: &[&[u8]],
        sigschemes: &[SignatureScheme],
    ) -> Option<Arc<CertifiedKey>> {
        if let Some(primary) = &self.primary
            && let Some(key) = primary.resolve(root_hint_subjects, sigschemes)
        {
            return Some(key);
        }
        let fallback = self.fallback.as_ref()?;
        fallback
            .key
            .choose_scheme(sigschemes)
            .map(|_| Arc::clone(fallback))
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

impl fmt::Debug for TlsBuilder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TlsBuilder")
            .field("trust", &self.trust)
            .field("client_certificates", &self.client_certificates)
            .field("settings", &self.settings)
            .field("policy", &self.policy.is_some())
            .field("send_sole_candidate", &self.send_sole_candidate)
            .finish_non_exhaustive()
    }
}
