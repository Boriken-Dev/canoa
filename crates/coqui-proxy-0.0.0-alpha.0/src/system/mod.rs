//! The operating system's proxy settings: Windows Internet Options, macOS System
//! Configuration, and the GNOME, MATE, KDE and NetworkManager settings on other Unix systems.
//!
//! Environment variables are not read here; see [`crate::Environment`].

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::bypass::BypassRules;
use crate::proxy::{Proxy, ProxyScheme, Route, Target};
pub use crate::settings::ReaderSettings;

#[cfg(target_os = "macos")]
#[expect(
    unsafe_code,
    reason = "CFNetwork and CoreFoundation are C APIs; no maintained safe crate runs a PAC script with a deadline"
)]
mod cfnetwork;
#[cfg(all(unix, feature = "libproxy"))]
#[expect(
    unsafe_code,
    reason = "libproxy is a C library opened at run time; no maintained safe wrapper exists"
)]
mod libproxy;
#[cfg(any(target_os = "macos", test))]
mod macos;
#[cfg(any(all(unix, not(target_os = "macos")), test))]
mod posix;
#[cfg(any(unix, test))]
pub(crate) mod process;
#[cfg(windows)]
#[expect(
    unsafe_code,
    reason = "the WinHTTP calls have no safe binding; each block states its contract"
)]
pub(crate) mod winhttp;
#[cfg(target_os = "macos")]
pub use cfnetwork::CfNetworkSource;
#[cfg(all(unix, feature = "libproxy"))]
pub use libproxy::LibProxySource;
#[cfg(windows)]
pub use winhttp::WinHttpSource;
#[cfg(any(windows, test))]
mod wininet;

/// [`SystemSettings`] as a [`ProxySource`](crate::ProxySource); see [`SystemSettings::to_source`].
#[derive(Debug, Clone)]
pub struct SystemSource {
    resolver: crate::Resolver,
}

impl crate::ProxySource for SystemSource {
    fn routes(&self, url: &str) -> Result<Option<Vec<crate::Route>>, crate::Error> {
        self.resolver.lookup(url).map(Some)
    }
}

/// Where a [`SystemSettings`] was read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum SettingsSource {
    /// Nothing could be read.
    #[default]
    None,
    /// Windows Internet Options.
    Windows,
    /// macOS System Configuration.
    MacOs,
    /// GNOME's `org.gnome.system.proxy`.
    Gnome,
    /// MATE's `org.mate.system.proxy`.
    Mate,
    /// KDE's `kioslaverc`.
    Kde,
    /// NetworkManager's per-connection proxy settings.
    NetworkManager,
}

/// What the settings say to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum SettingsState {
    /// The settings say to use no proxy.
    Direct,
    /// The settings name auto-detection, a PAC or a proxy.
    Configured,
    /// There were no settings to read.
    #[default]
    Unset,
}

/// This machine's proxy settings, as one operating system or desktop stores them.
///
/// `proxies` is keyed by request scheme (`http`, `https`, `ftp`) plus `socks`, the proxy for
/// every scheme without its own entry. `bypass` applies to `proxies` only, as it does in every
/// operating system.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct SystemSettings {
    /// Where the settings were read.
    pub source: SettingsSource,
    /// Whether they name a proxy at all.
    pub state: SettingsState,
    /// Auto-detection (WPAD) is switched on.
    pub auto_detect: bool,
    /// The location of a PAC script.
    pub pac_url: Option<String>,
    /// The text of a PAC script.
    pub pac_script: Option<String>,
    /// The manual proxies.
    pub proxies: BTreeMap<String, Proxy>,
    /// The hosts that bypass `proxies`.
    pub bypass: BypassRules,
}

impl SystemSettings {
    /// Starts settings that say `state`, read from `source`.
    pub fn builder(source: SettingsSource, state: SettingsState) -> SystemSettingsBuilder {
        SystemSettingsBuilder {
            settings: Self {
                source,
                state,
                ..Self::default()
            },
        }
    }

    /// Reads this machine's settings. With none readable, the default: [`SettingsState::Unset`].
    ///
    /// Windows reads the current user's Internet Options; macOS runs `scutil --proxy`; other
    /// Unix systems ask GNOME, MATE, KDE and NetworkManager, the first with a proxy configured
    /// winning, else the first with settings. A program that cannot run reads as no settings.
    pub fn read() -> Self {
        Self::read_with(&ReaderSettings::default())
    }

    /// Reads the operating system's proxy settings, as `reader` says to.
    pub fn read_with(reader: &ReaderSettings) -> Self {
        #[cfg(windows)]
        let settings = winhttp::read_settings();
        #[cfg(target_os = "macos")]
        let settings = macos::read(reader);
        #[cfg(all(unix, not(target_os = "macos")))]
        let settings = posix::read(reader);
        #[cfg(not(any(unix)))]
        let _ = reader;
        #[cfg(not(any(windows, unix)))]
        let settings: Option<Self> = None;
        let settings = settings.unwrap_or_default();
        tracing::debug!(source = ?settings.source, state = ?settings.state, "system proxy settings");
        settings
    }

    /// The manual proxies' answer for `target`: direct for what `bypass` matches and for a
    /// scheme with no proxy.
    /// These settings as a [`ProxySource`](crate::ProxySource): WinHTTP-less, environment-less, the
    /// same decision a [`Resolver`](crate::Resolver) takes at its step 4 (auto-detection when
    /// the settings ask for it or `wpad` is set, the PAC URL, the PAC script, then the manual
    /// proxies and the bypass list). `timeout` bounds each WPAD probe and PAC download.
    #[must_use]
    pub fn to_source(
        &self,
        wpad: bool,
        timeout: core::time::Duration,
        on_missing_engine: crate::OnMissingEngine,
    ) -> SystemSource {
        SystemSource {
            resolver: crate::Resolver::builder()
                .environment(crate::Environment::from_pairs(core::iter::empty::<(
                    String,
                    String,
                )>()))
                .settings(self.clone())
                .wpad(wpad)
                .timeout(timeout)
                .on_missing_engine(on_missing_engine)
                .build(),
        }
    }

    pub(crate) fn manual_routes(&self, target: &Target) -> Vec<Route> {
        if self.proxies.is_empty() || self.bypass.matches_target(target) {
            return alloc::vec![Route::Direct];
        }
        let fallback = match target.scheme.as_str() {
            "ws" => "http",
            "wss" => "https",
            _ => "",
        };
        [target.scheme.as_str(), fallback, "socks"]
            .into_iter()
            .filter(|key| !key.is_empty())
            .find_map(|key| self.proxies.get(key))
            .map_or_else(
                || alloc::vec![Route::Direct],
                |proxy| alloc::vec![Route::Proxy(proxy.clone())],
            )
    }
}

/// Builds a [`SystemSettings`], for a host that keeps its own proxy settings and for tests.
#[derive(Debug, Clone)]
#[must_use]
pub struct SystemSettingsBuilder {
    settings: SystemSettings,
}

impl SystemSettingsBuilder {
    /// Switches auto-detection (WPAD) on or off.
    pub fn auto_detect(mut self, auto_detect: bool) -> Self {
        self.settings.auto_detect = auto_detect;
        self
    }

    /// The location of a PAC script.
    pub fn pac_url(mut self, url: impl Into<String>) -> Self {
        self.settings.pac_url = Some(url.into());
        self
    }

    /// The text of a PAC script.
    pub fn pac_script(mut self, script: impl Into<String>) -> Self {
        self.settings.pac_script = Some(script.into());
        self
    }

    /// The manual proxy for `scheme` (`http`, `https`, `ftp`, or `socks` for the rest).
    pub fn proxy(mut self, scheme: impl Into<String>, proxy: Proxy) -> Self {
        self.settings.proxies.insert(scheme.into(), proxy);
        self
    }

    /// The hosts that bypass the manual proxies.
    pub fn bypass(mut self, bypass: BypassRules) -> Self {
        self.settings.bypass = bypass;
        self
    }

    /// The settings.
    pub fn build(self) -> SystemSettings {
        self.settings
    }
}

/// The proxy a setting names as `host[:port]` or `scheme://host[:port]`. `scheme` applies when
/// the setting has none, or names a bare `socks`, whose version the system fixes.
pub(crate) fn proxy_from(value: &str, scheme: ProxyScheme) -> Option<Proxy> {
    let value = value.trim().trim_end_matches('/');
    if value.is_empty() {
        return None;
    }
    let (scheme, rest) = match value.split_once("://") {
        Some((given, rest)) if !given.eq_ignore_ascii_case("socks") => {
            return Proxy::parse(&alloc::format!("{given}://{rest}")).ok();
        }
        Some((_, rest)) => (scheme, rest),
        None => (scheme, value),
    };
    Proxy::parse(&alloc::format!("{scheme}://{rest}")).ok()
}

/// The proxy named by separate host and port settings. A port that is missing, `0` or not a
/// number means the scheme's default.
#[cfg(any(unix, test))]
pub(crate) fn proxy_at(host: &str, port: &str, scheme: ProxyScheme) -> Option<Proxy> {
    let host = host.trim();
    if host.is_empty() {
        return None;
    }
    let (prefix, bare) = match host.rsplit_once("://") {
        Some((prefix, bare)) => (alloc::format!("{prefix}://"), bare),
        None => (String::new(), host),
    };
    let bare = bare.trim_matches(['[', ']']);
    let bare = if bare.contains(':') {
        alloc::format!("[{bare}]")
    } else {
        bare.to_string()
    };
    let port = port.trim().parse::<u16>().ok().filter(|port| *port != 0);
    let text = match port {
        Some(port) => alloc::format!("{prefix}{bare}:{port}"),
        None => alloc::format!("{prefix}{bare}"),
    };
    proxy_from(&text, scheme)
}

/// Proxies from `host:port` (every scheme) or `scheme=host:port` entries separated by `;` or
/// spaces: the Windows `ProxyServer` form. An `https=` or `ftp=` entry is an HTTP proxy for
/// that scheme; a `socks=` entry is a SOCKS proxy of `socks`.
#[cfg(any(windows, test))]
pub(crate) fn parse_proxy_list(text: &str, socks: ProxyScheme) -> BTreeMap<String, Proxy> {
    let mut proxies = BTreeMap::new();
    for token in text
        .split([';', ' ', '\t'])
        .filter(|token| !token.is_empty())
    {
        let Some((protocol, server)) = token.split_once('=') else {
            if let Some(proxy) = proxy_from(token, ProxyScheme::Http) {
                for key in ["http", "https", "ftp"] {
                    proxies
                        .entry(key.to_string())
                        .or_insert_with(|| proxy.clone());
                }
            }
            continue;
        };
        let protocol = protocol.trim().to_ascii_lowercase();
        if !["http", "https", "ftp", "socks"].contains(&protocol.as_str()) {
            continue;
        }
        let scheme = if protocol == "socks" {
            socks
        } else {
            ProxyScheme::Http
        };
        if let Some(proxy) = proxy_from(server, scheme) {
            proxies.insert(protocol, proxy);
        }
    }
    proxies
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_proxy_list_without_schemes_serves_http_https_and_ftp() {
        let proxies = parse_proxy_list("proxy:8080", ProxyScheme::Socks4);
        assert_eq!(proxies.len(), 3);
        assert_eq!(proxies["https"].to_string(), "http://proxy:8080");
    }

    #[test]
    fn a_proxy_list_with_schemes_keeps_each_and_reads_socks_as_given() {
        let proxies = parse_proxy_list("http=a:80;https=b:443;socks=c:1080", ProxyScheme::Socks4);
        assert_eq!(proxies["http"].host(), "a");
        assert_eq!(proxies["https"].scheme(), ProxyScheme::Http);
        assert_eq!(proxies["socks"].scheme(), ProxyScheme::Socks4);
        assert!(!proxies.contains_key("ftp"));
    }

    #[test]
    fn a_host_and_port_that_is_not_a_number_means_the_schemes_default_port() {
        let proxy = proxy_at("proxy.corp", "0", ProxyScheme::Http).unwrap();
        assert_eq!(proxy.port(), 80);
        let proxy = proxy_at("::1", "3128", ProxyScheme::Http).unwrap();
        assert_eq!(proxy.authority(), "[::1]:3128");
        assert!(proxy_at("", "80", ProxyScheme::Http).is_none());
    }

    #[test]
    fn manual_proxies_answer_by_scheme_with_bypass_first_and_a_scheme_without_one_direct() {
        let settings = SystemSettings {
            proxies: parse_proxy_list("https=secure:443", ProxyScheme::Socks4),
            bypass: BypassRules::parse("corp.example", crate::Dialect::NoProxy),
            ..SystemSettings::default()
        };
        let via = |url: &str| settings.manual_routes(&Target::parse(url).unwrap());
        assert!(matches!(&via("https://x.example/")[0], Route::Proxy(p) if p.host() == "secure"));
        assert_eq!(via("http://x.example/"), [Route::Direct]);
        assert_eq!(via("https://a.corp.example/"), [Route::Direct]);
        assert_eq!(via("https://localhost/"), [Route::Direct]);
    }

    #[test]
    fn a_program_that_is_not_installed_reads_as_no_output() {
        assert_eq!(
            process::run(
                "coqui-proxy-no-such-program",
                &[],
                core::time::Duration::from_secs(5)
            ),
            None
        );
    }
}
