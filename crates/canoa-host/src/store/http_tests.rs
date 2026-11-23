//! What the host's lookups cost a publisher: one client, so connections are reused; a document a publisher gave an `ETag` for asked
//! again with `If-None-Match`, so an unchanged one costs no body; and a lookup that waits less than a download. The publisher is a
//! stand-in on a loopback address that counts what it is asked and what it sends.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a test names what it uses from std"
)]

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use canoa::network::Network;
use canoa::store::InstallTools;
use coqui_syshttp::tls::TlsBuilder;

use super::tools::{HttpTransport, LocalTools, ToolsLimits, Transport};
use crate::network::tests::direct;

/// What the stand-in answers one request with.
pub(crate) struct Answer {
    pub(crate) status: u16,
    pub(crate) etag: Option<String>,
    pub(crate) body: Vec<u8>,
    pub(crate) delay: Duration,
}

/// What a request said.
pub(crate) struct Asked {
    pub(crate) path: String,
    pub(crate) if_none_match: Option<String>,
}

/// A publisher on a loopback address that counts its connections, requests and the bytes of body it sends.
pub(crate) struct Origin {
    pub(crate) url: String,
    pub(crate) connections: Arc<AtomicUsize>,
    pub(crate) requests: Arc<AtomicUsize>,
    pub(crate) bytes: Arc<AtomicUsize>,
    pub(crate) seen: Arc<Mutex<Vec<Asked>>>,
}

impl Origin {
    pub(crate) fn start(handler: impl Fn(&Asked) -> Answer + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
        let url = format!(
            "http://127.0.0.1:{}",
            listener.local_addr().expect("addr").port()
        );
        let origin = Self {
            url,
            connections: Arc::default(),
            requests: Arc::default(),
            bytes: Arc::default(),
            seen: Arc::default(),
        };
        let handler = Arc::new(handler);
        let (connections, requests, bytes, seen) = (
            Arc::clone(&origin.connections),
            Arc::clone(&origin.requests),
            Arc::clone(&origin.bytes),
            Arc::clone(&origin.seen),
        );
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                connections.fetch_add(1, Ordering::SeqCst);
                let (handler, requests, bytes, seen) = (
                    Arc::clone(&handler),
                    Arc::clone(&requests),
                    Arc::clone(&bytes),
                    Arc::clone(&seen),
                );
                std::thread::spawn(move || {
                    let _ = connection(stream, &*handler, &requests, &bytes, &seen);
                });
            }
        });
        origin
    }

    pub(crate) fn asked(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }

    pub(crate) fn sent(&self) -> usize {
        self.bytes.load(Ordering::SeqCst)
    }

    pub(crate) fn opened(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }
}

fn connection(
    stream: TcpStream,
    handler: &(dyn Fn(&Asked) -> Answer + Send + Sync),
    requests: &AtomicUsize,
    bytes: &AtomicUsize,
    seen: &Mutex<Vec<Asked>>,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(());
        }
        let path = line.split_whitespace().nth(1).unwrap_or("/").to_owned();
        let mut if_none_match = None;
        loop {
            let mut header = String::new();
            reader.read_line(&mut header)?;
            if header.trim().is_empty() {
                break;
            }
            if let Some((name, value)) = header.split_once(':')
                && name.eq_ignore_ascii_case("if-none-match")
            {
                if_none_match = Some(value.trim().to_owned());
            }
        }
        let asked = Asked {
            path,
            if_none_match,
        };
        requests.fetch_add(1, Ordering::SeqCst);
        let answer = handler(&asked);
        seen.lock().expect("lock").push(asked);
        std::thread::sleep(answer.delay);
        let mut head = format!(
            "HTTP/1.1 {} X\r\nContent-Length: {}\r\n",
            answer.status,
            answer.body.len()
        );
        if let Some(etag) = &answer.etag {
            head.push_str(&format!("ETag: {etag}\r\n"));
        }
        head.push_str("\r\n");
        writer.write_all(head.as_bytes())?;
        writer.write_all(&answer.body)?;
        bytes.fetch_add(answer.body.len(), Ordering::SeqCst);
    }
}

fn plain() -> Arc<Network> {
    Arc::new(Network::with(direct(), TlsBuilder::default()))
}

fn ok(etag: Option<&str>, body: &[u8]) -> Answer {
    Answer {
        status: 200,
        etag: etag.map(str::to_owned),
        body: body.to_vec(),
        delay: Duration::ZERO,
    }
}

fn lookup(transport: &HttpTransport, url: &str) -> String {
    let limits = ToolsLimits {
        lookup: true,
        ..ToolsLimits::default()
    };
    let mut text = String::new();
    transport
        .get(url, &limits)
        .expect("answered")
        .body
        .read_to_string(&mut text)
        .expect("read");
    text
}

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("canoa-http-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn one_client_serves_every_request_so_a_publisher_sees_one_connection() {
    let origin = Origin::start(|_| ok(None, b"{}"));
    let network = plain();
    let shared = HttpTransport::on(Arc::clone(&network));
    for path in ["a", "b", "a"] {
        lookup(&shared, &format!("{}/{path}", origin.url));
    }
    assert_eq!((origin.asked(), origin.opened()), (3, 1));

    // A client made for each request, as every request made one before, opens a connection each time.
    let wasteful = Origin::start(|_| ok(None, b"{}"));
    for path in ["a", "b", "a"] {
        lookup(
            &HttpTransport::on(Arc::clone(&network)),
            &format!("{}/{path}", wasteful.url),
        );
    }
    assert_eq!((wasteful.asked(), wasteful.opened()), (3, 3));
}

#[test]
fn a_document_the_publisher_gave_an_etag_for_costs_no_body_again_and_a_changed_one_is_read() {
    let body = vec![b'x'; 4000];
    let served = Arc::new(Mutex::new(("\"v1\"".to_owned(), body.clone())));
    let origin = {
        let served = Arc::clone(&served);
        Origin::start(move |asked| {
            let (etag, body) = served.lock().expect("lock").clone();
            if asked.if_none_match.as_deref() == Some(etag.as_str()) {
                Answer {
                    status: 304,
                    etag: Some(etag),
                    body: Vec::new(),
                    delay: Duration::ZERO,
                }
            } else {
                ok(Some(&etag), &body)
            }
        })
    };
    let dir = folder("etag");
    let transport = HttpTransport::on(plain());
    transport.keep_in(&dir);
    let url = format!("{}/releases", origin.url);
    let first = lookup(&transport, &url);
    assert_eq!((origin.asked(), origin.sent()), (1, 4000));
    let second = lookup(&transport, &url);
    assert_eq!(second, first, "the kept document answers");
    assert_eq!(
        (origin.asked(), origin.sent()),
        (2, 4000),
        "a request and no body"
    );
    {
        let seen = origin.seen.lock().expect("lock");
        assert_eq!(seen[1].path, "/releases");
        assert_eq!(seen[1].if_none_match.as_deref(), Some("\"v1\""));
    }

    // A run that starts later asks whether it changed too, from what was written.
    let later = HttpTransport::on(plain());
    later.keep_in(&dir);
    assert_eq!(lookup(&later, &url), first);
    assert_eq!((origin.asked(), origin.sent()), (3, 4000));

    // The publisher changes it: the new one is read in full and kept.
    *served.lock().expect("lock") = ("\"v2\"".to_owned(), vec![b'y'; 10]);
    assert_eq!(lookup(&transport, &url), "y".repeat(10));
    assert_eq!((origin.asked(), origin.sent()), (4, 4010));
    assert_eq!(lookup(&transport, &url), "y".repeat(10));
    assert_eq!((origin.asked(), origin.sent()), (5, 4010));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_document_with_no_etag_is_read_every_time_and_a_download_is_never_asked_conditionally() {
    let origin = Origin::start(|_| ok(None, b"plain"));
    let transport = HttpTransport::on(plain());
    let url = format!("{}/x", origin.url);
    lookup(&transport, &url);
    lookup(&transport, &url);
    assert_eq!((origin.asked(), origin.sent()), (2, 10));
    assert!(
        origin
            .seen
            .lock()
            .expect("lock")
            .iter()
            .all(|asked| asked.if_none_match.is_none())
    );
}

#[test]
fn a_lookup_waits_less_for_the_publisher_than_a_download() {
    let origin = Origin::start(|_| Answer {
        delay: Duration::from_millis(900),
        ..ok(None, b"slow")
    });
    let transport = HttpTransport::on(plain());
    let url = format!("{}/slow", origin.url);
    let mut limits = ToolsLimits {
        lookup_response: Duration::from_millis(250),
        lookup: true,
        ..ToolsLimits::default()
    };
    let started = std::time::Instant::now();
    let refused = transport.get(&url, &limits).expect_err("it gave up");
    assert!(started.elapsed() < Duration::from_millis(800), "{refused}");

    // The same publisher, asked as a download, is waited for.
    limits.lookup = false;
    let mut text = String::new();
    transport
        .get(&url, &limits)
        .expect("a download waits")
        .body
        .read_to_string(&mut text)
        .expect("read");
    assert_eq!(text, "slow");
}

#[test]
fn the_waits_are_the_tools_limits_and_a_store_reads_a_lookup_with_them() {
    let origin = Origin::start(|_| Answer {
        delay: Duration::from_millis(900),
        ..ok(None, b"slow")
    });
    let limits = ToolsLimits {
        lookup_response: Duration::from_millis(250),
        ..ToolsLimits::default()
    };
    let tools = LocalTools::new(
        Arc::new(HttpTransport::on(plain())),
        vec![origin.url.clone()],
    )
    .loopback_http(true)
    .limits(limits);
    let url = format!("{}/slow", origin.url);
    let started = std::time::Instant::now();
    let refused = tools.fetch_text(&url, 1024).expect_err("it gave up");
    assert!(
        started.elapsed() < Duration::from_millis(800),
        "{refused:?}"
    );
}
