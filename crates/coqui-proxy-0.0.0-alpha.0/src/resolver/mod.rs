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
use crate::settings::ResolverSettings;
use crate::source::ProxySource;
use crate::system::{SettingsState, SystemSettings};
pub use child::ChildEnvironment;

/// What an explicit proxy setting says, replacing the environment and the system.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProxySetting {
    /// Every URL goes through this proxy.
    Proxy(Proxy),
    /// The PAC script at this location (a URL, a `file:` URL or a path) chooses.
    Pac(String),
    /// These routes, in order: a PAC result written out (`PROXY a:1; DIRECT`).
    Routes(Vec<Route>),
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
    settings: Option<SystemSettings>,
    credentials: Option<Credentials>,
    resolver_settings: ResolverSettings,
    sources: Vec<Arc<dyn ProxySource>>,
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

    /// Adds a source asked after the explicit proxy and before the environment and the system:
    /// the first that has an opinion answers, and sources are asked in the order they were added.
    pub fn source(mut self, source: impl ProxySource + 'static) -> Self {
        self.sources.push(Arc::new(source));
        self
    }

    /// The hosts that go direct, replacing `NO_PROXY`.
    pub fn no_proxy(mut self, rules: BypassRules) -> Self {
        self.no_proxy = Some(rules);
        self
    }

    /// Whether the operating system's settings are consulted (default `true`).
    pub fn system(mut self, system: bool) -> Self {
        self.resolver_settings.system = system;
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
        self.resolver_settings.wpad = wpad;
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
        self.resolver_settings.timeout = timeout;
        self
    }

    /// Every bound the resolver applies, at once (the setters above change one each).
    pub fn resolver_settings(mut self, settings: ResolverSettings) -> Self {
        self.resolver_settings = settings;
        self
    }

    /// What a PAC location does when no JavaScript engine can run it (default: an error).
    pub fn on_missing_engine(mut self, behaviour: OnMissingEngine) -> Self {
        self.resolver_settings.on_missing_engine = behaviour;
        self
    }

    /// What the resolver this builder makes would do that is looser than the defaults.
    #[must_use]
    pub fn non_defaults(&self) -> Vec<String> {
        non_defaults(
            &self.resolver_settings,
            self.no_proxy.as_ref().unwrap_or(&BypassRules::default()),
        )
    }

    /// The resolver. Nothing is fetched until the first lookup.
    pub fn build(self) -> Resolver {
        let environment = self.environment.unwrap_or_else(Environment::process);
        let variables = environment.proxy_variables();
        let no_proxy = self
            .no_proxy
            .or_else(|| variables.as_ref().map(crate::ProxyVariables::no_proxy))
            .unwrap_or_default()
            .with_dns_timeout(self.resolver_settings.bypass_dns_timeout)
            .with_interface_cache_ttl(self.resolver_settings.interface_cache_ttl);
        let variables = variables.filter(crate::ProxyVariables::has_proxies);
        let setting = self.setting.map(|setting| match setting {
            ProxySetting::Proxy(proxy) => Slot::Routes(alloc::vec![Route::Proxy(proxy)]),
            ProxySetting::Pac(location) => {
                if location.contains("FindProxyForURL") {
                    Slot::pac(PacSource::Text(location))
                } else {
                    Slot::pac(PacSource::Location(location))
                }
            }
            ProxySetting::Routes(routes) => Slot::Routes(routes),
            ProxySetting::Direct => Slot::Routes(alloc::vec![Route::Direct]),
        });
        let system = (self.resolver_settings.system && setting.is_none() && variables.is_none())
            .then(|| SystemSource::new(self.settings, &self.resolver_settings));
        Resolver {
            inner: Arc::new(Inner {
                setting,
                variables,
                env_slots: Mutex::new(BTreeMap::new()),
                no_proxy,
                system,
                credentials: self.credentials,
                sources: self.sources,
                settings: self.resolver_settings,
                bad: Mutex::new(BTreeMap::new()),
                rotations: Mutex::new(Rotations::default()),
                probed: Mutex::new(BTreeMap::new()),
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
    sources: Vec<Arc<dyn ProxySource>>,
    settings: ResolverSettings,
    bad: Mutex<BTreeMap<String, Instant>>,
    rotations: Mutex<Rotations>,
    probed: Mutex<BTreeMap<String, Instant>>,
}

impl Resolver {
    /// A resolver that follows the system: the proxy variables of this process's environment
    /// first, then the operating system's settings, PAC and WPAD included.
    pub fn system() -> Self {
        Self::builder().build()
    }

    /// What this resolver does that is looser than the defaults: an empty list is the strict
    /// configuration.
    #[must_use]
    pub fn non_defaults(&self) -> Vec<String> {
        non_defaults(&self.inner.settings, &self.inner.no_proxy)
    }

    /// The sources added with [`ResolverBuilder::source`], in the order they are asked.
    #[must_use]
    pub fn sources(&self) -> &[Arc<dyn ProxySource>] {
        &self.inner.sources
    }

    /// The hosts that go direct.
    #[must_use]
    pub fn no_proxy(&self) -> &BypassRules {
        &self.inner.no_proxy
    }

    /// How many URLs have an answer remembered.
    #[must_use]
    pub fn cache_len(&self) -> usize {
        self.inner
            .system
            .as_ref()
            .map_or(0, SystemSource::cache_len)
    }

    /// The proxy for each scheme of a URL, as a client that keeps a table of them wants it: the
    /// first proxy of the answer for the URL, keyed by the URL's scheme (a bypassed URL or a
    /// direct answer has none). The path and query are not part of the decision.
    ///
    /// # Errors
    ///
    /// As [`Resolver::lookup`].
    pub fn scheme_proxies(&self, url: &str) -> Result<BTreeMap<String, Proxy>, Error> {
        let target = Target::parse(url)?;
        let mut proxies = BTreeMap::new();
        if let Some(Route::Proxy(proxy)) = self
            .lookup(&alloc::format!(
                "{}://{}/",
                target.scheme,
                crate::proxy::join_host_port(&target.host, target.port)
            ))?
            .into_iter()
            .find(|route| matches!(route, Route::Proxy(proxy) if !proxy.is_gateway()))
        {
            proxies.insert(target.scheme.clone(), proxy);
        }
        Ok(proxies)
    }

    /// The settings this resolver applies.
    #[must_use]
    pub fn settings(&self) -> &ResolverSettings {
        &self.inner.settings
    }

    /// Starts a resolver.
    pub fn builder() -> ResolverBuilder {
        ResolverBuilder {
            environment: None,
            setting: None,
            no_proxy: None,
            settings: None,
            credentials: None,
            resolver_settings: ResolverSettings::default(),
            sources: Vec::new(),
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
        } else if let Some(routes) = inner.ask_sources(url)? {
            routes
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
        inner.finish(routes)
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
    /// The first answer of the added sources, or `None` when none has an opinion.
    fn ask_sources(&self, url: &str) -> Result<Option<Vec<Route>>, Error> {
        for source in &self.sources {
            if let Some(routes) = source.routes(url)? {
                return Ok(Some(routes));
            }
        }
        Ok(None)
    }

    /// Gives each proxy without a login the resolver's, rotates and probes as the settings say.
    fn finish(&self, routes: Vec<Route>) -> Result<Vec<Route>, Error> {
        let mut routes = self.decorate(routes);
        if self.settings.rotate {
            self.rotate(&mut routes);
        }
        if self.settings.probe {
            return self.probe(routes);
        }
        Ok(routes)
    }

    /// Gives each proxy without a login the resolver's, and puts unhealthy proxies last.
    fn decorate(&self, mut routes: Vec<Route>) -> Vec<Route> {
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
        bad.retain(|_, at| at.elapsed() < self.settings.failure_memory);
        if !bad.is_empty() {
            routes.sort_by_key(|route| match route {
                Route::Proxy(proxy) => bad.contains_key(&proxy.authority()),
                Route::Direct => false,
            });
        }
        routes
    }

    /// Turns the answer so each lookup of the same answer starts at the next proxy.
    fn rotate(&self, routes: &mut [Route]) {
        if routes.len() < 2 {
            return;
        }
        let key: String = routes
            .iter()
            .map(|route| match route {
                Route::Proxy(proxy) => proxy.authority(),
                _ => "direct".to_string(),
            })
            .collect::<Vec<_>>()
            .join(",");
        let offset = self
            .rotations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .next(&key, self.settings.rotation_capacity);
        let length = routes.len();
        routes.rotate_left(offset % length);
    }

    /// Keeps the routes that accept a connection, probing in order until one does: the rest follow
    /// untried. A proxy that failed a probe is skipped for `probe_failure_memory`.
    fn probe(&self, routes: Vec<Route>) -> Result<Vec<Route>, Error> {
        let mut reachable = Vec::new();
        let mut rest = routes.into_iter();
        for route in rest.by_ref() {
            let Route::Proxy(proxy) = &route else {
                reachable.push(route);
                break;
            };
            if self.probe_one(proxy) {
                reachable.push(route);
                break;
            }
        }
        if reachable.is_empty() {
            return Err(Error::NoReachableProxy);
        }
        reachable.extend(rest);
        Ok(reachable)
    }

    fn probe_one(&self, proxy: &Proxy) -> bool {
        let key = proxy.authority();
        {
            let mut failed = self
                .probed
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            failed.retain(|_, at| at.elapsed() < self.settings.probe_failure_memory);
            if failed.contains_key(&key) {
                return false;
            }
        }
        let reachable = std::net::ToSocketAddrs::to_socket_addrs(&(proxy.host(), proxy.port()))
            .is_ok_and(|mut addresses| {
                addresses.any(|address| {
                    std::net::TcpStream::connect_timeout(&address, self.settings.probe_timeout)
                        .is_ok()
                })
            });
        if !reachable {
            self.probed
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(key, Instant::now());
        }
        reachable
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
            Slot::Pac(pac) => pac.routes(url, target, &self.settings),
        }
    }
}

fn non_defaults(settings: &ResolverSettings, no_proxy: &BypassRules) -> Vec<String> {
    let mut loose = Vec::new();
    if settings.on_missing_engine == OnMissingEngine::Direct {
        loose.push(
            "on_missing_engine(Direct): a PAC location nothing can run answers direct".to_string(),
        );
    }
    if no_proxy.loopback_proxied() {
        loose.push("<-loopback>: loopback hosts are sent through the proxy".to_string());
    }
    loose
}

/// How many times each answer was handed out, to rotate it; bounded by `rotation_capacity`.
#[derive(Debug, Default)]
struct Rotations {
    next_order: u64,
    counts: BTreeMap<String, (u64, usize)>,
}

impl Rotations {
    /// The offset for this lookup of `key`, then counts it.
    fn next(&mut self, key: &str, capacity: usize) -> usize {
        if !self.counts.contains_key(key)
            && self.counts.len() >= capacity
            && let Some(oldest) = self
                .counts
                .iter()
                .min_by_key(|(_, (order, _))| *order)
                .map(|(key, _)| key.clone())
        {
            self.counts.remove(&oldest);
        }
        self.next_order += 1;
        let order = self.next_order;
        let entry = self.counts.entry(key.to_string()).or_insert((order, 0));
        let offset = entry.1;
        entry.1 += 1;
        offset
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
    #[cfg(feature = "pac")]
    loaded_at: Option<Instant>,
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
        settings: &ResolverSettings,
    ) -> Result<Option<Vec<Route>>, Error> {
        let pac_settings = settings.pac_settings();
        let script = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let lifetime = match &self.source {
                PacSource::Location(_) => Some(pac_settings.download_cache_ttl),
                PacSource::Wpad => Some(pac_settings.wpad_cache_ttl),
                PacSource::Text(_) => None,
            };
            let stale = lifetime
                .zip(state.loaded_at)
                .is_some_and(|(lifetime, at)| at.elapsed() >= lifetime);
            if (state.script.is_none() || stale)
                && state
                    .failed_at
                    .is_none_or(|failed| failed.elapsed() >= settings.source_retry)
            {
                let loaded = match &self.source {
                    PacSource::Location(location) => {
                        crate::pac::PacScript::from_location(location, &pac_settings).map(Some)
                    }
                    PacSource::Text(text) => crate::pac::PacScript::builder(text.clone())
                        .settings(&pac_settings)
                        .build()
                        .map(Some),
                    PacSource::Wpad => Ok(crate::pac::wpad::discover(None, &pac_settings)),
                };
                match loaded {
                    Ok(Some(script)) => {
                        state.script = Some(Arc::new(script));
                        state.loaded_at = Some(Instant::now());
                        state.failed_at = None;
                    }
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
        settings: &ResolverSettings,
    ) -> Result<Option<Vec<Route>>, Error> {
        let _ = (&self.source, &self.state);
        match settings.on_missing_engine {
            OnMissingEngine::Error => Err(Error::MissingEngine),
            OnMissingEngine::Direct => Ok(Some(alloc::vec![Route::Direct])),
        }
    }
}

/// The operating system: WinHTTP where there is one, then the system's settings.
#[derive(Debug)]
struct SystemSource {
    given: Option<SystemSettings>,
    settings: ResolverSettings,
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
    fn new(given: Option<SystemSettings>, settings: &ResolverSettings) -> Self {
        Self {
            #[cfg(windows)]
            winhttp: if given.is_none() {
                crate::system::winhttp::WinHttpResolver::new(settings.reader.winhttp_timeout)
            } else {
                None
            },
            given,
            settings: settings.clone(),
            cached: Mutex::new(None),
            answers: Mutex::new(BTreeMap::new()),
        }
    }

    fn cache_len(&self) -> usize {
        self.answers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    fn clear(&self) {
        let mut cached = self
            .cached
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Settings read a moment ago are kept (`gsettings_cache_ttl`); what was loaded from them is not.
        match cached.as_mut() {
            Some(current) if current.at.elapsed() < self.settings.reader.gsettings_cache_ttl => {
                current.slots = Arc::new(self.slots_for(&current.settings));
            }
            _ => *cached = None,
        }
        drop(cached);
        self.answers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }

    /// The PAC sources the settings name, fresh: nothing loaded yet.
    fn slots_for(&self, settings: &SystemSettings) -> Vec<Slot> {
        let mut slots = Vec::new();
        if settings.state != SettingsState::Direct {
            if settings.auto_detect
                || (self.settings.wpad && settings.state == SettingsState::Unset)
            {
                slots.push(Slot::pac(PacSource::Wpad));
            }
            if let Some(url) = &settings.pac_url {
                slots.push(Slot::pac(PacSource::Location(url.clone())));
            }
            if let Some(script) = &settings.pac_script {
                slots.push(Slot::pac(PacSource::Text(script.clone())));
            }
        }
        slots
    }

    /// The settings and the PAC slots built from them, read again after `settings_refresh`.
    fn settings(&self) -> (Arc<SystemSettings>, Arc<Vec<Slot>>) {
        let mut cached = self
            .cached
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(current) = cached.as_ref()
            && (self.given.is_some() || current.at.elapsed() < self.settings.settings_refresh)
        {
            return (Arc::clone(&current.settings), Arc::clone(&current.slots));
        }
        let mut read = self
            .given
            .clone()
            .unwrap_or_else(|| SystemSettings::read_with(&self.settings.reader));
        read.bypass = read
            .bypass
            .clone()
            .with_dns_timeout(self.settings.bypass_dns_timeout)
            .with_interface_cache_ttl(self.settings.interface_cache_ttl);
        let settings = Arc::new(read);
        let slots = Arc::new(self.slots_for(&settings));
        *cached = Some(Cached {
            at: Instant::now(),
            settings: Arc::clone(&settings),
            slots: Arc::clone(&slots),
        });
        (settings, slots)
    }

    fn lookup(&self, inner: &Inner, url: &str, target: &Target) -> Result<Vec<Route>, Error> {
        let Some(ttl) = self
            .settings
            .cache_ttl
            .filter(|_| self.settings.cache_capacity > 0)
        else {
            return self.resolve(inner, url, target);
        };
        {
            let mut answers = self
                .answers
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some((at, routes)) = answers.get(url) {
                if at.elapsed() < ttl {
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
        if answers.len() >= self.settings.cache_capacity
            && let Some(oldest) = answers
                .iter()
                .min_by_key(|(_, (at, _))| *at)
                .map(|(url, _)| url.clone())
        {
            answers.remove(&oldest);
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
