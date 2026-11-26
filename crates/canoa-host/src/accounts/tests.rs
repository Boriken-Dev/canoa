//! Tests of the host's accounts: the stores, the newer-wins rule, the lease and the endpoint check.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use coqui_acp_api::{AccountKindInfo, AccountLoss, AccountRenewal, Credentials};

use super::store::real_store::RealStore;
use super::store::{FileStore, KeyringStore, SecretStore};
use super::*;
use crate::config::AccountConfig;

/// A directory of its own that goes with the test.
struct Dir(PathBuf);

impl Dir {
    fn new() -> Self {
        let dir = Self::fresh_path();
        fs::create_dir_all(&dir).expect("a directory");
        Self(dir)
    }

    fn fresh_path() -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos());
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let number = NEXT.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "canoa-accounts-{}-{stamp}-{number}",
            std::process::id()
        ))
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

/// The operating system's store, under a service of this run's own that is emptied afterwards, however the test ends. It
/// needs a credential store, which a headless Linux has none of: there it is ignored, and the file store is the one tested.
#[test]
#[cfg_attr(
    target_os = "linux",
    ignore = "needs the operating system's credential store: a Secret Service session on Linux"
)]
fn the_operating_systems_store_round_trips_a_record_over_the_blob_limit() {
    let real = RealStore::new();
    let store = real.store();
    let bytes = large();
    store
        .put("account-one", &bytes)
        .unwrap_or_else(|error| panic!("the store refused a record: {error}"));
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

/// An account a store instance only read counts as shown to it: when the platform store then answers "no such credential"
/// for its index, a put through that instance refuses rather than writing generation 1 over the live record.
#[test]
fn a_store_that_only_read_an_account_does_not_write_over_it_when_its_index_goes_missing() {
    let service = format!("in-memory.{}", line!());
    let writer = KeyringStore::new(service.clone());
    let reader = KeyringStore::new(service);
    writer.put("account-read", b"first").expect("writes");
    assert_eq!(
        reader.get("account-read").expect("reads"),
        Some(b"first".to_vec())
    );
    // The index entry alone goes, as it reads when the platform store answers a miss under load.
    writer
        .entry(&KeyringStore::index_user("account-read"))
        .expect("an entry")
        .delete_credential()
        .expect("deletes the index");
    let refused = reader.put("account-read", b"second");
    assert!(
        refused.is_err(),
        "a put wrote generation 1 over an account the instance had read"
    );
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
        .begin("work", &config, &info, Runs::plain("claude"), "a test")
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
        .begin(
            "work",
            &config,
            &rotating,
            Runs::plain("claude"),
            "the first",
        )
        .expect("held");
    match accounts.begin(
        "work",
        &config,
        &rotating,
        Runs::plain("claude"),
        "the second",
    ) {
        Err(Refusal::InUse(holder)) => assert_eq!(holder, "the first"),
        other => panic!("a second holder: {other:?}"),
    }
    assert!(accounts.held("a"));
    // Neither signing in again nor removing it can take it from the first.
    assert!(accounts.exclusive("a").is_err());
    drop(first);
    assert!(!accounts.held("a"));
    let second = accounts
        .begin(
            "work",
            &config,
            &rotating,
            Runs::plain("claude"),
            "the second",
        )
        .expect("held after");
    drop(second);

    // A kind that does not rotate is shared.
    let mut shared = AccountKindInfo::new("claude-login", "Claude");
    shared.state_dir = true;
    let one = accounts
        .begin("work", &config, &shared, Runs::plain("claude"), "one")
        .expect("held");
    let two = accounts
        .begin("work", &config, &shared, Runs::plain("claude"), "two")
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
        .begin("work", &config, &shared, Runs::plain("claude"), "one")
        .expect("held");
    let two = accounts
        .begin("work", &config, &shared, Runs::plain("claude"), "two")
        .expect("held too");
    let state = accounts.state_dir("a");
    let place = |held: &Held| {
        held.spawner(|_| {})
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

fn endpoint_kind() -> AccountKindInfo {
    use serde_json::json;
    let mut info = AccountKindInfo::new("gateway", "Gateway");
    info.settings = guatiao_serde::from_serde(json!({
        "type": "object",
        "properties": {"base_url": {"type": "string"}},
    }))
    .expect("a schema");
    info.endpoints =
        vec![coqui_acp_api::EndpointSlot::new(coqui_acp_api::MODEL_API).key("base_url")];
    info
}

fn settings_of_address(address: &str) -> crate::config::Settings {
    use serde_json::json;
    crate::config::Settings::new(
        json!({"base_url": address})
            .as_object()
            .cloned()
            .expect("a table"),
    )
}

#[test]
fn an_endpoint_is_any_http_or_https_address_and_plain_http_beyond_this_machine_is_told_of() {
    let info = endpoint_kind();
    for good in [
        "https://llm.example",
        "https://llm.example:8443/v1",
        "http://127.0.0.1:8080",
        "http://localhost/x",
        "http://[::1]:9",
        "http://192.0.2.11:4000",
        "http://llm.example",
    ] {
        let settings = settings_of_address(good);
        assert_eq!(check_settings(&info, Some(&settings)), Ok(()), "{good}");
    }
    for bad in [
        "ftp://llm.example",
        "llm.example",
        "https://",
        "https://user@llm.example",
    ] {
        let settings = settings_of_address(bad);
        assert!(check_settings(&info, Some(&settings)).is_err(), "{bad}");
    }
    for quiet in ["https://llm.example", "http://127.0.0.1:8080"] {
        let settings = settings_of_address(quiet);
        assert!(
            address_notes("gw", &info, Some(&settings)).is_empty(),
            "{quiet}"
        );
    }
    let settings = settings_of_address("http://192.0.2.11:4000");
    let notes = address_notes("gw", &info, Some(&settings));
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(
        notes[0].starts_with("account gw: base_url is plain http to 192.0.2.11"),
        "{notes:?}"
    );
}

fn config(id: &str) -> AccountConfig {
    AccountConfig {
        owner: None,
        kind: "claude-login".to_owned(),
        id: id.to_owned(),
        settings: None,
        route: None,
    }
}

/// What the chunking is for: Windows' Credential Manager holds at most 2560 bytes in one credential.
#[cfg(windows)]
#[test]
fn windows_refuses_one_credential_over_2560_bytes() {
    let real = RealStore::new();
    let _turn = super::store::one_at_a_time();
    let entry = real.store().entry("limit").expect("an entry");
    let at_limit = entry.set_secret(&vec![b'x'; 2560]);
    let _ = entry.delete_credential();
    let over = entry.set_secret(&vec![b'x'; 2561]);
    let _ = entry.delete_credential();
    at_limit.expect("2560 bytes are held");
    assert!(over.is_err(), "2561 bytes were held in one credential");
}

mod more;

/// The clock behind a directory's name ticks more slowly than tests start, so the name alone is not unique: two tests that
/// began together shared a directory, and one's cleanup removed the other's files.
#[test]
fn directories_made_at_the_same_moment_are_different() {
    const THREADS: usize = 8;
    const ROUNDS: usize = 500;
    let start = std::sync::Barrier::new(THREADS);
    let made = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..THREADS {
            scope.spawn(|| {
                for _ in 0..ROUNDS {
                    start.wait();
                    let path = Dir::fresh_path();
                    made.lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .push(path);
                }
            });
        }
    });
    let made = made.into_inner().unwrap_or_else(PoisonError::into_inner);
    let distinct: std::collections::HashSet<_> = made.iter().collect();
    assert_eq!(distinct.len(), made.len(), "two directories got one name");
}

/// A read of a stored account while another thread writes it over never fails and never finds it gone.
#[test]
fn a_file_account_read_while_it_is_written_over_is_always_there() {
    const READERS: usize = 4;
    const WRITES: usize = 2_000;
    let dir = Dir::new();
    let store = FileStore::new(&dir.0);
    store.put("one", b"first").expect("written");
    let writing = std::sync::atomic::AtomicBool::new(true);
    let failures = std::sync::Mutex::new(Vec::new());
    let reads = AtomicU64::new(0);
    std::thread::scope(|scope| {
        for _ in 0..READERS {
            scope.spawn(|| {
                while writing.load(Ordering::Relaxed) {
                    reads.fetch_add(1, Ordering::Relaxed);
                    match store.get("one") {
                        Ok(Some(_)) => {}
                        other => failures
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .push(format!("{other:?}")),
                    }
                }
            });
        }
        for round in 0..WRITES {
            store
                .put("one", format!("round {round}").as_bytes())
                .expect("written");
        }
        writing.store(false, Ordering::Relaxed);
    });
    let failures = failures
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert!(
        failures.is_empty(),
        "{} of {} reads failed, first: {:?}",
        failures.len(),
        reads.load(Ordering::Relaxed),
        failures.first()
    );
}

fn a_record() -> Record {
    let mut record = Record::default();
    record.revision = 3;
    record
}

fn no_bus(_: &str) -> Result<(), StoreError> {
    Err(StoreError(
        "the operating system's credential store is not available (no session bus)".into(),
    ))
}

/// The configuration a host with `text` for canoa.cfg has.
fn config_of(text: &str) -> crate::config::Config {
    crate::config::Config::from_str_in(text, &std::env::temp_dir()).expect("canoa.cfg parses")
}

#[test]
fn auto_keeps_accounts_in_sealed_files_and_says_why_when_the_store_does_not_answer() {
    let dir = Dir::new();
    let accounts = Accounts::open_probing(&config_of(""), &dir.0, &no_bus).expect("opens on files");
    let note = accounts.note().expect("a note");
    assert!(
        note.starts_with("non-default: [secrets] accounts is auto and "),
        "{note}"
    );
    assert!(note.contains("no session bus"), "{note}");
    assert!(accounts.save("one", &a_record()).is_ok());
    assert_eq!(
        accounts.load("one").expect("reads").expect("held").revision,
        3
    );
    assert!(holds_secret_files_in(&dir.0));
    assert!(!holds_files_in(&dir.0), "the earlier folder is not written");
}

#[test]
fn naming_the_system_store_where_it_does_not_answer_is_refused_in_plain_words() {
    let dir = Dir::new();
    let error = Accounts::open_probing(
        &config_of(
            "[secrets]
accounts = \"system\"
",
        ),
        &dir.0,
        &no_bus,
    )
    .expect_err("there is no store to keep them in");
    let said = error.to_string();
    assert!(
        said.starts_with("[secrets] accounts is \"system\""),
        "{said}"
    );
    assert!(
        said.contains("\"auto\"") && said.contains("\"file\""),
        "{said}"
    );
    // `file` and `auto` still open on the same host.
    assert!(
        Accounts::open_probing(
            &config_of(
                "[secrets]
accounts = \"file\"
"
            ),
            &dir.0,
            &no_bus
        )
        .is_ok()
    );
}

#[test]
fn auto_uses_the_operating_systems_store_when_it_answers() {
    let dir = Dir::new();
    let accounts = Accounts::open_probing(&config_of(""), &dir.0, &|_| Ok(())).expect("opens");
    assert!(accounts.note().is_none());
    // Nothing is written to the operating system's store: the choice is what is checked.
    assert_eq!(accounts.store.describe(), "a secret provider's store");
}

#[test]
fn accounts_kept_before_the_move_are_moved_into_the_provider_when_they_are_first_read_and_keep_working()
 {
    let dir = Dir::new();
    // An earlier host's sealed file in the accounts folder.
    let earlier = FileStore::new(dir.0.join("accounts"));
    let mut record = a_record();
    record
        .credentials
        .insert("TOKEN".to_owned(), "an-earlier-secret-zq9".to_owned());
    earlier
        .put("one", &serde_json::to_vec(&record).expect("a record"))
        .expect("kept by the earlier host");
    assert!(holds_files_in(&dir.0));
    let accounts = Accounts::open_probing(
        &config_of("[secrets]\naccounts = \"file\"\n"),
        &dir.0,
        &no_bus,
    )
    .expect("opens");
    let read = accounts.load("one").expect("reads").expect("still held");
    assert_eq!(read.revision, 3);
    assert_eq!(
        read.credentials.get("TOKEN").map(String::as_str),
        Some("an-earlier-secret-zq9")
    );
    assert!(!holds_files_in(&dir.0), "the earlier copy is gone");
    assert!(holds_secret_files_in(&dir.0), "the provider holds it now");
    // It is read from the provider from now on, and is no more where it was.
    assert!(earlier.get("one").expect("asks").is_none());
    assert_eq!(
        accounts.load("one").expect("reads").expect("held").revision,
        3
    );
    // Removing an account clears the provider and any place that still held it.
    earlier
        .put("two", &serde_json::to_vec(&a_record()).expect("a record"))
        .expect("another");
    accounts.forget_credentials("one").expect("forgotten");
    accounts.forget_credentials("two").expect("forgotten");
    assert!(accounts.load("one").expect("reads").is_none());
    assert!(accounts.load("two").expect("reads").is_none());
    assert!(earlier.get("two").expect("asks").is_none());
}

#[test]
fn an_explicit_file_choice_is_noted_and_an_entry_the_person_made_is_not() {
    let dir = Dir::new();
    let files = Accounts::open_probing(
        &config_of("[secrets]\naccounts = \"file\"\n"),
        &dir.0,
        &no_bus,
    )
    .expect("opens");
    assert!(
        files
            .note()
            .expect("a note")
            .contains("[secrets] accounts is file")
    );
    let other = Dir::new();
    let folder = other.0.join("mine");
    let text = format!(
        "[secrets]\naccounts = \"mine\"\n[secrets.providers.mine]\nprovider = \"file\"\ndirectory = {:?}\n",
        folder.to_string_lossy().replace('\\', "/")
    );
    let chosen = Accounts::open_probing(&config_of(&text), &other.0, &no_bus).expect("opens");
    assert!(chosen.note().is_none());
    chosen.save("one", &a_record()).expect("saved");
    assert!(store::holds_files(&folder), "it rests where the entry says");
    assert!(!holds_secret_files_in(&other.0));
}

/// An earlier host's record in the operating system's store, under the service of this home alone, is moved the same way. The entry is
/// made for the test, under a service nothing else uses, and is gone when it ends.
#[test]
#[cfg_attr(
    target_os = "linux",
    ignore = "needs the operating system's credential store: a Secret Service session on Linux"
)]
fn an_account_the_operating_systems_store_held_under_the_homes_own_service_is_moved_too() {
    let dir = Dir::new();
    let earlier = KeyringStore::new(store::service_for(&dir.0));
    let mut record = a_record();
    record
        .credentials
        .insert("TOKEN".to_owned(), "an-os-secret-zq9".to_owned());
    earlier
        .put("one", &serde_json::to_vec(&record).expect("a record"))
        .expect("the operating system's store holds a record");
    let accounts = Accounts::open_probing(
        &config_of("[secrets]\naccounts = \"file\"\n"),
        &dir.0,
        &no_bus,
    )
    .expect("opens");
    let read = accounts.load("one").expect("reads").expect("still held");
    assert_eq!(
        read.credentials.get("TOKEN").map(String::as_str),
        Some("an-os-secret-zq9")
    );
    assert!(
        earlier.get("one").expect("asks").is_none(),
        "the earlier copy is gone"
    );
    assert!(holds_secret_files_in(&dir.0));
    accounts.forget_credentials("one").expect("forgotten");
}

/// Two homes of one user, the same account id in both: each keeps its own value and lists only its own secrets, since the operating
/// system's store is kept under a service of the home. The entries are made under services of these homes alone and removed.
#[test]
#[cfg_attr(
    target_os = "linux",
    ignore = "needs the operating system's credential store: a Secret Service session on Linux"
)]
fn two_homes_with_the_same_account_id_keep_separate_values_and_list_only_their_own() {
    use crate::secrets::catalog::Catalog;
    use crate::secrets::vault::Vault;

    let first = Dir::new();
    let second = Dir::new();
    let text = "[secrets]\naccounts = \"system\"\n";
    let open =
        |home: &Dir| Accounts::open_probing(&config_of(text), &home.0, &|_| Ok(())).expect("opens");
    let (a, b) = (open(&first), open(&second));
    let record = |revision: u64, token: &str| {
        let mut record = a_record();
        record.revision = revision;
        record
            .credentials
            .insert("TOKEN".to_owned(), token.to_owned());
        record
    };
    a.save("alone-a", &record(1, "first-home-zq9"))
        .expect("the operating system's store holds a record");
    a.save("same", &record(1, "first-home-zq9")).expect("saved");
    b.save("alone-b", &record(2, "second-home-zq9"))
        .expect("saved");
    b.save("same", &record(2, "second-home-zq9"))
        .expect("saved");
    let token = |accounts: &Accounts| {
        accounts
            .load("same")
            .expect("reads")
            .expect("held")
            .credentials
            .get("TOKEN")
            .cloned()
    };
    assert_eq!(token(&a).as_deref(), Some("first-home-zq9"));
    assert_eq!(token(&b).as_deref(), Some("second-home-zq9"));
    let listed = |home: &Dir| {
        let vault = Vault::standalone(Catalog::built_in(), config_of(text).secrets, &home.0);
        let mut names: Vec<String> = vault
            .rows()
            .into_iter()
            .filter(|row| row.provider == "system")
            .map(|row| row.name)
            .collect();
        names.sort();
        names
    };
    assert_eq!(listed(&first), ["accounts/alone-a", "accounts/same"]);
    assert_eq!(listed(&second), ["accounts/alone-b", "accounts/same"]);
    for (accounts, ids) in [(&a, ["alone-a", "same"]), (&b, ["alone-b", "same"])] {
        for id in ids {
            accounts.forget_credentials(id).expect("forgotten");
        }
    }
    assert!(listed(&first).is_empty() && listed(&second).is_empty());
}

#[test]
fn the_sealed_files_are_under_the_home_of_the_host_that_keeps_them() {
    let first = Dir::new();
    let second = Dir::new();
    let text = "[secrets]\naccounts = \"file\"\n";
    let a = Accounts::open_probing(&config_of(text), &first.0, &no_bus).expect("opens");
    let b = Accounts::open_probing(&config_of(text), &second.0, &no_bus).expect("opens");
    a.save("same", &a_record()).expect("saved");
    assert!(holds_secret_files_in(&first.0));
    assert!(!holds_secret_files_in(&second.0));
    assert!(b.load("same").expect("reads").is_none());
}

#[test]
fn an_entry_that_keeps_nothing_it_is_given_cannot_hold_the_accounts() {
    let dir = Dir::new();
    let refused = Accounts::open_probing(
        &config_of("[secrets]\naccounts = \"form\"\n"),
        &dir.0,
        &no_bus,
    )
    .and_then(|accounts| accounts.save("one", &a_record()));
    assert!(refused.is_err());
    let unknown = Accounts::open_probing(
        &config_of("[secrets]\naccounts = \"ghost\"\n"),
        &dir.0,
        &no_bus,
    );
    assert!(unknown.is_err());
}

fn holds_files_in(home: &std::path::Path) -> bool {
    store::holds_files(&home.join("accounts"))
}

/// Whether the home's folder for sealed secrets holds one.
fn holds_secret_files_in(home: &std::path::Path) -> bool {
    store::holds_files(&home.join("secrets"))
}

#[test]
fn a_preview_names_the_directory_the_settings_and_the_standards_with_no_credential_and_makes_nothing()
 {
    let dir = Dir::new();
    let accounts = accounts(&dir);
    let mut info = AccountKindInfo::new("claude-login", "Claude");
    info.state_dir = true;
    let grant = accounts.preview(&config("id-p"), &info, Runs::plain("claude"));
    let expected = accounts.state_dir("id-p");
    assert_eq!(
        grant.state_dir.as_deref(),
        Some(expected.to_string_lossy().as_ref())
    );
    assert!(grant.credentials.expose().is_empty());
    assert!(!expected.exists(), "a preview makes no directory");
    // The login the program has where it runs has no directory of the host's.
    let mut login = config("id-p");
    login.kind = coqui_acp_api::HARNESS_LOGIN.to_owned();
    assert!(
        accounts
            .preview(&login, &info, Runs::plain("claude"))
            .state_dir
            .is_none()
    );
}

/// Every harness has an account called `local`, so usage is recorded under an id that names the harness too; any other account is
/// recorded under its own id.
#[test]
fn the_local_account_of_each_harness_is_recorded_under_an_id_of_its_own() {
    let dir = Dir::new();
    let accounts = accounts(&dir);
    let login = AccountKindInfo::new(coqui_acp_api::HARNESS_LOGIN, "The login it has");
    let held = |harness: &str| {
        let local = AccountConfig::local(harness);
        let held = accounts
            .begin("local", &local, &login, Runs::plain(harness), "a test")
            .expect("held");
        assert_eq!(held.usage_id(), local.usage_id());
        held.usage_id()
    };
    let (claude, codex) = (held("claude"), held("codex"));
    assert_eq!(claude, "local:claude");
    assert_eq!(codex, "local:codex");

    accounts
        .store_login("id-p", &credentials("1"), None)
        .expect("stored");
    let own = config("id-p");
    let key = AccountKindInfo::new("claude-login", "Claude");
    let held = accounts
        .begin("work", &own, &key, Runs::plain("claude"), "a test")
        .expect("held");
    assert_eq!(held.usage_id(), "id-p");
    assert_eq!(own.usage_id(), "id-p");
}

/// An account that names an address and the form of its key for each standard is given each harness the address and the key of the
/// standard that harness applies it as, and the account's maker is given what the account holds.
#[test]
fn a_harness_is_given_the_address_and_the_key_form_the_account_names_for_the_standard_it_applies() {
    use coqui_acp_api::{Accepted, AcceptedSlot, AccountCatalog, standard};
    use serde_json::{Value as Json, json};

    let dir = Dir::new();
    let accounts = accounts(&dir);
    accounts
        .store_login("gw", &Credentials::from_iter([("api_key", "k")]), None)
        .expect("stored");
    let mut info = AccountKindInfo::new("gateway", "Gateway")
        .provides([standard::OPENAI_API, standard::ANTHROPIC_API]);
    info.settings = guatiao_serde::from_serde(json!({
        "type": "object",
        "properties": {
            "base_url": {"type": "string"},
            "standards": {"type": "array", "items": {"type": "string"}, "x-provides": true},
            "by_standard": {"type": "object", "x-by-standard": true},
        },
    }))
    .expect("a schema");
    info.endpoints =
        vec![coqui_acp_api::EndpointSlot::new(coqui_acp_api::MODEL_API).key("base_url")];
    let account = AccountConfig {
        owner: None,
        kind: "gateway".to_owned(),
        id: "gw".to_owned(),
        settings: Some(crate::config::Settings::new(
            json!({
                "base_url": "https://gw.example/default",
                "standards": ["openai-api", "anthropic-api"],
                "by_standard": {
                    "openai-api": {"base_url": "https://gw.example/v1"},
                    "anthropic-api": {"base_url": "https://gw.example", "credential": "auth_token"},
                },
            })
            .as_object()
            .cloned()
            .expect("a table"),
        )),
        route: None,
    };
    let catalog = |standard: &str, credential: &str| {
        let mut catalog = AccountCatalog::new();
        catalog.accepts = vec![
            Accepted::new(standard)
                .slot(AcceptedSlot::new(credential, "own-key").base_url("base_url", None)),
        ];
        catalog
    };
    let run = |harness: &str, catalog: &AccountCatalog| {
        let runs = Runs {
            harness,
            catalog: Some(catalog),
            makers: None,
        };
        let held = accounts
            .begin("gw", &account, &info, runs, "holder")
            .expect("held");
        let grant = held.grant();
        let settings = guatiao_serde::adapt::from_value::<Json>(&grant.settings).expect("JSON");
        let keys: Vec<String> = grant.credentials.expose().keys().cloned().collect();
        let preview = accounts.preview(&account, &info, runs);
        let previewed = guatiao_serde::adapt::from_value::<Json>(&preview.settings).expect("JSON");
        assert_eq!(previewed, settings, "a profile is told what the harness is");
        // The maker is given what the account holds, whichever harness runs it.
        let maker = held.provider_grant();
        assert_eq!(
            maker
                .credentials
                .expose()
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
            ["api_key"]
        );
        let made = guatiao_serde::adapt::from_value::<Json>(&maker.settings).expect("JSON");
        assert_eq!(made["base_url"], "https://gw.example/default");
        assert!(made.get("by_standard").is_some() && made.get("standards").is_none());
        (settings, keys)
    };
    let (settings, keys) = run("codex", &catalog("openai-api", "api_key"));
    assert_eq!(settings, json!({"base_url": "https://gw.example/v1"}));
    assert_eq!(keys, ["api_key"]);
    let (settings, keys) = run("claude", &catalog("anthropic-api", "api_key"));
    assert_eq!(settings, json!({"base_url": "https://gw.example"}));
    assert_eq!(keys, ["auth_token"], "the key as a bearer token");
}

/// A name that is absent while a file made beside it is still there is a write in progress, not a missing account.
#[test]
fn a_file_account_absent_while_its_replacement_is_being_put_in_place_is_waited_for() {
    let dir = Dir::new();
    let store = FileStore::new(&dir.0);
    store.put("one", b"first").expect("written");
    let path = store.path_of("one");
    let beside = dir.0.join(format!(
        "{}.tmp.1",
        path.file_stem().expect("a stem").to_string_lossy()
    ));
    let sealed = fs::read(&path).expect("read");
    fs::write(&beside, &sealed).expect("beside");
    fs::remove_file(&path).expect("absent");
    std::thread::scope(|scope| {
        scope.spawn(|| {
            std::thread::sleep(core::time::Duration::from_millis(150));
            fs::rename(&beside, &path).expect("put in place");
        });
        assert_eq!(store.get("one").expect("read"), Some(b"first".to_vec()));
    });
    // With nothing beside it, an absent file is an account that is not there.
    fs::remove_file(&path).expect("removed");
    assert_eq!(store.get("one").expect("read"), None);
}
