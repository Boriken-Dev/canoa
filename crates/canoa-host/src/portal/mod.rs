//! `canoa portal`: the web app for people who sign in with their account on this machine, each getting a host of their own that
//! runs as them, in their home.
//!
//! The portal runs as a service user with no rights over anyone's files. It checks a password through the operating system's
//! accounts, keeps the signed-in browser in an `HttpOnly`, `Secure`, `SameSite=Strict` cookie, starts that user's
//! `canoa@USER.service`, introduces the browser's own key to it and relays the browser's channel to its sockets. The browser and
//! the host still prove their keys to each other through the relay.

mod conn;
mod instance;
mod relay;
mod session;
mod settings;
mod signin;

use alloc::sync::Arc;
use core::net::{IpAddr, Ipv4Addr, SocketAddr};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use core::time::Duration;
use std::fs::File;
use std::io::{self, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::thread;
use std::time::{Instant, SystemTime};

use coqui_transport_pipe::{PipeName, SocketGroup};

pub use conn::Conn;
pub use instance::{Controller, Instances, Lease, Systemd};
pub use session::{Attempt, Session, Sessions, Throttle};
pub use settings::Settings;
pub use signin::{Accounts, MAX_PASSWORD, NAME_PATTERN, SignIn, Standing, System, valid_user};

use crate::pipe::PipeChannel;
use crate::web::{self, Body, Lookup};

/// The session's cookie. The `__Host-` prefix makes a browser refuse it unless it is `Secure`, for the whole site and with no
/// `Domain`, so no sibling site can set it.
pub const COOKIE: &str = "__Host-canoa";
/// Where a browser signs in.
pub const LOGIN: &str = "/portal/login";
const MAX_HEAD: usize = 16 * 1024;
const MAX_HEADERS: usize = 64;
const MAX_FORM: usize = 4096;
const REST: Duration = Duration::from_millis(50);

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
    throttle: Throttle,
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
        let site = settings
            .site
            .as_deref()
            .ok_or_else(|| PortalError::Site("`site` names no directory".to_owned()))?;
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
        let inner = Inner {
            sessions: Sessions::new(
                settings.session,
                settings.max_sessions,
                settings.max_sessions_per_user,
            ),
            throttle: Throttle::new(
                settings.failure_window,
                settings.max_failures_per_pair,
                settings.max_failures_per_address,
                settings.slow_after_user_failures,
                settings.slow,
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

    fn token(&self) -> Option<String> {
        let mut found = None;
        for (name, value) in &self.headers {
            if name != "cookie" {
                continue;
            }
            for part in value.split(';') {
                if let Some(token) = part.trim().strip_prefix(&format!("{COOKIE}=")) {
                    // Two cookies of the name are two sessions the browser cannot be told apart by: none.
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
        "HTTP/1.1 {} {}\r\nDate: {}\r\nContent-Length: {length}\r\n",
        response.status,
        reason(response.status),
        web::http_date(SystemTime::now())
    );
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
        // A value written from a request is never allowed a line break.
        if value.contains(['\r', '\n']) {
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
/// socket, the last address in the header the proxy sets that is not one of ours. A proxy that says nothing usable counts all
/// such browsers as one address.
fn client_of(inner: &Inner, request: &Request, conn: &Conn) -> IpAddr {
    if !conn.from_proxy() {
        return conn.peer_ip().unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
    }
    request
        .header(&inner.settings.client_address_header)
        .and_then(|list| {
            list.rsplit(',')
                .filter_map(|part| part.trim().parse::<IpAddr>().ok())
                .find(|address| !inner.settings.trusted_proxies.contains(address))
        })
        .unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED))
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

fn login_page(message: &str) -> Vec<u8> {
    let note = if message.is_empty() {
        String::new()
    } else {
        format!("<p role=\"alert\">{message}</p>")
    };
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Sign in to Canoa</title><style>body{{font:16px system-ui,sans-serif;max-width:22rem;margin:12vh auto;padding:0 16px}}label,input,button{{display:block;width:100%;box-sizing:border-box}}input,button{{margin:.25rem 0 1rem;padding:.6rem;font:inherit}}p{{color:#b00020}}</style></head><body><h1>Canoa</h1>{note}<form method=\"post\" action=\"{LOGIN}\"><label for=\"user\">Account</label><input id=\"user\" name=\"user\" autocomplete=\"username\" autocapitalize=\"none\" required><label for=\"password\">Password</label><input id=\"password\" name=\"password\" type=\"password\" autocomplete=\"current-password\" required><button type=\"submit\">Sign in</button></form></body></html>"
    )
    .into_bytes()
}

const LOGIN_POLICY: &str = "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'";

/// The sign-in page.
fn login_response(status: u16, message: &str) -> Response {
    Response::text(status, "text/html; charset=utf-8", login_page(message))
        .with("Content-Security-Policy", LOGIN_POLICY.to_owned())
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
fn host_for(inner: &Inner, session: &Session) -> Result<Lease, Response> {
    if inner.accounts.still(&session.user, session.uid) == Standing::Gone {
        inner.sessions.close_user(&session.user);
        tracing::info!(
            "{} is no longer an account: its sessions ended",
            session.user
        );
        return Err(Response::refusal(401));
    }
    inner
        .instances
        .acquire(&session.user, session.uid)
        .map_err(|error| {
            tracing::warn!("{}'s host is not there: {error}", session.user);
            Response::refusal(503)
        })
}

fn answer(inner: &Inner, request: &Request, conn: &Conn) -> Response {
    let started = Instant::now();
    if !host_ok(inner, request) || !request.target.starts_with('/') {
        return Response::refusal(421);
    }
    let path = request.path();
    match (request.method.as_str(), path) {
        ("GET" | "HEAD", LOGIN) => {
            let message = match request.query() {
                "e=1" => "Sign-in failed.",
                "e=2" => "Too many failed attempts. Try again later.",
                _ => "",
            };
            login_response(200, message)
        }
        ("POST", LOGIN) => sign_in(inner, request, conn, started),
        ("POST", "/portal/logout") => {
            if !from_own_page(request, conn) {
                return Response::refusal(403);
            }
            if let Some(token) = request.token() {
                inner.sessions.close(&token);
            }
            Response::redirect(LOGIN).with(
                "Set-Cookie",
                format!("{COOKIE}=; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=0"),
            )
        }
        ("GET", "/portal/session") => match authed(inner, request) {
            Some(session) => Response::json(200, &serde_json::json!({"user": session.user})),
            None => Response::refusal(401),
        },
        ("GET" | "HEAD", web::HOST_INDEX) => match authed(inner, request) {
            // It starts the person's host, so only the site's own page may ask for it.
            Some(_) if !same_site(request) => Response::refusal(403),
            Some(session) => index(inner, request, conn, &session),
            None => Response::refusal(401),
        },
        ("POST", "/portal/pair") => match authed(inner, request) {
            Some(session) if from_own_page(request, conn) => pair(inner, request, &session),
            Some(_) => Response::refusal(403),
            None => Response::refusal(401),
        },
        ("GET" | "HEAD", _) if path.starts_with("/portal/") => Response::refusal(404),
        ("GET" | "HEAD", _) => match authed(inner, request) {
            Some(_) => site(inner, request),
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

fn sign_in(inner: &Inner, request: &Request, conn: &Conn, started: Instant) -> Response {
    if !from_own_page(request, conn) {
        return Response::refusal(403);
    }
    if !request
        .header("content-type")
        .is_some_and(|kind| kind.starts_with("application/x-www-form-urlencoded"))
    {
        return Response::refusal(400);
    }
    let (Some(user), Some(password)) = (
        form_field(&request.body, "user"),
        form_field(&request.body, "password"),
    ) else {
        return Response::refusal(400);
    };
    let address = client_of(inner, request, conn);
    let key: String = user.trim().to_ascii_lowercase().chars().take(64).collect();
    // Counted as a failure as it is let through, before the accounts are asked, so that tries made together cannot all pass the
    // limit and a refused try never reaches the directory, where it could lock an account.
    let Some(attempt) = inner.throttle.begin(address, &key) else {
        tracing::info!("a sign-in from {address} was refused for its failures");
        return after_floor(
            inner,
            started,
            login_response(429, "Too many failed attempts. Try again later.").with(
                "Retry-After",
                inner.settings.failure_window.as_secs().to_string(),
            ),
        );
    };
    if !attempt.wait.is_zero() {
        thread::sleep(attempt.wait);
    }
    if inner.signing_in.fetch_add(1, Ordering::SeqCst) >= inner.settings.max_sign_ins {
        inner.signing_in.fetch_sub(1, Ordering::SeqCst);
        // Nothing was asked of the accounts, so nothing counts against the address.
        inner.throttle.succeeded(attempt);
        return login_response(503, "Busy. Try again in a moment.");
    }
    let result = inner.accounts.sign_in(user.trim(), &password);
    inner.signing_in.fetch_sub(1, Ordering::SeqCst);
    match result {
        SignIn::Admitted { user, uid }
            if !crate::portal_link::system_uid(uid) && uid >= inner.settings.min_uid =>
        {
            let Some((token, _)) = inner.sessions.open(&user, uid) else {
                inner.throttle.succeeded(attempt);
                return login_response(503, "Busy. Try again in a moment.");
            };
            inner.throttle.succeeded(attempt);
            tracing::info!("{user} signed in from {address}");
            Response::redirect("/").with(
                "Set-Cookie",
                format!(
                    "{COOKIE}={token}; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age={}",
                    inner.settings.session.as_secs()
                ),
            )
        }
        // An account the accounts admit that the portal does not (root, or below the lowest user id) is refused as any other.
        SignIn::Admitted { .. } | SignIn::Refused => {
            tracing::info!("a sign-in from {address} failed");
            after_floor(inner, started, login_response(401, "Sign-in failed."))
        }
        SignIn::Failed(why) => {
            tracing::warn!("the accounts could not be asked: {why}");
            after_floor(
                inner,
                started,
                login_response(503, "Sign-in is not available."),
            )
        }
    }
}

/// `/canoa.json` for this user: their host's id, method and entries, each with the address of a channel through the portal.
fn index(inner: &Inner, request: &Request, conn: &Conn, session: &Session) -> Response {
    let lease = match host_for(inner, session) {
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
fn pair(inner: &Inner, request: &Request, session: &Session) -> Response {
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
    let lease = match host_for(inner, session) {
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

fn site(inner: &Inner, request: &Request) -> Response {
    let Some(segments) = web::segments(request.path()) else {
        return Response::refusal(404);
    };
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
    let Body::File(file, length) = found.body else {
        return Response::refusal(404);
    };
    // The site is behind the sign-in, so no shared cache may keep it.
    Response {
        status: 200,
        headers: vec![
            ("Content-Type", web::content_type(&name).to_owned()),
            ("Cache-Control", "private, no-cache".to_owned()),
            ("ETag", found.tag),
        ],
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
    let lease = match host_for(inner, &session) {
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
    relay::run(
        conn,
        channel,
        &|| inner.sessions.alive(id),
        &inner.stopped,
        max_message,
    );
    drop(lease);
}

#[cfg(test)]
mod tests;
