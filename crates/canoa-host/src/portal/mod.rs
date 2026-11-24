//! `canoa portal`: the web app for people who sign in with their account on this machine, each getting a host of their own that
//! runs as them, in their home.
//!
//! The portal runs as a service user with no rights over anyone's files. It checks a password through the operating system's
//! accounts, keeps the signed-in browser in an `HttpOnly`, `Secure`, `SameSite=Strict` cookie, starts that user's
//! `canoa@USER.service`, introduces the browser's own key to it and relays the browser's channel to its sockets. The browser and
//! the host still prove their keys to each other through the relay.

mod access;
mod account;
mod brand;
mod conn;
#[cfg(test)]
mod device;
mod factor;
mod instance;
mod office;
mod page;
mod passkeys;
mod policy;
mod relay;
mod session;
mod settings;
mod signin;
mod steps;

use alloc::sync::Arc;
use core::net::{IpAddr, Ipv4Addr, SocketAddr};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use core::time::Duration;
use std::fs::File;
use std::io::{self, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};
use std::thread;
use std::time::{Instant, SystemTime};

use coqui_access::{methods, purposes};
use coqui_transport_pipe::{PipeName, SocketGroup};

pub use access::{Access, FactorEntry};
pub use conn::Conn;
pub use instance::{Controller, Instances, Lease, Systemd};
pub use session::{Attempt, Session, Sessions, Throttle};
pub use settings::{Entry, MatcherRule, Settings};
pub use signin::{
    Accounts, CertificateStatus, CodeCheck, Enrolment, FactorError, FactorStanding, MAX_PASSWORD,
    Method, NAME_PATTERN, PasskeyAnswer, PasskeyChallenge, PasskeyError, PasskeyInfo, PinError,
    PinState, Presented, SignIn, Standing, valid_user,
};

use crate::pipe::PipeChannel;
use crate::web::{self, Body, Lookup};

/// The session's cookie. The `__Host-` prefix makes a browser refuse it unless it is `Secure`, for the whole site and with no
/// `Domain`, so no sibling site can set it.
pub const COOKIE: &str = "__Host-canoa";
/// The session cookie of an Office task pane's frame, which a third-party context only carries as `SameSite=None; Partitioned`.
/// It names the same kind of session as [`COOKIE`], made by claiming a sign-in that ended in the Office dialog.
pub const PANE_COOKIE: &str = "__Host-canoa-pane";
/// Set while a browser signs in for the Office dialog, so that the sign-in ends there and not at the site.
pub const OFFICE_COOKIE: &str = "__Host-canoa-office";
/// Where the Office dialog starts a sign-in.
pub const OFFICE_SIGN_IN: &str = "/office/signin";
/// Where the task pane asks whether it is signed in, and claims a sign-in the dialog made.
pub const OFFICE_SESSION: &str = "/office/session";
/// The page a sign-in ends at in the Office dialog; its fragment carries the string that claims the sign-in.
pub const OFFICE_DONE: &str = "/office/dialog.html";
/// Where a browser signs in.
pub const LOGIN: &str = "/portal/login";
/// Where a browser asks which ways in the portal accepts; no cookie is needed.
pub const METHODS: &str = "/portal/methods";
/// Where a signed-in person sees the certificate their browser offers, pins it to their account, or removes the pin.
pub const CERTIFICATE: &str = "/portal/certificate";
/// Where a signed-in person, or a sign-in waiting for a one-time code, sets one up.
pub const FACTOR: &str = "/portal/factor";
/// Where a browser asks for the challenge a passkey signs.
pub const PASSKEY: &str = "/portal/passkey";
/// Where a signed-in person sees their passkeys, adds one, names one or removes one.
pub const PASSKEYS: &str = "/portal/passkeys";
/// Where a signed-in browser asks which of the portal's own pages the person may open now.
pub const ACCOUNT: &str = "/portal/account";
/// Where a signed-in browser posts to end its session.
pub const LOGOUT: &str = "/portal/logout";
const MAX_HEAD: usize = 16 * 1024;
const MAX_HEADERS: usize = 64;
/// The largest form a browser may post: a passkey's signed answer is a few kilobytes.
const MAX_FORM: usize = 16 * 1024;
const REST: Duration = Duration::from_millis(50);
/// How often a channel of a session made with a one-time code looks at whether the code is still the account's.
const CODE_LOOK: Duration = Duration::from_secs(3);

/// The ways the portal cannot start.
#[derive(Debug)]
#[non_exhaustive]
pub enum PortalError {
    /// A setting that does not work.
    Settings(String),
    /// The site is missing or has no `index.html`.
    Site(String),
    /// The address or socket could not be bound.
    Listen(io::Error),
}

impl core::fmt::Display for PortalError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Settings(what) => write!(f, "portal setting {what}"),
            Self::Site(what) => write!(f, "portal site: {what}"),
            Self::Listen(error) => write!(f, "the portal could not listen: {error}"),
        }
    }
}

impl core::error::Error for PortalError {}

struct Inner {
    settings: Settings,
    root: PathBuf,
    sessions: Sessions,
    pendings: session::Pendings,
    /// The passkey challenges browsers hold.
    ceremonies: session::Ceremonies,
    throttle: Throttle,
    /// When the portal last warned of an account's failures tripping a limit, by user id: once for each window.
    warned: Mutex<alloc::collections::BTreeMap<u32, Instant>>,
    /// The unlock time the portal last acted on for each account.
    unlocked: Mutex<alloc::collections::BTreeMap<u32, i64>>,
    policy: policy::Policy,
    /// The name, mark and colours of the pages.
    brand: brand::Brand,
    /// The add-in's routes and their policies, when `office` asks for them.
    office: Option<web::Office>,
    /// The sign-ins that ended in the Office dialog and wait for the task pane's frame to claim them.
    handoffs: session::Handoffs,
    instances: Arc<Instances>,
    accounts: Arc<dyn Accounts>,
    stopped: Arc<AtomicBool>,
    connections: AtomicUsize,
    signing_in: AtomicUsize,
}

/// The portal, bound and not yet serving.
pub struct Portal {
    inner: Arc<Inner>,
    tcp: Option<TcpListener>,
    socket: Option<coqui_transport_pipe::Listener>,
}

impl core::fmt::Debug for Portal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Portal").finish_non_exhaustive()
    }
}

/// Ends a running portal.
#[derive(Clone, Debug)]
pub struct Stopper(Arc<AtomicBool>);

impl Stopper {
    /// Ends the accept loops and every relay; each ends within a second.
    pub fn stop(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

fn listen_error(error: impl core::fmt::Display) -> PortalError {
    PortalError::Listen(io::Error::other(error.to_string()))
}

impl Portal {
    /// Binds the portal on `settings.socket` and `settings.listen`, signing people in through `accounts` and running their
    /// hosts through `controller`.
    ///
    /// # Errors
    ///
    /// For settings that do not work, a site that cannot be served, or an address or socket that cannot be bound.
    pub fn bind(
        settings: Settings,
        accounts: Arc<dyn Accounts>,
        controller: Arc<dyn Controller>,
    ) -> Result<Self, PortalError> {
        settings.check().map_err(PortalError::Settings)?;
        let policy =
            policy::Policy::new(&settings, &accounts.methods()).map_err(PortalError::Settings)?;
        tracing::info!("sign-in policy: {}", policy.describe());
        Self::bind_with_policy(settings, accounts, controller, policy)
    }

    /// [`Portal::bind`] with the sign-in policy already built.
    fn bind_with_policy(
        settings: Settings,
        accounts: Arc<dyn Accounts>,
        controller: Arc<dyn Controller>,
        policy: policy::Policy,
    ) -> Result<Self, PortalError> {
        let site = settings
            .site
            .as_deref()
            .ok_or_else(|| PortalError::Site("`site` names no directory".to_owned()))?;
        let brand = match &settings.brand {
            Some(path) => {
                brand::Brand::load(path, settings.adjust_contrast).map_err(PortalError::Settings)?
            }
            None => brand::Brand::builtin(settings.adjust_contrast),
        };
        let root =
            web::site_root(site, &[]).map_err(|error| PortalError::Site(error.to_string()))?;
        let socket = match &settings.socket {
            Some(path) => {
                let mut builder = coqui_transport_pipe::Listener::builder()
                    .name(PipeName::at(path).map_err(listen_error)?)
                    .socket_mode(0o660);
                if let Some(group) = &settings.socket_group {
                    builder = builder.socket_group(
                        group
                            .parse::<SocketGroup>()
                            .map_err(|_| PortalError::Settings("socket_group".to_owned()))?,
                    );
                }
                Some(builder.build().map_err(listen_error)?)
            }
            None => None,
        };
        let tcp = match settings.listen {
            Some(address) => {
                let listener = TcpListener::bind(address).map_err(PortalError::Listen)?;
                listener
                    .set_nonblocking(true)
                    .map_err(PortalError::Listen)?;
                Some(listener)
            }
            None => None,
        };
        let instances = Instances::new(
            controller,
            settings.socket_dir.clone(),
            settings.idle,
            settings.start,
            settings.max_relays_per_user,
            settings.max_hosts,
        );
        instances.adopt();
        if settings.office {
            tracing::warn!(
                "non-default: portal office serves the Office add-in under /office/: Office on the web may frame its pages, and a sign-in may end in the Office dialog with a cookie the task pane's frame carries"
            );
            for source in &settings.office_frames {
                tracing::warn!(
                    "non-default: portal office_frames lets {source} frame the Office add-in's pages"
                );
            }
        }
        let inner = Inner {
            sessions: Sessions::new(
                settings.session,
                settings.max_sessions,
                settings.max_sessions_per_user,
            ),
            warned: Mutex::new(alloc::collections::BTreeMap::new()),
            unlocked: Mutex::new(alloc::collections::BTreeMap::new()),
            pendings: session::Pendings::new(
                settings.pending,
                settings.max_pending_tries,
                settings.max_sessions,
                settings.max_sessions_per_user,
            ),
            ceremonies: session::Ceremonies::new(settings.max_passkey_ceremonies),
            throttle: Throttle::new(
                settings.failure_window,
                settings.max_failures_per_pair,
                settings.max_failures_per_address,
                settings.slow_after_user_failures,
                settings.slow,
            )
            .with_person_limit(settings.max_factor_failures_per_person)
            .with_person_daily(settings.max_factor_failures_per_day, settings.factor_day),
            brand,
            policy,
            office: settings
                .office
                .then(|| web::Office::new(&[], &settings.office_frames)),
            handoffs: session::Handoffs::new(
                settings.office_handoff,
                settings.max_sessions,
                settings.max_sessions_per_user,
            ),
            root,
            instances,
            accounts,
            stopped: Arc::new(AtomicBool::new(false)),
            connections: AtomicUsize::new(0),
            signing_in: AtomicUsize::new(0),
            settings,
        };
        Ok(Self {
            inner: Arc::new(inner),
            tcp,
            socket,
        })
    }

    /// The loopback address taken, when `listen` asked for one.
    #[must_use]
    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.tcp.as_ref().and_then(|tcp| tcp.local_addr().ok())
    }

    /// For another thread to end [`Portal::run`].
    #[must_use]
    pub fn stopper(&self) -> Stopper {
        Stopper(Arc::clone(&self.inner.stopped))
    }

    /// Serves until stopped.
    pub fn run(self) {
        let inner = Arc::clone(&self.inner);
        let sweeper = {
            let inner = Arc::clone(&inner);
            let every = (inner.settings.idle / 10)
                .clamp(Duration::from_millis(200), Duration::from_secs(30));
            thread::Builder::new()
                .name("canoa-portal-idle".to_owned())
                .spawn(move || {
                    let mut last = Instant::now();
                    while !inner.stopped.load(Ordering::SeqCst) {
                        thread::sleep(REST);
                        if last.elapsed() >= every {
                            last = Instant::now();
                            for user in inner.instances.sweep() {
                                tracing::info!("the idle host of {user} was stopped");
                            }
                        }
                    }
                })
        };
        let socket_loop = self.socket.map(|socket| {
            let inner = Arc::clone(&inner);
            thread::Builder::new()
                .name("canoa-portal-socket".to_owned())
                .spawn(move || {
                    while !inner.stopped.load(Ordering::SeqCst) {
                        match socket.accept_timeout(Some(REST)) {
                            Ok(stream) => spawn_connection(&inner, Conn::Pipe(Arc::new(stream))),
                            Err(coqui_transport_pipe::Error::TimedOut) => {}
                            Err(error) => {
                                tracing::warn!("a connection failed: {error}");
                                thread::sleep(REST);
                            }
                        }
                    }
                })
        });
        while !inner.stopped.load(Ordering::SeqCst) {
            let Some(tcp) = &self.tcp else {
                thread::sleep(REST);
                continue;
            };
            match tcp.accept() {
                Ok((stream, _)) => {
                    let _ = stream.set_nonblocking(false);
                    let _ = stream.set_nodelay(true);
                    spawn_connection(&inner, Conn::Tcp(Arc::new(stream)));
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => thread::sleep(REST),
                Err(error) => {
                    tracing::warn!("a connection failed: {error}");
                    thread::sleep(REST);
                }
            }
        }
        for thread in [sweeper.ok(), socket_loop.and_then(Result::ok)]
            .into_iter()
            .flatten()
        {
            let _ = thread.join();
        }
    }
}

fn spawn_connection(inner: &Arc<Inner>, conn: Conn) {
    if inner.connections.fetch_add(1, Ordering::SeqCst) >= inner.settings.max_connections {
        inner.connections.fetch_sub(1, Ordering::SeqCst);
        conn.shutdown();
        return;
    }
    let spawned = thread::Builder::new()
        .name("canoa-portal".to_owned())
        .spawn({
            let inner = Arc::clone(inner);
            let conn = conn.clone();
            move || {
                serve_connection(&inner, conn);
                inner.connections.fetch_sub(1, Ordering::SeqCst);
            }
        });
    if spawned.is_err() {
        inner.connections.fetch_sub(1, Ordering::SeqCst);
        conn.shutdown();
    }
}

/// A request, read.
struct Request {
    method: String,
    target: String,
    version: u8,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(each, _)| each == name)
            .map(|(_, value)| value.as_str())
    }

    fn count(&self, name: &str) -> usize {
        self.headers.iter().filter(|(each, _)| each == name).count()
    }

    fn path(&self) -> &str {
        self.target.split(['?', '#']).next().unwrap_or("")
    }

    fn query(&self) -> &str {
        self.target.split_once('?').map_or("", |(_, query)| query)
    }

    /// The session the browser holds: the site's cookie, else the task pane's.
    fn token(&self) -> Option<String> {
        self.cookie(COOKIE).or_else(|| self.cookie(PANE_COOKIE))
    }

    /// The value of the cookie called `cookie`; none when the browser sent two of that name, since it cannot be told which is
    /// its own.
    fn cookie(&self, cookie: &str) -> Option<String> {
        let mut found = None;
        for (name, value) in &self.headers {
            if name != "cookie" {
                continue;
            }
            for part in value.split(';') {
                if let Some(token) = part.trim().strip_prefix(&format!("{cookie}=")) {
                    if found.is_some() {
                        return None;
                    }
                    found = Some(token.to_owned());
                }
            }
        }
        found
    }
}

/// Why a head was not read: the status to answer with, or nothing when the other end is gone.
enum Unread {
    Gone,
    Refuse(u16),
}

fn read_request(
    conn: &mut Conn,
    buffer: &mut Vec<u8>,
    inner: &Inner,
    first: bool,
) -> Result<Request, Unread> {
    let started = Instant::now();
    let mut chunk = [0_u8; 4096];
    let (length, mut request) = loop {
        let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
        let mut parsed = httparse::Request::new(&mut headers);
        match parsed.parse(buffer) {
            Ok(httparse::Status::Complete(length)) => {
                let request = Request {
                    method: parsed.method.unwrap_or("").to_owned(),
                    target: parsed.path.unwrap_or("").to_owned(),
                    version: parsed.version.unwrap_or(0),
                    headers: parsed
                        .headers
                        .iter()
                        .map(|header| {
                            (
                                header.name.to_ascii_lowercase(),
                                String::from_utf8_lossy(header.value).into_owned(),
                            )
                        })
                        .collect(),
                    body: Vec::new(),
                };
                break (length, request);
            }
            Ok(httparse::Status::Partial) => {}
            Err(httparse::Error::TooManyHeaders) => return Err(Unread::Refuse(431)),
            Err(_) => return Err(Unread::Refuse(400)),
        }
        if buffer.len() >= MAX_HEAD {
            return Err(Unread::Refuse(431));
        }
        let wait = if buffer.is_empty() && !first {
            inner.settings.keep_alive
        } else {
            inner.settings.request.saturating_sub(started.elapsed())
        };
        if wait.is_zero() {
            return Err(Unread::Refuse(408));
        }
        let _ = conn.set_read_timeout(Some(wait));
        match conn.read(&mut chunk) {
            Ok(0) => return Err(Unread::Gone),
            Ok(read) => buffer.extend_from_slice(&chunk[..read]),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                return Err(if buffer.is_empty() {
                    Unread::Gone
                } else {
                    Unread::Refuse(408)
                });
            }
            Err(_) => return Err(Unread::Gone),
        }
    };
    buffer.drain(..length);
    if request.count("transfer-encoding") > 0 || request.count("content-length") > 1 {
        return Err(Unread::Refuse(400));
    }
    let wanted = match request.header("content-length") {
        None => 0,
        Some(text) => text
            .trim()
            .parse::<usize>()
            .map_err(|_| Unread::Refuse(400))?,
    };
    if wanted > MAX_FORM {
        return Err(Unread::Refuse(413));
    }
    while buffer.len() < wanted {
        let _ = conn.set_read_timeout(Some(
            inner
                .settings
                .request
                .saturating_sub(started.elapsed())
                .max(REST),
        ));
        match conn.read(&mut chunk) {
            Ok(0) | Err(_) => return Err(Unread::Refuse(408)),
            Ok(read) => buffer.extend_from_slice(&chunk[..read]),
        }
    }
    request.body = buffer.drain(..wanted).collect();
    Ok(request)
}

/// What an answer carries after its headers.
enum Payload {
    Bytes(Vec<u8>),
    File(File, u64),
}

struct Response {
    status: u16,
    headers: Vec<(&'static str, String)>,
    payload: Payload,
}

const fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        303 => "See Other",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        413 => "Content Too Large",
        421 => "Misdirected Request",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        503 => "Service Unavailable",
        _ => "Error",
    }
}

impl Response {
    fn text(status: u16, kind: &'static str, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            headers: vec![
                ("Content-Type", kind.to_owned()),
                ("Cache-Control", "no-store".to_owned()),
            ],
            payload: Payload::Bytes(body.into()),
        }
    }

    fn json(status: u16, value: &serde_json::Value) -> Self {
        Self::text(status, "application/json", value.to_string())
    }

    fn refusal(status: u16) -> Self {
        Self::text(
            status,
            "text/plain; charset=utf-8",
            format!("{}\n", reason(status)),
        )
    }

    fn redirect(to: &str) -> Self {
        let mut answer = Self::text(303, "text/plain; charset=utf-8", "");
        answer.headers.push(("Location", to.to_owned()));
        answer
    }

    /// This header with `value`, in place of any the answer has under that name.
    fn replacing(mut self, name: &'static str, value: String) -> Self {
        self.headers.retain(|(each, _)| *each != name);
        self.headers.push((name, value));
        self
    }

    fn with(mut self, name: &'static str, value: String) -> Self {
        self.headers.push((name, value));
        self
    }
}

fn write_response(
    inner: &Inner,
    conn: &mut Conn,
    response: Response,
    head: bool,
    keep: bool,
) -> io::Result<()> {
    let length = match &response.payload {
        Payload::Bytes(bytes) => bytes.len() as u64,
        Payload::File(_, length) => *length,
    };
    let has = |name: &str| response.headers.iter().any(|(each, _)| *each == name);
    let mut text = format!(
        "HTTP/1.1 {} {}\r\nDate: {}\r\n",
        response.status,
        reason(response.status),
        web::http_date(SystemTime::now())
    );
    // A 304 has no body, and its length would be the file's, which it does not send.
    if response.status != 304 {
        text.push_str(&format!("Content-Length: {length}\r\n"));
    }
    if !has("Content-Security-Policy") {
        text.push_str(&format!(
            "Content-Security-Policy: {}\r\n",
            web::policy(&[])
        ));
    }
    let common = [
        ("X-Content-Type-Options", "nosniff"),
        // A browser sends `Origin: null` with a form or a fetch posted from a page that sends no referrer, and every post here
        // checks `Origin`, so the portal's own pages say `same-origin`: nothing leaves the site, and the origin is its own.
        ("Referrer-Policy", "same-origin"),
        ("X-Frame-Options", "DENY"),
        ("Cross-Origin-Opener-Policy", "same-origin"),
        ("Cross-Origin-Resource-Policy", "same-origin"),
        ("Connection", if keep { "keep-alive" } else { "close" }),
    ];
    for (name, value) in common
        .iter()
        .filter(|(name, _)| !has(name))
        .map(|(name, value)| (*name, (*value).to_owned()))
        .chain(
            response
                .headers
                .iter()
                .map(|(name, value)| (*name, value.clone())),
        )
    {
        // A value written from a request is never allowed a line break, and an empty one means the header is not sent.
        if value.is_empty() || value.contains(['\r', '\n']) {
            continue;
        }
        text.push_str(&format!("{name}: {value}\r\n"));
    }
    text.push_str("\r\n");
    let _ = conn.set_write_timeout(Some(inner.settings.request.max(Duration::from_secs(30))));
    conn.write_all(text.as_bytes())?;
    if head {
        return Ok(());
    }
    match response.payload {
        Payload::Bytes(bytes) => conn.write_all(&bytes),
        Payload::File(file, length) => io::copy(&mut file.take(length), conn).map(drop),
    }
}

/// The browser's address: a TCP connection's own, which is all that is believed of one anyone local can make; or, on the proxy's
/// socket, the last address in the header the proxy sets that is not one of ours. `None` when there is none (the header is
/// missing, malformed or all proxies of ours): the sign-in policy is told it is unknown, never a placeholder.
fn client_address(inner: &Inner, request: &Request, conn: &Conn) -> Option<IpAddr> {
    if !conn.from_proxy() {
        return conn.peer_ip();
    }
    request
        .header(&inner.settings.client_address_header)
        .and_then(|list| {
            list.rsplit(',')
                .filter_map(|part| part.trim().parse::<IpAddr>().ok())
                .find(|address| {
                    let address = plain(*address);
                    !inner
                        .settings
                        .trusted_proxies
                        .iter()
                        .any(|proxy| plain(*proxy) == address)
                })
        })
}

/// An IPv4 address a dual-stack socket or a header spells as `::ffff:a.b.c.d` is that IPv4 address.
fn plain(address: IpAddr) -> IpAddr {
    match address {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(address, IpAddr::V4),
        other => other,
    }
}

/// The address the throttle counts a try against: the browser's, or, when it is not known, one address shared by every such
/// browser.
fn client_of(inner: &Inner, request: &Request, conn: &Conn) -> IpAddr {
    client_address(inner, request, conn).unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED))
}

/// An address as a log line shows it.
fn shown(address: Option<IpAddr>) -> String {
    address.map_or_else(
        || "an unknown address".to_owned(),
        |address| address.to_string(),
    )
}

fn host_ok(inner: &Inner, request: &Request) -> bool {
    request.count("host") == 1
        && request
            .header("host")
            .is_some_and(|host| web::host_allowed(host, &inner.settings.hosts))
}

/// `https` when the proxy says so, or the host is not `localhost` or an address on this machine; else `http`: what the page's
/// own origin is.
fn scheme_of(request: &Request, conn: &Conn) -> &'static str {
    let host = request.header("host").unwrap_or("localhost");
    let name = host.rsplit_once(':').map_or(host, |(name, port)| {
        if port.bytes().all(|byte| byte.is_ascii_digit()) {
            name
        } else {
            host
        }
    });
    let local = matches!(name, "localhost" | "127.0.0.1" | "[::1]");
    let https = conn.from_proxy()
        && request
            .header("x-forwarded-proto")
            .is_some_and(|proto| proto.eq_ignore_ascii_case("https"));
    if https || !local { "https" } else { "http" }
}

/// Whether the browser says the request is no cross-site one. A browser that sends nothing is believed: `Origin` decides.
fn same_site(request: &Request) -> bool {
    request
        .header("sec-fetch-site")
        .is_none_or(|site| site == "same-origin" || site == "none")
}

/// Whether a request that changes something comes from this site's own page: `Origin` is the site's own scheme and host
/// (`null` is nobody's), and a browser that says the request crossed sites is believed.
fn from_own_page(request: &Request, conn: &Conn) -> bool {
    if request
        .header("sec-fetch-site")
        .is_some_and(|site| site != "same-origin")
    {
        return false;
    }
    let (Some(origin), Some(host)) = (request.header("origin"), request.header("host")) else {
        return false;
    };
    origin.eq_ignore_ascii_case(&format!("{}://{host}", scheme_of(request, conn)))
}

/// What the sign-in policy offers this client before it has passed a step: the page's methods, and what a sign-in's method is
/// decided against. A method this portal does not have is not offered, nor a certificate off the proxy's socket, the one
/// connection whose headers are believed. `None` when the policy could not decide: nothing is offered then.
fn offer_for(inner: &Inner, request: &Request, conn: &Conn) -> Option<policy::Decision> {
    let address = client_address(inner, request, conn);
    match inner.policy.decide(policy::offer_context(address)) {
        Ok(mut decision) => {
            // A step that checks a code for an account is never a first one, whatever the policy says.
            decision
                .offers
                .retain(|offer| !offer.method.needs_account());
            usable(inner, conn, &mut decision);
            Some(decision)
        }
        Err(why) => {
            tracing::warn!("the sign-in policy could not decide: {why}");
            None
        }
    }
}

/// Leaves in `decision` only what this portal can do on this connection: a method it has, and a certificate only on the proxy's
/// socket.
fn usable(inner: &Inner, conn: &Conn, decision: &mut policy::Decision) {
    let have = inner.accounts.methods();
    decision.offers.retain(|offer| {
        have.contains(&offer.method) && (offer.method != Method::Certificate || conn.from_proxy())
    });
}

/// The sign-in page, or, for a browser whose sign-in has passed a step, the page of the next.
fn login_response(
    inner: &Inner,
    request: &Request,
    conn: &Conn,
    status: u16,
    notice: Option<page::Notice>,
    retry: Option<u64>,
) -> Response {
    let mut stale = false;
    if let Some((token, pending)) = steps::pending_of(inner, request, session::Purpose::SignIn) {
        match steps::decide_pending(inner, request, conn, &pending, purposes::SIGN_IN) {
            Ok((decision, _)) if !decision.complete && !decision.offers.is_empty() => {
                let setup = steps::setup_offer(inner, request, conn, &pending, &decision);
                return Response::text(
                    status,
                    "text/html; charset=utf-8",
                    page::render_step(
                        &inner.brand,
                        &pending.user,
                        &decision.methods(),
                        notice,
                        retry,
                        setup,
                    ),
                )
                .with("Content-Security-Policy", page::POLICY.to_owned());
            }
            Err(steps::Stale::Unavailable) => {
                return Response::text(
                    503,
                    "text/html; charset=utf-8",
                    page::render(&inner.brand, &[], Some(page::Notice::Unavailable), None),
                )
                .with("Content-Security-Policy", page::POLICY.to_owned());
            }
            _ => {
                inner.pendings.close(&token);
                stale = true;
            }
        }
    }
    let mut response = login_first(inner, request, conn, status, notice, retry);
    if stale {
        response = response.with("Set-Cookie", steps::clear_pending());
    }
    response
}

/// The page of the first step.
fn login_first(
    inner: &Inner,
    request: &Request,
    conn: &Conn,
    status: u16,
    notice: Option<page::Notice>,
    retry: Option<u64>,
) -> Response {
    let (status, notice, methods) = match offer_for(inner, request, conn) {
        Some(decision) => (status, notice, decision.methods()),
        None => (503, Some(page::Notice::Unavailable), Vec::new()),
    };
    Response::text(
        status,
        "text/html; charset=utf-8",
        page::render(&inner.brand, &methods, notice, retry),
    )
    .with("Content-Security-Policy", page::POLICY.to_owned())
}

fn asset(kind: &'static str, body: &str) -> Response {
    Response::text(200, kind, body).replacing("Cache-Control", "no-cache".to_owned())
}

/// What a browser may ask before it signs in: the ways in this portal accepts here, or, once its sign-in has passed a step, the
/// next ones, which `"step": "next"` says.
fn methods_response(inner: &Inner, request: &Request, conn: &Conn) -> Response {
    if let Some((_, pending)) = steps::pending_of(inner, request, session::Purpose::SignIn) {
        match steps::decide_pending(inner, request, conn, &pending, purposes::SIGN_IN) {
            Ok((decision, _)) if !decision.complete && !decision.offers.is_empty() => {
                let ids: Vec<&str> = decision.methods().into_iter().map(Method::id).collect();
                return Response::json(200, &serde_json::json!({ "step": "next", "methods": ids }));
            }
            Err(steps::Stale::Unavailable) => return Response::refusal(503),
            _ => {}
        }
    }
    let Some(decision) = offer_for(inner, request, conn) else {
        return Response::refusal(503);
    };
    let ids: Vec<&str> = decision.methods().into_iter().map(Method::id).collect();
    Response::json(200, &serde_json::json!({ "methods": ids }))
}

fn serve_connection(inner: &Inner, mut conn: Conn) {
    let mut buffer = Vec::new();
    let mut served = 0_usize;
    while !inner.stopped.load(Ordering::SeqCst) {
        let request = match read_request(&mut conn, &mut buffer, inner, served == 0) {
            Ok(request) => request,
            Err(Unread::Gone) => return,
            Err(Unread::Refuse(status)) => {
                let _ = write_response(inner, &mut conn, Response::refusal(status), false, false);
                return;
            }
        };
        served += 1;
        let head = request.method == "HEAD";
        if request
            .header("upgrade")
            .is_some_and(|value| value.eq_ignore_ascii_case("websocket"))
        {
            upgrade(inner, &mut conn, &request);
            return;
        }
        let response = answer(inner, &request, &conn);
        let keep = request.version == 1
            && served < inner.settings.max_requests
            && !request
                .header("connection")
                .is_some_and(|value| value.to_ascii_lowercase().contains("close"))
            && response.status != 408;
        if write_response(inner, &mut conn, response, head, keep).is_err() || !keep {
            return;
        }
    }
}

fn authed(inner: &Inner, request: &Request) -> Option<Session> {
    inner.sessions.get(&request.token()?)
}

/// A host for the signed-in person, started if it must be: or the answer to give instead. An account that is no longer the one
/// that signed in ends its sessions here.
fn host_for(
    inner: &Inner,
    request: &Request,
    conn: &Conn,
    session: &Session,
) -> Result<Lease, Response> {
    resume(inner, request, conn, session)?;
    inner
        .instances
        .acquire(&session.user, session.uid)
        .map_err(|error| {
            tracing::warn!("{}'s host is not there: {error}", session.user);
            Response::refusal(503)
        })
}

/// Whether a one-time or recovery code is among the steps that made the session.
fn coded(session: &Session) -> bool {
    session
        .done
        .iter()
        .any(|done| done.method == methods::TOTP || done.method == methods::RECOVERY_CODE)
}

/// Whether the session may be used now, asked at each use that starts or reaches a host, shows who the person is or changes their
/// certificate: the account must still be the one that signed in, and the policy decides again with the steps the session was
/// opened with, the account's groups now and the address the browser is at now. A session carried to where its method is no
/// longer accepted ends. When the directory cannot say which groups the account is in, the request is not served (`503`) and
/// the session stands.
fn resume(
    inner: &Inner,
    request: &Request,
    conn: &Conn,
    session: &Session,
) -> Result<(), Response> {
    let groups = match inner.accounts.still(&session.user, session.uid) {
        Standing::Gone => {
            inner.sessions.close_user(&session.user);
            inner.pendings.close_user(&session.user);
            tracing::info!(
                "{} is no longer an account: its sessions ended",
                session.user
            );
            return Err(Response::refusal(401));
        }
        Standing::Present(groups) => groups,
        _ => {
            tracing::warn!(
                "{}'s groups could not be read: the request is not served",
                session.user
            );
            return Err(Response::refusal(503));
        }
    };
    // A session made with a one-time code does not outlive the code: an administrator's reset ends it at its next use.
    if coded(session) {
        let (standing, made) = inner.accounts.factor_record(&session.user, session.uid);
        match standing {
            FactorStanding::NotEnrolled | FactorStanding::Disabled | FactorStanding::Reset => {
                inner.sessions.close_user(&session.user);
                inner.pendings.close_user(&session.user);
                tracing::info!(
                    "{}'s one-time code is gone: its sessions ended",
                    session.user
                );
                return Err(Response::refusal(401));
            }
            FactorStanding::Unknown => {
                tracing::warn!("the one-time codes could not be read: the request is not served");
                return Err(Response::refusal(503));
            }
            _ => {}
        }
        // The code that made the session is the one the account has: one set up again since is another factor.
        if made != session.code_made {
            inner.sessions.close_user(&session.user);
            inner.pendings.close_user(&session.user);
            tracing::info!(
                "{}'s one-time code was set up again: the sessions made with the old one ended",
                session.user
            );
            return Err(Response::refusal(401));
        }
    }
    // A session made with a passkey does not outlive it: one removed, here or by an administrator, ends it at its next use.
    if let Some(credential) = &session.passkey {
        match inner.accounts.passkeys(&session.user, session.uid) {
            Ok(held)
                if held
                    .iter()
                    .any(|each| each.id == *credential && !each.disabled) => {}
            Ok(_) | Err(PasskeyError::Unsupported) => {
                inner
                    .sessions
                    .close_passkey(&session.user, Some(credential));
                tracing::info!(
                    "{}'s passkey is gone: the sessions it opened ended",
                    session.user
                );
                return Err(Response::refusal(401));
            }
            Err(_) => {
                tracing::warn!("the passkeys could not be read: the request is not served");
                return Err(Response::refusal(503));
            }
        }
    }
    let address = client_address(inner, request, conn);
    let context = policy::context(
        purposes::RESUME,
        address,
        Some(policy::person(&session.user, session.uid, &groups)),
        session.done.clone(),
    );
    match inner.policy.decide(context) {
        Ok(decision) if decision.complete => {}
        Ok(decision) => {
            if let Some(token) = request.token() {
                inner.sessions.close(&token);
            }
            tracing::info!(
                "{}'s session ended: not accepted from {} (rule {})",
                session.user,
                shown(address),
                decision.rule_name()
            );
            return Err(Response::refusal(401));
        }
        Err(why) => {
            tracing::warn!("the sign-in policy could not decide: {why}");
            return Err(Response::refusal(503));
        }
    }
    Ok(())
}

fn answer(inner: &Inner, request: &Request, conn: &Conn) -> Response {
    let response = route(inner, request, conn);
    // The Office dialog is a window another origin opened, which Office talks to through that link: while a browser signs in
    // there, no page of the portal cuts it with `Cross-Origin-Opener-Policy`.
    if inner.office.is_some() && request.cookie(OFFICE_COOKIE).is_some() {
        return response.replacing("Cross-Origin-Opener-Policy", String::new());
    }
    response
}

fn route(inner: &Inner, request: &Request, conn: &Conn) -> Response {
    let started = Instant::now();
    if !host_ok(inner, request) || !request.target.starts_with('/') {
        return Response::refusal(421);
    }
    let path = request.path();
    if let Some(response) = office::answer(inner, request, conn, path) {
        return response;
    }
    match (request.method.as_str(), path) {
        ("GET" | "HEAD", LOGIN) => {
            let notice = match request.query() {
                "e=1" => Some(page::Notice::Failed),
                "e=2" => Some(page::Notice::Throttled),
                _ => None,
            };
            login_response(inner, request, conn, 200, notice, None)
        }
        ("GET" | "HEAD", page::CSS_PATH) => asset("text/css; charset=utf-8", inner.brand.css()),
        ("GET" | "HEAD", page::ICON_PATH) => asset("image/svg+xml", inner.brand.icon()),
        ("GET" | "HEAD", page::JS_PATH) => asset("text/javascript; charset=utf-8", page::JS),
        ("GET" | "HEAD", METHODS) => methods_response(inner, request, conn),
        ("GET" | "HEAD", CERTIFICATE) => match authed(inner, request) {
            Some(session) => match resume(inner, request, conn, &session) {
                Ok(()) => certificate_page(inner, request, conn, &session, None),
                // A session that ended is a browser to sign in again.
                Err(refusal) if refusal.status == 401 => Response::redirect(LOGIN),
                Err(refusal) => refusal,
            },
            None => Response::redirect(LOGIN),
        },
        ("POST", CERTIFICATE) => match authed(inner, request) {
            Some(session) if from_own_page(request, conn) => {
                match resume(inner, request, conn, &session) {
                    Ok(()) => certificate_action(inner, request, conn, &session),
                    Err(refusal) => refusal,
                }
            }
            Some(_) => Response::refusal(403),
            None => Response::refusal(401),
        },
        ("POST", LOGIN) => steps::sign_in(inner, request, conn, started),
        ("POST", PASSKEY) => passkeys::challenge(inner, request, conn),
        ("GET" | "HEAD", ACCOUNT) => account::show(inner, request, conn),
        ("GET" | "HEAD", PASSKEYS) => passkeys::show(inner, request, conn),
        ("POST", PASSKEYS) => passkeys::act(inner, request, conn, started),
        ("GET" | "HEAD", FACTOR) => factor::show(inner, request, conn),
        ("POST", FACTOR) => factor::act(inner, request, conn, started),
        ("POST", LOGOUT) => {
            if !from_own_page(request, conn) {
                return Response::refusal(403);
            }
            for cookie in [COOKIE, PANE_COOKIE] {
                if let Some(token) = request.cookie(cookie) {
                    inner.sessions.close(&token);
                }
            }
            Response::redirect(LOGIN)
                .with(
                    "Set-Cookie",
                    format!("{COOKIE}=; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=0"),
                )
                .with(
                    "Set-Cookie",
                    format!(
                        "{PANE_COOKIE}=; Path=/; Secure; HttpOnly; SameSite=None; Partitioned; Max-Age=0"
                    ),
                )
        }
        ("GET", "/portal/session") => match authed(inner, request) {
            Some(session) => match resume(inner, request, conn, &session) {
                Ok(()) => Response::json(200, &serde_json::json!({"user": session.user})),
                Err(refusal) => refusal,
            },
            None => Response::refusal(401),
        },
        ("GET" | "HEAD", web::HOST_INDEX) => match authed(inner, request) {
            // It starts the person's host, so only the site's own page may ask for it.
            Some(_) if !same_site(request) => Response::refusal(403),
            Some(session) => index(inner, request, conn, &session),
            None => Response::refusal(401),
        },
        ("POST", "/portal/pair") => match authed(inner, request) {
            Some(session) if from_own_page(request, conn) => pair(inner, request, conn, &session),
            Some(_) => Response::refusal(403),
            None => Response::refusal(401),
        },
        ("GET" | "HEAD", _) if path.starts_with("/portal/") => Response::refusal(404),
        // A browser asks for these without the cookie, whoever is looking; they hold nothing of anyone's.
        ("GET" | "HEAD", _) if is_public_asset(path) => site(inner, request, true),
        ("GET" | "HEAD", _) => match authed(inner, request) {
            Some(_) => site(inner, request, false),
            None => Response::redirect(LOGIN),
        },
        _ => Response::refusal(405).with("Allow", "GET, HEAD, POST".to_owned()),
    }
}

fn form_field(body: &[u8], name: &str) -> Option<String> {
    url::form_urlencoded::parse(body)
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

/// Answers a failed sign-in no sooner than `failure_floor` after the request began, so that how long it took says nothing of
/// whether the account exists.
fn after_floor(inner: &Inner, started: Instant, response: Response) -> Response {
    let left = inner
        .settings
        .failure_floor
        .saturating_sub(started.elapsed());
    if !left.is_zero() {
        thread::sleep(left);
    }
    response
}

/// What the browser sent that a provider may read of a certificate sign-in: every header but the ones that carry the portal's
/// own or the browser's credentials.
fn presented(request: &Request) -> Vec<(String, String)> {
    request
        .headers
        .iter()
        .filter(|(name, _)| {
            !matches!(
                name.as_str(),
                "cookie" | "authorization" | "proxy-authorization"
            )
        })
        .cloned()
        .collect()
}

/// Whether the password typed to sign this session in, within `enrol`, was typed with the certificate `fingerprint` offered: then
/// the person has confirmed that very certificate, and needs no second password to pin it.
fn confirmed(inner: &Inner, session: &Session, fingerprint: &str) -> bool {
    session.method == Some(Method::Password)
        && session
            .password_at
            .is_some_and(|at| at.elapsed() <= inner.settings.enrol)
        && session.fingerprint.as_deref() == Some(fingerprint)
}

/// The page of the signed-in person's certificates: the one their browser offers here, and the one pinned to their account.
fn certificate_page(
    inner: &Inner,
    request: &Request,
    conn: &Conn,
    session: &Session,
    notice: Option<page::CertNotice>,
) -> Response {
    let headers = if conn.from_proxy() {
        presented(request)
    } else {
        Vec::new()
    };
    let status = inner
        .accounts
        .certificate_status(&session.user, session.uid, &headers);
    let needs_password = status
        .as_ref()
        .and_then(|status| status.presented.as_ref())
        .is_some_and(|(cert, _)| !confirmed(inner, session, &cert.fingerprint));
    let code = match notice {
        Some(page::CertNotice::Problem(_)) => 403,
        _ => 200,
    };
    Response::text(
        code,
        "text/html; charset=utf-8",
        page::render_certificate(
            &inner.brand,
            &session.user,
            status.as_ref(),
            needs_password,
            notice,
            account::links(inner, &session.user, session.uid),
        ),
    )
    .with("Content-Security-Policy", page::POLICY.to_owned())
}

/// `POST /portal/certificate`: `action=enrol` pins the request's certificate to the account, `action=remove` takes the pin away.
/// Pinning needs the password: the one that signed this session in with this very certificate, not long ago, or else the
/// password typed again into the form, through the throttle. So a stolen cookie, or a certificate alone, never adds a way in.
fn certificate_action(
    inner: &Inner,
    request: &Request,
    conn: &Conn,
    session: &Session,
) -> Response {
    if !request
        .header("content-type")
        .is_some_and(|kind| kind.starts_with("application/x-www-form-urlencoded"))
    {
        return Response::refusal(400);
    }
    let started = Instant::now();
    let address = client_of(inner, request, conn);
    let problem = |text| {
        certificate_page(
            inner,
            request,
            conn,
            session,
            Some(page::CertNotice::Problem(text)),
        )
    };
    match form_field(&request.body, "action").as_deref() {
        Some("enrol") => {
            if !conn.from_proxy() {
                return problem("This connection carries no certificate.");
            }
            let headers = presented(request);
            let offered = inner
                .accounts
                .certificate_status(&session.user, session.uid, &headers)
                .and_then(|status| status.presented);
            let Some((cert, _)) = offered else {
                return problem("Your browser did not offer a certificate to this site.");
            };
            if !confirmed(inner, session, &cert.fingerprint) {
                let Some(password) =
                    form_field(&request.body, "password").filter(|text| !text.is_empty())
                else {
                    return problem("Enter your password to add this certificate.");
                };
                if let Err(refusal) =
                    confirm_password(inner, request, conn, session, address, &password, started)
                {
                    return problem(refusal);
                }
            }
            match inner.accounts.pin(&session.user, session.uid, &headers) {
                Ok(()) => {
                    tracing::warn!(
                        "{} pinned certificate {} from {address}",
                        session.user,
                        cert.fingerprint
                    );
                    // What a replaced pin let in does not outlive it.
                    inner
                        .sessions
                        .close_certificates(&session.user, Some(&cert.fingerprint));
                    certificate_page(
                        inner,
                        request,
                        conn,
                        session,
                        Some(page::CertNotice::Done("This certificate now signs you in.")),
                    )
                }
                Err(PinError::NoCertificate) => {
                    problem("Your browser did not offer a certificate to this site.")
                }
                Err(PinError::Expired) => problem("That certificate has expired."),
                Err(PinError::Disabled) => problem(
                    "The certificate pinned to your account is disabled: ask an administrator.",
                ),
                Err(PinError::Taken) => problem("That certificate belongs to another account."),
                Err(PinError::Unsupported) => problem("This portal does not keep certificates."),
                Err(PinError::Failed(why)) => {
                    tracing::warn!("pinning a certificate failed: {why}");
                    problem("The certificate could not be saved.")
                }
            }
        }
        Some("remove") => match inner.accounts.unpin(&session.user) {
            Ok(removed) => {
                if removed {
                    let offered = if conn.from_proxy() {
                        inner
                            .accounts
                            .certificate_status(&session.user, session.uid, &presented(request))
                            .and_then(|status| status.presented)
                            .map(|(cert, _)| cert.fingerprint)
                    } else {
                        None
                    };
                    tracing::warn!(
                        "{} removed their pinned certificate from {address} (the browser offered {})",
                        session.user,
                        offered.as_deref().unwrap_or("none")
                    );
                    inner.sessions.close_certificates(&session.user, None);
                }
                certificate_page(
                    inner,
                    request,
                    conn,
                    session,
                    Some(page::CertNotice::Done("The pinned certificate is removed.")),
                )
            }
            Err(PinError::Disabled) => {
                problem("The certificate pinned to your account is disabled: ask an administrator.")
            }
            Err(PinError::Unsupported) => problem("This portal does not keep certificates."),
            Err(error) => {
                tracing::warn!("removing a pinned certificate failed: {error:?}");
                problem("The certificate could not be removed.")
            }
        },
        _ => Response::refusal(400),
    }
}

/// Checks the password of `session`'s account typed into the enrolment form, through the same throttle and floor as a sign-in.
///
/// # Errors
///
/// What to tell the person.
fn confirm_password(
    inner: &Inner,
    request: &Request,
    conn: &Conn,
    session: &Session,
    address: IpAddr,
    password: &str,
    started: Instant,
) -> Result<(), &'static str> {
    // A password typed here is a password sent: the policy decides whether this client may send one, before the throttle and
    // before any provider.
    let Some(providers) = offer_for(inner, request, conn).and_then(|decision| {
        decision
            .offer(Method::Password)
            .map(|offer| offer.providers.clone())
    }) else {
        tracing::info!(
            "a password for {} from {} is not accepted",
            session.user,
            shown(client_address(inner, request, conn))
        );
        return Err("A password is not accepted from here.");
    };
    let key: String = session.user.to_ascii_lowercase().chars().take(64).collect();
    let Some(attempt) = inner.throttle.begin(address, &key) else {
        tracing::info!(
            "a password for {} from {address} was refused for its failures",
            session.user
        );
        thread::sleep(
            inner
                .settings
                .failure_floor
                .saturating_sub(started.elapsed()),
        );
        return Err("Too many failed attempts. Try again later.");
    };
    if !attempt.wait.is_zero() {
        thread::sleep(attempt.wait);
    }
    if inner.signing_in.fetch_add(1, Ordering::SeqCst) >= inner.settings.max_sign_ins {
        inner.signing_in.fetch_sub(1, Ordering::SeqCst);
        inner.throttle.succeeded(attempt);
        return Err("Busy. Try again in a moment.");
    }
    let result = inner.accounts.sign_in(&session.user, password, &providers);
    inner.signing_in.fetch_sub(1, Ordering::SeqCst);
    match result {
        SignIn::Admitted { user, uid, .. } if user == session.user && uid == session.uid => {
            inner.throttle.succeeded(attempt);
            Ok(())
        }
        SignIn::Failed(why) => {
            tracing::warn!("the accounts could not be asked: {why}");
            Err("Sign-in is not available.")
        }
        _ => {
            thread::sleep(
                inner
                    .settings
                    .failure_floor
                    .saturating_sub(started.elapsed()),
            );
            Err("That password was not accepted.")
        }
    }
}

/// `/canoa.json` for this user: their host's id, method and entries, each with the address of a channel through the portal.
fn index(inner: &Inner, request: &Request, conn: &Conn, session: &Session) -> Response {
    let lease = match host_for(inner, request, conn, session) {
        Ok(lease) => lease,
        Err(response) => return response,
    };
    let asked = inner
        .instances
        .ask(&session.user, session.uid, r#"{"op":"index"}"#);
    drop(lease);
    let Ok(mut index) = asked else {
        return Response::refusal(503);
    };
    let origin = format!(
        "{}://{}",
        if scheme_of(request, conn) == "https" {
            "wss"
        } else {
            "ws"
        },
        request.header("host").unwrap_or("localhost")
    );
    if let Some(entries) = index
        .get_mut("entries")
        .and_then(serde_json::Value::as_array_mut)
    {
        for entry in entries {
            let kind = entry
                .get("kind")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let name = entry
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let url = if kind == "control" {
                format!("{origin}/portal/ws/control")
            } else if entry.get("local").is_some() {
                continue;
            } else {
                format!("{origin}/portal/ws/entry/{name}")
            };
            entry["url"] = url.into();
        }
    }
    index["user"] = session.user.clone().into();
    Response::json(200, &index).with("Cache-Control", "no-store".to_owned())
}

/// Accepts the key the browser says is its own, once it has connected through a channel and waits in the host.
fn pair(inner: &Inner, request: &Request, conn: &Conn, session: &Session) -> Response {
    let Some(peer) = serde_json::from_slice::<serde_json::Value>(&request.body)
        .ok()
        .and_then(|body| {
            body.get("peer")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .filter(|peer| peer.len() <= 128 && !peer.chars().any(char::is_control))
    else {
        return Response::refusal(400);
    };
    let lease = match host_for(inner, request, conn, session) {
        Ok(lease) => lease,
        Err(response) => return response,
    };
    let ask = serde_json::json!({"op": "accept", "peer": peer}).to_string();
    let until = Instant::now() + inner.settings.pair;
    let result = loop {
        match inner.instances.ask(&session.user, session.uid, &ask) {
            Ok(answer) if answer.get("ok") == Some(&serde_json::Value::Bool(true)) => {
                break Response::json(200, &serde_json::json!({"ok": true}));
            }
            // A host that is not the person's, or that cannot be dialed for good, is not waited on.
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                break Response::refusal(503);
            }
            Ok(_) | Err(_) if Instant::now() < until => thread::sleep(Duration::from_millis(250)),
            _ => {
                break Response::json(
                    404,
                    &serde_json::json!({"error": "that key has not connected to this host"}),
                );
            }
        }
    };
    drop(lease);
    result
}

/// The web manifest, the favicon and the app's icons: what the page links in its head, which a browser fetches with no
/// credentials, so the sign-in cannot gate them.
fn is_public_asset(path: &str) -> bool {
    match web::segments(path).as_deref() {
        Some([name]) => name == "manifest.json" || name == "favicon.ico",
        Some([folder, name]) => folder == "icons" && !name.is_empty(),
        _ => false,
    }
}

/// A file of the site. `public` says it is given to a browser that has not signed in.
fn site(inner: &Inner, request: &Request, public: bool) -> Response {
    let Some(segments) = web::segments(request.path()) else {
        return Response::refusal(404);
    };
    site_file(inner, request, segments, public, Vec::new())
}

/// The file of the site that `segments` name, with `own` among its headers.
fn site_file(
    inner: &Inner,
    request: &Request,
    segments: Vec<String>,
    public: bool,
    own: Vec<(&'static str, String)>,
) -> Response {
    let found = match web::lookup_dir(&inner.root, &segments) {
        Lookup::Found(found) => Some((found, segments.last().cloned())),
        Lookup::Refused => None,
        Lookup::Missing if web::is_app_route(&segments) => {
            match web::lookup_dir(&inner.root, &["index.html".to_owned()]) {
                Lookup::Found(found) => Some((found, Some("index.html".to_owned()))),
                _ => None,
            }
        }
        Lookup::Missing => None,
    };
    let Some((found, name)) = found else {
        return Response::refusal(404);
    };
    let name = name.unwrap_or_else(|| "index.html".to_owned());
    // The rest of the site is behind the sign-in, so no shared cache may keep it.
    let caching = if public {
        "no-cache".to_owned()
    } else {
        web::cache_control(&segments, &name, web::Limits::default().immutable_age, true)
    };
    let mut headers = web::validators(&found, caching, &name);
    headers.extend(own);
    if web::not_modified(
        request.header("if-none-match"),
        request.header("if-modified-since"),
        &found,
    ) {
        headers.retain(|(name, _)| *name != "Content-Type");
        return Response {
            status: 304,
            headers,
            payload: Payload::Bytes(Vec::new()),
        };
    }
    let Body::File(file, length) = found.body else {
        return Response::refusal(404);
    };
    Response {
        status: 200,
        headers,
        payload: Payload::File(file, length),
    }
}

/// A channel to the user's host: only for the one signed in, only from the site's own page, only to one of that user's own
/// sockets, whose path is made from the session's account name and a name checked here, never from anything else the browser
/// says.
fn upgrade(inner: &Inner, conn: &mut Conn, request: &Request) {
    let refuse = |conn: &mut Conn, status: u16| {
        let _ = write_response(inner, conn, Response::refusal(status), false, false);
    };
    if !host_ok(inner, request) {
        return refuse(conn, 421);
    }
    let Some(session) = authed(inner, request) else {
        return refuse(conn, 401);
    };
    if request.method != "GET" || !from_own_page(request, conn) {
        return refuse(conn, 403);
    }
    let name = match request.path() {
        "/portal/ws/control" => None,
        other => match other.strip_prefix("/portal/ws/entry/") {
            Some(name)
                if !name.is_empty()
                    && name.len() <= 64
                    && name.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')
                    }) =>
            {
                Some(name.to_owned())
            }
            _ => return refuse(conn, 404),
        },
    };
    let Some(key) = request
        .header("sec-websocket-key")
        .filter(|key| key.len() <= 64)
    else {
        return refuse(conn, 400);
    };
    if request.header("sec-websocket-version") != Some("13") {
        return refuse(conn, 400);
    }
    let lease = match host_for(inner, request, conn, &session) {
        Ok(lease) => lease,
        Err(response) => return refuse(conn, response.status),
    };
    let path = match &name {
        Some(name) => inner.instances.entry_socket(&session.user, name),
        None => inner
            .instances
            .dir_of(&session.user)
            .join(crate::portal_link::CONTROL),
    };
    // The socket is dialed for this person and must have been made by this person's own process.
    let max_message = inner.settings.max_message;
    let channel =
        instance::dial(&path, session.uid).map(|socket| PipeChannel::new(socket, max_message));
    let channel = match channel {
        Ok(channel) => channel,
        Err(error) => {
            tracing::warn!("{} was not given a channel: {error}", session.user);
            return refuse(conn, 404);
        }
    };
    if conn
        .write_all(relay::upgrade_answer(key).as_bytes())
        .is_err()
    {
        return;
    }
    let id = session.id;
    // A channel outlives the request that opened it, so a session made with a code is looked at again every few seconds: a reset
    // ends its channels too.
    let looked = Mutex::new(Instant::now());
    relay::run(
        conn,
        channel,
        &|| {
            if !inner.sessions.alive(id) {
                return false;
            }
            if coded(&session) || session.passkey.is_some() {
                let mut at = looked.lock().unwrap_or_else(PoisonError::into_inner);
                if at.elapsed() >= CODE_LOOK {
                    *at = Instant::now();
                    // One read says both; one that fails keeps the channel, as a request that cannot be served is answered
                    // `503` and keeps its session.
                    if coded(&session) {
                        let (standing, made) =
                            inner.accounts.factor_record(&session.user, session.uid);
                        if matches!(
                            standing,
                            FactorStanding::NotEnrolled
                                | FactorStanding::Disabled
                                | FactorStanding::Reset
                        ) || (standing == FactorStanding::Enrolled && made != session.code_made)
                        {
                            inner.sessions.close_id(id);
                            return false;
                        }
                    }
                    if let Some(credential) = &session.passkey {
                        match inner.accounts.passkeys(&session.user, session.uid) {
                            Ok(held)
                                if held
                                    .iter()
                                    .any(|each| each.id == *credential && !each.disabled) => {}
                            Ok(_) | Err(PasskeyError::Unsupported) => {
                                inner.sessions.close_id(id);
                                return false;
                            }
                            Err(_) => {}
                        }
                    }
                }
            }
            true
        },
        &inner.stopped,
        max_message,
        inner.settings.ping,
    );
    drop(lease);
}

#[cfg(test)]
mod tests;
