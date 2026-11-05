//! The web app on a listener of its own, over HTTP on loopback or HTTPS anywhere: the site's files, from a directory or
//! built in, and [`HOST_INDEX`], which names this host's entries for the page to offer. Nothing here pairs a key or vouches
//! for one: a page still proves its key to an entry, and checks the host's.

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;
use std::fs::File;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::{Instant, SystemTime};

use crate::config::{WebConfig, WebTls};
use crate::serve::{Gate, Listening, Serving};

/// Where the site is served when canoa.cfg does not say: loopback, port 7800.
pub const DEFAULT_LISTEN: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 7800);
/// The path of the list of this host's entries: `{"peer", "method", "entries": [{"name", "kind", "url"}]}`.
pub const HOST_INDEX: &str = "/canoa.json";
/// Whether this build holds a site of its own (feature `web`), served when no directory is given.
pub const BUILT_IN: bool = cfg!(feature = "web");
/// Headers one request may carry; past it the request is refused with 431.
const MAX_HEADERS: usize = 64;
/// What one read takes off the connection.
const CHUNK: usize = 4096;
/// How long an accept loop rests after the system refuses a connection, so a full descriptor table is not a busy loop.
const ACCEPT_REST: Duration = Duration::from_millis(50);
/// How often an idle listener looks up to see whether [`WebStopper::stop`] was called.
const STOP_POLL: Duration = Duration::from_millis(25);

/// The site built in by the `web` feature: each file's path under the site, its bytes and its entity tag, by path.
#[cfg(feature = "web")]
static BUILT: &[(&str, &[u8], &str)] = include!(concat!(env!("OUT_DIR"), "/web_site.rs"));

/// What one connection may take. Every limit is a setting; a looser one than the default is logged as weakened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Limits {
    /// Connections at once; past it a new one is closed unanswered. Default 64.
    pub max_connections: usize,
    /// Connections at once from one address, so one peer cannot hold them all. Default 16; behind a proxy every
    /// connection comes from the proxy's address.
    pub max_connections_per_address: usize,
    /// How long one connection may last, however busy; the answer being written then ends with it. Default 600 s.
    pub connection_time: Duration,
    /// The request line and headers, in bytes; past it the request is refused with 431. Default 16 KiB.
    pub max_request_bytes: usize,
    /// Requests one connection may make before it is closed. Default 1000.
    pub max_requests: usize,
    /// For a request's line and headers to arrive, counted from the connection (its TLS handshake included) for the
    /// first, from its first byte for each later one. Default 10 s.
    pub request_time: Duration,
    /// For a kept connection's next request to start. Default 30 s.
    pub idle_time: Duration,
    /// For one write to the connection to make progress. Default 30 s.
    pub send_time: Duration,
    /// How long a browser may keep a file whose name holds a hash (`main.3f9a1b2c.js`) without asking again. Default
    /// 365 days. Every other file is asked about each time (`no-cache`, answered 304 while its tag holds).
    pub immutable_age: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_connections: 64,
            max_connections_per_address: 16,
            connection_time: Duration::from_secs(600),
            max_request_bytes: 16 * 1024,
            max_requests: 1000,
            request_time: Duration::from_secs(10),
            idle_time: Duration::from_secs(30),
            send_time: Duration::from_secs(30),
            immutable_age: Duration::from_secs(365 * 24 * 60 * 60),
        }
    }
}

impl Limits {
    /// Connections at once.
    #[must_use]
    pub const fn with_max_connections(mut self, count: usize) -> Self {
        self.max_connections = count;
        self
    }

    /// Connections at once from one address.
    #[must_use]
    pub const fn with_max_connections_per_address(mut self, count: usize) -> Self {
        self.max_connections_per_address = count;
        self
    }

    /// How long one connection may last.
    #[must_use]
    pub const fn with_connection_time(mut self, time: Duration) -> Self {
        self.connection_time = time;
        self
    }

    /// The request line and headers, in bytes.
    #[must_use]
    pub const fn with_max_request_bytes(mut self, bytes: usize) -> Self {
        self.max_request_bytes = bytes;
        self
    }

    /// Requests one connection may make.
    #[must_use]
    pub const fn with_max_requests(mut self, count: usize) -> Self {
        self.max_requests = count;
        self
    }

    /// For a request's line and headers to arrive.
    #[must_use]
    pub const fn with_request_time(mut self, time: Duration) -> Self {
        self.request_time = time;
        self
    }

    /// For a kept connection's next request to start.
    #[must_use]
    pub const fn with_idle_time(mut self, time: Duration) -> Self {
        self.idle_time = time;
        self
    }

    /// For one write to make progress.
    #[must_use]
    pub const fn with_send_time(mut self, time: Duration) -> Self {
        self.send_time = time;
        self
    }

    /// How long a hashed file may be kept unasked.
    #[must_use]
    pub const fn with_immutable_age(mut self, age: Duration) -> Self {
        self.immutable_age = age;
        self
    }

    /// Each limit looser than the default, as `(setting, value, default)` in canoa.cfg's words.
    fn weakened(&self) -> Vec<(&'static str, String, String)> {
        let strict = Self::default();
        let mut looser = Vec::new();
        let mut count = |setting, value: usize, default: usize| {
            if value > default {
                looser.push((setting, value.to_string(), default.to_string()));
            }
        };
        count(
            "max_connections",
            self.max_connections,
            strict.max_connections,
        );
        count(
            "max_connections_per_address",
            self.max_connections_per_address,
            strict.max_connections_per_address,
        );
        count(
            "max_request_bytes",
            self.max_request_bytes,
            strict.max_request_bytes,
        );
        count("max_requests", self.max_requests, strict.max_requests);
        let mut time = |setting, value: Duration, default: Duration| {
            if value > default {
                looser.push((
                    setting,
                    value.as_secs().to_string(),
                    default.as_secs().to_string(),
                ));
            }
        };
        time("request_seconds", self.request_time, strict.request_time);
        time("idle_seconds", self.idle_time, strict.idle_time);
        time("send_seconds", self.send_time, strict.send_time);
        time(
            "connection_seconds",
            self.connection_time,
            strict.connection_time,
        );
        looser
    }

    /// The first limit that would serve nothing, in canoa.cfg's words.
    fn zero(&self) -> Option<&'static str> {
        [
            ("max_connections", self.max_connections == 0),
            (
                "max_connections_per_address",
                self.max_connections_per_address == 0,
            ),
            ("connection_seconds", self.connection_time.is_zero()),
            ("max_request_bytes", self.max_request_bytes == 0),
            ("max_requests", self.max_requests == 0),
            ("request_seconds", self.request_time.is_zero()),
            ("idle_seconds", self.idle_time.is_zero()),
            ("send_seconds", self.send_time.is_zero()),
        ]
        .into_iter()
        .find_map(|(setting, zero)| zero.then_some(setting))
    }
}

/// Why the web app could not be served. No variant quotes a file's contents.
#[derive(Debug)]
#[non_exhaustive]
pub enum WebError {
    /// No directory was given, and this build holds no site (it was built without the `web` feature).
    NoSite,
    /// The site's directory cannot be served.
    Site {
        /// The directory.
        dir: PathBuf,
        /// Why: absent, not a directory, or without `index.html`.
        why: &'static str,
    },
    /// Plain HTTP was asked for beyond loopback, where it is refused: give a certificate and key.
    PlainBeyondLoopback(SocketAddr),
    /// A limit is zero, which would serve nothing.
    Limit(&'static str),
    /// A `connect` source is not one origin: `http`, `https`, `ws` or `wss` and a host, with an optional port and path.
    Connect,
    /// The certificate or key could not be used.
    Tls {
        /// The file, or the certificate's when the two do not go together.
        file: PathBuf,
        /// Why, in words of the TLS library's, which never quote the key.
        why: String,
    },
    /// The address could not be bound.
    Listen {
        /// The address.
        address: SocketAddr,
        /// What the operating system said.
        source: io::Error,
    },
}

impl fmt::Display for WebError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSite => f.write_str(
                "no site to serve: give [host.web] dir or --web DIR, or build canoa with the web feature",
            ),
            Self::Site { dir, why } => write!(f, "the site {} {why}", dir.display()),
            Self::PlainBeyondLoopback(address) => write!(
                f,
                "the web app listens on {address}, beyond loopback, where plain HTTP is refused: give [host.web] cert and key"
            ),
            Self::Limit(setting) => write!(f, "[host.web] {setting} must be above zero"),
            Self::Connect => f.write_str(
                "a [host.web] connect source is not http, https, ws or wss and a host, with an optional port and path",
            ),
            Self::Tls { file, why } => write!(f, "TLS from {}: {why}", file.display()),
            Self::Listen { address, source } => {
                write!(f, "the web app could not listen on {address}: {source}")
            }
        }
    }
}

impl core::error::Error for WebError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Listen { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Where the files come from.
#[derive(Debug)]
enum Site {
    /// A directory, resolved: every file served is under it once its links are resolved too.
    Dir(PathBuf),
    /// The site built in.
    #[cfg(feature = "web")]
    BuiltIn,
}

/// What every connection shares.
struct Shared {
    site: Site,
    tls: Option<Arc<rustls::ServerConfig>>,
    limits: Limits,
    hosts: Vec<String>,
    policy: String,
    index: Vec<u8>,
    /// Set by [`WebStopper::stop`]: the listener ends, and each connection after the answer it is sending.
    stopped: Arc<AtomicBool>,
}

/// The web app bound to its address, not yet taking connections.
pub struct WebServer {
    listener: TcpListener,
    bound: SocketAddr,
    shared: Arc<Shared>,
}

impl fmt::Debug for WebServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebServer")
            .field("bound", &self.bound)
            .field("site", &self.shared.site)
            .field("tls", &self.shared.tls.is_some())
            .finish_non_exhaustive()
    }
}

impl WebServer {
    /// Binds `web`'s address for the site it names, listing `serving`'s entries at [`HOST_INDEX`] and allowing the page to
    /// connect to them and nowhere else. Nothing is accepted until [`WebServer::run`].
    ///
    /// # Errors
    ///
    /// [`WebError::NoSite`] with no directory in a build without the `web` feature; [`WebError::Site`] for a directory
    /// with no `index.html`; [`WebError::PlainBeyondLoopback`] for an address beyond loopback without TLS;
    /// [`WebError::Limit`] for a zero limit; [`WebError::Tls`] for a certificate or key that cannot be used;
    /// [`WebError::Listen`] for an address that cannot be bound.
    pub fn bind(web: &WebConfig, serving: &Serving) -> Result<Self, WebError> {
        let peer = serving.peer().to_string();
        Self::bind_listing(web, &peer, serving.method().name(), serving.listening())
    }

    /// As [`WebServer::bind`], listing `entries` under the host id `peer`, which proves by `method`.
    pub(crate) fn bind_listing(
        web: &WebConfig,
        peer: &str,
        method: &str,
        entries: &[Listening],
    ) -> Result<Self, WebError> {
        if web.tls.is_none() && !web.listen.ip().is_loopback() {
            return Err(WebError::PlainBeyondLoopback(web.listen));
        }
        if let Some(setting) = web.limits.zero() {
            return Err(WebError::Limit(setting));
        }
        let site = match &web.dir {
            Some(dir) => Site::Dir(site_root(dir, &web.keep_out)?),
            #[cfg(feature = "web")]
            None => Site::BuiltIn,
            #[cfg(not(feature = "web"))]
            None => return Err(WebError::NoSite),
        };
        let tls = web.tls.as_ref().map(tls_config).transpose()?;
        for (setting, value, default) in web.limits.weakened() {
            tracing::warn!("weakened: host.web.{setting} is {value}, looser than {default}");
        }
        if !web.connect.iter().all(|source| connect_source(source)) {
            return Err(WebError::Connect);
        }
        for source in &web.connect {
            tracing::warn!(
                "weakened: host.web.connect lets the page connect to {source}, beyond this host's entries"
            );
        }
        let listener = TcpListener::bind(web.listen).map_err(|source| WebError::Listen {
            address: web.listen,
            source,
        })?;
        let bound = listener.local_addr().map_err(|source| WebError::Listen {
            address: web.listen,
            source,
        })?;
        let connect: Vec<&str> = entries
            .iter()
            .map(|entry| entry.url.as_str())
            .chain(web.connect.iter().map(String::as_str))
            .collect();
        Ok(Self {
            listener,
            bound,
            shared: Arc::new(Shared {
                site,
                tls,
                limits: web.limits,
                hosts: web.hosts.clone(),
                policy: policy(&connect),
                index: host_index(peer, method, entries),
                stopped: Arc::new(AtomicBool::new(false)),
            }),
        })
    }

    /// What ends this server, for a thread other than the one in [`WebServer::run`].
    #[must_use]
    pub fn stopper(&self) -> WebStopper {
        WebStopper(Arc::clone(&self.shared.stopped))
    }

    /// The address it is bound to.
    #[must_use]
    pub const fn local_addr(&self) -> SocketAddr {
        self.bound
    }

    /// The address a browser opens: `http://` or `https://` and the bound address, or the first of `hosts` when bound to
    /// every interface.
    #[must_use]
    pub fn url(&self) -> String {
        let scheme = if self.shared.tls.is_some() {
            "https"
        } else {
            "http"
        };
        match self.shared.hosts.first() {
            Some(name) if self.bound.ip().is_unspecified() => {
                format!("{scheme}://{name}:{}", self.bound.port())
            }
            _ => format!("{scheme}://{}", self.bound),
        }
    }

    /// Takes connections until [`WebStopper::stop`] or the process ends, each on a thread of its own, at most
    /// `max_connections` at once and `max_connections_per_address` from one address. It returns with the address free.
    pub fn run(self) {
        let gate = Gate::new(self.shared.limits.max_connections);
        let addresses = Arc::new(Mutex::new(BTreeMap::<IpAddr, usize>::new()));
        // Waiting in `accept` could never notice a stop.
        if let Err(error) = self.listener.set_nonblocking(true) {
            return tracing::warn!("web: the listener cannot be set up: {error}");
        }
        while !self.shared.stopped.load(Ordering::SeqCst) {
            let (tcp, peer) = match self.listener.accept() {
                Ok(accepted) => accepted,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(STOP_POLL);
                    continue;
                }
                Err(error) => {
                    tracing::warn!("web: a connection failed: {error}");
                    std::thread::sleep(ACCEPT_REST);
                    continue;
                }
            };
            // On some systems an accepted socket takes the listener's mode.
            if let Err(error) = tcp.set_nonblocking(false) {
                tracing::warn!("web: a connection failed: {error}");
                continue;
            }
            let Some(entered) = gate.enter() else {
                tracing::debug!("web: a connection was closed: max_connections are open");
                continue;
            };
            let Some(from) = FromAddress::enter(
                &addresses,
                peer.ip(),
                self.shared.limits.max_connections_per_address,
            ) else {
                tracing::debug!(
                    "web: a connection was closed: max_connections_per_address are open from its address"
                );
                continue;
            };
            let shared = Arc::clone(&self.shared);
            let spawned = std::thread::Builder::new()
                .name("canoa-web".to_owned())
                .spawn(move || {
                    connection(&shared, tcp);
                    drop((entered, from));
                });
            if let Err(error) = spawned {
                tracing::warn!("web: a connection was dropped: {error}");
            }
        }
    }
}

/// Ends a [`WebServer`] from another thread.
#[derive(Clone, Debug)]
pub struct WebStopper(Arc<AtomicBool>);

impl WebStopper {
    /// Ends [`WebServer::run`] within a moment. A connection already open ends after the answer it is sending, or when it
    /// next goes quiet.
    pub fn stop(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// One connection's place among those from its address, given back when dropped.
struct FromAddress {
    addresses: Arc<Mutex<BTreeMap<IpAddr, usize>>>,
    address: IpAddr,
}

impl FromAddress {
    /// A place for one more connection from `address`, or `None` when `ceiling` are open from it.
    fn enter(
        addresses: &Arc<Mutex<BTreeMap<IpAddr, usize>>>,
        address: IpAddr,
        ceiling: usize,
    ) -> Option<Self> {
        let mut open = addresses.lock().unwrap_or_else(PoisonError::into_inner);
        let count = open.entry(address).or_insert(0);
        if *count >= ceiling {
            return None;
        }
        *count += 1;
        Some(Self {
            addresses: Arc::clone(addresses),
            address,
        })
    }
}

impl Drop for FromAddress {
    fn drop(&mut self) {
        let mut open = self
            .addresses
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(count) = open.get_mut(&self.address) {
            *count -= 1;
            if *count == 0 {
                open.remove(&self.address);
            }
        }
    }
}

/// `dir` resolved, holding `index.html` and none of `keep_out`.
fn site_root(dir: &Path, keep_out: &[PathBuf]) -> Result<PathBuf, WebError> {
    let refused = |why| WebError::Site {
        dir: dir.to_owned(),
        why,
    };
    let root = std::fs::canonicalize(dir).map_err(|_| refused("cannot be found"))?;
    if !root.is_dir() {
        return Err(refused("is not a directory"));
    }
    if !root.join("index.html").is_file() {
        return Err(refused("has no index.html"));
    }
    // A directory that does not exist yet is held by no site.
    if keep_out
        .iter()
        .filter_map(|private| std::fs::canonicalize(private).ok())
        .any(|private| private.starts_with(&root))
    {
        return Err(refused(
            "holds a directory the host keeps private, canoa.cfg's or its identity's: serve a directory of the site's own",
        ));
    }
    Ok(root)
}

/// `time` from now, or a year from now when that is past what this clock can count.
fn after(time: Duration) -> Instant {
    let now = Instant::now();
    now.checked_add(time)
        .or_else(|| now.checked_add(Duration::from_secs(365 * 24 * 60 * 60)))
        .unwrap_or(now)
}

/// rustls's server configuration from `tls`'s PEM files, for HTTP/1.1, asking no client certificate.
fn tls_config(tls: &WebTls) -> Result<Arc<rustls::ServerConfig>, WebError> {
    use rustls::pki_types::pem::PemObject as _;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    let failed = |file: &Path, why: String| WebError::Tls {
        file: file.to_owned(),
        why,
    };
    let read = |file: &Path| std::fs::read(file).map_err(|error| failed(file, error.to_string()));
    let chain = CertificateDer::pem_slice_iter(&read(&tls.cert)?)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| failed(&tls.cert, "not PEM certificates".to_owned()))?;
    if chain.is_empty() {
        return Err(failed(&tls.cert, "holds no certificate".to_owned()));
    }
    let key = PrivateKeyDer::from_pem_slice(&read(&tls.key)?)
        .map_err(|_| failed(&tls.key, "holds no PEM private key".to_owned()))?;
    #[cfg(unix)]
    if readable_by_others(&tls.key) {
        tracing::warn!(
            "weakened: host.web.key {} may be read by other users, who could then serve this site as this host",
            tls.key.display()
        );
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|error| failed(&tls.cert, error.to_string()))?
        .with_no_client_auth()
        .with_single_cert(chain, key)
        .map_err(|error| failed(&tls.cert, error.to_string()))?;
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Arc::new(config))
}

/// A Content-Security-Policy source naming one origin: `http`, `https`, `ws` or `wss`, `://`, a host of letters, digits,
/// `-` and `.`, an optional port and an optional path, with nothing that could end the directive or widen it.
pub(crate) fn connect_source(source: &str) -> bool {
    let Some(rest) = ["https://", "http://", "wss://", "ws://"]
        .iter()
        .find_map(|scheme| source.strip_prefix(scheme))
    else {
        return false;
    };
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let (host, port) = authority.split_once(':').unwrap_or((authority, "0"));
    !host.is_empty()
        && host
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'.')
        && !port.is_empty()
        && port.len() <= 5
        && port.bytes().all(|byte| byte.is_ascii_digit())
        && path.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'.' | b'_' | b'~')
        })
}

/// Whether anyone but its owner may read `file`.
#[cfg(unix)]
fn readable_by_others(file: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(file).is_ok_and(|metadata| metadata.permissions().mode() & 0o077 != 0)
}

/// The list at [`HOST_INDEX`]: this host's id and method, and each entry's name, kind and address, as `serve` prints them.
fn host_index(peer: &str, method: &str, entries: &[Listening]) -> Vec<u8> {
    let entries: Vec<_> = entries
        .iter()
        .map(|entry| serde_json::json!({"name": entry.name, "kind": entry.kind, "url": entry.url}))
        .collect();
    serde_json::to_vec(&serde_json::json!({"peer": peer, "method": method, "entries": entries}))
        .unwrap_or_default()
}

/// The Content-Security-Policy every answer carries: scripts, styles, images, fonts and workers from this origin only,
/// WebAssembly compiled from them, and connections to this origin and to `connect`, the host's entries, alone.
fn policy(connect: &[&str]) -> String {
    let mut sources = String::from("'self'");
    for url in connect {
        sources.push(' ');
        sources.push_str(url);
    }
    format!(
        "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; \
         img-src 'self' data: blob:; font-src 'self' data:; media-src 'self' blob:; connect-src {sources}; \
         worker-src 'self' blob:; manifest-src 'self'; object-src 'none'; base-uri 'self'; form-action 'none'; \
         frame-ancestors 'none'"
    )
}

/// A request target's path, split and percent-decoded, or `None` for one that could name anything outside the site: a
/// target not starting with `/`, `.` or `..`, a segment starting or ending with a dot or ending with a space, an empty
/// segment but a last one, a separator, colon or control character, a character Windows refuses in a name, or a name
/// Windows keeps for a device. The query is left out.
fn segments(target: &str) -> Option<Vec<String>> {
    let path = target.split(['?', '#']).next()?;
    let rest = path.strip_prefix('/')?;
    let mut segments = Vec::new();
    if rest.is_empty() {
        return Some(segments);
    }
    let raw: Vec<&str> = rest.split('/').collect();
    for (index, part) in raw.iter().enumerate() {
        if part.is_empty() && index + 1 == raw.len() {
            break;
        }
        let decoded = percent_encoding::percent_decode_str(part)
            .decode_utf8()
            .ok()?;
        if !plain_segment(&decoded) {
            return None;
        }
        segments.push(decoded.into_owned());
    }
    Some(segments)
}

/// A name every platform reads as one name inside its directory.
fn plain_segment(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && !name.ends_with('.')
        && !name.ends_with(' ')
        && !name.chars().any(|c| {
            c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        })
        && !windows_device(name)
}

/// Whether Windows reads `name` as a device, with any extension: `CON`, `PRN`, `AUX`, `NUL`, `CONIN$`, `CONOUT$`,
/// `CLOCK$`, and `COM` or `LPT` with a digit or a superscript one, two or three.
fn windows_device(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).trim_end();
    let upper = stem.to_ascii_uppercase();
    if matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$" | "CLOCK$"
    ) {
        return true;
    }
    let Some(number) = upper
        .strip_prefix("COM")
        .or_else(|| upper.strip_prefix("LPT"))
    else {
        return false;
    };
    let mut chars = number.chars();
    matches!(
        (chars.next(), chars.next()),
        (Some('0'..='9' | '\u{b9}' | '\u{b2}' | '\u{b3}'), None)
    )
}

/// Whether `segments` name an app route, which is answered with `index.html`: a last segment with no extension.
fn is_app_route(segments: &[String]) -> bool {
    segments.last().is_none_or(|last| !last.contains('.'))
}

/// The media type of a file named `name`, by its extension; `application/octet-stream` for one not known.
fn content_type(name: &str) -> &'static str {
    let Some((_, extension)) = name.rsplit_once('.') else {
        return "application/octet-stream";
    };
    match extension.to_ascii_lowercase().as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "txt" => "text/plain; charset=utf-8",
        "json" | "map" => "application/json",
        "webmanifest" => "application/manifest+json",
        "wasm" => "application/wasm",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "mp3" => "audio/mpeg",
        "ogg" => "audio/ogg",
        "wav" => "audio/wav",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}

/// Whether `name` holds a hash: a part of eight hexadecimal digits or more, after the first, split at `.`, `-` and `_`
/// before the extension (`main.3f9a1b2c.js`, `index-3f9a1b2c.css`).
fn is_hashed(name: &str) -> bool {
    let Some((stem, _)) = name.rsplit_once('.') else {
        return false;
    };
    stem.split(['.', '-', '_'])
        .skip(1)
        .any(|part| part.len() >= 8 && part.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

/// Whether a request's `Host` names this host: `localhost`, an IP address, or one of `hosts`, with any port. A name a
/// page's own site resolves to this address (DNS rebinding) is none of these.
fn host_allowed(value: &str, hosts: &[String]) -> bool {
    let port = |text: &str| {
        !text.is_empty() && text.len() <= 5 && text.bytes().all(|byte| byte.is_ascii_digit())
    };
    if let Some(bracketed) = value.strip_prefix('[') {
        let Some((address, after)) = bracketed.split_once(']') else {
            return false;
        };
        return address.parse::<Ipv6Addr>().is_ok()
            && (after.is_empty() || after.strip_prefix(':').is_some_and(port));
    }
    let name = match value.split_once(':') {
        Some((name, after)) if port(after) => name,
        Some(_) => return false,
        None => value,
    };
    name.eq_ignore_ascii_case("localhost")
        || name.parse::<Ipv4Addr>().is_ok()
        || hosts.iter().any(|host| host.eq_ignore_ascii_case(name))
}

/// `time` as HTTP writes a date: `Sun, 06 Nov 1994 08:49:37 GMT`.
fn http_date(time: SystemTime) -> String {
    const DAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let seconds = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let days = seconds / 86_400;
    let of_day = seconds % 86_400;
    // The civil date of a day count from 1970-01-01, in the proleptic Gregorian calendar, by eras of 400 years.
    let shifted = days + 719_468;
    let era = shifted / 146_097;
    let of_era = shifted % 146_097;
    let year_of_era = (of_era - of_era / 1460 + of_era / 36_524 - of_era / 146_096) / 365;
    let day_of_year = of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 2
    } else {
        month_index - 10
    };
    let year = year_of_era + era * 400 + u64::from(month < 2);
    format!(
        "{}, {day:02} {} {year} {:02}:{:02}:{:02} GMT",
        DAYS[usize::try_from(days % 7).unwrap_or(0)],
        MONTHS[usize::try_from(month).unwrap_or(0)],
        of_day / 3600,
        of_day % 3600 / 60,
        of_day % 60
    )
}

/// A connection's socket, whose every read waits only until its deadline and every write at most the send limit, and
/// neither past the connection's lifetime, so a slow or busy peer holds a connection no longer than the limits allow, TLS
/// records included.
struct Timed {
    tcp: TcpStream,
    until: Instant,
    send: Duration,
    end: Instant,
}

impl Timed {
    /// What is left of `wait`, the connection's lifetime counted; an error once nothing is.
    fn left(&self, wait: Duration) -> io::Result<Duration> {
        let left = wait.min(self.end.saturating_duration_since(Instant::now()));
        if left.is_zero() {
            return Err(io::ErrorKind::TimedOut.into());
        }
        Ok(left)
    }
}

impl Read for Timed {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let left = self.left(self.until.saturating_duration_since(Instant::now()))?;
        self.tcp.set_read_timeout(Some(left))?;
        self.tcp.read(buffer)
    }
}

impl Write for Timed {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let left = self.left(self.send)?;
        self.tcp.set_write_timeout(Some(left))?;
        self.tcp.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.tcp.flush()
    }
}

/// A connection, plain or TLS, whose reads end at a deadline.
trait Wire: Read + Write {
    /// The time by which the next reads must have arrived.
    fn until(&mut self, until: Instant);
    /// Ends the connection, saying so in TLS.
    fn finish(&mut self);
}

impl Wire for Timed {
    fn until(&mut self, until: Instant) {
        self.until = until;
    }

    fn finish(&mut self) {
        let _ = self.tcp.shutdown(std::net::Shutdown::Write);
    }
}

impl Wire for rustls::StreamOwned<rustls::ServerConnection, Timed> {
    fn until(&mut self, until: Instant) {
        self.sock.until = until;
    }

    fn finish(&mut self) {
        self.conn.send_close_notify();
        let _ = self.flush();
        self.sock.finish();
    }
}

fn connection(shared: &Shared, tcp: TcpStream) {
    let _ = tcp.set_nodelay(true);
    let timed = Timed {
        tcp,
        until: after(shared.limits.request_time),
        send: shared.limits.send_time,
        end: after(shared.limits.connection_time),
    };
    match &shared.tls {
        None => exchange(shared, timed),
        Some(config) => match rustls::ServerConnection::new(Arc::clone(config)) {
            Ok(session) => exchange(shared, rustls::StreamOwned::new(session, timed)),
            Err(error) => tracing::debug!("web: TLS could not start: {error}"),
        },
    }
}

/// Why a request head was not read.
enum Unread {
    /// The peer closed, failed or went quiet before a request began: nothing to answer.
    Gone,
    /// An answer, after which the connection closes.
    Refuse(u16),
}

/// Reads until `buffer` holds a whole request head, and answers its length.
fn read_head<W: Wire>(
    wire: &mut W,
    buffer: &mut Vec<u8>,
    limits: &Limits,
    first: bool,
) -> Result<usize, Unread> {
    let mut chunk = [0_u8; CHUNK];
    if !first {
        let wait = if buffer.is_empty() {
            limits.idle_time
        } else {
            limits.request_time
        };
        wire.until(after(wait));
    }
    let mut started = !buffer.is_empty() || first;
    loop {
        if !buffer.is_empty() {
            let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
            match httparse::Request::new(&mut headers).parse(buffer) {
                Ok(httparse::Status::Complete(length)) if length > limits.max_request_bytes => {
                    return Err(Unread::Refuse(431));
                }
                Ok(httparse::Status::Complete(length)) => return Ok(length),
                Ok(httparse::Status::Partial) => {}
                Err(httparse::Error::TooManyHeaders) => return Err(Unread::Refuse(431)),
                Err(_) => return Err(Unread::Refuse(400)),
            }
            if buffer.len() >= limits.max_request_bytes {
                return Err(Unread::Refuse(431));
            }
        }
        let read = match wire.read(&mut chunk) {
            Ok(0) => return Err(Unread::Gone),
            Ok(read) => read,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) =>
            {
                return Err(if buffer.is_empty() {
                    Unread::Gone
                } else {
                    Unread::Refuse(408)
                });
            }
            Err(_) => return Err(Unread::Gone),
        };
        if !started {
            started = true;
            wire.until(after(limits.request_time));
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
}

/// Serves requests on one connection until it closes, a limit is reached or an answer closes it.
fn exchange<W: Wire>(shared: &Shared, mut wire: W) {
    let mut buffer = Vec::with_capacity(CHUNK);
    let mut served = 0_usize;
    while !shared.stopped.load(Ordering::SeqCst) {
        let length = match read_head(&mut wire, &mut buffer, &shared.limits, served == 0) {
            Ok(length) => length,
            Err(Unread::Gone) => break,
            Err(Unread::Refuse(status)) => {
                let _ = send(shared, &mut wire, Answer::refusal(status), false, false);
                break;
            }
        };
        served += 1;
        let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
        let mut request = httparse::Request::new(&mut headers);
        if !matches!(
            request.parse(&buffer[..length]),
            Ok(httparse::Status::Complete(_))
        ) {
            break;
        }
        let (mut reply, head) = answer(shared, &request);
        // A name the host serves over HTTPS stays on HTTPS; localhost and addresses, which other servers share, do not.
        if shared.tls.is_some()
            && header(&request, "host").is_some_and(|host| {
                let name = host.split(':').next().unwrap_or(host);
                shared
                    .hosts
                    .iter()
                    .any(|named| named.eq_ignore_ascii_case(name))
            })
        {
            reply
                .headers
                .push(("Strict-Transport-Security", "max-age=31536000".to_owned()));
        }
        let keep = reply.keep
            && request.version == Some(1)
            && served < shared.limits.max_requests
            && !header(&request, "connection").is_some_and(|value| has_token(value, "close"));
        if send(shared, &mut wire, reply, head, keep).is_err() || !keep {
            break;
        }
        buffer.drain(..length);
    }
    wire.finish();
}

/// The value of the one header called `name`, or `None` when there is none.
fn header<'a>(request: &httparse::Request<'_, 'a>, name: &str) -> Option<&'a str> {
    request
        .headers
        .iter()
        .find(|header| header.name.eq_ignore_ascii_case(name))
        .and_then(|header| core::str::from_utf8(header.value).ok())
}

/// How many headers are called `name`.
fn count_headers(request: &httparse::Request<'_, '_>, name: &str) -> usize {
    request
        .headers
        .iter()
        .filter(|header| header.name.eq_ignore_ascii_case(name))
        .count()
}

/// Whether a comma-separated header value holds `token`, in any case.
fn has_token(value: &str, token: &str) -> bool {
    value
        .split(',')
        .any(|part| part.trim().eq_ignore_ascii_case(token))
}

/// What an answer carries after its headers.
enum Body {
    None,
    #[cfg(feature = "web")]
    Bytes(&'static [u8]),
    Owned(Vec<u8>),
    File(File, u64),
}

impl Body {
    fn len(&self) -> u64 {
        match self {
            Self::None => 0,
            #[cfg(feature = "web")]
            Self::Bytes(bytes) => bytes.len() as u64,
            Self::Owned(bytes) => bytes.len() as u64,
            Self::File(_, length) => *length,
        }
    }
}

/// One answer: its status, headers of its own, body, and whether the connection may be kept after it.
struct Answer {
    status: u16,
    headers: Vec<(&'static str, String)>,
    body: Body,
    keep: bool,
}

impl Answer {
    /// An answer of `status` with a line of text saying what it means, after which the connection closes.
    fn refusal(status: u16) -> Self {
        Self {
            status,
            headers: vec![("Content-Type", "text/plain; charset=utf-8".to_owned())],
            body: Body::Owned(format!("{}\n", reason(status)).into_bytes()),
            keep: false,
        }
    }
}

const fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        304 => "Not Modified",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        421 => "Misdirected Request",
        431 => "Request Header Fields Too Large",
        _ => "Error",
    }
}

/// The answer to one request, and whether it was `HEAD`, which is answered without a body.
fn answer(shared: &Shared, request: &httparse::Request<'_, '_>) -> (Answer, bool) {
    let head = request.method == Some("HEAD");
    if !head && request.method != Some("GET") {
        let mut refused = Answer::refusal(405);
        refused.headers.push(("Allow", "GET, HEAD".to_owned()));
        return (refused, false);
    }
    let body_follows = count_headers(request, "transfer-encoding") > 0
        || count_headers(request, "content-length") > 1
        || header(request, "content-length").is_some_and(|length| length.trim() != "0");
    // One Host in every request, HTTP/1.0 included, so none goes unchecked.
    if body_follows || count_headers(request, "host") != 1 {
        return (Answer::refusal(400), head);
    }
    if !header(request, "host").is_some_and(|host| host_allowed(host, &shared.hosts)) {
        return (Answer::refusal(421), head);
    }
    let Some(target) = request.path else {
        return (Answer::refusal(400), head);
    };
    if !target.starts_with('/') {
        return (Answer::refusal(400), head);
    }
    if target.split(['?', '#']).next() == Some(HOST_INDEX) {
        let found = Answer {
            status: 200,
            headers: vec![
                ("Content-Type", "application/json".to_owned()),
                ("Cache-Control", "no-store".to_owned()),
            ],
            body: Body::Owned(shared.index.clone()),
            keep: true,
        };
        return (found, head);
    }
    let Some(segments) = segments(target) else {
        tracing::debug!("web: a path that is not plain was refused");
        return (Answer::refusal(404), head);
    };
    let found = match lookup(&shared.site, &segments) {
        Lookup::Found(file) => Some((file, segments.last().map(String::as_str))),
        Lookup::Refused => None,
        Lookup::Missing if is_app_route(&segments) => {
            match lookup(&shared.site, &["index.html".to_owned()]) {
                Lookup::Found(file) => Some((file, Some("index.html"))),
                Lookup::Missing | Lookup::Refused => None,
            }
        }
        Lookup::Missing => None,
    };
    let Some((file, name)) = found else {
        return (Answer::refusal(404), head);
    };
    let name = name.unwrap_or("index.html");
    let cache = if is_hashed(name) {
        format!(
            "public, max-age={}, immutable",
            shared.limits.immutable_age.as_secs()
        )
    } else {
        "no-cache".to_owned()
    };
    let mut headers = vec![
        ("Content-Type", content_type(name).to_owned()),
        ("Cache-Control", cache),
        ("ETag", file.tag.clone()),
    ];
    if header(request, "if-none-match").is_some_and(|tags| tag_matches(tags, &file.tag)) {
        headers.retain(|(name, _)| *name != "Content-Type");
        return (
            Answer {
                status: 304,
                headers,
                body: Body::None,
                keep: true,
            },
            head,
        );
    }
    (
        Answer {
            status: 200,
            headers,
            body: file.body,
            keep: true,
        },
        head,
    )
}

/// Whether an `If-None-Match` value names `tag`, compared weakly as a `GET` is.
fn tag_matches(tags: &str, tag: &str) -> bool {
    let opaque = |tag: &str| tag.trim().trim_start_matches("W/").to_owned();
    tags.trim() == "*" || tags.split(',').any(|each| opaque(each) == opaque(tag))
}

/// A file found in the site.
struct Found {
    body: Body,
    tag: String,
}

enum Lookup {
    Found(Found),
    /// Nothing is there, or it is not a file.
    Missing,
    /// It resolves outside the site.
    Refused,
}

fn lookup(site: &Site, segments: &[String]) -> Lookup {
    match site {
        Site::Dir(root) => lookup_dir(root, segments),
        #[cfg(feature = "web")]
        Site::BuiltIn => {
            let path = segments.join("/");
            match BUILT.binary_search_by(|(each, _, _)| (*each).cmp(path.as_str())) {
                Ok(at) => {
                    let (_, bytes, tag) = BUILT[at];
                    Lookup::Found(Found {
                        body: Body::Bytes(bytes),
                        tag: tag.to_owned(),
                    })
                }
                Err(_) => Lookup::Missing,
            }
        }
    }
}

/// The file `segments` name under `root`, once every link on the way is resolved; one that resolves outside `root` is
/// refused, whatever it is.
fn lookup_dir(root: &Path, segments: &[String]) -> Lookup {
    let mut path = root.to_path_buf();
    for segment in segments {
        path.push(segment);
    }
    let Ok(real) = std::fs::canonicalize(&path) else {
        return Lookup::Missing;
    };
    let Ok(inside) = real.strip_prefix(root) else {
        tracing::info!("web: a link out of the site was refused");
        return Lookup::Refused;
    };
    // What the name resolved to is held to the rules the request was: a link, or an NTFS short name such as GIT~1, can
    // lead to a dot-name the request could not have named.
    if !inside
        .components()
        .all(|part| part.as_os_str().to_str().is_some_and(plain_segment))
    {
        tracing::info!("web: a name that resolves to one never served was refused");
        return Lookup::Refused;
    }
    let Ok(file) = File::open(&real) else {
        return Lookup::Missing;
    };
    let Ok(metadata) = file.metadata() else {
        return Lookup::Missing;
    };
    if !metadata.is_file() {
        return Lookup::Missing;
    }
    let length = metadata.len();
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map_or(0, |since| since.as_nanos());
    Lookup::Found(Found {
        body: Body::File(file, length),
        tag: format!("W/\"{length:x}-{modified:x}\""),
    })
}

/// Writes `answer`, with the headers every answer carries, and its body unless `head`.
fn send<W: Wire>(
    shared: &Shared,
    wire: &mut W,
    answer: Answer,
    head: bool,
    keep: bool,
) -> io::Result<()> {
    use core::fmt::Write as _;
    let mut text = format!(
        "HTTP/1.1 {} {}\r\nDate: {}\r\n",
        answer.status,
        reason(answer.status),
        http_date(SystemTime::now())
    );
    let length = answer.body.len();
    let common = [
        ("Content-Security-Policy", shared.policy.as_str()),
        ("X-Content-Type-Options", "nosniff"),
        ("Referrer-Policy", "no-referrer"),
        ("X-Frame-Options", "DENY"),
        ("Cross-Origin-Opener-Policy", "same-origin"),
        ("Cross-Origin-Resource-Policy", "same-origin"),
        ("Connection", if keep { "keep-alive" } else { "close" }),
    ];
    for (name, value) in common.iter().copied().chain(
        answer
            .headers
            .iter()
            .map(|(name, value)| (*name, value.as_str())),
    ) {
        let _ = write!(text, "{name}: {value}\r\n");
    }
    if answer.status != 304 {
        let _ = write!(text, "Content-Length: {length}\r\n");
    }
    text.push_str("\r\n");
    tracing::debug!(status = answer.status, "web: answered");
    wire.write_all(text.as_bytes())?;
    if !head {
        match answer.body {
            Body::None => {}
            #[cfg(feature = "web")]
            Body::Bytes(bytes) => wire.write_all(bytes)?,
            Body::Owned(bytes) => wire.write_all(&bytes)?,
            Body::File(file, length) => {
                // A file that shrank while it was sent leaves the length promised unmet: the connection must end.
                let copied = io::copy(&mut file.take(length), wire)?;
                if copied != length {
                    return Err(io::ErrorKind::UnexpectedEof.into());
                }
            }
        }
    }
    wire.flush()
}

#[cfg(test)]
mod tests;
