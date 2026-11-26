//! What the operating system's credential store holds that Canoa did not write: the person's own credentials for other programs
//! (Windows' Credential Manager, the Keychain, the Secret Service). It is listed through the store's own search, so no platform call is
//! made here.
//!
//! An entry is named as the store spells it: Windows' target name, and `service/user` where a store has no target. Canoa's own entries
//! (every service this program makes starts with [`OWN`]) are left out, since the vault lists those by name already and an account's
//! chunks are records, not secrets.

use std::collections::HashMap;

use keyring_core::{Entry, Error as KeyError};

use super::{KeyringStore, StoreError, failed, one_at_a_time};

/// What every service Canoa keeps entries under starts with, whichever home or test made it.
const OWN: &str = canoa::brand::ID;

/// An entry of the store that another program wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Foreign {
    /// The name as the store spells it.
    pub(crate) name: String,
    /// What the store says of it besides its name (the user name of a credential).
    pub(crate) summary: Option<String>,
}

/// The entries of the person's store that Canoa did not write. A value read is bytes, since another program chose how to write it.
pub(crate) trait Outside: Send + Sync + core::fmt::Debug {
    /// Every such entry the account can see, by name.
    fn list(&self) -> Result<Vec<Foreign>, StoreError>;

    /// The bytes of the entry `name`; `None` when there is no such entry.
    fn read(&self, name: &str) -> Result<Option<Vec<u8>>, StoreError>;

    /// Deletes the entry `name`, and whether there was one.
    fn remove(&self, name: &str) -> Result<bool, StoreError>;
}

/// The name and summary a store gives `entry`, or none when it gives no name a secret could be asked for by.
fn describe(entry: &Entry, own_service: &str) -> Option<Foreign> {
    let attributes = entry.get_attributes().unwrap_or_default();
    let name = attributes.get("target_name").cloned().or_else(|| {
        entry
            .get_specifiers()
            .map(|(service, user)| format!("{service}/{user}"))
    })?;
    let own = entry
        .get_specifiers()
        .is_some_and(|(service, _)| service == own_service)
        || name.ends_with(&format!(".{own_service}"));
    if own || name.is_empty() || name.chars().any(char::is_control) || name.contains(OWN) {
        return None;
    }
    let summary = attributes
        .get("username")
        .filter(|user| !user.is_empty())
        .cloned();
    Some(Foreign { name, summary })
}

impl KeyringStore {
    /// Every entry of the store with its handle.
    fn all_entries(&self) -> Result<Vec<(Foreign, Entry)>, StoreError> {
        let (store, platform) = self.backend.open()?;
        // The process's own memory (what a test or `CANOA_CREDENTIAL_STORE=memory` asks for) holds nothing of the person's, except
        // that a unit test of this listing puts the entries it stands in for under `<service>.others`.
        let mut spec = HashMap::new();
        let others = format!("{}.others", self.service);
        if !platform {
            if !cfg!(test) {
                return Ok(Vec::new());
            }
            spec.insert("service", others.as_str());
        }
        let found = match store.search(&spec) {
            Ok(found) => found,
            // A store that cannot be searched shows nothing beyond what Canoa keeps.
            Err(KeyError::NotSupportedByStore(_)) => return Ok(Vec::new()),
            Err(error) => {
                return Err(failed(
                    "the operating system's credential store could not be listed",
                    error,
                ));
            }
        };
        Ok(found
            .into_iter()
            .filter(|entry| {
                platform
                    || entry
                        .get_specifiers()
                        .is_some_and(|(service, _)| service == others)
            })
            .filter_map(|entry| describe(&entry, &self.service).map(|each| (each, entry)))
            .collect())
    }
}

impl Outside for KeyringStore {
    fn list(&self) -> Result<Vec<Foreign>, StoreError> {
        let _turn = one_at_a_time();
        let mut listed: Vec<Foreign> = self
            .all_entries()?
            .into_iter()
            .map(|(each, _)| each)
            .collect();
        listed.sort_by(|a, b| a.name.cmp(&b.name));
        listed.dedup_by(|a, b| a.name == b.name);
        Ok(listed)
    }

    fn read(&self, name: &str) -> Result<Option<Vec<u8>>, StoreError> {
        let _turn = one_at_a_time();
        let Some((_, entry)) = self
            .all_entries()?
            .into_iter()
            .find(|(each, _)| each.name == name)
        else {
            return Ok(None);
        };
        match entry.get_secret() {
            Ok(bytes) => Ok(Some(bytes)),
            Err(KeyError::NoEntry) => Ok(None),
            Err(error) => Err(failed(
                "the credential store could not read the entry",
                error,
            )),
        }
    }

    fn remove(&self, name: &str) -> Result<bool, StoreError> {
        let _turn = one_at_a_time();
        let Some((_, entry)) = self
            .all_entries()?
            .into_iter()
            .find(|(each, _)| each.name == name)
        else {
            return Ok(false);
        };
        match entry.delete_credential() {
            Ok(()) => Ok(true),
            Err(KeyError::NoEntry) => Ok(false),
            Err(error) => Err(failed(
                "the credential store could not delete the entry",
                error,
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The in-memory store lists entries by `service/user`, and a store lists as another program's only those under its own service
    /// and `.others`, so one test's entries never show in another's.
    fn viewer(tag: &str) -> (KeyringStore, KeyringStore) {
        let service = format!("viewer-{}-{tag}", std::process::id());
        let theirs = KeyringStore::new(format!("{service}.others"));
        (KeyringStore::new(service), theirs)
    }

    #[test]
    fn what_another_program_wrote_is_listed_by_name_and_read_and_deleted() {
        let (viewer, theirs) = viewer("listed");
        theirs.entry("alice").unwrap().set_secret(b"first").unwrap();
        theirs.entry("bob").unwrap().set_secret(b"second").unwrap();
        let names = |viewer: &KeyringStore| -> Vec<String> {
            viewer
                .list()
                .unwrap()
                .into_iter()
                .map(|each| each.name)
                .collect()
        };
        let prefix = format!("{}.others", viewer.service);
        assert_eq!(
            names(&viewer),
            [format!("{prefix}/alice"), format!("{prefix}/bob")]
        );
        assert_eq!(
            viewer.read(&format!("{prefix}/bob")).unwrap().as_deref(),
            Some(&b"second"[..])
        );
        assert_eq!(viewer.read(&format!("{prefix}/nobody")).unwrap(), None);
        assert!(viewer.remove(&format!("{prefix}/alice")).unwrap());
        // The in-memory store keeps a deleted entry's record, so the entry is asked for rather than listed again.
        assert_eq!(viewer.read(&format!("{prefix}/alice")).unwrap(), None);
        assert!(!viewer.remove(&format!("{prefix}/alice")).unwrap());
    }

    #[test]
    fn canoas_own_entries_and_a_name_nothing_could_ask_for_are_not_listed() {
        let (viewer, theirs) = viewer("own");
        theirs
            .entry("line\nbreak")
            .unwrap()
            .set_secret(b"x")
            .unwrap();
        theirs
            .entry(&format!("{OWN}.0123456789abcdef"))
            .unwrap()
            .set_secret(b"x")
            .unwrap();
        theirs.entry("fine").unwrap().set_secret(b"x").unwrap();
        // Canoa's own service, which is the viewer's: its entries are named by the vault, not listed as another program's.
        viewer.entry("s:mine").unwrap().set_secret(b"x").unwrap();
        let listed: Vec<String> = viewer
            .list()
            .unwrap()
            .into_iter()
            .map(|each| each.name)
            .collect();
        assert_eq!(listed, [format!("{}.others/fine", viewer.service)]);
        theirs
            .entry("line\nbreak")
            .unwrap()
            .delete_credential()
            .unwrap();
        theirs
            .entry(&format!("{OWN}.0123456789abcdef"))
            .unwrap()
            .delete_credential()
            .unwrap();
        theirs.entry("fine").unwrap().delete_credential().unwrap();
        viewer.entry("s:mine").unwrap().delete_credential().unwrap();
    }
}
