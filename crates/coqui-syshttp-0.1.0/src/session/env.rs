//! Where the session layer's settings come from: a list of prefixes over one environment, and the
//! proxy settings as a [`Resolver`].
//!
//! Every setting is a variable with a full name (`SYSHTTP_PROXY`, `SYSHTTP_HOME`,
//! `SYSHTTP_HOOKS_<KEY>`, and the conventional `HTTPS_PROXY`, `NO_PROXY`, `CA_BUNDLE`), read as
//! `<PREFIX><NAME>` for each prefix in turn, the first one set winning. The prefixes come from
//! `SYSHTTP_PREFIX`, a `:`-separated list taken literally (`AGENTS_` reads `AGENTS_HTTPS_PROXY`);
//! blank or unset is one empty prefix, the names as written. The names are Python syshttp's, so a
//! Python and a Rust program in one environment read the same settings.
//!
//! Nothing is read when an [`HttpEnv`] is made from the process: a variable set later takes effect
//! on the next lookup.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;

use coqui_proxy::settings::ResolverSettings;
use coqui_proxy::source::ProxySource;
use coqui_proxy::{
    BypassRules, Credentials, Dialect, EnvSource, Environment, Proxy, ProxyScheme, Resolver, Route,
    Secret,
};

use super::error::Error;

/// The variable naming the prefixes, `:`-separated. Read as written: it has no prefix of its own.
pub const PREFIX_VAR: &str = "SYSHTTP_PREFIX";
/// A proxy for this package's traffic, distinct from the machine's.
pub const PROXY: &str = "SYSHTTP_PROXY";
/// The credential of [`PROXY`]: a header value sent verbatim.
pub const PROXY_AUTH: &str = "SYSHTTP_PROXY_AUTH";
/// The header [`PROXY_AUTH`] rides in (default `Proxy-Authorization`).
pub const PROXY_AUTH_HEADER: &str = "SYSHTTP_PROXY_AUTH_HEADER";
/// How the proxy is spoken to: `connect` (default) or `prefix[:/endpoint]`.
pub const PROXY_TYPE: &str = "SYSHTTP_PROXY_TYPE";
/// Whether the system's proxy configuration is consulted, as a boolean (default on).
pub const SYSTEM_PROXY: &str = "SYSHTTP_SYSTEM_PROXY";
/// The header value of the machine's proxy.
pub const MACHINE_PROXY_AUTH: &str = "HTTP_PROXY_AUTH";
/// The default name of the header [`PROXY_AUTH`] is sent in.
pub const DEFAULT_PROXY_AUTH_HEADER: &str = "Proxy-Authorization";
/// A PEM file that replaces the system's trust store for origins.
pub const CA_BUNDLE: &str = "CA_BUNDLE";
/// Hosts that share cookies: hosts split by `,`, groups by `;`.
pub const COOKIE_HOSTS: &str = "SYSHTTP_COOKIE_HOSTS";

const PROXY_VARIABLES: [&str; 6] = [
    "http_proxy",
    "https_proxy",
    "ftp_proxy",
    "ws_proxy",
    "wss_proxy",
    "all_proxy",
];

/// What a call overrides of the proxy settings, as curl's `-x`, `--noproxy` and `-U` do.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct ProxyOverrides {
    /// Replaces the settings with this proxy, a plain one whose credential is its own userinfo;
    /// `Some("")` means direct.
    pub proxy: Option<String>,
    /// Replaces `NO_PROXY`; an entry `<-loopback>` turns the loopback bypass off.
    pub no_proxy: Option<String>,
    /// `user:password`, the credential of whichever proxy answers.
    pub proxy_user: Option<Secret>,
}

#[derive(Debug, Clone)]
enum Variables {
    Process,
    Map(Arc<BTreeMap<String, String>>),
}

/// A view of the environment through a list of prefixes.
///
/// Two views with different prefixes or variables never see each other's settings, which is how
/// one process serves several configurations. Cheap to clone.
#[derive(Debug, Clone)]
pub struct HttpEnv {
    prefixes: Option<Vec<String>>,
    variables: Variables,
}

impl Default for HttpEnv {
    fn default() -> Self {
        Self::process()
    }
}

/// Makes an [`HttpEnv`].
#[derive(Debug, Default)]
#[must_use]
pub struct HttpEnvBuilder {
    prefixes: Option<Vec<String>>,
    variables: Option<BTreeMap<String, String>>,
}

impl HttpEnvBuilder {
    /// The prefixes, each used literally and upper-cased (`""` means the names as written; an
    /// empty list reads nothing). Without this the prefixes are read from `SYSHTTP_PREFIX` at each
    /// lookup.
    pub fn prefixes(mut self, prefixes: impl IntoIterator<Item = impl AsRef<str>>) -> Self {
        self.prefixes = Some(
            prefixes
                .into_iter()
                .map(|prefix| prefix.as_ref().to_string())
                .collect(),
        );
        self
    }

    /// The prefixes as one `:`-separated string, as `SYSHTTP_PREFIX` has them.
    pub fn prefix_list(self, list: &str) -> Self {
        self.prefixes(list.split(':'))
    }

    /// The variables to read, instead of the process's, as they are at each lookup.
    pub fn environment<K: Into<String>, V: Into<String>>(
        mut self,
        variables: impl IntoIterator<Item = (K, V)>,
    ) -> Self {
        self.variables = Some(
            variables
                .into_iter()
                .map(|(key, value)| (key.into(), value.into()))
                .collect(),
        );
        self
    }

    /// The view.
    ///
    /// # Errors
    ///
    /// [`Error::Configuration`] naming `prefixes` for a prefix that holds anything but letters,
    /// digits and underscores.
    pub fn build(self) -> Result<HttpEnv, Error> {
        let prefixes = match self.prefixes {
            Some(list) => Some(prefix_list(list.iter().map(String::as_str), "prefixes")?),
            None => None,
        };
        Ok(HttpEnv {
            prefixes,
            variables: self
                .variables
                .map_or(Variables::Process, |map| Variables::Map(Arc::new(map))),
        })
    }
}

fn check_prefix(prefix: &str, source: &str) -> Result<String, Error> {
    let prefix = prefix.trim();
    if !prefix
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        return Err(Error::config(
            source,
            "holds letters, digits and underscores only",
        ));
    }
    Ok(prefix.to_ascii_uppercase())
}

fn prefix_list<'a>(
    prefixes: impl Iterator<Item = &'a str>,
    source: &str,
) -> Result<Vec<String>, Error> {
    let mut found: Vec<String> = Vec::new();
    for prefix in prefixes {
        let prefix = check_prefix(prefix, source)?;
        if !found.contains(&prefix) {
            found.push(prefix);
        }
    }
    Ok(found)
}

impl HttpEnv {
    /// The process's environment, read at each lookup; the prefixes come from `SYSHTTP_PREFIX`.
    #[must_use]
    pub fn process() -> Self {
        Self {
            prefixes: None,
            variables: Variables::Process,
        }
    }

    /// Starts a view with prefixes or variables of its own.
    pub fn builder() -> HttpEnvBuilder {
        HttpEnvBuilder::default()
    }

    fn variable(&self, name: &str) -> Option<String> {
        match &self.variables {
            Variables::Map(map) => map.get(name).cloned(),
            Variables::Process => std::env::var_os(name).and_then(|value| value.into_string().ok()),
        }
    }

    /// The variable `name` as written, no prefix applied (for the platform's own, such as
    /// `LOCALAPPDATA`).
    pub(crate) fn raw(&self, name: &str) -> Option<String> {
        self.variable(name)
    }

    fn all(&self) -> BTreeMap<String, String> {
        match &self.variables {
            Variables::Map(map) => (**map).clone(),
            Variables::Process => std::env::vars_os()
                .filter_map(|(key, value)| {
                    let key = key.into_string().ok()?;
                    // Windows compares variable names without regard to case.
                    let key = if cfg!(windows) {
                        key.to_ascii_uppercase()
                    } else {
                        key
                    };
                    Some((key, value.into_string().ok()?))
                })
                .collect(),
        }
    }

    /// The variables this view reads, as a coqui-proxy environment.
    #[must_use]
    pub fn environment(&self) -> Environment {
        match &self.variables {
            Variables::Process => Environment::process(),
            Variables::Map(map) => {
                Environment::from_pairs(map.iter().map(|(key, value)| (key.clone(), value.clone())))
            }
        }
    }

    /// The prefixes in force, in lookup order: the argument, else `SYSHTTP_PREFIX`, else one empty
    /// prefix.
    ///
    /// # Errors
    ///
    /// [`Error::Configuration`] naming `SYSHTTP_PREFIX` when it is malformed.
    pub fn prefixes(&self) -> Result<Vec<String>, Error> {
        if let Some(prefixes) = &self.prefixes {
            return Ok(prefixes.clone());
        }
        let raw = self.variable(PREFIX_VAR).unwrap_or_default();
        let raw = raw.trim();
        if raw.is_empty() {
            return Ok(alloc::vec![String::new()]);
        }
        prefix_list(raw.split(':'), PREFIX_VAR)
    }

    /// The variable for `name` under the first prefix: what a message names when nothing is set,
    /// and what a child process is given.
    ///
    /// # Errors
    ///
    /// As [`HttpEnv::prefixes`].
    pub fn name(&self, name: &str) -> Result<String, Error> {
        let prefixes = self.prefixes()?;
        Ok(alloc::format!(
            "{}{name}",
            prefixes.first().map_or("", String::as_str)
        ))
    }

    /// `(variable, value)` for the first of `names` set and not blank, under the first prefix
    /// that has one (every name is tried under a prefix before the next prefix); the value
    /// stripped.
    ///
    /// # Errors
    ///
    /// As [`HttpEnv::prefixes`].
    pub fn find(&self, names: &[&str]) -> Result<Option<(String, String)>, Error> {
        for prefix in self.prefixes()? {
            for name in names {
                let variable = alloc::format!("{prefix}{name}");
                if let Some(value) = self.variable(&variable) {
                    let value = value.trim();
                    if !value.is_empty() {
                        return Ok(Some((variable, value.to_string())));
                    }
                }
            }
        }
        Ok(None)
    }

    /// The value [`HttpEnv::find`] finds.
    ///
    /// # Errors
    ///
    /// As [`HttpEnv::prefixes`].
    pub fn get(&self, names: &[&str]) -> Result<Option<String>, Error> {
        Ok(self.find(names)?.map(|(_, value)| value))
    }

    /// `(rest, value, variable)` for every variable named `<PREFIX><stem><rest>` with a non-empty
    /// `rest` and a value that is not blank (stripped), the first prefix winning for a `rest`
    /// (compared upper-cased); in prefix order, then name order.
    ///
    /// # Errors
    ///
    /// As [`HttpEnv::prefixes`].
    pub fn family(&self, stem: &str) -> Result<Vec<(String, String, String)>, Error> {
        let all = self.all();
        let mut seen = BTreeSet::new();
        let mut found = Vec::new();
        for prefix in self.prefixes()? {
            let head = alloc::format!("{prefix}{stem}");
            for (variable, value) in &all {
                if !variable.starts_with(&head) || variable.len() == head.len() {
                    continue;
                }
                let rest = &variable[head.len()..];
                let value = value.trim();
                if !value.is_empty() && seen.insert(rest.to_ascii_uppercase()) {
                    found.push((rest.to_string(), value.to_string(), variable.clone()));
                }
            }
        }
        Ok(found)
    }

    fn variable_named(&self, setting: &str) -> String {
        match self.find(&[setting]) {
            Ok(Some((variable, _))) => variable,
            _ => self.name(setting).unwrap_or_else(|_| setting.to_string()),
        }
    }

    /// `SYSHTTP_SYSTEM_PROXY` as a boolean, default on.
    ///
    /// # Errors
    ///
    /// [`Error::Configuration`] naming the variable for anything but `1`/`true`/`yes`/`on` or
    /// `0`/`false`/`no`/`off` (any case).
    pub fn system_proxy_enabled(&self) -> Result<bool, Error> {
        let Some(raw) = self.get(&[SYSTEM_PROXY])? else {
            return Ok(true);
        };
        match raw.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Ok(true),
            "0" | "false" | "no" | "off" => Ok(false),
            _ => Err(Error::config(
                self.variable_named(SYSTEM_PROXY),
                "must be one of 1/true/yes/on/0/false/no/off",
            )),
        }
    }

    /// The name of the header `SYSHTTP_PROXY_AUTH` is sent in: `SYSHTTP_PROXY_AUTH_HEADER`, else
    /// `Proxy-Authorization`.
    ///
    /// # Errors
    ///
    /// [`Error::Configuration`] for a value with a colon or whitespace.
    pub fn proxy_auth_header(&self) -> Result<String, Error> {
        match self.get(&[PROXY_AUTH_HEADER])? {
            None => Ok(DEFAULT_PROXY_AUTH_HEADER.to_string()),
            Some(name) => {
                if name.contains(':') || name.chars().any(char::is_whitespace) {
                    return Err(Error::config(
                        self.variable_named(PROXY_AUTH_HEADER),
                        "must be a header name",
                    ));
                }
                Ok(name)
            }
        }
    }

    /// `("connect", None)` for a standard proxy, or `("prefix", Some("/endpoint"))` for a
    /// URL-prefix gateway (the endpoint defaults to `/`), from `SYSHTTP_PROXY_TYPE`.
    ///
    /// # Errors
    ///
    /// [`Error::Configuration`] for anything else.
    pub fn proxy_type(&self) -> Result<(String, Option<String>), Error> {
        let Some(raw) = self.get(&[PROXY_TYPE])? else {
            return Ok(("connect".to_string(), None));
        };
        let (kind, endpoint) = raw.split_once(':').unwrap_or((raw.as_str(), ""));
        match kind.trim().to_ascii_lowercase().as_str() {
            "connect" => Ok(("connect".to_string(), None)),
            "prefix" => {
                let endpoint = endpoint.trim();
                let endpoint = if endpoint.is_empty() {
                    "/".to_string()
                } else if endpoint.starts_with('/') {
                    endpoint.to_string()
                } else {
                    alloc::format!("/{endpoint}")
                };
                Ok(("prefix".to_string(), Some(endpoint)))
            }
            _ => Err(Error::config(
                self.variable_named(PROXY_TYPE),
                "must be 'connect' or 'prefix[:/endpoint]'",
            )),
        }
    }

    /// `SYSHTTP_PROXY` as a proxy with its credential and kind, or `None` when it is unset.
    /// `SYSHTTP_PROXY_AUTH` replaces the URL's userinfo; `SYSHTTP_PROXY_TYPE=prefix` makes a
    /// gateway.
    ///
    /// # Errors
    ///
    /// [`Error::Configuration`] naming the variable for a malformed value (never the value),
    /// [`Error::Proxy`] for a proxy URL that cannot be read.
    pub fn configured_proxy(&self) -> Result<Option<Proxy>, Error> {
        let Some(raw) = self.get(&[PROXY])? else {
            return Ok(None);
        };
        let mut found = Proxy::parse(&raw)?;
        let (kind, endpoint) = self.proxy_type()?;
        if kind == "prefix" {
            let tls = match found.scheme() {
                ProxyScheme::Http => false,
                ProxyScheme::Https => true,
                _ => {
                    return Err(Error::config(
                        self.variable_named(PROXY_TYPE),
                        alloc::format!(
                            "prefix needs an http or https {}",
                            self.variable_named(PROXY)
                        ),
                    ));
                }
            };
            let mut builder =
                Proxy::builder(ProxyScheme::Gateway { tls }, found.host(), found.port())
                    .path(endpoint.unwrap_or_else(|| "/".to_string()));
            if let Some(credentials) = found.credentials() {
                builder = builder.credentials(credentials.clone());
            }
            found = builder.build()?;
        }
        let Some((variable, value)) = self.find(&[PROXY_AUTH])? else {
            return Ok(Some(found));
        };
        let value = checked(&variable, value)?;
        let header = self.proxy_auth_header()?;
        Ok(Some(with_header(&found, &header, Secret::new(value))?))
    }

    /// Refuses the proxy settings that cannot be used (a malformed `SYSHTTP_PROXY`,
    /// `SYSHTTP_PROXY_TYPE`, `SYSHTTP_PROXY_AUTH`, `SYSHTTP_PROXY_AUTH_HEADER`,
    /// `SYSHTTP_SYSTEM_PROXY` or `SYSHTTP_PREFIX`), so a client says so when it is made rather than
    /// at its first request.
    ///
    /// # Errors
    ///
    /// The first [`Error::Configuration`] found.
    pub fn check_proxy_settings(&self) -> Result<(), Error> {
        self.system_proxy_enabled()?;
        self.configured_proxy()?;
        if let Some((variable, value)) = self.find(&[MACHINE_PROXY_AUTH])? {
            checked(&variable, value)?;
        }
        Ok(())
    }

    /// The routing of these settings as a [`Resolver`] that reads them at every lookup, in
    /// order: `overrides`, `SYSHTTP_PROXY`, the machine's proxy (the `*_PROXY` variables when any
    /// is set, else the system's configuration unless `SYSHTTP_SYSTEM_PROXY` is off), direct.
    /// `NO_PROXY` hosts go direct before any of it.
    ///
    /// A malformed setting is an error of the lookup that would use it. The system's answer for a
    /// URL is kept for `ResolverSettings::cache_ttl`; the settings themselves are never cached.
    #[must_use]
    pub fn resolver(&self, overrides: ProxyOverrides) -> Resolver {
        self.resolver_with(overrides, ResolverSettings::default())
    }

    /// [`HttpEnv::resolver`] with the resolver's bounds (`settings_refresh`, `failure_memory`,
    /// probing, PAC) given.
    #[must_use]
    pub fn resolver_with(&self, overrides: ProxyOverrides, settings: ResolverSettings) -> Resolver {
        let nothing = Environment::from_pairs(Vec::<(String, String)>::new());
        let system = Resolver::builder()
            .environment(nothing.clone())
            .resolver_settings(settings.clone())
            .build();
        // The answer of this layer is never cached: the settings are read at each lookup.
        let mut outer = settings;
        outer.cache_ttl = None;
        outer.system = false;
        let mut builder = Resolver::builder()
            .environment(nothing)
            .resolver_settings(outer);
        // The implicit loopback bypass is the resolver's own; `<-loopback>` switches it off.
        let no_proxy = overrides
            .no_proxy
            .clone()
            .or_else(|| self.get(&["no_proxy", "NO_PROXY"]).ok().flatten());
        if let Some(text) = &no_proxy
            && text.contains("<-loopback>")
        {
            builder = builder.no_proxy(BypassRules::parse(text, Dialect::NoProxy));
        }
        builder
            .source(EnvRoutes {
                env: self.clone(),
                overrides,
                system,
            })
            .build()
    }
}

/// `value`, the credential read from `variable`, unless it holds a CR, LF or NUL, which would end
/// the header. The message names the variable, never the value.
pub(crate) fn checked(variable: &str, value: String) -> Result<String, Error> {
    if value.contains(['\r', '\n', '\0']) {
        return Err(Error::config(
            variable,
            "must not contain a carriage return, line feed or NUL character",
        ));
    }
    Ok(value)
}

/// `proxy` with `value` sent in `header` besides what it has.
fn with_header(proxy: &Proxy, header: &str, value: Secret) -> Result<Proxy, coqui_proxy::Error> {
    let mut builder = Proxy::builder(proxy.scheme(), proxy.host(), proxy.port());
    if proxy.is_gateway()
        && let Some(path) = proxy.path()
    {
        builder = builder.path(path);
    }
    if let Some(credentials) = proxy.credentials() {
        builder = builder.credentials(credentials.clone());
    }
    for (name, existing) in proxy.headers() {
        if !name.eq_ignore_ascii_case(header) {
            builder = builder.header(name, Secret::new(existing));
        }
    }
    builder.header(header, value).build()
}

/// `proxy` with curl's `-U user:password` as its only credential.
fn with_user(proxy: &Proxy, user: &Secret) -> Result<Proxy, coqui_proxy::Error> {
    let (name, password) = user.expose().split_once(':').unwrap_or((user.expose(), ""));
    let mut builder = Proxy::builder(proxy.scheme(), proxy.host(), proxy.port())
        .credentials(Credentials::new(name, password));
    if proxy.is_gateway()
        && let Some(path) = proxy.path()
    {
        builder = builder.path(path);
    }
    builder.build()
}

/// The settings as a proxy source, read again at every lookup.
#[derive(Debug)]
struct EnvRoutes {
    env: HttpEnv,
    overrides: ProxyOverrides,
    system: Resolver,
}

fn to_proxy(error: Error) -> coqui_proxy::Error {
    match error {
        Error::Proxy(error) => error,
        other => coqui_proxy::Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            other.to_string(),
        )),
    }
}

impl EnvRoutes {
    fn machine(&self, url: &str) -> Result<Vec<Route>, Error> {
        let mut variables: Vec<(String, String)> = Vec::new();
        for name in PROXY_VARIABLES {
            if let Some(value) = self.env.get(&[name, &name.to_ascii_uppercase()])? {
                variables.push((name.to_string(), value));
            }
        }
        let mut routes = if !variables.is_empty() || !self.env.system_proxy_enabled()? {
            // Its variables alone decide.
            EnvSource::builder()
                .environment(Environment::from_pairs(variables))
                .build()
                .routes(url)?
                .unwrap_or_else(|| alloc::vec![Route::Direct])
        } else {
            // A configuration that cannot be read or run is "no opinion", never a failed request.
            self.system
                .lookup(url)
                .unwrap_or_else(|_| alloc::vec![Route::Direct])
        };
        if let Some((variable, value)) = self.env.find(&[MACHINE_PROXY_AUTH])? {
            let value = checked(&variable, value)?;
            for route in &mut routes {
                if let Route::Proxy(proxy) = route
                    && !matches!(proxy.scheme(), ProxyScheme::Socks4 | ProxyScheme::Socks5)
                {
                    *proxy =
                        with_header(proxy, DEFAULT_PROXY_AUTH_HEADER, Secret::new(value.clone()))?;
                }
            }
        }
        Ok(routes)
    }

    fn compute(&self, url: &str) -> Result<Vec<Route>, Error> {
        let no_proxy = match &self.overrides.no_proxy {
            Some(text) => Some(text.clone()),
            None => self.env.get(&["no_proxy", "NO_PROXY"])?,
        };
        if let Some(text) = no_proxy
            && BypassRules::parse(&text, Dialect::NoProxy).matches(url)
        {
            return Ok(alloc::vec![Route::Direct]);
        }
        let mut routes = if let Some(proxy) = &self.overrides.proxy {
            if proxy.is_empty() {
                alloc::vec![Route::Direct]
            } else {
                alloc::vec![Route::Proxy(Proxy::parse(proxy)?)]
            }
        } else if let Some(proxy) = self.env.configured_proxy()? {
            alloc::vec![Route::Proxy(proxy)]
        } else {
            self.machine(url)?
        };
        if let Some(user) = &self.overrides.proxy_user {
            for route in &mut routes {
                if let Route::Proxy(proxy) = route {
                    *proxy = with_user(proxy, user)?;
                }
            }
        }
        Ok(routes)
    }
}

impl ProxySource for EnvRoutes {
    fn routes(&self, url: &str) -> Result<Option<Vec<Route>>, coqui_proxy::Error> {
        self.compute(url).map(Some).map_err(to_proxy)
    }
}
