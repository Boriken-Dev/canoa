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
    Held, Keeping, Named, Names, ProviderInfo, SecretProvider, bad_value, internal, name_problem,
    unavailable, unsupported,
};
use coqui_acp_api::ProcessSpawner;
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{FromValue, Provider, Schema, ToValue};
use guatiao_intake::Form;

use crate::accounts::store::{FileStore, KeyringStore, SecretStore};

/// The id of the form provider, and of the entry that is there with no configuration.
pub(crate) const FORM: &str = "form";
/// The id of the system store provider, and of the entry that is there with no configuration.
pub(crate) const SYSTEM: &str = "system";

/// The id of the sealed-files provider, and of the entry that is there with no configuration.
pub(crate) const FILE: &str = "file";

/// The service the system provider keeps its entries under unless the host or the person names another.
pub(crate) const DEFAULT_SERVICE: &str = "dev.boriken.canoa.secrets";

/// [`DEFAULT_SERVICE`], except in this crate's own tests, which each process gives a service of its own so that no test reads or
/// writes what the person keeps in the operating system's store.
pub(crate) fn default_service() -> String {
    if cfg!(test) {
        format!("{DEFAULT_SERVICE}.test.{}", std::process::id())
    } else {
        DEFAULT_SERVICE.to_owned()
    }
}

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

/// The operating system's credential store (Windows' Credential Manager, the Secret Service, the Keychain): a secret is an entry
/// of one service, and an index entry names the secrets, since the store cannot be listed by what it holds.
#[derive(Provider)]
#[provider(SecretProvider, id = "system", config = SystemSettings, form = SystemSettings, crate = ::coqui_acp_api::guatiao)]
pub(crate) struct SystemSecrets {
    store: Box<dyn SecretStore>,
    /// One change of the index at a time.
    index: Mutex<()>,
}

/// What the index entry is kept as; a secret's own is `s:` and its name.
const INDEX: &str = "i:names";

impl From<SystemSettings> for SystemSecrets {
    fn from(settings: SystemSettings) -> Self {
        let service = settings
            .service
            .filter(|service| !service.trim().is_empty())
            .unwrap_or_else(default_service);
        Self::over(Box::new(KeyringStore::new(service)))
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
            index: Mutex::new(()),
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
        let _one = self.index.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(Names::new(self.names_held()?.into_iter().collect()))
    }

    fn get(&self, request: Named, _: Object<dyn ProcessSpawner>) -> Result<Held, ProviderError> {
        check(&request.name)?;
        let key = format!("s:{}", request.name);
        let Some(mut bytes) = self.store.get(&key).map_err(|why| failed(&why))? else {
            return Ok(Held::none());
        };
        let held = String::from_utf8(bytes.clone()).map(Held::of);
        zeroize::Zeroize::zeroize(&mut bytes);
        held.map_err(|_| internal("the stored value is not text"))
    }

    fn put(&self, request: Keeping, _: Object<dyn ProcessSpawner>) -> Result<(), ProviderError> {
        check(&request.name)?;
        let value = request.value.expose();
        if value.len() > canoa::secret_provider::MAX_VALUE {
            return Err(bad_value("the value is longer than a secret may be"));
        }
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
        check(&request.name)?;
        let _one = self.index.lock().unwrap_or_else(PoisonError::into_inner);
        let mut names = self.names_held()?;
        self.store
            .remove(&format!("s:{}", request.name))
            .map_err(|why| failed(&why))?;
        if names.remove(&request.name) {
            self.write_names(&names)?;
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
        if value.len() > canoa::secret_provider::MAX_VALUE {
            return Err(bad_value("the value is longer than a secret may be"));
        }
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

    /// A made-up service, so the operating system's real store is exercised with an entry nobody else has, and removed.
    #[test]
    fn the_operating_systems_store_reads_back_what_it_was_given_when_it_answers() {
        let service = format!("dev.boriken.canoa.test.secrets.{}", std::process::id());
        let provider = SystemSecrets::from(SystemSettings {
            service: Some(service),
        });
        let put = provider.put(Keeping::new("probe", "a-made-up-value"), spawner());
        let Ok(()) = put else {
            // No credential store here (a container with no session bus): the memory store above is the test.
            eprintln!("NOT RUN: the operating system's store does not answer here");
            return;
        };
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
