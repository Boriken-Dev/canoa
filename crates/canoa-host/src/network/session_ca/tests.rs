//! The authority a session's proxy signs with: made for one session in memory, limited to the hosts the profile names, and a
//! certificate for each destination signed with a key of its own.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a test names what it uses from std"
)]

use std::sync::Arc;

use rustls::RootCertStore;
use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::ServerCertVerifier as _;
use rustls::pki_types::pem::PemObject as _;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};

use super::SessionCa;

fn hosts(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

/// Whether a client that trusts `ca` and nothing else accepts the certificate the authority signs for `host`: what a program the
/// session gave the authority's certificate to does, name constraints included.
fn vouches(ca: &SessionCa, host: &str) -> bool {
    let mut roots = RootCertStore::empty();
    for der in CertificateDer::pem_slice_iter(ca.certificate_pem().as_bytes()).flatten() {
        roots.add(der).expect("a root");
    }
    let verifier = WebPkiServerVerifier::builder_with_provider(
        Arc::new(roots),
        Arc::new(rustls::crypto::ring::default_provider()),
    )
    .build()
    .expect("a verifier");
    let leaf = ca.leaf(host).expect("signed");
    verifier
        .verify_server_cert(
            &leaf.chain[0],
            &[],
            &ServerName::try_from(host.to_owned()).expect("a name"),
            &[],
            UnixTime::now(),
        )
        .is_ok()
}

#[test]
fn each_authority_is_new_holds_no_key_in_what_a_program_is_given_and_needs_a_host() {
    let names = hosts(&["api.example.test"]);
    let (first, second) = (
        SessionCa::new(&names).expect("made"),
        SessionCa::new(&names).expect("made"),
    );
    assert_ne!(first.certificate_pem(), second.certificate_pem());
    assert!(!first.certificate_pem().contains("PRIVATE"));
    assert!(!first.with_system_roots().contains("PRIVATE"));
    assert!(SessionCa::new(&[]).is_err());
}

#[test]
fn a_certificate_the_authority_signs_is_accepted_for_the_hosts_the_profile_names_and_no_others() {
    let ca = SessionCa::new(&hosts(&[
        "Api.Example.test",
        "*.corp.example",
        "10.0.0.7",
        "2001:db8::7",
    ]))
    .expect("made");
    for yes in [
        "api.example.test",
        "corp.example",
        "a.b.corp.example",
        "10.0.0.7",
        "2001:db8::7",
    ] {
        assert!(vouches(&ca, yes), "{yes} is named");
    }
    for no in [
        "other.example.test",
        "example.test",
        "evilcorp.example",
        "corp.example.evil.test",
        "localhost",
        "10.0.0.8",
        "10.0.0.70",
        "127.0.0.1",
        "::1",
        "2001:db8::8",
    ] {
        assert!(!vouches(&ca, no), "{no} is not named");
    }
}

#[test]
fn an_authority_for_names_alone_vouches_for_no_address_and_one_for_addresses_alone_for_no_name() {
    let names = SessionCa::new(&hosts(&["api.example.test"])).expect("made");
    assert!(vouches(&names, "api.example.test"));
    for address in ["10.0.0.7", "127.0.0.1", "::1"] {
        assert!(!vouches(&names, address), "{address}");
    }
    let addresses = SessionCa::new(&hosts(&["10.0.0.7"])).expect("made");
    assert!(vouches(&addresses, "10.0.0.7"));
    for no in ["api.example.test", "localhost", "10.0.0.8", "::1"] {
        assert!(!vouches(&addresses, no), "{no}");
    }
    let v6 = SessionCa::new(&hosts(&["::1"])).expect("made");
    assert!(vouches(&v6, "::1"));
    assert!(!vouches(&v6, "127.0.0.1"));
}

#[test]
fn a_name_is_permitted_with_what_is_under_it_which_a_dns_constraint_cannot_stop_short_of() {
    let ca = SessionCa::new(&hosts(&["api.example.test"])).expect("made");
    assert!(vouches(&ca, "x.api.example.test"));
    assert!(!vouches(&ca, "example.test"));
}

#[test]
fn the_constraint_is_in_the_certificate_and_critical_so_a_client_that_ignored_it_refuses_the_certificate()
 {
    let ca = SessionCa::new(&hosts(&["api.example.test"])).expect("made");
    let der = CertificateDer::pem_slice_iter(ca.certificate_pem().as_bytes())
        .next()
        .expect("one")
        .expect("a certificate");
    // The extension's identifier (2.5.29.30) followed by its `critical` flag, set.
    let marker = [0x06, 0x03, 0x55, 0x1D, 0x1E, 0x01, 0x01, 0xFF];
    assert!(
        der.as_ref()
            .windows(marker.len())
            .any(|window| window == marker),
        "a critical nameConstraints extension"
    );
}

#[test]
fn each_certificate_for_a_destination_has_a_key_of_its_own_and_an_address_is_carried_as_one() {
    let ca = SessionCa::new(&hosts(&["api.example.test", "10.0.0.7", "::1"])).expect("made");
    let (one, two) = (
        ca.leaf("api.example.test").expect("signed"),
        ca.leaf("api.example.test").expect("signed"),
    );
    assert_ne!(one.chain[0].as_ref(), two.chain[0].as_ref());
    ca.leaf("10.0.0.7").expect("an address is carried");
    ca.leaf("::1").expect("so is a v6 one");
    assert!(ca.leaf("not a name").is_err());
}

#[test]
fn the_bundle_for_a_program_that_replaces_its_roots_begins_with_the_authority() {
    let ca = SessionCa::new(&hosts(&["api.example.test"])).expect("made");
    let bundle = ca.with_system_roots();
    assert!(bundle.starts_with(ca.certificate_pem()));
    let all = CertificateDer::pem_slice_iter(bundle.as_bytes())
        .flatten()
        .count();
    assert!(all >= 1);
    assert!(!bundle.contains("PRIVATE"));
}
