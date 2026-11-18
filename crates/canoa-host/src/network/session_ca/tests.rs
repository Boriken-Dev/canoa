//! The authority a session's proxy signs with: kept once per home for its owner alone, made again when a year old, and a certificate
//! for each destination signed with a key of its own.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a test names what it uses from std"
)]

use std::fs;
use std::time::{Duration, SystemTime};

use rustls::pki_types::CertificateDer;
use rustls::pki_types::pem::PemObject as _;

use super::SessionCa;
use crate::network::tests::Dir;

fn file_of(home: &Dir) -> std::path::PathBuf {
    home.0.join("network").join("session-ca").join("ca.pem")
}

#[test]
fn an_authority_is_made_once_for_a_home_and_kept_with_its_key_in_one_file() {
    let home = Dir::new("ca-once");
    let first = SessionCa::open(Some(&home.0)).expect("made");
    let second = SessionCa::open(Some(&home.0)).expect("read again");
    assert_eq!(first.certificate_pem(), second.certificate_pem());
    let kept = fs::read_to_string(file_of(&home)).expect("kept");
    assert!(kept.starts_with(first.certificate_pem()));
    assert!(kept.contains("BEGIN PRIVATE KEY"));
    assert!(
        !first.certificate_pem().contains("PRIVATE"),
        "what a program is given has no key"
    );
    let other = Dir::new("ca-other");
    let elsewhere = SessionCa::open(Some(&other.0)).expect("made");
    assert_ne!(first.certificate_pem(), elsewhere.certificate_pem());
    let in_memory = SessionCa::open(None).expect("made");
    assert_ne!(first.certificate_pem(), in_memory.certificate_pem());
}

#[cfg(unix)]
#[test]
fn the_file_and_its_directory_are_the_owners_alone() {
    use std::os::unix::fs::PermissionsExt as _;
    let home = Dir::new("ca-mode");
    SessionCa::open(Some(&home.0)).expect("made");
    let file = file_of(&home);
    let mode =
        |path: &std::path::Path| fs::metadata(path).expect("exists").permissions().mode() & 0o777;
    assert_eq!(mode(&file), 0o600);
    assert_eq!(mode(file.parent().expect("a directory")), 0o700);
}

#[test]
fn an_authority_a_year_old_or_a_file_that_is_not_one_is_replaced() {
    let home = Dir::new("ca-old");
    let first = SessionCa::open(Some(&home.0)).expect("made");
    let file = file_of(&home);
    let handle = fs::OpenOptions::new()
        .write(true)
        .open(&file)
        .expect("opens");
    handle
        .set_modified(SystemTime::now() - Duration::from_secs(400 * 24 * 3600))
        .expect("ages it");
    drop(handle);
    let renewed = SessionCa::open(Some(&home.0)).expect("made again");
    assert_ne!(first.certificate_pem(), renewed.certificate_pem());
    fs::write(&file, "not an authority").expect("spoils it");
    let again = SessionCa::open(Some(&home.0)).expect("made again");
    assert_ne!(renewed.certificate_pem(), again.certificate_pem());
    assert!(
        fs::read_to_string(&file)
            .expect("kept")
            .starts_with(again.certificate_pem())
    );
}

#[test]
fn each_certificate_for_a_destination_has_a_key_of_its_own_and_an_address_is_carried_as_one() {
    let ca = SessionCa::open(None).expect("made");
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
    let ca = SessionCa::open(None).expect("made");
    let bundle = ca.with_system_roots();
    assert!(bundle.starts_with(ca.certificate_pem()));
    let all = CertificateDer::pem_slice_iter(bundle.as_bytes())
        .flatten()
        .count();
    assert!(all >= 1);
    assert!(!bundle.contains("PRIVATE"));
}
