//! The add-in on the person's own computer, through a real host: the home's secret is asked for when the configuration says so, and
//! a page's key stays admitted across a restart, with its grants. No Office client runs; the requests are the page's own.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod linked;
mod support;

use std::io::{Read as _, Write as _};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use linked::{Hosted, entry};
use serde_json::{Value, json};

fn site() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("site")
}

fn config(secret: bool) -> impl FnOnce(&Path) -> String {
    move |_| {
        let mut text = entry("agent", "");
        text.push_str("[host.control]\nlisten = \"127.0.0.1:0\"\n");
        text.push_str("[host.web]\nlisten = \"127.0.0.1:0\"\noffice = true\noffice_local = true\n");
        if secret {
            text.push_str("office_secret = true\n");
        }
        text
    }
}

/// The address of the web server, once `serve` has printed it.
fn authority(host: &Hosted) -> String {
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        let found = host
            .printed
            .lock()
            .expect("not poisoned")
            .iter()
            .find_map(|line| line["web"].as_str().map(str::to_owned));
        if let Some(url) = found {
            return url
                .strip_prefix("http://")
                .expect("an http address")
                .to_owned();
        }
        assert!(Instant::now() < until, "the web address is printed");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The status and JSON of `POST path`, as the page's own `fetch` sends it, with `extra` headers.
fn post(authority: &str, path: &str, extra: &[(&str, &str)]) -> (u16, Value) {
    let mut tcp = TcpStream::connect(authority).expect("it connects");
    tcp.set_read_timeout(Some(Duration::from_secs(20)))
        .expect("a timeout");
    let mut request = format!(
        "POST {path} HTTP/1.1\r\nHost: {authority}\r\nOrigin: http://{authority}\r\nSec-Fetch-Site: same-origin\r\nContent-Length: 0\r\nConnection: close\r\n"
    );
    for (name, value) in extra {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("\r\n");
    tcp.write_all(request.as_bytes()).expect("sent");
    let mut received = Vec::new();
    let _ = tcp.read_to_end(&mut received);
    let text = String::from_utf8_lossy(&received).into_owned();
    let status = text
        .split(' ')
        .nth(1)
        .and_then(|status| status.parse().ok())
        .expect("a status");
    let body = text.split_once("\r\n\r\n").map_or("", |(_, body)| body);
    (status, serde_json::from_str(body).unwrap_or(Value::Null))
}

#[test]
fn with_the_secret_on_the_page_is_refused_without_it_and_asked_about_its_key_with_it() {
    let host = Hosted::start_with_setup(config(true), &["--web", &site().to_string_lossy()]);
    let authority = authority(&host);
    let secret = std::fs::read_to_string(host.scratch.0.join("home").join("office").join("secret"))
        .expect("the host made the secret in its home");
    let secret = secret.trim();
    assert_eq!(secret.len(), 64);
    let unknown = format!("sha256:{}", "ab".repeat(32));
    let admit = format!("/office/admit?peer={unknown}");

    let (status, said) = post(&authority, &admit, &[]);
    assert_eq!(status, 403, "no secret");
    assert!(
        said["said"].as_str().unwrap_or_default().contains("secret"),
        "{said}"
    );
    assert_eq!(
        post(
            &authority,
            &admit,
            &[("X-Canoa-Office-Key", &"0".repeat(64))]
        )
        .0,
        403,
        "another secret"
    );
    assert_eq!(
        post(&authority, &admit, &[("X-Canoa-Office-Key", secret)]).0,
        404,
        "with the secret the request is looked at: this key never connected"
    );
    host.stop();
}

#[test]
fn a_pages_key_stays_admitted_across_a_restart_with_its_grants() {
    let (guest, stranger) = (support::key(), support::key());
    let first = Hosted::start_with_setup(config(false), &["--web", &site().to_string_lossy()]);
    let line = first
        .line_of("@control")
        .expect("the control channel is served");
    let peer = line["peer"]
        .as_str()
        .expect("the host's id")
        .parse()
        .expect("an id");
    let url_of = |host: &Hosted| {
        host.line_of("@control").expect("a control line")["url"]
            .as_str()
            .expect("an address")
            .to_owned()
    };
    let ledger = |host: &Hosted, who: &_| {
        support::with_link(&url_of(host), who, peer, |rpc| {
            rpc.request("ledger/list", json!({"limit": 1}))
        })
    };
    let works = |answer: Result<Result<Value, Value>, String>| matches!(answer, Ok(Ok(_)));

    // The page's key connects once and is turned away, asks to be admitted, and then has the control channel.
    assert!(!works(ledger(&first, &guest)));
    let (status, _) = post(
        &authority(&first),
        &format!("/office/admit?peer={}", guest.peer_id()),
        &[],
    );
    assert_eq!(status, 200);
    assert!(works(ledger(&first, &guest)));
    let kept = first
        .scratch
        .0
        .join("home")
        .join("office")
        .join("admitted.json");
    assert!(
        std::fs::read_to_string(&kept)
            .expect("the admitted keys are kept in the home")
            .contains(&guest.peer_id().to_string())
    );
    first.stop();
    first
        .ended
        .join()
        .expect("the host ended")
        .expect("it ended well");

    // The host starts again on the same home: the key is held as the page's at once, with no request.
    let second = Hosted::start_in_setup(
        first.scratch,
        config(false),
        &["--web", &site().to_string_lossy()],
    );
    assert!(
        works(ledger(&second, &guest)),
        "a restart needs no new admission"
    );
    // A key that never asked is not held, whatever the file says of others.
    assert!(!works(ledger(&second, &stranger)));
    second.stop();
}
