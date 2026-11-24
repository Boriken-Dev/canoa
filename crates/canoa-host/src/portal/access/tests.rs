#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::path::PathBuf;

use coqui_access::{AccountPut, Accounts as AccountKind};
use sha2::{Digest as _, Sha256};

use super::super::signin::PinState;
use super::*;

fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// The library cargo built beside the test binary for the dev-dependency `name`; its file name carries a hash.
fn plugin(name: &str) -> PathBuf {
    let prefix = format!("lib{name}_plugin-");
    std::fs::read_dir(exe_dir())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|file| {
            let name = file.file_name().to_string_lossy().into_owned();
            name.starts_with(&prefix) && name.ends_with(".so")
        })
        .max_by_key(|file| file.metadata().and_then(|meta| meta.modified()).ok())
        .map(|file| file.path())
        .unwrap_or_else(|| panic!("{name}'s plugin is built beside the tests"))
}

/// This process's user id: an account every machine has, so the system directory has someone to admit.
fn mine() -> u32 {
    std::os::unix::fs::MetadataExt::uid(&std::fs::metadata("/proc/self").unwrap())
}

fn my_name() -> String {
    let system = SystemAccounts::builder().min_uid(0).build().unwrap();
    system
        .lookup(Lookup {
            by: "id".to_owned(),
            value: mine().to_string(),
            claims: None,
        })
        .unwrap()
        .expect("this process's user is in the directory")
        .name
}

fn file_source(path: &Path) -> serde_json::Value {
    serde_json::json!({"source": {"file": {"path": path, "format": "toml"}}})
}

/// Writes `name` into the provider's file with `secret`, through the provider's own account kind.
fn seed(library: &str, id: &str, config: &serde_json::Value, name: &str, secret: &str) {
    let mut registry = Registry::new("seed", "0");
    open_library(&mut registry, &plugin(library)).unwrap();
    let config = value_of(config.as_object().unwrap()).unwrap();
    let accounts: Arc<dyn AccountKind> = Arc::new(
        registry
            .offer::<dyn AccountKind>(id)
            .unwrap()
            .unwrap()
            .instantiate(&config)
            .unwrap(),
    );
    accounts
        .put(AccountPut {
            name: name.to_owned(),
            secret: Some(secret.to_owned()),
            ..AccountPut::default()
        })
        .unwrap();
}

fn entry(provider: &str, library: Option<PathBuf>, config: &serde_json::Value) -> Entry {
    let mut entry = Entry::new(provider);
    entry.library = library;
    entry.config = config.as_object().cloned().unwrap_or_default();
    entry
}

fn settings(entries: Vec<Entry>) -> Settings {
    Settings {
        min_uid: mine().max(1),
        authenticators: entries,
        // The libraries are built under the tester's own target directory.
        libraries_any_owner: true,
        ..Settings::default()
    }
}

fn client() -> IpAddr {
    "198.51.100.7".parse().unwrap()
}

fn password_entry(file: &Path) -> Entry {
    entry(
        "coqui_access_password",
        Some(plugin("coqui_access_password")),
        &file_source(file),
    )
}

fn mtls_entry(file: &Path) -> Entry {
    entry(
        "coqui_access_mtls",
        Some(plugin("coqui_access_mtls")),
        &file_source(file),
    )
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("canoa-access-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn seed_password(file: &Path, name: &str, password: &str) {
    seed(
        "coqui_access_password",
        "coqui_access_password",
        &file_source(file),
        name,
        password,
    );
}

#[test]
fn a_password_from_a_file_signs_in_the_system_account_of_that_name() {
    let dir = scratch("password");
    let file = dir.join("accounts.toml");
    let me = my_name();
    seed_password(&file, &me, "file-pw");
    let access = Access::new(&settings(vec![password_entry(&file)])).unwrap();
    assert_eq!(access.methods(), [Method::Password]);
    admits(
        access.sign_in(&me, "file-pw", &[]),
        &me,
        "coqui_access_password",
    );
    assert_eq!(access.sign_in(&me, "wrong", &[]), SignIn::Refused);
    assert_eq!(
        access.sign_in("nobody-here", "file-pw", &[]),
        SignIn::Refused
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_account_a_method_names_must_pass_the_systems_groups_and_uid() {
    let dir = scratch("admit");
    let file = dir.join("accounts.toml");
    let me = my_name();
    seed_password(&file, &me, "file-pw");
    let mut outside = settings(vec![password_entry(&file)]);
    outside.groups = vec!["canoa-no-such-group".to_owned()];
    assert_eq!(
        Access::new(&outside).unwrap().sign_in(&me, "file-pw", &[]),
        SignIn::Refused
    );
    let mut above = settings(vec![password_entry(&file)]);
    above.min_uid = mine() + 1;
    assert_eq!(
        Access::new(&above).unwrap().sign_in(&me, "file-pw", &[]),
        SignIn::Refused
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn unknown_passes_the_credential_on_and_invalid_ends_the_try() {
    let dir = scratch("order");
    let (first, second) = (dir.join("first.toml"), dir.join("second.toml"));
    let me = my_name();
    seed_password(&first, &me, "pw-one");
    seed_password(&second, &me, "pw-two");
    // Invalid: the first provider owns the name and refuses `pw-two`, so the second is never asked.
    let both = Access::new(&settings(vec![
        password_entry(&first),
        password_entry(&second),
    ]))
    .unwrap();
    assert!(matches!(
        both.sign_in(&me, "pw-one", &[]),
        SignIn::Admitted { .. }
    ));
    assert_eq!(both.sign_in(&me, "pw-two", &[]), SignIn::Refused);
    // Unknown: the system provider, limited to a group this account is not in, says it is not its to judge.
    let narrow = entry(
        SYSTEM,
        None,
        &serde_json::json!({"groups": ["canoa-no-such-group"]}),
    );
    let passed_on = Access::new(&settings(vec![narrow, password_entry(&second)])).unwrap();
    assert!(matches!(
        passed_on.sign_in(&me, "pw-two", &[]),
        SignIn::Admitted { .. }
    ));
    std::fs::remove_dir_all(dir).unwrap();
}

/// `result` is a sign-in of `me`, by this process's user id, accepted by `provider`.
fn admits(result: SignIn, me: &str, provider: &str) {
    match result {
        SignIn::Admitted {
            user,
            uid,
            provider: by,
            ..
        } => assert_eq!((user.as_str(), uid, by.as_str()), (me, mine(), provider)),
        other => panic!("not admitted: {other:?}"),
    }
}

fn certificate() -> (Vec<u8>, String) {
    let made = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
    let der = made.cert.der().to_vec();
    let fingerprint = format!(
        "sha256:{}",
        data_encoding::HEXLOWER.encode(&Sha256::digest(&der))
    );
    (der, fingerprint)
}

fn header(value: &str) -> Vec<(String, String)> {
    vec![("x-client-cert".to_owned(), value.to_owned())]
}

fn pinned(file: &Path, groups: &[&str]) -> (Access, String, Vec<u8>) {
    let me = my_name();
    let (der, fingerprint) = certificate();
    seed(
        "coqui_access_mtls",
        "coqui_access_mtls",
        &file_source(file),
        &me,
        &fingerprint,
    );
    let mut settings = settings(vec![mtls_entry(file)]);
    settings.groups = groups.iter().map(|group| (*group).to_owned()).collect();
    (Access::new(&settings).unwrap(), me, der)
}

#[test]
fn a_pinned_certificate_signs_in_its_account_and_any_other_is_refused() {
    let dir = scratch("pinned");
    let (access, me, der) = pinned(&dir.join("peers.toml"), &[]);
    assert_eq!(access.methods(), [Method::Certificate]);
    let b64 = data_encoding::BASE64.encode(&der);
    admits(
        access.sign_in_certificate(&header(&b64), client(), &[]),
        &me,
        "coqui_access_mtls",
    );
    // A certificate nobody pinned, however well formed.
    let (other, _) = certificate();
    let other = data_encoding::BASE64.encode(&other);
    assert_eq!(
        access.sign_in_certificate(&header(&other), client(), &[]),
        SignIn::Unpinned
    );
    // Nothing is told apart from garbage, two headers, and a chain longer than the portal reads.
    assert!(!access.carries_certificate(&[]));
    assert_eq!(
        access.sign_in_certificate(&[], client(), &[]),
        SignIn::NoCertificate
    );
    assert_eq!(
        access.sign_in_certificate(&header("not base64!"), client(), &[]),
        SignIn::Refused
    );
    let twice = vec![
        ("x-client-cert".to_owned(), b64.clone()),
        ("x-client-cert".to_owned(), b64.clone()),
    ];
    assert_eq!(
        access.sign_in_certificate(&twice, client(), &[]),
        SignIn::Refused
    );
    let long = [b64.as_str(); 9].join(",");
    assert_eq!(
        access.sign_in_certificate(&header(&long), client(), &[]),
        SignIn::Refused
    );
    // A password goes to no certificate provider.
    assert_eq!(access.sign_in(&me, "anything", &[]), SignIn::Refused);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_pinned_certificate_of_an_account_outside_the_groups_is_refused() {
    let dir = scratch("pinned-groups");
    let (access, _, der) = pinned(&dir.join("peers.toml"), &["canoa-no-such-group"]);
    assert_eq!(
        access.sign_in_certificate(&header(&data_encoding::BASE64.encode(&der)), client(), &[]),
        SignIn::Refused
    );
    std::fs::remove_dir_all(dir).unwrap();
}

fn enrolling(file: &Path) -> Access {
    let mut entry = mtls_entry(file);
    entry.enrol = true;
    Access::new(&settings(vec![entry])).unwrap()
}

/// The provider adds a pin to an account that has one; enrolling a certificate means it is the account's only one, so the
/// one it replaces stops signing in, and removing takes the whole record.
#[test]
fn pinning_a_certificate_replaces_the_one_before_it_in_the_real_provider() {
    let dir = scratch("replace");
    let file = dir.join("peers.toml");
    let me = my_name();
    let (other_der, other_fingerprint) = certificate();
    seed(
        "coqui_access_mtls",
        "coqui_access_mtls",
        &file_source(&file),
        "someone-else",
        &other_fingerprint,
    );
    let access = enrolling(&file);
    let (first, _) = certificate();
    let (second, _) = certificate();
    let (third, _) = certificate();
    let headers: Vec<_> = [&first, &second, &third]
        .into_iter()
        .map(|der| header(&data_encoding::BASE64.encode(der)))
        .collect();
    for (index, enrolled) in headers.iter().enumerate() {
        access.pin(&me, mine(), enrolled).unwrap();
        for (other, probe) in headers.iter().enumerate().take(index + 1) {
            let signed = access.sign_in_certificate(probe, client(), &[]);
            if other == index {
                admits(signed, &me, "coqui_access_mtls");
            } else {
                assert_eq!(signed, SignIn::Unpinned, "pin {other} after pin {index}");
            }
        }
    }
    // Pinning the certificate it already has changes nothing.
    access.pin(&me, mine(), &headers[2]).unwrap();
    admits(
        access.sign_in_certificate(&headers[2], client(), &[]),
        &me,
        "coqui_access_mtls",
    );
    // Removing takes the whole record, and another account's is untouched.
    assert_eq!(access.unpin(&me), Ok(true));
    for probe in &headers {
        assert_eq!(
            access.sign_in_certificate(probe, client(), &[]),
            SignIn::Unpinned
        );
    }
    assert_eq!(access.unpin(&me), Ok(false));
    let _ = other_der;
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_certificate_is_pinned_and_removed_through_the_providers_accounts() {
    let dir = scratch("enrol");
    let file = dir.join("peers.toml");
    let me = my_name();
    // Someone else's certificate, pinned first: it also makes the file.
    let (other_der, other_fingerprint) = certificate();
    seed(
        "coqui_access_mtls",
        "coqui_access_mtls",
        &file_source(&file),
        "someone-else",
        &other_fingerprint,
    );
    let access = enrolling(&file);
    let (der, fingerprint) = certificate();
    let mine_header = header(&data_encoding::BASE64.encode(&der));
    let other_header = header(&data_encoding::BASE64.encode(&other_der));
    assert_eq!(
        access.sign_in_certificate(&mine_header, client(), &[]),
        SignIn::Unpinned
    );
    let status = access
        .certificate_status(&me, mine(), &mine_header)
        .unwrap();
    let (presented, state) = status.presented.unwrap();
    assert_eq!(
        (presented.fingerprint.as_str(), state),
        (fingerprint.as_str(), PinState::Unpinned)
    );
    assert!(!status.pinned);
    // Pinned, the certificate alone signs the account in.
    access.pin(&me, mine(), &mine_header).unwrap();
    admits(
        access.sign_in_certificate(&mine_header, client(), &[]),
        &me,
        "coqui_access_mtls",
    );
    let status = access
        .certificate_status(&me, mine(), &mine_header)
        .unwrap();
    assert_eq!(status.presented.unwrap().1, PinState::Yours);
    assert!(status.pinned);
    // One pinned to another account is that account's, and cannot be taken.
    assert_eq!(
        access
            .certificate_status(&me, mine(), &other_header)
            .unwrap()
            .presented
            .unwrap()
            .1,
        PinState::Another
    );
    assert_eq!(access.pin(&me, mine(), &other_header), Err(PinError::Taken));
    assert_eq!(access.pin(&me, mine(), &[]), Err(PinError::NoCertificate));
    assert_eq!(
        access.pin(&me, mine(), &header("not base64!")),
        Err(PinError::NoCertificate)
    );
    // Removed, it signs no one in, and removing again finds nothing.
    assert_eq!(access.unpin(&me), Ok(true));
    assert_eq!(
        access.sign_in_certificate(&mine_header, client(), &[]),
        SignIn::Unpinned
    );
    assert_eq!(access.unpin(&me), Ok(false));
    assert!(
        !access
            .certificate_status(&me, mine(), &mine_header)
            .unwrap()
            .pinned
    );
    // The other account's pin was never touched.
    assert_eq!(
        access
            .certificate_status(&me, mine(), &other_header)
            .unwrap()
            .presented
            .unwrap()
            .1,
        PinState::Another
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_portal_that_keeps_no_enrolled_certificates_pins_none() {
    let dir = scratch("no-enrol");
    let (access, me, der) = pinned(&dir.join("peers.toml"), &[]);
    let h = header(&data_encoding::BASE64.encode(&der));
    assert_eq!(access.certificate_status(&me, mine(), &h), None);
    assert_eq!(access.pin(&me, mine(), &h), Err(PinError::Unsupported));
    assert_eq!(access.unpin(&me), Err(PinError::Unsupported));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_password_file_passes_on_a_name_it_does_not_hold_and_refuses_a_wrong_password_for_one_it_does()
{
    // The password provider answers Unknown for a name its file lacks, so the providers after it are asked, and Invalid for
    // a wrong password of a name it holds, which ends the try.
    let dir = scratch("lockout");
    let (first, second) = (dir.join("first.toml"), dir.join("second.toml"));
    let me = my_name();
    seed_password(&first, "someone-else", "pw-x");
    seed_password(&second, &me, "pw-two");
    let first_listed = Access::new(&settings(vec![
        password_entry(&first),
        password_entry(&second),
    ]))
    .unwrap();
    admits(
        first_listed.sign_in(&me, "pw-two", &[]),
        &me,
        "coqui_access_password",
    );
    assert_eq!(
        first_listed.sign_in("someone-else", "pw-x", &[]),
        SignIn::Refused
    );
    // A name the first file holds with another password is refused there, and the second is not asked.
    seed_password(&first, &me, "pw-one");
    let held = Access::new(&settings(vec![
        password_entry(&first),
        password_entry(&second),
    ]))
    .unwrap();
    assert_eq!(held.sign_in(&me, "pw-two", &[]), SignIn::Refused);
    // Only the providers a policy names are asked, in the order it names them.
    assert_eq!(
        held.sign_in(&me, "pw-two", &["coqui_access_password".to_owned()]),
        SignIn::Refused
    );
    assert_eq!(
        held.sign_in(&me, "pw-one", &["nobody".to_owned()]),
        SignIn::Refused
    );
    // Listed after a provider that leaves the name alone, the same file is reached.
    let narrow = entry(
        SYSTEM,
        None,
        &serde_json::json!({"groups": ["canoa-no-such-group"]}),
    );
    let after = Access::new(&settings(vec![narrow, password_entry(&second)])).unwrap();
    assert!(matches!(
        after.sign_in(&me, "pw-two", &[]),
        SignIn::Admitted { .. }
    ));
    std::fs::remove_dir_all(dir).unwrap();
}

fn expired_certificate() -> (Vec<u8>, String) {
    let mut params = rcgen::CertificateParams::new(vec!["localhost".to_owned()]).unwrap();
    params.not_before = rcgen::date_time_ymd(2019, 1, 1);
    params.not_after = rcgen::date_time_ymd(2020, 1, 1);
    let key = rcgen::KeyPair::generate().unwrap();
    let der = params.self_signed(&key).unwrap().der().to_vec();
    let fingerprint = format!(
        "sha256:{}",
        data_encoding::HEXLOWER.encode(&Sha256::digest(&der))
    );
    (der, fingerprint)
}

#[test]
fn an_expired_certificate_signs_no_one_in_even_when_it_is_pinned_and_is_never_pinned() {
    let dir = scratch("expired");
    let me = my_name();
    let (der, fingerprint) = expired_certificate();
    let file = dir.join("peers.toml");
    seed(
        "coqui_access_mtls",
        "coqui_access_mtls",
        &file_source(&file),
        &me,
        &fingerprint,
    );
    let access = enrolling(&file);
    let h = header(&data_encoding::BASE64.encode(&der));
    assert_eq!(
        access.sign_in_certificate(&h, client(), &[]),
        SignIn::Expired
    );
    let (cert, _) = access
        .certificate_status(&me, mine(), &h)
        .unwrap()
        .presented
        .unwrap();
    assert!(cert.expired);
    assert_eq!(access.pin(&me, mine(), &h), Err(PinError::Expired));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_pin_made_for_another_user_id_is_not_the_accounts_and_can_be_pinned_over() {
    let dir = scratch("uid");
    let file = dir.join("peers.toml");
    let me = my_name();
    let (other_der, other_fp) = certificate();
    seed(
        "coqui_access_mtls",
        "coqui_access_mtls",
        &file_source(&file),
        "someone-else",
        &other_fp,
    );
    let access = enrolling(&file);
    let (der, _) = certificate();
    let h = header(&data_encoding::BASE64.encode(&der));
    // Pinned for the user id this name had before it was made again.
    access.pin(&me, mine() + 1, &h).unwrap();
    assert_eq!(
        access.sign_in_certificate(&h, client(), &[]),
        SignIn::Refused
    );
    let status = access.certificate_status(&me, mine(), &h).unwrap();
    assert!(!status.pinned);
    assert_eq!(status.presented.unwrap().1, PinState::Unpinned);
    // The person pins again, and the pin is theirs now.
    access.pin(&me, mine(), &h).unwrap();
    assert!(matches!(
        access.sign_in_certificate(&h, client(), &[]),
        SignIn::Admitted { .. }
    ));
    let _ = other_der;
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_pin_an_administrator_disabled_cannot_be_removed_or_replaced_from_here() {
    let dir = scratch("disabled");
    let file = dir.join("peers.toml");
    let me = my_name();
    let (_, fingerprint) = certificate();
    seed(
        "coqui_access_mtls",
        "coqui_access_mtls",
        &file_source(&file),
        &me,
        &fingerprint,
    );
    let mut registry = Registry::new("seed", "0");
    open_library(&mut registry, &plugin("coqui_access_mtls")).unwrap();
    let config = value_of(file_source(&file).as_object().unwrap()).unwrap();
    let accounts: Arc<dyn AccountKind> = Arc::new(
        registry
            .offer::<dyn AccountKind>("coqui_access_mtls")
            .unwrap()
            .unwrap()
            .instantiate(&config)
            .unwrap(),
    );
    accounts
        .put(AccountPut {
            name: me.clone(),
            disabled: Some(true),
            ..AccountPut::default()
        })
        .unwrap();
    let access = enrolling(&file);
    let (der, _) = certificate();
    let h = header(&data_encoding::BASE64.encode(&der));
    assert_eq!(access.pin(&me, mine(), &h), Err(PinError::Disabled));
    assert_eq!(access.unpin(&me), Err(PinError::Disabled));
    let status = access.certificate_status(&me, mine(), &h).unwrap();
    assert!(status.disabled && !status.pinned);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_library_is_loaded_only_from_files_root_owns_unless_the_setting_says_otherwise() {
    let dir = scratch("owner");
    let copy = dir.join("libcopy.so");
    std::fs::copy(plugin("coqui_access_mtls"), &copy).unwrap();
    let file = dir.join("peers.toml");
    let make = |any_owner: bool| {
        let mut settings = settings(vec![entry(
            "coqui_access_mtls",
            Some(copy.clone()),
            &file_source(&file),
        )]);
        settings.libraries_any_owner = any_owner;
        Access::new(&settings)
    };
    // Owned by whoever runs the test, which is not root: refused (a test run as root cannot show this).
    if mine() != 0 {
        let refused = make(false).unwrap_err();
        assert!(refused.contains("not owned by root"), "{refused}");
    }
    assert!(make(true).is_ok());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn source_files_a_group_or_others_can_read_are_found() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = scratch("loose");
    let (tight, loose) = (dir.join("tight.toml"), dir.join("loose.toml"));
    for (path, mode) in [(&tight, 0o600), (&loose, 0o644)] {
        std::fs::write(path, "").unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }
    let config = serde_json::json!({
        "source": {"file": {"path": tight}},
        "waiting": {"file": {"path": loose}},
    });
    assert_eq!(loose_sources(config.as_object().unwrap()), vec![loose]);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_list_the_portal_cannot_build_does_not_start() {
    for entries in [
        vec![entry("coqui_access_nothing", None, &serde_json::json!({}))],
        vec![entry(
            SYSTEM,
            Some(PathBuf::from("/x.so")),
            &serde_json::json!({}),
        )],
        vec![entry(SYSTEM, None, &serde_json::json!({"no_such_key": 1}))],
        vec![{
            let mut system = Entry::new(SYSTEM);
            system.enrol = true;
            system
        }],
    ] {
        assert!(Access::new(&settings(entries)).is_err());
    }
}

/// A directory of the test's own for the matcher to ask: `alice@example.org` is this process's account, `twin@example.org`
/// is two accounts, and the filter and id lookups find nobody. It counts the lookups it was asked.
struct Book {
    asked: std::sync::atomic::AtomicUsize,
}

impl Directory for Book {
    fn lookup(
        &self,
        lookup: Lookup,
    ) -> Result<Option<coqui_access::Person>, coqui_access::guatiao::library::ProviderError> {
        self.asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        match (lookup.by.as_str(), lookup.value.as_str()) {
            ("email", "alice@example.org") => Ok(Some(coqui_access::Person {
                id: mine().to_string(),
                name: my_name(),
                ..coqui_access::Person::default()
            })),
            ("email", "twin@example.org") => Err(coqui_access::error::ambiguous()),
            // A directory that finds an account by a name that holds an `@`, and one that means another user id.
            ("name", name) if name == format!("{}@evil", my_name()) => {
                Ok(Some(coqui_access::Person {
                    id: mine().to_string(),
                    name: my_name(),
                    ..coqui_access::Person::default()
                }))
            }
            ("email", "mismatch@example.org") => Ok(Some(coqui_access::Person {
                id: (mine() + 7).to_string(),
                name: my_name(),
                ..coqui_access::Person::default()
            })),
            ("name", name) if name == my_name() => Ok(Some(coqui_access::Person {
                id: mine().to_string(),
                name: my_name(),
                ..coqui_access::Person::default()
            })),
            _ => Ok(None),
        }
    }
}

fn claims(pairs: &[(&str, &str)]) -> Lookup {
    let mut values = std::collections::BTreeMap::<String, Vec<String>>::new();
    for (name, value) in pairs {
        values
            .entry((*name).to_owned())
            .or_default()
            .push((*value).to_owned());
    }
    Lookup {
        by: "claims".to_owned(),
        value: "a certificate".to_owned(),
        claims: Some(coqui_access::Claims {
            values,
            certificate: None,
        }),
    }
}

fn matching(change: impl FnOnce(&mut Settings)) -> (Access, Arc<Book>) {
    let dir = scratch("matcher");
    let file = dir.join("accounts.toml");
    seed_password(&file, &my_name(), "x");
    let mut own = settings(vec![password_entry(&file)]);
    change(&mut own);
    let mut access = Access::new(&own).unwrap();
    let book = Arc::new(Book {
        asked: std::sync::atomic::AtomicUsize::new(0),
    });
    access.matcher_directory = Some(book.clone());
    (access, book)
}

#[test]
fn a_upn_claim_that_is_an_account_name_signs_that_account_in() {
    let (access, _) = matching(|_| {});
    let me = my_name();
    admits(
        access.match_claims("cert", &claims(&[("san.upn", &me)])),
        &me,
        "cert",
    );
}

#[test]
fn an_email_claim_finds_the_account_the_directory_gives_it() {
    let (access, book) = matching(|_| {});
    let me = my_name();
    admits(
        access.match_claims("cert", &claims(&[("san.email", "alice@example.org")])),
        &me,
        "cert",
    );
    assert!(book.asked.load(std::sync::atomic::Ordering::SeqCst) > 0);
}

#[test]
fn an_ambiguous_claim_and_one_nobody_has_are_refused() {
    let (access, _) = matching(|_| {});
    for lookup in [
        claims(&[("san.email", "twin@example.org")]),
        claims(&[("san.email", "nobody@example.org")]),
        claims(&[("subject.cn", &my_name())]),
        claims(&[]),
    ] {
        assert_eq!(access.match_claims("cert", &lookup), SignIn::Refused);
    }
}

#[test]
fn a_rule_may_strip_the_realm_only_when_it_names_it() {
    let rule = |domain: Option<&str>| {
        vec![crate::portal::MatcherRule::new(
            "name",
            "{san.upn|local}",
            domain,
        )]
    };
    let dir = scratch("realm");
    let file = dir.join("accounts.toml");
    seed_password(&file, &my_name(), "x");
    let mut bare = settings(vec![password_entry(&file)]);
    bare.matcher = Some(rule(None));
    assert!(
        Access::new(&bare).is_err(),
        "`local` without a realm does not start"
    );
    let (access, _) = matching(|own| own.matcher = Some(rule(Some("corp.example"))));
    let me = my_name();
    admits(
        access.match_claims(
            "cert",
            &claims(&[("san.upn", &format!("{me}@corp.example"))]),
        ),
        &me,
        "cert",
    );
    // The same name in another realm is not this account.
    assert_eq!(
        access.match_claims(
            "cert",
            &claims(&[("san.upn", &format!("{me}@evil.example"))])
        ),
        SignIn::Refused
    );
}

fn totp_entry(file: &Path) -> Entry {
    let mut config = file_source(file);
    config["skew"] = 10.into();
    entry(
        "coqui_access_totp",
        Some(plugin("coqui_access_totp")),
        &config,
    )
}

#[test]
fn a_code_store_needs_a_source_file_and_only_one_provider_keeps_codes() {
    let dir = scratch("totp-source");
    let file = dir.join("accounts.toml");
    seed_password(&file, &my_name(), "x");
    let memory = entry(
        "coqui_access_totp",
        Some(plugin("coqui_access_totp")),
        &serde_json::json!({}),
    );
    assert!(Access::new(&settings(vec![password_entry(&file), memory])).is_err());
    let twice = settings(vec![
        password_entry(&file),
        totp_entry(&dir.join("a.toml")),
        totp_entry(&dir.join("b.toml")),
    ]);
    assert!(Access::new(&twice).is_err());
}

#[test]
fn an_administrators_listing_names_who_has_a_code_and_never_shows_a_secret() {
    let dir = scratch("totp-list");
    let file = dir.join("accounts.toml");
    let me = my_name();
    seed_password(&file, &me, "x");
    let access = Access::new(&settings(vec![
        password_entry(&file),
        totp_entry(&dir.join("totp.toml")),
    ]))
    .unwrap();
    assert!(access.methods().contains(&Method::Totp));
    assert_eq!(access.admitted(&me), Ok((me.clone(), mine())));
    assert!(access.admitted("nobody-here").is_err());
    assert!(access.admitted("../x").is_err());
    assert_eq!(access.list_factors(), Ok(Vec::new()));
    let begun = access.begin_factor(&me, mine(), false).unwrap();
    let waiting = access.list_factors().unwrap();
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].user.as_deref(), Some(me.as_str()));
    assert!((waiting[0].waiting, waiting[0].enrolled) == (true, false));
    let shown = format!("{waiting:?}{begun:?}");
    assert!(!shown.contains("otpauth") && !shown.contains("secret="));
    assert_eq!(
        access.factor_standing(&me, mine()),
        FactorStanding::NotEnrolled
    );
    // A setup that was never confirmed is no code: a reset takes the setup away and marks nothing.
    assert_eq!(access.reset_factor(&me, mine()), Ok(false));
    assert_eq!(access.list_factors(), Ok(Vec::new()));
    let begun = access.begin_factor(&me, mine(), false).unwrap();
    access
        .confirm_factor(&me, mine(), &totp_in(&begun.uri))
        .unwrap();
    assert_eq!(access.reset_factor(&me, mine()), Ok(true));
    assert_eq!(
        access.reset_factor(&me, mine()),
        Ok(false),
        "a code is reset once"
    );
    // Reset leaves the account marked for an administrator, with nothing else of it.
    let after = access.list_factors().unwrap();
    assert_eq!(
        (after.len(), after[0].reset, after[0].enrolled),
        (1, true, false)
    );
    // A code is checked for the account named, whatever the portal's other methods say.
    assert_eq!(
        access.check_code(&me, mine(), Method::Totp, "000000", &[]),
        CodeCheck::Refused
    );
    assert_eq!(
        access.check_code(&me, mine(), Method::Password, "000000", &[]),
        CodeCheck::Refused
    );
    assert_eq!(
        access.check_code(&me, mine(), Method::Totp, "000000", &["another".to_owned()]),
        CodeCheck::Refused,
        "a provider the policy did not name is not asked"
    );
}

#[test]
fn a_value_with_two_ats_is_refused_before_the_realm_is_stripped() {
    let (access, _) = matching(|own| {
        own.matcher = Some(vec![crate::portal::MatcherRule::new(
            "name",
            "{san.upn|local}",
            Some("corp.example"),
        )]);
    });
    let me = my_name();
    // One `@` is the realm and strips to the account; two leave a name the directory above finds as the same account.
    admits(
        access.match_claims(
            "cert",
            &claims(&[("san.upn", &format!("{me}@corp.example"))]),
        ),
        &me,
        "cert",
    );
    for value in [
        format!("{me}@evil@corp.example"),
        format!("@{me}@corp.example"),
    ] {
        assert_eq!(
            access.match_claims("cert", &claims(&[("san.upn", &value)])),
            SignIn::Refused,
            "{value}"
        );
    }
    assert_eq!(
        access.match_claims(
            "cert",
            &claims(&[("san.email", &format!("{me}@evil@corp.example"))])
        ),
        SignIn::Refused
    );
}

#[test]
fn a_directory_that_means_another_user_id_than_the_system_does_is_not_the_account() {
    let (access, _) = matching(|_| {});
    assert_eq!(
        access.match_claims("cert", &claims(&[("san.email", "mismatch@example.org")])),
        SignIn::Refused
    );
}

#[test]
fn the_proxy_provider_is_refused_and_so_is_a_listing_that_names_it() {
    let dir = scratch("proxy");
    let file = dir.join("accounts.toml");
    seed_password(&file, &my_name(), "x");
    let mut proxy = Entry::new("coqui_access_proxy");
    proxy.library = Some(PathBuf::from("/does/not/exist.so"));
    let error = Access::new(&settings(vec![password_entry(&file), proxy])).unwrap_err();
    assert!(error.contains("not used by the portal"), "{error}");
}

#[test]
fn code_files_made_by_another_account_are_refused_before_anything_is_changed() {
    let dir = scratch("owned");
    let file = dir.join("accounts.toml");
    seed_password(&file, &my_name(), "x");
    let ours = Access::new(&settings(vec![
        password_entry(&file),
        totp_entry(&dir.join("totp.toml")),
    ]))
    .unwrap();
    assert_eq!(ours.check_code_files_owned(), Ok(()));
    // `/tmp` belongs to root: files made there as this account would be root's to remove, and root's as the other way round.
    let name = format!("canoa-not-made-{}.toml", std::process::id());
    let rooted = Access::new(&settings(vec![
        password_entry(&file),
        totp_entry(&PathBuf::from("/tmp").join(&name)),
    ]))
    .unwrap();
    let error = rooted.check_code_files_owned().unwrap_err();
    assert!(error.contains("belongs to user id 0"), "{error}");
    assert!(!PathBuf::from("/tmp").join(&name).exists());
    // In a key the provider is given for a second file, it is the same.
    let mut config = file_source(&dir.join("t.toml"));
    config["pending"] = file_source(&PathBuf::from("/tmp").join(&name))["source"].clone();
    let other = Access::new(&settings(vec![
        password_entry(&file),
        entry(
            "coqui_access_totp",
            Some(plugin("coqui_access_totp")),
            &config,
        ),
    ]))
    .unwrap();
    let error = other.check_code_files_owned().unwrap_err();
    assert!(error.contains("belongs to user id 0"), "{error}");
    assert!(!PathBuf::from("/tmp").join(&name).exists());
}

fn access_with_codes(name: &str) -> (Access, PathBuf, String) {
    let dir = scratch(name);
    let file = dir.join("accounts.toml");
    let me = my_name();
    seed_password(&file, &me, "x");
    let totp = dir.join("totp.toml");
    let access = Access::new(&settings(vec![password_entry(&file), totp_entry(&totp)])).unwrap();
    (access, totp, me)
}

#[test]
fn a_code_table_that_cannot_be_read_is_unknown_and_never_a_code_that_is_not_there() {
    let (access, table, me) = access_with_codes("unreadable");
    let begun = access.begin_factor(&me, mine(), false).unwrap();
    access
        .confirm_factor(&me, mine(), &totp_in(&begun.uri))
        .unwrap();
    let (standing, made) = access.factor_record(&me, mine());
    assert_eq!(standing, FactorStanding::Enrolled);
    assert!(made.is_some());
    let kept = std::fs::read(&table).unwrap();
    std::fs::write(&table, b"[[accounts\nnot a table").unwrap();
    assert_eq!(
        access.factor_record(&me, mine()),
        (FactorStanding::Unknown, None)
    );
    assert_eq!(access.factor_standing(&me, mine()), FactorStanding::Unknown);
    std::fs::write(&table, kept).unwrap();
    assert_eq!(
        access.factor_record(&me, mine()).0,
        FactorStanding::Enrolled
    );
}

/// The code an app shows now for the secret in a setup link.
fn totp_in(link: &str) -> String {
    use hmac::{Hmac, KeyInit, Mac as _};
    let secret = link
        .split(['?', '&'])
        .find_map(|part| part.strip_prefix("secret="))
        .unwrap();
    let key = data_encoding::BASE32_NOPAD
        .decode(secret.as_bytes())
        .unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut mac = <Hmac<sha1::Sha1> as KeyInit>::new_from_slice(&key).unwrap();
    mac.update(&(now / 30).to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let offset = usize::from(digest[19] & 0x0f);
    let number = u32::from_be_bytes([
        digest[offset] & 0x7f,
        digest[offset + 1],
        digest[offset + 2],
        digest[offset + 3],
    ]);
    format!("{:06}", number % 1_000_000)
}

#[test]
fn a_reset_that_cannot_be_written_leaves_the_code_and_many_at_once_each_stand() {
    let (access, table, me) = access_with_codes("marks");
    let enrol = |user: &str, uid: u32| {
        let begun = access.begin_factor(user, uid, false).unwrap();
        access
            .confirm_factor(user, uid, &totp_in(&begun.uri))
            .unwrap();
    };
    enrol(&me, mine());
    // A table that cannot be written: the reset fails, and the account keeps its code and is not marked.
    let key = format!("{}:{me}", mine());
    let mut lock = table.clone().into_os_string();
    lock.push(".lock");
    let lock = PathBuf::from(lock);
    let mut permissions = std::fs::metadata(&lock).unwrap().permissions();
    let writable = permissions.clone();
    std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o400);
    std::fs::set_permissions(&lock, permissions).unwrap();
    let outcome = access.reset_by_id(&key);
    std::fs::set_permissions(&lock, writable).unwrap();
    assert!(outcome.is_err(), "the reset could not be made");
    let listed = access.list_factors().unwrap();
    assert!(
        listed
            .iter()
            .any(|entry| entry.id == key && entry.enrolled && !entry.reset),
        "{listed:?}"
    );
    for index in 0..16_u32 {
        enrol(&format!("person{index}"), 5000 + index);
    }
    // Many commands at once each leave their mark.
    std::thread::scope(|scope| {
        for (id, _) in access
            .list_factors()
            .unwrap()
            .into_iter()
            .map(|entry| (entry.id, ()))
        {
            let access = &access;
            scope.spawn(move || {
                assert_eq!(access.reset_by_id(&id), Ok(true));
            });
        }
    });
    let marked = access
        .list_factors()
        .unwrap()
        .into_iter()
        .filter(|entry| entry.reset && !entry.enrolled)
        .count();
    assert_eq!(marked, 17, "a mark was lost to another command's write");
}

#[test]
fn an_account_renamed_or_made_again_under_the_same_user_id_is_reset_and_not_open_to_a_first_code() {
    let (access, _, me) = access_with_codes("renamed");
    let begun = access.begin_factor(&me, mine(), false).unwrap();
    access
        .confirm_factor(&me, mine(), &totp_in(&begun.uri))
        .unwrap();
    // The same user id under another name finds no code, and is not an account with none to set up.
    assert_eq!(
        access.factor_standing("renamed-account", mine()),
        FactorStanding::Reset
    );
    // A reset recorded for the old name carries to the new one, and an administrator's setup lifts both.
    access.reset_factor(&me, mine()).unwrap();
    assert_eq!(
        access.factor_standing("renamed-account", mine()),
        FactorStanding::Reset
    );
    access.clear_reset(mine()).unwrap();
    assert_eq!(
        access.factor_standing("renamed-account", mine()),
        FactorStanding::NotEnrolled
    );
    assert_eq!(
        access.factor_standing("elsewhere", mine() + 1),
        FactorStanding::NotEnrolled,
        "another user id is not this one"
    );
}

#[test]
fn an_unlock_is_a_time_the_portal_reads_and_an_account_with_no_code_has_none() {
    let (access, _, me) = access_with_codes("unlock");
    // Nothing is held for an account with no code: nothing to clear, and no time to read.
    access.unlock(&me, mine()).unwrap();
    assert_eq!(access.unlock_stamp(&me, mine()), None);
    let begun = access.begin_factor(&me, mine(), false).unwrap();
    access
        .confirm_factor(&me, mine(), &totp_in(&begun.uri))
        .unwrap();
    assert_eq!(access.unlock_stamp(&me, mine()), None);
    access.unlock(&me, mine()).unwrap();
    let first = access.unlock_stamp(&me, mine()).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    access.unlock(&me, mine()).unwrap();
    assert!(access.unlock_stamp(&me, mine()).unwrap() > first);
    assert_eq!(access.unlock_stamp("someone-else", mine()), None);
}

#[test]
fn an_account_with_a_code_is_given_a_new_one_only_when_its_current_code_was_proved() {
    let (access, _, me) = access_with_codes("rotate-proof");
    let begun = access.begin_factor(&me, mine(), false).unwrap();
    access
        .confirm_factor(&me, mine(), &totp_in(&begun.uri))
        .unwrap();
    let refused = access.begin_factor(&me, mine(), false);
    let Err(FactorError::Failed(why)) = refused else {
        panic!("a rotation with no proof was not refused: {refused:?}");
    };
    assert!(!why.is_empty() && !why.contains("secret="), "{why}");
    assert!(
        access
            .list_factors()
            .unwrap()
            .iter()
            .all(|each| !each.waiting)
    );
    let fresh = access.begin_factor(&me, mine(), true).unwrap();
    assert_ne!(fresh.uri, begun.uri);
    assert!(
        access
            .list_factors()
            .unwrap()
            .iter()
            .any(|each| each.waiting)
    );
}

#[test]
fn an_unlock_leaves_a_code_an_administrator_disabled_disabled() {
    let (access, _, me) = access_with_codes("unlock-disabled");
    let begun = access.begin_factor(&me, mine(), false).unwrap();
    access
        .confirm_factor(&me, mine(), &totp_in(&begun.uri))
        .unwrap();
    let store = access.codes.as_ref().unwrap();
    store
        .accounts
        .put(AccountPut {
            name: format!("{}:{me}", mine()),
            disabled: Some(true),
            ..AccountPut::default()
        })
        .unwrap();
    assert_eq!(
        access.factor_standing(&me, mine()),
        FactorStanding::Disabled
    );
    access.unlock(&me, mine()).unwrap();
    assert_eq!(
        access.factor_standing(&me, mine()),
        FactorStanding::Disabled,
        "an unlock is not an enable"
    );
    // And an enabled one stays enabled.
    store
        .accounts
        .put(AccountPut {
            name: format!("{}:{me}", mine()),
            disabled: Some(false),
            ..AccountPut::default()
        })
        .unwrap();
    access.unlock(&me, mine()).unwrap();
    assert_eq!(
        access.factor_standing(&me, mine()),
        FactorStanding::Enrolled
    );
}

#[test]
fn the_owner_check_looks_at_the_code_providers_files_and_no_others() {
    let dir = scratch("owner-scope");
    // A password table under a folder root owns, and a code table under ours: nothing to refuse.
    let name = format!("canoa-owner-scope-{}.toml", std::process::id());
    let password = entry(
        "coqui_access_password",
        Some(plugin("coqui_access_password")),
        &file_source(&PathBuf::from("/tmp").join(&name)),
    );
    let ours = Access::new(&settings(vec![
        password.clone(),
        totp_entry(&dir.join("totp.toml")),
    ]))
    .unwrap();
    assert_eq!(ours.check_code_files_owned(), Ok(()));
    // The code table under root's folder is the one that counts.
    let codes = Access::new(&settings(vec![
        password,
        totp_entry(&PathBuf::from("/tmp").join(&name)),
    ]))
    .unwrap();
    let error = codes.check_code_files_owned().unwrap_err();
    assert!(error.contains("belongs to user id 0"), "{error}");
}

fn passkey_entry(file: &Path) -> Entry {
    let mut config = file_source(file);
    config["rp_id"] = super::super::device::RP_ID.into();
    config["origins"] = serde_json::json!([super::super::device::ORIGIN]);
    entry(
        "coqui_access_passkey",
        Some(plugin("coqui_access_passkey")),
        &config,
    )
}

/// An access with the passkey provider keeping its tables under `dir`, and a password provider so the portal has a first way in.
fn passkey_access(dir: &Path) -> Access {
    let file = dir.join("accounts.toml");
    seed_password(&file, &my_name(), "x");
    Access::new(&settings(vec![
        password_entry(&file),
        passkey_entry(&dir.join("passkeys.toml")),
    ]))
    .unwrap()
}

/// Adds a passkey to the account the way the Passkeys page does: begin, the device answers, confirm.
fn add_passkey(
    access: &Access,
    device: &mut super::super::device::Device,
    user: &str,
    uid: u32,
    label: &str,
) -> String {
    let options = access.begin_passkey(user, uid).unwrap();
    let answer = device
        .register(super::super::device::ORIGIN, &options)
        .unwrap();
    access.confirm_passkey(user, uid, &answer, label).unwrap()
}

/// A sign-in by passkey the way the login page does it: a challenge (for `account`, or for any passkey), the device's answer,
/// and the check.
fn passkey_in(
    access: &Access,
    device: &mut super::super::device::Device,
    account: Option<(&str, u32)>,
) -> PasskeyAnswer {
    let made = access.passkey_challenge(account, client(), &[]).unwrap();
    let answer = device
        .authenticate(super::super::device::ORIGIN, &made.options)
        .unwrap();
    access.sign_in_passkey(&answer, &made.challenge, client(), &[])
}

#[test]
fn a_passkey_registered_for_an_account_signs_it_in_with_no_name_typed_and_with_one() {
    let dir = scratch("passkey-in");
    let access = passkey_access(&dir);
    let me = my_name();
    assert!(access.methods().contains(&Method::Passkey));
    let mut device = super::super::device::Device::new();
    let id = add_passkey(&access, &mut device, &me, mine(), "laptop");
    assert!(!id.is_empty());
    let listed = access.passkeys(&me, mine()).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, id);
    assert_eq!(listed[0].label.as_deref(), Some("laptop"));
    assert_eq!(listed[0].used_unix, None);
    for account in [None, Some((me.as_str(), mine()))] {
        let signed = passkey_in(&access, &mut device, account);
        assert!(
            matches!(&signed.result, SignIn::Admitted { user, uid, .. } if *user == me && *uid == mine()),
            "{signed:?}"
        );
        assert_eq!(signed.credential.as_deref(), Some(id.as_str()));
    }
    assert!(access.passkeys(&me, mine()).unwrap()[0].used_unix.is_some());
}

#[test]
fn a_passkey_answer_is_taken_once_and_one_for_another_challenge_is_refused() {
    let dir = scratch("passkey-once");
    let access = passkey_access(&dir);
    let me = my_name();
    let mut device = super::super::device::Device::new();
    add_passkey(&access, &mut device, &me, mine(), "laptop");
    let made = access.passkey_challenge(None, client(), &[]).unwrap();
    let answer = device
        .authenticate(super::super::device::ORIGIN, &made.options)
        .unwrap();
    let other = access.passkey_challenge(None, client(), &[]).unwrap();
    // The answer was signed over the first challenge: the second one's key does not take it.
    assert_eq!(
        access
            .sign_in_passkey(&answer, &other.challenge, client(), &[])
            .result,
        SignIn::Refused
    );
    assert!(matches!(
        access
            .sign_in_passkey(&answer, &made.challenge, client(), &[])
            .result,
        SignIn::Admitted { .. }
    ));
    assert_eq!(
        access
            .sign_in_passkey(&answer, &made.challenge, client(), &[])
            .result,
        SignIn::Refused,
        "the same answer, again"
    );
}

#[test]
fn a_passkey_of_an_account_the_system_does_not_admit_signs_no_one_in() {
    let dir = scratch("passkey-groups");
    let access = passkey_access(&dir);
    let me = my_name();
    let mut device = super::super::device::Device::new();
    add_passkey(&access, &mut device, &me, mine(), "laptop");
    // The same files, read by a portal that admits only a group the account is not in.
    let mut narrow = settings(vec![passkey_entry(&dir.join("passkeys.toml"))]);
    narrow.groups = vec!["no-such-group-here".to_owned()];
    let narrow = Access::new(&narrow).unwrap();
    assert_eq!(
        passkey_in(&narrow, &mut device, None).result,
        SignIn::Refused
    );
}

#[test]
fn a_passkey_kept_for_another_user_id_under_the_same_name_is_nobodys_and_is_replaced() {
    let dir = scratch("passkey-uid");
    let access = passkey_access(&dir);
    let me = my_name();
    let mut device = super::super::device::Device::new();
    // Added when the account had another user id.
    let old = add_passkey(&access, &mut device, &me, mine() + 1, "before");
    assert!(access.passkeys(&me, mine()).unwrap().is_empty());
    assert_eq!(access.passkeys(&me, mine() + 1).unwrap().len(), 1);
    assert_eq!(
        access.passkey_challenge(Some((&me, mine())), client(), &[]),
        Err(PasskeyError::NoPasskey)
    );
    // Asked for with no name, the answer is the provider's valid one and the mark says whose it is: not this user id's.
    assert_eq!(
        passkey_in(&access, &mut device, None).result,
        SignIn::Refused
    );
    // The account adds one now: what the name held before goes first, so the old one signs no one in afterwards either.
    let mut fresh = super::super::device::Device::new();
    let new = add_passkey(&access, &mut fresh, &me, mine(), "now");
    let held = access.passkeys(&me, mine()).unwrap();
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].id, new);
    assert!(
        access
            .passkeys(&me, mine() + 1)
            .unwrap()
            .iter()
            .all(|passkey| passkey.id != old)
    );
    assert!(matches!(
        passkey_in(&access, &mut fresh, None).result,
        SignIn::Admitted { .. }
    ));
}

#[test]
fn a_passkey_store_needs_a_source_file_and_only_one_provider_keeps_passkeys() {
    let dir = scratch("passkey-source");
    let file = dir.join("accounts.toml");
    seed_password(&file, &my_name(), "x");
    let mut memory = passkey_entry(&dir.join("a.toml"));
    memory.config.remove("source");
    let error = Access::new(&settings(vec![password_entry(&file), memory])).unwrap_err();
    assert!(
        error.contains("coqui_access_passkey") && error.contains("source"),
        "{error}"
    );
    let twice = settings(vec![
        password_entry(&file),
        passkey_entry(&dir.join("a.toml")),
        passkey_entry(&dir.join("b.toml")),
    ]);
    assert!(Access::new(&twice).is_err());
    // Without a passkey provider, the portal keeps none.
    let none = Access::new(&settings(vec![password_entry(&file)])).unwrap();
    assert!(!none.methods().contains(&Method::Passkey));
    assert_eq!(
        none.passkey_challenge(None, client(), &[]),
        Err(PasskeyError::Unsupported)
    );
}

#[test]
fn a_policy_that_names_other_providers_asks_no_passkey_provider() {
    let dir = scratch("passkey-providers");
    let access = passkey_access(&dir);
    let me = my_name();
    let mut device = super::super::device::Device::new();
    add_passkey(&access, &mut device, &me, mine(), "laptop");
    let other = vec!["somebody_else".to_owned()];
    assert_eq!(
        access.passkey_challenge(None, client(), &other),
        Err(PasskeyError::Unsupported)
    );
    let made = access.passkey_challenge(None, client(), &[]).unwrap();
    let answer = device
        .authenticate(super::super::device::ORIGIN, &made.options)
        .unwrap();
    assert_eq!(
        access
            .sign_in_passkey(&answer, &made.challenge, client(), &other)
            .result,
        SignIn::Refused
    );
}

#[test]
fn a_removed_passkey_signs_no_one_in_and_the_listing_never_shows_a_key() {
    let dir = scratch("passkey-remove");
    let access = passkey_access(&dir);
    let me = my_name();
    let mut device = super::super::device::Device::new();
    let id = add_passkey(&access, &mut device, &me, mine(), "laptop");
    let listing = access.list_passkeys().unwrap();
    assert_eq!(listing.len(), 1);
    assert_eq!(access.remove_passkey(&me, mine(), "nope"), Ok(false));
    access.label_passkey(&me, mine(), &id, "desk").unwrap();
    assert_eq!(
        access.passkeys(&me, mine()).unwrap()[0].label.as_deref(),
        Some("desk")
    );
    assert_eq!(access.remove_passkey(&me, mine(), &id), Ok(true));
    assert_eq!(access.remove_passkey(&me, mine(), &id), Ok(false));
    assert!(access.passkeys(&me, mine()).unwrap().is_empty());
    // The device still holds the credential, and the provider holds no account for it.
    assert_eq!(device.store.len(), 1);
    let made = access.passkey_challenge(None, client(), &[]).unwrap();
    let answer = device
        .authenticate(super::super::device::ORIGIN, &made.options)
        .unwrap();
    assert_eq!(
        access
            .sign_in_passkey(&answer, &made.challenge, client(), &[])
            .result,
        SignIn::Refused
    );
    // An administrator's removal by name takes what is left, and counts it.
    add_passkey(&access, &mut device, &me, mine(), "again");
    assert_eq!(access.remove_passkeys_of(&me, None), Ok(1));
    assert_eq!(access.remove_passkeys_of(&me, None), Ok(0));
}

/// `der` as a PEM block, its lines 64 columns wide, each ending in `eol`.
fn pem(der: &[u8], eol: &str) -> String {
    let body = data_encoding::BASE64.encode(der);
    let mut out = format!("-----BEGIN CERTIFICATE-----{eol}");
    for line in body.as_bytes().chunks(64) {
        out.push_str(core::str::from_utf8(line).unwrap());
        out.push_str(eol);
    }
    out.push_str("-----END CERTIFICATE-----");
    out.push_str(eol);
    out
}

/// What nginx's `$ssl_client_escaped_cert` makes of `text`: every byte but a letter or digit as `%XX`.
fn escaped(text: &str) -> String {
    text.bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() {
                char::from(byte).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

fn chain_of(value: &str) -> Option<Vec<Vec<u8>>> {
    forwarded_chain(&header(value), "x-client-cert")
}

#[test]
fn every_form_a_proxy_forwards_a_certificate_in_reads_as_the_same_chain() {
    let (leaf, _) = certificate();
    let (second, _) = certificate();
    let one = vec![leaf.clone()];
    let two = vec![leaf.clone(), second.clone()];
    let base64 = |ders: &[&[u8]]| {
        ders.iter()
            .map(|der| data_encoding::BASE64.encode(der))
            .collect::<Vec<_>>()
            .join(",")
    };
    assert_eq!(chain_of(&base64(&[&leaf])), Some(one.clone()));
    assert_eq!(chain_of(&base64(&[&leaf, &second])), Some(two.clone()));
    // HAProxy and nginx's `$ssl_client_cert` and `$ssl_client_escaped_cert`, and plain PEM.
    assert_eq!(chain_of(&pem(&leaf, "\n")), Some(one.clone()));
    assert_eq!(chain_of(&pem(&leaf, "\r\n")), Some(one.clone()));
    assert_eq!(chain_of(&pem(&leaf, "\n\t")), Some(one.clone()));
    assert_eq!(chain_of(&pem(&leaf, " ")), Some(one.clone()));
    assert_eq!(chain_of(&escaped(&pem(&leaf, "\n"))), Some(one));
    let both = format!("{}{}", pem(&leaf, "\n"), pem(&second, "\n"));
    assert_eq!(chain_of(&both), Some(two.clone()));
    assert_eq!(chain_of(&escaped(&both)), Some(two.clone()));
    assert_eq!(
        chain_of(&format!(
            "{}\n\t{}",
            pem(&leaf, "\n\t"),
            pem(&second, "\n\t")
        )),
        Some(two)
    );
}

#[test]
fn a_forwarded_certificate_that_does_not_decode_is_refused_not_a_crash() {
    let (leaf, _) = certificate();
    let good = pem(&leaf, "\n");
    // A block with no end, text around a block, a body that is not base64, another kind of block, and no block's body at all.
    for bad in [
        good.replace("-----END CERTIFICATE-----", ""),
        format!("junk\n{good}"),
        format!("{good}junk"),
        good.replacen("MI", "M!", 1),
        good.replace("CERTIFICATE", "PRIVATE KEY"),
        "-----BEGIN CERTIFICATE----------END CERTIFICATE-----".to_owned(),
        "-----BEGIN CERTIFICATE-----".to_owned(),
    ] {
        assert_eq!(chain_of(&bad), None, "{bad}");
        assert_eq!(chain_of(&escaped(&bad)), None, "{bad}");
    }
    // A percent sign that is no escape, one cut short, and escapes that are not UTF-8.
    for bad in [
        "%zz",
        "%4",
        "%",
        "%FF%FE",
        "-----BEGIN%20CERTIFICATE-----%4",
    ] {
        assert_eq!(chain_of(bad), None, "{bad}");
    }
    assert_eq!(chain_of(""), None);
}

#[test]
fn a_forwarded_header_is_measured_as_sent_and_holds_eight_certificates_at_most() {
    let (leaf, _) = certificate();
    let block = pem(&leaf, "\n");
    let eight = block.repeat(8);
    assert_eq!(chain_of(&eight).map(|chain| chain.len()), Some(8));
    assert_eq!(chain_of(&block.repeat(9)), None);
    assert_eq!(chain_of(&escaped(&block.repeat(9))), None);
    // Over 16 KiB as sent, though little of it is certificate.
    let padded = format!("{block}{}", " ".repeat(super::certs::MAX_HEADER));
    assert_eq!(chain_of(&padded), None);
    let at_limit = format!(
        "{block}{}",
        " ".repeat(super::certs::MAX_HEADER - block.len())
    );
    assert_eq!(at_limit.len(), super::certs::MAX_HEADER);
    assert_eq!(chain_of(&at_limit).map(|chain| chain.len()), Some(1));
}

#[test]
fn a_certificate_nginx_forwards_as_escaped_pem_signs_its_account_in() {
    let dir = scratch("pinned-nginx");
    let (access, me, der) = pinned(&dir.join("peers.toml"), &[]);
    for value in [
        escaped(&pem(&der, "\n")),
        pem(&der, "\n\t"),
        pem(&der, "\n"),
    ] {
        admits(
            access.sign_in_certificate(&header(&value), client(), &[]),
            &me,
            "coqui_access_mtls",
        );
        assert!(access.carries_certificate(&header(&value)));
    }
    let (other, _) = certificate();
    assert_eq!(
        access.sign_in_certificate(&header(&escaped(&pem(&other, "\n"))), client(), &[]),
        SignIn::Unpinned
    );
    assert_eq!(
        access.sign_in_certificate(&header("%zz"), client(), &[]),
        SignIn::Refused
    );
    std::fs::remove_dir_all(dir).unwrap();
}
