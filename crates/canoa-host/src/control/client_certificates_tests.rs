//! `network/client_certificates` over stand-in keystores: what a profile's proxy item is offered to choose between.

use std::sync::Arc;

use canoa::network::{ClientCertificate, Network, NetworkSettings, ProxyChoice};
use coqui_syshttp::tls::{ClientCertificates, Identity, Selector, TlsBuilder};

use super::overlays_tests::{ask, dir};
use super::*;
use crate::network::standins::Ca;
use crate::network::tests::direct;

fn identity(ca: &Ca, name: &str) -> Identity {
    let leaf = ca.issue_client(name);
    Identity::from_pem(leaf.chain_pem.as_bytes(), leaf.key_pem.as_bytes()).expect("one")
}

fn hex(identity: &Identity) -> String {
    data_encoding::HEXLOWER.encode(identity.info().fingerprint())
}

fn control_on(network: Network) -> Control {
    Control::open(
        None,
        DEFAULT_MAX_RECORDS,
        DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens")
    .with_client_certificates(client_certificates::ClientCertificates::on(Arc::new(
        network,
    )))
}

#[test]
fn the_certificates_the_keystores_hold_are_listed_by_what_they_say_of_themselves_and_never_a_key() {
    let ca = Ca::new("Origin CA");
    let (alice, bob) = (identity(&ca, "Alice Example"), identity(&ca, "Bob Example"));
    let selector = Selector::builder()
        .source(vec![alice.clone(), bob.clone()])
        .build();
    let tls =
        TlsBuilder::default().client_certificates(ClientCertificates::Selector(Arc::new(selector)));
    let control = control_on(Network::with(direct(), tls));
    let answer = ask(
        &control,
        &Caller::Owner,
        "network/client_certificates",
        json!({}),
    );
    let listed = answer["result"]["certificates"].as_array().expect("a list");
    assert_eq!(listed.len(), 2, "{answer}");
    for (identity, name) in [(&alice, "Alice Example"), (&bob, "Bob Example")] {
        let row = listed
            .iter()
            .find(|row| row["thumbprint"] == hex(identity).as_str())
            .unwrap_or_else(|| panic!("{name} is listed"));
        assert!(
            row["subject"]
                .as_str()
                .is_some_and(|subject| subject.contains(name)),
            "{row}"
        );
        assert!(
            row["issuer"]
                .as_str()
                .is_some_and(|issuer| issuer.contains("Origin CA")),
            "{row}"
        );
        assert!(
            row["not_after"].as_u64().is_some_and(|ends| ends > 0),
            "{row}"
        );
        assert!(row["source"].is_string(), "{row}");
    }
    let text = answer.to_string();
    assert!(
        !text.contains("PRIVATE") && !text.contains("key_pem"),
        "no key in the answer"
    );
}

#[test]
fn the_certificate_file_of_host_network_is_listed_and_none_lists_nothing() {
    let ca = Ca::new("Origin CA");
    let leaf = ca.issue_client("Carol Example");
    let tmp = dir("client-certificates");
    let file = tmp.join("me.pem");
    std::fs::write(&file, format!("{}{}", leaf.chain_pem, leaf.key_pem)).expect("writes");
    let mut settings = NetworkSettings::default();
    settings.proxy = ProxyChoice::None;
    settings.client_certificate = ClientCertificate::File(file);
    let control = control_on(Network::new(&settings).expect("a network"));
    let answer = ask(
        &control,
        &Caller::Owner,
        "network/client_certificates",
        json!({}),
    );
    let listed = answer["result"]["certificates"].as_array().expect("a list");
    assert_eq!(listed.len(), 1, "{answer}");
    assert!(
        listed[0]["subject"]
            .as_str()
            .is_some_and(|subject| subject.contains("Carol Example"))
    );
    assert!(!answer.to_string().contains("PRIVATE"));
    settings.client_certificate = ClientCertificate::None;
    let control = control_on(Network::new(&settings).expect("a network"));
    let answer = ask(
        &control,
        &Caller::Owner,
        "network/client_certificates",
        json!({}),
    );
    assert_eq!(answer["result"]["certificates"], json!([]), "{answer}");
}

#[test]
fn a_key_that_may_not_write_the_configuration_is_not_told_which_certificates_the_person_holds() {
    let control = control_on(Network::with(direct(), TlsBuilder::default()));
    let reader = Caller::Peer {
        id: "sha256:aa".to_owned(),
        grants: [Grant::ConfigRead].into_iter().collect(),
    };
    let answer = ask(&control, &reader, "network/client_certificates", json!({}));
    assert!(answer.get("error").is_some(), "{answer}");
    let writer = Caller::Peer {
        id: "sha256:bb".to_owned(),
        grants: [Grant::ConfigWrite].into_iter().collect(),
    };
    let answer = ask(&control, &writer, "network/client_certificates", json!({}));
    assert!(answer.get("result").is_some(), "{answer}");
}
