//! The secret providers the host carries, registered as providers of the `secret_provider` kind the way an extension's are: the
//! person's form, which holds nothing and is asked in the app, and the operating system's credential store, which keeps what the
//! person tells it to.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "locks are std's"
)]

use std::collections::BTreeSet;
use std::sync::{Mutex, PoisonError};

use canoa::secret_provider::{
    Held, Keeping, Named, Names, Other, ProviderInfo, SecretProvider, name_problem,
};
use canoa::status::{bad_value, internal, unavailable, unsupported};
use coqui_acp_api::ProcessSpawner;
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{FromValue, Provider, Schema, ToValue};
use guatiao_intake::Form;

use crate::accounts::store::outside::Outside;
use crate::accounts::store::{FileStore, KeyringStore, SecretStore};

/// The id of the form provider, and of the entry that is there with no configuration.
pub(crate) const FORM: &str = "form";
/// The id of the system store provider, and of the entry that is there with no configuration.
pub(crate) const SYSTEM: &str = "system";

/// The id of the sealed-files provider, and of the entry that is there with no configuration.
pub(crate) const FILE: &str = "file";

/// The person's form: it holds nothing, so a secret it is to answer is typed in the app, which the host asks itself.
#[derive(Default, Provider)]
#[provider(SecretProvider, id = "form", crate = ::coqui_acp_api::guatiao)]
pub(crate) struct FormProvider;

impl SecretProvider for FormProvider {
    fn describe(&self) -> Result<ProviderInfo, ProviderError> {
        let mut info = ProviderInfo::new("Form");
        info.description = Some(
            "Asks you in the app, and keeps the value as long as you choose: once, for the session, until Canoa closes or for a time."
                .to_owned(),
        );
        info.needs_person = true;
        Ok(info)
    }

    fn names(&self, _: Object<dyn ProcessSpawner>) -> Result<Names, ProviderError> {
        Err(unsupported("a form holds nothing to list"))
    }

    fn get(&self, request: Named, _: Object<dyn ProcessSpawner>) -> Result<Held, ProviderError> {
        check(&request.name)?;
        Ok(Held::none())
    }

    fn put(&self, _: Keeping, _: Object<dyn ProcessSpawner>) -> Result<(), ProviderError> {
        Err(unsupported(
            "a form keeps nothing: the host keeps it for as long as you chose",
        ))
    }

    fn forget(&self, _: Named, _: Object<dyn ProcessSpawner>) -> Result<(), ProviderError> {
        Err(unsupported("a form keeps nothing"))
    }
}

fn check(name: &str) -> Result<(), ProviderError> {
    name_problem(name).map_or(Ok(()), |why| Err(bad_value(why)))
}

/// What the system provider is configured with.
#[derive(ToValue, FromValue, Schema, Form, Debug, Clone, Default)]
#[form(section(id = "store", label = "System store"))]
#[non_exhaustive]
pub(crate) struct SystemSettings {
    /// The name the entries are kept under in the operating system's store, so two hosts of one user never share a secret.
    #[schema(advanced, section = "store", title = "Service")]
    pub(crate) service: Option<String>,
}

/// The operating system's credential store (Windows' Credential Manager, the Secret Service, the Keychain): a secret of Canoa's is an
/// entry of one service, and an index entry names them. It lists the person's other credentials in the store as well, each by the name
/// the store gives it, and reads one when asked; it writes only its own, and deletes another's only when asked to forget it.
#[derive(Provider)]
#[provider(SecretProvider, id = "system", config = SystemSettings, form = SystemSettings, crate = ::coqui_acp_api::guatiao)]
pub(crate) struct SystemSecrets {
    store: Box<dyn SecretStore>,
    /// The store's entries that Canoa did not write; none for a provider over a store that is only its own.
    outside: Option<Box<dyn Outside>>,
    /// One change of the index at a time.
    index: Mutex<()>,
    /// Whether the operating system's own store is behind it, so that a host with none says so in one clear sentence.
    system: bool,
}

/// What the index entry is kept as; a secret's own is `s:` and its name.
const INDEX: &str = "i:names";

impl From<SystemSettings> for SystemSecrets {
    fn from(settings: SystemSettings) -> Self {
        let service = settings
            .service
            .filter(|service| !service.trim().is_empty())
            .unwrap_or_else(|| format!("{}.secrets", canoa::brand::ID));
        Self {
            system: true,
            ..Self::over_with(
                Box::new(KeyringStore::new(service.clone())),
                Box::new(KeyringStore::new(service)),
            )
        }
    }
}

fn failed(why: &crate::accounts::store::StoreError) -> ProviderError {
    // The store's words name the failure and never a value.
    unavailable(why.to_string())
}

impl SystemSecrets {
    /// The provider over `store`.
    pub(crate) fn over(store: Box<dyn SecretStore>) -> Self {
        Self {
            store,
            outside: None,
            index: Mutex::new(()),
            system: false,
        }
    }

    /// Why this provider cannot answer on this host, when the operating system's store is behind it and does not.
    fn ready(&self) -> Result<(), ProviderError> {
        if !self.system {
            return Ok(());
        }
        crate::accounts::store::os_store().map_err(|why| {
            unavailable(format!(
                "this host has no operating system credential store to use ({why}); keep it in another entry, such as `file`"
            ))
        })
    }

    /// The provider over `store`, which also lists and reads the entries `outside` shows.
    pub(crate) fn over_with(store: Box<dyn SecretStore>, outside: Box<dyn Outside>) -> Self {
        Self {
            outside: Some(outside),
            ..Self::over(store)
        }
    }

    fn names_held(&self) -> Result<BTreeSet<String>, ProviderError> {
        let Some(mut bytes) = self.store.get(INDEX).map_err(|why| failed(&why))? else {
            return Ok(BTreeSet::new());
        };
        let names = serde_json::from_slice(&bytes)
            .map_err(|_| internal("the store's list of names is damaged"));
        zeroize::Zeroize::zeroize(&mut bytes);
        names
    }

    fn write_names(&self, names: &BTreeSet<String>) -> Result<(), ProviderError> {
        let bytes =
            serde_json::to_vec(names).map_err(|_| internal("the list of names cannot be kept"))?;
        self.store.put(INDEX, &bytes).map_err(|why| failed(&why))
    }
}

/// The text another program wrote as a credential's value: UTF-8, or UTF-16 as Windows' own tools write a password. Anything else is
/// not shown as text.
fn text_of(mut bytes: Vec<u8>) -> Result<String, ProviderError> {
    let utf8 = core::str::from_utf8(&bytes)
        .ok()
        .filter(|text| !text.contains('\0'))
        .map(str::to_owned);
    let text = utf8.or_else(|| {
        if bytes.is_empty() || !bytes.len().is_multiple_of(2) {
            return None;
        }
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .collect();
        String::from_utf16(&units)
            .ok()
            .filter(|text| !text.contains('\0'))
    });
    let empty = bytes.is_empty();
    zeroize::Zeroize::zeroize(&mut bytes);
    match text {
        Some(text) => Ok(text),
        None if empty => Ok(String::new()),
        None => Err(bad_value(
            "this credential's value is not text (it is neither UTF-8 nor UTF-16), so it cannot be shown",
        )),
    }
}

impl SecretProvider for SystemSecrets {
    fn describe(&self) -> Result<ProviderInfo, ProviderError> {
        let mut info = ProviderInfo::new("System store");
        info.description = Some(
            "The operating system's credential store: Windows' Credential Manager, the Secret Service or the Keychain."
                .to_owned(),
        );
        info.can_store = true;
        info.can_list = true;
        Ok(info)
    }

    fn names(&self, _: Object<dyn ProcessSpawner>) -> Result<Names, ProviderError> {
        self.ready()?;
        let _one = self.index.lock().unwrap_or_else(PoisonError::into_inner);
        let own = self.names_held()?;
        let mut others = Vec::new();
        if let Some(outside) = &self.outside {
            // The store failing to list what is not Canoa's never hides what is.
            match outside.list() {
                Ok(found) => others.extend(
                    found
                        .into_iter()
                        .filter(|each| !own.contains(&each.name))
                        .map(|each| Other::new(each.name, each.summary)),
                ),
                Err(why) => tracing::info!("the store's other entries are not listed: {why}"),
            }
        }
        Ok(Names::new(own.into_iter().collect()).with_others(others))
    }

    fn get(&self, request: Named, _: Object<dyn ProcessSpawner>) -> Result<Held, ProviderError> {
        self.ready()?;
        check(&request.name)?;
        let key = format!("s:{}", request.name);
        let Some(mut bytes) = self.store.get(&key).map_err(|why| failed(&why))? else {
            // Not one of Canoa's: an entry of the person's own for another program, by the name the store gives it.
            let Some(outside) = &self.outside else {
                return Ok(Held::none());
            };
            return Ok(
                match outside.read(&request.name).map_err(|why| failed(&why))? {
                    Some(bytes) => Held::of(text_of(bytes)?),
                    None => Held::none(),
                },
            );
        };
        let held = String::from_utf8(bytes.clone()).map(Held::of);
        zeroize::Zeroize::zeroize(&mut bytes);
        held.map_err(|_| internal("the stored value is not text"))
    }

    fn put(&self, request: Keeping, _: Object<dyn ProcessSpawner>) -> Result<(), ProviderError> {
        self.ready()?;
        check(&request.name)?;
        let value = request.value.expose();
        let _one = self.index.lock().unwrap_or_else(PoisonError::into_inner);
        let mut names = self.names_held()?;
        // The value goes first: an index that names a secret the store lacks would be worse than the reverse.
        self.store
            .put(&format!("s:{}", request.name), value.as_bytes())
            .map_err(|why| failed(&why))?;
        if names.insert(request.name) {
            self.write_names(&names)?;
        }
        Ok(())
    }

    fn forget(&self, request: Named, _: Object<dyn ProcessSpawner>) -> Result<(), ProviderError> {
        self.ready()?;
        check(&request.name)?;
        let _one = self.index.lock().unwrap_or_else(PoisonError::into_inner);
        let mut names = self.names_held()?;
        self.store
            .remove(&format!("s:{}", request.name))
            .map_err(|why| failed(&why))?;
        if names.remove(&request.name) {
            self.write_names(&names)?;
        } else if let Some(outside) = &self.outside {
            // Not one of Canoa's: forgetting it deletes the person's credential for another program, which the app asks them about.
            outside.remove(&request.name).map_err(|why| failed(&why))?;
        }
        Ok(())
    }
}

/// What the sealed-files provider is configured with.
#[derive(ToValue, FromValue, Schema, Form, Debug, Clone, Default)]
#[form(section(id = "files", label = "Sealed files"))]
#[non_exhaustive]
pub(crate) struct FileSettings {
    /// The folder the files are kept in, with the key that seals them. The host's own folder for secrets when this is left out.
    #[schema(section = "files", title = "Folder")]
    pub(crate) directory: Option<String>,
}

/// A file for each secret in a folder, sealed to a key beside it: for a computer with no credential store (a service with no
/// session bus). It protects against a copied file and not against anyone who can read the folder, which is why the operating
/// system's store is the default. It cannot say what it holds: a file is named by a digest of the secret's name.
#[derive(Provider)]
#[provider(SecretProvider, id = "file", config = FileSettings, form = FileSettings, crate = ::coqui_acp_api::guatiao)]
pub(crate) struct FileSecrets {
    store: Option<FileStore>,
}

impl From<FileSettings> for FileSecrets {
    fn from(settings: FileSettings) -> Self {
        Self {
            store: settings
                .directory
                .filter(|directory| !directory.trim().is_empty())
                .map(FileStore::new),
        }
    }
}

impl FileSecrets {
    fn store(&self) -> Result<&FileStore, ProviderError> {
        self.store.as_ref().ok_or_else(|| {
            unavailable("this host has no folder to keep sealed files in: name one as `directory`")
        })
    }
}

impl SecretProvider for FileSecrets {
    fn describe(&self) -> Result<ProviderInfo, ProviderError> {
        let mut info = ProviderInfo::new("Sealed files");
        info.description = Some(
            "A file for each secret in a folder of this computer, sealed to a key beside it: for a computer with no credential store."
                .to_owned(),
        );
        info.can_store = true;
        Ok(info)
    }

    fn names(&self, _: Object<dyn ProcessSpawner>) -> Result<Names, ProviderError> {
        Err(unsupported(
            "a sealed file is named by a digest, so it cannot say what it holds",
        ))
    }

    fn get(&self, request: Named, _: Object<dyn ProcessSpawner>) -> Result<Held, ProviderError> {
        check(&request.name)?;
        let Some(mut bytes) = self
            .store()?
            .get(&request.name)
            .map_err(|why| failed(&why))?
        else {
            return Ok(Held::none());
        };
        let held = String::from_utf8(bytes.clone()).map(Held::of);
        zeroize::Zeroize::zeroize(&mut bytes);
        held.map_err(|_| internal("the stored value is not text"))
    }

    fn put(&self, request: Keeping, _: Object<dyn ProcessSpawner>) -> Result<(), ProviderError> {
        check(&request.name)?;
        let value = request.value.expose();
        self.store()?
            .put(&request.name, value.as_bytes())
            .map_err(|why| failed(&why))
    }

    fn forget(&self, request: Named, _: Object<dyn ProcessSpawner>) -> Result<(), ProviderError> {
        check(&request.name)?;
        self.store()?
            .remove(&request.name)
            .map_err(|why| failed(&why))
    }
}

coqui_acp_api::guatiao::local_providers!(
    id = "canoa-secret-providers",
    version = env!("CARGO_PKG_VERSION"),
    providers = [FormProvider, SystemSecrets, FileSecrets]
);

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::accounts::store::StoreError;
    use crate::accounts::store::outside::Foreign;
    use crate::secrets::spawner::spawner;

    /// A store that keeps bytes in memory: what the operating system's store is in a test.
    #[derive(Debug, Default)]
    pub(crate) struct Memory(Mutex<HashMap<String, Vec<u8>>>);

    impl SecretStore for Memory {
        fn describe(&self) -> &'static str {
            "memory"
        }
        fn get(&self, id: &str) -> Result<Option<Vec<u8>>, StoreError> {
            Ok(self.0.lock().unwrap().get(id).cloned())
        }
        fn put(&self, id: &str, bytes: &[u8]) -> Result<(), StoreError> {
            self.0.lock().unwrap().insert(id.to_owned(), bytes.to_vec());
            Ok(())
        }
        fn remove(&self, id: &str) -> Result<(), StoreError> {
            self.0.lock().unwrap().remove(id);
            Ok(())
        }
    }

    fn system() -> SystemSecrets {
        SystemSecrets::over(Box::new(Memory::default()))
    }

    /// What the person's other programs keep in the store, in memory: three entries with the names a store gives them.
    #[derive(Debug)]
    struct Others(Mutex<Vec<Entry>>);

    /// A name, what the store says of it, and its bytes.
    type Entry = (&'static str, Option<&'static str>, Vec<u8>);

    impl Others {
        fn new() -> Self {
            Self(Mutex::new(vec![
                (
                    "git:https://example.test",
                    Some("alice"),
                    b"hunter2".to_vec(),
                ),
                // Windows' own tools write a password as UTF-16.
                (
                    "Wifi",
                    None,
                    "p\u{e4}ss"
                        .encode_utf16()
                        .flat_map(u16::to_le_bytes)
                        .collect(),
                ),
                ("Binary", Some("device"), vec![0xff, 0x00, 0xfe, 0x80, 0x01]),
            ]))
        }
    }

    impl Outside for Others {
        fn list(&self) -> Result<Vec<Foreign>, StoreError> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .iter()
                .map(|(name, summary, _)| Foreign {
                    name: (*name).to_owned(),
                    summary: summary.map(str::to_owned),
                })
                .collect())
        }
        fn read(&self, name: &str) -> Result<Option<Vec<u8>>, StoreError> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .iter()
                .find(|(each, _, _)| *each == name)
                .map(|(_, _, bytes)| bytes.clone()))
        }
        fn remove(&self, name: &str) -> Result<bool, StoreError> {
            let mut all = self.0.lock().unwrap();
            let before = all.len();
            all.retain(|(each, _, _)| *each != name);
            Ok(all.len() != before)
        }
    }

    fn with_others() -> SystemSecrets {
        let provider =
            SystemSecrets::over_with(Box::new(Memory::default()), Box::new(Others::new()));
        provider
            .put(Keeping::new("deploy", "mine-1"), spawner())
            .unwrap();
        provider
            .put(Keeping::new("token", "mine-2"), spawner())
            .unwrap();
        provider
    }

    #[test]
    fn the_system_provider_lists_the_stores_other_entries_apart_from_its_own_and_reads_one_when_asked()
     {
        let provider = with_others();
        let names = provider.names(spawner()).unwrap();
        assert_eq!(names.names, ["deploy", "token"], "what Canoa wrote");
        let others: Vec<(&str, Option<&str>)> = names
            .others
            .iter()
            .map(|other| (other.name.as_str(), other.summary.as_deref()))
            .collect();
        assert_eq!(
            others,
            [
                ("git:https://example.test", Some("alice")),
                ("Wifi", None),
                ("Binary", Some("device")),
            ]
        );
        let read = |name: &str| provider.get(Named::new(name), spawner());
        assert_eq!(
            read("git:https://example.test")
                .unwrap()
                .value
                .unwrap()
                .expose(),
            "hunter2"
        );
        assert_eq!(read("Wifi").unwrap().value.unwrap().expose(), "p\u{e4}ss");
        assert_eq!(read("deploy").unwrap().value.unwrap().expose(), "mine-1");
        assert!(read("nothing").unwrap().value.is_none());
    }

    #[test]
    fn a_value_that_is_no_text_is_said_so_and_is_never_shown_as_bytes() {
        let provider = with_others();
        let why = provider
            .get(Named::new("Binary"), spawner())
            .expect_err("not text");
        assert!(why.message().contains("not text"), "{}", why.message());
        assert!(!why.message().contains("255"));
        assert_eq!(text_of(b"plain".to_vec()).unwrap(), "plain");
        assert_eq!(text_of(Vec::new()).unwrap(), "");
        assert_eq!(
            text_of("ab".encode_utf16().flat_map(u16::to_le_bytes).collect()).unwrap(),
            "ab"
        );
        assert!(text_of(vec![0xd8, 0x00, 0x00]).is_err());
    }

    #[test]
    fn a_secret_set_by_the_person_is_canoas_own_even_under_the_name_of_another_programs_entry() {
        let provider = with_others();
        provider
            .put(Keeping::new("Wifi", "mine-3"), spawner())
            .unwrap();
        let names = provider.names(spawner()).unwrap();
        assert_eq!(names.names, ["Wifi", "deploy", "token"]);
        assert!(
            names.others.iter().all(|other| other.name != "Wifi"),
            "listed once, as Canoa's"
        );
        let read = provider.get(Named::new("Wifi"), spawner()).unwrap();
        assert_eq!(read.value.unwrap().expose(), "mine-3");
        // Forgetting it takes Canoa's and leaves the other program's, which is then what the name reads.
        provider.forget(Named::new("Wifi"), spawner()).unwrap();
        let read = provider.get(Named::new("Wifi"), spawner()).unwrap();
        assert_eq!(read.value.unwrap().expose(), "p\u{e4}ss");
    }

    #[test]
    fn forgetting_another_programs_entry_deletes_it_and_a_failing_listing_hides_nothing_of_canoas()
    {
        let provider = with_others();
        provider.forget(Named::new("Wifi"), spawner()).unwrap();
        let names = provider.names(spawner()).unwrap();
        assert_eq!(names.others.len(), 2);
        assert_eq!(names.names, ["deploy", "token"]);

        #[derive(Debug)]
        struct Failing;
        impl Outside for Failing {
            fn list(&self) -> Result<Vec<Foreign>, StoreError> {
                Err(StoreError("the store is locked".into()))
            }
            fn read(&self, _: &str) -> Result<Option<Vec<u8>>, StoreError> {
                Err(StoreError("the store is locked".into()))
            }
            fn remove(&self, _: &str) -> Result<bool, StoreError> {
                Err(StoreError("the store is locked".into()))
            }
        }
        let locked = SystemSecrets::over_with(Box::new(Memory::default()), Box::new(Failing));
        locked.put(Keeping::new("a", "1"), spawner()).unwrap();
        let names = locked.names(spawner()).unwrap();
        assert_eq!(names.names, ["a"]);
        assert!(names.others.is_empty());
        assert!(
            locked.get(Named::new("b"), spawner()).is_err(),
            "asked for a name that is not Canoa's, the failure is said"
        );
    }

    #[test]
    fn the_system_provider_keeps_reads_lists_and_forgets_a_secret() {
        let provider = system();
        assert!(
            provider
                .get(Named::new("token"), spawner())
                .unwrap()
                .value
                .is_none()
        );
        provider
            .put(Keeping::new("token", "tr0ub4dor-value"), spawner())
            .unwrap();
        provider.put(Keeping::new("other", "x"), spawner()).unwrap();
        let held = provider.get(Named::new("token"), spawner()).unwrap();
        assert_eq!(held.value.unwrap().expose(), "tr0ub4dor-value");
        assert_eq!(held.expires_in, None);
        assert_eq!(provider.names(spawner()).unwrap().names, ["other", "token"]);
        provider.forget(Named::new("token"), spawner()).unwrap();
        assert!(
            provider
                .get(Named::new("token"), spawner())
                .unwrap()
                .value
                .is_none()
        );
        assert_eq!(provider.names(spawner()).unwrap().names, ["other"]);
        // Forgetting what is not there is no error.
        provider.forget(Named::new("token"), spawner()).unwrap();
    }

    #[test]
    fn a_replaced_secret_is_listed_once_and_a_secret_called_like_the_index_is_only_a_secret() {
        let provider = system();
        provider.put(Keeping::new("a", "1"), spawner()).unwrap();
        provider.put(Keeping::new("a", "2"), spawner()).unwrap();
        provider
            .put(Keeping::new("i:names", "3"), spawner())
            .unwrap();
        assert_eq!(provider.names(spawner()).unwrap().names, ["a", "i:names"]);
        assert_eq!(
            provider
                .get(Named::new("a"), spawner())
                .unwrap()
                .value
                .unwrap()
                .expose(),
            "2"
        );
    }

    #[test]
    fn a_name_that_is_no_name_is_refused_before_the_store_is_touched() {
        let provider = system();
        assert!(provider.put(Keeping::new("", "v"), spawner()).is_err());
        assert!(provider.get(Named::new("a\nb"), spawner()).is_err());
        assert!(provider.names(spawner()).unwrap().names.is_empty());
    }

    #[test]
    fn every_name_is_listed_and_no_value_or_name_is_cut_by_a_length_of_the_hosts_own() {
        let provider = system();
        for number in 0..1200 {
            provider
                .put(Keeping::new(format!("n{number:04}"), "v"), spawner())
                .unwrap();
        }
        let long_name = "k".repeat(1000);
        let long_value = "v".repeat(200 * 1024);
        provider
            .put(
                Keeping::new(long_name.clone(), long_value.clone()),
                spawner(),
            )
            .unwrap();
        let names = provider.names(spawner()).unwrap().names;
        assert_eq!(names.len(), 1201, "the listing is not cut");
        let held = provider.get(Named::new(long_name), spawner()).unwrap();
        assert_eq!(
            held.value.as_ref().map(|v| v.expose().len()),
            Some(long_value.len())
        );
    }

    #[test]
    fn the_form_holds_nothing_keeps_nothing_and_needs_a_person() {
        let form = FormProvider;
        let info = form.describe().unwrap();
        assert!(info.needs_person && !info.can_store && !info.can_list);
        assert!(
            form.get(Named::new("x"), spawner())
                .unwrap()
                .value
                .is_none()
        );
        assert!(form.put(Keeping::new("x", "v"), spawner()).is_err());
        assert!(form.names(spawner()).is_err());
    }

    /// A host with no operating system store says so in one sentence, and not in the platform's words about a bus.
    #[test]
    fn the_provider_built_from_its_settings_says_a_missing_store_in_one_sentence() {
        let _no_store = crate::accounts::store::naming_os_store(Err(
            "the operating system's credential store is not available (no session bus)".to_owned(),
        ));
        let provider = SystemSecrets::from(SystemSettings {
            service: Some("in-memory.absent".to_owned()),
        });
        let error = provider
            .put(Keeping::new("probe", "a-made-up-value"), spawner())
            .expect_err("nothing to keep it in");
        let said = error.message().to_string();
        assert!(
            said.contains("has no operating system credential store to use"),
            "{said}"
        );
        assert!(said.contains("another entry, such as `file`"), "{said}");
        assert!(provider.get(Named::new("probe"), spawner()).is_err());
        assert!(provider.names(spawner()).is_err());
    }

    /// The provider the host builds from its settings keeps what it is given under the service they name (in memory, as every test but
    /// the ones about the operating system's store).
    #[test]
    fn the_provider_built_from_its_settings_reads_back_what_it_was_given() {
        let provider = SystemSecrets::from(SystemSettings {
            service: Some("in-memory.settings".to_owned()),
        });
        provider
            .put(Keeping::new("probe", "a-made-up-value"), spawner())
            .unwrap();
        let held = provider.get(Named::new("probe"), spawner()).unwrap();
        assert_eq!(held.value.unwrap().expose(), "a-made-up-value");
        provider.forget(Named::new("probe"), spawner()).unwrap();
    }

    /// The operating system's real store, under a service of this run's own, emptied when the test ends however it ends.
    #[test]
    #[cfg_attr(
        target_os = "linux",
        ignore = "needs the operating system's credential store: a Secret Service session on Linux"
    )]
    fn the_operating_systems_store_reads_back_what_it_was_given() {
        let real = crate::accounts::store::real_store::RealStore::new();
        let provider = SystemSecrets::over(Box::new(real.store()));
        provider
            .put(Keeping::new("probe", "a-made-up-value"), spawner())
            .expect("the operating system's store holds a value");
        let held = provider.get(Named::new("probe"), spawner()).unwrap();
        assert_eq!(held.value.unwrap().expose(), "a-made-up-value");
        assert_eq!(provider.names(spawner()).unwrap().names, ["probe"]);
        provider.forget(Named::new("probe"), spawner()).unwrap();
        assert!(
            provider
                .get(Named::new("probe"), spawner())
                .unwrap()
                .value
                .is_none()
        );
        assert!(provider.names(spawner()).unwrap().names.is_empty());
        // The index entry the test made is removed with the rest.
        provider.store.remove(INDEX).unwrap();
    }
}
