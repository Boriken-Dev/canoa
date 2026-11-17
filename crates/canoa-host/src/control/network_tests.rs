//! `registry/fetch` and `skill/scan` through the stand-in proxy with its login, over loopback.

use std::sync::Arc;

use super::overlays_tests::{ask, dir};
use super::*;
use crate::network::tests::{PASSWORD, USER, through, world};
use crate::store::tools::HttpTransport;

#[test]
fn a_registry_fetch_goes_through_the_proxy_with_its_login() {
    let world = world("registry", &["registry.example.test"], b"unused");
    let control = Control::open(None, 10, 1024, Vec::new())
        .expect("opens")
        .with_registry(
            Registry::new(None).with_transport(Arc::new(HttpTransport::on(through(&world)))),
        );
    let answer = ask(
        &control,
        &Caller::Owner,
        "registry/fetch",
        json!({"url": "https://registry.example.test/v0.1/servers?limit=30"}),
    );
    assert_eq!(answer["result"]["body"], r#"{"servers":[]}"#, "{answer}");
    assert_eq!(
        world.proxy.allowed(USER, PASSWORD),
        ["registry.example.test:443"]
    );
}

#[test]
fn a_skill_scan_starts_git_with_the_proxy_and_its_login() {
    // The proxy has no origin, so git's request is answered with a refusal: what it proves is where git sent it.
    let world = world("scan", &["skills.example.test"], b"unused");
    let proxy = crate::network::standins::StandInProxy::start(USER, PASSWORD, None);
    let tmp = dir("net-scan");
    let overlays = tmp.join("cfg").join("overlays");
    // A person's own proxy variables would be kept, so the test names none.
    let none: Vec<(&str, std::ffi::OsString)> = [
        "HTTP_PROXY",
        "http_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
    ]
    .into_iter()
    .map(|name| (name, std::ffi::OsString::new()))
    .collect();
    let mut settings = crate::network::tests::settings(&world, PASSWORD);
    settings.proxy = canoa::network::ProxyChoice::Proxy(format!("http://{}", proxy.addr));
    let network = Arc::new(canoa::network::Network::new(&settings).expect("a network"));
    let control = Control::open(
        None,
        DEFAULT_MAX_RECORDS,
        DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens")
    .with_overlays(
        Overlays::new(Some(overlays), OverlayLimits::default())
            .with_person_env(&none)
            .with_network(network),
    );
    let answer = ask(
        &control,
        &Caller::Owner,
        "skill/scan",
        json!({"repo": "https://skills.example.test/skills.git", "ref": "main", "path": ""}),
    );
    assert!(answer.get("error").is_some(), "{answer}");
    assert_eq!(
        proxy.allowed(USER, PASSWORD),
        ["skills.example.test:443"],
        "{:?}",
        proxy.seen()
    );
}
