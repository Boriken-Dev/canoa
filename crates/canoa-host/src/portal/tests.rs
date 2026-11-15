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
    /// The certificate header value that is the named account's certificate.
    certificates: Mutex<HashMap<String, String>>,
    /// The address the last certificate sign-in was asked for.
    last_client: Mutex<Option<IpAddr>>,
    /// The headers each certificate sign-in was asked with.
    presented: Mutex<Vec<Vec<(String, String)>>>,
}

/// This process's user id: the stand-in hosts run in it, so it is whose process made their sockets.
fn mine() -> u32 {
    std::os::unix::fs::MetadataExt::uid(&std::fs::metadata("/proc/self").unwrap())
}

impl Accounts for Fixed {
    fn methods(&self) -> Vec<Method> {
        vec![Method::Password, Method::Certificate]
    }

    fn certificate_status(
        &self,
        user: &str,
        _uid: u32,
        headers: &[(String, String)],
    ) -> Option<CertificateStatus> {
        let pins = self.certificates.lock().unwrap();
        let presented = headers
            .iter()
            .find(|(name, _)| name == "x-client-cert")
            .map(|(_, cert)| {
                let subject = if cert == "MIIB-xss" {
                    "<script>alert(1)</script>".to_owned()
                } else {
                    format!("CN={user}")
                };
                let state = match pins.get(cert) {
                    Some(owner) if owner == user => PinState::Yours,
                    Some(_) => PinState::Another,
                    None => PinState::Unpinned,
                };
                (
                    Presented {
                        subject,
                        fingerprint: format!("sha256:{cert}"),
                        expired: cert == "MIIB-expired",
                    },
                    state,
                )
            });
        let pinned = pins.values().any(|owner| owner == user);
        Some(CertificateStatus {
            presented,
            pinned,
            disabled: false,
        })
    }

    fn pin(&self, user: &str, _uid: u32, headers: &[(String, String)]) -> Result<(), PinError> {
        let cert = headers
            .iter()
            .find(|(name, _)| name == "x-client-cert")
            .map(|(_, cert)| cert.clone())
            .ok_or(PinError::NoCertificate)?;
        let mut pins = self.certificates.lock().unwrap();
        if pins.get(&cert).is_some_and(|owner| owner != user) {
            return Err(PinError::Taken);
        }
        pins.retain(|_, owner| owner != user);
        pins.insert(cert, user.to_owned());
        Ok(())
    }

    fn unpin(&self, user: &str) -> Result<bool, PinError> {
        let mut pins = self.certificates.lock().unwrap();
        let before = pins.len();
        pins.retain(|_, owner| owner != user);
        Ok(pins.len() != before)
    }

    fn sign_in_certificate(
        &self,
        headers: &[(String, String)],
        client: IpAddr,
        _providers: &[String],
    ) -> SignIn {
        self.asked.fetch_add(1, Ordering::SeqCst);
        *self.last_client.lock().unwrap() = Some(client);
        self.presented.lock().unwrap().push(headers.to_vec());
        let value = headers
            .iter()
            .find(|(name, _)| name == "x-client-cert")
            .map(|(_, value)| value.as_str());
        let user = value.and_then(|value| self.certificates.lock().unwrap().get(value).cloned());
        match user.and_then(|user| {
            let uid = self
                .accounts
                .lock()
                .unwrap()
                .get(&user)
                .map(|(_, uid)| *uid)?;
            Some((user, uid))
        }) {
            Some((user, uid)) => SignIn::Admitted {
                user,
                uid,
                groups: Vec::new(),
                provider: "stand-in".to_owned(),
            },
            None => SignIn::Refused,
        }
    }

    fn sign_in(&self, user: &str, password: &str, _providers: &[String]) -> SignIn {
        self.asked.fetch_add(1, Ordering::SeqCst);
        if password == "FAIL" {
            return SignIn::Failed("the directory is down".to_owned());
        }
        match self.accounts.lock().unwrap().get(user) {
            Some((expected, uid)) if expected == password => SignIn::Admitted {
                user: user.to_owned(),
                uid: *uid,
                groups: Vec::new(),
                provider: "stand-in".to_owned(),
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
            Standing::Present(Vec::new())
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

struct Rig<A = Fixed> {
    addr: SocketAddr,
    sock: PathBuf,
    accounts: Arc<A>,
    hosts: Arc<Hosts>,
    stopper: Stopper,
    inner: Arc<Inner>,
    dir: PathBuf,
    root: PathBuf,
}

impl<A> Drop for Rig<A> {
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
        certificates: Mutex::new(
            [("MIIB-test-cert".to_owned(), "canoatesta".to_owned())]
                .into_iter()
                .collect(),
        ),
        presented: Mutex::new(Vec::new()),
        last_client: Mutex::new(None),
    });
    rig_with(accounts, change, refuse_accepts)
}

/// A portal over `accounts` and stand-in hosts, on a Unix socket for the proxy and a loopback address.
fn rig_with<A: Accounts + 'static>(
    accounts: Arc<A>,
    change: impl FnOnce(&mut Settings),
    refuse_accepts: usize,
) -> Rig<A> {
    rig_policy(accounts, change, refuse_accepts, None)
}

/// [`rig_with`] with a sign-in policy of the test's own, when it gives one.
fn rig_policy<A: Accounts + 'static>(
    accounts: Arc<A>,
    change: impl FnOnce(&mut Settings),
    refuse_accepts: usize,
    policy: Option<policy::Policy>,
) -> Rig<A> {
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
    let portal = match policy {
        Some(policy) => {
            settings.check().unwrap();
            Portal::bind_with_policy(settings, accounts.clone(), hosts.clone(), policy)
        }
        None => Portal::bind(settings, accounts.clone(), hosts.clone()),
    }
    .unwrap();
    let addr = portal.local_addr().unwrap();
    let stopper = portal.stopper();
    let inner = Arc::clone(&portal.inner);
    thread::spawn(move || portal.run());
    Rig {
        addr,
        sock,
        accounts,
        hosts,
        stopper,
        inner,
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
fn the_manifest_the_favicon_and_the_icons_are_given_to_a_browser_that_has_not_signed_in() {
    // A browser fetches the manifest with no cookie, so the sign-in must not gate what the page links in its head.
    let rig = rig(|_| {}, 0);
    std::fs::create_dir_all(rig.root.join("icons")).unwrap();
    std::fs::write(rig.root.join("manifest.json"), "{}").unwrap();
    std::fs::write(rig.root.join("favicon.ico"), "ico").unwrap();
    std::fs::write(rig.root.join("icons").join("Icon-192.png"), "png").unwrap();
    std::fs::write(rig.root.join("main.dart.js"), "code").unwrap();
    for path in ["/manifest.json", "/favicon.ico", "/icons/Icon-192.png"] {
        let answer = call(rig.addr, "GET", path, &[], "");
        assert_eq!(answer.status, 200, "{path}");
        assert_eq!(answer.header("cache-control"), Some("no-cache"), "{path}");
    }
    assert_eq!(
        call(rig.addr, "GET", "/icons/none.png", &[], "").status,
        404
    );
    // Nothing else of the site is.
    for path in ["/main.dart.js", "/index.html", "/icons", "/icons/x/y"] {
        let answer = call(rig.addr, "GET", path, &[], "");
        assert_eq!(
            (answer.status, answer.header("location")),
            (303, Some(LOGIN)),
            "{path}"
        );
    }
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
            certificates: Mutex::new(HashMap::new()),
            presented: Mutex::new(Vec::new()),
            last_client: Mutex::new(None),
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
            certificates: Mutex::new(HashMap::new()),
            presented: Mutex::new(Vec::new()),
            last_client: Mutex::new(None),
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

#[test]
fn the_site_behind_the_sign_in_is_kept_private_and_asked_about_with_its_tag() {
    let rig = rig(|_| {}, 0);
    std::fs::create_dir_all(rig.root.join("canvaskit")).unwrap();
    std::fs::write(rig.root.join("main.dart.js"), "code").unwrap();
    std::fs::write(rig.root.join("main.3f9a1b2c.js"), "hashed").unwrap();
    std::fs::write(rig.root.join("canvaskit").join("canvaskit.wasm"), "wasm").unwrap();
    let cookie = signed_in(&rig, "canoatesta");
    let with = |path: &str, extra: &[(&str, String)]| {
        let mut headers = vec![("Cookie", cookie.clone())];
        headers.extend(extra.iter().cloned());
        call(rig.addr, "GET", path, &headers, "")
    };
    for (path, cache) in [
        ("/", "private, no-cache"),
        ("/main.dart.js", "private, no-cache"),
        ("/canvaskit/canvaskit.wasm", "private, no-cache"),
        ("/main.3f9a1b2c.js", "private, max-age=31536000, immutable"),
    ] {
        let answer = with(path, &[]);
        assert_eq!(answer.status, 200, "{path}");
        assert_eq!(answer.header("cache-control"), Some(cache), "{path}");
        assert!(answer.header("last-modified").is_some(), "{path}");
    }
    let first = with("/main.dart.js", &[]);
    let tag = first.header("etag").unwrap().to_owned();
    assert!(!tag.starts_with("W/"), "{tag}");
    let again = with("/main.dart.js", &[("If-None-Match", tag.clone())]);
    assert_eq!(again.status, 304);
    assert!(again.body.is_empty());
    assert_eq!(again.header("etag"), Some(tag.as_str()));
    assert_eq!(again.header("cache-control"), Some("private, no-cache"));
    assert_eq!(again.header("content-length"), None);

    std::fs::write(rig.root.join("main.dart.js"), "new code, longer").unwrap();
    let changed = with("/main.dart.js", &[("If-None-Match", tag.clone())]);
    assert_eq!(changed.status, 200);
    assert_eq!(changed.body, "new code, longer");
    assert_ne!(changed.header("etag"), Some(tag.as_str()));

    // Without the sign-in the tag buys nothing: the sign-in comes first.
    let anonymous = call(
        rig.addr,
        "GET",
        "/main.dart.js",
        &[("If-None-Match", tag)],
        "",
    );
    assert_eq!(anonymous.status, 303);
}

/// A certificate sign-in as the proxy makes it, carrying `cert` in the header the portal reads, on the Unix socket.
fn cert_login(rig: &Rig, cert: Option<&str>, forwarded: &str) -> Answer {
    let to = To::Unix(rig.sock.clone());
    let mut extra = vec![
        ("Origin", to.origin()),
        (
            "Content-Type",
            "application/x-www-form-urlencoded".to_owned(),
        ),
        ("X-Forwarded-For", forwarded.to_owned()),
        ("Cookie", "__Host-canoa=not-a-session".to_owned()),
        ("Authorization", "Bearer secret".to_owned()),
    ];
    if let Some(cert) = cert {
        extra.push(("X-Client-Cert", cert.to_owned()));
    }
    request(&to, "POST", LOGIN, &extra, "method=certificate")
}

#[test]
fn the_methods_list_what_this_connection_may_use_and_needs_no_cookie() {
    let rig = rig(|_| {}, 0);
    let proxied = request(&To::Unix(rig.sock.clone()), "GET", METHODS, &[], "");
    assert_eq!(proxied.status, 200);
    let list: serde_json::Value = serde_json::from_str(&proxied.body).unwrap();
    assert_eq!(
        list,
        serde_json::json!({"methods": ["password", "certificate"]})
    );
    assert_eq!(proxied.header("cache-control"), Some("no-store"));
    // A header on the loopback address is anyone's: a certificate is not offered there.
    let local = call(rig.addr, "GET", METHODS, &[], "");
    let list: serde_json::Value = serde_json::from_str(&local.body).unwrap();
    assert_eq!(list, serde_json::json!({"methods": ["password"]}));
}

#[test]
fn the_page_offers_the_methods_and_runs_nothing_inline() {
    let rig = rig(|_| {}, 0);
    let proxied = request(&To::Unix(rig.sock.clone()), "GET", LOGIN, &[], "");
    assert!(proxied.body.contains("name=\"password\""));
    assert!(proxied.body.contains("Sign in with your certificate"));
    let local = call(rig.addr, "GET", LOGIN, &[], "");
    assert!(local.body.contains("name=\"password\""));
    assert!(!local.body.contains("certificate"));
    let policy = proxied.header("content-security-policy").unwrap();
    assert!(policy.contains("script-src 'self'") && policy.contains("style-src 'self'"));
    assert!(!policy.contains("unsafe-inline"));
    for body in [&proxied.body, &local.body] {
        assert!(!body.contains("style=") && !body.contains("<style") && !body.contains(" onclick"));
        assert_eq!(body.matches("<script").count(), 1);
        assert!(body.contains("src=\"/portal/login.js\""));
    }
    for (path, kind) in [
        ("/portal/login.css", "text/css"),
        ("/portal/login.js", "text/javascript"),
    ] {
        let asset = call(rig.addr, "GET", path, &[], "");
        assert_eq!(asset.status, 200);
        assert!(asset.header("content-type").unwrap().starts_with(kind));
    }
}

#[test]
fn a_certificate_from_the_proxy_signs_in_and_the_provider_sees_no_cookie() {
    let rig = rig(|_| {}, 0);
    let answer = cert_login(&rig, Some("MIIB-test-cert"), "198.51.100.7");
    assert_eq!((answer.status, answer.header("location")), (303, Some("/")));
    let cookie = answer.cookie().expect("a session cookie");
    let home = call(rig.addr, "GET", "/", &[("Cookie", cookie)], "");
    assert_eq!(home.status, 200);
    let seen = rig.accounts.presented.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert!(seen[0].iter().any(|(name, _)| name == "x-client-cert"));
    assert!(
        !seen[0]
            .iter()
            .any(|(name, _)| name == "cookie" || name == "authorization")
    );
}

#[test]
fn a_certificate_is_refused_on_the_loopback_address_whatever_header_it_carries() {
    let rig = rig(|_| {}, 0);
    let answer = call(
        rig.addr,
        "POST",
        LOGIN,
        &[
            ("Origin", origin(&rig)),
            (
                "Content-Type",
                "application/x-www-form-urlencoded".to_owned(),
            ),
            ("X-Client-Cert", "MIIB-test-cert".to_owned()),
        ],
        "method=certificate",
    );
    assert_eq!(answer.status, 403);
    assert!(answer.cookie().is_none());
    assert_eq!(rig.accounts.asked.load(Ordering::SeqCst), 0);
}

#[test]
fn an_unknown_certificate_reads_as_refused_and_the_throttle_holds_every_method() {
    let rig = rig(|settings| settings.max_failures_per_pair = 2, 0);
    for _ in 0..2 {
        let answer = cert_login(&rig, Some("MIIB-other"), "203.0.113.50");
        assert_eq!(answer.status, 401);
        assert!(answer.body.contains("That certificate was not accepted"));
        assert!(answer.cookie().is_none());
    }
    // No certificate at all is a failure of the same kind, and past the limit even the right one is not asked of the accounts.
    let blocked = cert_login(&rig, Some("MIIB-test-cert"), "203.0.113.50");
    assert_eq!(blocked.status, 429);
    assert_eq!(blocked.header("retry-after"), Some("900"));
    assert!(blocked.body.contains("data-retry=\"900\""));
    assert_eq!(rig.accounts.asked.load(Ordering::SeqCst), 2);
    // The password form from that address still has its own count, and another address its own.
    assert_eq!(
        login(&rig, "canoatesta", "pw-a", "203.0.113.50").status,
        303
    );
    assert_eq!(cert_login(&rig, None, "203.0.113.51").status, 401);
}

#[test]
fn a_certificate_for_an_account_the_portal_does_not_admit_is_refused() {
    let rig = rig(|_| {}, 0);
    rig.accounts
        .certificates
        .lock()
        .unwrap()
        .insert("MIIB-root".to_owned(), "rootish".to_owned());
    rig.accounts
        .certificates
        .lock()
        .unwrap()
        .insert("MIIB-low".to_owned(), "lowuid".to_owned());
    for cert in ["MIIB-root", "MIIB-low"] {
        let answer = cert_login(&rig, Some(cert), "203.0.113.60");
        assert_eq!(answer.status, 401);
        assert!(answer.cookie().is_none());
    }
}

/// A request to the certificate page or its actions, with `cookie`, as the proxy makes it, carrying `cert`.
fn cert_page(rig: &Rig, method: &str, cookie: &str, cert: Option<&str>, body: &str) -> Answer {
    let to = To::Unix(rig.sock.clone());
    let mut extra = vec![
        ("Origin", to.origin()),
        (
            "Content-Type",
            "application/x-www-form-urlencoded".to_owned(),
        ),
        ("X-Forwarded-For", "198.51.100.7".to_owned()),
        ("Cookie", cookie.to_owned()),
    ];
    if let Some(cert) = cert {
        extra.push(("X-Client-Cert", cert.to_owned()));
    }
    request(&to, method, CERTIFICATE, &extra, body)
}

fn cert_session(rig: &Rig) -> String {
    cert_login(rig, Some("MIIB-test-cert"), "198.51.100.8")
        .cookie()
        .expect("a certificate session")
}

#[test]
fn a_person_signed_in_by_password_with_a_new_certificate_is_offered_it_and_pinning_it_lets_it_sign_in()
 {
    let rig = rig(|_| {}, 0);
    let to = To::Unix(rig.sock.clone());
    // The password sign-in carries the browser's certificate: the person is taken to the page that offers it.
    let answer = request(
        &to,
        "POST",
        LOGIN,
        &[
            ("Origin", to.origin()),
            (
                "Content-Type",
                "application/x-www-form-urlencoded".to_owned(),
            ),
            ("X-Forwarded-For", "198.51.100.7".to_owned()),
            ("X-Client-Cert", "MIIB-new".to_owned()),
        ],
        "user=canoatestb&password=pw-b",
    );
    assert_eq!(
        (answer.status, answer.header("location")),
        (303, Some(CERTIFICATE))
    );
    let cookie = answer.cookie().unwrap();
    let page = cert_page(&rig, "GET", &cookie, Some("MIIB-new"), "");
    assert_eq!(page.status, 200);
    assert!(page.body.contains("Use this certificate to sign in"));
    assert!(page.body.contains("CN=canoatestb") && page.body.contains("sha256:MIIB-new"));
    assert_eq!(
        cert_login(&rig, Some("MIIB-new"), "198.51.100.9").status,
        401
    );
    let done = cert_page(&rig, "POST", &cookie, Some("MIIB-new"), "action=enrol");
    assert_eq!(done.status, 200);
    assert!(done.body.contains("now signs you in"));
    // The certificate alone signs the person in, as themselves.
    let alone = cert_login(&rig, Some("MIIB-new"), "198.51.100.10");
    assert_eq!(alone.status, 303);
    let session = call(
        rig.addr,
        "GET",
        "/portal/session",
        &[("Cookie", alone.cookie().unwrap())],
        "",
    );
    assert!(session.body.contains("canoatestb"));
    // A password sign-in without a certificate, or on the loopback address, goes straight in.
    assert_eq!(
        login(&rig, "canoatesta", "pw-a", "198.51.100.7").header("location"),
        Some("/")
    );
}

#[test]
fn pinning_needs_a_fresh_password_sign_in_in_the_same_session() {
    let rig = rig(|_| {}, 0);
    // A session a certificate opened never pins, however recent.
    let by_certificate = cert_session(&rig);
    let refused = cert_page(
        &rig,
        "POST",
        &by_certificate,
        Some("MIIB-new"),
        "action=enrol",
    );
    assert_eq!(refused.status, 403);
    assert!(refused.body.contains("Enter your password"));
    assert!(
        !rig.accounts
            .certificates
            .lock()
            .unwrap()
            .contains_key("MIIB-new")
    );
    // The page asks for the password with the button.
    let page = cert_page(&rig, "GET", &by_certificate, Some("MIIB-new"), "");
    assert!(page.body.contains("name=\"password\""));
    // A password session whose time is up does not either.
    let stale = rig_with_enrol_window_zero();
    let cookie = signed_in(&stale, "canoatesta");
    std::thread::sleep(Duration::from_millis(5));
    let late = cert_page(&stale, "POST", &cookie, Some("MIIB-new"), "action=enrol");
    assert_eq!(late.status, 403);
    assert!(
        !stale
            .accounts
            .certificates
            .lock()
            .unwrap()
            .contains_key("MIIB-new")
    );
    // Nor does a request with no session, or from another site's page.
    let to = To::Unix(rig.sock.clone());
    let anonymous = request(
        &to,
        "POST",
        CERTIFICATE,
        &[
            ("Origin", to.origin()),
            (
                "Content-Type",
                "application/x-www-form-urlencoded".to_owned(),
            ),
        ],
        "action=enrol",
    );
    assert_eq!(anonymous.status, 401);
    let password = signed_in(&rig, "canoatesta");
    let cross = request(
        &to,
        "POST",
        CERTIFICATE,
        &[
            ("Origin", "http://evil.example".to_owned()),
            (
                "Content-Type",
                "application/x-www-form-urlencoded".to_owned(),
            ),
            ("Cookie", password),
            ("X-Client-Cert", "MIIB-new".to_owned()),
        ],
        "action=enrol",
    );
    assert_eq!(cross.status, 403);
    assert!(
        !rig.accounts
            .certificates
            .lock()
            .unwrap()
            .contains_key("MIIB-new")
    );
}

fn rig_with_enrol_window_zero() -> Rig {
    rig(|settings| settings.enrol = Duration::ZERO, 0)
}

#[test]
fn a_certificate_pinned_to_another_account_cannot_be_enrolled() {
    let rig = rig(|_| {}, 0);
    // MIIB-test-cert is canoatesta's.
    let cookie = signed_in(&rig, "canoatestb");
    let page = cert_page(&rig, "GET", &cookie, Some("MIIB-test-cert"), "");
    assert!(page.body.contains("already used by another account"));
    assert!(!page.body.contains("Use this certificate to sign in"));
    let refused = cert_page(
        &rig,
        "POST",
        &cookie,
        Some("MIIB-test-cert"),
        "action=enrol&password=pw-b",
    );
    assert!(refused.body.contains("belongs to another account"));
    assert_eq!(
        rig.accounts
            .certificates
            .lock()
            .unwrap()
            .get("MIIB-test-cert")
            .map(String::as_str),
        Some("canoatesta")
    );
}

#[test]
fn a_pinned_certificate_is_shown_and_can_be_removed_and_then_signs_no_one_in() {
    let rig = rig(|_| {}, 0);
    let cookie = signed_in(&rig, "canoatesta");
    let page = cert_page(&rig, "GET", &cookie, Some("MIIB-test-cert"), "");
    assert!(page.body.contains("signs you in") && page.body.contains("A certificate is pinned"));
    let removed = cert_page(
        &rig,
        "POST",
        &cookie,
        Some("MIIB-test-cert"),
        "action=remove",
    );
    assert_eq!(removed.status, 200);
    assert!(
        removed.body.contains("is removed") && removed.body.contains("No certificate is pinned")
    );
    assert_eq!(
        cert_login(&rig, Some("MIIB-test-cert"), "198.51.100.11").status,
        401
    );
    // Removing is for the signed-in person's own pin only.
    let other = signed_in(&rig, "canoatestb");
    rig.accounts
        .certificates
        .lock()
        .unwrap()
        .insert("MIIB-b".to_owned(), "canoatestb".to_owned());
    cert_page(&rig, "POST", &cookie, None, "action=remove");
    assert!(
        rig.accounts
            .certificates
            .lock()
            .unwrap()
            .contains_key("MIIB-b")
    );
    drop(other);
}

#[test]
fn a_certificates_subject_is_escaped_on_the_page() {
    let rig = rig(|_| {}, 0);
    let cookie = signed_in(&rig, "canoatesta");
    let page = cert_page(&rig, "GET", &cookie, Some("MIIB-xss"), "");
    assert!(page.body.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    assert!(!page.body.contains("<script>alert"));
}

#[test]
fn a_fresh_password_session_cannot_pin_a_certificate_other_than_the_one_it_signed_in_with() {
    let rig = rig(|settings| settings.max_failures_per_pair = 3, 0);
    let to = To::Unix(rig.sock.clone());
    let signed = request(
        &to,
        "POST",
        LOGIN,
        &[
            ("Origin", to.origin()),
            (
                "Content-Type",
                "application/x-www-form-urlencoded".to_owned(),
            ),
            ("X-Forwarded-For", "198.51.100.7".to_owned()),
            ("X-Client-Cert", "MIIB-mine".to_owned()),
        ],
        "user=canoatestb&password=pw-b",
    );
    let cookie = signed.cookie().unwrap();
    // A thief holding the cookie, with a certificate of their own, within the fresh window.
    let pinned = |cert: &str| rig.accounts.certificates.lock().unwrap().contains_key(cert);
    let page = cert_page(&rig, "GET", &cookie, Some("MIIB-thief"), "");
    assert!(page.body.contains("name=\"password\""));
    let without = cert_page(&rig, "POST", &cookie, Some("MIIB-thief"), "action=enrol");
    assert_eq!(without.status, 403);
    assert!(without.body.contains("Enter your password"));
    assert!(!pinned("MIIB-thief"));
    // Guessing the password at the form goes through the throttle: three wrong ones and the fourth is not asked.
    for _ in 0..3 {
        let wrong = cert_page(
            &rig,
            "POST",
            &cookie,
            Some("MIIB-thief"),
            "action=enrol&password=nope",
        );
        assert!(wrong.body.contains("That password was not accepted"));
    }
    let asked = rig.accounts.asked.load(Ordering::SeqCst);
    let blocked = cert_page(
        &rig,
        "POST",
        &cookie,
        Some("MIIB-thief"),
        "action=enrol&password=pw-b",
    );
    assert!(blocked.body.contains("Too many failed attempts"));
    assert_eq!(rig.accounts.asked.load(Ordering::SeqCst), asked);
    assert!(!pinned("MIIB-thief"));
    // The certificate the password was typed with pins with no second password.
    let own = cert_page(&rig, "POST", &cookie, Some("MIIB-mine"), "action=enrol");
    assert_eq!(own.status, 200);
    assert!(pinned("MIIB-mine"));
}

#[test]
fn a_person_who_types_the_password_again_may_pin_the_certificate_on_offer() {
    let rig = rig(|_| {}, 0);
    let cookie = signed_in(&rig, "canoatestb");
    let done = cert_page(
        &rig,
        "POST",
        &cookie,
        Some("MIIB-card"),
        "action=enrol&password=pw-b",
    );
    assert_eq!(done.status, 200);
    assert_eq!(
        rig.accounts
            .certificates
            .lock()
            .unwrap()
            .get("MIIB-card")
            .map(String::as_str),
        Some("canoatestb")
    );
    // Another account's password does not do.
    let other = cert_page(
        &rig,
        "POST",
        &cookie,
        Some("MIIB-card-2"),
        "action=enrol&password=pw-a",
    );
    assert!(other.body.contains("That password was not accepted"));
    assert!(
        !rig.accounts
            .certificates
            .lock()
            .unwrap()
            .contains_key("MIIB-card-2")
    );
}

#[test]
fn what_a_removed_or_replaced_pin_let_in_does_not_outlive_it() {
    let rig = rig(|_| {}, 0);
    // A certificate session of canoatesta, whose pin is MIIB-test-cert.
    let by_certificate = cert_session(&rig);
    let session = |cookie: &String| {
        call(
            rig.addr,
            "GET",
            "/portal/session",
            &[("Cookie", cookie.clone())],
            "",
        )
        .status
    };
    assert_eq!(session(&by_certificate), 200);
    // The account pins another certificate, from a password session: the old pin's session ends.
    let to = To::Unix(rig.sock.clone());
    let signed = request(
        &to,
        "POST",
        LOGIN,
        &[
            ("Origin", to.origin()),
            (
                "Content-Type",
                "application/x-www-form-urlencoded".to_owned(),
            ),
            ("X-Forwarded-For", "198.51.100.7".to_owned()),
            ("X-Client-Cert", "MIIB-renewed".to_owned()),
        ],
        "user=canoatesta&password=pw-a",
    );
    let by_password = signed.cookie().unwrap();
    assert_eq!(session(&by_certificate), 200, "signing in changes nothing");
    cert_page(
        &rig,
        "POST",
        &by_password,
        Some("MIIB-renewed"),
        "action=enrol",
    );
    assert_eq!(session(&by_certificate), 401);
    assert_eq!(session(&by_password), 200);
    // A session the new pin opened ends when the pin is removed.
    let renewed = cert_login(&rig, Some("MIIB-renewed"), "198.51.100.12")
        .cookie()
        .unwrap();
    assert_eq!(session(&renewed), 200);
    cert_page(&rig, "POST", &by_password, None, "action=remove");
    assert_eq!(session(&renewed), 401);
    assert_eq!(session(&by_password), 200);
}

#[test]
fn an_expired_certificate_is_said_to_be_and_not_offered_for_pinning() {
    let rig = rig(|_| {}, 0);
    let cookie = signed_in(&rig, "canoatestb");
    let page = cert_page(&rig, "GET", &cookie, Some("MIIB-expired"), "");
    assert!(page.body.contains("has expired"));
    assert!(!page.body.contains("Use this certificate to sign in"));
}

#[test]
fn the_certificate_sign_in_is_asked_for_the_browsers_address_not_the_proxys() {
    let rig = rig(|_| {}, 0);
    cert_login(&rig, Some("MIIB-test-cert"), "198.51.100.77");
    assert_eq!(
        *rig.accounts.last_client.lock().unwrap(),
        Some("198.51.100.77".parse().unwrap())
    );
}

mod sign_in_policy;

#[test]
fn no_answer_carries_a_header_twice() {
    let rig = rig(|_| {}, 0);
    let to = To::Unix(rig.sock.clone());
    let cookie = signed_in(&rig, "canoatesta");
    for (path, cookie) in [
        (LOGIN, ""),
        (METHODS, ""),
        ("/portal/login.css", ""),
        ("/portal/login.js", ""),
        (CERTIFICATE, cookie.as_str()),
    ] {
        let extra: Vec<(&str, String)> = if cookie.is_empty() {
            vec![]
        } else {
            vec![("Cookie", cookie.to_owned())]
        };
        let answer = request(&to, "GET", path, &extra, "");
        let count = |name: &str| {
            answer
                .headers
                .iter()
                .filter(|(each, _)| each.eq_ignore_ascii_case(name))
                .count()
        };
        for name in [
            "cache-control",
            "content-security-policy",
            "content-type",
            "referrer-policy",
        ] {
            assert!(
                count(name) <= 1,
                "{path}: {name} sent {} times",
                count(name)
            );
        }
        assert_eq!(count("cache-control"), 1, "{path}");
    }
}
