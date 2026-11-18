//! The authority a host's proxies sign with: made once for each limit in memory, limited to the hosts a profile names when every
//! pattern is a plain host, unlimited when one is a regular expression, and a certificate for each destination made on demand with a
//! key of its own and kept until shortly before it expires.
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

use super::ProxyCa;

fn limited(hosts: &[String]) -> Result<ProxyCa, String> {
    ProxyCa::new(Some(hosts))
}

fn hosts(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

/// Whether a client that trusts `ca` and nothing else accepts the certificate the authority signs for `host`: what a program the
/// session gave the authority's certificate to does, name constraints included.
fn vouches(ca: &ProxyCa, host: &str) -> bool {
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
        limited(&names).expect("made"),
        limited(&names).expect("made"),
    );
    assert_ne!(first.certificate_pem(), second.certificate_pem());
    assert!(!first.certificate_pem().contains("PRIVATE"));
    assert!(!first.with_system_roots().contains("PRIVATE"));
    assert!(limited(&[]).is_err());
}

#[test]
fn a_certificate_the_authority_signs_is_accepted_for_the_hosts_the_profile_names_and_no_others() {
    let ca = limited(&hosts(&[
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
    let names = limited(&hosts(&["api.example.test"])).expect("made");
    assert!(vouches(&names, "api.example.test"));
    for address in ["10.0.0.7", "127.0.0.1", "::1"] {
        assert!(!vouches(&names, address), "{address}");
    }
    let addresses = limited(&hosts(&["10.0.0.7"])).expect("made");
    assert!(vouches(&addresses, "10.0.0.7"));
    for no in ["api.example.test", "localhost", "10.0.0.8", "::1"] {
        assert!(!vouches(&addresses, no), "{no}");
    }
    let v6 = limited(&hosts(&["::1"])).expect("made");
    assert!(vouches(&v6, "::1"));
    assert!(!vouches(&v6, "127.0.0.1"));
}

#[test]
fn a_name_is_permitted_with_what_is_under_it_which_a_dns_constraint_cannot_stop_short_of() {
    let ca = limited(&hosts(&["api.example.test"])).expect("made");
    assert!(vouches(&ca, "x.api.example.test"));
    assert!(!vouches(&ca, "example.test"));
}

#[test]
fn the_constraint_is_in_the_certificate_and_critical_so_a_client_that_ignored_it_refuses_the_certificate()
 {
    let ca = limited(&hosts(&["api.example.test"])).expect("made");
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
    let ca = limited(&hosts(&["api.example.test", "10.0.0.7", "::1"])).expect("made");
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
    let ca = limited(&hosts(&["api.example.test"])).expect("made");
    let bundle = ca.with_system_roots();
    assert!(bundle.starts_with(ca.certificate_pem()));
    let all = CertificateDer::pem_slice_iter(bundle.as_bytes())
        .flatten()
        .count();
    assert!(all >= 1);
    assert!(!bundle.contains("PRIVATE"));
}

#[test]
fn the_process_makes_one_authority_for_each_limit_and_gives_every_proxy_that_asks_the_same_one() {
    let a = ProxyCa::shared(Some(&hosts(&["Api.Example.test", "*.corp.example"]))).expect("made");
    let again = ProxyCa::shared(Some(&hosts(&[
        "*.corp.example",
        "api.example.test",
        "api.example.test",
    ])))
    .expect("made");
    assert!(
        Arc::ptr_eq(&a, &again),
        "the same set of hosts, in any order and case, is one authority"
    );
    let other = ProxyCa::shared(Some(&hosts(&["other.example.test"]))).expect("made");
    assert!(!Arc::ptr_eq(&a, &other));
    assert_ne!(a.certificate_pem(), other.certificate_pem());
    let everything = ProxyCa::shared(None).expect("made");
    assert!(Arc::ptr_eq(
        &everything,
        &ProxyCa::shared(None).expect("made")
    ));
    assert!(!Arc::ptr_eq(&everything, &a));
    assert_ne!(everything.certificate_pem(), a.certificate_pem());
}

#[test]
fn an_authority_with_no_limit_vouches_for_every_name_and_address_and_one_with_a_limit_does_not() {
    let everything = ProxyCa::new(None).expect("made");
    for yes in [
        "api.example.test",
        "localhost",
        "10.0.0.7",
        "::1",
        "anything.example",
    ] {
        assert!(vouches(&everything, yes), "{yes}");
    }
    let der = CertificateDer::pem_slice_iter(everything.certificate_pem().as_bytes())
        .next()
        .expect("one")
        .expect("a certificate");
    let marker = [0x06, 0x03, 0x55, 0x1D, 0x1E];
    assert!(
        !der.as_ref()
            .windows(marker.len())
            .any(|window| window == marker),
        "no nameConstraints extension"
    );
    assert!(!everything.certificate_pem().contains("PRIVATE"));
}

#[test]
fn a_certificate_made_for_a_destination_is_kept_and_reused_until_shortly_before_it_expires() {
    let ca = ProxyCa::new(None).expect("made");
    let start = super::now();
    let first = ca.answer_at("Api.Example.test.", start).expect("answers");
    let second = ca
        .answer_at("api.example.test", start + 3600)
        .expect("answers");
    assert!(
        Arc::ptr_eq(&first, &second),
        "the same certificate and key, whatever the spelling of the name"
    );
    assert_eq!(ca.made(), 1);
    ca.answer_at("10.0.0.7", start).expect("answers");
    ca.answer_at("10.0.0.7", start).expect("answers");
    assert_eq!(
        ca.made(),
        2,
        "one for each destination, an address included"
    );
    // A day before it expires it is made again, and the new one is kept in its place.
    let near = start + 30 * 24 * 3600 - 3600;
    let renewed = ca.answer_at("api.example.test", near).expect("answers");
    assert!(!Arc::ptr_eq(&first, &renewed));
    assert_eq!(ca.made(), 3);
    assert!(Arc::ptr_eq(
        &renewed,
        &ca.answer_at("api.example.test", near + 60)
            .expect("answers")
    ));
    assert_eq!(ca.made(), 3);
    // A name no certificate can carry is no answer, and nothing is kept for it.
    assert!(ca.answer("not a name").is_err());
    assert_eq!(ca.made(), 3);
}

#[test]
fn a_certificate_lives_less_long_than_the_authority_and_never_past_it() {
    let ca = ProxyCa::new(Some(&hosts(&["api.example.test"]))).expect("made");
    let leaf = ca
        .leaf_at("api.example.test", super::now())
        .expect("signed");
    assert!(leaf.expires <= super::now() + 31 * 24 * 3600);
    assert!(leaf.expires < ca.not_after.unix_timestamp());
    // Asked for at the end of the authority's own life, it is not given more.
    let late = ca.not_after.unix_timestamp() - 3600;
    let last = ca.leaf_at("api.example.test", late).expect("signed");
    assert_eq!(last.expires, ca.not_after.unix_timestamp());
}
