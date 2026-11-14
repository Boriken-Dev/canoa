//! The portal against stand-in accounts and a stand-in for each user's host: the sign-in, the cookie, the throttle, the relay and
//! the idle stop. The real accounts and real hosts are in tests/portal_users.rs.

use alloc::sync::Arc;
use core::net::SocketAddr;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use core::time::Duration;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::thread;

use coqui_transport::Channel;
use coqui_transport_pipe::{Listener, PipeName};

use super::*;

/// Accounts that are `name:password` pairs; counts each time it is asked.
struct Fixed {
    /// Each name's password and user id.
    accounts: Mutex<HashMap<String, (String, u32)>>,
    asked: AtomicUsize,
}

/// This process's user id: the stand-in hosts run in it, so it is whose process made their sockets.
fn mine() -> u32 {
    std::os::unix::fs::MetadataExt::uid(&std::fs::metadata("/proc/self").unwrap())
}

impl Accounts for Fixed {
    fn sign_in(&self, user: &str, password: &str) -> SignIn {
        self.asked.fetch_add(1, Ordering::SeqCst);
        match self.accounts.lock().unwrap().get(user) {
            Some((expected, uid)) if expected == password => SignIn::Admitted {
                user: user.to_owned(),
                uid: *uid,
            },
            _ => SignIn::Refused,
        }
    }
}

/// A host for each user that starts when asked: an introduction socket and one entry, `x`, that answers each message with the
/// user's name and the message.
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
        let introduction =
            Listener::bind(&PipeName::at(home.join("portal.sock")).unwrap()).unwrap();
        let entry =
            Listener::bind(&PipeName::at(home.join("entries").join("x.sock")).unwrap()).unwrap();
        let control = Listener::bind(&PipeName::at(home.join("control.sock")).unwrap()).unwrap();
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
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("index.html"), "<!doctype html>the app").unwrap();
    let accounts = Arc::new(Fixed {
        accounts: Mutex::new(
            [("canoatesta", "pw-a"), ("canoatestb", "pw-b")]
                .into_iter()
                .map(|(name, password)| (name.to_owned(), (password.to_owned(), mine())))
                .collect(),
        ),
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
        listen: "127.0.0.1:0".parse().unwrap(),
        site: Some(root.clone()),
        socket_dir: dir.clone(),
        pair: Duration::from_secs(3),
        ..Settings::default()
    };
    change(&mut settings);
    let portal = Portal::bind(settings, accounts.clone(), hosts.clone()).unwrap();
    let addr = portal.local_addr().unwrap();
    let stopper = portal.stopper();
    thread::spawn(move || portal.run());
    Rig {
        addr,
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

fn call(addr: SocketAddr, method: &str, path: &str, extra: &[(&str, &str)], body: &str) -> Answer {
    let mut stream = TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut text = format!("{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n");
    for (name, value) in extra {
        text.push_str(&format!("{name}: {value}\r\n"));
    }
    if !body.is_empty() {
        text.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    text.push_str("\r\n");
    text.push_str(body);
    stream.write_all(text.as_bytes()).unwrap();
    let mut raw = Vec::new();
    let _ = stream.read_to_end(&mut raw);
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

fn login(rig: &Rig, user: &str, password: &str, forwarded: &str) -> Answer {
    let origin = format!("http://{}", rig.addr);
    call(
        rig.addr,
        "POST",
        LOGIN,
        &[
            ("Origin", origin.as_str()),
            ("Content-Type", "application/x-www-form-urlencoded"),
            ("X-Forwarded-For", forwarded),
        ],
        &format!("user={user}&password={password}"),
    )
}

fn signed_in(rig: &Rig, user: &str) -> String {
    let password = if user == "canoatesta" { "pw-a" } else { "pw-b" };
    login(rig, user, password, "198.51.100.7")
        .cookie()
        .expect("a cookie")
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
    let home = call(rig.addr, "GET", "/", &[("Cookie", &cookie)], "");
    assert_eq!(home.status, 200);
    assert!(home.body.contains("the app"));
    // The token is in no body, so no page can read it.
    let token = cookie.split_once('=').unwrap().1;
    let session = call(
        rig.addr,
        "GET",
        "/portal/session",
        &[("Cookie", &cookie)],
        "",
    );
    assert_eq!(session.status, 200);
    assert!(session.body.contains("canoatesta") && !session.body.contains(token));
    assert!(!home.body.contains(token) && !answer.body.contains(token));
}

#[test]
fn a_wrong_password_is_refused_and_the_failures_of_one_address_stop_the_tries_before_they_reach_the_accounts()
 {
    let rig = rig(|settings| settings.max_failures_per_address = 3, 0);
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
    // Another browser behind the proxy is not held to it.
    assert_eq!(
        login(&rig, "canoatesta", "pw-a", "203.0.113.10").status,
        303
    );
}

#[test]
fn the_address_a_proxy_names_counts_only_when_the_proxy_is_trusted() {
    // The test's own address is not a trusted proxy here, so a header it sends is not believed: every try is from one address.
    let rig = rig(
        |settings| {
            settings.max_failures_per_address = 2;
            settings.trusted_proxies = Vec::new();
        },
        0,
    );
    login(&rig, "canoatesta", "wrong", "203.0.113.1");
    login(&rig, "canoatesta", "wrong", "203.0.113.2");
    assert_eq!(login(&rig, "canoatesta", "pw-a", "203.0.113.3").status, 429);
    // A proxy's own address in the list is skipped for the one before it.
    let rig = self::rig(|settings| settings.max_failures_per_address = 2, 0);
    login(&rig, "canoatesta", "wrong", "203.0.113.1, 127.0.0.1");
    login(&rig, "canoatesta", "wrong", "evil, 203.0.113.1, 127.0.0.1");
    assert_eq!(login(&rig, "canoatesta", "pw-a", "203.0.113.1").status, 429);
    assert_eq!(login(&rig, "canoatesta", "pw-a", "203.0.113.2").status, 303);
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
                ("Origin", origin),
                ("Content-Type", "application/x-www-form-urlencoded"),
            ],
            "user=canoatesta&password=pw-a",
        );
        assert_eq!(answer.status, 403, "{origin}");
    }
    let none = call(
        rig.addr,
        "POST",
        LOGIN,
        &[("Content-Type", "application/x-www-form-urlencoded")],
        "user=canoatesta&password=pw-a",
    );
    assert_eq!(none.status, 403);
    assert_eq!(rig.accounts.asked.load(Ordering::SeqCst), 0);
    // And a Host that is not this portal's is turned away.
    let host = call(rig.addr, "GET", LOGIN, &[("Host", "evil.example")], "");
    assert!(host.status == 421 || host.status == 400, "{}", host.status);
}

#[test]
fn signing_out_ends_the_session_and_clears_the_cookie() {
    let rig = rig(|_| {}, 0);
    let cookie = signed_in(&rig, "canoatesta");
    let origin = format!("http://{}", rig.addr);
    let out = call(
        rig.addr,
        "POST",
        "/portal/logout",
        &[("Cookie", &cookie), ("Origin", &origin)],
        "",
    );
    assert_eq!(out.status, 303);
    assert!(out.header("set-cookie").unwrap().contains("Max-Age=0"));
    assert_eq!(
        call(
            rig.addr,
            "GET",
            "/portal/session",
            &[("Cookie", &cookie)],
            ""
        )
        .status,
        401
    );
}

#[test]
fn the_index_lists_the_users_own_host_with_channels_through_the_portal_and_starts_it_once() {
    let rig = rig(|_| {}, 0);
    let cookie = signed_in(&rig, "canoatesta");
    for _ in 0..2 {
        let index = call(rig.addr, "GET", web::HOST_INDEX, &[("Cookie", &cookie)], "");
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
    let origin = format!("http://{}", rig.addr);
    let mut for_a = open_channel(&rig, "/portal/ws/entry/x", &a, &origin).unwrap();
    let mut for_b = open_channel(&rig, "/portal/ws/entry/x", &b, &origin).unwrap();
    assert_eq!(exchange(&mut for_a, b"hello"), "canoatesta/entry:hello");
    assert_eq!(exchange(&mut for_b, b"hello"), "canoatestb/entry:hello");
    let mut control = open_channel(&rig, "/portal/ws/control", &a, &origin).unwrap();
    assert_eq!(
        exchange(&mut control, b"ledger"),
        "canoatesta/control:ledger"
    );
    // Nothing the browser says picks another user's socket.
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
    let origin = format!("http://{}", rig.addr);
    let mut socket = open_channel(&rig, "/portal/ws/entry/x", &cookie, &origin).unwrap();
    assert_eq!(exchange(&mut socket, b"a"), "canoatesta/entry:a");
    call(
        rig.addr,
        "POST",
        "/portal/logout",
        &[("Cookie", &cookie), ("Origin", &origin)],
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
    let origin = format!("http://{}", rig.addr);
    let _one = open_channel(&rig, "/portal/ws/entry/x", &cookie, &origin).unwrap();
    let _two = open_channel(&rig, "/portal/ws/entry/x", &cookie, &origin).unwrap();
    assert_eq!(
        open_channel(&rig, "/portal/ws/entry/x", &cookie, &origin).err(),
        Some(503)
    );
}

#[test]
fn the_browsers_key_is_introduced_to_its_users_host_only_and_waited_for() {
    let rig = rig(|_| {}, 2);
    let cookie = signed_in(&rig, "canoatesta");
    let origin = format!("http://{}", rig.addr);
    let pair = |cookie: &str, origin: &str| {
        call(
            rig.addr,
            "POST",
            "/portal/pair",
            &[
                ("Cookie", cookie),
                ("Origin", origin),
                ("Content-Type", "application/json"),
            ],
            r#"{"peer":"sha256:browserkey"}"#,
        )
    };
    // The host says "not waiting" twice, then accepts: the portal asks again until it does.
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
    let origin = format!("http://{}", rig.addr);
    let answer = call(
        rig.addr,
        "POST",
        "/portal/pair",
        &[("Cookie", &cookie), ("Origin", &origin)],
        r#"{"peer":"sha256:x"}"#,
    );
    assert_eq!(answer.status, 404);
    assert!(rig.hosts.introduced.lock().unwrap().is_empty());
}

#[test]
fn an_idle_host_is_stopped_and_comes_back_at_the_next_use_and_one_in_use_is_not() {
    let rig = rig(|settings| settings.idle = Duration::from_millis(300), 0);
    let cookie = signed_in(&rig, "canoatesta");
    let origin = format!("http://{}", rig.addr);
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
    };
    for bad in ["", "a b", "a@b", "../x", "a/b", "-x", "a\nb"] {
        let error = systemd.start(bad).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{bad:?}");
        assert_eq!(
            systemd.stop(bad).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }
    // A good name gets as far as running the program, which is not there.
    assert_ne!(
        systemd.start("canoatesta").unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
}

#[test]
fn the_running_hosts_are_the_users_named_in_the_units_systemctl_lists() {
    let dir = std::env::temp_dir().join(format!("canoa-portal-ctl-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("systemctl");
    std::fs::write(
        &script,
        "#!/bin/sh\ncat <<'LIST'\ncanoa@canoatesta.service loaded active running Canoa for canoatesta\ncanoa@bad@name.service loaded active running x\ncanoa@..%2Fx.service loaded active running x\nother.service loaded active running x\nLIST\n",
    )
    .unwrap();
    std::os::unix::fs::PermissionsExt::set_mode(
        &mut std::fs::metadata(&script).unwrap().permissions(),
        0o755,
    );
    std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let systemd = Systemd {
        systemctl: script,
        elevate: Vec::new(),
        unit: "canoa@{user}.service".to_owned(),
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
            asked: AtomicUsize::new(0),
        }),
        Arc::new(Systemd::new()),
    );
    assert!(matches!(missing, Err(PortalError::Site(_))));
    let _ = std::fs::remove_dir_all(empty);
    let _ = Path::new("");
}

#[test]
fn a_socket_another_users_process_made_is_never_dialed_for_a_person_whose_folder_leads_to_it() {
    // The stand-in hosts' sockets are made by this process's user. A person whose account has another user id, whose folder
    // holds a link to a socket like these, is dialed for that id, and what answers is not theirs.
    let rig = rig(|_| {}, 0);
    rig.accounts
        .accounts
        .lock()
        .unwrap()
        .get_mut("canoatestb")
        .unwrap()
        .1 = mine() + 1;
    let a = signed_in(&rig, "canoatesta");
    let b = {
        let password = "pw-b";
        login(&rig, "canoatestb", password, "198.51.100.8")
            .cookie()
            .expect("a cookie")
    };
    let origin = format!("http://{}", rig.addr);
    // The host for a starts and is reached: its sockets were made by a's user.
    let mut channel = open_channel(&rig, "/portal/ws/entry/x", &a, &origin).unwrap();
    assert_eq!(exchange(&mut channel, b"hello"), "canoatesta/entry:hello");
    // b is the same process's, but b's account says another user: nothing is given.
    assert_eq!(
        call(rig.addr, "GET", web::HOST_INDEX, &[("Cookie", &b)], "").status,
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
        &[("Cookie", &b), ("Origin", &origin)],
        r#"{"peer":"sha256:x"}"#,
    );
    assert_eq!(pair.status, 503);
    assert!(rig.hosts.introduced.lock().unwrap().is_empty());
}
