//! Tests of the renewal chain, validation, private directories, sweeps, the release race and the stores' names.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use super::*;
use crate::accounts::store;

fn place(held: &Held) {
    held.spawner(|_| {})
        .write_file(
            ".credentials.json",
            coqui_acp_api::guatiao::Secret::new(coqui_acp_api::SecretBytes::new(b"x".to_vec())),
            true,
        )
        .expect("placed");
}

fn rotating() -> AccountKindInfo {
    let mut info = AccountKindInfo::new("claude-login", "Claude");
    info.rotates = true;
    info.state_dir = true;
    info
}

#[test]
fn every_renewal_of_one_hold_is_stored_in_the_order_of_its_sequence() {
    let dir = Dir::new();
    let accounts = accounts(&dir);
    accounts
        .store_login(
            "a",
            &credentials("1"),
            Some("2026-11-11T10:00:00Z".to_owned()),
        )
        .expect("stored");
    // A provider's hold names the revision of the grant it was given in every renewal of the hold, and counts them.
    accounts.renewed(
        1,
        "a",
        &renewal("a", 1, 1, "2", Some("2026-11-11T18:00:00Z")),
    );
    accounts.renewed(
        1,
        "a",
        &renewal("a", 1, 2, "3", Some("2026-11-12T02:00:00Z")),
    );
    accounts.renewed(
        1,
        "a",
        &renewal("a", 1, 3, "4", Some("2026-11-12T10:00:00Z")),
    );
    let record = stored(&accounts, "a");
    assert_eq!(record.revision, 4);
    assert_eq!(record.credentials["credentials_file"], "{\"access\":\"4\"}");

    // The same event heard again changes nothing.
    accounts.renewed(
        1,
        "a",
        &renewal("a", 1, 2, "3", Some("2026-11-12T02:00:00Z")),
    );
    assert_eq!(stored(&accounts, "a").revision, 4);

    // Another provider instance that began from the first copy is a stale report, however late its expiry.
    accounts.renewed(
        2,
        "a",
        &renewal("a", 1, 1, "stale", Some("2027-01-01T00:00:00Z")),
    );
    let record = stored(&accounts, "a");
    assert_eq!(record.revision, 4);
    assert_eq!(record.credentials["credentials_file"], "{\"access\":\"4\"}");

    // A chain stops where another source replaced the copy: the first source's next report is then stale.
    accounts.renewed(
        2,
        "a",
        &renewal("a", 4, 2, "5", Some("2026-11-13T00:00:00Z")),
    );
    assert_eq!(stored(&accounts, "a").revision, 5);
    accounts.renewed(
        1,
        "a",
        &renewal("a", 1, 4, "6", Some("2026-11-14T00:00:00Z")),
    );
    assert_eq!(stored(&accounts, "a").revision, 5);
}

#[test]
fn a_report_of_an_earlier_expiry_from_a_source_that_did_not_chain_is_ignored() {
    let dir = Dir::new();
    let accounts = accounts(&dir);
    accounts
        .store_login(
            "a",
            &credentials("1"),
            Some("2026-11-11T10:00:00Z".to_owned()),
        )
        .expect("stored");
    accounts.renewed(
        1,
        "a",
        &renewal("a", 1, 1, "older", Some("2026-11-11T09:00:00Z")),
    );
    assert_eq!(stored(&accounts, "a").revision, 1);
}

#[test]
fn a_renewal_that_is_empty_or_lacks_what_the_stored_login_has_is_never_stored() {
    let dir = Dir::new();
    let accounts = accounts(&dir);
    accounts
        .store_login("a", &credentials("1"), None)
        .expect("stored");
    let mut empty_map = renewal("a", 1, 1, "x", None);
    empty_map.credentials = coqui_acp_api::guatiao::Secret::new(Credentials::new());
    let mut empty_value = Credentials::new();
    empty_value.insert("credentials_file", "");
    let mut other_key = Credentials::new();
    other_key.insert("something_else", "{}");
    accounts.renewed(1, "a", &empty_map);
    for (sequence, credentials) in [(2, empty_value), (3, other_key)] {
        accounts.renewed(1, "a", &AccountRenewal::new("a", 1, sequence, credentials));
    }
    let record = stored(&accounts, "a");
    assert_eq!(record.revision, 1);
    assert_eq!(record.credentials["credentials_file"], "{\"access\":\"1\"}");
}

#[test]
fn the_accounts_directories_and_files_are_the_owners_alone() {
    let dir = Dir::new();
    let accounts = accounts(&dir);
    accounts
        .store_login("a", &credentials("1"), None)
        .expect("stored");
    let held = accounts
        .begin("work", &config("a"), &rotating(), "a test")
        .expect("held");
    let root = dir.0.join("accounts");
    #[cfg(unix)]
    let own = accounts.account_dir("a");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let login = accounts.login_dir().expect("a sign-in directory");
        for (path, wanted) in [
            (root.clone(), 0o700),
            (own.clone(), 0o700),
            (own.join("state"), 0o700),
            (own.join("lease"), 0o600),
            (login.path().to_owned(), 0o700),
        ] {
            let mode = fs::metadata(&path).expect("exists").permissions().mode() & 0o777;
            assert_eq!(mode, wanted, "{}", path.display());
        }
    }
    #[cfg(windows)]
    {
        let listed = std::process::Command::new("icacls")
            .arg(&root)
            .output()
            .expect("icacls runs");
        let listed = String::from_utf8_lossy(&listed.stdout).into_owned();
        let user = std::env::var("USERNAME").expect("a user").to_lowercase();
        let mut principals = Vec::new();
        for line in listed.lines().filter(|line| line.contains(":(")) {
            let line = line
                .trim()
                .strip_prefix(&root.display().to_string())
                .unwrap_or(line)
                .trim();
            principals.push(
                line.split(":(")
                    .next()
                    .expect("a principal")
                    .trim()
                    .to_lowercase(),
            );
        }
        assert!(!principals.is_empty(), "{listed}");
        for principal in &principals {
            assert!(
                principal.ends_with(&user) || principal.ends_with("system"),
                "{principal} is granted access to {}:\n{listed}",
                root.display()
            );
        }
        assert!(principals.iter().any(|p| p.ends_with("system")), "{listed}");
    }
    drop(held);
}

#[test]
fn what_a_crash_left_in_the_accounts_directory_is_swept_when_it_opens() {
    let dir = Dir::new();
    let root = dir.0.join("accounts");
    let leftover = root.join("login").join("0123abcd");
    fs::create_dir_all(&leftover).expect("made");
    fs::write(leftover.join(".credentials.json"), "x").expect("written");
    let accounts = accounts(&dir);
    assert!(
        !leftover.exists(),
        "a sign-in directory a crash left was kept"
    );

    // A login file a connection placed and a crash left is swept too, once nobody holds the account.
    accounts
        .store_login("a", &credentials("1"), None)
        .expect("stored");
    let mut shared = AccountKindInfo::new("claude-login", "Claude");
    shared.state_dir = true;
    let held = accounts
        .begin("work", &config("a"), &shared, "one")
        .expect("held");
    let state = accounts.state_dir("a");
    place(&held);
    let own = accounts.account_dir("a");
    let note = fs::read(own.join("placed")).expect("what was placed is written down, for a crash");
    drop(held);
    assert!(!state.join(".credentials.json").exists());
    // As a crash leaves it: the file there and the note of it.
    fs::write(state.join(".credentials.json"), "x").expect("restored");
    fs::write(own.join("placed"), note).expect("restored");
    drop(accounts);
    let reopened =
        Accounts::with_store(root.clone(), Box::new(FileStore::new(root))).expect("opens");
    assert!(
        !state.join(".credentials.json").exists(),
        "a file a crash left was kept"
    );
    drop(reopened);
}

#[test]
fn a_file_a_new_holder_placed_survives_the_old_holder_letting_go() {
    let dir = Dir::new();
    let accounts = accounts(&dir);
    accounts
        .store_login("a", &credentials("1"), None)
        .expect("stored");
    let file = accounts.state_dir("a").join(".credentials.json");
    for round in 0..150 {
        let old = accounts
            .begin("work", &config("a"), &rotating(), "old")
            .expect("held");
        place(&old);
        let letting_go = std::thread::spawn(move || drop(old));
        let new = loop {
            match accounts.begin("work", &config("a"), &rotating(), "new") {
                Ok(new) => break new,
                Err(_) => std::thread::yield_now(),
            }
        };
        place(&new);
        letting_go.join().expect("it ends");
        assert!(
            file.exists(),
            "round {round}: the new holder's file went with the old holder"
        );
        drop(new);
    }
}

#[test]
fn instances_making_the_sealing_key_at_once_end_with_one_key_all_can_use() {
    for _ in 0..20 {
        let dir = Dir::new();
        let root = dir.0.join("accounts");
        let handles: Vec<_> = (0..4)
            .map(|n| {
                let root = root.clone();
                std::thread::spawn(move || {
                    let store = FileStore::new(root);
                    let id = format!("id{n}");
                    store.put(&id, id.as_bytes()).map(|()| store.get(&id))
                })
            })
            .collect();
        for handle in handles {
            handle
                .join()
                .expect("ends")
                .expect("writes")
                .expect("reads");
        }
        let store = FileStore::new(&root);
        for n in 0..4 {
            let id = format!("id{n}");
            assert_eq!(store.get(&id).expect("reads"), Some(id.into_bytes()));
        }
    }
}

#[test]
fn an_id_with_hash_signs_does_not_collide_with_another_accounts_chunks() {
    let service = format!("dev.boriken.canoa.test.{}.{}", std::process::id(), line!());
    let store = KeyringStore::new(service);
    let _forget = Forget(&store, &["x", "x#1#0"]);
    if store
        .put("x", b"first")
        .is_err_and(|e| super::store_unavailable(&e.to_string()))
    {
        return;
    }
    // "x#1#0" is the name the first account's first chunk would have.
    let put = store.put("x#1#0", b"second");
    let first = store.get("x");
    let second = store.get("x#1#0");
    store.remove("x").expect("removes");
    store.remove("x#1#0").expect("removes");
    put.expect("puts");
    assert_eq!(first.expect("reads"), Some(b"first".to_vec()));
    assert_eq!(second.expect("reads"), Some(b"second".to_vec()));
}

/// Windows' store roams a credential with the profile unless it is made local.
#[cfg(windows)]
#[test]
fn the_windows_credential_is_local_to_this_machine() {
    let service = format!("dev.boriken.canoa.test.{}.{}", std::process::id(), line!());
    let store = KeyringStore::new(service);
    let _forget = Forget(&store, &["account-one"]);
    if store
        .put("account-one", b"x")
        .is_err_and(|e| super::store_unavailable(&e.to_string()))
    {
        return;
    }
    let persistence = store.persistence_of("account-one");
    store.remove("account-one").expect("removes");
    assert_eq!(persistence.as_deref(), Some("Local"));
}

#[test]
fn two_homes_have_two_services_and_no_id_is_in_a_name() {
    let one = store::service_for(Path::new("/home/a"));
    let two = store::service_for(Path::new("/home/b"));
    assert_ne!(one, two);
    assert!(one.starts_with("dev.boriken.canoa."), "{one}");
    assert!(!store::tag("x#1#0").contains('#'));
}

#[test]
fn a_chunk_a_failed_write_left_and_every_old_generation_are_removed_with_the_account() {
    let service = format!("dev.boriken.canoa.test.{}.{}", std::process::id(), line!());
    let store = KeyringStore::new(service);
    let _forget = Forget(&store, &["x"]);
    let big = vec![b'a'; 9_000];
    if store
        .put("x", &big)
        .is_err_and(|e| super::store_unavailable(&e.to_string()))
    {
        return;
    }
    // The record shrinks: the new generation has fewer chunks, and the old one's go.
    store.put("x", b"small").expect("puts");
    assert_eq!(store.get("x").expect("reads"), Some(b"small".to_vec()));
    // A write that stopped half way left chunks under the next generation: the next put clears them, those past its own
    // count included.
    for number in 0..3 {
        store
            .entry(&KeyringStore::chunk_user("x", 3, number))
            .expect("an entry")
            .set_secret(b"left over")
            .expect("planted");
    }
    store.put("x", b"third").expect("puts");
    let stale = store
        .entry(&KeyringStore::chunk_user("x", 3, 2))
        .expect("an entry")
        .get_secret();
    store.remove("x").expect("removes");
    let first_gone = store
        .entry(&KeyringStore::chunk_user("x", 1, 0))
        .expect("an entry")
        .get_secret();
    assert!(stale.is_err(), "a chunk of the failed write was kept");
    assert!(first_gone.is_err(), "an old generation's chunk was kept");
    assert!(store.get("x").expect("reads").is_none());
}

#[test]
fn an_index_that_cannot_be_read_is_not_written_over() {
    let service = format!("dev.boriken.canoa.test.{}.{}", std::process::id(), line!());
    let store = KeyringStore::new(service);
    let _forget = Forget(&store, &["x"]);
    if store
        .put("x", b"first")
        .is_err_and(|e| super::store_unavailable(&e.to_string()))
    {
        return;
    }
    store
        .entry(&KeyringStore::index_user("x"))
        .expect("an entry")
        .set_secret(b"not an index")
        .expect("damaged");
    let refused = store.put("x", b"second");
    let removed = store.remove("x");
    assert!(refused.is_err(), "a damaged index was written over");
    removed.expect("a damaged account can still be removed");
}

#[test]
fn a_store_failure_is_not_told_to_a_client() {
    let refusal =
        Refusal::Store("C:\\Users\\someone\\accounts: Access is denied. (os error 5)".to_owned());
    let told = refusal.client_reason();
    assert!(
        !told.contains("someone") && !told.contains("os error"),
        "{told}"
    );
    assert!(Refusal::NoCredentials.client_reason().contains("sign in"));
}

#[test]
fn a_sign_in_directory_goes_when_the_sign_in_does_and_a_live_one_is_not_swept() {
    let dir = Dir::new();
    let accounts = accounts(&dir);
    let live = accounts.login_dir().expect("a directory");
    fs::write(live.path().join(".credentials.json"), "x").expect("written");
    let gone = {
        let other = accounts.login_dir().expect("another");
        // Opening another sign-in sweeps the directories nothing holds, and not this one's.
        assert!(live.path().exists());
        other.path().to_owned()
    };
    assert!(!gone.exists(), "a sign-in directory outlived its sign-in");
    accounts.sweep_logins();
    assert!(
        live.path().join(".credentials.json").exists(),
        "a live sign-in was swept"
    );
}
