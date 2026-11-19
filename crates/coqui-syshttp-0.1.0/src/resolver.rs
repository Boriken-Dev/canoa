//! The proxy resolver: which proxy, if any, a URL goes through.
//!
//! The order is the same on every operating system:
//!
//! 1. `NO_PROXY` (or the explicit bypass list): those hosts go direct.
//! 2. An explicit [`ProxySetting`].
//! 3. The proxy environment variables, when any is set: they alone decide, and a scheme with no
//!    variable goes direct.
//! 4. The operating system: on Windows WinHTTP (which does WPAD and PAC itself), then the
//!    system's settings with auto-detection, the PAC URL, the PAC script and the manual proxies.
//! 5. Direct.

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::time::Duration;
use std::sync::Mutex;
use std::time::Instant;

use crate::bypass::BypassRules;
use crate::env::{Environment, Source};
use crate::error::Error;
use crate::proxy::{Credentials, Proxy, ProxyScheme, Route, Target};
use crate::system::{SettingsState, SystemSettings};
pub use child::ChildEnvironment;

/// How long the system's settings are trusted before they are read again.
const SETTINGS_MAX_AGE: Duration = Duration::from_secs(60);
/// How long a PAC source that failed is left alone before it is tried again.
#[cfg(feature = "pac")]
const RETRY_AFTER: Duration = Duration::from_secs(30);
/// How long a proxy that refused a connection is tried last.
const BAD_PROXY_TTL: Duration = Duration::from_secs(300);
const ANSWER_CACHE: usize = 1024;

/// What an explicit proxy setting says, replacing the environment and the system.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProxySetting {
    /// Every URL goes through this proxy.
    Proxy(Proxy),
    /// The PAC script at this location (a URL, a `file:` URL or a path) chooses.
    Pac(String),
    /// No proxy at all.
    Direct,
}

/// What to do with a PAC location when no JavaScript engine can run it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum OnMissingEngine {
    /// Fail the lookup with [`Error::MissingEngine`].
    #[default]
    Error,
    /// Answer direct.
    Direct,
}

/// Builds a [`Resolver`].
#[derive(Debug, Clone)]
#[must_use]
pub struct ResolverBuilder {
    environment: Option<Environment>,
    setting: Option<ProxySetting>,
    no_proxy: Option<BypassRules>,
    system: bool,
    settings: Option<SystemSettings>,
    wpad: bool,
    credentials: Option<Credentials>,
    timeout: Duration,
    on_missing_engine: OnMissingEngine,
}

impl ResolverBuilder {
    /// The environment the proxy variables are read from (default: the process's, as it is when
    /// the resolver is built).
    pub fn environment(mut self, environment: Environment) -> Self {
        self.environment = Some(environment);
        self
    }

    /// An explicit proxy, PAC location or "none", replacing the environment and the system.
    pub fn proxy(mut self, setting: ProxySetting) -> Self {
        self.setting = Some(setting);
        self
    }

    /// The hosts that go direct, replacing `NO_PROXY`.
    pub fn no_proxy(mut self, rules: BypassRules) -> Self {
        self.no_proxy = Some(rules);
        self
    }

    /// Whether the operating system's settings are consulted (default `true`).
    pub fn system(mut self, system: bool) -> Self {
        self.system = system;
        self
    }

    /// Settings to use instead of reading the operating system's.
    pub fn settings(mut self, settings: SystemSettings) -> Self {
        self.settings = Some(settings);
        self
    }

    /// Whether WPAD runs when the system has no settings at all (default `false`: it runs only
    /// when the settings switch auto-detection on).
    pub fn wpad(mut self, wpad: bool) -> Self {
        self.wpad = wpad;
        self
    }

    /// The login for every HTTP proxy that names none of its own (Basic, or Negotiate and NTLM
    /// when a [`crate::Negotiate`] policy asks for it).
    pub fn credentials(mut self, credentials: Credentials) -> Self {
        self.credentials = Some(credentials);
        self
    }

    /// The time limit of each WPAD probe and PAC download (default 10 seconds).
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// What a PAC location does when no JavaScript engine can run it (default: an error).
    pub fn on_missing_engine(mut self, behaviour: OnMissingEngine) -> Self {
        self.on_missing_engine = behaviour;
        self
    }

    /// The resolver. Nothing is fetched until the first lookup.
    pub fn build(self) -> Resolver {
        let environment = self.environment.unwrap_or_else(Environment::process);
        let variables = environment.proxy_variables();
        let no_proxy = self
            .no_proxy
            .or_else(|| variables.as_ref().map(crate::ProxyVariables::no_proxy))
            .unwrap_or_default();
        let variables = variables.filter(crate::ProxyVariables::has_proxies);
        let setting = self.setting.map(|setting| match setting {
            ProxySetting::Proxy(proxy) => Slot::Routes(alloc::vec![Route::Proxy(proxy)]),
            ProxySetting::Pac(location) => Slot::pac(PacSource::Location(location)),
            ProxySetting::Direct => Slot::Routes(alloc::vec![Route::Direct]),
        });
        let system = (self.system && setting.is_none() && variables.is_none())
            .then(|| SystemSource::new(self.settings, self.wpad));
        Resolver {
            inner: Arc::new(Inner {
                setting,
                variables,
                env_slots: Mutex::new(BTreeMap::new()),
                no_proxy,
                system,
                credentials: self.credentials,
                timeout: self.timeout,
                on_missing_engine: self.on_missing_engine,
                bad: Mutex::new(BTreeMap::new()),
            }),
        }
    }
}

/// Answers which proxy a URL goes through, following the system by default.
///
/// A resolver is cheap to clone and safe to share between threads. It reads nothing until the
/// first lookup, and then reads the operating system's settings again every minute.
#[derive(Debug, Clone)]
pub struct Resolver {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    setting: Option<Slot>,
    variables: Option<crate::ProxyVariables>,
    env_slots: Mutex<BTreeMap<String, Arc<Slot>>>,
    no_proxy: BypassRules,
    system: Option<SystemSource>,
    credentials: Option<Credentials>,
    timeout: Duration,
    on_missing_engine: OnMissingEngine,
    bad: Mutex<BTreeMap<String, Instant>>,
}

impl Resolver {
    /// A resolver that follows the system: the proxy variables of this process's environment
    /// first, then the operating system's settings, PAC and WPAD included.
    pub fn system() -> Self {
        Self::builder().build()
    }

    /// Starts a resolver.
    pub fn builder() -> ResolverBuilder {
        ResolverBuilder {
            environment: None,
            setting: None,
            no_proxy: None,
            system: true,
            settings: None,
            wpad: false,
            credentials: None,
            timeout: Duration::from_secs(10),
            on_missing_engine: OnMissingEngine::default(),
        }
    }

    /// The routes to try for `url`, in order; never empty. A proxy that recently refused a
    /// connection ([`Resolver::report_failure`]) is moved to the end.
    ///
    /// # Errors
    ///
    /// [`Error::Parse`] for a URL without a scheme and host, [`Error::MissingEngine`] for a PAC
    /// location this build cannot run (unless [`OnMissingEngine::Direct`]), and
    /// [`Error::Pac`] for an explicit PAC that fails.
    pub fn lookup(&self, url: &str) -> Result<Vec<Route>, Error> {
        let target = Target::parse(url)?;
        let inner = &*self.inner;
        let routes = if inner.no_proxy.matches_target(&target) {
            alloc::vec![Route::Direct]
        } else if let Some(setting) = &inner.setting {
            inner
                .answer(setting, url, &target)?
                .unwrap_or_else(|| alloc::vec![Route::Direct])
        } else if let Some(variables) = &inner.variables {
            match variables.source_for(&target) {
                Some(source) => inner.env_answer(source, url, &target)?,
                None => alloc::vec![Route::Direct],
            }
        } else if let Some(system) = &inner.system {
            system.lookup(inner, url, &target)?
        } else {
            alloc::vec![Route::Direct]
        };
        Ok(inner.finish(routes))
    }

    /// Records that connecting to `proxy` failed: for five minutes it is tried after the others.
    pub fn report_failure(&self, proxy: &Proxy) {
        self.inner
            .bad
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(proxy.authority(), Instant::now());
    }

    /// Forgets every recorded failure and every cached answer.
    pub fn clear(&self) {
        self.inner
            .bad
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        if let Some(system) = &self.inner.system {
            system.clear();
        }
    }
}

impl Inner {
    /// Gives each proxy without a login the resolver's, and puts unhealthy proxies last.
    fn finish(&self, mut routes: Vec<Route>) -> Vec<Route> {
        if let Some(credentials) = &self.credentials {
            for route in &mut routes {
                if let Route::Proxy(proxy) = route
                    && matches!(proxy.scheme(), ProxyScheme::Http | ProxyScheme::Https)
                    && proxy.credentials().is_none()
                {
                    *proxy = proxy.clone().or_credentials(Some(credentials));
                }
            }
        }
        let mut bad = self
            .bad
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        bad.retain(|_, at| at.elapsed() < BAD_PROXY_TTL);
        if !bad.is_empty() {
            routes.sort_by_key(|route| match route {
                Route::Proxy(proxy) => bad.contains_key(&proxy.authority()),
                Route::Direct => false,
            });
        }
        routes
    }

    fn env_answer(&self, source: Source, url: &str, target: &Target) -> Result<Vec<Route>, Error> {
        let key = alloc::format!("{source:?}");
        let slot = {
            let mut slots = self
                .env_slots
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            Arc::clone(
                slots
                    .entry(key)
                    .or_insert_with(|| Arc::new(Slot::from_source(source))),
            )
        };
        Ok(self
            .answer(&slot, url, target)?
            .unwrap_or_else(|| alloc::vec![Route::Direct]))
    }

    /// `Ok(None)` is "no opinion": a PAC that cannot be loaded or run.
    fn answer(&self, slot: &Slot, url: &str, target: &Target) -> Result<Option<Vec<Route>>, Error> {
        match slot {
            Slot::Routes(routes) => Ok(Some(routes.clone())),
            Slot::Pac(pac) => pac.routes(url, target, self.timeout, self.on_missing_engine),
        }
    }
}

/// Where a PAC script comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PacSource {
    Location(String),
    Text(String),
    Wpad,
}

#[derive(Debug)]
enum Slot {
    Routes(Vec<Route>),
    Pac(PacSlot),
}

impl Slot {
    fn pac(source: PacSource) -> Self {
        Self::Pac(PacSlot::new(source))
    }

    fn from_source(source: Source) -> Self {
        match source {
            Source::Routes(routes) => Self::Routes(routes),
            Source::PacLocation(location) => Self::pac(PacSource::Location(location)),
            Source::PacScript(text) => Self::pac(PacSource::Text(text)),
        }
    }
}

/// A PAC script built on the first lookup that needs it, and again after a failure once the
/// retry interval has passed.
#[derive(Debug)]
struct PacSlot {
    source: PacSource,
    state: Mutex<PacState>,
}

#[derive(Debug, Default)]
struct PacState {
    #[cfg(feature = "pac")]
    script: Option<Arc<crate::pac::PacScript>>,
    #[cfg(feature = "pac")]
    failed_at: Option<Instant>,
}

impl PacSlot {
    fn new(source: PacSource) -> Self {
        Self {
            source,
            state: Mutex::new(PacState::default()),
        }
    }

    #[cfg(feature = "pac")]
    fn routes(
        &self,
        url: &str,
        target: &Target,
        timeout: Duration,
        _on_missing_engine: OnMissingEngine,
    ) -> Result<Option<Vec<Route>>, Error> {
        let script = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.script.is_none()
                && state
                    .failed_at
                    .is_none_or(|failed| failed.elapsed() >= RETRY_AFTER)
            {
                let loaded = match &self.source {
                    PacSource::Location(location) => {
                        crate::pac::PacScript::from_location(location, timeout).map(Some)
                    }
                    PacSource::Text(text) => crate::pac::PacScript::builder(text.clone())
                        .build()
                        .map(Some),
                    PacSource::Wpad => Ok(crate::pac::wpad::discover(None, timeout)),
                };
                match loaded {
                    Ok(Some(script)) => state.script = Some(Arc::new(script)),
                    Ok(None) => state.failed_at = Some(Instant::now()),
                    Err(error) => {
                        tracing::debug!(%error, "a pac source failed to load");
                        state.failed_at = Some(Instant::now());
                    }
                }
            }
            state.script.clone()
        };
        let Some(script) = script else {
            return Ok(None);
        };
        match script.find_proxy_for_url(url, &target.host) {
            Ok(text) => Ok(Some(Route::parse_pac_result(&text))),
            Err(error) => {
                tracing::debug!(%error, "a pac script failed");
                Ok(None)
            }
        }
    }

    #[cfg(not(feature = "pac"))]
    fn routes(
        &self,
        _url: &str,
        _target: &Target,
        _timeout: Duration,
        on_missing_engine: OnMissingEngine,
    ) -> Result<Option<Vec<Route>>, Error> {
        let _ = (&self.source, &self.state);
        match on_missing_engine {
            OnMissingEngine::Error => Err(Error::MissingEngine),
            OnMissingEngine::Direct => Ok(Some(alloc::vec![Route::Direct])),
        }
    }
}

/// The operating system: WinHTTP where there is one, then the system's settings.
#[derive(Debug)]
struct SystemSource {
    given: Option<SystemSettings>,
    wpad: bool,
    cached: Mutex<Option<Cached>>,
    answers: Mutex<BTreeMap<String, (Instant, Vec<Route>)>>,
    #[cfg(windows)]
    winhttp: Option<crate::system::winhttp::WinHttpResolver>,
}

#[derive(Debug)]
struct Cached {
    at: Instant,
    settings: Arc<SystemSettings>,
    slots: Arc<Vec<Slot>>,
}

impl SystemSource {
    fn new(given: Option<SystemSettings>, wpad: bool) -> Self {
        Self {
            #[cfg(windows)]
            winhttp: if given.is_none() {
                crate::system::winhttp::WinHttpResolver::new()
            } else {
                None
            },
            given,
            wpad,
            cached: Mutex::new(None),
            answers: Mutex::new(BTreeMap::new()),
        }
    }

    fn clear(&self) {
        *self
            .cached
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        self.answers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }

    /// The settings and the PAC slots built from them, read again after [`SETTINGS_MAX_AGE`].
    fn settings(&self) -> (Arc<SystemSettings>, Arc<Vec<Slot>>) {
        let mut cached = self
            .cached
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(current) = cached.as_ref()
            && (self.given.is_some() || current.at.elapsed() < SETTINGS_MAX_AGE)
        {
            return (Arc::clone(&current.settings), Arc::clone(&current.slots));
        }
        let settings = Arc::new(self.given.clone().unwrap_or_else(SystemSettings::read));
        let mut slots = Vec::new();
        if settings.state != SettingsState::Direct {
            if settings.auto_detect || (self.wpad && settings.state == SettingsState::Unset) {
                slots.push(Slot::pac(PacSource::Wpad));
            }
            if let Some(url) = &settings.pac_url {
                slots.push(Slot::pac(PacSource::Location(url.clone())));
            }
            if let Some(script) = &settings.pac_script {
                slots.push(Slot::pac(PacSource::Text(script.clone())));
            }
        }
        let slots = Arc::new(slots);
        *cached = Some(Cached {
            at: Instant::now(),
            settings: Arc::clone(&settings),
            slots: Arc::clone(&slots),
        });
        (settings, slots)
    }

    fn lookup(&self, inner: &Inner, url: &str, target: &Target) -> Result<Vec<Route>, Error> {
        {
            let mut answers = self
                .answers
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some((at, routes)) = answers.get(url) {
                if at.elapsed() < SETTINGS_MAX_AGE {
                    return Ok(routes.clone());
                }
                answers.remove(url);
            }
        }
        let routes = self.resolve(inner, url, target)?;
        let mut answers = self
            .answers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if answers.len() >= ANSWER_CACHE {
            answers.clear();
        }
        answers.insert(url.to_string(), (Instant::now(), routes.clone()));
        Ok(routes)
    }

    fn resolve(&self, inner: &Inner, url: &str, target: &Target) -> Result<Vec<Route>, Error> {
        #[cfg(windows)]
        if let Some(winhttp) = &self.winhttp
            && let Some(routes) = winhttp.lookup(url, target)
        {
            return Ok(routes);
        }
        let (settings, slots) = self.settings();
        if settings.state == SettingsState::Direct {
            return Ok(alloc::vec![Route::Direct]);
        }
        for slot in slots.iter() {
            if let Some(routes) = inner.answer(slot, url, target)? {
                return Ok(routes);
            }
        }
        Ok(settings.manual_routes(target))
    }
}

mod child;
#[cfg(test)]
mod tests;
