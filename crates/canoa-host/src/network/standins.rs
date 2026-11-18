//! Loopback stand-ins for the network tests: a proxy that wants a login, a TLS server that may ask for a client certificate, and the
//! test CA that signs for both. Nothing leaves this computer, and no key is kept: each test makes its own.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a test names what it uses from std"
)]

use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rcgen::{
    BasicConstraints, CertificateParams, CertifiedIssuer, DnType, ExtendedKeyUsagePurpose, IsCa,
    KeyPair, KeyUsagePurpose,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::server::WebPkiClientVerifier;
use rustls::{RootCertStore, ServerConfig, ServerConnection, StreamOwned};

/// A certificate chain (leaf first) and its key.
#[derive(Clone)]
pub(crate) struct Leaf {
    pub(crate) chain: Vec<CertificateDer<'static>>,
    pub(crate) chain_pem: String,
    pub(crate) key_pem: String,
    key_pkcs8: Vec<u8>,
}

impl Leaf {
    fn key(&self) -> PrivateKeyDer<'static> {
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.key_pkcs8.clone()))
    }
}

/// A test certificate authority.
pub(crate) struct Ca {
    issuer: CertifiedIssuer<'static, KeyPair>,
}

impl Ca {
    pub(crate) fn new(common_name: &str) -> Self {
        let mut params = CertificateParams::new(Vec::<String>::new()).expect("params");
        params
            .distinguished_name
            .push(DnType::CommonName, common_name);
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let issuer =
            CertifiedIssuer::self_signed(params, KeyPair::generate().expect("key")).expect("ca");
        Self { issuer }
    }

    pub(crate) fn der(&self) -> CertificateDer<'static> {
        self.issuer.as_ref().der().clone()
    }

    pub(crate) fn pem(&self) -> String {
        self.issuer.as_ref().pem()
    }

    /// A server certificate valid for each of `names`.
    pub(crate) fn issue_server(&self, names: &[&str]) -> Leaf {
        let mut params =
            CertificateParams::new(names.iter().map(ToString::to_string).collect::<Vec<_>>())
                .expect("params");
        params.distinguished_name.push(DnType::CommonName, names[0]);
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        self.sign(params)
    }

    /// A client certificate named `common_name`.
    pub(crate) fn issue_client(&self, common_name: &str) -> Leaf {
        let mut params = CertificateParams::new(Vec::<String>::new()).expect("params");
        params
            .distinguished_name
            .push(DnType::CommonName, common_name);
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        self.sign(params)
    }

    fn sign(&self, params: CertificateParams) -> Leaf {
        let key = KeyPair::generate().expect("key");
        let certificate = params.signed_by(&key, &self.issuer).expect("signs");
        Leaf {
            chain: vec![certificate.der().clone(), self.der()],
            chain_pem: format!("{}{}", certificate.pem(), self.pem()),
            key_pem: key.serialize_pem(),
            key_pkcs8: key.serialize_der(),
        }
    }
}

/// `Basic` and the base64 of `user:password`.
fn basic(user: &str, password: &str) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = format!("{user}:{password}").into_bytes();
    let mut text = String::from("Basic ");
    for chunk in bytes.chunks(3) {
        let group = u32::from(chunk[0]) << 16
            | u32::from(*chunk.get(1).unwrap_or(&0)) << 8
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for (index, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            if index <= chunk.len() {
                text.push(char::from(ALPHABET[(group >> shift) as usize & 63]));
            } else {
                text.push('=');
            }
        }
    }
    text
}

/// What a request to the stand-in proxy carried.
#[derive(Debug, Clone)]
pub(crate) struct Seen {
    pub(crate) request_line: String,
    pub(crate) authorization: Option<String>,
}

/// A loopback HTTP proxy that wants `Proxy-Authorization: Basic user:password`. It tunnels each `CONNECT` it allows to `origin`
/// whatever host the request names, or answers 403 when it has no origin.
pub(crate) struct StandInProxy {
    pub(crate) addr: SocketAddr,
    seen: Arc<Mutex<Vec<Seen>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl StandInProxy {
    pub(crate) fn start(user: &str, password: &str, origin: Option<SocketAddr>) -> Self {
        let wanted = basic(user, password);
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (seen_in, stop_in) = (Arc::clone(&seen), Arc::clone(&stop));
        let thread = thread::spawn(move || {
            for stream in listener.incoming() {
                if stop_in.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(stream) = stream else { continue };
                let (seen, wanted) = (Arc::clone(&seen_in), wanted.clone());
                thread::spawn(move || serve(stream, &wanted, origin, &seen));
            }
        });
        Self {
            addr,
            seen,
            stop,
            thread: Some(thread),
        }
    }

    pub(crate) fn seen(&self) -> Vec<Seen> {
        self.seen.lock().expect("lock").clone()
    }

    /// The `CONNECT` targets the proxy let through.
    pub(crate) fn allowed(&self, user: &str, password: &str) -> Vec<String> {
        let wanted = basic(user, password);
        self.seen()
            .into_iter()
            .filter(|seen| seen.authorization.as_deref() == Some(wanted.as_str()))
            .filter_map(|seen| {
                seen.request_line
                    .strip_prefix("CONNECT ")
                    .and_then(|rest| rest.split_whitespace().next())
                    .map(str::to_owned)
            })
            .collect()
    }
}

impl Drop for StandInProxy {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn read_head(stream: &mut TcpStream) -> Option<Seen> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte).ok()? == 0 {
            return None;
        }
        head.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&head).into_owned();
    let mut lines = text.split("\r\n");
    let request_line = lines.next()?.to_owned();
    let authorization = lines
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.trim().eq_ignore_ascii_case("proxy-authorization"))
        .map(|(_, value)| value.trim().to_owned());
    Some(Seen {
        request_line,
        authorization,
    })
}

fn serve(mut client: TcpStream, wanted: &str, origin: Option<SocketAddr>, log: &Mutex<Vec<Seen>>) {
    client.set_read_timeout(Some(Duration::from_secs(10))).ok();
    loop {
        let Some(seen) = read_head(&mut client) else {
            return;
        };
        log.lock().expect("lock").push(seen.clone());
        if seen.authorization.as_deref() != Some(wanted) {
            let reply = "HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"stand-in\"\r\nContent-Length: 0\r\n\r\n";
            if client.write_all(reply.as_bytes()).is_err() {
                return;
            }
            continue;
        }
        let Some(origin) = origin else {
            let _ = client.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n");
            return;
        };
        let Ok(upstream) = TcpStream::connect(origin) else {
            let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n");
            return;
        };
        if client
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .is_err()
        {
            return;
        }
        pipe(client, upstream);
        return;
    }
}

fn pipe(client: TcpStream, upstream: TcpStream) {
    let (mut client_read, mut upstream_write) = (
        client.try_clone().expect("clone"),
        upstream.try_clone().expect("clone"),
    );
    let forward = thread::spawn(move || {
        let _ = std::io::copy(&mut client_read, &mut upstream_write);
        let _ = upstream_write.shutdown(Shutdown::Write);
    });
    let (mut upstream_read, mut client_write) = (upstream, client);
    let _ = std::io::copy(&mut upstream_read, &mut client_write);
    let _ = client_write.shutdown(Shutdown::Write);
    let _ = forward.join();
}

/// What a TLS server does about client certificates.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClientAuth {
    /// Never asks.
    Off,
    /// Asks, and refuses a client with none.
    Required,
}

/// What the server answers: its body for a path, given whether the client sent a certificate.
pub(crate) type Handler = Arc<dyn Fn(&str, bool) -> Vec<u8> + Send + Sync>;

/// A loopback HTTPS server.
pub(crate) struct TlsServer {
    pub(crate) addr: SocketAddr,
    /// The leaf certificate each connection presented, if any.
    presented: Arc<Mutex<Vec<Option<Vec<u8>>>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl TlsServer {
    pub(crate) fn start(
        server: &Leaf,
        client_roots: &[CertificateDer<'static>],
        auth: ClientAuth,
        handler: Handler,
    ) -> Self {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let builder = ServerConfig::builder_with_provider(Arc::clone(&provider))
            .with_safe_default_protocol_versions()
            .expect("versions");
        let builder = match auth {
            ClientAuth::Off => builder.with_no_client_auth(),
            ClientAuth::Required => {
                let mut roots = RootCertStore::empty();
                for root in client_roots {
                    roots.add(root.clone()).expect("root");
                }
                let verifier =
                    WebPkiClientVerifier::builder_with_provider(Arc::new(roots), provider)
                        .build()
                        .expect("verifier");
                builder.with_client_cert_verifier(verifier)
            }
        };
        let config = Arc::new(
            builder
                .with_single_cert(server.chain.clone(), server.key())
                .expect("certificate"),
        );
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let presented = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (presented_in, stop_in) = (Arc::clone(&presented), Arc::clone(&stop));
        let thread = thread::spawn(move || {
            for stream in listener.incoming() {
                if stop_in.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(stream) = stream else { continue };
                let (config, presented, handler) = (
                    Arc::clone(&config),
                    Arc::clone(&presented_in),
                    Arc::clone(&handler),
                );
                thread::spawn(move || answer(stream, config, &presented, &handler));
            }
        });
        Self {
            addr,
            presented,
            stop,
            thread: Some(thread),
        }
    }

    pub(crate) fn presented(&self) -> Vec<Option<Vec<u8>>> {
        self.presented.lock().expect("lock").clone()
    }
}

impl Drop for TlsServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn answer(
    stream: TcpStream,
    config: Arc<ServerConfig>,
    presented: &Mutex<Vec<Option<Vec<u8>>>>,
    handler: &Handler,
) {
    stream.set_read_timeout(Some(Duration::from_secs(10))).ok();
    let Ok(connection) = ServerConnection::new(config) else {
        return;
    };
    let mut tls = StreamOwned::new(connection, stream);
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        match tls.read(&mut byte) {
            Ok(1) => head.push(byte[0]),
            _ => return,
        }
    }
    let text = String::from_utf8_lossy(&head).into_owned();
    let path = text.split_whitespace().nth(1).unwrap_or("/").to_owned();
    let certificate = tls
        .conn
        .peer_certificates()
        .and_then(|chain| chain.first())
        .map(|der| der.as_ref().to_vec());
    let body = handler(&path, certificate.is_some());
    presented.lock().expect("lock").push(certificate);
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = tls.write_all(head.as_bytes());
    let _ = tls.write_all(&body);
    tls.conn.send_close_notify();
    let _ = tls.flush();
}

/// A loopback SOCKS5 server that wants no login. It records the `host:port` each client asks for, and carries each connection it
/// accepts to `origin` whatever it was asked for, as [`StandInProxy`] does.
pub(crate) struct StandInSocks {
    pub(crate) addr: SocketAddr,
    asked: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl StandInSocks {
    pub(crate) fn start(origin: SocketAddr) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let asked = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (asked_in, stop_in) = (Arc::clone(&asked), Arc::clone(&stop));
        let thread = thread::spawn(move || {
            for stream in listener.incoming() {
                if stop_in.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(stream) = stream else { continue };
                let asked = Arc::clone(&asked_in);
                thread::spawn(move || socks(stream, origin, &asked));
            }
        });
        Self {
            addr,
            asked,
            stop,
            thread: Some(thread),
        }
    }

    /// The `host:port` each client asked to be connected to.
    pub(crate) fn asked(&self) -> Vec<String> {
        self.asked.lock().expect("lock").clone()
    }
}

impl Drop for StandInSocks {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn socks(mut client: TcpStream, origin: SocketAddr, asked: &Mutex<Vec<String>>) {
    client.set_read_timeout(Some(Duration::from_secs(10))).ok();
    let mut greeting = [0u8; 2];
    if client.read_exact(&mut greeting).is_err() || greeting[0] != 5 {
        return;
    }
    let mut methods = vec![0u8; usize::from(greeting[1])];
    if client.read_exact(&mut methods).is_err() || !methods.contains(&0) {
        let _ = client.write_all(&[5, 0xff]);
        return;
    }
    if client.write_all(&[5, 0]).is_err() {
        return;
    }
    let mut head = [0u8; 4];
    if client.read_exact(&mut head).is_err() || head[0] != 5 || head[1] != 1 {
        return;
    }
    let host = match head[3] {
        1 => {
            let mut octets = [0u8; 4];
            if client.read_exact(&mut octets).is_err() {
                return;
            }
            std::net::Ipv4Addr::from(octets).to_string()
        }
        3 => {
            let mut length = [0u8; 1];
            if client.read_exact(&mut length).is_err() {
                return;
            }
            let mut name = vec![0u8; usize::from(length[0])];
            if client.read_exact(&mut name).is_err() {
                return;
            }
            String::from_utf8_lossy(&name).into_owned()
        }
        4 => {
            let mut octets = [0u8; 16];
            if client.read_exact(&mut octets).is_err() {
                return;
            }
            std::net::Ipv6Addr::from(octets).to_string()
        }
        _ => return,
    };
    let mut port = [0u8; 2];
    if client.read_exact(&mut port).is_err() {
        return;
    }
    asked
        .lock()
        .expect("lock")
        .push(format!("{host}:{}", u16::from_be_bytes(port)));
    let Ok(upstream) = TcpStream::connect(origin) else {
        let _ = client.write_all(&[5, 5, 0, 1, 0, 0, 0, 0, 0, 0]);
        return;
    };
    if client.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).is_err() {
        return;
    }
    pipe(client, upstream);
}
