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
    /// Try the next route of an answer when connecting through one fails (true). Off, only the
    /// first route is tried and its error is the result.
    pub failover: bool,
    /// How long a proxy that failed to connect is tried after the others by this connector (300 s).
    pub bad_proxy_ttl: Duration,
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
    /// The default failover.
    pub const DEFAULT_FAILOVER: bool = true;
    /// The default time a failed proxy is tried last.
    pub const DEFAULT_BAD_PROXY_TTL: Duration = Duration::from_secs(300);
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
            failover: Self::DEFAULT_FAILOVER,
            bad_proxy_ttl: Self::DEFAULT_BAD_PROXY_TTL,
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

/// What the in-process [`relay`](crate::relay) listens on and how much of a client it accepts.
///
/// The relay has no access control but its credential, so it only listens on a loopback address;
/// every other bound is a setting, and raising one costs what its line says.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RelaySettings {
    /// The loopback address to listen on (`127.0.0.1`); any other address is refused.
    pub address: core::net::IpAddr,
    /// The port to listen on (0: the system picks a free one).
    pub port: u16,
    /// The most client connections served at once (256). Past it, the asynchronous relay lets new
    /// connections wait in the listener's backlog and the blocking one answers `503` and closes them.
    /// Raising it lets more local clients hold sockets and tasks or threads.
    pub max_connections: usize,
    /// How long a client may take to send a request head (30 s). Raising it lets a client hold a
    /// connection slot while sending slowly.
    pub header_timeout: Duration,
    /// How long a connection, to the client or to the destination, may carry no byte in either
    /// direction before it is closed (300 s). A reply that starts later than this is cut;
    /// raising it keeps idle connections, and the slots they hold, for longer.
    pub idle_timeout: Duration,
    /// The longest request head accepted (64 KiB), at least 8 KiB (the smallest buffer hyper's
    /// HTTP/1 server works with). A longer head is answered `431` and the connection closed.
    pub max_head_bytes: usize,
    /// How long the listener waits after a failed `accept` (a full descriptor table, for
    /// example) before trying again (100 ms).
    pub accept_retry: Duration,
    /// How long dropping or shutting down a [`blocking`](crate::relay::blocking) relay waits for
    /// its threads to end before it stops waiting and leaves them to finish on their own (5 s;
    /// zero: do not wait). A thread that is still dialling a destination ends when the dial
    /// does. Raising it makes a drop block for longer. Meaningless to the asynchronous relay.
    pub shutdown_wait: Duration,
    /// How often a [blocking](crate::relay::blocking) relay looks at both sides of a tunnel to an
    /// `https:` proxy (10 ms), which one thread serves because a TLS stream cannot be read and
    /// written from two. A tunnel through a plain connection uses two threads and no polling.
    /// Lowering it answers sooner and wakes the thread more often. Meaningless to the
    /// asynchronous relay.
    pub poll_interval: Duration,
}

impl RelaySettings {
    /// The default listening address.
    pub const DEFAULT_ADDRESS: core::net::IpAddr =
        core::net::IpAddr::V4(core::net::Ipv4Addr::LOCALHOST);
    /// The default port: any free one.
    pub const DEFAULT_PORT: u16 = 0;
    /// The default most simultaneous connections.
    pub const DEFAULT_MAX_CONNECTIONS: usize = 256;
    /// The default time to send a request head.
    pub const DEFAULT_HEADER_TIMEOUT: Duration = Duration::from_secs(30);
    /// The default idle time.
    pub const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(300);
    /// The default longest request head.
    pub const DEFAULT_MAX_HEAD_BYTES: usize = 64 * 1024;
    /// The default pause after a failed `accept`.
    pub const DEFAULT_ACCEPT_RETRY: Duration = Duration::from_millis(100);
    /// The default wait for a blocking relay's threads when it stops.
    pub const DEFAULT_SHUTDOWN_WAIT: Duration = Duration::from_secs(5);
    /// The default polling interval of a tunnel through an `https:` proxy.
    pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(10);
    /// The smallest request head limit the HTTP/1 server accepts.
    pub const MIN_MAX_HEAD_BYTES: usize = 8 * 1024;
}

impl Default for RelaySettings {
    fn default() -> Self {
        Self {
            address: Self::DEFAULT_ADDRESS,
            port: Self::DEFAULT_PORT,
            max_connections: Self::DEFAULT_MAX_CONNECTIONS,
            header_timeout: Self::DEFAULT_HEADER_TIMEOUT,
            idle_timeout: Self::DEFAULT_IDLE_TIMEOUT,
            max_head_bytes: Self::DEFAULT_MAX_HEAD_BYTES,
            accept_retry: Self::DEFAULT_ACCEPT_RETRY,
            shutdown_wait: Self::DEFAULT_SHUTDOWN_WAIT,
            poll_interval: Self::DEFAULT_POLL_INTERVAL,
        }
    }
}

impl RelaySettings {
    /// What these settings do that is looser than the defaults: an empty list is the strict
    /// configuration.
    #[must_use]
    pub fn non_defaults(&self) -> alloc::vec::Vec<alloc::string::String> {
        let mut loose = alloc::vec::Vec::new();
        if self.max_connections > Self::DEFAULT_MAX_CONNECTIONS {
            loose.push(alloc::format!(
                "max_connections({}): more local connections than the default {}",
                self.max_connections,
                Self::DEFAULT_MAX_CONNECTIONS
            ));
        }
        if self.header_timeout > Self::DEFAULT_HEADER_TIMEOUT {
            loose.push(alloc::format!(
                "header_timeout({:?}): a client may hold a slot longer while sending a head",
                self.header_timeout
            ));
        }
        if self.idle_timeout > Self::DEFAULT_IDLE_TIMEOUT {
            loose.push(alloc::format!(
                "idle_timeout({:?}): an idle connection keeps its slot longer",
                self.idle_timeout
            ));
        }
        if self.max_head_bytes > Self::DEFAULT_MAX_HEAD_BYTES {
            loose.push(alloc::format!(
                "max_head_bytes({}): a client may make the relay buffer a longer head",
                self.max_head_bytes
            ));
        }
        if self.shutdown_wait > Self::DEFAULT_SHUTDOWN_WAIT {
            loose.push(alloc::format!(
                "shutdown_wait({:?}): dropping a blocking relay may block for longer",
                self.shutdown_wait
            ));
        }
        loose
    }
}
