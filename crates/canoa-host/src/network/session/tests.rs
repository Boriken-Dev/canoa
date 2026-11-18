//! A session's network over loopback stand-ins: the proxy with a login and the origin behind it that the network tests use, a PAC script,
//! an origin that answers plain HTTP, and a program that listens where it is told.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a test names what it uses from std"
)]

use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use canoa::network::{BypassRules, Dialect, Network};
use canoa::profile_item::NetworkEffect;

use super::{Middleman, Services, start};
use crate::network::standins::StandInSocks;
use crate::network::tests::{
    Dir, PASSWORD, USER, environment_of, printing_environment, proxied, through, world,
};

pub(super) const TOKEN: &str = "0123456789abcdef";

pub(super) fn basic(token: &str) -> String {
    format!(
        "Basic {}",
        data_encoding::BASE64.encode(format!("canoa:{token}").as_bytes())
    )
}

fn direct_only(bypass: &[&str]) -> BypassRules {
    BypassRules::from_entries(bypass.iter().copied(), Dialect::NoProxy)
}

/// What the proxy answered to `request`, head and then whatever came with it.
pub(super) fn ask(proxy: SocketAddr, request: &str) -> (TcpStream, String) {
    let mut stream = TcpStream::connect(proxy).expect("connects");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("timeout");
    stream.write_all(request.as_bytes()).expect("writes");
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte).expect("reads") == 0 {
            break;
        }
        head.push(byte[0]);
    }
    (stream, String::from_utf8_lossy(&head).into_owned())
}

pub(super) fn connect(target: &str, authorization: Option<&str>) -> String {
    let mut text = format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n");
    if let Some(value) = authorization {
        text.push_str(&format!("Proxy-Authorization: {value}\r\n"));
    }
    text.push_str("\r\n");
    text
}

/// Closes `stream`'s sending half, as a program that has said all it has to says, and checks the proxy passes that on to the origin
/// (which then closes) and the origin's close back: the stream reads to its end.
fn closes(stream: &mut TcpStream) {
    stream
        .shutdown(std::net::Shutdown::Write)
        .expect("half-closes");
    let mut rest = Vec::new();
    stream
        .read_to_end(&mut rest)
        .expect("the origin's close came back to the client");
    assert!(rest.is_empty(), "{rest:?}");
}

/// An origin that echoes what it is sent, on loopback.
fn echo() -> (SocketAddr, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let thread = thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        let mut buffer = [0u8; 64];
        while let Ok(count) = stream.read(&mut buffer) {
            if count == 0 || stream.write_all(&buffer[..count]).is_err() {
                break;
            }
        }
    });
    (addr, thread)
}

#[test]
fn a_program_with_the_sessions_secret_reaches_its_destination_through_the_network_and_its_proxy_login()
 {
    let world = world("session-system", &["downloads.example.test"], b"x");
    let proxy =
        Middleman::start(through(&world), BypassRules::none(), TOKEN, None).expect("starts");
    let (_stream, head) = ask(
        proxy.addr,
        &connect("downloads.example.test:443", Some(&basic(TOKEN))),
    );
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    assert_eq!(
        world.proxy.allowed(USER, PASSWORD),
        ["downloads.example.test:443"],
        "the system's proxy was asked, and with its login"
    );
}

#[test]
fn another_process_on_this_computer_is_turned_away_without_the_sessions_secret() {
    let world = world("session-secret", &["downloads.example.test"], b"x");
    let proxy =
        Middleman::start(through(&world), BypassRules::none(), TOKEN, None).expect("starts");
    for authorization in [None, Some(basic("wrong").as_str()), Some("Basic !!!")] {
        let (_stream, head) = ask(
            proxy.addr,
            &connect("downloads.example.test:443", authorization),
        );
        assert!(head.starts_with("HTTP/1.1 407"), "{head}");
        assert!(head.contains("Proxy-Authenticate: Basic"), "{head}");
    }
    assert!(
        world.proxy.seen().is_empty(),
        "nothing was sent on as the person"
    );
}

#[test]
fn a_host_on_the_bypass_list_goes_straight_to_its_destination() {
    let world = world("session-bypass", &["downloads.example.test"], b"x");
    let (origin, thread) = echo();
    let proxy = Middleman::start(through(&world), direct_only(&["127.0.0.1"]), TOKEN, None)
        .expect("starts");
    let (mut stream, head) = ask(
        proxy.addr,
        &connect(&origin.to_string(), Some(&basic(TOKEN))),
    );
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    stream.write_all(b"ping").expect("writes");
    let mut back = [0u8; 4];
    stream.read_exact(&mut back).expect("echoed");
    assert_eq!(&back, b"ping");
    assert!(world.proxy.seen().is_empty(), "the proxy was not asked");
    closes(&mut stream);
    let _ = thread.join();
}

#[test]
fn a_plain_http_request_is_sent_on_in_the_form_the_origin_reads_with_no_proxy_headers() {
    let origin = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = origin.local_addr().expect("addr");
    let seen = Arc::new(Mutex::new(String::new()));
    let kept = Arc::clone(&seen);
    let served = thread::spawn(move || {
        let (mut stream, _) = origin.accept().expect("accepts");
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            if stream.read(&mut byte).expect("reads") == 0 {
                break;
            }
            head.push(byte[0]);
        }
        *kept.lock().expect("lock") = String::from_utf8_lossy(&head).into_owned();
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nhi")
            .expect("answers");
    });
    let world = world("session-plain", &["downloads.example.test"], b"x");
    let proxy = Middleman::start(through(&world), direct_only(&["127.0.0.1"]), TOKEN, None)
        .expect("starts");
    let request = format!(
        "GET http://{addr}/path?x=1 HTTP/1.1\r\nHost: {addr}\r\nProxy-Authorization: {}\r\nProxy-Connection: keep-alive\r\nAccept: */*\r\n\r\n",
        basic(TOKEN)
    );
    let (mut stream, head) = ask(proxy.addr, &request);
    assert!(head.starts_with("HTTP/1.1 200 OK"), "{head}");
    let mut body = [0u8; 2];
    stream.read_exact(&mut body).expect("body");
    assert_eq!(&body, b"hi");
    served.join().expect("served");
    let received = seen.lock().expect("lock").clone();
    assert!(
        received.starts_with("GET /path?x=1 HTTP/1.1\r\n"),
        "{received}"
    );
    assert!(received.contains("Accept: */*"), "{received}");
    assert!(
        !received.to_ascii_lowercase().contains("proxy-"),
        "{received}"
    );
}

#[test]
fn a_request_that_names_no_destination_is_refused_as_such() {
    let world = world("session-bad", &["downloads.example.test"], b"x");
    let proxy =
        Middleman::start(through(&world), BypassRules::none(), TOKEN, None).expect("starts");
    let (_s, head) = ask(
        proxy.addr,
        &format!(
            "GET /only-a-path HTTP/1.1\r\nProxy-Authorization: {}\r\n\r\n",
            basic(TOKEN)
        ),
    );
    assert!(head.starts_with("HTTP/1.1 400"), "{head}");
    let (_s, head) = ask(
        proxy.addr,
        &connect("no-port-and-no-colon:x", Some(&basic(TOKEN))),
    );
    assert!(head.starts_with("HTTP/1.1 400"), "{head}");
}

#[test]
fn a_destination_the_network_cannot_reach_is_a_bad_gateway_with_the_reason() {
    let dead = {
        let probe = TcpListener::bind("127.0.0.1:0").expect("bind");
        probe.local_addr().expect("addr")
    };
    let world = world("session-down", &["downloads.example.test"], b"x");
    let proxy = Middleman::start(through(&world), direct_only(&["127.0.0.1"]), TOKEN, None)
        .expect("starts");
    let (_s, head) = ask(proxy.addr, &connect(&dead.to_string(), Some(&basic(TOKEN))));
    assert!(head.starts_with("HTTP/1.1 502"), "{head}");
}

#[test]
fn dropping_the_session_network_stops_its_proxy() {
    let world = world("session-stop", &["downloads.example.test"], b"x");
    let proxy =
        Middleman::start(through(&world), BypassRules::none(), TOKEN, None).expect("starts");
    let addr = proxy.addr;
    let (mut held, head) = ask(
        addr,
        &connect("downloads.example.test:443", Some(&basic(TOKEN))),
    );
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    drop(proxy);
    // The connection it carried is closed, and nothing listens there any more.
    let mut buffer = [0u8; 1];
    assert!(held.read(&mut buffer).map_or(true, |count| count == 0));
    assert!(TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_err());
}

fn effect(mode: &str) -> NetworkEffect {
    NetworkEffect::new(mode)
}

fn no_environment(_: &str) -> Option<String> {
    None
}

#[test]
fn the_system_mode_gives_the_programs_a_loopback_proxy_with_the_secret_and_keeps_loopback_off_it() {
    let mut asked = effect("system");
    asked.bypass = vec!["internal.example".to_owned()];
    let session = start(&asked, &no_environment, &Services::default()).expect("starts");
    let value = |name: &str| {
        session
            .variables
            .iter()
            .find(|(each, _)| each == name)
            .map(|(_, value)| value.clone())
    };
    let url = value("HTTPS_PROXY").expect("a proxy");
    assert!(url.starts_with("http://canoa:"), "{url}");
    assert!(url.contains("@127.0.0.1:"), "{url}");
    for name in [
        "HTTP_PROXY",
        "ALL_PROXY",
        "https_proxy",
        "http_proxy",
        "all_proxy",
    ] {
        assert_eq!(value(name), Some(url.clone()), "{name}");
    }
    assert_eq!(
        value("NO_PROXY").as_deref(),
        Some("localhost,127.0.0.1,::1,internal.example")
    );
    assert!(session.description.contains("following the system"));
    assert!(
        !session.description.contains("canoa:"),
        "the secret is not in what the report says"
    );
    // The address in the variables is the proxy that is running.
    let target = url
        .rsplit_once('@')
        .and_then(|(_, address)| address.parse::<SocketAddr>().ok())
        .expect("an address");
    assert!(TcpStream::connect_timeout(&target, Duration::from_secs(2)).is_ok());
    drop(session);
    assert!(TcpStream::connect_timeout(&target, Duration::from_millis(500)).is_err());
}

#[test]
fn the_none_mode_says_no_proxy_for_anything_and_the_hosts_own_starter_adds_none() {
    let session = start(&effect("none"), &no_environment, &Services::default()).expect("starts");
    let names: Vec<&str> = session
        .variables
        .iter()
        .map(|(name, _)| name.as_str())
        .collect();
    assert_eq!(names, ["NO_PROXY", "no_proxy"]);
    assert!(session.variables.iter().all(|(_, value)| value == "*"));
    // A program the host starts with them, whatever the host's own network says, is given no proxy of the system's.
    let dir = Dir::new("session-none");
    let mut command = printing_environment();
    command.envs(session.variables.iter().cloned());
    command.network(proxied(&dir.0));
    let seen = environment_of(&mut command);
    assert!(
        !seen.to_ascii_lowercase().contains("_proxy=http"),
        "a proxy was added: {seen}"
    );
    assert!(
        seen.lines()
            .any(|line| line.eq_ignore_ascii_case("NO_PROXY=*")),
        "{seen}"
    );
    assert!(!seen.contains("carol"), "{seen}");
}

#[test]
fn the_proxy_mode_gives_the_programs_the_proxy_it_names() {
    let mut asked = effect("proxy");
    asked.url = Some("http://gateway.example:3128".to_owned());
    asked.bypass = vec!["direct.example".to_owned()];
    let session = start(&asked, &no_environment, &Services::default()).expect("starts");
    let value = |name: &str| {
        session
            .variables
            .iter()
            .find(|(each, _)| each == name)
            .map(|(_, value)| value.as_str())
    };
    assert_eq!(value("HTTPS_PROXY"), Some("http://gateway.example:3128"));
    assert!(value("NO_PROXY").is_some_and(|list| list.contains("direct.example")));
    // A user whose password variable is not set is said, not guessed.
    asked.user = Some("alice".to_owned());
    asked.password_variable = Some("NO_SUCH_PASSWORD".to_owned());
    let why = start(&asked, &no_environment, &Services::default()).expect_err("refused");
    assert!(why.contains("NO_SUCH_PASSWORD"), "{why}");
}

#[test]
fn the_pac_mode_chooses_the_route_for_each_destination_and_logs_in() {
    let world = world("session-pac", &["downloads.example.test"], b"x");
    let (origin, thread) = echo();
    let dir = Dir::new("session-pac-script");
    let script = dir.0.join("proxy.pac");
    fs::write(
        &script,
        format!(
            "function FindProxyForURL(url, host) {{ if (host == 'downloads.example.test') return 'PROXY {}'; return 'DIRECT'; }}",
            world.proxy.addr
        ),
    )
    .expect("writes");
    let mut asked = effect("pac");
    asked.url = Some(script.to_string_lossy().into_owned());
    asked.user = Some(USER.to_owned());
    asked.password_variable = Some("PROXY_PASSWORD".to_owned());
    let session = start(
        &asked,
        &|name| (name == "PROXY_PASSWORD").then(|| PASSWORD.to_owned()),
        &Services::default(),
    )
    .expect("starts");
    let url = session
        .variables
        .iter()
        .find(|(name, _)| name == "HTTPS_PROXY")
        .map(|(_, value)| value.clone())
        .expect("a proxy");
    let (address, token) = (
        url.rsplit_once('@')
            .and_then(|(_, address)| address.parse::<SocketAddr>().ok())
            .expect("an address"),
        url.trim_start_matches("http://canoa:")
            .split('@')
            .next()
            .expect("a token")
            .to_owned(),
    );
    // The destination the script sends to the proxy goes there, with the login; the one it sends direct does not.
    let (_stream, head) = ask(
        address,
        &connect("downloads.example.test:443", Some(&basic(&token))),
    );
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    assert_eq!(
        world.proxy.allowed(USER, PASSWORD),
        ["downloads.example.test:443"]
    );
    let (mut direct, head) = ask(address, &connect(&origin.to_string(), Some(&basic(&token))));
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    direct.write_all(b"ok").expect("writes");
    let mut back = [0u8; 2];
    direct.read_exact(&mut back).expect("echoed");
    assert_eq!(&back, b"ok");
    assert_eq!(
        world.proxy.allowed(USER, PASSWORD).len(),
        1,
        "the direct destination did not go through the proxy"
    );
    closes(&mut direct);
    let _ = thread.join();
}

/// Not a test of anything on its own: the program a `command` network starts is this test binary, told by its last argument where to
/// listen. Run by a person it returns at once, since no such argument is there.
#[test]
fn stand_in_proxy_program() {
    let Some(listen) = std::env::args()
        .next_back()
        .and_then(|last| last.parse::<SocketAddr>().ok())
    else {
        return;
    };
    let listener = TcpListener::bind(listen).expect("listens");
    let stop = Arc::new(AtomicBool::new(false));
    while !stop.load(Ordering::SeqCst) {
        let _ = listener.accept();
    }
}

#[test]
fn the_command_mode_starts_the_program_where_it_is_told_to_listen_and_stops_it_with_the_session() {
    let exe = std::env::current_exe().expect("this program");
    let mut asked = effect("command");
    asked.command = vec![
        exe.to_string_lossy().into_owned(),
        "--exact".to_owned(),
        "network::session::tests::stand_in_proxy_program".to_owned(),
        "{listen}".to_owned(),
    ];
    let session = start(&asked, &no_environment, &Services::default()).expect("starts and listens");
    let url = session
        .variables
        .iter()
        .find(|(name, _)| name == "HTTP_PROXY")
        .map(|(_, value)| value.clone())
        .expect("a proxy");
    let target: SocketAddr = url
        .trim_start_matches("http://")
        .parse()
        .expect("the address it was told");
    assert!(session.description.contains("listening on"));
    drop(session);
    let started = Instant::now();
    while TcpStream::connect_timeout(&target, Duration::from_millis(200)).is_ok() {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the program still listens after the session"
        );
        thread::sleep(Duration::from_millis(100));
    }
    // A program that does not exist is said.
    asked.command = vec!["/no/such/program".to_owned()];
    let why = start(&asked, &no_environment, &Services::default()).expect_err("refused");
    assert!(why.contains("could not start"), "{why}");
}

#[test]
fn the_command_mode_sends_the_hosts_own_requests_for_the_session_through_the_program_too() {
    let exe = std::env::current_exe().expect("this program");
    let mut asked = effect("command");
    asked.command = vec![
        exe.to_string_lossy().into_owned(),
        "--exact".to_owned(),
        "network::session::tests::stand_in_proxy_program".to_owned(),
        "{listen}".to_owned(),
    ];
    let session = start(&asked, &no_environment, &Services::default()).expect("starts and listens");
    let routes = session
        .network
        .resolver()
        .lookup("https://api.example.test/")
        .expect("a route");
    let shown = format!("{routes:?}");
    let listen = session
        .variables
        .iter()
        .find(|(name, _)| name == "HTTP_PROXY")
        .map(|(_, value)| value.trim_start_matches("http://").to_owned())
        .expect("a proxy");
    assert!(
        shown.contains("Proxy") && shown.contains(&listen),
        "{shown}"
    );
}

#[test]
fn a_socks_upstream_carries_what_a_program_asks_the_session_proxy_for() {
    let world = world("session-socks", &["downloads.example.test"], b"x");
    let (origin, thread) = echo();
    let socks = StandInSocks::start(origin);
    let dir = Dir::new("session-socks-script");
    let script = dir.0.join("proxy.pac");
    fs::write(
        &script,
        format!(
            "function FindProxyForURL(url, host) {{ if (host == 'reached-by-socks.example.test') return 'SOCKS5 {}'; return 'DIRECT'; }}",
            socks.addr
        ),
    )
    .expect("writes");
    let mut asked = effect("pac");
    asked.url = Some(script.to_string_lossy().into_owned());
    let session = start(&asked, &no_environment, &Services::default()).expect("starts");
    let url = session
        .variables
        .iter()
        .find(|(name, _)| name == "HTTPS_PROXY")
        .map(|(_, value)| value.clone())
        .expect("a proxy");
    let address = url
        .rsplit_once('@')
        .and_then(|(_, address)| address.parse::<SocketAddr>().ok())
        .expect("an address");
    let token = url
        .trim_start_matches("http://canoa:")
        .split('@')
        .next()
        .expect("a token")
        .to_owned();
    let (mut stream, head) = ask(
        address,
        &connect("reached-by-socks.example.test:443", Some(&basic(&token))),
    );
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    stream.write_all(b"ping").expect("writes");
    let mut back = [0u8; 4];
    stream
        .read_exact(&mut back)
        .expect("echoed by the origin behind the SOCKS server");
    assert_eq!(&back, b"ping");
    assert_eq!(socks.asked(), ["reached-by-socks.example.test:443"]);
    assert!(world.proxy.seen().is_empty(), "no HTTP proxy was asked");
    closes(&mut stream);
    let _ = thread.join();
}

#[test]
fn a_session_network_keeps_the_roots_and_certificate_of_host_network_and_not_its_proxy_login() {
    use canoa::network::{ClientCertificate, ProxyChoice, Secret};
    let mut installed = canoa::network::NetworkSettings::default();
    installed.proxy = ProxyChoice::Proxy("http://corporate.example:3128".to_owned());
    installed.proxy_user = Some("me".to_owned());
    installed.proxy_password = Some(Secret::new("hers"));
    installed.no_proxy = Some(vec!["internal.example".to_owned()]);
    installed.ca_file = Some("roots.pem".into());
    installed.client_certificate = ClientCertificate::File("me.pem".into());
    let mut asked = effect("pac");
    asked.url = Some("proxy.pac".to_owned());
    let made = super::settings_over(
        installed.clone(),
        ProxyChoice::Pac("proxy.pac".to_owned()),
        &asked,
        &no_environment,
    )
    .expect("settings");
    assert_eq!(made.proxy, ProxyChoice::Pac("proxy.pac".to_owned()));
    assert_eq!(made.ca_file, installed.ca_file);
    assert_eq!(made.client_certificate, installed.client_certificate);
    assert!(made.proxy_user.is_none() && made.proxy_password.is_none());
    assert!(
        made.no_proxy.is_none(),
        "the profile's own list, which is empty"
    );
}

#[test]
fn the_system_mode_uses_the_network_the_host_installed_for_its_own_requests() {
    // The session's requests are the process's network's: nothing here installs one, so it is the system's.
    let network: Arc<Network> = Network::current();
    let session = start(&effect("system"), &no_environment, &Services::default()).expect("starts");
    assert!(Arc::ptr_eq(&session.network, &network));
}
