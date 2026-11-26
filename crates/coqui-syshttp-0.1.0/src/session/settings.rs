//! The session layer's settings: every bound, interval, count and size, with Python syshttp's
//! defaults.

use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::time::Duration;

use ureq::http::Method;

use super::jar::{CookieJar, TokenJar};

/// The default time to connect (10 s).
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// The default time to wait for the server at each step of a reply (60 s).
pub const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(60);

/// Where a client keeps the cookies of the hosts it talks to.
#[derive(Clone, Default)]
#[non_exhaustive]
pub enum CookieStore {
    /// In memory only (the default).
    #[default]
    Memory,
    /// Netscape files under the jar root, one per origin host (its cookie host group's first).
    PerHost,
    /// One Netscape file named by this key for every host.
    Key(String),
    /// A jar of the caller's, one entry per origin host.
    Jar(Arc<dyn CookieJar>),
}

impl core::fmt::Debug for CookieStore {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Memory => formatter.write_str("Memory"),
            Self::PerHost => formatter.write_str("PerHost"),
            Self::Key(key) => formatter.debug_tuple("Key").field(key).finish(),
            Self::Jar(_) => formatter.write_str("Jar(..)"),
        }
    }
}

/// Where a client reads the tokens that become `Authorization` headers.
#[derive(Clone, Default)]
#[non_exhaustive]
pub enum TokenStore {
    /// No tokens (the default).
    #[default]
    None,
    /// Files under the jar root, one per origin host.
    PerHost,
    /// One token file named by this key for every host.
    Key(String),
    /// A jar of the caller's, one entry per origin host.
    Jar(Arc<dyn TokenJar>),
}

impl core::fmt::Debug for TokenStore {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::None => formatter.write_str("None"),
            Self::PerHost => formatter.write_str("PerHost"),
            Self::Key(key) => formatter.debug_tuple("Key").field(key).finish(),
            Self::Jar(_) => formatter.write_str("Jar(..)"),
        }
    }
}

/// When and how a request is sent again.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RetrySettings {
    /// Most retries after the first attempt (3); zero turns retries off.
    pub count: u32,
    /// The wait before the second retry, doubled for each later one (1 s). The first retry is
    /// immediate.
    pub backoff: Duration,
    /// Most a retry waits, whatever `Retry-After` asks (120 s).
    pub backoff_max: Duration,
    /// The reply statuses that are retried (408, 429, 500, 502, 503, 504).
    pub statuses: Vec<u16>,
    /// The methods that are safe to send twice (GET, HEAD, OPTIONS, PUT, DELETE, TRACE,
    /// PROPFIND).
    pub methods: Vec<Method>,
}

impl RetrySettings {
    /// The default retry count.
    pub const DEFAULT_COUNT: u32 = 3;
    /// The default backoff.
    pub const DEFAULT_BACKOFF: Duration = Duration::from_secs(1);
    /// The default cap on a wait before a retry.
    pub const DEFAULT_BACKOFF_MAX: Duration = Duration::from_secs(120);
    /// The statuses curl calls transient: request timeout, too many requests, and the 5xx a retry
    /// can fix.
    pub const TRANSIENT_STATUSES: [u16; 6] = [408, 429, 500, 502, 503, 504];
}

impl Default for RetrySettings {
    fn default() -> Self {
        Self {
            count: Self::DEFAULT_COUNT,
            backoff: Self::DEFAULT_BACKOFF,
            backoff_max: Self::DEFAULT_BACKOFF_MAX,
            statuses: Self::TRANSIENT_STATUSES.to_vec(),
            methods: [
                "GET", "HEAD", "OPTIONS", "PUT", "DELETE", "TRACE", "PROPFIND",
            ]
            .iter()
            .filter_map(|name| Method::from_bytes(name.as_bytes()).ok())
            .collect(),
        }
    }
}

/// The idle connections a transport keeps.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PoolSettings {
    /// Most idle connections kept (8).
    pub max_idle: usize,
    /// Most idle connections kept for one route (1).
    pub max_idle_per_route: usize,
    /// A request that cannot be sent again uses an idle connection only if it carried a reply
    /// within this long (1 s): an older one may have been dropped by the server.
    pub reuse_window: Duration,
    /// How long an idle connection is kept (15 s).
    pub idle_timeout: Duration,
}

impl Default for PoolSettings {
    fn default() -> Self {
        Self {
            max_idle: 8,
            max_idle_per_route: 1,
            reuse_window: Duration::from_secs(1),
            idle_timeout: Duration::from_secs(15),
        }
    }
}

/// How cookies are kept and bounded.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct CookieSettings {
    /// Where cookies are kept ([`CookieStore::Memory`]).
    pub store: CookieStore,
    /// Groups of hosts that share cookies; `None` reads `SYSHTTP_COOKIE_HOSTS` when the client is
    /// built.
    pub hosts: Option<Vec<Vec<String>>>,
    /// The longest lifetime a cookie keeps, a later expiry being cut to it (400 days).
    pub max_age: Duration,
    /// Most cookies kept for one host; the oldest go first (180).
    pub max_per_host: usize,
    /// Most bytes in a cookie's name and value together; a larger cookie is ignored (4096).
    pub max_bytes: usize,
}

impl Default for CookieSettings {
    fn default() -> Self {
        Self {
            store: CookieStore::Memory,
            hosts: None,
            max_age: Duration::from_secs(400 * 24 * 3600),
            max_per_host: 180,
            max_bytes: 4096,
        }
    }
}

/// Where tokens come from.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct TokenSettings {
    /// Where tokens are read ([`TokenStore::None`]).
    pub store: TokenStore,
}

/// Every setting of a [`Client`](super::Client), with Python syshttp's defaults.
///
/// Build it from `Default` and assign fields, or use the [`ClientBuilder`](super::ClientBuilder)
/// setters, which change the same fields.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SessionSettings {
    /// The time to connect (10 s); `None` waits without limit.
    pub connect_timeout: Option<Duration>,
    /// The time to wait for the server at each step of a reply (60 s); `None` waits without limit.
    pub read_timeout: Option<Duration>,
    /// The time a whole request may take, redirects, retries and the reading of the body
    /// included (none). Zero is refused.
    pub deadline: Option<Duration>,
    /// The `User-Agent` (`coqui-syshttp/<version>`).
    pub user_agent: String,
    /// The headers sent with every request, laid over the user agent and under a request's own;
    /// `None` removes the name (`Accept: */*`).
    pub headers: Vec<(String, Option<String>)>,
    /// Most redirects a `GET`, `HEAD` or `OPTIONS` follows (30).
    pub max_redirects: u32,
    /// Retries.
    pub retry: RetrySettings,
    /// Most bytes of a reply read whole, after any content decoding (8 MiB).
    pub max_reply_bytes: usize,
    /// Most decoded bytes one decompression step produces (256 KiB).
    pub decode_chunk: usize,
    /// Most interim (1xx) replies skipped before the reply (8).
    pub max_interim_replies: usize,
    /// Idle connections.
    pub pool: PoolSettings,
    /// Cookies.
    pub cookies: CookieSettings,
    /// Tokens.
    pub tokens: TokenSettings,
    /// How long the system's TLS configuration is reused before it is rebuilt (60 s), so a
    /// certificate added to the system store is picked up.
    pub tls_max_age: Duration,
    /// How long the system's proxy configuration, and its answer for one URL, are reused (60 s).
    pub system_proxy_max_age: Duration,
    /// Most characters of text from the peer that a message shows (120).
    pub peer_text_limit: usize,
}

impl SessionSettings {
    /// The default `User-Agent`.
    #[must_use]
    pub fn default_user_agent() -> String {
        alloc::format!("coqui-syshttp/{}", crate::VERSION)
    }

    /// What these settings do that is looser than the defaults, one line each; an empty list is
    /// the strict configuration.
    #[must_use]
    pub fn non_defaults(&self) -> Vec<String> {
        let defaults = Self::default();
        let mut loose = Vec::new();
        if self.connect_timeout.is_none() || self.read_timeout.is_none() {
            loose.push("timeout(None): a wait without limit".to_string());
        }
        if self.retry.count == 0 {
            loose.push("retries(0): no request is sent again".to_string());
        }
        if self.max_reply_bytes > defaults.max_reply_bytes {
            loose.push(
                "max_reply_bytes above the default: a larger reply is read whole".to_string(),
            );
        }
        if self.max_redirects > defaults.max_redirects {
            loose.push("max_redirects above the default".to_string());
        }
        if self.cookies.max_per_host > defaults.cookies.max_per_host
            || self.cookies.max_bytes > defaults.cookies.max_bytes
        {
            loose.push("cookie bounds above the defaults".to_string());
        }
        loose
    }
}

impl Default for SessionSettings {
    fn default() -> Self {
        Self {
            connect_timeout: Some(DEFAULT_CONNECT_TIMEOUT),
            read_timeout: Some(DEFAULT_READ_TIMEOUT),
            deadline: None,
            user_agent: Self::default_user_agent(),
            headers: alloc::vec![("Accept".to_string(), Some("*/*".to_string()))],
            max_redirects: 30,
            retry: RetrySettings::default(),
            max_reply_bytes: 8 * 1024 * 1024,
            decode_chunk: 256 * 1024,
            max_interim_replies: 8,
            pool: PoolSettings::default(),
            cookies: CookieSettings::default(),
            tokens: TokenSettings::default(),
            tls_max_age: Duration::from_secs(60),
            system_proxy_max_age: Duration::from_secs(60),
            peer_text_limit: 120,
        }
    }
}
