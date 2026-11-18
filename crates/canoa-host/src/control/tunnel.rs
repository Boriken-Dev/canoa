//! `network/tunnel`: the connection the app makes to a remote Canoa host, opened by this host so a WebSocket the app opens follows
//! the system as the host's own requests do: the proxy this host's network chooses for it with its login, and, for `https`, the TLS
//! to the server made here with the operating system's trust and the client certificates of its keystores. The app's own library
//! has no PAC, no WPAD, no Negotiate or NTLM login, only its own roots and none of the keystores.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "sockets, threads and clocks are std's"
)]

use std::fmt::Write as _;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use canoa::network::{Connection, Network, SecureConnection, ServerIdentity};
use serde_json::{Value, json};

use super::{REFUSED, bad, text_param};

type Outcome = Result<Value, (i32, String)>;

/// How long the app has to open the connection it was told about unless it says otherwise.
const DEFAULT_SECONDS: u64 = 30;
/// The longest it may ask for.
const MAX_SECONDS: u64 = 300;
/// How often a tunnel that cannot be split (TLS) looks at both of its ends: such a stream cannot be read and written from two
/// threads, so one thread waits this long on each end in turn.
const POLL: Duration = Duration::from_millis(5);
/// How long a connection to the loopback port has to say the token it was given, before the next one is looked at.
const PREAMBLE: Duration = Duration::from_secs(3);
/// Random bytes in a token (written as twice as many hex digits).
const TOKEN_BYTES: usize = 16;

/// The connections the app sends through this host's network.
#[derive(Clone, Default)]
pub(crate) struct Tunnel {
    /// The network they go through; the process's when `None`.
    network: Option<Arc<Network>>,
}

impl core::fmt::Debug for Tunnel {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Tunnel").finish_non_exhaustive()
    }
}

impl Tunnel {
    /// Tunnels through `network` instead of the process's.
    #[cfg(test)]
    pub(crate) fn on(network: Arc<Network>) -> Self {
        Self {
            network: Some(network),
        }
    }

    /// `network/tunnel` `{"url", "seconds"?}` -> `{"direct": true}` or `{"port", "token", "tls"?}`. The URL is the `http` or
    /// `https` one the app is about to connect to; only its host and port matter, and its scheme decides who makes the TLS.
    ///
    /// `http`: `direct` when no proxy applies, so the app connects itself; otherwise this host has connected through the proxy
    /// (and logged in) and the bytes it carries are the app's.
    ///
    /// `https`: this host has connected (through the proxy, or straight to the server when none applies), made the TLS with its
    /// network's trust and client certificates and verified the server for the URL's host, so a failure is this call's error and
    /// `tls` says who the server proved to be (`name`, `subject`, `sha256` of its certificate); the bytes it carries are what
    /// the app would send inside the TLS.
    ///
    /// Either way this host then listens on the loopback `port` for the one connection the app makes within `seconds` (1 to
    /// 300, 30 unless it says), which must begin with the `token` as the app got it and then carries the bytes both ways until
    /// either end closes.
    pub(crate) fn run(&self, params: &Value) -> Outcome {
        let url = text_param(params, "url")?.ok_or_else(|| bad("url is needed"))?;
        let scheme = url.split_once("://").map(|(scheme, _)| scheme);
        if !matches!(scheme, Some(s) if s.eq_ignore_ascii_case("http") || s.eq_ignore_ascii_case("https"))
        {
            return Err(bad("url is http or https"));
        }
        let secure = scheme.is_some_and(|scheme| scheme.eq_ignore_ascii_case("https"));
        let seconds = match params.get("seconds") {
            None | Some(Value::Null) => DEFAULT_SECONDS,
            Some(value) => value
                .as_u64()
                .filter(|seconds| (1..=MAX_SECONDS).contains(seconds))
                .ok_or_else(|| bad("seconds is 1 to 300"))?,
        };
        let network = self.network.clone().unwrap_or_else(Network::current);
        let refused = |error: canoa::network::NetworkError| {
            (
                REFUSED,
                format!("The host cannot open the connection: {error}"),
            )
        };
        let (upstream, server) = if secure {
            let connection = network.secure_tunnel(url).map_err(refused)?;
            let server = connection.server().clone();
            (Upstream::Secure(connection), Some(server))
        } else {
            match network.tunnel(url).map_err(refused)? {
                None => return Ok(json!({"direct": true})),
                Some(connection) => (Upstream::Plain(connection), None),
            }
        };
        let token = new_token().map_err(|error| {
            (
                REFUSED,
                format!("The host cannot make the connection's token: {error}"),
            )
        })?;
        let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|error| {
            (
                REFUSED,
                format!("The host cannot listen for the connection: {error}"),
            )
        })?;
        let port = listener
            .local_addr()
            .map_err(|error| (REFUSED, format!("The host cannot listen: {error}")))?
            .port();
        let waiting = token.clone();
        thread::Builder::new()
            .name("network-tunnel".to_owned())
            .spawn(move || {
                serve(
                    &listener,
                    upstream,
                    Duration::from_secs(seconds),
                    waiting.as_bytes(),
                );
            })
            .map_err(|error| (REFUSED, format!("The host cannot carry it: {error}")))?;
        let mut reply = json!({"port": port, "token": token});
        if let Some(ServerIdentity {
            name,
            subject,
            sha256,
            ..
        }) = server
        {
            reply["tls"] = json!({"name": name, "subject": subject, "sha256": sha256});
        }
        Ok(reply)
    }
}

/// A random token as hex: what the app says first on the loopback connection, so that no other process there is carried in its
/// place.
fn new_token() -> Result<String, getrandom::Error> {
    let mut bytes = [0u8; TOKEN_BYTES];
    getrandom::fill(&mut bytes)?;
    let mut text = String::with_capacity(TOKEN_BYTES * 2);
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    Ok(text)
}

/// What the app's connection is carried to: the proxy's tunnel, or a server with the TLS made.
enum Upstream {
    Plain(Connection),
    Secure(SecureConnection),
}

impl Upstream {
    fn socket(&self) -> &TcpStream {
        match self {
            Self::Plain(connection) => connection.socket(),
            Self::Secure(connection) => connection.socket(),
        }
    }

    /// Ends it: a TLS connection says so to the server first.
    fn end(&mut self) {
        if let Self::Secure(connection) = self {
            connection.close_notify();
        }
        let _ = self.socket().shutdown(Shutdown::Both);
    }
}

impl Read for Upstream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Plain(connection) => connection.read(buffer),
            Self::Secure(connection) => connection.read(buffer),
        }
    }
}

impl Write for Upstream {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        match self {
            Self::Plain(connection) => connection.write(buffer),
            Self::Secure(connection) => connection.write(buffer),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Plain(connection) => connection.flush(),
            Self::Secure(connection) => connection.flush(),
        }
    }
}

/// Waits `wait` for the one connection the app makes to `listener` (the one that begins with `token`), then carries it to
/// `upstream` until either end closes. Nobody connecting in time closes `upstream`.
fn serve(listener: &TcpListener, mut upstream: Upstream, wait: Duration, token: &[u8]) {
    let Some(client) = accept(listener, wait, token) else {
        upstream.end();
        return;
    };
    // Whatever the proxy connection was given for its own setup, a tunnel idles as long as the two ends like.
    let socket = upstream.socket();
    let _ = socket.set_read_timeout(None);
    let _ = socket.set_write_timeout(None);
    let _ = client.set_nodelay(true);
    match upstream {
        Upstream::Plain(Connection::Tcp(stream)) => pump_tcp(client, stream),
        other => pump_polling(&client, other),
    }
}

/// The first connection to `listener` within `wait` that begins with `token`. The host's own app is the only one that has it, so
/// another process on this computer that connects first is dropped, not carried (with a server's TLS made here, it would
/// otherwise speak as the person to the server, and read what the app is sent).
fn accept(listener: &TcpListener, wait: Duration, token: &[u8]) -> Option<TcpStream> {
    listener.set_nonblocking(true).ok()?;
    let deadline = Instant::now() + wait;
    loop {
        match listener.accept() {
            Ok((mut client, _)) => {
                if client.set_nonblocking(false).is_ok() && opens_with(&mut client, token) {
                    return Some(client);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return None;
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(_) => return None,
        }
    }
}

/// Whether `client` says `token` first, within [`PREAMBLE`]. What it sends after is left unread.
fn opens_with(client: &mut TcpStream, token: &[u8]) -> bool {
    if client.set_read_timeout(Some(PREAMBLE)).is_err() {
        return false;
    }
    let mut said = vec![0u8; token.len()];
    if client.read_exact(&mut said).is_err() {
        return false;
    }
    // Every byte is looked at whatever the first difference, so how long this takes says nothing of how much matched.
    let different = said
        .iter()
        .zip(token)
        .fold(0u8, |different, (said, wanted)| different | (said ^ wanted));
    different == 0 && client.set_read_timeout(None).is_ok()
}

/// Both directions at once, each on its own thread; a half-close passes on, an error ends both.
fn pump_tcp(client: TcpStream, upstream: TcpStream) {
    let (Ok(mut client_read), Ok(mut upstream_write)) = (client.try_clone(), upstream.try_clone())
    else {
        return;
    };
    let forward = thread::spawn(move || {
        if io::copy(&mut client_read, &mut upstream_write).is_err() {
            let _ = upstream_write.shutdown(Shutdown::Both);
        } else {
            let _ = upstream_write.shutdown(Shutdown::Write);
        }
    });
    let (mut upstream_read, mut client_write) = (upstream, client);
    if io::copy(&mut upstream_read, &mut client_write).is_err() {
        let _ = client_write.shutdown(Shutdown::Both);
    } else {
        let _ = client_write.shutdown(Shutdown::Write);
    }
    let _ = forward.join();
}

/// Both directions on one thread, for a connection that cannot be split: each end waits [`POLL`] for something to read.
fn pump_polling(client: &TcpStream, mut upstream: Upstream) {
    let (mut client_read, mut client_write) = (client, client);
    let _ = upstream.socket().set_read_timeout(Some(POLL));
    let _ = client.set_read_timeout(Some(POLL));
    let mut buffer = vec![0u8; 16 * 1024];
    loop {
        match upstream.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                if client_write.write_all(&buffer[..count]).is_err() {
                    break;
                }
            }
            Err(error) if is_idle(&error) => {}
            Err(_) => break,
        }
        match client_read.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                if upstream
                    .write_all(&buffer[..count])
                    .and_then(|()| upstream.flush())
                    .is_err()
                {
                    break;
                }
            }
            Err(error) if is_idle(&error) => {}
            Err(_) => break,
        }
    }
    let _ = client.shutdown(Shutdown::Both);
    upstream.end();
}

/// A read that found nothing yet, which Windows reports as a time-out and Unix as would-block.
fn is_idle(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &[u8] = b"0123456789abcdef0123456789abcdef";

    /// A TCP echo server: what it is sent comes back, and it ends when the sender closes.
    fn echo() -> (std::net::SocketAddr, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buffer = [0u8; 1024];
            while let Ok(count) = stream.read(&mut buffer) {
                if count == 0 || stream.write_all(&buffer[..count]).is_err() {
                    break;
                }
            }
        });
        (addr, handle)
    }

    #[test]
    fn a_connection_with_no_one_to_use_it_is_closed_when_the_wait_ends() {
        let (addr, origin) = echo();
        let upstream = TcpStream::connect(addr).expect("connects");
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let started = Instant::now();
        serve(
            &listener,
            Upstream::Plain(Connection::Tcp(upstream)),
            Duration::from_millis(100),
            TOKEN,
        );
        assert!(started.elapsed() < Duration::from_secs(5));
        // The echo server's read saw the close, so it ended: a connection left open would hold it for ever.
        origin.join().expect("the origin saw the close");
    }

    #[test]
    fn a_polled_tunnel_carries_both_ways_and_ends_when_either_side_closes() {
        let (addr, origin) = echo();
        let upstream =
            Upstream::Plain(Connection::Tcp(TcpStream::connect(addr).expect("connects")));
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let tunnel = thread::spawn(move || {
            let (client, _) = listener.accept().expect("accept");
            pump_polling(&client, upstream);
        });
        let mut app = TcpStream::connect(("127.0.0.1", port)).expect("connects");
        app.set_read_timeout(Some(Duration::from_secs(10)))
            .expect("timeout");
        app.write_all(b"ping").expect("writes");
        let mut echoed = [0u8; 4];
        app.read_exact(&mut echoed).expect("reads");
        assert_eq!(&echoed, b"ping");
        // An idle spell, then more: nothing is lost to the polling.
        thread::sleep(Duration::from_millis(60));
        app.write_all(b"again").expect("writes");
        let mut echoed = [0u8; 5];
        app.read_exact(&mut echoed).expect("reads");
        assert_eq!(&echoed, b"again");
        drop(app);
        tunnel.join().expect("the tunnel ended");
        origin.join().expect("the origin saw the close");
    }

    #[test]
    fn only_the_connection_that_says_the_token_is_carried() {
        let (addr, origin) = echo();
        let upstream = TcpStream::connect(addr).expect("connects");
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let tunnel = thread::spawn(move || {
            serve(
                &listener,
                Upstream::Plain(Connection::Tcp(upstream)),
                Duration::from_secs(20),
                TOKEN,
            );
        });
        // Another process on this computer gets there first with the wrong words, then with none.
        let mut wrong = TcpStream::connect(("127.0.0.1", port)).expect("connects");
        wrong.write_all(&[b'x'; 32]).expect("writes");
        wrong
            .set_read_timeout(Some(Duration::from_secs(10)))
            .expect("timeout");
        let mut nothing = [0u8; 1];
        assert!(
            matches!(wrong.read(&mut nothing), Ok(0) | Err(_)),
            "the wrong token was carried"
        );
        // The app, a moment later, is carried; what follows its token in the same write is not lost.
        let mut app = TcpStream::connect(("127.0.0.1", port)).expect("connects");
        app.set_read_timeout(Some(Duration::from_secs(10)))
            .expect("timeout");
        let mut hello = TOKEN.to_vec();
        hello.extend_from_slice(b"hello");
        app.write_all(&hello).expect("writes");
        let mut echoed = [0u8; 5];
        app.read_exact(&mut echoed).expect("reads");
        assert_eq!(&echoed, b"hello");
        drop(app);
        tunnel.join().expect("the tunnel ended");
        origin.join().expect("the origin saw the close");
    }
}
