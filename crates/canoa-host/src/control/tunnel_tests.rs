//! `network/tunnel` through the stand-in proxy with its login, and to TLS servers whose certificates only this host's network trusts
//! or that ask for a client certificate, over loopback.
#![allow(
    clippy::std_instead_of_core,
    reason = "a test names what it uses from std"
)]

use std::fs;
use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use canoa::network::{ClientCertificate, Network, NetworkSettings, ProxyChoice, Secret};
use coqui_syshttp::tls::{ClientCertificates, Identity, Selector, TlsBuilder, Trust};

use super::overlays_tests::ask;
use super::*;
use crate::network::standins::{Ca, ClientAuth, Leaf, StandInProxy, TlsServer};
use crate::network::tests::{Dir, Keystore, PASSWORD, USER, direct, settings, world};

/// A TCP server that sends back what it is sent, standing for the remote Canoa host the proxy reaches.
fn echo() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    thread::spawn(move || {
        while let Ok((mut stream, _)) = listener.accept() {
            thread::spawn(move || {
                let mut buffer = [0u8; 1024];
                while let Ok(count) = stream.read(&mut buffer) {
                    if count == 0 || stream.write_all(&buffer[..count]).is_err() {
                        break;
                    }
                }
            });
        }
    });
    addr
}

/// The settings that send everything through `proxy`, logging in with `password`.
fn through(proxy: &StandInProxy, password: &str) -> Arc<Network> {
    let mut settings = NetworkSettings::default();
    settings.proxy = ProxyChoice::Proxy(format!("http://{}", proxy.addr));
    settings.proxy_user = Some(USER.to_owned());
    settings.proxy_password = Some(Secret::new(password));
    Arc::new(Network::new(&settings).expect("a network"))
}

fn control(network: Arc<Network>) -> Control {
    Control::open(None, 10, 1024, Vec::new())
        .expect("opens")
        .with_tunnel(tunnel::Tunnel::on(network))
}

/// What `network/tunnel` answered: the loopback port, the token to say first, and who the server proved to be.
fn answered(answer: &Value) -> (u16, String) {
    let port = answer["result"]["port"].as_u64().expect("a port");
    let token = answer["result"]["token"].as_str().expect("a token");
    (u16::try_from(port).expect("a port"), token.to_owned())
}

/// The app's side of a tunnel: it connects, says the token and sends `bytes`.
fn app(port: u16, token: &str, bytes: &[u8]) -> TcpStream {
    let mut app = TcpStream::connect(("127.0.0.1", port)).expect("connects");
    app.set_read_timeout(Some(Duration::from_secs(10)))
        .expect("timeout");
    app.write_all(token.as_bytes()).expect("says the token");
    app.write_all(bytes).expect("writes");
    app
}

/// What an HTTPS server answers a `GET` the app sends through a tunnel, until it closes.
fn get(port: u16, token: &str) -> String {
    let mut app = app(port, token, b"GET /x HTTP/1.1\r\nHost: test\r\n\r\n");
    let mut reply = Vec::new();
    let _ = app.read_to_end(&mut reply);
    String::from_utf8_lossy(&reply).into_owned()
}

#[test]
fn a_connection_goes_through_the_proxy_with_its_login_and_carries_bytes_both_ways() {
    let proxy = StandInProxy::start(USER, PASSWORD, Some(echo()));
    let control = control(through(&proxy, PASSWORD));
    let answer = ask(
        &control,
        &Caller::Owner,
        "network/tunnel",
        json!({"url": "http://remote.example.test:8443/", "seconds": 10}),
    );
    let (port, token) = answered(&answer);
    assert!(
        answer["result"].get("tls").is_none(),
        "no TLS is made for an http address: {answer}"
    );
    assert_eq!(
        proxy.allowed(USER, PASSWORD),
        ["remote.example.test:8443"],
        "the proxy was asked, with the login, before the app connected: {answer}"
    );

    let mut app = app(port, &token, b"hello through");
    let mut echoed = [0u8; 13];
    app.read_exact(&mut echoed).expect("reads");
    assert_eq!(&echoed, b"hello through");
}

#[test]
fn a_proxy_that_refuses_the_login_leaves_the_app_without_a_connection() {
    for scheme in ["http", "https"] {
        let proxy = StandInProxy::start(USER, PASSWORD, Some(echo()));
        let control = control(through(&proxy, "wrong"));
        let answer = ask(
            &control,
            &Caller::Owner,
            "network/tunnel",
            json!({"url": format!("{scheme}://remote.example.test:8443/")}),
        );
        assert_eq!(answer["error"]["code"], REFUSED, "{answer}");
        assert!(
            answer["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("proxy")),
            "{answer}"
        );
        assert!(
            proxy.allowed(USER, PASSWORD).is_empty(),
            "nothing got through"
        );
    }
}

#[test]
fn with_no_proxy_an_http_address_is_told_to_connect_itself_and_the_proxy_is_not_asked() {
    let proxy = StandInProxy::start(USER, PASSWORD, Some(echo()));
    let mut settings = NetworkSettings::default();
    settings.proxy = ProxyChoice::None;
    let control = control(Arc::new(Network::new(&settings).expect("a network")));
    let answer = ask(
        &control,
        &Caller::Owner,
        "network/tunnel",
        json!({"url": "http://remote.example.test:8443/"}),
    );
    assert_eq!(answer["result"], json!({"direct": true}), "{answer}");
    assert!(proxy.seen().is_empty());
}

#[test]
fn a_key_is_refused_and_a_url_that_is_not_http_is_not_a_request() {
    let proxy = StandInProxy::start(USER, PASSWORD, Some(echo()));
    let control = control(through(&proxy, PASSWORD));
    let key = Caller::Peer {
        id: "key".to_owned(),
        grants: BTreeSet::new(),
    };
    let denied = ask(
        &control,
        &key,
        "network/tunnel",
        json!({"url": "https://remote.example.test:8443/"}),
    );
    assert_eq!(denied["error"]["code"], DENIED, "{denied}");
    for params in [
        json!({}),
        json!({"url": "ftp://remote.example.test/"}),
        json!({"url": "https://remote.example.test/", "seconds": 0}),
    ] {
        let bad = ask(&control, &Caller::Owner, "network/tunnel", params);
        assert_eq!(bad["error"]["code"], BAD_PARAMS, "{bad}");
    }
    assert!(proxy.seen().is_empty());
}

#[test]
fn a_connection_that_does_not_say_the_token_is_not_carried() {
    let proxy = StandInProxy::start(USER, PASSWORD, Some(echo()));
    let control = control(through(&proxy, PASSWORD));
    let answer = ask(
        &control,
        &Caller::Owner,
        "network/tunnel",
        json!({"url": "http://remote.example.test:8443/", "seconds": 20}),
    );
    let (port, token) = answered(&answer);
    // Another process on this computer finds the port first.
    let mut other = app(port, &"0".repeat(token.len()), b"not for you");
    let mut heard = [0u8; 1];
    assert!(
        matches!(other.read(&mut heard), Ok(0) | Err(_)),
        "a connection that did not know the token was carried"
    );
    // The app still gets its turn.
    let mut mine = app(port, &token, b"mine");
    let mut echoed = [0u8; 4];
    mine.read_exact(&mut echoed).expect("reads");
    assert_eq!(&echoed, b"mine");
}

#[test]
fn a_server_certified_by_a_ca_only_this_hosts_network_trusts_is_verified_and_carried_through_the_proxy()
 {
    let world = world("tunnel-ca", &["remote.example.test"], b"the remote host");
    let control = control(Arc::new(
        Network::new(&settings(&world, PASSWORD)).expect("a network"),
    ));
    let answer = ask(
        &control,
        &Caller::Owner,
        "network/tunnel",
        json!({"url": "https://remote.example.test:8443/", "seconds": 10}),
    );
    let (port, token) = answered(&answer);
    assert_eq!(
        world.proxy.allowed(USER, PASSWORD),
        ["remote.example.test:8443"],
        "{answer}"
    );
    let tls = &answer["result"]["tls"];
    assert_eq!(tls["name"], "remote.example.test", "{answer}");
    assert!(
        tls["subject"]
            .as_str()
            .is_some_and(|subject| subject.contains("remote.example.test")),
        "{answer}"
    );
    assert_eq!(tls["sha256"].as_str().map(str::len), Some(64), "{answer}");

    // What the app sends is what goes inside the TLS: the server answers it in the clear.
    let reply = get(port, &token);
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
    assert!(reply.ends_with("the remote host"), "{reply}");
}

#[test]
fn a_server_this_hosts_network_does_not_trust_is_refused_before_the_app_connects() {
    let world = world("tunnel-untrusted", &["remote.example.test"], b"never");
    // The operating system's trust, which does not hold this test's CA.
    let mut untrusting = settings(&world, PASSWORD);
    untrusting.ca_file = None;
    let control = control(Arc::new(Network::new(&untrusting).expect("a network")));
    let answer = ask(
        &control,
        &Caller::Owner,
        "network/tunnel",
        json!({"url": "https://remote.example.test:8443/"}),
    );
    assert_eq!(answer["error"]["code"], REFUSED, "{answer}");
    assert!(
        answer["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("TLS")),
        "{answer}"
    );
}

#[test]
fn a_server_reached_with_no_proxy_is_verified_by_the_host_too() {
    let ca = Ca::new("Test CA");
    let server = ca.issue_server(&["localhost"]);
    let origin = TlsServer::start(
        &server,
        &[],
        ClientAuth::Off,
        Arc::new(|_, _| b"direct and verified".to_vec()),
    );
    let dir = Dir::new("tunnel-direct");
    let ca_file = dir.0.join("ca.pem");
    fs::write(&ca_file, ca.pem()).expect("writes");
    let mut settings = NetworkSettings::default();
    settings.proxy = ProxyChoice::None;
    settings.ca_file = Some(ca_file);
    let control = control(Arc::new(Network::new(&settings).expect("a network")));
    let answer = ask(
        &control,
        &Caller::Owner,
        "network/tunnel",
        json!({"url": format!("https://localhost:{}/", origin.addr.port())}),
    );
    // The app is not told to connect itself, which would leave the TLS to its own roots.
    let (port, token) = answered(&answer);
    assert_eq!(answer["result"]["tls"]["name"], "localhost", "{answer}");
    assert!(get(port, &token).ends_with("direct and verified"));
}

/// A server that insists on a client certificate from `ca`'s clients and says whether it got one.
fn asking(ca: &Ca) -> (TlsServer, Leaf) {
    let server = ca.issue_server(&["localhost"]);
    let origin = TlsServer::start(
        &server,
        &[ca.der()],
        ClientAuth::Required,
        Arc::new(|_, seen| {
            if seen {
                b"client certificate seen".to_vec()
            } else {
                b"no client certificate".to_vec()
            }
        }),
    );
    (origin, ca.issue_client("Alice Example"))
}

fn url_of(origin: &TlsServer) -> Value {
    json!({"url": format!("https://localhost:{}/", origin.addr.port())})
}

#[test]
fn a_server_that_asks_for_a_client_certificate_gets_the_one_host_network_selects() {
    let ca = Ca::new("Test CA");
    let (origin, client) = asking(&ca);
    let dir = Dir::new("tunnel-mtls");
    let ca_file = dir.0.join("ca.pem");
    fs::write(&ca_file, ca.pem()).expect("writes");
    let identity_file = dir.0.join("client.pem");
    fs::write(
        &identity_file,
        format!("{}{}", client.chain_pem, client.key_pem),
    )
    .expect("writes");
    let mut settings = NetworkSettings::default();
    settings.proxy = ProxyChoice::None;
    settings.ca_file = Some(ca_file);
    settings.client_certificate = ClientCertificate::File(identity_file);
    let control_with = control(Arc::new(Network::new(&settings).expect("a network")));
    let answer = ask(
        &control_with,
        &Caller::Owner,
        "network/tunnel",
        url_of(&origin),
    );
    let (port, token) = answered(&answer);
    assert!(
        get(port, &token).ends_with("client certificate seen"),
        "{answer}"
    );
    assert_eq!(
        origin.presented()[0].as_deref(),
        Some(client.chain[0].as_ref())
    );

    // With none selected the server that insists answers nothing: the setting is what made the difference.
    settings.client_certificate = ClientCertificate::None;
    let control_without = control(Arc::new(Network::new(&settings).expect("a network")));
    let answer = ask(
        &control_without,
        &Caller::Owner,
        "network/tunnel",
        url_of(&origin),
    );
    let seen = if answer.get("error").is_some() {
        String::new()
    } else {
        let (port, token) = answered(&answer);
        get(port, &token)
    };
    assert!(!seen.contains("certificate seen"), "{seen}");
    assert_eq!(
        origin.presented().len(),
        1,
        "no second connection was served"
    );
}

#[test]
fn a_server_that_asks_for_a_client_certificate_gets_the_keystores_one() {
    let ca = Ca::new("Test CA");
    let (origin, client) = asking(&ca);
    let identity = Identity::from_pem(client.chain_pem.as_bytes(), client.key_pem.as_bytes())
        .expect("an identity");
    let selector = Selector::builder().source(Keystore(identity)).build();
    let tls = TlsBuilder::default()
        .trust(Trust::Roots(vec![ca.der()]))
        .client_certificates(ClientCertificates::Selector(Arc::new(selector)));
    let control = control(Arc::new(Network::with(direct(), tls)));
    let answer = ask(&control, &Caller::Owner, "network/tunnel", url_of(&origin));
    let (port, token) = answered(&answer);
    assert!(
        get(port, &token).ends_with("client certificate seen"),
        "{answer}"
    );
    assert_eq!(
        origin.presented()[0].as_deref(),
        Some(client.chain[0].as_ref())
    );
}
