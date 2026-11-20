use core::net::SocketAddr;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::io::{Read as _, Write as _};
use std::net::TcpStream;
use std::sync::Arc;

use super::*;

const FAST: Pace = Pace {
    poll: Duration::from_millis(20),
    take_after: 3,
    start_wait: Duration::from_secs(5),
};

fn free_address() -> SocketAddr {
    let held = std::net::TcpListener::bind("127.0.0.1:0").expect("a free port");
    held.local_addr().expect("its address")
}

fn web(listen: SocketAddr, secret: bool) -> WebConfig {
    let mut web = WebConfig::new();
    web.listen = listen;
    web.office = true;
    web.office_local = true;
    web.office_secret = secret;
    web
}

/// The status, the head in lower case and the body of a request as the pane's own page would send it.
fn ask(
    address: SocketAddr,
    method: &str,
    path: &str,
    extra: &[(&str, &str)],
) -> Option<(u16, String, String)> {
    let mut tcp = TcpStream::connect(address).ok()?;
    tcp.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let mut headers = vec![
        ("Host", address.to_string()),
        ("Origin", format!("http://{address}")),
        ("Sec-Fetch-Site", "same-origin".to_owned()),
        ("Content-Length", "0".to_owned()),
        ("Connection", "close".to_owned()),
    ];
    for (name, value) in extra {
        headers.retain(|(held, _)| !held.eq_ignore_ascii_case(name));
        headers.push((*name, (*value).to_owned()));
    }
    let mut request = format!("{method} {path} HTTP/1.1\r\n");
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("\r\n");
    tcp.write_all(request.as_bytes()).ok()?;
    let mut received = Vec::new();
    let _ = tcp.read_to_end(&mut received);
    let text = String::from_utf8_lossy(&received).into_owned();
    let (head, body) = text.split_once("\r\n\r\n")?;
    let status = head.split(' ').nth(1)?.parse().ok()?;
    Some((status, head.to_ascii_lowercase(), body.to_owned()))
}

fn until<T>(what: &str, mut found: impl FnMut() -> Option<T>) -> T {
    let limit = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(value) = found() {
            return value;
        }
        assert!(Instant::now() < limit, "{what}");
        thread::sleep(Duration::from_millis(10));
    }
}

/// A launcher whose host is the flag `running`, and whose start counts itself and sets the flag.
struct Standing {
    address: SocketAddr,
    running: Arc<AtomicBool>,
    started: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    done: Option<thread::JoinHandle<Result<(), String>>>,
    dir: std::path::PathBuf,
}

impl Standing {
    fn new(name: &str, secret: bool) -> Self {
        let address = free_address();
        let dir =
            std::env::temp_dir().join(format!("canoa-launcher-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (running, started, stop) = (
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicUsize::new(0)),
            Arc::new(AtomicBool::new(false)),
        );
        let config = web(address, secret);
        let (runs, starts, flag, ends, home) = (
            Arc::clone(&running),
            Arc::clone(&started),
            Arc::clone(&running),
            Arc::clone(&stop),
            dir.clone(),
        );
        let done = thread::spawn(move || {
            run(
                &config,
                &home,
                &|| runs.load(Ordering::SeqCst),
                &|| {
                    starts.fetch_add(1, Ordering::SeqCst);
                    flag.store(true, Ordering::SeqCst);
                    Ok(4242)
                },
                &|_| {},
                &|| ends.load(Ordering::SeqCst),
                &FAST,
            )
        });
        Self {
            address,
            running,
            started,
            stop,
            done: Some(done),
            dir,
        }
    }

    /// The launcher's answer once it holds the address.
    fn offers(&self) -> String {
        until("the launcher takes the address", || {
            ask(self.address, "GET", "/canoa.json", &[]).filter(|(status, ..)| *status == 200)
        })
        .2
    }
}

impl Drop for Standing {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(done) = self.done.take() {
            let _ = done.join();
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn a_pane_opened_with_no_host_finds_the_offer_and_asking_starts_a_host_and_lets_go_of_the_address()
{
    let launcher = Standing::new("offer", false);
    let index: serde_json::Value = serde_json::from_str(&launcher.offers()).expect("JSON");
    assert_eq!(index["launcher"], crate::web::LAUNCH_PATH);

    // The task pane's own address answers with the page that offers, under the add-in's policy.
    let (status, head, body) =
        ask(launcher.address, "GET", "/office/taskpane.html", &[]).expect("an answer");
    assert_eq!(status, 200);
    assert!(head.contains("content-type: text/html"), "{head}");
    assert_eq!(
        head.contains("officeapis.public.onecdn.static.microsoft"),
        cfg!(office_js_cdn),
        "Office.js is this origin's own unless the build names Microsoft's CDN: {head}"
    );
    assert!(body.contains("launch-page.js"), "{body}");
    assert_eq!(
        launcher.started.load(Ordering::SeqCst),
        0,
        "looking starts nothing"
    );
    // Nothing else of a host is served.
    for path in [
        "/",
        "/index.html",
        "/office/app/",
        "/office/admit",
        "/office/tools/poll?session=x",
    ] {
        let (status, ..) = ask(launcher.address, "GET", path, &[]).expect("an answer");
        assert_eq!(status, 404, "{path}");
    }

    // A request from a page of another origin does not start anything.
    let (status, ..) = ask(
        launcher.address,
        "POST",
        "/office/launch",
        &[("Origin", "https://elsewhere.example")],
    )
    .expect("an answer");
    assert_eq!(status, 403);
    assert_eq!(launcher.started.load(Ordering::SeqCst), 0);

    // The page's own request starts the host once, and the launcher lets go of the address for it.
    let (status, head, body) =
        ask(launcher.address, "POST", "/office/launch", &[]).expect("an answer");
    assert_eq!(status, 200, "{body}");
    assert!(head.contains("cache-control: no-store"));
    until("a host is started", || {
        (launcher.started.load(Ordering::SeqCst) == 1).then_some(())
    });
    until("the address is free for the host", || {
        std::net::TcpListener::bind(launcher.address).ok().map(drop)
    });

    // The host stops: the launcher holds the address again, after it has seen the host gone for a few looks.
    launcher.running.store(false, Ordering::SeqCst);
    launcher.offers();
}

#[test]
fn with_the_secret_on_a_request_without_it_is_refused_and_one_with_it_is_not() {
    let launcher = Standing::new("secret", true);
    launcher.offers();
    let value = std::fs::read_to_string(launcher.dir.join("secret")).expect("it is in the home");
    let value = value.trim();
    assert_eq!(value.len(), 64);

    let (status, ..) = ask(launcher.address, "POST", "/office/launch", &[]).expect("an answer");
    assert_eq!(status, 403, "no secret");
    let (status, ..) = ask(
        launcher.address,
        "POST",
        "/office/launch",
        &[("X-Canoa-Office-Key", &"0".repeat(64))],
    )
    .expect("an answer");
    assert_eq!(status, 403, "another secret");
    assert_eq!(launcher.started.load(Ordering::SeqCst), 0);

    let (status, ..) = ask(
        launcher.address,
        "POST",
        "/office/launch",
        &[("X-Canoa-Office-Key", value)],
    )
    .expect("an answer");
    assert_eq!(status, 200);
    until("a host is started", || {
        (launcher.started.load(Ordering::SeqCst) == 1).then_some(())
    });
}

#[test]
fn the_launcher_never_holds_the_address_while_a_host_runs() {
    let launcher = Standing::new("yield", false);
    launcher.offers();
    // A host appears (one the application started, say): the launcher lets go without being asked.
    launcher.running.store(true, Ordering::SeqCst);
    until("the address is free for the host", || {
        std::net::TcpListener::bind(launcher.address).ok().map(drop)
    });
    thread::sleep(Duration::from_millis(300));
    assert!(
        std::net::TcpListener::bind(launcher.address).is_ok(),
        "and does not take it again while the host runs"
    );
    assert_eq!(launcher.started.load(Ordering::SeqCst), 0);
}
