//! `network/tunnel` through the stand-in proxy with its login, over loopback.
#![allow(
    clippy::std_instead_of_core,
    reason = "a test names what it uses from std"
)]

use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use canoa::network::{Network, NetworkSettings, ProxyChoice, Secret};

use super::overlays_tests::ask;
use super::*;
use crate::network::standins::StandInProxy;
use crate::network::tests::{PASSWORD, USER};

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

#[test]
fn a_connection_goes_through_the_proxy_with_its_login_and_carries_bytes_both_ways() {
    let proxy = StandInProxy::start(USER, PASSWORD, Some(echo()));
    let control = control(through(&proxy, PASSWORD));
    let answer = ask(
        &control,
        &Caller::Owner,
        "network/tunnel",
        json!({"url": "https://remote.example.test:8443/", "seconds": 10}),
    );
    let port = answer["result"]["port"].as_u64().expect("a port");
    assert_eq!(
        proxy.allowed(USER, PASSWORD),
        ["remote.example.test:8443"],
        "the proxy was asked, with the login, before the app connected: {answer}"
    );

    let mut app =
        TcpStream::connect(("127.0.0.1", u16::try_from(port).expect("a port"))).expect("connects");
    app.set_read_timeout(Some(Duration::from_secs(10)))
        .expect("timeout");
    app.write_all(b"hello through").expect("writes");
    let mut echoed = [0u8; 13];
    app.read_exact(&mut echoed).expect("reads");
    assert_eq!(&echoed, b"hello through");
}

#[test]
fn a_proxy_that_refuses_the_login_leaves_the_app_without_a_connection() {
    let proxy = StandInProxy::start(USER, PASSWORD, Some(echo()));
    let control = control(through(&proxy, "wrong"));
    let answer = ask(
        &control,
        &Caller::Owner,
        "network/tunnel",
        json!({"url": "https://remote.example.test:8443/"}),
    );
    assert_eq!(answer["error"]["code"], REFUSED, "{answer}");
    assert!(
        proxy.allowed(USER, PASSWORD).is_empty(),
        "nothing got through"
    );
}

#[test]
fn with_no_proxy_the_app_is_told_to_connect_itself_and_the_proxy_is_not_asked() {
    let proxy = StandInProxy::start(USER, PASSWORD, Some(echo()));
    let mut settings = NetworkSettings::default();
    settings.proxy = ProxyChoice::None;
    let control = control(Arc::new(Network::new(&settings).expect("a network")));
    let answer = ask(
        &control,
        &Caller::Owner,
        "network/tunnel",
        json!({"url": "https://remote.example.test:8443/"}),
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
