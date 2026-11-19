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

use super::identity::Identity;
use super::policy::{AutoSelectRule, browser_policies};
use crate::error::Error;

/// Where identities come from: a keystore, a smart card, a file.
pub trait IdentitySource: Send + Sync + fmt::Debug {
    /// Lists the identities now. A source that cannot be read returns an error, which the
    /// selector logs and skips.
    ///
    /// # Errors
    ///
    /// Whatever stops the keystore being read.
    fn identities(&self) -> Result<Vec<Identity>, Error>;
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
        }
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
    pub fn system() -> Self {
        let mut builder = Self::builder().browser_policy(true);
        for source in super::system_sources() {
            builder.selector.sources.push(source);
        }
        builder.build()
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

    fn policies(&self) -> Vec<AutoSelectRule> {
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
            self.state()
                .remembered
                .insert(host, *identity.info().fingerprint());
        }
        chosen
    }

    /// A `rustls` resolver that answers for connections to `host`.
    pub fn resolver(self: &Arc<Self>, host: Option<String>) -> Arc<dyn ResolvesClientCert> {
        Arc::new(HostResolver {
            selector: Arc::clone(self),
            host,
        })
    }
}

#[derive(Debug)]
struct HostResolver {
    selector: Arc<Selector>,
    host: Option<String>,
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
        );
        let identity = self.selector.select(&request)?;
        tracing::debug!(host = ?self.host, subject = identity.info().subject(), "answering a certificate request");
        Some(identity.certified_key())
    }

    fn has_certs(&self) -> bool {
        true
    }
}
