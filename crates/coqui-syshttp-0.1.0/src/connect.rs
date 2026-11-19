//! Connecting through a proxy: a direct connection, an HTTP `CONNECT` tunnel logged in with
//! Basic credentials, a proxy's extra headers, or Negotiate and NTLM, or a SOCKS tunnel.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::net::SocketAddr;
use core::time::Duration;
use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};

use crate::auth::{self, Negotiate, Scheme};
use crate::error::{Error, Reason};
use crate::proxy::{Proxy, ProxyScheme, Route, join_host_port};
use crate::resolver::Resolver;

/// The longest reply head read from a proxy.
const MAX_HEAD: usize = 64 * 1024;
/// The longest reply body drained between the legs of a login.
const MAX_BODY: usize = 64 * 1024;

/// A connected stream: to the server directly, or through a proxy's tunnel.
#[derive(Debug)]
#[non_exhaustive]
pub enum Connection {
    /// A plain TCP connection (a tunnel through an `http:` proxy is one).
    Tcp(TcpStream),
    /// A TCP connection inside TLS to an `https:` proxy.
    #[cfg(feature = "tls")]
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl Connection {
    /// The TCP socket underneath, for timeouts and shutdown.
    pub fn socket(&self) -> &TcpStream {
        match self {
            Self::Tcp(stream) => stream,
            #[cfg(feature = "tls")]
            Self::Tls(stream) => &stream.sock,
        }
    }
}

impl Read for Connection {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Tcp(stream) => stream.read(buffer),
            #[cfg(feature = "tls")]
            Self::Tls(stream) => stream.read(buffer),
        }
    }
}

impl Write for Connection {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        match self {
            Self::Tcp(stream) => stream.write(buffer),
            #[cfg(feature = "tls")]
            Self::Tls(stream) => stream.write(buffer),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Tcp(stream) => stream.flush(),
            #[cfg(feature = "tls")]
            Self::Tls(stream) => stream.flush(),
        }
    }
}

/// Makes connections, through a proxy where a [`Route`] says so.
#[derive(Debug, Clone)]
pub struct ProxyConnector {
    timeout: Duration,
    negotiate: Option<Negotiate>,
    user_agent: String,
    #[cfg(feature = "tls")]
    proxy_tls: Option<alloc::sync::Arc<rustls::ClientConfig>>,
}

/// Builds a [`ProxyConnector`].
#[derive(Debug, Clone)]
#[must_use]
pub struct ProxyConnectorBuilder {
    connector: ProxyConnector,
}

impl ProxyConnectorBuilder {
    /// The time limit of each connect and each read or write on the proxy connection (default
    /// 30 seconds).
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.connector.timeout = timeout;
        self
    }

    /// The proxies to log in to with Negotiate or NTLM (default: none).
    pub fn negotiate(mut self, policy: Negotiate) -> Self {
        self.connector.negotiate = Some(policy);
        self
    }

    /// The `User-Agent` sent in a `CONNECT` (default `coqui-syshttp`).
    pub fn user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.connector.user_agent = user_agent.into();
        self
    }

    /// The TLS configuration for speaking to an `https:` proxy (default: the system's).
    #[cfg(feature = "tls")]
    #[cfg_attr(docsrs, doc(cfg(feature = "tls")))]
    pub fn proxy_tls(mut self, config: alloc::sync::Arc<rustls::ClientConfig>) -> Self {
        self.connector.proxy_tls = Some(config);
        self
    }

    /// The connector.
    pub fn build(self) -> ProxyConnector {
        self.connector
    }
}

impl Default for ProxyConnector {
    fn default() -> Self {
        Self::builder().build()
    }
}

impl ProxyConnector {
    /// Starts a connector.
    pub fn builder() -> ProxyConnectorBuilder {
        ProxyConnectorBuilder {
            connector: Self {
                timeout: Duration::from_secs(30),
                negotiate: None,
                user_agent: "coqui-syshttp".to_string(),
                #[cfg(feature = "tls")]
                proxy_tls: None,
            },
        }
    }

    /// The same connector with another time limit.
    #[cfg(feature = "ureq")]
    pub(crate) fn with_timeout(&self, timeout: Duration) -> Self {
        Self {
            timeout,
            ..self.clone()
        }
    }

    /// Connects to `host:port` over `route`: straight to it, or through a `CONNECT` tunnel of an
    /// HTTP or HTTPS proxy. A plain `http:` origin is tunnelled too, so a proxy that allows
    /// `CONNECT` only to port 443 refuses it.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] when a connection fails, [`Error::Login`] when the proxy refuses the
    /// credentials, and [`Error::Refused`] for any other refusal.
    pub fn connect(&self, route: &Route, host: &str, port: u16) -> Result<Connection, Error> {
        match route {
            Route::Direct => self.connect_tcp(host, port).map(Connection::Tcp),
            Route::Proxy(proxy) => self.tunnel(proxy, host, port),
        }
    }

    /// Connects to the server of `url` over the first route of `resolver` that works, reporting
    /// each proxy that does not.
    ///
    /// # Errors
    ///
    /// [`Error::Parse`] for a URL without a host, [`Error::NoReachableProxy`] when every route
    /// failed to connect, and the first non-connection failure otherwise (a refused login stops
    /// the search).
    pub fn connect_url(&self, resolver: &Resolver, url: &str) -> Result<Connection, Error> {
        let (host, port) = split_url(url)?;
        let mut last = None;
        for route in resolver.lookup(url)? {
            match self.connect(&route, &host, port) {
                Ok(connection) => return Ok(connection),
                Err(error @ (Error::Login(_) | Error::Refused(_))) => return Err(error),
                Err(error) => {
                    tracing::debug!(%error, "a route failed");
                    if let Route::Proxy(proxy) = &route {
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

    fn connect_tcp(&self, host: &str, port: u16) -> Result<TcpStream, Error> {
        let addresses: Vec<SocketAddr> = (host, port).to_socket_addrs()?.collect();
        let mut last = None;
        for address in addresses {
            match TcpStream::connect_timeout(&address, self.timeout) {
                Ok(stream) => {
                    stream.set_read_timeout(Some(self.timeout))?;
                    stream.set_write_timeout(Some(self.timeout))?;
                    stream.set_nodelay(true)?;
                    return Ok(stream);
                }
                Err(error) => last = Some(error),
            }
        }
        Err(Error::Io(last.unwrap_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "no address")
        })))
    }

    fn tunnel(&self, proxy: &Proxy, host: &str, port: u16) -> Result<Connection, Error> {
        let mut tcp = self.connect_tcp(proxy.host(), proxy.port())?;
        if matches!(proxy.scheme(), ProxyScheme::Socks4 | ProxyScheme::Socks5) {
            crate::socks::connect(&mut tcp, proxy.scheme(), proxy.credentials(), host, port)?;
            return Ok(Connection::Tcp(tcp));
        }
        let mut connection = if proxy.scheme() == ProxyScheme::Https {
            self.wrap_tls(tcp, proxy)?
        } else {
            Connection::Tcp(tcp)
        };
        let secure = proxy.scheme() == ProxyScheme::Https;
        self.connect_request(&mut connection, proxy, host, port, secure)?;
        Ok(connection)
    }

    #[cfg(feature = "tls")]
    fn wrap_tls(&self, tcp: TcpStream, proxy: &Proxy) -> Result<Connection, Error> {
        let config = match &self.proxy_tls {
            Some(config) => alloc::sync::Arc::clone(config),
            None => crate::tls::Tls::builder()
                .build()?
                .client_config(proxy.host())?,
        };
        let name = rustls::pki_types::ServerName::try_from(proxy.host().to_string())
            .map_err(|_| Error::Tls(Reason::new("the proxy's host is not a valid name")))?;
        let session = rustls::ClientConnection::new(config, name)
            .map_err(|error| Error::Tls(Reason::new(error.to_string())))?;
        let mut stream = rustls::StreamOwned::new(session, tcp);
        // Complete the handshake now so a certificate failure is reported as one.
        while stream.conn.is_handshaking() {
            stream.conn.complete_io(&mut stream.sock)?;
        }
        Ok(Connection::Tls(Box::new(stream)))
    }

    #[cfg(not(feature = "tls"))]
    #[expect(clippy::unused_self, reason = "same signature as the `tls` build")]
    fn wrap_tls(&self, _tcp: TcpStream, _proxy: &Proxy) -> Result<Connection, Error> {
        Err(Error::Unsupported(Reason::new(
            "an https proxy needs the `tls` feature",
        )))
    }

    /// Sends `CONNECT` and logs in: Basic from the proxy's credentials, its extra headers, or a
    /// Negotiate or NTLM handshake on the same connection.
    fn connect_request(
        &self,
        connection: &mut Connection,
        proxy: &Proxy,
        host: &str,
        port: u16,
        secure: bool,
    ) -> Result<(), Error> {
        let authority = join_host_port(host, port);
        let mut authorization = proxy.credentials().map(auth::basic_header);
        let mut handshake: Option<(Scheme, alloc::boxed::Box<dyn auth::Client>)> = None;
        let max_legs = self
            .negotiate
            .as_ref()
            .map_or(auth::DEFAULT_MAX_LEGS, Negotiate::max_legs);
        for _leg in 0..=max_legs {
            self.send_connect(connection, proxy, &authority, authorization.as_deref())?;
            let head = read_head(connection)?;
            match head.status {
                200 => return self.finish_handshake(handshake, &head),
                407 => {}
                status => {
                    return Err(Error::Refused(Reason::new(alloc::format!(
                        "status {status}"
                    ))));
                }
            }
            let challenges = auth::parse_challenges(head.all("proxy-authenticate"));
            if proxy.credentials().is_some() {
                return Err(Error::Login(Reason::new(
                    "the proxy refused the credentials",
                )));
            }
            let Some(policy) = self
                .negotiate
                .as_ref()
                .filter(|policy| policy.wants_proxy(proxy.host()))
            else {
                return Err(Error::Login(Reason::new(
                    "the proxy asks for a login that is not configured",
                )));
            };
            let server_token = match &handshake {
                None => None,
                Some((scheme, _)) => challenges
                    .iter()
                    .find(|challenge| challenge.scheme == scheme.header_name().to_ascii_lowercase())
                    .and_then(|challenge| challenge.token.as_deref())
                    .map(|text| auth::decode_token(text, policy.max_token()))
                    .transpose()?,
            };
            if handshake.is_none() {
                let scheme = Scheme::choose(&challenges).ok_or_else(|| {
                    Error::Login(Reason::new("the proxy offers no scheme this client speaks"))
                })?;
                policy.check_transport(proxy.host(), secure)?;
                handshake = Some((
                    scheme,
                    auth::start(scheme, proxy.host(), policy.credentials())?,
                ));
            } else if server_token.is_none() {
                return Err(Error::Login(Reason::new(
                    "the proxy refused the credentials",
                )));
            }
            drain_body(connection, &head)?;
            if head
                .header("connection")
                .is_some_and(|value| value.eq_ignore_ascii_case("close"))
            {
                return Err(Error::Login(Reason::new(
                    "the proxy closed the connection during the login",
                )));
            }
            let Some((scheme, client)) = handshake.as_mut() else {
                return Err(Error::Login(Reason::new("no handshake")));
            };
            let step = client.step(server_token.as_deref())?;
            let Some(token) = step.token else {
                return Err(Error::Login(Reason::new("the handshake produced no token")));
            };
            if token.len() > policy.max_token() {
                return Err(Error::Login(Reason::new("a token is over the limit")));
            }
            authorization = Some(auth::header_value(*scheme, &token));
        }
        Err(Error::Login(Reason::new("the login took too many legs")))
    }

    fn finish_handshake(
        &self,
        handshake: Option<(Scheme, alloc::boxed::Box<dyn auth::Client>)>,
        head: &Head,
    ) -> Result<(), Error> {
        let Some((scheme, mut client)) = handshake else {
            return Ok(());
        };
        // A mechanism that proves the server (Kerberos) sends its closing token with the 200.
        let max = self
            .negotiate
            .as_ref()
            .map_or(auth::DEFAULT_MAX_TOKEN, Negotiate::max_token);
        let closing = auth::parse_challenges(head.all("proxy-authenticate"))
            .into_iter()
            .find(|challenge| challenge.scheme == scheme.header_name().to_ascii_lowercase())
            .and_then(|challenge| challenge.token);
        if let Some(text) = closing {
            let token = auth::decode_token(&text, max)?;
            client.step(Some(&token))?;
        }
        Ok(())
    }

    fn send_connect(
        &self,
        connection: &mut Connection,
        proxy: &Proxy,
        authority: &str,
        authorization: Option<&str>,
    ) -> Result<(), Error> {
        let mut request = alloc::format!(
            "CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\nProxy-Connection: Keep-Alive\r\nUser-Agent: {}\r\n",
            self.user_agent
        );
        let own_authorization = proxy
            .headers()
            .any(|(name, _)| name.eq_ignore_ascii_case("proxy-authorization"));
        if let Some(value) = authorization.filter(|_| !own_authorization) {
            request.push_str("Proxy-Authorization: ");
            request.push_str(value);
            request.push_str("\r\n");
        }
        for (name, value) in proxy.headers() {
            request.push_str(name);
            request.push_str(": ");
            request.push_str(value);
            request.push_str("\r\n");
        }
        request.push_str("\r\n");
        connection.write_all(request.as_bytes())?;
        connection.flush()?;
        Ok(())
    }
}

fn split_url(url: &str) -> Result<(String, u16), Error> {
    let target = crate::proxy::Target::parse(url)?;
    if target.port == 0 {
        return Err(Error::Parse("url: it has no port"));
    }
    Ok((target.host, target.port))
}

/// A reply's status and headers.
#[derive(Debug)]
struct Head {
    status: u16,
    headers: Vec<(String, String)>,
}

impl Head {
    fn header(&self, name: &str) -> Option<&str> {
        self.all(name).first().copied()
    }

    fn all(&self, name: &str) -> Vec<&str> {
        self.headers
            .iter()
            .filter(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
            .collect()
    }
}

/// Reads a reply head one byte at a time, so nothing of the tunnel behind it is consumed.
fn read_head(reader: &mut impl Read) -> Result<Head, Error> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() >= MAX_HEAD {
            return Err(Error::Refused(Reason::new("the reply head is too long")));
        }
        if reader.read(&mut byte)? == 0 {
            return Err(Error::Refused(Reason::new(
                "the proxy closed the connection",
            )));
        }
        head.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&head);
    let mut lines = text.split("\r\n");
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| Error::Refused(Reason::new("the reply is not HTTP")))?;
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_string(), value.trim().to_string()))
        .collect();
    Ok(Head { status, headers })
}

/// Reads and drops a reply's body so the connection can carry the next request.
fn drain_body(reader: &mut impl Read, head: &Head) -> Result<(), Error> {
    if head
        .header("transfer-encoding")
        .is_some_and(|value| value.to_ascii_lowercase().contains("chunked"))
    {
        loop {
            let mut size_line = Vec::new();
            let mut byte = [0u8; 1];
            while !size_line.ends_with(b"\r\n") {
                if reader.read(&mut byte)? == 0 || size_line.len() > 32 {
                    return Err(Error::Login(Reason::new("the proxy's reply ended early")));
                }
                size_line.push(byte[0]);
            }
            let size = core::str::from_utf8(&size_line)
                .ok()
                .and_then(|line| {
                    usize::from_str_radix(line.trim().split(';').next().unwrap_or(""), 16).ok()
                })
                .ok_or_else(|| Error::Login(Reason::new("the proxy's reply is malformed")))?;
            skip(reader, size + 2)?;
            if size == 0 {
                return Ok(());
            }
        }
    }
    let length: usize = head
        .header("content-length")
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    skip(reader, length)
}

fn skip(reader: &mut impl Read, mut length: usize) -> Result<(), Error> {
    if length > MAX_BODY {
        return Err(Error::Login(Reason::new("the proxy's reply is too long")));
    }
    let mut buffer = [0u8; 4096];
    while length > 0 {
        let wanted = length.min(buffer.len());
        let count = reader.read(&mut buffer[..wanted])?;
        if count == 0 {
            return Err(Error::Login(Reason::new("the proxy's reply ended early")));
        }
        length -= count;
    }
    Ok(())
}
