//! `network/fetch` through the stand-in proxy with its login, over loopback.

use std::sync::Arc;

use canoa::network::Network;
use data_encoding::BASE64;

use super::overlays_tests::ask;
use super::*;
use crate::network::tests::{PASSWORD, USER, settings, through, world};

fn control(network: Arc<Network>) -> Control {
    Control::open(None, 10, 1024, Vec::new())
        .expect("opens")
        .with_fetch(fetch::Fetch::on(network))
}

fn peer() -> Caller {
    Caller::Peer {
        id: "key".to_owned(),
        grants: BTreeSet::new(),
    }
}

#[test]
fn a_request_goes_through_the_proxy_with_its_login_and_the_answer_comes_back() {
    let world = world("net-fetch", &["host.example.test"], b"{\"peer\":\"x\"}");
    let control = control(through(&world));
    let answer = ask(
        &control,
        &Caller::Owner,
        "network/fetch",
        json!({"url": "https://host.example.test/canoa.json", "headers": [["accept", "application/json"]]}),
    );
    assert_eq!(answer["result"]["status"], 200, "{answer}");
    let body = answer["result"]["body"].as_str().expect("a body");
    assert_eq!(
        BASE64.decode(body.as_bytes()).expect("base64"),
        b"{\"peer\":\"x\"}"
    );
    assert_eq!(
        world.proxy.allowed(USER, PASSWORD),
        ["host.example.test:443"]
    );
}

#[test]
fn a_proxy_that_refuses_the_login_leaves_the_request_without_an_answer() {
    let world = world("net-fetch-refused", &["host.example.test"], b"never");
    let network = Arc::new(Network::new(&settings(&world, "wrong")).expect("a network"));
    let control = control(network);
    let answer = ask(
        &control,
        &Caller::Owner,
        "network/fetch",
        json!({"url": "https://host.example.test/", "seconds": 10}),
    );
    assert_eq!(answer["error"]["code"], REFUSED, "{answer}");
    assert!(world.proxy.allowed(USER, PASSWORD).is_empty());
}

#[test]
fn how_long_a_request_may_take_has_no_ceiling_of_the_hosts_own() {
    let world = world("net-fetch-long", &["host.example.test"], b"slow");
    let control = control(through(&world));
    let answer = ask(
        &control,
        &Caller::Owner,
        "network/fetch",
        json!({"url": "https://host.example.test/", "seconds": 86400}),
    );
    assert_eq!(answer["result"]["status"], 200, "{answer}");
}

#[test]
fn only_the_person_running_the_host_sends_through_its_network() {
    let world = world("net-fetch-key", &["host.example.test"], b"x");
    let control = control(through(&world));
    let answer = ask(
        &control,
        &peer(),
        "network/fetch",
        json!({"url": "https://host.example.test/"}),
    );
    assert_eq!(answer["error"]["code"], DENIED, "{answer}");
    assert!(world.proxy.seen().is_empty());
}

#[test]
fn a_request_that_is_not_http_or_not_well_formed_is_refused_before_anything_is_sent() {
    let world = world("net-fetch-bad", &["host.example.test"], b"x");
    let control = control(through(&world));
    for params in [
        json!({}),
        json!({"url": "ftp://host.example.test/"}),
        json!({"url": "file:///etc/passwd"}),
        json!({"url": "https://host.example.test/", "method": "NOT A METHOD"}),
        json!({"url": "https://host.example.test/", "body": "***"}),
        json!({"url": "https://host.example.test/", "headers": [["only-a-name"]]}),
        json!({"url": "https://host.example.test/", "seconds": 0}),
    ] {
        let answer = ask(&control, &Caller::Owner, "network/fetch", params.clone());
        assert_eq!(answer["error"]["code"], BAD_PARAMS, "{params}: {answer}");
    }
    assert!(world.proxy.seen().is_empty());
}
