//! `ureq` that sends through the system's proxy, logs in to it, and verifies and authenticates
//! with the system's TLS.
//!
//! Every request asks the [`Resolver`] for its routes, connects through the first that works with
//! a [`ProxyConnector`] and, for `https:`, speaks TLS with a [`Tls`]. A plain `http:` URL through
//! a proxy is tunnelled with `CONNECT`, as `ureq` does for every proxy.

use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::net::{IpAddr, Ipv4Addr, SocketAddr};
use core::time::Duration;
use std::io::{self, Read, Write};
use std::net::TcpStream;

use rustls::pki_types::ServerName;
use rustls::{ClientConnection, StreamOwned};
use ureq::config::Config;
use ureq::unversioned::resolver::{ResolvedSocketAddrs, Resolver as UreqResolver};
use ureq::unversioned::transport::{
    Buffers, ConnectionDetails, Connector, LazyBuffers, NextTimeout, Transport,
};

use coqui_proxy::auth::{Answer, OriginAuth};
use coqui_proxy::{Connection, Error, ProxyConnector, Resolver};
use coqui_sslcontext::Tls;

/// Builds a `ureq` agent that follows the system.
#[derive(Debug)]
#[must_use]
pub struct AgentBuilder {
    resolver: Option<Resolver>,
    tls: Option<Tls>,
    connector: Option<ProxyConnector>,
    config: Option<Config>,
}

impl Default for AgentBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentBuilder {
    /// Starts an agent that follows the system: [`Resolver::system`], a browser's TLS, a plain
    /// connector.
    pub fn new() -> Self {
        Self {
            resolver: None,
            tls: None,
            connector: None,
            config: None,
        }
    }

    /// Who decides the proxy (default: [`Resolver::system`]).
    pub fn resolver(mut self, resolver: Resolver) -> Self {
        self.resolver = Some(resolver);
        self
    }

    /// How servers are verified and client certificates chosen (default: the system's).
    pub fn tls(mut self, tls: Tls) -> Self {
        self.tls = Some(tls);
        self
    }

    /// How connections and proxy logins are made.
    pub fn connector(mut self, connector: ProxyConnector) -> Self {
        self.connector = Some(connector);
        self
    }

    /// The `ureq` settings: timeouts, redirects, the user agent. Its own proxy and TLS settings are
    /// not used.
    pub fn config(mut self, config: Config) -> Self {
        self.config = Some(config);
        self
    }

    /// The agent.
    ///
    /// # Errors
    ///
    /// [`crate::Error::Tls`] when the operating system's verifier cannot start.
    pub fn build(self) -> Result<ureq::Agent, crate::Error> {
        let tls = match self.tls {
            Some(tls) => tls,
            None => Tls::builder().build()?,
        };
        let connector = SystemConnector {
            resolver: self.resolver.unwrap_or_else(Resolver::system),
            connector: self.connector.unwrap_or_default(),
            tls: Arc::new(tls),
        };
        Ok(ureq::Agent::with_parts(
            self.config.unwrap_or_default(),
            connector,
            NoLookup,
        ))
    }
}

/// A name resolver that resolves nothing: the connector reaches the host itself, directly or
/// through a proxy, and a host behind a proxy may have no address where this client runs.
#[derive(Debug)]
struct NoLookup;

impl UreqResolver for NoLookup {
    fn resolve(
        &self,
        uri: &ureq::http::Uri,
        _config: &Config,
        _timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let port = uri
            .port_u16()
            .unwrap_or(if uri.scheme_str() == Some("https") {
                443
            } else {
                80
            });
        let mut addresses = self.empty();
        addresses.push(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port));
        Ok(addresses)
    }
}

#[derive(Debug)]
struct SystemConnector {
    resolver: Resolver,
    connector: ProxyConnector,
    tls: Arc<Tls>,
}

fn to_ureq(error: Error) -> ureq::Error {
    match error {
        Error::Io(error) => ureq::Error::Io(error),
        Error::Login(reason) => ureq::Error::ConnectProxyFailed(alloc::format!("login: {reason}")),
        Error::Refused(reason) => ureq::Error::ConnectProxyFailed(reason.to_string()),
        other => ureq::Error::Io(io::Error::other(other.to_string())),
    }
}

impl Connector<()> for SystemConnector {
    type Out = StreamTransport;

    fn connect(
        &self,
        details: &ConnectionDetails,
        _chained: Option<()>,
    ) -> Result<Option<Self::Out>, ureq::Error> {
        let uri = details.uri;
        let host = uri
            .host()
            .ok_or(ureq::Error::HostNotFound)?
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_string();
        let mut connector = self.connector.clone();
        if let Some(limit) = limit(details.timeout) {
            connector = connector.with_timeout(limit);
        }
        let connection = connector
            .connect_url(&self.resolver, &uri.to_string())
            .map_err(to_ureq)?;
        let buffers = LazyBuffers::new(
            details.config.input_buffer_size(),
            details.config.output_buffer_size(),
        );
        if !details.needs_tls() {
            return Ok(Some(StreamTransport {
                io: Io::Plain(connection),
                buffers,
            }));
        }
        let config = self
            .tls
            .client_config(&host)
            .map_err(|error| ureq::Error::Io(io::Error::other(error.to_string())))?;
        let name = ServerName::try_from(host)
            .map_err(|_| ureq::Error::Tls("the host is not a valid server name"))?;
        let session = ClientConnection::new(config, name)
            .map_err(|error| ureq::Error::Io(io::Error::other(error)))?;
        let mut stream = StreamOwned::new(session, connection);
        // Finish the handshake here, so a verification failure is reported as one.
        while stream.conn.is_handshaking() {
            stream
                .conn
                .complete_io(&mut stream.sock)
                .map_err(ureq::Error::Io)?;
        }
        Ok(Some(StreamTransport {
            io: Io::Tls(Box::new(stream)),
            buffers,
        }))
    }
}

enum Io {
    Plain(Connection),
    Tls(Box<StreamOwned<ClientConnection, Connection>>),
}

impl Io {
    fn socket(&self) -> &TcpStream {
        match self {
            Self::Plain(connection) => connection.socket(),
            Self::Tls(stream) => stream.sock.socket(),
        }
    }
}

impl Read for Io {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Plain(connection) => connection.read(buffer),
            Self::Tls(stream) => stream.read(buffer),
        }
    }
}

impl Write for Io {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        match self {
            Self::Plain(connection) => connection.write(buffer),
            Self::Tls(stream) => stream.write(buffer),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Plain(connection) => connection.flush(),
            Self::Tls(stream) => stream.flush(),
        }
    }
}

/// A connection, however it was made, as `ureq` reads and writes it.
struct StreamTransport {
    io: Io,
    buffers: LazyBuffers,
}

impl core::fmt::Debug for StreamTransport {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("StreamTransport")
            .field("tls", &matches!(self.io, Io::Tls(_)))
            .finish_non_exhaustive()
    }
}

fn limit(timeout: NextTimeout) -> Option<Duration> {
    timeout.not_zero().map(|duration| *duration)
}

/// The time left, or the timeout error when none is.
fn left(timeout: NextTimeout) -> Result<NextTimeout, ureq::Error> {
    if timeout.after.is_zero() {
        Err(ureq::Error::Timeout(timeout.reason))
    } else {
        Ok(timeout)
    }
}

impl Transport for StreamTransport {
    fn buffers(&mut self) -> &mut dyn Buffers {
        &mut self.buffers
    }

    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), ureq::Error> {
        let timeout = left(timeout.for_write())?;
        self.io.socket().set_write_timeout(limit(timeout))?;
        let output = &self.buffers.output()[..amount];
        match self.io.write_all(output) {
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) =>
            {
                Err(ureq::Error::Timeout(timeout.reason))
            }
            other => other.map_err(ureq::Error::Io),
        }
    }

    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, ureq::Error> {
        let timeout = left(timeout.for_read())?;
        self.io.socket().set_read_timeout(limit(timeout))?;
        let input = self.buffers.input_append_buf();
        let amount = match self.io.read(input) {
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) =>
            {
                return Err(ureq::Error::Timeout(timeout.reason));
            }
            // A server that closes TLS without a close_notify has still sent what it sent.
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => 0,
            other => other.map_err(ureq::Error::Io)?,
        };
        self.buffers.input_appended(amount);
        Ok(amount > 0)
    }

    fn is_open(&mut self) -> bool {
        let socket = self.io.socket();
        if socket.set_nonblocking(true).is_err() {
            return false;
        }
        let mut byte = [0u8];
        let open = matches!((&*socket).read(&mut byte), Err(error) if error.kind() == io::ErrorKind::WouldBlock);
        socket.set_nonblocking(false).is_ok() && open
    }

    fn is_tls(&self) -> bool {
        matches!(self.io, Io::Tls(_))
    }
}

/// A `ureq` agent that answers an origin server's `401` with Basic, Negotiate or NTLM, as an
/// [`OriginAuth`] says.
///
/// `ureq` runs a request once, and its middleware cannot send it again, so this wraps the agent
/// and runs the requests whose server has a rule itself: it sends the request, answers a `401`
/// with the next leg of the login and sends it again, on the agent's pooled connection, which an
/// NTLM login needs. The body is sent from memory once per leg, so a request is given as bytes.
/// A request to a server with no rule, or one that already holds an `Authorization` header, is
/// sent as it is.
///
/// A `401` that comes from another host than the request's (after a redirect) is not answered:
/// the login belongs to the host that was asked.
#[derive(Debug, Clone)]
pub struct OriginAgent {
    agent: ureq::Agent,
    auth: Arc<OriginAuth>,
}

impl OriginAgent {
    /// Wraps `agent` with the logins `auth` describes.
    pub fn new(agent: ureq::Agent, auth: OriginAuth) -> Self {
        Self {
            agent,
            auth: Arc::new(auth),
        }
    }

    /// The agent underneath, for requests that need no login.
    pub fn agent(&self) -> &ureq::Agent {
        &self.agent
    }

    /// Runs `request`, answering a `401` as the policy says.
    ///
    /// The agent's `http_status_as_error` setting applies to the final reply, as for the agent
    /// itself; a per-request setting is replaced by the agent's for a server with a rule.
    ///
    /// # Errors
    ///
    /// What `ureq` reports, and a failed login as an I/O error of kind `PermissionDenied`.
    pub fn run(
        &self,
        request: ureq::http::Request<impl AsRef<[u8]>>,
    ) -> Result<ureq::http::Response<ureq::Body>, ureq::Error> {
        use ureq::http::header::{AUTHORIZATION, WWW_AUTHENTICATE};

        let (parts, body) = request.into_parts();
        let body = body.as_ref();
        let login = if parts.headers.contains_key(AUTHORIZATION) {
            None
        } else {
            self.auth
                .login(&parts.uri.to_string())
                .map_err(login_error)?
        };
        let Some(mut login) = login else {
            return self.send(&parts, None, body, false);
        };
        let host = parts.uri.host().map(str::to_ascii_lowercase);
        let status_as_error = self.agent.config().http_status_as_error();
        let mut answer: Option<Answer> = None;
        loop {
            let authorization = answer.as_ref().map(Answer::authorization);
            let probe = answer.as_ref().is_some_and(Answer::probe);
            let mut response =
                self.send(&parts, authorization, if probe { &[] } else { body }, true)?;
            let values: Vec<String> = response
                .headers()
                .get_all(WWW_AUTHENTICATE)
                .iter()
                .filter_map(|value| value.to_str().ok().map(str::to_string))
                .collect();
            let values: Vec<&str> = values.iter().map(String::as_str).collect();
            let theirs = ureq::ResponseExt::get_uri(&response)
                .host()
                .map(str::to_ascii_lowercase)
                == host;
            if response.status() == ureq::http::StatusCode::UNAUTHORIZED && theirs {
                if let Some(next) = login.answer(&values).map_err(login_error)? {
                    drain(&mut response, self.auth.max_body());
                    answer = Some(next);
                    continue;
                }
            } else if theirs {
                login.finish(&values).map_err(login_error)?;
            }
            let status = response.status();
            if status_as_error && (status.is_client_error() || status.is_server_error()) {
                return Err(ureq::Error::StatusCode(status.as_u16()));
            }
            return Ok(response);
        }
    }

    /// One request of `parts` and `body`; with `leg` a client or server error is a reply, not an
    /// error, so the caller can read a `401`.
    fn send(
        &self,
        parts: &ureq::http::request::Parts,
        authorization: Option<&str>,
        body: &[u8],
        leg: bool,
    ) -> Result<ureq::http::Response<ureq::Body>, ureq::Error> {
        fn run_one<B: ureq::AsSendBody>(
            agent: &ureq::Agent,
            request: ureq::http::Request<B>,
            leg: bool,
        ) -> Result<ureq::http::Response<ureq::Body>, ureq::Error> {
            if leg {
                agent.run(
                    agent
                        .configure_request(request)
                        .http_status_as_error(false)
                        .build(),
                )
            } else {
                agent.run(request)
            }
        }
        // A body-less POST still says so (`Content-Length: 0`); `ureq` would send it chunked.
        let carries_body = matches!(
            parts.method,
            ureq::http::Method::POST | ureq::http::Method::PUT | ureq::http::Method::PATCH
        );
        if body.is_empty() && !carries_body {
            run_one(&self.agent, request_of(parts, authorization, ()), leg)
        } else {
            run_one(
                &self.agent,
                request_of(parts, authorization, body.to_vec()),
                leg,
            )
        }
    }
}

/// A request made of `parts` and `body`, with `authorization` set when given.
fn request_of<B>(
    parts: &ureq::http::request::Parts,
    authorization: Option<&str>,
    body: B,
) -> ureq::http::Request<B> {
    let mut request = ureq::http::Request::new(body);
    *request.method_mut() = parts.method.clone();
    *request.uri_mut() = parts.uri.clone();
    *request.version_mut() = parts.version;
    *request.headers_mut() = parts.headers.clone();
    // `ureq` states the length of the body each leg carries, which is not the first leg's.
    request
        .headers_mut()
        .remove(ureq::http::header::CONTENT_LENGTH);
    request
        .headers_mut()
        .remove(ureq::http::header::TRANSFER_ENCODING);
    *request.extensions_mut() = parts.extensions.clone();
    if let Some(value) = authorization
        && let Ok(mut value) = ureq::http::HeaderValue::from_str(value)
    {
        value.set_sensitive(true);
        request
            .headers_mut()
            .insert(ureq::http::header::AUTHORIZATION, value);
    }
    request
}

/// Reads and drops a reply's body, up to `limit` bytes, so the connection can carry the next leg.
fn drain(response: &mut ureq::http::Response<ureq::Body>, limit: usize) {
    let mut reader = response.body_mut().as_reader();
    let _ = io::copy(&mut Read::take(&mut reader, limit as u64), &mut io::sink());
}

fn login_error(error: Error) -> ureq::Error {
    ureq::Error::Io(io::Error::new(
        io::ErrorKind::PermissionDenied,
        error.to_string(),
    ))
}
