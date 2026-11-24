//! Plain settings types: every bound the library applies, with its default.
//!
//! Each type is data with a `Default`; the builders that use one take it whole
//! ([`ResolverBuilder::resolver_settings`](crate::ResolverBuilder::resolver_settings),
//! [`ProxyConnectorBuilder::settings`](crate::ProxyConnectorBuilder::settings),
//! `PacScriptBuilder::settings`). A `DEFAULT_*` constant names each default.

use core::time::Duration;
use std::path::PathBuf;

use crate::resolver::OnMissingEngine;

/// How a [`ProxyConnector`](crate::ProxyConnector) connects and how much of a proxy's reply it
/// accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ConnectorSettings {
    /// The time limit of each connect and each read or write on the proxy connection (30 s).
    pub timeout: Duration,
    /// The `User-Agent` sent in a `CONNECT` (`coqui-proxy`).
    pub user_agent: String,
    /// The longest reply head read from a proxy (64 KiB).
    pub max_response_head_bytes: usize,
    /// The most headers a proxy's reply may carry (100).
    pub max_response_headers: usize,
    /// The longest reply body drained between the legs of a login (64 KiB).
    pub max_response_body_bytes: usize,
    /// How long a connection attempt may run before the next address is tried alongside it, as
    /// in RFC 8305 (250 ms).
    pub happy_eyeballs_delay: Duration,
}

impl ConnectorSettings {
    /// The default time limit.
    pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
    /// The default `User-Agent`.
    pub const DEFAULT_USER_AGENT: &'static str = "coqui-proxy";
    /// The default longest reply head.
    pub const DEFAULT_MAX_RESPONSE_HEAD_BYTES: usize = 64 * 1024;
    /// The default most reply headers.
    pub const DEFAULT_MAX_RESPONSE_HEADERS: usize = 100;
    /// The default longest drained reply body.
    pub const DEFAULT_MAX_RESPONSE_BODY_BYTES: usize = 64 * 1024;
    /// The default delay before the next address is tried.
    pub const DEFAULT_HAPPY_EYEBALLS_DELAY: Duration = Duration::from_millis(250);
}

impl Default for ConnectorSettings {
    fn default() -> Self {
        Self {
            timeout: Self::DEFAULT_TIMEOUT,
            user_agent: Self::DEFAULT_USER_AGENT.to_string(),
            max_response_head_bytes: Self::DEFAULT_MAX_RESPONSE_HEAD_BYTES,
            max_response_headers: Self::DEFAULT_MAX_RESPONSE_HEADERS,
            max_response_body_bytes: Self::DEFAULT_MAX_RESPONSE_BODY_BYTES,
            happy_eyeballs_delay: Self::DEFAULT_HAPPY_EYEBALLS_DELAY,
        }
    }
}

/// How the programs and files the system's proxy settings live in are read.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReaderSettings {
    /// How long a helper program (`gsettings`, `nmcli`, `dbus-send`, `scutil`) may run (5 s).
    pub command_timeout: Duration,
    /// The directories searched for DHCP lease files (option 252, WPAD).
    pub dhcp_lease_paths: Vec<PathBuf>,
    /// How long a [`Resolver`](crate::Resolver) keeps the settings it read after
    /// [`Resolver::clear`](crate::Resolver::clear) (10 s).
    pub gsettings_cache_ttl: Duration,
    /// The time limit of WinHTTP's name resolution, connect, send and receive (10 s).
    pub winhttp_timeout: Duration,
    /// How long CFNetwork may take to resolve a PAC script on macOS (5 s).
    pub cfnetwork_deadline: Duration,
}

impl ReaderSettings {
    /// The default helper-program time limit.
    pub const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
    /// The default settings reuse after a clear.
    pub const DEFAULT_GSETTINGS_CACHE_TTL: Duration = Duration::from_secs(10);
    /// The default WinHTTP time limit.
    pub const DEFAULT_WINHTTP_TIMEOUT: Duration = Duration::from_secs(10);
    /// The default CFNetwork deadline.
    pub const DEFAULT_CFNETWORK_DEADLINE: Duration = Duration::from_secs(5);

    /// The directories DHCP lease files are searched in by default.
    #[must_use]
    pub fn default_dhcp_lease_paths() -> Vec<PathBuf> {
        [
            "/var/lib/dhcp",
            "/var/lib/dhclient",
            "/var/lib/NetworkManager",
        ]
        .into_iter()
        .map(PathBuf::from)
        .collect()
    }
}

impl Default for ReaderSettings {
    fn default() -> Self {
        Self {
            command_timeout: Self::DEFAULT_COMMAND_TIMEOUT,
            dhcp_lease_paths: Self::default_dhcp_lease_paths(),
            gsettings_cache_ttl: Self::DEFAULT_GSETTINGS_CACHE_TTL,
            winhttp_timeout: Self::DEFAULT_WINHTTP_TIMEOUT,
            cfnetwork_deadline: Self::DEFAULT_CFNETWORK_DEADLINE,
        }
    }
}

/// How PAC scripts and WPAD discovery are run and fetched.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PacSettings {
    /// How long a discovered WPAD answer, found or not, is remembered (300 s).
    pub wpad_cache_ttl: Duration,
    /// The time limit of loading a script and of each call (5 s).
    pub call_timeout: Duration,
    /// How long a script that overran is refused before it is started afresh (30 s).
    pub refusal_after_overrun: Duration,
    /// The loop iterations a script may run in one call (20 000 000).
    pub loop_iterations: u64,
    /// The recursion depth a script may reach (400).
    pub recursion: usize,
    /// The largest script accepted (1 MiB).
    pub max_script_bytes: usize,
    /// The time limit of each step of a script download (10 s).
    pub download_timeout: Duration,
    /// The redirects a download follows (3).
    pub max_redirects: usize,
    /// How long a downloaded script is reused before it is fetched again (300 s).
    pub download_cache_ttl: Duration,
    /// How long each name lookup a builtin makes may take (2 s).
    pub dns_timeout: Duration,
    /// How long `dnsResolve` remembers an answer, a failure included (30 s).
    pub dns_cache_ttl: Duration,
    /// How many names `dnsResolve` remembers (256).
    pub dns_cache_capacity: usize,
    /// The time limit of each WPAD probe (3 s).
    pub wpad_probe_timeout: Duration,
    /// Reduce an `https:` URL to `https://host/` before the script sees it, as browsers do (false).
    pub browser_compatibility: bool,
    /// The TLS configuration for an `https:` PAC location (default: the system's verifier).
    #[cfg(feature = "tls")]
    pub tls: Option<alloc::sync::Arc<rustls::ClientConfig>>,
    /// The directories searched for DHCP lease files and the time limit of helper programs.
    pub reader: ReaderSettings,
}

impl PacSettings {
    /// The default WPAD answer lifetime.
    pub const DEFAULT_WPAD_CACHE_TTL: Duration = Duration::from_secs(300);
    /// The default call time limit.
    pub const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(5);
    /// The default refusal after an overrun.
    pub const DEFAULT_REFUSAL_AFTER_OVERRUN: Duration = Duration::from_secs(30);
    /// The default loop iteration limit.
    pub const DEFAULT_LOOP_ITERATIONS: u64 = 20_000_000;
    /// The default recursion limit.
    pub const DEFAULT_RECURSION: usize = 400;
    /// The default largest script.
    pub const DEFAULT_MAX_SCRIPT_BYTES: usize = 1 << 20;
    /// The default download time limit.
    pub const DEFAULT_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(10);
    /// The default redirect limit.
    pub const DEFAULT_MAX_REDIRECTS: usize = 3;
    /// The default download reuse.
    pub const DEFAULT_DOWNLOAD_CACHE_TTL: Duration = Duration::from_secs(300);
    /// The default builtin name-lookup time limit.
    pub const DEFAULT_DNS_TIMEOUT: Duration = Duration::from_secs(2);
    /// The default `dnsResolve` memory.
    pub const DEFAULT_DNS_CACHE_TTL: Duration = Duration::from_secs(30);
    /// The default `dnsResolve` capacity.
    pub const DEFAULT_DNS_CACHE_CAPACITY: usize = 256;
    /// The default WPAD probe time limit.
    pub const DEFAULT_WPAD_PROBE_TIMEOUT: Duration = Duration::from_secs(3);
}

impl Default for PacSettings {
    fn default() -> Self {
        Self {
            wpad_cache_ttl: Self::DEFAULT_WPAD_CACHE_TTL,
            call_timeout: Self::DEFAULT_CALL_TIMEOUT,
            refusal_after_overrun: Self::DEFAULT_REFUSAL_AFTER_OVERRUN,
            loop_iterations: Self::DEFAULT_LOOP_ITERATIONS,
            recursion: Self::DEFAULT_RECURSION,
            max_script_bytes: Self::DEFAULT_MAX_SCRIPT_BYTES,
            download_timeout: Self::DEFAULT_DOWNLOAD_TIMEOUT,
            max_redirects: Self::DEFAULT_MAX_REDIRECTS,
            download_cache_ttl: Self::DEFAULT_DOWNLOAD_CACHE_TTL,
            dns_timeout: Self::DEFAULT_DNS_TIMEOUT,
            dns_cache_ttl: Self::DEFAULT_DNS_CACHE_TTL,
            dns_cache_capacity: Self::DEFAULT_DNS_CACHE_CAPACITY,
            wpad_probe_timeout: Self::DEFAULT_WPAD_PROBE_TIMEOUT,
            browser_compatibility: false,
            #[cfg(feature = "tls")]
            tls: None,
            reader: ReaderSettings::default(),
        }
    }
}

/// How a [`Resolver`](crate::Resolver) decides, remembers and rotates.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ResolverSettings {
    /// The time limit of each WPAD probe and PAC download (10 s).
    pub timeout: Duration,
    /// How long the system's settings are trusted before they are read again (60 s).
    pub settings_refresh: Duration,
    /// How long a PAC source that failed is left alone before it is tried again (30 s).
    pub source_retry: Duration,
    /// How long a proxy that refused a connection is tried last (5 min).
    pub failure_memory: Duration,
    /// How long an answer for a URL is reused; `None` asks every time (60 s).
    pub cache_ttl: Option<Duration>,
    /// How many URLs' answers are kept (1024).
    pub cache_capacity: usize,
    /// Rotate the proxies of an answer between lookups, so load spreads (false).
    pub rotate: bool,
    /// How many answers keep a rotation position (1024).
    pub rotation_capacity: usize,
    /// Run WPAD when the system has no settings at all (false).
    pub wpad: bool,
    /// Consult the operating system's settings (true).
    pub system: bool,
    /// What a PAC location does when no JavaScript engine can run it (an error).
    pub on_missing_engine: OnMissingEngine,
    /// Check that a proxy accepts a connection before answering with it (false).
    pub probe: bool,
    /// The time limit of one probe (5 s).
    pub probe_timeout: Duration,
    /// How long a proxy that failed a probe is skipped (30 s).
    pub probe_failure_memory: Duration,
    /// How long this machine's interface addresses are remembered for `<subnet>` (10 s).
    pub interface_cache_ttl: Duration,
    /// The time limit of a name lookup made to match a bypass rule (5 s).
    pub bypass_dns_timeout: Duration,
    /// How the system's settings are read.
    pub reader: ReaderSettings,
    /// How PAC scripts and WPAD run.
    pub pac: PacSettings,
}

impl ResolverSettings {
    /// The default WPAD and PAC time limit.
    pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
    /// The default settings refresh.
    pub const DEFAULT_SETTINGS_REFRESH: Duration = Duration::from_secs(60);
    /// The default PAC source retry.
    pub const DEFAULT_SOURCE_RETRY: Duration = Duration::from_secs(30);
    /// The default failure memory.
    pub const DEFAULT_FAILURE_MEMORY: Duration = Duration::from_secs(300);
    /// The default answer lifetime.
    pub const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(60);
    /// The default answer capacity.
    pub const DEFAULT_CACHE_CAPACITY: usize = 1024;
    /// The default rotation capacity.
    pub const DEFAULT_ROTATION_CAPACITY: usize = 1024;
    /// The default probe time limit.
    pub const DEFAULT_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
    /// The default failed-probe memory.
    pub const DEFAULT_PROBE_FAILURE_MEMORY: Duration = Duration::from_secs(30);
    /// The default interface address memory.
    pub const DEFAULT_INTERFACE_CACHE_TTL: Duration = Duration::from_secs(10);
    /// The default bypass name-lookup time limit.
    pub const DEFAULT_BYPASS_DNS_TIMEOUT: Duration = Duration::from_secs(5);
}

impl Default for ResolverSettings {
    fn default() -> Self {
        Self {
            timeout: Self::DEFAULT_TIMEOUT,
            settings_refresh: Self::DEFAULT_SETTINGS_REFRESH,
            source_retry: Self::DEFAULT_SOURCE_RETRY,
            failure_memory: Self::DEFAULT_FAILURE_MEMORY,
            cache_ttl: Some(Self::DEFAULT_CACHE_TTL),
            cache_capacity: Self::DEFAULT_CACHE_CAPACITY,
            rotate: false,
            rotation_capacity: Self::DEFAULT_ROTATION_CAPACITY,
            wpad: false,
            system: true,
            on_missing_engine: OnMissingEngine::default(),
            probe: false,
            probe_timeout: Self::DEFAULT_PROBE_TIMEOUT,
            probe_failure_memory: Self::DEFAULT_PROBE_FAILURE_MEMORY,
            interface_cache_ttl: Self::DEFAULT_INTERFACE_CACHE_TTL,
            bypass_dns_timeout: Self::DEFAULT_BYPASS_DNS_TIMEOUT,
            reader: ReaderSettings::default(),
            pac: PacSettings::default(),
        }
    }
}

impl ResolverSettings {
    /// The PAC settings a resolver uses: its own download limit and reader settings applied.
    #[cfg(feature = "pac")]
    pub(crate) fn pac_settings(&self) -> PacSettings {
        let mut pac = self.pac.clone();
        pac.download_timeout = self.timeout;
        pac.reader = self.reader.clone();
        pac
    }
}
