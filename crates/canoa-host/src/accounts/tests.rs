//! Tests of the host's accounts: the stores, the newer-wins rule, the lease and the endpoint check.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::path::PathBuf;
use std::time::SystemTime;

use coqui_acp_api::{AccountKindInfo, AccountLoss, AccountRenewal, Credentials};

use super::store::{FileStore, KeyringStore, SecretStore};
use super::*;
use crate::config::AccountConfig;

/// A directory of its own that goes with the test.
struct Dir(PathBuf);

impl Dir {
    fn new() -> Self {
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos());
        let dir =
            std::env::temp_dir().join(format!("canoa-accounts-{}-{stamp}", std::process::id()));
        fs::create_dir_all(&dir).expect("a directory");
        Self(dir)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// More than any one entry of the operating system's store holds: a Codex `auth.json` is several kilobytes.
fn large() -> Vec<u8> {
    (0..9_000_u32)
        .map(|n| b'a' + u8::try_from(n % 26).expect("a letter"))
        .collect()
}

#[test]
fn the_file_store_round_trips_a_large_record_and_holds_no_plaintext() {
    let dir = Dir::new();
    let store = FileStore::new(&dir.0);
    assert!(store.get("one").expect("reads").is_none());
    let bytes = large();
    store.put("one", &bytes).expect("writes");
    assert_eq!(store.get("one").expect("reads"), Some(bytes.clone()));
    for entry in fs::read_dir(&dir.0).expect("lists") {
        let path = entry.expect("an entry").path();
        let text = fs::read(&path).expect("reads");
        assert!(
            !text.windows(40).any(|window| window == &bytes[..40]),
            "{} holds the record in the clear",
            path.display()
        );
    }
    store.put("one", b"shorter").expect("replaces");
    assert_eq!(store.get("one").expect("reads"), Some(b"shorter".to_vec()));
    store.remove("one").expect("removes");
    assert!(store.get("one").expect("reads").is_none());
    store.remove("one").expect("a second removal is no error");
}

#[test]
fn a_sealed_file_does_not_open_under_another_account_or_another_key() {
    let dir = Dir::new();
    let store = FileStore::new(&dir.0);
    store.put("one", b"secret").expect("writes");
    let sealed = fs::read_dir(&dir.0)
        .expect("lists")
        .map(|entry| entry.expect("an entry").path())
        .find(|path| path.extension().is_some_and(|ext| ext == "sealed"))
        .expect("the sealed file");
    // The same file under another id is another account's: its id is part of what is sealed.
    let copy = FileStore::new(&dir.0).path_of("two");
    fs::copy(&sealed, &copy).expect("copies");
    assert!(store.get("two").is_err(), "opened under another id");
    // Another key does not open it either.
    let other = Dir::new();
    fs::copy(&sealed, FileStore::new(&other.0).path_of("one")).expect("copies");
    assert!(FileStore::new(&other.0).get("one").is_err());
}

#[cfg(unix)]
#[test]
fn the_file_store_keeps_its_key_and_files_to_the_owner() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = Dir::new();
    let store = FileStore::new(&dir.0);
    store.put("one", b"secret").expect("writes");
    for entry in fs::read_dir(&dir.0).expect("lists") {
        let path = entry.expect("an entry").path();
        let mode = fs::metadata(&path).expect("metadata").permissions().mode();
        assert_eq!(mode & 0o077, 0, "{} is readable by others", path.display());
    }
}

/// The operating system's store, under a service of this test's own that is emptied afterwards. It needs a credential
/// store, which a headless Linux has none of: there it is skipped, and the file store is the one tested.
#[test]
fn the_operating_systems_store_round_trips_a_record_over_the_blob_limit() {
    let service = format!("dev.boriken.canoa.test.{}.{}", std::process::id(), line!());
    let store = KeyringStore::new(service);
    let bytes = large();
    match store.put("account-one", &bytes) {
        Ok(()) => {}
        Err(error) if store_unavailable(&error.to_string()) => {
            eprintln!("no credential store here, skipped: {error}");
            return;
        }
        Err(error) => panic!("the store refused a record: {error}"),
    }
    let read = store.get("account-one");
    // Replacing leaves one generation, and removing leaves none.
    let replaced = store.put("account-one", &bytes[..5_000]);
    let after = store.get("account-one");
    store.remove("account-one").expect("removes");
    let gone = store.get("account-one");
    assert_eq!(read.expect("reads"), Some(bytes.clone()));
    replaced.expect("replaces");
    assert_eq!(after.expect("reads"), Some(bytes[..5_000].to_vec()));
    assert!(gone.expect("reads").is_none());
}

fn accounts(dir: &Dir) -> Arc<Accounts> {
    Accounts::with_store(
        dir.0.join("accounts"),
        Box::new(FileStore::new(dir.0.join("accounts"))),
    )
    .expect("opens")
}

fn credentials(access: &str) -> Credentials {
    let mut credentials = Credentials::new();
    credentials.insert("credentials_file", format!("{{\"access\":\"{access}\"}}"));
    credentials
}

fn renewal(
    id: &str,
    from: u64,
    sequence: u64,
    access: &str,
    expires: Option<&str>,
) -> AccountRenewal {
    let mut renewal = AccountRenewal::new(id, from, sequence, credentials(access));
    renewal.expires_at = expires.map(str::to_owned);
    renewal
}

fn stored(accounts: &Accounts, id: &str) -> Record {
    accounts.load(id).expect("reads").expect("there")
}

#[test]
fn a_lost_login_marks_the_account_and_keeps_the_stored_copy_for_the_next_start() {
    let dir = Dir::new();
    let accounts = accounts(&dir);
    accounts
        .store_login("a", &credentials("1"), None)
        .expect("stored");
    accounts.lost(
        1,
        "a",
        &AccountLoss::new("a", 1, "the harness deleted its login"),
    );
    let record = stored(&accounts, "a");
    assert!(record.needs_sign_in);
    assert_eq!(record.credentials["credentials_file"], "{\"access\":\"1\"}");

    // The next start is handed the stored copy.
    let config = config("a");
    let mut info = AccountKindInfo::new("claude-login", "Claude");
    info.state_dir = true;
    let held = accounts
        .begin("work", &config, &info, "a test")
        .expect("held");
    assert_eq!(
        held.grant().credentials.expose().get("credentials_file"),
        Some("{\"access\":\"1\"}")
    );
    // A sign-in replaces the copy and clears the mark.
    accounts
        .store_login("a", &credentials("2"), None)
        .expect("stored");
    assert!(!stored(&accounts, "a").needs_sign_in);
}

#[test]
fn an_account_that_rotates_is_held_by_one_connection_at_a_time() {
    let dir = Dir::new();
    let accounts = accounts(&dir);
    accounts
        .store_login("a", &credentials("1"), None)
        .expect("stored");
    let config = config("a");
    let mut rotating = AccountKindInfo::new("claude-login", "Claude");
    rotating.rotates = true;
    rotating.state_dir = true;

    let first = accounts
        .begin("work", &config, &rotating, "the first")
        .expect("held");
    match accounts.begin("work", &config, &rotating, "the second") {
        Err(Refusal::InUse(holder)) => assert_eq!(holder, "the first"),
        other => panic!("a second holder: {other:?}"),
    }
    assert!(accounts.held("a"));
    // Neither signing in again nor removing it can take it from the first.
    assert!(accounts.exclusive("a").is_err());
    drop(first);
    assert!(!accounts.held("a"));
    let second = accounts
        .begin("work", &config, &rotating, "the second")
        .expect("held after");
    drop(second);

    // A kind that does not rotate is shared.
    let mut shared = AccountKindInfo::new("claude-login", "Claude");
    shared.state_dir = true;
    let one = accounts
        .begin("work", &config, &shared, "one")
        .expect("held");
    let two = accounts
        .begin("work", &config, &shared, "two")
        .expect("held too");
    drop((one, two));
}

#[test]
fn the_files_a_provider_placed_go_with_the_last_connection() {
    let dir = Dir::new();
    let accounts = accounts(&dir);
    accounts
        .store_login("a", &credentials("1"), None)
        .expect("stored");
    let config = config("a");
    let mut shared = AccountKindInfo::new("claude-login", "Claude");
    shared.state_dir = true;
    let one = accounts
        .begin("work", &config, &shared, "one")
        .expect("held");
    let two = accounts
        .begin("work", &config, &shared, "two")
        .expect("held too");
    let state = accounts.state_dir("a");
    let place = |held: &Held| {
        held.spawner()
            .write_file(
                ".credentials.json",
                coqui_acp_api::guatiao::Secret::new(coqui_acp_api::SecretBytes::new(b"x".to_vec())),
                true,
            )
            .expect("placed");
    };
    place(&one);
    assert!(state.join(".credentials.json").is_file());
    drop(one);
    assert!(
        state.join(".credentials.json").is_file(),
        "the second still uses it"
    );
    drop(two);
    assert!(
        !state.join(".credentials.json").exists(),
        "the last took it away"
    );
}

#[test]
fn an_endpoint_is_https_or_http_to_this_machine() {
    for good in [
        "https://llm.example",
        "https://llm.example:8443/v1",
        "http://127.0.0.1:8080",
        "http://localhost/x",
        "http://[::1]:9",
    ] {
        assert!(is_endpoint(good), "{good}");
    }
    for bad in [
        "http://llm.example",
        "ftp://llm.example",
        "llm.example",
        "https://",
        "https://user@llm.example",
        "http://127.0.0.1.example.com",
    ] {
        assert!(!is_endpoint(bad), "{bad}");
    }
}

fn config(id: &str) -> AccountConfig {
    AccountConfig {
        harness: "claude".to_owned(),
        kind: "claude-login".to_owned(),
        id: id.to_owned(),
        settings: None,
    }
}

/// What the chunking is for: Windows' Credential Manager holds at most 2560 bytes in one credential.
#[cfg(windows)]
#[test]
fn windows_refuses_one_credential_over_2560_bytes() {
    let _turn = super::store::one_at_a_time();
    let service = format!("dev.boriken.canoa.test.{}.{}", std::process::id(), line!());
    let entry = KeyringStore::new(service).entry("limit").expect("an entry");
    let at_limit = entry.set_secret(&vec![b'x'; 2560]);
    let _ = entry.delete_credential();
    let over = entry.set_secret(&vec![b'x'; 2561]);
    let _ = entry.delete_credential();
    if at_limit
        .as_ref()
        .is_err_and(|error| store_unavailable(&format!("{error:?}")))
    {
        eprintln!("the store is not usable just now, skipped");
        return;
    }
    at_limit.expect("2560 bytes are held");
    assert!(over.is_err(), "2561 bytes were held in one credential");
}

mod more;

/// Whether the operating system's store cannot be used here: a Linux with no Secret Service, or a Windows that answers
/// "not enough memory" (error 8) to a write while the machine is short of it. The tests of that store are then skipped,
/// as they are for the first, and the file store is the one tested.
pub(crate) fn store_unavailable(why: &str) -> bool {
    cfg!(target_os = "linux") || why.contains("Platform failure") || why.contains("PlatformError(")
}
