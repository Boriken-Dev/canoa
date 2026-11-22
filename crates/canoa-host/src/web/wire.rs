//! HTTP/1.1 connection framing, timeouts, TLS wrapping, and response writing for the web app server.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "sockets, IO and clocks are std's"
)]

use alloc::borrow::ToOwned;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::net::IpAddr;
use core::sync::atomic::Ordering;
use core::time::Duration;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::time::{Instant, SystemTime};

use super::assets::{Body, http_date};
use super::handle::{Answer, answer, reason};
use super::limits::Limits;
use super::{Shared, local};

/// Headers one request may carry; past it the request is refused with 431.
pub(super) const MAX_HEADERS: usize = 64;
/// What one read takes off the connection.
const CHUNK: usize = 4096;

/// `time` from now, or a year from now when that is past what this clock can count.
fn after(time: Duration) -> Instant {
    let now = Instant::now();
    now.checked_add(time)
        .or_else(|| now.checked_add(Duration::from_secs(365 * 24 * 60 * 60)))
        .unwrap_or(now)
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

pub(super) fn connection(shared: &Shared, tcp: TcpStream) {
    let _ = tcp.set_nodelay(true);
    let from = tcp.peer_addr().ok().map(|address| address.ip());
    let timed = Timed {
        tcp,
        until: after(shared.limits.request_time),
        send: shared.limits.send_time,
        end: after(shared.limits.connection_time),
    };
    match &shared.tls {
        None => exchange(shared, timed, from),
        Some(config) => match rustls::ServerConnection::new(Arc::clone(config)) {
            Ok(session) => exchange(shared, rustls::StreamOwned::new(session, timed), from),
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
fn exchange<W: Wire>(shared: &Shared, mut wire: W, from: Option<IpAddr>) {
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
        // Only the pane's tools take a body, and only up to what a tool provider's message may be.
        let mut body_length = 0;
        if let Some(wanted) = body_wanted(shared, &buffer[..length]) {
            if wanted > local::MAX_MESSAGE {
                let _ = send(shared, &mut wire, Answer::refusal(413), false, false);
                break;
            }
            body_length = wanted;
            if read_body(&mut wire, &mut buffer, length + wanted, &shared.limits).is_err() {
                break;
            }
        }
        let body = buffer[length..length + body_length].to_vec();
        let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
        let mut request = httparse::Request::new(&mut headers);
        if !matches!(
            request.parse(&buffer[..length]),
            Ok(httparse::Status::Complete(_))
        ) {
            break;
        }
        let (mut reply, head) = answer(shared, &request, from, &body);
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
        buffer.drain(..length + body_length);
    }
    wire.finish();
}

/// How long the body of the request whose head is `head` is, when it is one that takes a body: `POST` of a tools message, on a
/// server that carries the pane's tools. `Some(usize::MAX)` for a length that cannot be read, which is too long to take.
fn body_wanted(shared: &Shared, head: &[u8]) -> Option<usize> {
    shared.tools.as_ref()?;
    let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
    let mut request = httparse::Request::new(&mut headers);
    request.parse(head).ok()?;
    if !tools_send(&request) {
        return None;
    }
    Some(
        header(&request, "content-length")
            .and_then(|length| length.trim().parse::<usize>().ok())
            .unwrap_or(usize::MAX),
    )
}

/// Whether the request is `POST` of a message from the pane's tools.
pub(super) fn tools_send(request: &httparse::Request<'_, '_>) -> bool {
    request.method == Some("POST")
        && request
            .path
            .and_then(|target| target.split(['?', '#']).next())
            .and_then(|path| path.strip_prefix(local::TOOLS_PATH))
            == Some("/send")
        && count_headers(request, "transfer-encoding") == 0
        && count_headers(request, "content-length") == 1
}

/// Reads until `buffer` holds `until` bytes, within the time a request has to arrive.
fn read_body<W: Wire>(
    wire: &mut W,
    buffer: &mut Vec<u8>,
    until: usize,
    limits: &Limits,
) -> Result<(), ()> {
    wire.until(after(limits.request_time));
    let mut chunk = [0_u8; CHUNK];
    while buffer.len() < until {
        match wire.read(&mut chunk) {
            Ok(0) | Err(_) => return Err(()),
            Ok(read) => buffer.extend_from_slice(&chunk[..read]),
        }
    }
    Ok(())
}

/// The value of the one header called `name`, or `None` when there is none.
pub(super) fn header<'a>(request: &httparse::Request<'_, 'a>, name: &str) -> Option<&'a str> {
    request
        .headers
        .iter()
        .find(|header| header.name.eq_ignore_ascii_case(name))
        .and_then(|header| core::str::from_utf8(header.value).ok())
}

/// How many headers are called `name`.
pub(super) fn count_headers(request: &httparse::Request<'_, '_>, name: &str) -> usize {
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
    // A route may replace a header every answer carries, or drop it with an empty value.
    let replaced = |name: &str| {
        answer
            .headers
            .iter()
            .any(|(each, _)| each.eq_ignore_ascii_case(name))
    };
    for (name, value) in common
        .iter()
        .copied()
        .filter(|(name, _)| !replaced(name))
        .chain(
            answer
                .headers
                .iter()
                .map(|(name, value)| (*name, value.as_str())),
        )
        .filter(|(_, value)| !value.is_empty())
    {
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
