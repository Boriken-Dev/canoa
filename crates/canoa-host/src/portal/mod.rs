//! `canoa portal`: the web app for people who sign in with their account on this machine, each getting a host of their own that
//! runs as them, in their home.
//!
//! The portal runs as a service user with no rights over anyone's files. It checks a password through the operating system's
//! accounts, keeps the signed-in browser in an `HttpOnly`, `Secure`, `SameSite=Strict` cookie, starts that user's
//! `canoa@USER.service`, introduces the browser's own key to it and relays the browser's channel to its sockets. The browser and
//! the host still prove their keys to each other through the relay.

mod instance;
mod relay;
mod session;
mod settings;
mod signin;

use alloc::sync::Arc;
use core::net::{IpAddr, SocketAddr};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use core::time::Duration;
use std::fs::File;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::thread;
use std::time::{Instant, SystemTime};

pub use instance::{Controller, Instances, Lease, Systemd};
pub use session::{Session, Sessions, Throttle};
pub use settings::Settings;
pub use signin::{Accounts, MAX_PASSWORD, SignIn, System, valid_user};

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
    /// The address could not be bound.
    Listen(io::Error),
}

impl core::fmt::Display for PortalError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Settings(what) => write!(f, "portal setting {what}"),
            Self::Site(what) => write!(f, "portal site: {what}"),
            Self::Listen(error) => write!(f, "the portal's address could not be bound: {error}"),
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
    listener: TcpListener,
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
    /// Ends the accept loop and every relay; each ends within a second.
    pub fn stop(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

impl Portal {
    /// Binds the portal on `settings.listen`, signing people in through `accounts` and running their hosts through
    /// `controller`.
    ///
    /// # Errors
    ///
    /// For settings that do not work, a site that cannot be served, or an address that cannot be bound.
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
        let listener = TcpListener::bind(settings.listen).map_err(PortalError::Listen)?;
        listener
            .set_nonblocking(true)
            .map_err(PortalError::Listen)?;
        let instances = Instances::new(
            controller,
            settings.socket_dir.clone(),
            settings.idle,
            settings.start,
            settings.max_relays_per_user,
        );
        instances.adopt();
        let inner = Inner {
            sessions: Sessions::new(settings.session, settings.max_sessions),
            throttle: Throttle::new(
                settings.failure_window,
                settings.max_failures_per_address,
                settings.max_failures_per_user,
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
            listener,
        })
    }

    /// The address taken.
    ///
    /// # Errors
    ///
    /// When the system cannot say.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
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
        while !inner.stopped.load(Ordering::SeqCst) {
            match self.listener.accept() {
                Ok((stream, peer)) => {
                    if inner.connections.fetch_add(1, Ordering::SeqCst)
                        >= inner.settings.max_connections
                    {
                        inner.connections.fetch_sub(1, Ordering::SeqCst);
                        continue;
                    }
                    let inner = Arc::clone(&inner);
                    let spawned = thread::Builder::new()
                        .name("canoa-portal".to_owned())
                        .spawn({
                            let inner = Arc::clone(&inner);
                            move || {
                                let _ = stream.set_nonblocking(false);
                                let _ = stream.set_nodelay(true);
                                serve_connection(&inner, stream, peer.ip());
                                inner.connections.fetch_sub(1, Ordering::SeqCst);
                            }
                        });
                    if spawned.is_err() {
                        inner.connections.fetch_sub(1, Ordering::SeqCst);
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => thread::sleep(REST),
                Err(error) => {
                    tracing::warn!("a connection failed: {error}");
                    thread::sleep(REST);
                }
            }
        }
        if let Ok(sweeper) = sweeper {
            let _ = sweeper.join();
        }
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
    stream: &mut TcpStream,
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
        let _ = stream.set_read_timeout(Some(wait));
        match stream.read(&mut chunk) {
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
        let _ = stream.set_read_timeout(Some(
            inner
                .settings
                .request
                .saturating_sub(started.elapsed())
                .max(REST),
        ));
        match stream.read(&mut chunk) {
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

fn http_date() -> String {
    web::http_date(SystemTime::now())
}

fn write_response(
    inner: &Inner,
    stream: &mut TcpStream,
    response: Response,
    head: bool,
    keep: bool,
) -> io::Result<()> {
    let length = match &response.payload {
        Payload::Bytes(bytes) => bytes.len() as u64,
        Payload::File(_, length) => *length,
    };
    let policy = if response
        .headers
        .iter()
        .any(|(name, _)| *name == "Content-Security-Policy")
    {
        None
    } else {
        Some(web::policy(&[]))
    };
    let mut text = format!(
        "HTTP/1.1 {} {}\r\nDate: {}\r\nContent-Length: {length}\r\n",
        response.status,
        reason(response.status),
        http_date()
    );
    let common = [
        ("X-Content-Type-Options", "nosniff"),
        ("Referrer-Policy", "no-referrer"),
        ("X-Frame-Options", "DENY"),
        ("Cross-Origin-Opener-Policy", "same-origin"),
        ("Cross-Origin-Resource-Policy", "same-origin"),
        ("Connection", if keep { "keep-alive" } else { "close" }),
    ];
    if let Some(policy) = &policy {
        text.push_str(&format!("Content-Security-Policy: {policy}\r\n"));
    }
    for (name, value) in common
        .iter()
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
    let _ = stream.set_write_timeout(Some(inner.settings.request.max(Duration::from_secs(30))));
    stream.write_all(text.as_bytes())?;
    if head {
        return Ok(());
    }
    match response.payload {
        Payload::Bytes(bytes) => stream.write_all(&bytes),
        Payload::File(file, length) => io::copy(&mut file.take(length), stream).map(drop),
    }
}

/// The browser's address: the connection's, or what a trusted proxy says it is.
fn client_of(inner: &Inner, request: &Request, peer: IpAddr) -> IpAddr {
    if !inner.settings.trusted_proxies.contains(&peer) {
        return peer;
    }
    request
        .header(&inner.settings.client_address_header)
        .and_then(|list| {
            list.rsplit(',')
                .filter_map(|part| part.trim().parse::<IpAddr>().ok())
                .find(|address| !inner.settings.trusted_proxies.contains(address))
        })
        .unwrap_or(peer)
}

fn host_ok(inner: &Inner, request: &Request) -> bool {
    request.count("host") == 1
        && request
            .header("host")
            .is_some_and(|host| web::host_allowed(host, &inner.settings.hosts))
}

/// Whether a request that changes something comes from this site's own page: `Origin` names this host, and a browser that says
/// the request crossed sites is believed.
fn from_own_page(request: &Request) -> bool {
    if request
        .header("sec-fetch-site")
        .is_some_and(|site| site != "same-origin")
    {
        return false;
    }
    let (Some(origin), Some(host)) = (request.header("origin"), request.header("host")) else {
        return false;
    };
    origin.split_once("://").is_some_and(|(scheme, authority)| {
        matches!(scheme, "http" | "https") && authority.eq_ignore_ascii_case(host)
    })
}

/// `scheme://host`: what the page uses for the addresses of the channels.
fn public_origin(inner: &Inner, request: &Request, peer: IpAddr) -> String {
    let host = request.header("host").unwrap_or("localhost");
    let name = host.rsplit_once(':').map_or(host, |(name, port)| {
        if port.bytes().all(|byte| byte.is_ascii_digit()) {
            name
        } else {
            host
        }
    });
    let local = matches!(name, "localhost" | "127.0.0.1" | "[::1]");
    let proxied = inner.settings.trusted_proxies.contains(&peer)
        && request
            .header("x-forwarded-proto")
            .is_some_and(|proto| proto.eq_ignore_ascii_case("https"));
    format!("{}://{host}", if proxied || !local { "wss" } else { "ws" })
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

fn login_response(status: u16, message: &str) -> Response {
    Response::text(status, "text/html; charset=utf-8", login_page(message))
        .with("Content-Security-Policy", LOGIN_POLICY.to_owned())
}

fn serve_connection(inner: &Inner, mut stream: TcpStream, peer: IpAddr) {
    let mut buffer = Vec::new();
    let mut served = 0_usize;
    while !inner.stopped.load(Ordering::SeqCst) {
        let request = match read_request(&mut stream, &mut buffer, inner, served == 0) {
            Ok(request) => request,
            Err(Unread::Gone) => return,
            Err(Unread::Refuse(status)) => {
                let _ = write_response(inner, &mut stream, Response::refusal(status), false, false);
                return;
            }
        };
        served += 1;
        let head = request.method == "HEAD";
        if request
            .header("upgrade")
            .is_some_and(|value| value.eq_ignore_ascii_case("websocket"))
        {
            upgrade(inner, &mut stream, &request, peer);
            return;
        }
        let response = answer(inner, &request, peer);
        let keep = request.version == 1
            && served < inner.settings.max_requests
            && !request
                .header("connection")
                .is_some_and(|value| value.to_ascii_lowercase().contains("close"))
            && response.status != 408;
        if write_response(inner, &mut stream, response, head, keep).is_err() || !keep {
            return;
        }
    }
}

fn authed(inner: &Inner, request: &Request) -> Option<Session> {
    inner.sessions.get(&request.token()?)
}

fn answer(inner: &Inner, request: &Request, peer: IpAddr) -> Response {
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
        ("POST", LOGIN) => sign_in(inner, request, peer),
        ("POST", "/portal/logout") => {
            if !from_own_page(request) {
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
            Some(session) => index(inner, request, peer, &session),
            None => Response::refusal(401),
        },
        ("POST", "/portal/pair") => match authed(inner, request) {
            Some(session) if from_own_page(request) => pair(inner, request, &session),
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

fn sign_in(inner: &Inner, request: &Request, peer: IpAddr) -> Response {
    if !from_own_page(request) {
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
    let address = client_of(inner, request, peer);
    let key: String = user.trim().to_ascii_lowercase().chars().take(64).collect();
    // Asked before the accounts are, so that a refused try never reaches the directory, where it could lock an account.
    if !inner.throttle.allows(address, &key) {
        tracing::info!("a sign-in from {address} was refused for its failures");
        return login_response(429, "Too many failed attempts. Try again later.").with(
            "Retry-After",
            inner.settings.failure_window.as_secs().to_string(),
        );
    }
    if inner.signing_in.fetch_add(1, Ordering::SeqCst) >= inner.settings.max_sign_ins {
        inner.signing_in.fetch_sub(1, Ordering::SeqCst);
        return login_response(503, "Busy. Try again in a moment.");
    }
    let result = inner.accounts.sign_in(user.trim(), &password);
    inner.signing_in.fetch_sub(1, Ordering::SeqCst);
    match result {
        SignIn::Admitted { user, uid } => {
            let Some((token, _)) = inner.sessions.open(&user, uid) else {
                return login_response(503, "Busy. Try again in a moment.");
            };
            tracing::info!("{user} signed in from {address}");
            Response::redirect("/").with(
                "Set-Cookie",
                format!(
                    "{COOKIE}={token}; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age={}",
                    inner.settings.session.as_secs()
                ),
            )
        }
        SignIn::Refused => {
            inner.throttle.failed(address, &key);
            tracing::info!("a sign-in from {address} failed");
            login_response(401, "Sign-in failed.")
        }
        SignIn::Failed(why) => {
            tracing::warn!("the accounts could not be asked: {why}");
            login_response(503, "Sign-in is not available.")
        }
    }
}

/// `/canoa.json` for this user: their host's id, method and entries, each with the address of a channel through the portal.
fn index(inner: &Inner, request: &Request, peer: IpAddr, session: &Session) -> Response {
    let lease = match inner.instances.acquire(&session.user, session.uid) {
        Ok(lease) => lease,
        Err(error) => {
            tracing::warn!("{}'s host did not start: {error}", session.user);
            return Response::refusal(503);
        }
    };
    let asked = inner
        .instances
        .ask(&session.user, session.uid, r#"{"op":"index"}"#);
    drop(lease);
    let Ok(mut index) = asked else {
        return Response::refusal(503);
    };
    let origin = public_origin(inner, request, peer);
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
    let Ok(lease) = inner.instances.acquire(&session.user, session.uid) else {
        return Response::refusal(503);
    };
    let ask = serde_json::json!({"op": "accept", "peer": peer}).to_string();
    let until = Instant::now() + inner.settings.pair;
    let result = loop {
        match inner.instances.ask(&session.user, session.uid, &ask) {
            Ok(answer) if answer.get("ok") == Some(&serde_json::Value::Bool(true)) => {
                break Response::json(200, &serde_json::json!({"ok": true}));
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
/// sockets, whose path is made from the session's name and a name checked here, never from anything else the browser says.
fn upgrade(inner: &Inner, stream: &mut TcpStream, request: &Request, peer: IpAddr) {
    let refuse = |stream: &mut TcpStream, status: u16| {
        let _ = write_response(inner, stream, Response::refusal(status), false, false);
    };
    let _ = peer;
    if !host_ok(inner, request) {
        return refuse(stream, 421);
    }
    let Some(session) = authed(inner, request) else {
        return refuse(stream, 401);
    };
    if request.method != "GET" || !from_own_page(request) {
        return refuse(stream, 403);
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
            _ => return refuse(stream, 404),
        },
    };
    let Some(key) = request
        .header("sec-websocket-key")
        .filter(|key| key.len() <= 64)
    else {
        return refuse(stream, 400);
    };
    if request.header("sec-websocket-version") != Some("13") {
        return refuse(stream, 400);
    }
    let lease = match inner.instances.acquire(&session.user, session.uid) {
        Ok(lease) => lease,
        Err(error) => {
            tracing::warn!("{}'s host is not there: {error}", session.user);
            return refuse(stream, 503);
        }
    };
    let path = match &name {
        Some(name) => inner.instances.entry_socket(&session.user, name),
        None => inner
            .instances
            .dir_of(&session.user)
            .join(crate::portal_link::CONTROL),
    };
    // The socket is dialed for this person and must have been made by this person's own process.
    let channel = instance::dial(&path, session.uid)
        .map(|socket| PipeChannel::new(socket, relay::MAX_MESSAGE));
    let channel = match channel {
        Ok(channel) => channel,
        Err(error) => {
            tracing::warn!("{} was not given a channel: {error}", session.user);
            return refuse(stream, 404);
        }
    };
    if stream
        .write_all(relay::upgrade_answer(key).as_bytes())
        .is_err()
    {
        return;
    }
    let id = session.id;
    relay::run(
        stream,
        channel,
        &|| inner.sessions.alive(id),
        &inner.stopped,
    );
    drop(lease);
}

#[cfg(test)]
mod tests;
