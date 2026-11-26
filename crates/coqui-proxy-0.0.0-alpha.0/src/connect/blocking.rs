//! Drives the sans-I/O cores over `std::net`: the `CONNECT` exchange with its login, and the
//! SOCKS handshakes.

use alloc::vec::Vec;
use core::net::SocketAddr;
use core::time::Duration;
use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::mpsc;
use std::thread;

use super::core::{
    BodyFraming, ConnectRequest, ConnectResponse, Outcome, Progress, ProxyLogin, decode_chunk_size,
};
use super::socks::{Greeting, Method, PasswordLogin, Reply, Request, Version};
use super::{Connection, ProxyConnector};
use crate::error::{Error, Reason};
use crate::proxy::{Credentials, Proxy, ProxyScheme, Route, join_host_port};
use crate::resolver::Resolver;
use crate::settings::ConnectorSettings;

fn io_error(error: io::Error) -> Error {
    if error.kind() == io::ErrorKind::UnexpectedEof {
        Error::Refused(Reason::new("the proxy closed the connection"))
    } else {
        Error::Io(error)
    }
}

/// Reads until `decode` has a value, asking for no more bytes than it says it needs.
fn read_until<T>(
    stream: &mut impl Read,
    mut decode: impl FnMut(&[u8]) -> Result<Progress<T>, Error>,
) -> Result<T, Error> {
    let mut buffer = Vec::new();
    loop {
        match decode(&buffer)? {
            Progress::Complete { value, .. } => return Ok(value),
            Progress::Incomplete(more) => {
                let start = buffer.len();
                buffer.resize(start + more, 0);
                stream.read_exact(&mut buffer[start..]).map_err(io_error)?;
            }
        }
    }
}

/// Asks the proxy on `stream` to connect to `host:port`, logging in with `credentials` where the
/// proxy asks for it. On `Ok` the stream is the tunnel.
///
/// # Errors
///
/// [`Error::Login`] when the proxy wants a login that was not given or refuses it,
/// [`Error::Refused`] for any other refusal or a reply that is not SOCKS, [`Error::Unsupported`]
/// for what the protocol cannot say (an IPv6 address in SOCKS 4), [`Error::Io`] for the stream.
pub(super) fn connect(
    stream: &mut (impl Read + Write),
    scheme: ProxyScheme,
    credentials: Option<&Credentials>,
    host: &str,
    port: u16,
) -> Result<(), Error> {
    match scheme {
        ProxyScheme::Socks4 => {
            let user = credentials.map(Credentials::username);
            let request = Request::new(Version::V4, host, port, user).encode()?;
            stream.write_all(&request)?;
            stream.flush()?;
            read_until(stream, Reply::decode4).map(|_| ())
        }
        ProxyScheme::Socks5 => socks5(stream, credentials, host, port),
        _ => Err(Error::Unsupported(Reason::new("not a SOCKS proxy"))),
    }
}

fn socks5(
    stream: &mut (impl Read + Write),
    credentials: Option<&Credentials>,
    host: &str,
    port: u16,
) -> Result<(), Error> {
    stream.write_all(&Greeting::new(credentials.is_some()).encode())?;
    stream.flush()?;
    match read_until(stream, Greeting::decode_choice)? {
        Method::None => {}
        Method::Password => {
            let credentials = credentials.ok_or_else(|| {
                Error::Login(Reason::new(
                    "the SOCKS 5 proxy asks for a login that is not configured",
                ))
            })?;
            stream.write_all(&PasswordLogin::new(credentials).encode()?)?;
            stream.flush()?;
            read_until(stream, PasswordLogin::decode_verdict)?;
        }
    }
    stream.write_all(&Request::new(Version::V5, host, port, None).encode()?)?;
    stream.flush()?;
    read_until(stream, Reply::decode5).map(|_| ())
}

impl ProxyConnector {
    /// Connects to `host:port` over `route`: straight to it, or through a `CONNECT` tunnel of an
    /// HTTP or HTTPS proxy, or a SOCKS proxy. A plain `http:` origin is tunnelled too, so a proxy
    /// that allows `CONNECT` only to port 443 refuses it.
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
        let (host, port) = super::split_url(url)?;
        let mut last = None;
        for route in self.order(resolver.lookup(url)?) {
            match self.connect(&route, &host, port) {
                Ok(connection) => return Ok(connection),
                Err(error @ (Error::Login(_) | Error::Refused(_))) => return Err(error),
                Err(error) => {
                    tracing::debug!(%error, "a route failed");
                    if let Route::Proxy(proxy) = &route {
                        self.note_failure(proxy);
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
        let timeout = self.settings.timeout;
        let addresses = interleaved((host, port).to_socket_addrs()?.collect());
        let stream = race(addresses, timeout, self.settings.happy_eyeballs_delay)?;
        stream.set_read_timeout(Some(timeout))?;
        stream.set_write_timeout(Some(timeout))?;
        stream.set_nodelay(true)?;
        Ok(stream)
    }

    fn tunnel(&self, proxy: &Proxy, host: &str, port: u16) -> Result<Connection, Error> {
        if proxy.is_gateway() {
            return Err(Error::Unsupported(Reason::new(
                "a URL-prefix gateway is asked for the URL, it is not a tunnel",
            )));
        }
        let mut tcp = self.connect_tcp(proxy.host(), proxy.port())?;
        if matches!(proxy.scheme(), ProxyScheme::Socks4 | ProxyScheme::Socks5) {
            connect(&mut tcp, proxy.scheme(), proxy.credentials(), host, port)?;
            return Ok(Connection::Tcp(tcp));
        }
        let secure = proxy.scheme() == ProxyScheme::Https;
        let mut connection = if secure {
            self.wrap_tls(tcp, proxy)?
        } else {
            Connection::Tcp(tcp)
        };
        self.connect_request(&mut connection, proxy, host, port, secure)?;
        Ok(connection)
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
        let mut login = ProxyLogin::new(proxy, self.negotiate.as_ref(), secure);
        let mut authorization = login.first_authorization();
        loop {
            let mut request = ConnectRequest::new(&authority, &self.settings.user_agent)
                .authorization(authorization.take());
            for (name, value) in proxy.headers() {
                request = request.header(name, value);
            }
            connection.write_all(&request.encode())?;
            connection.flush()?;
            let head = read_head(connection, &self.settings)?;
            match login.answer(&head)? {
                Outcome::Established => return Ok(()),
                Outcome::Retry {
                    authorization: next,
                } => {
                    drain_body(connection, &head, &self.settings)?;
                    authorization = Some(next);
                }
            }
        }
    }
}

/// The addresses with the families alternating, the first family first (RFC 8305).
pub(super) fn interleaved(addresses: Vec<SocketAddr>) -> Vec<SocketAddr> {
    let Some(first) = addresses.first().map(SocketAddr::is_ipv6) else {
        return addresses;
    };
    let (mut same, mut other): (Vec<_>, Vec<_>) = addresses
        .into_iter()
        .partition(|address| address.is_ipv6() == first);
    same.reverse();
    other.reverse();
    let mut ordered = Vec::new();
    while !same.is_empty() || !other.is_empty() {
        ordered.extend(same.pop());
        ordered.extend(other.pop());
    }
    ordered
}

/// Connects to the first address that answers: each attempt starts when the one before it failed
/// or has run for `delay`, and the first success wins; slower attempts finish on their own.
fn race(
    addresses: Vec<SocketAddr>,
    timeout: Duration,
    delay: Duration,
) -> Result<TcpStream, Error> {
    let (sender, receiver) = mpsc::channel();
    let total = addresses.len();
    let mut last = None;
    let mut finished = 0;
    for address in addresses {
        let sender = sender.clone();
        drop(thread::spawn(move || {
            let _ = sender.send(TcpStream::connect_timeout(&address, timeout));
        }));
        match receiver.recv_timeout(delay) {
            Ok(Ok(stream)) => return Ok(stream),
            Ok(Err(error)) => {
                finished += 1;
                last = Some(error);
            }
            Err(_) => {}
        }
    }
    while finished < total {
        match receiver.recv() {
            Ok(Ok(stream)) => return Ok(stream),
            Ok(Err(error)) => {
                finished += 1;
                last = Some(error);
            }
            Err(_) => break,
        }
    }
    Err(Error::Io(last.unwrap_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "no address")
    })))
}

/// Reads a reply head one byte at a time, so nothing of the tunnel behind it is consumed.
fn read_head(
    reader: &mut impl Read,
    settings: &ConnectorSettings,
) -> Result<ConnectResponse, Error> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        if reader.read(&mut byte)? == 0 {
            return Err(Error::Refused(Reason::new(
                "the proxy closed the connection",
            )));
        }
        head.push(byte[0]);
        // Deciding on every byte would rescan the head; a head ends on a line end.
        if byte[0] != b'\n' && head.len() < settings.max_response_head_bytes {
            continue;
        }
        if let Progress::Complete { value, .. } = ConnectResponse::decode(&head, settings)? {
            return Ok(value);
        }
    }
}

/// Reads and drops a reply's body so the connection can carry the next request.
fn drain_body(
    reader: &mut impl Read,
    head: &ConnectResponse,
    settings: &ConnectorSettings,
) -> Result<(), Error> {
    match head.body_framing() {
        BodyFraming::Length(length) => skip(reader, length, settings),
        BodyFraming::Chunked => {
            let mut total = 0usize;
            loop {
                let mut line = Vec::new();
                let mut byte = [0u8; 1];
                let size = loop {
                    if reader.read(&mut byte)? == 0 {
                        return Err(Error::Login(Reason::new("the proxy's reply ended early")));
                    }
                    line.push(byte[0]);
                    if let Progress::Complete { value, .. } = decode_chunk_size(&line)? {
                        break value;
                    }
                };
                total = total.saturating_add(size);
                skip(reader, size.saturating_add(2), settings)?;
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

fn skip(
    reader: &mut impl Read,
    mut length: usize,
    settings: &ConnectorSettings,
) -> Result<(), Error> {
    if length > settings.max_response_body_bytes {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// A stream that reads canned bytes and records what was written.
    struct Canned {
        read: Cursor<Vec<u8>>,
        written: Vec<u8>,
    }

    impl Canned {
        fn new(reply: &[u8]) -> Self {
            Self {
                read: Cursor::new(reply.to_vec()),
                written: Vec::new(),
            }
        }
    }

    impl Read for Canned {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            self.read.read(buffer)
        }
    }

    impl Write for Canned {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            self.written.extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn socks5_sends_the_name_for_the_proxy_to_resolve() {
        let mut stream = Canned::new(&[5, 0, 5, 0, 0, 1, 1, 2, 3, 4, 0, 80]);
        connect(
            &mut stream,
            ProxyScheme::Socks5,
            None,
            "mail.corp.example",
            443,
        )
        .unwrap();
        let mut expected = Vec::from([5u8, 1, 0, 5, 1, 0, 3, 17]);
        expected.extend_from_slice(b"mail.corp.example");
        expected.extend_from_slice(&443u16.to_be_bytes());
        assert_eq!(stream.written, expected);
    }

    #[test]
    fn socks5_sends_an_address_as_an_address() {
        let mut stream = Canned::new(&[5, 0, 5, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
        connect(&mut stream, ProxyScheme::Socks5, None, "10.1.2.3", 25).unwrap();
        assert_eq!(&stream.written[3..], [5, 1, 0, 1, 10, 1, 2, 3, 0, 25]);
        let mut stream = Canned::new(&[
            5, 0, 5, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ]);
        connect(&mut stream, ProxyScheme::Socks5, None, "[::1]", 25).unwrap();
        assert_eq!(stream.written[6], 4);
    }

    #[test]
    fn socks5_logs_in_with_username_and_password_when_it_is_asked_for() {
        let mut stream = Canned::new(&[5, 2, 1, 0, 5, 0, 0, 3, 1, b'x', 0, 0]);
        let credentials = Credentials::new("ada", "s3cret");
        connect(&mut stream, ProxyScheme::Socks5, Some(&credentials), "h", 1).unwrap();
        assert_eq!(
            &stream.written[..13],
            [5, 2, 0, 2, 1, 3, b'a', b'd', b'a', 6, b's', b'3', b'c']
        );
    }

    #[test]
    fn socks5_refusals_are_told_apart() {
        let wanted = Canned::new(&[5, 2]);
        let mut stream = wanted;
        let error = connect(&mut stream, ProxyScheme::Socks5, None, "h", 1).unwrap_err();
        assert!(matches!(error, Error::Login(_)), "{error}");
        let mut stream = Canned::new(&[5, 0xff]);
        let error = connect(&mut stream, ProxyScheme::Socks5, None, "h", 1).unwrap_err();
        assert!(matches!(error, Error::Login(_)), "{error}");
        let mut stream = Canned::new(&[5, 0, 5, 5, 0, 1]);
        let error = connect(&mut stream, ProxyScheme::Socks5, None, "h", 1).unwrap_err();
        assert!(matches!(error, Error::Refused(_)), "{error}");
        let mut stream = Canned::new(&[5, 0]);
        let error = connect(&mut stream, ProxyScheme::Socks5, None, "h", 1).unwrap_err();
        assert!(matches!(error, Error::Refused(_)), "{error}");
    }

    #[test]
    fn socks4_sends_an_address_and_socks4a_a_name() {
        let mut stream = Canned::new(&[0, 0x5a, 0, 0, 0, 0, 0, 0]);
        connect(&mut stream, ProxyScheme::Socks4, None, "10.1.2.3", 80).unwrap();
        assert_eq!(stream.written, [4, 1, 0, 80, 10, 1, 2, 3, 0]);
        let mut stream = Canned::new(&[0, 0x5a, 0, 0, 0, 0, 0, 0]);
        let credentials = Credentials::new("ada", "ignored");
        connect(
            &mut stream,
            ProxyScheme::Socks4,
            Some(&credentials),
            "h.example",
            80,
        )
        .unwrap();
        let mut expected = Vec::from([4u8, 1, 0, 80, 0, 0, 0, 1]);
        expected.extend_from_slice(b"ada\0h.example\0");
        assert_eq!(stream.written, expected);
    }

    #[test]
    fn socks4_cannot_name_ipv6_and_reports_a_refusal() {
        let mut stream = Canned::new(&[]);
        let error = connect(&mut stream, ProxyScheme::Socks4, None, "::1", 80).unwrap_err();
        assert!(matches!(error, Error::Unsupported(_)), "{error}");
        let mut stream = Canned::new(&[0, 0x5b, 0, 0, 0, 0, 0, 0]);
        let error = connect(&mut stream, ProxyScheme::Socks4, None, "h", 80).unwrap_err();
        assert!(matches!(error, Error::Refused(_)), "{error}");
    }
}

#[cfg(test)]
mod race_tests {
    use alloc::string::{String, ToString};

    use super::*;

    fn address(text: &str) -> SocketAddr {
        text.parse().unwrap()
    }

    #[test]
    fn the_families_alternate_starting_with_the_first() {
        let ordered = interleaved(alloc::vec![
            address("[::1]:1"),
            address("[::2]:1"),
            address("127.0.0.1:1"),
            address("127.0.0.2:1"),
            address("[::3]:1"),
        ]);
        let texts: Vec<String> = ordered.iter().map(ToString::to_string).collect();
        assert_eq!(
            texts,
            [
                "[::1]:1",
                "127.0.0.1:1",
                "[::2]:1",
                "127.0.0.2:1",
                "[::3]:1"
            ]
        );
        assert!(interleaved(Vec::new()).is_empty());
    }

    #[test]
    fn a_refused_address_gives_way_to_the_next_at_once() {
        let dead = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap()
        };
        let live = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let started = std::time::Instant::now();
        let stream = race(
            alloc::vec![dead, live.local_addr().unwrap()],
            Duration::from_secs(5),
            Duration::from_secs(30),
        )
        .unwrap();
        assert_eq!(stream.peer_addr().unwrap(), live.local_addr().unwrap());
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(
            race(
                alloc::vec![dead],
                Duration::from_secs(5),
                Duration::from_secs(1)
            )
            .is_err()
        );
        assert!(race(Vec::new(), Duration::from_secs(1), Duration::from_secs(1)).is_err());
    }
}
