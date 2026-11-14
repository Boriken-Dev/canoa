//! The portal against stand-in accounts and a stand-in for each user's host: the sign-in, the cookie, the throttle, the relay and
//! the idle stop. The real accounts and real hosts are in tests/portal_users.rs.
//!
//! A browser behind the proxy comes on the Unix socket (`callu`, the proxy's address in `X-Forwarded-For`); one on the
//! loopback TCP address (`call`) is what any local user could be, and is believed about nothing.

use alloc::sync::Arc;
use core::net::SocketAddr;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use core::time::Duration;
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::Mutex;
use std::thread;

use coqui_transport::Channel;
use coqui_transport_pipe::{Listener, PipeName};

use super::*;

/// Accounts that are `name:password` pairs; counts each time it is asked. The password `FAIL` is an accounts failure.
struct Fixed {
    /// Each name's password and user id.
    accounts: Mutex<HashMap<String, (String, u32)>>,
    gone: Mutex<HashSet<String>>,
    unsure: Mutex<HashSet<String>>,
    asked: AtomicUsize,
}

/// This process's user id: the stand-in hosts run in it, so it is whose process made their sockets.
fn mine() -> u32 {
    std::os::unix::fs::MetadataExt::uid(&std::fs::metadata("/proc/self").unwrap())
}

impl Accounts for Fixed {
    fn sign_in(&self, user: &str, password: &str) -> SignIn {
        self.asked.fetch_add(1, Ordering::SeqCst);
        if password == "FAIL" {
            return SignIn::Failed("the directory is down".to_owned());
        }
        match self.accounts.lock().unwrap().get(user) {
            Some((expected, uid)) if expected == password => SignIn::Admitted {
                user: user.to_owned(),
                uid: *uid,
            },
            _ => SignIn::Refused,
        }
    }

    fn still(&self, user: &str, _uid: u32) -> Standing {
        if self.gone.lock().unwrap().contains(user) {
            Standing::Gone
        } else if self.unsure.lock().unwrap().contains(user) {
            Standing::Unknown
        } else {
            Standing::Present
        }
    }
}

/// A host for each user that starts when asked: an introduction socket and one entry, `x`, that answers each message with the
/// user's name and the message. The host of `canoatestslow` takes a request and never answers it.
struct Hosts {
    dir: PathBuf,
    running: Mutex<HashMap<String, Arc<AtomicBool>>>,
    starts: AtomicUsize,
    stops: AtomicUsize,
    introduced: Arc<Mutex<Vec<String>>>,
    /// How many `accept` requests are answered "not waiting" before one succeeds.
    refuse_accepts: usize,
}

impl Controller for Hosts {
    fn start(&self, user: &str) -> io::Result<()> {
        if !valid_user(user) {
            return Err(io::Error::other("not a user"));
        }
        let mut running = self.running.lock().unwrap();
        if running.contains_key(user) {
            return Ok(());
        }
        self.starts.fetch_add(1, Ordering::SeqCst);
        let stop = Arc::new(AtomicBool::new(false));
        running.insert(user.to_owned(), Arc::clone(&stop));
        let home = self.dir.join(user);
        std::fs::create_dir_all(home.join("entries"))?;
        let bind = |path: PathBuf| Listener::bind(&PipeName::at(path).unwrap()).unwrap();
        let introduction = bind(home.join("portal.sock"));
        let entry = bind(home.join("entries").join("x.sock"));
        let control = bind(home.join("control.sock"));
        let (introduced, refuse) = (Arc::clone(&self.introduced), self.refuse_accepts);
        let name = user.to_owned();
        {
            let stop = Arc::clone(&stop);
            thread::spawn(move || {
                let mut refused = 0;
                while !stop.load(Ordering::SeqCst) {
                    let Ok(stream) = introduction.accept_timeout(Some(Duration::from_millis(20)))
                    else {
                        continue;
                    };
                    if name == "canoatestslow" {
                        // Takes the request and says nothing, for as long as the test runs.
                        let stop = Arc::clone(&stop);
                        thread::spawn(move || {
                            while !stop.load(Ordering::SeqCst) {
                                thread::sleep(Duration::from_millis(50));
                            }
                            drop(stream);
                        });
                        continue;
                    }
                    let mut line = Vec::new();
                    let mut byte = [0_u8; 1];
                    let mut reader = &stream;
                    while reader.read(&mut byte).is_ok_and(|read| read == 1) && byte[0] != b'\n' {
                        line.push(byte[0]);
                    }
                    let request: serde_json::Value =
                        serde_json::from_slice(&line).unwrap_or_default();
                    let reply = match request["op"].as_str() {
                        Some("index") => {
                            serde_json::json!({"peer": "sha256:host", "method": "noise", "os": "linux",
                            "entries": [{"name": "x", "kind": "agent"}, {"name": "@control", "kind": "control"}]})
                        }
                        Some("accept") if refused < refuse => {
                            refused += 1;
                            serde_json::json!({"error": "that key is not waiting"})
                        }
                        Some("accept") => {
                            introduced
                                .lock()
                                .unwrap()
                                .push(format!("{name}:{}", request["peer"].as_str().unwrap_or("")));
                            serde_json::json!({"ok": true})
                        }
                        _ => serde_json::json!({"error": "no"}),
                    };
                    let mut writer = &stream;
                    let _ = writer.write_all(format!("{reply}\n").as_bytes());
                }
            });
        }
        for (listener, tag) in [(entry, "entry"), (control, "control")] {
            let (stop, name) = (Arc::clone(&stop), user.to_owned());
            thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    let Ok(stream) = listener.accept_timeout(Some(Duration::from_millis(20)))
                    else {
                        continue;
                    };
                    let (stop, name) = (Arc::clone(&stop), name.clone());
                    thread::spawn(move || {
                        let channel = crate::pipe::PipeChannel::new(stream, 1 << 20);
                        while !stop.load(Ordering::SeqCst) {
                            match channel.receive(Some(Duration::from_millis(50))) {
                                Ok(Some(message)) => {
                                    let mut reply = format!("{name}/{tag}:").into_bytes();
                                    reply.extend_from_slice(&message);
                                    if channel.send(&reply).is_err() {
                                        break;
                                    }
                                }
                                Ok(None) => break,
                                Err(coqui_transport::Failure::Timeout) => {}
                                Err(_) => break,
                            }
                        }
                    });
                }
            });
        }
        Ok(())
    }

    fn stop(&self, user: &str) -> io::Result<()> {
        self.stops.fetch_add(1, Ordering::SeqCst);
        if let Some(stop) = self.running.lock().unwrap().remove(user) {
            stop.store(true, Ordering::SeqCst);
        }
        // The listeners notice within their rest and remove their sockets.
        thread::sleep(Duration::from_millis(100));
        Ok(())
    }

    fn running(&self) -> io::Result<Vec<String>> {
        Ok(self.running.lock().unwrap().keys().cloned().collect())
    }
}

struct Rig {
    addr: SocketAddr,
    sock: PathBuf,
    accounts: Arc<Fixed>,
    hosts: Arc<Hosts>,
    stopper: Stopper,
    dir: PathBuf,
    root: PathBuf,
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.stopper.stop();
        for stop in self.hosts.running.lock().unwrap().values() {
            stop.store(true, Ordering::SeqCst);
        }
        thread::sleep(Duration::from_millis(80));
        let _ = std::fs::remove_dir_all(&self.dir);
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn rig(change: impl FnOnce(&mut Settings), refuse_accepts: usize) -> Rig {
    static COUNT: AtomicUsize = AtomicUsize::new(0);
    let id = COUNT.fetch_add(1, Ordering::SeqCst);
    let base = std::env::temp_dir().join(format!("canoa-portal-{}-{id}", std::process::id()));
    let dir = base.join("run");
    let root = base.join("site");
    let sock = base.join("front").join("portal.sock");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(sock.parent().unwrap()).unwrap();
    std::fs::write(root.join("index.html"), "<!doctype html>the app").unwrap();
    let accounts = Arc::new(Fixed {
        accounts: Mutex::new(
            [
                ("canoatesta", "pw-a", mine()),
                ("canoatestb", "pw-b", mine()),
                ("canoatestslow", "pw-s", mine()),
                ("rootish", "pw-r", 0),
                ("lowuid", "pw-l", 999),
            ]
            .into_iter()
            .map(|(name, password, uid)| (name.to_owned(), (password.to_owned(), uid)))
            .collect(),
        ),
        gone: Mutex::new(HashSet::new()),
        unsure: Mutex::new(HashSet::new()),
        asked: AtomicUsize::new(0),
    });
    let hosts = Arc::new(Hosts {
        dir: dir.clone(),
        running: Mutex::new(HashMap::new()),
        starts: AtomicUsize::new(0),
        stops: AtomicUsize::new(0),
        introduced: Arc::new(Mutex::new(Vec::new())),
        refuse_accepts,
    });
    let mut settings = Settings {
        socket: Some(sock.clone()),
        listen: Some("127.0.0.1:0".parse().unwrap()),
        site: Some(root.clone()),
        socket_dir: dir.clone(),
        pair: Duration::from_secs(3),
        failure_floor: Duration::ZERO,
        slow: Duration::ZERO,
        ..Settings::default()
    };
    change(&mut settings);
    let portal = Portal::bind(settings, accounts.clone(), hosts.clone()).unwrap();
    let addr = portal.local_addr().unwrap();
    let stopper = portal.stopper();
    thread::spawn(move || portal.run());
    Rig {
        addr,
        sock,
        accounts,
        hosts,
        stopper,
        dir,
        root,
    }
}

struct Answer {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Answer {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(each, _)| each.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    fn cookie(&self) -> Option<String> {
        self.header("set-cookie")
            .and_then(|line| line.split(';').next())
            .filter(|pair| !pair.ends_with('='))
            .map(str::to_owned)
    }
}

/// Where a request is made: the loopback address, or the proxy's socket.
#[derive(Clone)]
enum To {
    Tcp(SocketAddr),
    Unix(PathBuf),
}

impl To {
    fn host(&self) -> String {
        match self {
            Self::Tcp(addr) => addr.to_string(),
            Self::Unix(_) => "localhost".to_owned(),
        }
    }

    fn origin(&self) -> String {
        format!("http://{}", self.host())
    }

    fn connect(&self) -> Conn {
        match self {
            Self::Tcp(addr) => Conn::Tcp(Arc::new(TcpStream::connect(addr).unwrap())),
            Self::Unix(path) => Conn::Pipe(Arc::new(
                coqui_transport_pipe::Stream::connect(&PipeName::at(path).unwrap()).unwrap(),
            )),
        }
    }
}

fn request(to: &To, method: &str, path: &str, extra: &[(&str, String)], body: &str) -> Answer {
    let mut conn = to.connect();
    conn.set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    let mut text = format!(
        "{method} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n",
        to.host()
    );
    for (name, value) in extra {
        text.push_str(&format!("{name}: {value}\r\n"));
    }
    if !body.is_empty() {
        text.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    text.push_str("\r\n");
    text.push_str(body);
    conn.write_all(text.as_bytes()).unwrap();
    let mut raw = Vec::new();
    let _ = conn.read_to_end(&mut raw);
    let raw = String::from_utf8_lossy(&raw).into_owned();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    Answer {
        status,
        headers: lines
            .filter_map(|line| line.split_once(':'))
            .map(|(name, value)| (name.trim().to_owned(), value.trim().to_owned()))
            .collect(),
        body: body.to_owned(),
    }
}

fn call(
    addr: SocketAddr,
    method: &str,
    path: &str,
    extra: &[(&str, String)],
    body: &str,
) -> Answer {
    request(&To::Tcp(addr), method, path, extra, body)
}

/// A sign-in from the proxy, saying the browser is at `forwarded`.
fn login(rig: &Rig, user: &str, password: &str, forwarded: &str) -> Answer {
    let to = To::Unix(rig.sock.clone());
    request(
        &to,
        "POST",
        LOGIN,
        &[
            ("Origin", to.origin()),
            (
                "Content-Type",
                "application/x-www-form-urlencoded".to_owned(),
            ),
            ("X-Forwarded-For", forwarded.to_owned()),
        ],
        &format!("user={user}&password={password}"),
    )
}

fn signed_in(rig: &Rig, user: &str) -> String {
    let password = match user {
        "canoatesta" => "pw-a",
        "canoatestb" => "pw-b",
        _ => "pw-s",
    };
    login(rig, user, password, "198.51.100.7")
        .cookie()
        .expect("a cookie")
}

fn origin(rig: &Rig) -> String {
    format!("http://{}", rig.addr)
}

#[test]
fn nothing_but_the_login_is_given_to_a_browser_that_has_not_signed_in() {
    let rig = rig(|_| {}, 0);
    let page = call(rig.addr, "GET", LOGIN, &[], "");
    assert_eq!(page.status, 200);
    assert!(page.body.contains("<form"));
    let home = call(rig.addr, "GET", "/", &[], "");
    assert_eq!((home.status, home.header("location")), (303, Some(LOGIN)));
    assert_eq!(call(rig.addr, "GET", web::HOST_INDEX, &[], "").status, 401);
    assert_eq!(
        call(rig.addr, "GET", "/portal/session", &[], "").status,
        401
    );
    assert_eq!(call(rig.addr, "GET", "/index.html", &[], "").status, 303);
}

#[test]
fn a_right_password_gives_a_cookie_that_script_cannot_read_and_the_site() {
    let rig = rig(|_| {}, 0);
    let answer = login(&rig, "canoatesta", "pw-a", "198.51.100.7");
    assert_eq!((answer.status, answer.header("location")), (303, Some("/")));
    let line = answer.header("set-cookie").unwrap().to_owned();
    for attribute in [
        "__Host-canoa=",
        "HttpOnly",
        "Secure",
        "SameSite=Strict",
        "Path=/",
        "Max-Age=",
    ] {
        assert!(line.contains(attribute), "{attribute} in {line}");
    }
    assert!(!line.contains("Domain"));
    let cookie = answer.cookie().unwrap();
    let home = call(rig.addr, "GET", "/", &[("Cookie", cookie.clone())], "");
    assert_eq!(home.status, 200);
    assert!(home.body.contains("the app"));
    let token = cookie.split_once('=').unwrap().1;
    let session = call(
        rig.addr,
        "GET",
        "/portal/session",
        &[("Cookie", cookie.clone())],
        "",
    );
    assert_eq!(session.status, 200);
    assert!(session.body.contains("canoatesta") && !session.body.contains(token));
    assert!(!home.body.contains(token) && !answer.body.contains(token));
}

#[test]
fn the_login_form_works_with_the_headers_a_real_browser_sends() {
    // Edge and Chrome send Origin, Sec-Fetch-Site and a Referer with a form posted from the page, and `null` for the origin
    // when the page said it sends no referrer: the login page must not say that.
    let rig = rig(|_| {}, 0);
    let to = To::Unix(rig.sock.clone());
    let page = request(&to, "GET", LOGIN, &[], "");
    assert_eq!(page.header("referrer-policy"), Some("same-origin"));
    assert_eq!(
        request(&to, "GET", "/portal/session", &[], "").header("referrer-policy"),
        Some("same-origin")
    );
    let browser = |origin: &str, site: &str| {
        request(
            &to,
            "POST",
            LOGIN,
            &[
                ("Origin", origin.to_owned()),
                ("Sec-Fetch-Site", site.to_owned()),
                ("Sec-Fetch-Mode", "navigate".to_owned()),
                ("Referer", "http://localhost/portal/login".to_owned()),
                (
                    "Content-Type",
                    "application/x-www-form-urlencoded".to_owned(),
                ),
                ("X-Forwarded-For", "198.51.100.7".to_owned()),
            ],
            "user=canoatesta&password=pw-a",
        )
    };
    assert_eq!(browser("http://localhost", "same-origin").status, 303);
    // A `null` origin is nobody's, and a form from another site is turned away.
    assert_eq!(browser("null", "same-origin").status, 403);
    assert_eq!(browser("http://localhost", "cross-site").status, 403);
    assert_eq!(browser("http://evil.example", "same-site").status, 403);
}

#[test]
fn the_origin_must_be_the_sites_own_scheme_and_host() {
    let rig = rig(|_| {}, 0);
    let to = To::Unix(rig.sock.clone());
    let post = |origin: &str, proto: Option<&str>| {
        let mut extra = vec![
            ("Origin", origin.to_owned()),
            (
                "Content-Type",
                "application/x-www-form-urlencoded".to_owned(),
            ),
            ("X-Forwarded-For", "198.51.100.7".to_owned()),
        ];
        if let Some(proto) = proto {
            extra.push(("X-Forwarded-Proto", proto.to_owned()));
        }
        request(&to, "POST", LOGIN, &extra, "user=canoatesta&password=pw-a").status
    };
    // Plain http on localhost is the site's own scheme; https is not, until the proxy says the page came over it.
    assert_eq!(post("https://localhost", None), 403);
    assert_eq!(post("http://localhost", Some("https")), 403);
    assert_eq!(post("https://localhost", Some("https")), 303);
    assert_eq!(post("http://localhost", None), 303);
}

#[test]
fn a_wrong_password_is_refused_and_one_addresss_failures_for_an_account_stop_the_tries_before_they_reach_the_accounts()
 {
    let rig = rig(|settings| settings.max_failures_per_pair = 3, 0);
    for _ in 0..3 {
        let answer = login(&rig, "canoatesta", "wrong", "203.0.113.9");
        assert_eq!(answer.status, 401);
        assert!(answer.cookie().is_none());
    }
    assert_eq!(rig.accounts.asked.load(Ordering::SeqCst), 3);
    // Past the limit even the right password is not asked of the accounts.
    let blocked = login(&rig, "canoatesta", "pw-a", "203.0.113.9");
    assert_eq!(blocked.status, 429);
    assert!(blocked.cookie().is_none());
    assert_eq!(rig.accounts.asked.load(Ordering::SeqCst), 3);
    // Another browser behind the proxy is not held to it, nor this one for another account.
    assert_eq!(
        login(&rig, "canoatesta", "pw-a", "203.0.113.10").status,
        303
    );
    assert_eq!(login(&rig, "canoatestb", "pw-b", "203.0.113.9").status, 303);
}

#[test]
fn tries_made_together_cannot_all_pass_the_limit_before_any_is_counted() {
    let rig = Arc::new(rig(|settings| settings.max_failures_per_pair = 3, 0));
    let threads: Vec<_> = (0..16)
        .map(|_| {
            let rig = Arc::clone(&rig);
            thread::spawn(move || login(&rig, "canoatesta", "wrong", "203.0.113.20").status)
        })
        .collect();
    let statuses: Vec<u16> = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();
    assert_eq!(rig.accounts.asked.load(Ordering::SeqCst), 3, "{statuses:?}");
    assert_eq!(statuses.iter().filter(|status| **status == 429).count(), 13);
}

#[test]
fn an_accounts_failure_counts_and_a_busy_portal_does_not() {
    let rig = rig(|settings| settings.max_failures_per_pair = 2, 0);
    assert_eq!(
        login(&rig, "canoatesta", "FAIL", "203.0.113.30").status,
        503
    );
    assert_eq!(
        login(&rig, "canoatesta", "FAIL", "203.0.113.30").status,
        503
    );
    assert_eq!(
        login(&rig, "canoatesta", "pw-a", "203.0.113.30").status,
        429
    );
    assert_eq!(rig.accounts.asked.load(Ordering::SeqCst), 2);
}

#[test]
fn an_address_the_proxy_names_counts_and_one_a_local_user_names_does_not() {
    // On the loopback TCP address any local user can reach, the header is not believed: every try is from one address.
    let rig = rig(|settings| settings.max_failures_per_pair = 2, 0);
    let over_tcp = |forwarded: &str, password: &str| {
        call(
            rig.addr,
            "POST",
            LOGIN,
            &[
                ("Origin", origin(&rig)),
                (
                    "Content-Type",
                    "application/x-www-form-urlencoded".to_owned(),
                ),
                ("X-Forwarded-For", forwarded.to_owned()),
            ],
            &format!("user=canoatesta&password={password}"),
        )
        .status
    };
    assert_eq!(over_tcp("203.0.113.1", "wrong"), 401);
    assert_eq!(over_tcp("203.0.113.2", "wrong"), 401);
    assert_eq!(over_tcp("203.0.113.3", "pw-a"), 429);
    // On the proxy's socket the last address is the browser's; what the browser itself sent before it is not.
    let rig = self::rig(
        |settings| {
            settings.max_failures_per_pair = 2;
            settings.trusted_proxies = vec!["10.0.0.1".parse().unwrap()];
        },
        0,
    );
    login(&rig, "canoatesta", "wrong", "203.0.113.1");
    login(&rig, "canoatesta", "wrong", "evil, 203.0.113.1, 10.0.0.1");
    assert_eq!(login(&rig, "canoatesta", "pw-a", "203.0.113.1").status, 429);
    assert_eq!(login(&rig, "canoatesta", "pw-a", "203.0.113.2").status, 303);
}

#[test]
fn an_account_is_slowed_for_everyone_once_it_has_many_failures_and_never_shut_out() {
    let rig = rig(
        |settings| {
            settings.slow_after_user_failures = 1;
            settings.slow = Duration::from_millis(300);
            settings.max_failures_per_pair = 5;
        },
        0,
    );
    for from in ["203.0.113.40", "203.0.113.41"] {
        login(&rig, "canoatesta", "wrong", from);
    }
    let started = std::time::Instant::now();
    let slowed = login(&rig, "canoatesta", "pw-a", "203.0.113.42");
    assert_eq!(slowed.status, 303, "the right password still gets in");
    assert!(
        started.elapsed() >= Duration::from_millis(250),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn a_failed_sign_in_takes_as_long_whether_or_not_the_account_exists() {
    let rig = rig(
        |settings| settings.failure_floor = Duration::from_millis(400),
        0,
    );
    for user in ["canoatesta", "no-such-canoa-account", "rootish", "lowuid"] {
        let started = std::time::Instant::now();
        let answer = login(&rig, user, "wrong", "203.0.113.50");
        assert_eq!(answer.status, 401, "{user}");
        assert!(
            started.elapsed() >= Duration::from_millis(380),
            "{user}: {:?}",
            started.elapsed()
        );
    }
    // A success is not held back.
    let started = std::time::Instant::now();
    assert_eq!(
        login(&rig, "canoatesta", "pw-a", "203.0.113.51").status,
        303
    );
    assert!(
        started.elapsed() < Duration::from_millis(300),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn root_and_an_account_below_the_lowest_user_id_never_sign_in_even_with_the_right_password() {
    let rig = rig(|_| {}, 0);
    for (user, password) in [("rootish", "pw-r"), ("lowuid", "pw-l")] {
        let answer = login(&rig, user, password, "203.0.113.60");
        assert_eq!(answer.status, 401, "{user}");
        assert!(answer.cookie().is_none());
    }
}

#[test]
fn a_sign_in_from_another_site_is_refused_before_the_accounts_are_asked() {
    let rig = rig(|_| {}, 0);
    for origin in ["http://evil.example", "null"] {
        let answer = call(
            rig.addr,
            "POST",
            LOGIN,
            &[
                ("Origin", origin.to_owned()),
                (
                    "Content-Type",
                    "application/x-www-form-urlencoded".to_owned(),
                ),
            ],
            "user=canoatesta&password=pw-a",
        );
        assert_eq!(answer.status, 403, "{origin}");
    }
    let none = call(
        rig.addr,
        "POST",
        LOGIN,
        &[(
            "Content-Type",
            "application/x-www-form-urlencoded".to_owned(),
        )],
        "user=canoatesta&password=pw-a",
    );
    assert_eq!(none.status, 403);
    assert_eq!(rig.accounts.asked.load(Ordering::SeqCst), 0);
    let host = call(
        rig.addr,
        "GET",
        LOGIN,
        &[("Host", "evil.example".to_owned())],
        "",
    );
    assert!(host.status == 421 || host.status == 400, "{}", host.status);
}

#[test]
fn signing_out_ends_the_session_and_clears_the_cookie() {
    let rig = rig(|_| {}, 0);
    let cookie = signed_in(&rig, "canoatesta");
    let out = call(
        rig.addr,
        "POST",
        "/portal/logout",
        &[("Cookie", cookie.clone()), ("Origin", origin(&rig))],
        "",
    );
    assert_eq!(out.status, 303);
    assert!(out.header("set-cookie").unwrap().contains("Max-Age=0"));
    assert_eq!(
        call(
            rig.addr,
            "GET",
            "/portal/session",
            &[("Cookie", cookie)],
            ""
        )
        .status,
        401
    );
}

#[test]
fn an_account_keeps_only_so_many_sessions() {
    let rig = rig(|settings| settings.max_sessions_per_user = 2, 0);
    let first = signed_in(&rig, "canoatesta");
    let second = signed_in(&rig, "canoatesta");
    let other = signed_in(&rig, "canoatestb");
    let third = signed_in(&rig, "canoatesta");
    let alive = |cookie: &str| {
        call(
            rig.addr,
            "GET",
            "/portal/session",
            &[("Cookie", cookie.to_owned())],
            "",
        )
        .status
    };
    assert_eq!(alive(&first), 401, "the oldest went");
    assert_eq!(
        (alive(&second), alive(&third), alive(&other)),
        (200, 200, 200)
    );
}

#[test]
fn the_index_lists_the_users_own_host_with_channels_through_the_portal_and_starts_it_once() {
    let rig = rig(|_| {}, 0);
    let cookie = signed_in(&rig, "canoatesta");
    for _ in 0..2 {
        let index = call(
            rig.addr,
            "GET",
            web::HOST_INDEX,
            &[("Cookie", cookie.clone())],
            "",
        );
        assert_eq!(index.status, 200);
        let json: serde_json::Value = serde_json::from_str(&index.body).unwrap();
        assert_eq!(json["peer"], "sha256:host");
        assert_eq!(json["user"], "canoatesta");
        let urls: Vec<&str> = json["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["url"].as_str().unwrap())
            .collect();
        let host = rig.addr.to_string();
        assert_eq!(
            urls,
            [
                format!("ws://{host}/portal/ws/entry/x"),
                format!("ws://{host}/portal/ws/control")
            ]
        );
    }
    assert_eq!(rig.hosts.starts.load(Ordering::SeqCst), 1);
}

#[test]
fn only_the_sites_own_page_may_start_the_users_host() {
    let rig = rig(|_| {}, 0);
    let cookie = signed_in(&rig, "canoatesta");
    let get = |site: Option<&str>| {
        let mut extra = vec![("Cookie", cookie.clone())];
        if let Some(site) = site {
            extra.push(("Sec-Fetch-Site", site.to_owned()));
        }
        call(rig.addr, "GET", web::HOST_INDEX, &extra, "").status
    };
    assert_eq!(get(Some("cross-site")), 403);
    assert_eq!(get(Some("same-site")), 403);
    assert_eq!(
        rig.hosts.starts.load(Ordering::SeqCst),
        0,
        "a cross-site request started a host"
    );
    assert_eq!(get(Some("same-origin")), 200);
    assert_eq!(get(None), 200);
}

#[test]
fn an_account_that_is_gone_is_given_no_host_and_loses_its_sessions() {
    let rig = rig(|_| {}, 0);
    let cookie = signed_in(&rig, "canoatesta");
    assert_eq!(
        call(
            rig.addr,
            "GET",
            web::HOST_INDEX,
            &[("Cookie", cookie.clone())],
            ""
        )
        .status,
        200
    );
    rig.accounts
        .gone
        .lock()
        .unwrap()
        .insert("canoatesta".to_owned());
    assert_eq!(
        call(
            rig.addr,
            "GET",
            web::HOST_INDEX,
            &[("Cookie", cookie.clone())],
            ""
        )
        .status,
        401
    );
    assert_eq!(
        call(
            rig.addr,
            "GET",
            "/portal/session",
            &[("Cookie", cookie)],
            ""
        )
        .status,
        401
    );
}

#[test]
fn only_so_many_hosts_run_at_once() {
    let rig = rig(|settings| settings.max_hosts = 1, 0);
    let (a, b) = (signed_in(&rig, "canoatesta"), signed_in(&rig, "canoatestb"));
    assert_eq!(
        call(rig.addr, "GET", web::HOST_INDEX, &[("Cookie", a)], "").status,
        200
    );
    assert_eq!(
        call(rig.addr, "GET", web::HOST_INDEX, &[("Cookie", b)], "").status,
        503
    );
    assert_eq!(rig.hosts.starts.load(Ordering::SeqCst), 1);
}

#[test]
fn a_host_that_never_answers_delays_nobody_else() {
    let rig = Arc::new(rig(|settings| settings.start = Duration::from_secs(2), 0));
    let (slow, b) = (
        signed_in(&rig, "canoatestslow"),
        signed_in(&rig, "canoatestb"),
    );
    let waiting = {
        let rig = Arc::clone(&rig);
        thread::spawn(move || {
            call(rig.addr, "GET", web::HOST_INDEX, &[("Cookie", slow)], "").status
        })
    };
    thread::sleep(Duration::from_millis(300));
    let started = std::time::Instant::now();
    assert_eq!(
        call(rig.addr, "GET", web::HOST_INDEX, &[("Cookie", b)], "").status,
        200
    );
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    // The one that is waiting ends within the question's own deadline, not a read at a time.
    let began = std::time::Instant::now();
    assert_eq!(waiting.join().unwrap(), 503);
    assert!(
        began.elapsed() < Duration::from_secs(14),
        "{:?}",
        began.elapsed()
    );
}

fn open_channel(
    rig: &Rig,
    path: &str,
    cookie: &str,
    origin: &str,
) -> Result<tungstenite::WebSocket<TcpStream>, u16> {
    let stream = TcpStream::connect(rig.addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let request = tungstenite::http::Request::builder()
        .uri(format!("ws://{}{path}", rig.addr))
        .header("Host", rig.addr.to_string())
        .header("Origin", origin)
        .header("Cookie", cookie)
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header(
            "Sec-WebSocket-Key",
            tungstenite::handshake::client::generate_key(),
        )
        .body(())
        .unwrap();
    match tungstenite::client(request, stream) {
        Ok((socket, _)) => Ok(socket),
        Err(tungstenite::HandshakeError::Failure(tungstenite::Error::Http(response))) => {
            Err(response.status().as_u16())
        }
        Err(_) => Err(0),
    }
}

fn exchange(socket: &mut tungstenite::WebSocket<TcpStream>, message: &[u8]) -> String {
    socket
        .send(tungstenite::Message::Binary(message.to_vec().into()))
        .unwrap();
    match socket.read().unwrap() {
        tungstenite::Message::Binary(reply) => String::from_utf8_lossy(&reply).into_owned(),
        other => panic!("{other:?}"),
    }
}

#[test]
fn each_user_reaches_only_their_own_host_and_a_channel_needs_the_cookie_the_page_and_a_plain_name()
{
    let rig = rig(|_| {}, 0);
    let (a, b) = (signed_in(&rig, "canoatesta"), signed_in(&rig, "canoatestb"));
    let origin = origin(&rig);
    let mut for_a = open_channel(&rig, "/portal/ws/entry/x", &a, &origin).unwrap();
    let mut for_b = open_channel(&rig, "/portal/ws/entry/x", &b, &origin).unwrap();
    assert_eq!(exchange(&mut for_a, b"hello"), "canoatesta/entry:hello");
    assert_eq!(exchange(&mut for_b, b"hello"), "canoatestb/entry:hello");
    let mut control = open_channel(&rig, "/portal/ws/control", &a, &origin).unwrap();
    assert_eq!(
        exchange(&mut control, b"ledger"),
        "canoatesta/control:ledger"
    );
    for path in [
        "/portal/ws/entry/..%2Fcanoatestb",
        "/portal/ws/entry/../../canoatestb/entries/x",
        "/portal/ws/entry/x.sock",
        "/portal/ws/entry/",
        "/portal/ws/entry/canoatestb/x",
        "/portal/ws/canoatestb",
    ] {
        assert!(
            matches!(open_channel(&rig, path, &a, &origin), Err(404 | 400)),
            "{path}"
        );
    }
    assert_eq!(
        open_channel(&rig, "/portal/ws/entry/x", "__Host-canoa=nothing", &origin).err(),
        Some(401)
    );
    assert_eq!(
        open_channel(&rig, "/portal/ws/entry/x", &a, "http://evil.example").err(),
        Some(403)
    );
    assert_eq!(
        open_channel(&rig, "/portal/ws/entry/x", &a, "").err(),
        Some(403)
    );
}

#[test]
fn signing_out_closes_the_channels_the_session_holds() {
    let rig = rig(|_| {}, 0);
    let cookie = signed_in(&rig, "canoatesta");
    let origin = origin(&rig);
    let mut socket = open_channel(&rig, "/portal/ws/entry/x", &cookie, &origin).unwrap();
    assert_eq!(exchange(&mut socket, b"a"), "canoatesta/entry:a");
    call(
        rig.addr,
        "POST",
        "/portal/logout",
        &[("Cookie", cookie), ("Origin", origin)],
        "",
    );
    let ended = (0..40).any(|_| {
        thread::sleep(Duration::from_millis(100));
        socket
            .send(tungstenite::Message::Binary(b"b".to_vec().into()))
            .is_err()
            || socket.read().is_err()
    });
    assert!(ended, "the channel outlived the session");
}

#[test]
fn a_user_may_hold_only_so_many_channels() {
    let rig = rig(|settings| settings.max_relays_per_user = 2, 0);
    let cookie = signed_in(&rig, "canoatesta");
    let origin = origin(&rig);
    let _one = open_channel(&rig, "/portal/ws/entry/x", &cookie, &origin).unwrap();
    let _two = open_channel(&rig, "/portal/ws/entry/x", &cookie, &origin).unwrap();
    assert_eq!(
        open_channel(&rig, "/portal/ws/entry/x", &cookie, &origin).err(),
        Some(503)
    );
}

#[test]
fn a_message_over_the_limit_ends_the_channel() {
    let rig = rig(|settings| settings.max_message = 1000, 0);
    let cookie = signed_in(&rig, "canoatesta");
    let mut socket = open_channel(&rig, "/portal/ws/entry/x", &cookie, &origin(&rig)).unwrap();
    assert_eq!(
        exchange(&mut socket, &[b'a'; 100]).len(),
        "canoatesta/entry:".len() + 100
    );
    let _ = socket.send(tungstenite::Message::Binary(vec![b'a'; 5000].into()));
    let ended = (0..40).any(|_| {
        thread::sleep(Duration::from_millis(100));
        socket.read().is_err()
    });
    assert!(ended, "a message over the limit was carried");
}

#[test]
fn the_browsers_key_is_introduced_to_its_users_host_only_and_waited_for() {
    let rig = rig(|_| {}, 2);
    let cookie = signed_in(&rig, "canoatesta");
    let origin = origin(&rig);
    let pair = |cookie: &str, origin: &str| {
        call(
            rig.addr,
            "POST",
            "/portal/pair",
            &[
                ("Cookie", cookie.to_owned()),
                ("Origin", origin.to_owned()),
                ("Content-Type", "application/json".to_owned()),
            ],
            r#"{"peer":"sha256:browserkey"}"#,
        )
    };
    assert_eq!(pair(&cookie, &origin).status, 200);
    assert_eq!(
        rig.hosts.introduced.lock().unwrap().as_slice(),
        ["canoatesta:sha256:browserkey"]
    );
    assert_eq!(pair("", &origin).status, 401);
    assert_eq!(pair(&cookie, "http://evil.example").status, 403);
    assert_eq!(rig.hosts.introduced.lock().unwrap().len(), 1);
}

#[test]
fn a_key_that_never_connects_is_not_introduced() {
    let rig = rig(
        |settings| settings.pair = Duration::from_millis(600),
        usize::MAX,
    );
    let cookie = signed_in(&rig, "canoatesta");
    let answer = call(
        rig.addr,
        "POST",
        "/portal/pair",
        &[("Cookie", cookie), ("Origin", origin(&rig))],
        r#"{"peer":"sha256:x"}"#,
    );
    assert_eq!(answer.status, 404);
    assert!(rig.hosts.introduced.lock().unwrap().is_empty());
}

#[test]
fn an_idle_host_is_stopped_and_comes_back_at_the_next_use_and_one_in_use_is_not() {
    let rig = rig(|settings| settings.idle = Duration::from_millis(300), 0);
    let cookie = signed_in(&rig, "canoatesta");
    let origin = origin(&rig);
    let mut socket = open_channel(&rig, "/portal/ws/entry/x", &cookie, &origin).unwrap();
    assert_eq!(exchange(&mut socket, b"1"), "canoatesta/entry:1");
    thread::sleep(Duration::from_millis(1500));
    assert_eq!(
        rig.hosts.stops.load(Ordering::SeqCst),
        0,
        "a host in use was stopped"
    );
    assert_eq!(exchange(&mut socket, b"2"), "canoatesta/entry:2");
    drop(socket);
    let stopped = (0..60).any(|_| {
        thread::sleep(Duration::from_millis(100));
        rig.hosts.stops.load(Ordering::SeqCst) == 1
    });
    assert!(stopped, "an idle host was not stopped");
    let mut again = open_channel(&rig, "/portal/ws/entry/x", &cookie, &origin).unwrap();
    assert_eq!(exchange(&mut again, b"3"), "canoatesta/entry:3");
    assert_eq!(rig.hosts.starts.load(Ordering::SeqCst), 2);
}

#[test]
fn the_unit_controller_takes_no_name_a_unit_is_not_made_of() {
    let systemd = Systemd {
        systemctl: PathBuf::from("/nonexistent/systemctl"),
        elevate: Vec::new(),
        unit: "canoa@{user}.service".to_owned(),
        ..Systemd::new()
    };
    for bad in ["", "a b", "a@b", "../x", "a/b", "-x", "a\nb", "1000", "0"] {
        let error = systemd.start(bad).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{bad:?}");
        assert_eq!(
            systemd.stop(bad).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }
    assert_ne!(
        systemd.start("canoatesta").unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
}

#[test]
fn start_and_stop_never_wait_for_a_password() {
    let dir = std::env::temp_dir().join(format!("canoa-portal-words-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("systemctl");
    std::fs::write(&script, "#!/bin/sh\necho \"$@\" > \"$0.said\"\n").unwrap();
    std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let systemd = Systemd {
        systemctl: script.clone(),
        elevate: Vec::new(),
        unit: "canoa@{user}.service".to_owned(),
        ..Systemd::new()
    };
    systemd.start("canoatesta").unwrap();
    let said = std::fs::read_to_string(dir.join("systemctl.said")).unwrap();
    assert_eq!(
        said.trim(),
        "start --no-ask-password canoa@canoatesta.service"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_running_hosts_are_the_users_named_in_the_units_systemctl_lists() {
    let dir = std::env::temp_dir().join(format!("canoa-portal-ctl-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("systemctl");
    std::fs::write(
        &script,
        "#!/bin/sh\ncat <<'LIST'\ncanoa@canoatesta.service loaded active running Canoa for canoatesta\ncanoa@bad@name.service loaded active running x\ncanoa@..%2Fx.service loaded active running x\ncanoa@1000.service loaded active running x\nother.service loaded active running x\nLIST\n",
    )
    .unwrap();
    std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    // Listing is its own program: the one that starts and stops may be a helper that takes no other words.
    let systemd = Systemd {
        systemctl: PathBuf::from("/nonexistent/helper"),
        list: script,
        ..Systemd::new()
    };
    assert_eq!(systemd.running().unwrap(), ["canoatesta"]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_portal_with_no_site_or_a_site_without_an_index_does_not_start() {
    let none = Portal::bind(
        Settings::default(),
        Arc::new(Fixed {
            accounts: Mutex::new(HashMap::new()),
            gone: Mutex::new(HashSet::new()),
            unsure: Mutex::new(HashSet::new()),
            asked: AtomicUsize::new(0),
        }),
        Arc::new(Systemd::new()),
    );
    assert!(matches!(none, Err(PortalError::Site(_))));
    let empty = std::env::temp_dir().join(format!("canoa-portal-empty-{}", std::process::id()));
    std::fs::create_dir_all(&empty).unwrap();
    let settings = Settings {
        site: Some(empty.clone()),
        ..Settings::default()
    };
    let missing = Portal::bind(
        settings,
        Arc::new(Fixed {
            accounts: Mutex::new(HashMap::new()),
            gone: Mutex::new(HashSet::new()),
            unsure: Mutex::new(HashSet::new()),
            asked: AtomicUsize::new(0),
        }),
        Arc::new(Systemd::new()),
    );
    assert!(matches!(missing, Err(PortalError::Site(_))));
    let _ = std::fs::remove_dir_all(empty);
}

#[test]
fn a_socket_another_users_process_made_is_never_dialed_for_a_person_whose_folder_leads_to_it() {
    let rig = rig(|_| {}, 0);
    rig.accounts
        .accounts
        .lock()
        .unwrap()
        .get_mut("canoatestb")
        .unwrap()
        .1 = mine() + 1;
    let a = signed_in(&rig, "canoatesta");
    let b = login(&rig, "canoatestb", "pw-b", "198.51.100.8")
        .cookie()
        .expect("a cookie");
    let origin = origin(&rig);
    let mut channel = open_channel(&rig, "/portal/ws/entry/x", &a, &origin).unwrap();
    assert_eq!(exchange(&mut channel, b"hello"), "canoatesta/entry:hello");
    assert_eq!(
        call(
            rig.addr,
            "GET",
            web::HOST_INDEX,
            &[("Cookie", b.clone())],
            ""
        )
        .status,
        503
    );
    assert_eq!(
        open_channel(&rig, "/portal/ws/entry/x", &b, &origin).err(),
        Some(503)
    );
    let pair = call(
        rig.addr,
        "POST",
        "/portal/pair",
        &[("Cookie", b), ("Origin", origin)],
        r#"{"peer":"sha256:x"}"#,
    );
    assert_eq!(pair.status, 503);
    assert!(rig.hosts.introduced.lock().unwrap().is_empty());
}

#[test]
fn the_rule_the_guide_gives_polkit_is_the_validators_pattern() {
    // Names that the pattern's first character or length refuses are refused by the validator too, and the other way round.
    for (name, valid) in [
        ("a", true),
        ("_", true),
        ("Zed.x-y_9", true),
        ("9a", false),
        ("1000", false),
        (".a", false),
        ("-a", false),
        ("a@b", false),
    ] {
        assert_eq!(valid_user(name), valid, "{name}");
    }
}

#[test]
fn the_root_helper_refuses_what_the_validator_does_and_every_account_below_the_lowest_user_id() {
    let helper = format!("{}/dist/canoa-instance", env!("CARGO_MANIFEST_DIR"));
    for words in [
        ["start", "canoa@root.service"],
        ["start", "canoa@daemon.service"],
        ["start", "canoa@1000.service"],
        ["start", "canoa@0.service"],
        ["start", "canoa@a@b.service"],
        ["start", "canoa@.service"],
        ["start", "canoa@a/b.service"],
        ["start", "other@alice.service"],
        ["restart", "canoa@alice.service"],
        ["start", "canoa@-x.service"],
    ] {
        let status = std::process::Command::new("sh")
            .arg(&helper)
            .args(words)
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(2), "{words:?}");
    }
}

#[test]
fn a_session_stands_while_the_directory_cannot_say_and_ends_only_when_it_says_gone() {
    let rig = rig(|_| {}, 0);
    let cookie = signed_in(&rig, "canoatesta");
    rig.accounts
        .unsure
        .lock()
        .unwrap()
        .insert("canoatesta".to_owned());
    let index = |cookie: &str| {
        call(
            rig.addr,
            "GET",
            web::HOST_INDEX,
            &[("Cookie", cookie.to_owned())],
            "",
        )
        .status
    };
    assert_eq!(
        index(&cookie),
        200,
        "a directory that does not answer signs nobody out"
    );
    rig.accounts
        .gone
        .lock()
        .unwrap()
        .insert("canoatesta".to_owned());
    assert_eq!(index(&cookie), 401);
}

#[test]
fn a_call_to_systemctl_that_never_ends_is_ended() {
    let dir = std::env::temp_dir().join(format!("canoa-portal-slow-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("systemctl");
    std::fs::write(&script, "#!/bin/sh\nexec sleep 30\n").unwrap();
    std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let systemd = Systemd {
        systemctl: script,
        timeout: Duration::from_millis(400),
        ..Systemd::new()
    };
    let started = std::time::Instant::now();
    let error = systemd.start("canoatesta").unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_portal_that_starts_again_adopts_the_hosts_already_running_and_stops_them_when_idle() {
    let first = rig(|_| {}, 0);
    let cookie = signed_in(&first, "canoatesta");
    assert_eq!(
        call(
            first.addr,
            "GET",
            web::HOST_INDEX,
            &[("Cookie", cookie)],
            ""
        )
        .status,
        200
    );
    first.stopper.stop();
    thread::sleep(Duration::from_millis(200));
    // A second portal, on the same machine's units, with nobody using the host that is up.
    let base = first.dir.parent().unwrap().to_path_buf();
    let settings = Settings {
        socket: Some(base.join("front").join("second.sock")),
        listen: None,
        site: Some(first.root.clone()),
        socket_dir: first.dir.clone(),
        idle: Duration::from_millis(400),
        failure_floor: Duration::ZERO,
        ..Settings::default()
    };
    let second = Portal::bind(settings, first.accounts.clone(), first.hosts.clone()).unwrap();
    let stopper = second.stopper();
    thread::spawn(move || second.run());
    let stopped = (0..60).any(|_| {
        thread::sleep(Duration::from_millis(100));
        first.hosts.stops.load(Ordering::SeqCst) == 1
    });
    stopper.stop();
    assert!(
        stopped,
        "a host that was up when the portal started was never stopped"
    );
}
