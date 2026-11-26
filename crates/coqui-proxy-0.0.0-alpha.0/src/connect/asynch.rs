//! The asynchronous driver of the connector, on Tokio: the same sans-I/O cores as the blocking
//! [`ProxyConnector`](super::ProxyConnector), over `tokio::net`.
//!
//! The [`Resolver`] is synchronous (it may read the operating system's settings or run a PAC
//! script), so the driver asks it through [`tokio::task::spawn_blocking`]: it needs a Tokio
//! runtime, and a multi-thread or a current-thread one both work.

use alloc::collections::VecDeque;
use alloc::string::ToString;
use alloc::vec::Vec;
use core::net::SocketAddr;
use core::pin::Pin;
use core::task::{Context, Poll};
use core::time::Duration;
use std::io;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::TcpStream;
use tokio::task::JoinSet;

use super::blocking::interleaved;
use super::core::{
    BodyFraming, ConnectRequest, ConnectResponse, Outcome, Progress, ProxyLogin, decode_chunk_size,
};
use super::socks::{Greeting, Method, PasswordLogin, Reply, Request, Version};
use crate::error::{Error, Reason};
use crate::proxy::{Credentials, Proxy, ProxyScheme, Route, join_host_port};
use crate::resolver::Resolver;
use crate::settings::ConnectorSettings;

/// A connected stream: to the server directly, or through a proxy's tunnel.
#[derive(Debug)]
#[non_exhaustive]
pub enum Connection {
    /// A plain TCP connection (a tunnel through an `http:` proxy is one).
    Tcp(TcpStream),
    /// A TCP connection inside TLS to an `https:` proxy.
    #[cfg(feature = "tls")]
    Tls(alloc::boxed::Box<tokio_rustls::client::TlsStream<TcpStream>>),
}

impl AsyncRead for Connection {
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Tcp(stream) => Pin::new(stream).poll_read(context, buffer),
            #[cfg(feature = "tls")]
            Self::Tls(stream) => Pin::new(stream.as_mut()).poll_read(context, buffer),
        }
    }
}

impl AsyncWrite for Connection {
    fn poll_write(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Self::Tcp(stream) => Pin::new(stream).poll_write(context, buffer),
            #[cfg(feature = "tls")]
            Self::Tls(stream) => Pin::new(stream.as_mut()).poll_write(context, buffer),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Tcp(stream) => Pin::new(stream).poll_flush(context),
            #[cfg(feature = "tls")]
            Self::Tls(stream) => Pin::new(stream.as_mut()).poll_flush(context),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Tcp(stream) => Pin::new(stream).poll_shutdown(context),
            #[cfg(feature = "tls")]
            Self::Tls(stream) => Pin::new(stream.as_mut()).poll_shutdown(context),
        }
    }
}

/// The routes of `resolver` for `url`, asked on Tokio's blocking pool so that reading the system's
/// settings or running a PAC script never stalls the runtime's workers.
///
/// # Errors
///
/// As [`Resolver::lookup`]; [`Error::Io`] if the blocking task is lost.
pub async fn lookup(resolver: &Resolver, url: &str) -> Result<Vec<Route>, Error> {
    let resolver = resolver.clone();
    let url = url.to_string();
    tokio::task::spawn_blocking(move || resolver.lookup(&url))
        .await
        .map_err(|error| Error::Io(io::Error::other(error)))?
}

/// Makes connections on Tokio, through a proxy where a [`Route`] says so.
///
/// It is made from a blocking [`ProxyConnector`](super::ProxyConnector) and shares its settings,
/// Negotiate policy, proxy TLS and memory of failed proxies.
#[derive(Debug, Clone)]
pub struct ProxyConnector {
    inner: super::ProxyConnector,
}

impl From<super::ProxyConnector> for ProxyConnector {
    fn from(inner: super::ProxyConnector) -> Self {
        Self { inner }
    }
}

impl Default for ProxyConnector {
    fn default() -> Self {
        super::ProxyConnector::default().into()
    }
}

impl super::ProxyConnector {
    /// The same connector, driven by Tokio.
    #[must_use]
    pub fn asynch(&self) -> ProxyConnector {
        self.clone().into()
    }
}

impl ProxyConnector {
    /// The same connector with another time limit.
    #[must_use]
    pub fn with_timeout(&self, timeout: Duration) -> Self {
        self.inner.with_timeout(timeout).into()
    }

    /// The same connector with another failover choice and failed-proxy memory time.
    #[must_use]
    pub fn with_failover(&self, failover: bool, bad_proxy_ttl: Duration) -> Self {
        self.inner.with_failover(failover, bad_proxy_ttl).into()
    }

    /// The blocking connector this one shares its settings with.
    #[must_use]
    pub fn blocking(&self) -> &super::ProxyConnector {
        &self.inner
    }

    /// Connects to `host:port` over `route`: straight to it, or through a `CONNECT` tunnel of an
    /// HTTP or HTTPS proxy, or a SOCKS proxy.
    ///
    /// # Errors
    ///
    /// As [`ProxyConnector::connect`](super::ProxyConnector::connect).
    pub async fn connect(&self, route: &Route, host: &str, port: u16) -> Result<Connection, Error> {
        match route {
            Route::Direct => self.connect_tcp(host, port).await.map(Connection::Tcp),
            Route::Proxy(proxy) => self.tunnel(proxy, host, port).await,
        }
    }

    /// Connects to the server of `url` over the first route of `resolver` that works, reporting
    /// each proxy that does not. The resolver is asked through [`lookup`].
    ///
    /// # Errors
    ///
    /// As [`ProxyConnector::connect_url`](super::ProxyConnector::connect_url).
    pub async fn connect_url(&self, resolver: &Resolver, url: &str) -> Result<Connection, Error> {
        let (host, port) = super::split_url(url)?;
        let routes = self.inner.order(lookup(resolver, url).await?);
        let mut last = None;
        for route in routes {
            match self.connect(&route, &host, port).await {
                Ok(connection) => return Ok(connection),
                Err(error @ (Error::Login(_) | Error::Refused(_))) => return Err(error),
                Err(error) => {
                    tracing::debug!(%error, "a route failed");
                    if let Route::Proxy(proxy) = &route {
                        self.inner.note_failure(proxy);
                        resolver.report_failure(proxy);
                    }
                    last = Some(error);
                }
            }
        }
        Err(match last {
            Some(Error::Unsupported(reason)) => Error::Unsupported(reason),
            _ => Error::NoReachableProxy,
        })
    }

    fn timeout(&self) -> Duration {
        self.inner.settings.timeout
    }

    async fn limited<T>(
        &self,
        what: impl core::future::Future<Output = Result<T, Error>>,
    ) -> Result<T, Error> {
        tokio::time::timeout(self.timeout(), what)
            .await
            .map_err(|_| Error::Io(io::Error::new(io::ErrorKind::TimedOut, "timed out")))?
    }

    async fn connect_tcp(&self, host: &str, port: u16) -> Result<TcpStream, Error> {
        let timeout = self.timeout();
        let addresses: Vec<SocketAddr> = tokio::time::timeout(
            timeout,
            tokio::net::lookup_host((host.trim_matches(['[', ']']), port)),
        )
        .await
        .map_err(|_| {
            Error::Io(io::Error::new(
                io::ErrorKind::TimedOut,
                "name lookup timed out",
            ))
        })??
        .collect();
        let stream = race(
            interleaved(addresses),
            timeout,
            self.inner.settings.happy_eyeballs_delay,
        )
        .await?;
        stream.set_nodelay(true)?;
        Ok(stream)
    }

    async fn tunnel(&self, proxy: &Proxy, host: &str, port: u16) -> Result<Connection, Error> {
        if proxy.is_gateway() {
            return Err(Error::Unsupported(Reason::new(
                "a URL-prefix gateway is asked for the URL, it is not a tunnel",
            )));
        }
        let mut tcp = self.connect_tcp(proxy.host(), proxy.port()).await?;
        if matches!(proxy.scheme(), ProxyScheme::Socks4 | ProxyScheme::Socks5) {
            self.limited(socks(
                &mut tcp,
                proxy.scheme(),
                proxy.credentials(),
                host,
                port,
            ))
            .await?;
            return Ok(Connection::Tcp(tcp));
        }
        let secure = proxy.scheme() == ProxyScheme::Https;
        let mut connection = if secure {
            self.limited(self.wrap_tls(tcp, proxy)).await?
        } else {
            Connection::Tcp(tcp)
        };
        self.limited(self.connect_request(&mut connection, proxy, host, port, secure))
            .await?;
        Ok(connection)
    }

    #[cfg(feature = "tls")]
    async fn wrap_tls(&self, tcp: TcpStream, proxy: &Proxy) -> Result<Connection, Error> {
        let config = match &self.inner.proxy_tls {
            Some(config) => alloc::sync::Arc::clone(config),
            None => crate::tls::platform_config()?,
        };
        let name = rustls::pki_types::ServerName::try_from(proxy.host().to_string())
            .map_err(|_| Error::Tls(Reason::new("the proxy's host is not a valid name")))?;
        let stream = tokio_rustls::TlsConnector::from(config)
            .connect(name, tcp)
            .await?;
        Ok(Connection::Tls(alloc::boxed::Box::new(stream)))
    }

    #[cfg(not(feature = "tls"))]
    #[expect(clippy::unused_async, reason = "same signature as the `tls` build")]
    async fn wrap_tls(&self, _tcp: TcpStream, _proxy: &Proxy) -> Result<Connection, Error> {
        Err(Error::Unsupported(Reason::new(
            "an https proxy needs the `tls` feature",
        )))
    }

    /// Sends `CONNECT` and logs in, as the blocking connector does.
    async fn connect_request(
        &self,
        connection: &mut Connection,
        proxy: &Proxy,
        host: &str,
        port: u16,
        secure: bool,
    ) -> Result<(), Error> {
        let settings = &self.inner.settings;
        let authority = join_host_port(host, port);
        let mut login = ProxyLogin::new(proxy, self.inner.negotiate.as_ref(), secure);
        let mut authorization = login.first_authorization();
        loop {
            let mut request = ConnectRequest::new(&authority, &settings.user_agent)
                .authorization(authorization.take());
            for (name, value) in proxy.headers() {
                request = request.header(name, value);
            }
            connection.write_all(&request.encode()).await?;
            connection.flush().await?;
            let head = read_head(connection, settings).await?;
            match login.answer(&head)? {
                Outcome::Established => return Ok(()),
                Outcome::Retry {
                    authorization: next,
                } => {
                    drain_body(connection, &head, settings).await?;
                    authorization = Some(next);
                }
            }
        }
    }
}

fn io_error(error: io::Error) -> Error {
    if error.kind() == io::ErrorKind::UnexpectedEof {
        Error::Refused(Reason::new("the proxy closed the connection"))
    } else {
        Error::Io(error)
    }
}

/// Reads until `decode` has a value, asking for no more bytes than it says it needs.
async fn read_until<T>(
    stream: &mut (impl AsyncRead + Unpin),
    mut decode: impl FnMut(&[u8]) -> Result<Progress<T>, Error>,
) -> Result<T, Error> {
    let mut buffer = Vec::new();
    loop {
        match decode(&buffer)? {
            Progress::Complete { value, .. } => return Ok(value),
            Progress::Incomplete(more) => {
                let start = buffer.len();
                buffer.resize(start + more, 0);
                stream
                    .read_exact(&mut buffer[start..])
                    .await
                    .map_err(io_error)?;
            }
        }
    }
}

async fn socks(
    stream: &mut (impl AsyncRead + AsyncWrite + Unpin),
    scheme: ProxyScheme,
    credentials: Option<&Credentials>,
    host: &str,
    port: u16,
) -> Result<(), Error> {
    match scheme {
        ProxyScheme::Socks4 => {
            let user = credentials.map(Credentials::username);
            let request = Request::new(Version::V4, host, port, user).encode()?;
            stream.write_all(&request).await?;
            stream.flush().await?;
            read_until(stream, Reply::decode4).await.map(|_| ())
        }
        ProxyScheme::Socks5 => socks5(stream, credentials, host, port).await,
        _ => Err(Error::Unsupported(Reason::new("not a SOCKS proxy"))),
    }
}

async fn socks5(
    stream: &mut (impl AsyncRead + AsyncWrite + Unpin),
    credentials: Option<&Credentials>,
    host: &str,
    port: u16,
) -> Result<(), Error> {
    stream
        .write_all(&Greeting::new(credentials.is_some()).encode())
        .await?;
    stream.flush().await?;
    match read_until(stream, Greeting::decode_choice).await? {
        Method::None => {}
        Method::Password => {
            let credentials = credentials.ok_or_else(|| {
                Error::Login(Reason::new(
                    "the SOCKS 5 proxy asks for a login that is not configured",
                ))
            })?;
            stream
                .write_all(&PasswordLogin::new(credentials).encode()?)
                .await?;
            stream.flush().await?;
            read_until(stream, PasswordLogin::decode_verdict).await?;
        }
    }
    stream
        .write_all(&Request::new(Version::V5, host, port, None).encode()?)
        .await?;
    stream.flush().await?;
    read_until(stream, Reply::decode5).await.map(|_| ())
}

/// Connects to the first address that answers: each attempt starts when the one before it failed
/// or has run for `delay`, and the first success wins; the others are dropped.
async fn race(
    addresses: Vec<SocketAddr>,
    timeout: Duration,
    delay: Duration,
) -> Result<TcpStream, Error> {
    let mut queue: VecDeque<SocketAddr> = addresses.into();
    let mut attempts = JoinSet::new();
    let mut last = None;
    loop {
        if let Some(address) = queue.pop_front() {
            attempts.spawn(async move {
                match tokio::time::timeout(timeout, TcpStream::connect(address)).await {
                    Ok(result) => result,
                    Err(_) => Err(io::Error::new(io::ErrorKind::TimedOut, "connect timed out")),
                }
            });
        } else if attempts.is_empty() {
            break;
        }
        let finished = if queue.is_empty() {
            match attempts.join_next().await {
                Some(finished) => finished,
                None => break,
            }
        } else {
            match tokio::time::timeout(delay, attempts.join_next()).await {
                Ok(Some(finished)) => finished,
                Ok(None) | Err(_) => continue,
            }
        };
        match finished {
            Ok(Ok(stream)) => return Ok(stream),
            Ok(Err(error)) => last = Some(error),
            Err(error) => last = Some(io::Error::other(error)),
        }
    }
    Err(Error::Io(last.unwrap_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "no address")
    })))
}

/// Reads a reply head one byte at a time, so nothing of the tunnel behind it is consumed.
async fn read_head(
    reader: &mut (impl AsyncRead + Unpin),
    settings: &ConnectorSettings,
) -> Result<ConnectResponse, Error> {
    let mut head = Vec::new();
    loop {
        let byte = reader.read_u8().await.map_err(io_error)?;
        head.push(byte);
        if byte != b'\n' && head.len() < settings.max_response_head_bytes {
            continue;
        }
        if let Progress::Complete { value, .. } = ConnectResponse::decode(&head, settings)? {
            return Ok(value);
        }
    }
}

/// Reads and drops a reply's body so the connection can carry the next request.
async fn drain_body(
    reader: &mut (impl AsyncRead + Unpin),
    head: &ConnectResponse,
    settings: &ConnectorSettings,
) -> Result<(), Error> {
    match head.body_framing() {
        BodyFraming::Length(length) => skip(reader, length, settings).await,
        BodyFraming::Chunked => {
            let mut total = 0usize;
            loop {
                let mut line = Vec::new();
                let size = loop {
                    let byte = reader
                        .read_u8()
                        .await
                        .map_err(|_| Error::Login(Reason::new("the proxy's reply ended early")))?;
                    line.push(byte);
                    if let Progress::Complete { value, .. } = decode_chunk_size(&line)? {
                        break value;
                    }
                };
                total = total.saturating_add(size);
                skip(reader, size.saturating_add(2), settings).await?;
                if size == 0 {
                    return Ok(());
                }
                if total > settings.max_response_body_bytes {
                    return Err(Error::Login(Reason::new("the proxy's reply is too long")));
                }
            }
        }
    }
}

async fn skip(
    reader: &mut (impl AsyncRead + Unpin),
    mut length: usize,
    settings: &ConnectorSettings,
) -> Result<(), Error> {
    if length > settings.max_response_body_bytes {
        return Err(Error::Login(Reason::new("the proxy's reply is too long")));
    }
    let mut buffer = [0u8; 4096];
    while length > 0 {
        let wanted = length.min(buffer.len());
        let count = reader.read(&mut buffer[..wanted]).await?;
        if count == 0 {
            return Err(Error::Login(Reason::new("the proxy's reply ended early")));
        }
        length -= count;
    }
    Ok(())
}
