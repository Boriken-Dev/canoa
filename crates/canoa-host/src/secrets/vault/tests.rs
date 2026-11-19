//! The vault against stand-in providers: one that answers from its own settings (a password manager's part), one that keeps what it is
//! given (the system store's), each counting its calls so a test can see whether the provider was asked.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a test names what it uses from std"
)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use canoa::secret_provider::{
    Held, Keeping, Named, Names, ProviderInfo, SecretProvider, unavailable, unsupported,
};
use coqui_acp_api::ProcessSpawner;
use coqui_acp_api::guatiao::library::{Object, ProviderError, Registry};
use coqui_acp_api::guatiao::{FromValue, Provider, Schema, ToValue};

use super::*;

/// What a stand-in manager is configured with.
#[derive(ToValue, FromValue, Schema, Debug, Clone)]
struct ManagerSettings {
    /// Names its calls apart from every other test's.
    #[map(default)]
    tag: String,
    /// What it holds, as a JSON object of name to value.
    values: Option<String>,
    /// The end it gives every value, in seconds.
    expires_in: Option<u64>,
    /// The session key it is unlocked with: without it, it is locked.
    unlock: Option<String>,
    /// What it takes to open: when set, no key opens it.
    key: Option<String>,
}

static CALLS: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn calls(tag: &str) -> usize {
    lock(&CALLS).iter().filter(|each| *each == tag).count()
}

#[derive(Provider)]
#[provider(SecretProvider, id = "manager", config = ManagerSettings, crate = ::coqui_acp_api::guatiao)]
struct Manager {
    settings: ManagerSettings,
}

impl From<ManagerSettings> for Manager {
    fn from(settings: ManagerSettings) -> Self {
        Self { settings }
    }
}

impl SecretProvider for Manager {
    fn describe(&self) -> Result<ProviderInfo, ProviderError> {
        let mut info = ProviderInfo::new("Manager");
        info.can_list = true;
        Ok(info)
    }

    fn names(&self, _: Object<dyn ProcessSpawner>) -> Result<Names, ProviderError> {
        Err(unsupported("it lists nothing here"))
    }

    fn get(&self, request: Named, _: Object<dyn ProcessSpawner>) -> Result<Held, ProviderError> {
        lock(&CALLS).push(self.settings.tag.clone());
        if self.settings.unlock.is_some() && self.settings.unlock != self.settings.key {
            return Err(unavailable("the manager is locked"));
        }
        let values: HashMap<String, String> = self
            .settings
            .values
            .as_deref()
            .and_then(|text| serde_json::from_str(text).ok())
            .unwrap_or_default();
        Ok(match values.get(&request.name) {
            Some(value) => {
                let held = Held::of(value.clone());
                match self.settings.expires_in {
                    Some(seconds) => held.expiring(seconds),
                    None => held,
                }
            }
            None => Held::none(),
        })
    }

    fn put(&self, _: Keeping, _: Object<dyn ProcessSpawner>) -> Result<(), ProviderError> {
        Err(unsupported("it keeps nothing"))
    }

    fn forget(&self, _: Named, _: Object<dyn ProcessSpawner>) -> Result<(), ProviderError> {
        Err(unsupported("it keeps nothing"))
    }
}

#[derive(ToValue, FromValue, Schema, Debug, Clone)]
struct KeeperSettings {
    #[map(default)]
    tag: String,
}

static KEPT: Mutex<Vec<(String, String, String)>> = Mutex::new(Vec::new());

#[derive(Provider)]
#[provider(SecretProvider, id = "keeper", config = KeeperSettings, crate = ::coqui_acp_api::guatiao)]
struct Keeper {
    tag: String,
}

impl From<KeeperSettings> for Keeper {
    fn from(settings: KeeperSettings) -> Self {
        Self { tag: settings.tag }
    }
}

impl SecretProvider for Keeper {
    fn describe(&self) -> Result<ProviderInfo, ProviderError> {
        let mut info = ProviderInfo::new("Keeper");
        info.can_store = true;
        info.can_list = true;
        Ok(info)
    }

    fn names(&self, _: Object<dyn ProcessSpawner>) -> Result<Names, ProviderError> {
        let mut names: Vec<String> = lock(&KEPT)
            .iter()
            .filter(|(tag, _, _)| *tag == self.tag)
            .map(|(_, name, _)| name.clone())
            .collect();
        names.sort();
        Ok(Names::new(names))
    }

    fn get(&self, request: Named, _: Object<dyn ProcessSpawner>) -> Result<Held, ProviderError> {
        lock(&CALLS).push(self.tag.clone());
        Ok(lock(&KEPT)
            .iter()
            .find(|(tag, name, _)| *tag == self.tag && *name == request.name)
            .map_or_else(Held::none, |(_, _, value)| Held::of(value.clone())))
    }

    fn put(&self, request: Keeping, _: Object<dyn ProcessSpawner>) -> Result<(), ProviderError> {
        let mut kept = lock(&KEPT);
        kept.retain(|(tag, name, _)| !(*tag == self.tag && *name == request.name));
        kept.push((
            self.tag.clone(),
            request.name,
            request.value.expose().clone(),
        ));
        Ok(())
    }

    fn forget(&self, request: Named, _: Object<dyn ProcessSpawner>) -> Result<(), ProviderError> {
        lock(&KEPT).retain(|(tag, name, _)| !(*tag == self.tag && *name == request.name));
        Ok(())
    }
}

coqui_acp_api::guatiao::local_providers!(
    id = "canoa-secret-standins",
    version = "0.0.0",
    providers = [Manager, Keeper]
);

fn catalog() -> Catalog {
    let mut registry = Registry::new("canoa", "0.0.0");
    registry
        .register_local("canoa-secret-standins", library)
        .expect("registered");
    Catalog::from_registries(vec![("test".to_owned(), registry)])
}

struct Rig {
    vault: Arc<Vault>,
    clock: Arc<AtomicU64>,
}

impl Rig {
    fn seconds(&self, seconds: u64) {
        self.clock.fetch_add(seconds * 1000, Ordering::SeqCst);
    }
}

fn rig(section: Value) -> Rig {
    let clock = Arc::new(AtomicU64::new(1_000_000));
    let read = Arc::clone(&clock);
    let vault = Vault::with_clock(
        catalog(),
        Arc::new(move || Ok(Some(section.clone()))),
        Arc::new(move || read.load(Ordering::SeqCst)),
    );
    Rig { vault, clock }
}

/// A made-up value no real secret looks like, so a leak is found by its text.
const VALUE: &str = "zq9-not-a-real-secret-7f3a";

fn recalled(vault: &Vault, session: &str, name: &str) -> Option<String> {
    vault
        .recall(session, name)
        .map(|value| String::from_utf8(value.to_vec()).expect("text"))
}

#[test]
fn a_name_the_cache_holds_is_answered_and_one_it_does_not_is_left_for_the_form_to_ask() {
    let rig = rig(json!({}));
    assert_eq!(
        recalled(&rig.vault, "s1", "token"),
        None,
        "nothing is held yet"
    );
    rig.vault
        .remember(Some("s1"), "token", VALUE.as_bytes(), "close")
        .unwrap();
    assert_eq!(recalled(&rig.vault, "s1", "token").as_deref(), Some(VALUE));
    // Until Canoa closes, so another session asking for the name is answered too.
    assert_eq!(recalled(&rig.vault, "s2", "token").as_deref(), Some(VALUE));
    assert_eq!(recalled(&rig.vault, "s2", "other"), None);
}

#[test]
fn once_keeps_nothing_and_this_session_keeps_it_for_that_session_alone() {
    let rig = rig(json!({}));
    rig.vault
        .remember(Some("s1"), "once", VALUE.as_bytes(), "once")
        .unwrap();
    assert_eq!(recalled(&rig.vault, "s1", "once"), None);
    rig.vault
        .remember(Some("s1"), "mine", VALUE.as_bytes(), "session")
        .unwrap();
    assert_eq!(recalled(&rig.vault, "s1", "mine").as_deref(), Some(VALUE));
    assert_eq!(
        recalled(&rig.vault, "s2", "mine"),
        None,
        "another session is not given it"
    );
    rig.vault.end_session("s1");
    assert_eq!(
        recalled(&rig.vault, "s1", "mine"),
        None,
        "the session ended"
    );
}

#[test]
fn a_time_to_live_ends_and_the_table_counts_it_down() {
    let rig = rig(json!({}));
    rig.vault
        .remember(Some("s1"), "short", VALUE.as_bytes(), "15m")
        .unwrap();
    let row = rig.vault.rows().remove(0);
    assert_eq!(
        (row.provider.as_str(), row.name.as_str()),
        ("form", "short")
    );
    assert_eq!(row.kept, Kept::Process);
    assert_eq!(row.expires_in, Some(900));
    rig.seconds(899);
    assert_eq!(recalled(&rig.vault, "s2", "short").as_deref(), Some(VALUE));
    assert_eq!(rig.vault.rows()[0].expires_in, Some(1));
    rig.seconds(2);
    assert_eq!(recalled(&rig.vault, "s2", "short"), None);
    assert!(rig.vault.rows().is_empty());
}

#[test]
fn a_provider_gives_a_value_and_the_answer_is_kept_as_its_entry_says() {
    let tag = "gives";
    let rig = rig(json!({
        "names": {"api_key": "mgr"},
        "providers": {"mgr": {"provider": "manager", "keep": "close", "tag": tag,
            "values": "{\"api_key\": \"zq9-not-a-real-secret-7f3a\"}"}}
    }));
    assert_eq!(
        recalled(&rig.vault, "s1", "api_key").as_deref(),
        Some(VALUE)
    );
    assert_eq!(calls(tag), 1);
    // The second request is the cache's: the provider is not asked again.
    assert_eq!(
        recalled(&rig.vault, "s2", "api_key").as_deref(),
        Some(VALUE)
    );
    assert_eq!(calls(tag), 1);
    let row = &rig.vault.rows()[0];
    assert_eq!(row.provider, "mgr");
    assert_eq!(row.expires_in, None);
}

#[test]
fn what_a_provider_says_of_its_own_lifetime_ends_a_value_before_the_person_said_it_should() {
    let tag = "shorter";
    let rig = rig(json!({
        "names": {"token": "mgr"},
        "providers": {"mgr": {"provider": "manager", "keep": "close", "tag": tag, "expires_in": 30,
            "values": "{\"token\": \"zq9-not-a-real-secret-7f3a\"}"}}
    }));
    assert_eq!(recalled(&rig.vault, "s1", "token").as_deref(), Some(VALUE));
    assert_eq!(
        rig.vault.rows()[0].expires_in,
        Some(30),
        "kept for 30 s though the entry says until Canoa closes"
    );
    rig.seconds(29);
    assert_eq!(recalled(&rig.vault, "s1", "token").as_deref(), Some(VALUE));
    assert_eq!(calls(tag), 1);
    rig.seconds(2);
    assert!(
        rig.vault.rows().is_empty(),
        "the value ended at the provider's word"
    );
    // Asked again, the provider is asked again.
    assert_eq!(recalled(&rig.vault, "s1", "token").as_deref(), Some(VALUE));
    assert_eq!(calls(tag), 2);
}

#[test]
fn the_shorter_of_the_persons_choice_and_the_providers_end_wins_either_way() {
    let rig = rig(json!({
        "names": {"token": "mgr"},
        "providers": {"mgr": {"provider": "manager", "keep": "ttl:10", "tag": "either", "expires_in": 3600,
            "values": "{\"token\": \"zq9-not-a-real-secret-7f3a\"}"}}
    }));
    assert_eq!(recalled(&rig.vault, "s1", "token").as_deref(), Some(VALUE));
    assert_eq!(rig.vault.rows()[0].expires_in, Some(10));
}

#[test]
fn a_value_a_provider_says_is_already_over_is_not_given() {
    let rig = rig(json!({
        "names": {"token": "mgr"},
        "providers": {"mgr": {"provider": "manager", "tag": "over", "expires_in": 0,
            "values": "{\"token\": \"zq9-not-a-real-secret-7f3a\"}"}}
    }));
    assert_eq!(recalled(&rig.vault, "s1", "token"), None);
    assert!(rig.vault.rows().is_empty());
}

#[test]
fn remember_keeps_a_secret_in_the_provider_that_keeps_and_it_is_read_from_there_afterwards() {
    let tag = "keeps";
    let rig = rig(json!({
        "store": "kept",
        "providers": {"kept": {"provider": "keeper", "tag": tag, "keep": "once"}}
    }));
    let keeps: Vec<String> = rig.vault.keeps().iter().map(|keep| keep.id()).collect();
    assert_eq!(keeps.first().map(String::as_str), Some("session"));
    assert_eq!(keeps.last().map(String::as_str), Some("store"));
    rig.vault
        .remember(Some("s1"), "deploy", VALUE.as_bytes(), "store")
        .unwrap();
    // It is the provider's now; nothing of it is held in memory, so a later request reads it from the provider.
    assert_eq!(calls(tag), 0);
    assert_eq!(recalled(&rig.vault, "s9", "deploy").as_deref(), Some(VALUE));
    assert_eq!(calls(tag), 1);
    let rows = rig.vault.rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (
            rows[0].provider.as_str(),
            rows[0].name.as_str(),
            &rows[0].kept
        ),
        ("kept", "deploy", &Kept::Stored)
    );
    assert_eq!(
        rig.vault
            .reveal("kept", "deploy")
            .as_deref()
            .map(String::as_str),
        Some(VALUE)
    );
    assert!(rig.vault.forget("kept", "deploy").unwrap());
    assert_eq!(recalled(&rig.vault, "s9", "deploy"), None);
    assert!(rig.vault.rows().is_empty());
}

#[test]
fn with_no_provider_that_keeps_the_form_does_not_offer_to_remember() {
    let rig = rig(json!({"store": "none"}));
    assert!(rig.vault.keeps().iter().all(|keep| keep.id() != "store"));
    assert!(rig.vault.remember(Some("s1"), "x", b"v", "store").is_err());
}

#[test]
fn a_way_to_keep_that_is_none_a_name_that_is_none_and_a_value_that_is_no_text_are_refused() {
    let rig = rig(json!({}));
    assert!(
        rig.vault
            .remember(Some("s1"), "x", b"v", "forever")
            .is_err()
    );
    assert!(rig.vault.remember(Some("s1"), "", b"v", "close").is_err());
    assert!(
        rig.vault
            .remember(Some("s1"), "x", &[0xff, 0xfe], "close")
            .is_err()
    );
    assert!(rig.vault.rows().is_empty());
}

#[test]
fn no_value_is_in_a_debug_print_a_row_an_error_or_the_listing_of_providers() {
    let rig = rig(json!({
        "names": {"api_key": "mgr"},
        "providers": {"mgr": {"provider": "manager", "tag": "quiet", "keep": "close",
            "values": "{\"api_key\": \"zq9-not-a-real-secret-7f3a\"}"}}
    }));
    rig.vault
        .remember(Some("s1"), "typed", VALUE.as_bytes(), "close")
        .unwrap();
    assert_eq!(
        recalled(&rig.vault, "s1", "api_key").as_deref(),
        Some(VALUE)
    );
    let mut seen = format!("{:?}", rig.vault);
    for row in rig.vault.rows() {
        seen.push_str(&format!("{row:?}{}", row.json()));
    }
    seen.push_str(&format!("{:?}", rig.vault.providers()));
    for error in [
        rig.vault
            .remember(Some("s1"), "x", VALUE.as_bytes(), "forever")
            .unwrap_err(),
        rig.vault
            .set("ghost", "x", VALUE.as_bytes(), "close")
            .unwrap_err(),
        rig.vault
            .set("form", &format!("bad\n{VALUE}"), VALUE.as_bytes(), "close")
            .unwrap_err(),
    ] {
        seen.push_str(&error);
    }
    assert!(!seen.contains(VALUE), "a value reached: {seen}");
    assert!(!seen.contains("zq9"), "a piece of a value reached: {seen}");
}

#[test]
fn setting_a_secret_by_hand_is_kept_as_chosen_and_shown_under_the_provider_it_was_set_in() {
    let rig = rig(json!({}));
    rig.vault
        .set("form", "hand", VALUE.as_bytes(), "8h")
        .unwrap();
    assert_eq!(recalled(&rig.vault, "s1", "hand").as_deref(), Some(VALUE));
    assert_eq!(rig.vault.rows()[0].expires_in, Some(8 * 3600));
    assert!(rig.vault.forget("form", "hand").unwrap());
    assert_eq!(recalled(&rig.vault, "s1", "hand"), None);
}

#[test]
fn a_change_of_canoa_cfg_applies_at_the_next_request() {
    let clock = Arc::new(AtomicU64::new(0));
    let section = Arc::new(Mutex::new(json!({})));
    let read = Arc::clone(&section);
    let vault = Vault::with_clock(
        catalog(),
        Arc::new(move || Ok(Some(lock(&read).clone()))),
        Arc::new(move || clock.load(Ordering::SeqCst)),
    );
    assert_eq!(recalled(&vault, "s1", "late"), None);
    *lock(&section) = json!({
        "names": {"late": "mgr"},
        "providers": {"mgr": {"provider": "manager", "tag": "late",
            "values": "{\"late\": \"zq9-not-a-real-secret-7f3a\"}"}}
    });
    assert_eq!(recalled(&vault, "s1", "late").as_deref(), Some(VALUE));
    // A section that cannot be read is not used, and the defaults answer.
    *lock(&section) = json!({"surprise": 1});
    assert_eq!(recalled(&vault, "s2", "other"), None);
}

#[test]
fn an_accounts_credentials_are_a_named_secret_the_table_lists_and_removing_the_account_forgets() {
    use crate::accounts::{Accounts, Record};
    use crate::secrets::handle::HandleStore;

    let tag = "accounts";
    let rig = rig(json!({
        "providers": {"kept": {"provider": "keeper", "tag": tag}},
        "accounts": "kept"
    }));
    let dir = std::env::temp_dir().join(format!("canoa-accounts-handle-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let accounts = Accounts::with_store(
        dir.clone(),
        Box::new(HandleStore::new(Arc::clone(&rig.vault), "kept")),
    )
    .expect("opens");
    let mut record = Record::default();
    record.revision = 7;
    record
        .credentials
        .insert("API_KEY".to_owned(), VALUE.to_owned());
    accounts.save("acc-1", &record).expect("kept");
    // The key is a secret of the chosen provider, under a name, which the Secrets table lists without its value.
    let rows = rig.vault.rows();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(
        (rows[0].provider.as_str(), rows[0].name.as_str()),
        ("kept", "accounts/acc-1")
    );
    assert!(!format!("{rows:?}").contains(VALUE));
    // The account's session reads it back from the provider.
    let read = accounts.load("acc-1").expect("reads").expect("held");
    assert_eq!(read.revision, 7);
    assert_eq!(
        read.credentials.get("API_KEY").map(String::as_str),
        Some(VALUE)
    );
    // The person can ask to see it, as any secret of the table.
    let shown = rig.vault.reveal("kept", "accounts/acc-1").expect("shown");
    assert!(shown.contains(VALUE));
    // Removing the account forgets it.
    accounts.forget_credentials("acc-1").expect("forgotten");
    assert!(rig.vault.rows().is_empty());
    assert!(accounts.load("acc-1").expect("reads").is_none());
    let _ = std::fs::remove_dir_all(&dir);
}
