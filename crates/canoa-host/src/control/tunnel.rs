//! `network/tunnel`: the connection the app makes to a remote Canoa host, carried through the proxy this host's network chooses for it,
//! with its login, so a WebSocket the app opens follows the system's proxy as the host's own requests do. The app's own library has
//! no PAC, no WPAD and no Negotiate or NTLM login.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "sockets, threads and clocks are std's"
)]

use std::io::{self, Read as _, Write as _};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use canoa::network::{Connection, Network};
use serde_json::{Value, json};

use super::{REFUSED, bad, text_param};

type Outcome = Result<Value, (i32, String)>;

/// How long the app has to open the connection it was told about unless it says otherwise.
const DEFAULT_SECONDS: u64 = 30;
/// The longest it may ask for.
const MAX_SECONDS: u64 = 300;
/// How often a tunnel through a proxy reached over TLS looks at both of its ends: such a stream cannot be read and written from two
/// threads, so one thread waits this long on each end in turn.
const POLL: Duration = Duration::from_millis(5);

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

    /// `network/tunnel` `{"url", "seconds"?}` -> `{"direct": true}` or `{"port"}`. The URL is the `http` or `https` one the app is
    /// about to connect to; only its host and port matter. `direct`: no proxy applies, so the app connects itself. `port`: this host
    /// has already connected through the proxy (and logged in) and listens on that loopback port for the one connection the app
    /// makes, within `seconds` (1 to 300, 30 unless it says), then carries its bytes both ways untouched: the app makes any TLS to
    /// the server itself, over the tunnel.
    pub(crate) fn run(&self, params: &Value) -> Outcome {
        let url = text_param(params, "url")?.ok_or_else(|| bad("url is needed"))?;
        let scheme = url.split_once("://").map(|(scheme, _)| scheme);
        if !matches!(scheme, Some(s) if s.eq_ignore_ascii_case("http") || s.eq_ignore_ascii_case("https"))
        {
            return Err(bad("url is http or https"));
        }
        let seconds = match params.get("seconds") {
            None | Some(Value::Null) => DEFAULT_SECONDS,
            Some(value) => value
                .as_u64()
                .filter(|seconds| (1..=MAX_SECONDS).contains(seconds))
                .ok_or_else(|| bad("seconds is 1 to 300"))?,
        };
        let network = self.network.clone().unwrap_or_else(Network::current);
        let connection = match network.tunnel(url) {
            Ok(None) => return Ok(json!({"direct": true})),
            Ok(Some(connection)) => connection,
            Err(error) => {
                return Err((
                    REFUSED,
                    format!("The proxy cannot carry the connection: {error}"),
                ));
            }
        };
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
        thread::Builder::new()
            .name("network-tunnel".to_owned())
            .spawn(move || serve(&listener, connection, Duration::from_secs(seconds)))
            .map_err(|error| (REFUSED, format!("The host cannot carry it: {error}")))?;
        Ok(json!({"port": port}))
    }
}

/// Waits `wait` for the one connection the app makes to `listener`, then carries it to `upstream` until either end closes. Nobody
/// connecting in time closes `upstream`.
fn serve(listener: &TcpListener, upstream: Connection, wait: Duration) {
    let Some(client) = accept(listener, wait) else {
        let _ = upstream.socket().shutdown(Shutdown::Both);
        return;
    };
    // Whatever the proxy connection was given for its own setup, a tunnel idles as long as the two ends like.
    let socket = upstream.socket();
    let _ = socket.set_read_timeout(None);
    let _ = socket.set_write_timeout(None);
    let _ = client.set_nodelay(true);
    match upstream {
        Connection::Tcp(stream) => pump_tcp(client, stream),
        other => pump_polling(&client, other),
    }
}

/// The first connection to `listener` within `wait`.
fn accept(listener: &TcpListener, wait: Duration) -> Option<TcpStream> {
    listener.set_nonblocking(true).ok()?;
    let deadline = Instant::now() + wait;
    loop {
        match listener.accept() {
            Ok((client, _)) => {
                client.set_nonblocking(false).ok()?;
                return Some(client);
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
fn pump_polling(client: &TcpStream, mut upstream: Connection) {
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
    let _ = upstream.socket().shutdown(Shutdown::Both);
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
            Connection::Tcp(upstream),
            Duration::from_millis(100),
        );
        assert!(started.elapsed() < Duration::from_secs(5));
        // The echo server's read saw the close, so it ended: a connection left open would hold it for ever.
        origin.join().expect("the origin saw the close");
    }

    #[test]
    fn a_polled_tunnel_carries_both_ways_and_ends_when_either_side_closes() {
        let (addr, origin) = echo();
        let upstream = Connection::Tcp(TcpStream::connect(addr).expect("connects"));
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
}
