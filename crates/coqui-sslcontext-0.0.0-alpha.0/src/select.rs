//! Choosing a client certificate when a server asks, the way a browser does.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use core::time::Duration;
use std::sync::Mutex;
use std::time::{Instant, SystemTime};

use rustls::SignatureScheme;
use rustls::client::ResolvesClientCert;
use rustls::sign::CertifiedKey;

use crate::error::{Error, Reason};
use crate::identity::Identity;
use crate::identity::KeyType;
use crate::pin::Pin;
use crate::policy::{AutoSelectRule, browser_policies};
use crate::settings::{SelectorSettings, TlsVersion};

/// Where identities come from: a keystore, a smart card, a file.
pub trait IdentitySource: Send + Sync + fmt::Debug {
    /// Lists the identities now. A source that cannot be read returns an error, which the
    /// selector logs and skips.
    ///
    /// # Errors
    ///
    /// Whatever stops the keystore being read.
    fn identities(&self) -> Result<Vec<Identity>, Error>;

    /// The identities matching `query`, the one expiring last first.
    ///
    /// # Errors
    ///
    /// Whatever stops the keystore being read.
    fn find(&self, query: &IdentityQuery) -> Result<Vec<Identity>, Error> {
        let mut found: Vec<Identity> = self
            .identities()?
            .into_iter()
            .filter(|identity| query.matches(identity))
            .collect();
        found.sort_by_key(|identity| core::cmp::Reverse(identity.info().not_after()));
        Ok(found)
    }

    /// The best match for `query`: of several, the one that expires last, so a renewed certificate
    /// wins over the one it replaces.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] when nothing matches, or whatever stops the keystore being read.
    fn find_one(&self, query: &IdentityQuery) -> Result<Identity, Error> {
        self.find(query)?
            .into_iter()
            .next()
            .ok_or_else(|| Error::NotFound(Reason::new("no identity matches the query")))
    }

    /// Releases what the source holds open (a token session, a store). Dropping does the same;
    /// the default does nothing.
    fn close(&self) {}
}

/// What to look for among identities: every criterion given must match.
///
/// A thumbprint is the certificate's SHA-1 or SHA-256 in hex (case, spaces and colons ignored). A
/// subject or issuer matches a case-insensitive substring of the RFC 4514 name; a label matches
/// the keystore's own name for the entry exactly, ignoring case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityQuery {
    thumbprint: Option<String>,
    subject: Option<String>,
    issuer: Option<String>,
    label: Option<String>,
    valid_only: bool,
}

impl Default for IdentityQuery {
    fn default() -> Self {
        Self {
            thumbprint: None,
            subject: None,
            issuer: None,
            label: None,
            valid_only: true,
        }
    }
}

/// Builds an [`IdentityQuery`].
#[derive(Debug, Clone, Default)]
#[must_use]
pub struct IdentityQueryBuilder {
    query: IdentityQuery,
}

impl IdentityQueryBuilder {
    /// The certificate's SHA-1 or SHA-256 in hex.
    pub fn thumbprint(mut self, thumbprint: impl Into<String>) -> Self {
        self.query.thumbprint = Some(thumbprint.into());
        self
    }

    /// Text found in the subject, without regard to case.
    pub fn subject(mut self, subject: impl Into<String>) -> Self {
        self.query.subject = Some(subject.into());
        self
    }

    /// Text found in the issuer, without regard to case.
    pub fn issuer(mut self, issuer: impl Into<String>) -> Self {
        self.query.issuer = Some(issuer.into());
        self
    }

    /// The keystore's name for the entry.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.query.label = Some(label.into());
        self
    }

    /// Keep only certificates valid now (default `true`).
    pub fn valid_only(mut self, valid_only: bool) -> Self {
        self.query.valid_only = valid_only;
        self
    }

    /// The query.
    pub fn build(self) -> IdentityQuery {
        self.query
    }
}

fn hex(bytes: &[u8]) -> String {
    use core::fmt::Write;
    bytes.iter().fold(String::new(), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}

impl IdentityQuery {
    /// Starts a query that matches every valid identity.
    pub fn builder() -> IdentityQueryBuilder {
        IdentityQueryBuilder::default()
    }

    /// Whether `identity` satisfies every criterion.
    pub fn matches(&self, identity: &Identity) -> bool {
        let info = identity.info();
        if let Some(wanted) = &self.thumbprint {
            let wanted: String = wanted
                .chars()
                .filter(|character| !matches!(character, ':' | ' '))
                .map(|character| character.to_ascii_lowercase())
                .collect();
            if wanted != hex(info.sha1()) && wanted != hex(info.sha256()) {
                return false;
            }
        }
        let contains = |text: &str, wanted: &Option<String>| {
            wanted
                .as_ref()
                .is_none_or(|wanted| text.to_lowercase().contains(&wanted.to_lowercase()))
        };
        contains(info.subject(), &self.subject)
            && contains(info.issuer(), &self.issuer)
            && self
                .label
                .as_ref()
                .is_none_or(|label| label.eq_ignore_ascii_case(identity.label()))
            && (!self.valid_only || info.valid_now())
    }
}

impl IdentitySource for Identity {
    fn identities(&self) -> Result<Vec<Identity>, Error> {
        Ok(alloc::vec![self.clone()])
    }
}

impl IdentitySource for Vec<Identity> {
    fn identities(&self) -> Result<Vec<Identity>, Error> {
        Ok(self.clone())
    }
}

/// What a server asked for when it requested a client certificate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertificateRequest {
    host: Option<String>,
    authorities: Vec<Vec<u8>>,
    signature_schemes: Vec<SignatureScheme>,
    tls_version: Option<TlsVersion>,
}

impl CertificateRequest {
    /// A request from `host`, naming `authorities` (DER names; none means any CA) and the
    /// signature schemes it verifies.
    pub fn new(
        host: Option<String>,
        authorities: Vec<Vec<u8>>,
        signature_schemes: Vec<SignatureScheme>,
    ) -> Self {
        Self {
            host,
            authorities,
            signature_schemes,
            tls_version: None,
        }
    }

    /// The same request, knowing the TLS version it will be answered in.
    #[must_use]
    pub fn with_tls_version(mut self, version: Option<TlsVersion>) -> Self {
        self.tls_version = version;
        self
    }

    /// The names of the CAs the server accepts, as RFC 4514 strings (a name that does not decode
    /// is skipped).
    pub fn authority_names(&self) -> Vec<String> {
        use x509_cert::der::Decode;
        self.authorities
            .iter()
            .filter_map(|der| x509_cert::name::Name::from_der(der).ok())
            .map(|name| name.to_string())
            .collect()
    }

    /// The key types the server can verify, from its signature schemes.
    pub fn key_types(&self) -> Vec<KeyType> {
        [
            KeyType::Rsa,
            KeyType::EcP256,
            KeyType::EcP384,
            KeyType::EcP521,
            KeyType::Ed25519,
        ]
        .into_iter()
        .filter(|key_type| key_type.usable_with(&self.signature_schemes))
        .collect()
    }

    /// Whether the server accepts RSA signatures only as PSS (it offers no PKCS#1 v1.5 scheme),
    /// as TLS 1.3 requires.
    pub fn rsa_pss_required(&self) -> bool {
        let offers =
            |wanted: fn(&SignatureScheme) -> bool| self.signature_schemes.iter().any(wanted);
        offers(|scheme| {
            matches!(
                scheme,
                SignatureScheme::RSA_PSS_SHA256
                    | SignatureScheme::RSA_PSS_SHA384
                    | SignatureScheme::RSA_PSS_SHA512
            )
        }) && !offers(|scheme| {
            matches!(
                scheme,
                SignatureScheme::RSA_PKCS1_SHA256
                    | SignatureScheme::RSA_PKCS1_SHA384
                    | SignatureScheme::RSA_PKCS1_SHA512
            )
        })
    }

    /// The TLS version the request will be answered in, when it is known: `Some(Tls13)` only when
    /// the configuration allows TLS 1.3 alone (rustls does not tell a resolver the version it
    /// negotiated); otherwise `None`.
    pub fn tls_version(&self) -> Option<TlsVersion> {
        self.tls_version
    }

    /// The server name the client sent (SNI), if any.
    pub fn host(&self) -> Option<&str> {
        self.host.as_deref()
    }

    /// The DER names of the CAs the server accepts; empty means any.
    pub fn authorities(&self) -> &[Vec<u8>] {
        &self.authorities
    }

    /// The signature schemes the server verifies.
    pub fn signature_schemes(&self) -> &[SignatureScheme] {
        &self.signature_schemes
    }
}

/// Picks one of the candidates by index, or `None` to send no certificate.
pub type Chooser = Arc<dyn Fn(&CertificateRequest, &[Identity]) -> Option<usize> + Send + Sync>;

struct State {
    listed: Option<(Instant, Vec<Identity>)>,
    policies: Option<(Instant, Vec<AutoSelectRule>)>,
    remembered: BTreeMap<String, [u8; 32]>,
}

/// Answers a server's certificate request with an identity from its sources.
///
/// Candidates are what a browser would offer: valid now, usable for client authentication, of a
/// key type the server's signature schemes allow, and, when the server names CAs, issued under
/// one of them (the leaf's issuer, or any subject or issuer in its chain); the one expiring
/// last first. The choice, in order: the identity remembered for the host, the first rule that
/// matches the host and a candidate, the same for the browsers' policies when enabled, the
/// [`Chooser`], and, with none, the only candidate when there is exactly one. Otherwise no
/// certificate is sent and the server decides.
pub struct Selector {
    sources: Vec<Box<dyn IdentitySource>>,
    chooser: Option<Chooser>,
    rules: Vec<AutoSelectRule>,
    browser_policy: bool,
    remember: bool,
    cache_ttl: Duration,
    remembered_capacity: usize,
    state: Mutex<State>,
}

impl fmt::Debug for Selector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Selector")
            .field("sources", &self.sources)
            .field("rules", &self.rules.len())
            .field("browser_policy", &self.browser_policy)
            .finish_non_exhaustive()
    }
}

/// Builds a [`Selector`].
#[derive(Debug)]
#[must_use]
pub struct SelectorBuilder {
    selector: Selector,
}

impl SelectorBuilder {
    /// Adds a source of identities.
    pub fn source(mut self, source: impl IdentitySource + 'static) -> Self {
        self.selector.sources.push(Box::new(source));
        self
    }

    /// Adds rules that pick a certificate for a host without asking.
    pub fn rules(mut self, rules: impl IntoIterator<Item = AutoSelectRule>) -> Self {
        self.selector.rules.extend(rules);
        self
    }

    /// Whether the browsers' `AutoSelectCertificateForUrls` policies also apply (default `false`).
    pub fn browser_policy(mut self, enabled: bool) -> Self {
        self.selector.browser_policy = enabled;
        self
    }

    /// What picks among several candidates when no rule does (default: send none).
    pub fn chooser(mut self, chooser: Chooser) -> Self {
        self.selector.chooser = Some(chooser);
        self
    }

    /// Whether the choice made for a host is reused for it (default `true`).
    pub fn remember(mut self, remember: bool) -> Self {
        self.selector.remember = remember;
        self
    }

    /// How long a listing of the sources is reused (default 60 seconds).
    pub fn cache_ttl(mut self, ttl: Duration) -> Self {
        self.selector.cache_ttl = ttl;
        self
    }

    /// How many hosts' choices are remembered (default 256); the oldest is forgotten beyond it.
    pub fn remembered_capacity(mut self, capacity: usize) -> Self {
        self.selector.remembered_capacity = capacity;
        self
    }

    /// All the plain settings at once.
    pub fn settings(mut self, settings: SelectorSettings) -> Self {
        self.selector.cache_ttl = settings.cache_ttl;
        self.selector.remember = settings.remember;
        self.selector.browser_policy = settings.browser_policy;
        self.selector.remembered_capacity = settings.remembered_capacity;
        self
    }

    /// The selector.
    pub fn build(self) -> Selector {
        self.selector
    }
}

impl Selector {
    /// Starts a selector with no source: it sends no certificate.
    pub fn builder() -> SelectorBuilder {
        SelectorBuilder {
            selector: Self {
                sources: Vec::new(),
                chooser: None,
                rules: Vec::new(),
                browser_policy: false,
                remember: true,
                cache_ttl: Duration::from_secs(60),
                remembered_capacity: SelectorSettings::DEFAULT_REMEMBERED,
                state: Mutex::new(State {
                    listed: None,
                    policies: None,
                    remembered: BTreeMap::new(),
                }),
            },
        }
    }

    /// The selector a browser on this system would use: the operating system's keystore (Windows
    /// CNG, the macOS Keychain, PKCS#11's default module where there is one) and the browsers'
    /// auto-select policies.
    ///
    /// A smart card's PIN is asked of the keystore itself, as a browser does
    /// ([`Pin::Keystore`]); [`system_builder`](Self::system_builder) takes another source.
    pub fn system() -> Self {
        Self::system_builder(Pin::Keystore).build()
    }

    /// The builder of [`system`](Self::system), with `pin` giving smart card PINs: [`Pin::None`]
    /// never waits on a person, and keys that need a PIN fail instead.
    pub fn system_builder(pin: Pin) -> SelectorBuilder {
        let mut builder = Self::builder().browser_policy(true);
        for source in crate::keystore::system_sources(pin) {
            builder.selector.sources.push(source);
        }
        builder
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Every identity the sources hold, listed at most every cache interval. A source that fails is
    /// logged and skipped.
    pub fn identities(&self) -> Vec<Identity> {
        let mut state = self.state();
        if let Some((at, listed)) = &state.listed
            && at.elapsed() < self.cache_ttl
        {
            return listed.clone();
        }
        let mut listed = Vec::new();
        for source in &self.sources {
            match source.identities() {
                Ok(found) => listed.extend(found),
                Err(error) => tracing::warn!(%error, ?source, "a keystore could not be listed"),
            }
        }
        state.listed = Some((Instant::now(), listed.clone()));
        listed
    }

    /// The browsers' auto-select rules, read at most every cache interval; none unless
    /// [`SelectorBuilder::browser_policy`] is on.
    pub fn policies(&self) -> Vec<AutoSelectRule> {
        if !self.browser_policy {
            return Vec::new();
        }
        let mut state = self.state();
        if let Some((at, policies)) = &state.policies
            && at.elapsed() < self.cache_ttl
        {
            return policies.clone();
        }
        let policies = browser_policies();
        state.policies = Some((Instant::now(), policies.clone()));
        policies
    }

    /// Lists the sources and reads the policies again on the next request.
    pub fn refresh(&self) {
        let mut state = self.state();
        state.listed = None;
        state.policies = None;
    }

    /// Forgets the choice made for `host`, or for every host.
    pub fn forget(&self, host: Option<&str>) {
        let mut state = self.state();
        match host {
            Some(host) => {
                state.remembered.remove(&host.to_lowercase());
            }
            None => state.remembered.clear(),
        }
    }

    /// The identities of every source matching `query`, the one expiring last first.
    pub fn find(&self, query: &IdentityQuery) -> Vec<Identity> {
        let mut found: Vec<Identity> = self
            .identities()
            .into_iter()
            .filter(|identity| query.matches(identity))
            .collect();
        found.sort_by_key(|identity| core::cmp::Reverse(identity.info().not_after()));
        found
    }

    /// The identities a browser would offer for `request`, the one expiring last first.
    pub fn candidates(&self, request: &CertificateRequest) -> Vec<Identity> {
        let now = SystemTime::now();
        let mut found: Vec<Identity> = self
            .identities()
            .into_iter()
            .filter(|identity| {
                let info = identity.info();
                info.valid_at(now)
                    && info.client_auth()
                    && info.key_type().usable_with(request.signature_schemes())
                    && info.issued_under(request.authorities())
            })
            .collect();
        found.sort_by_key(|identity| core::cmp::Reverse(identity.info().not_after()));
        found
    }

    /// The identity to answer `request` with, or `None` for none.
    pub fn select(&self, request: &CertificateRequest) -> Option<Identity> {
        let candidates = self.candidates(request);
        if candidates.is_empty() {
            return None;
        }
        let host = request.host().map(str::to_lowercase);
        if let Some(host) = &host
            && let Some(fingerprint) = self.state().remembered.get(host).copied()
            && let Some(identity) = candidates
                .iter()
                .find(|identity| *identity.info().fingerprint() == fingerprint)
        {
            return Some(identity.clone());
        }
        let by_rule = self
            .rules
            .iter()
            .cloned()
            .chain(self.policies())
            .filter(|rule| rule.matches_host(request.host()))
            .find_map(|rule| {
                candidates
                    .iter()
                    .find(|identity| rule.matches(identity.info()))
            });
        let chosen = by_rule.cloned().or_else(|| match &self.chooser {
            Some(chooser) => {
                chooser(request, &candidates).and_then(|index| candidates.get(index).cloned())
            }
            None if candidates.len() == 1 => candidates.first().cloned(),
            None => {
                tracing::info!(
                    candidates = candidates.len(),
                    "several certificates fit and nothing picks one; sending none"
                );
                None
            }
        });
        if let (Some(identity), Some(host), true) = (&chosen, host, self.remember) {
            let mut state = self.state();
            if !state.remembered.contains_key(&host)
                && state.remembered.len() >= self.remembered_capacity.max(1)
                && let Some(oldest) = state.remembered.keys().next().cloned()
            {
                state.remembered.remove(&oldest);
            }
            state
                .remembered
                .insert(host, *identity.info().fingerprint());
        }
        chosen
    }

    /// A `rustls` resolver that answers for connections to `host`.
    pub fn resolver(self: &Arc<Self>, host: Option<String>) -> Arc<dyn ResolvesClientCert> {
        self.resolver_for(host, None)
    }

    /// Like [`resolver`](Self::resolver), telling each request the TLS version it will be
    /// answered in ([`CertificateRequest::tls_version`]).
    pub fn resolver_for(
        self: &Arc<Self>,
        host: Option<String>,
        version: Option<TlsVersion>,
    ) -> Arc<dyn ResolvesClientCert> {
        Arc::new(HostResolver {
            selector: Arc::clone(self),
            host,
            version,
        })
    }
}

#[derive(Debug)]
struct HostResolver {
    selector: Arc<Selector>,
    host: Option<String>,
    version: Option<TlsVersion>,
}

impl ResolvesClientCert for HostResolver {
    fn resolve(
        &self,
        root_hint_subjects: &[&[u8]],
        sigschemes: &[SignatureScheme],
    ) -> Option<Arc<CertifiedKey>> {
        let request = CertificateRequest::new(
            self.host.clone(),
            root_hint_subjects
                .iter()
                .map(|name| name.to_vec())
                .collect(),
            sigschemes.to_vec(),
        )
        .with_tls_version(self.version);
        let identity = self.selector.select(&request)?;
        tracing::debug!(host = ?self.host, subject = identity.info().subject(), "answering a certificate request");
        Some(identity.certified_key())
    }

    fn has_certs(&self) -> bool {
        true
    }
}
