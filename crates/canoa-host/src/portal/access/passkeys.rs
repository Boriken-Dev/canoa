//! Passkeys for the portal: the provider's challenge, answer and account kinds, and whose a passkey is.
//!
//! A passkey's account is named by the operating system account's name, and carries `uid:<n>` among its groups, so that an
//! account deleted and made again under the same name does not inherit the passkeys of the one before. The provider keeps what a
//! ceremony needs in files its instances share, so the kinds the portal builds separately find each other's challenges.

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::net::IpAddr;

use coqui_access::guatiao::Status;
use coqui_access::guatiao::library::{ProviderError, Registry};
use coqui_access::{
    AccountInfo, AccountPut, Accounts as AccountKind, Attempt as Credential, ChallengeRequest,
    Challenger, Verdict, error, methods,
};

use super::super::signin::{PasskeyChallenge, PasskeyError, PasskeyInfo};
use super::Link;
use super::certs::UID_MARKER;

/// The longest answer from a browser the portal passes on, in bytes. The provider bounds it too; this is the form's own limit.
pub(super) const MAX_ANSWER: usize = 16 * 1024;
/// The longest label a person may give a passkey, in bytes.
pub(super) const MAX_LABEL: usize = 64;

/// The provider that keeps passkeys: it answers a challenge, checks the signed answer, and its account kind registers and
/// removes them.
pub(super) struct PasskeyStore {
    /// The authenticator and the directory that says whose a valid answer is.
    pub(super) link: Link,
    pub(super) challenger: Arc<dyn Challenger>,
    pub(super) accounts: Arc<dyn AccountKind>,
    /// The files the provider's configuration keeps its tables in, and the folder they are made in.
    pub(super) files: Vec<std::path::PathBuf>,
}

/// Whether the groups name `uid`: a passkey kept without the mark is nobody's.
pub(super) fn marked_for(groups: &[String], uid: u32) -> bool {
    groups
        .iter()
        .find_map(|group| group.strip_prefix(UID_MARKER))
        .is_some_and(|marker| marker == uid.to_string())
}

fn failed(error: &ProviderError) -> PasskeyError {
    PasskeyError::Failed(error.to_string())
}

fn info_of(account: &AccountInfo) -> Vec<PasskeyInfo> {
    account
        .credentials
        .iter()
        .flatten()
        .map(|credential| PasskeyInfo {
            id: credential.id.clone(),
            label: credential.label.clone(),
            made_unix: credential.made_unix,
            used_unix: credential.used_unix,
            synced: credential.synced,
            disabled: credential.disabled == Some(true) || account.disabled == Some(true),
        })
        .collect()
}

impl PasskeyStore {
    /// The account called `user` when it is this user id's.
    fn account_of(&self, user: &str, uid: u32) -> Result<Option<AccountInfo>, PasskeyError> {
        let all = self.accounts.accounts().map_err(|error| failed(&error))?;
        Ok(all
            .into_iter()
            .find(|account| account.name == user && marked_for(&account.groups, uid)))
    }

    pub(super) fn passkeys(&self, user: &str, uid: u32) -> Result<Vec<PasskeyInfo>, PasskeyError> {
        Ok(self
            .account_of(user, uid)?
            .map(|account| info_of(&account))
            .unwrap_or_default())
    }

    /// Every account with passkeys, for an administrator's listing: the name, and its passkeys.
    pub(super) fn all(&self) -> Result<Vec<(String, Vec<PasskeyInfo>)>, PasskeyError> {
        Ok(self
            .accounts
            .accounts()
            .map_err(|error| failed(&error))?
            .iter()
            .map(|account| (account.name.clone(), info_of(account)))
            .collect())
    }

    pub(super) fn challenge(
        &self,
        account: Option<(&str, u32)>,
        client: IpAddr,
        providers: &[String],
    ) -> Result<PasskeyChallenge, PasskeyError> {
        if !providers.is_empty() && !providers.contains(&self.link.id) {
            return Err(PasskeyError::Unsupported);
        }
        // An account asked for by name has passkeys that are its own, or none: nothing is asked of the provider for a name
        // whose passkeys belong to an account that was there before.
        if let Some((user, uid)) = account
            && !self
                .passkeys(user, uid)?
                .iter()
                .any(|passkey| !passkey.disabled)
        {
            return Err(PasskeyError::NoPasskey);
        }
        self.challenger
            .challenge(ChallengeRequest {
                method: Some(methods::PASSKEY.to_owned()),
                name: account.map(|(user, _)| user.to_owned()),
                peer: Some(client.to_string()),
            })
            .map(|made| PasskeyChallenge {
                challenge: made.challenge,
                options: made.options,
                expires_unix: made.expires_unix,
            })
            .map_err(|error| {
                if error::is_busy(&error) {
                    PasskeyError::Busy
                } else if error.status == Status::GUATIAO_ERR_NOT_FOUND {
                    PasskeyError::NoPasskey
                } else if error.status == Status::GUATIAO_ERR_UNSUPPORTED {
                    PasskeyError::Unsupported
                } else {
                    failed(&error)
                }
            })
    }

    /// What the provider says of an answer: the credential's id and the account id it names.
    pub(super) fn authenticate(
        &self,
        answer: &str,
        challenge: &str,
        client: IpAddr,
        providers: &[String],
    ) -> Result<Verdict, PasskeyError> {
        if answer.is_empty()
            || answer.len() > MAX_ANSWER
            || (!providers.is_empty() && !providers.contains(&self.link.id))
        {
            return Ok(Verdict::Unknown);
        }
        self.link
            .authenticator
            .authenticate(Credential {
                secret: Some(answer.to_owned()),
                challenge: Some(challenge.to_owned()),
                method: Some(methods::PASSKEY.to_owned()),
                peer: Some(client.to_string()),
                secure: Some(true),
                ..Credential::default()
            })
            .map_err(|error| failed(&error))
    }

    pub(super) fn begin(&self, user: &str, uid: u32) -> Result<String, PasskeyError> {
        // Whatever an account of this name held before this user id is not this account's: it goes before a passkey is added.
        let all = self.accounts.accounts().map_err(|error| failed(&error))?;
        if all
            .iter()
            .any(|account| account.name == user && !marked_for(&account.groups, uid))
        {
            self.accounts
                .remove(user.to_owned())
                .map_err(|error| failed(&error))?;
        }
        self.accounts
            .put(AccountPut {
                name: user.to_owned(),
                display_name: Some(user.to_owned()),
                ..AccountPut::default()
            })
            .map_err(|error| {
                if error.status == Status::GUATIAO_ERR_BAD_VALUE {
                    PasskeyError::Refused(error.to_string())
                } else {
                    failed(&error)
                }
            })?
            .secret
            .ok_or_else(|| PasskeyError::Failed("the provider returned no options".to_owned()))
    }

    pub(super) fn confirm(
        &self,
        user: &str,
        uid: u32,
        answer: &str,
        label: &str,
    ) -> Result<String, PasskeyError> {
        if answer.is_empty() || answer.len() > MAX_ANSWER || !answer.starts_with('{') {
            return Err(PasskeyError::Refused("not a passkey answer".to_owned()));
        }
        let before: Vec<String> = self
            .passkeys(user, uid)?
            .into_iter()
            .map(|passkey| passkey.id)
            .collect();
        let label: String = label.trim().chars().take(MAX_LABEL).collect();
        self.accounts
            .put(AccountPut {
                name: user.to_owned(),
                secret: Some(answer.to_owned()),
                groups: Some(vec![format!("{UID_MARKER}{uid}")]),
                display_name: (!label.is_empty()).then_some(label),
                ..AccountPut::default()
            })
            .map_err(|error| match error.status {
                Status::GUATIAO_ERR_NOT_FOUND => PasskeyError::NothingWaiting,
                Status::GUATIAO_ERR_BAD_VALUE => PasskeyError::Refused(error.to_string()),
                _ => failed(&error),
            })?;
        Ok(self
            .passkeys(user, uid)?
            .into_iter()
            .map(|passkey| passkey.id)
            .find(|id| !before.contains(id))
            .unwrap_or_default())
    }

    pub(super) fn remove(&self, user: &str, uid: u32, id: &str) -> Result<bool, PasskeyError> {
        if !self.passkeys(user, uid)?.iter().any(|each| each.id == id) {
            return Ok(false);
        }
        self.drop_credential(user, id)
    }

    /// Takes `id` from the account called `name`, whoever it was made for.
    pub(super) fn drop_credential(&self, name: &str, id: &str) -> Result<bool, PasskeyError> {
        match self.accounts.put(AccountPut {
            name: name.to_owned(),
            secret: Some(format!("remove:{id}")),
            ..AccountPut::default()
        }) {
            Ok(_) => Ok(true),
            Err(error) if error.status == Status::GUATIAO_ERR_NOT_FOUND => Ok(false),
            Err(error) => Err(failed(&error)),
        }
    }

    pub(super) fn label(
        &self,
        user: &str,
        uid: u32,
        id: &str,
        label: &str,
    ) -> Result<(), PasskeyError> {
        if !self.passkeys(user, uid)?.iter().any(|each| each.id == id) {
            return Err(PasskeyError::NoPasskey);
        }
        let label: String = label.trim().chars().take(MAX_LABEL).collect();
        self.accounts
            .put(AccountPut {
                name: user.to_owned(),
                secret: Some(format!("label:{id}")),
                display_name: Some(label),
                ..AccountPut::default()
            })
            .map(drop)
            .map_err(|error| match error.status {
                Status::GUATIAO_ERR_NOT_FOUND => PasskeyError::NoPasskey,
                _ => failed(&error),
            })
    }

    /// Takes the account away whole, passkeys and all, for an administrator.
    pub(super) fn remove_account(&self, name: &str) -> Result<bool, PasskeyError> {
        match self.accounts.remove(name.to_owned()) {
            Ok(()) => Ok(true),
            Err(error) if error.status == Status::GUATIAO_ERR_NOT_FOUND => Ok(false),
            Err(error) => Err(failed(&error)),
        }
    }
}

/// The provider's account and challenge kinds, built from the one configuration the authenticator was.
pub(super) fn store(
    registry: &Registry,
    id: &str,
    config: &coqui_access::guatiao::Value,
    link: Link,
    files: Vec<std::path::PathBuf>,
) -> Result<PasskeyStore, String> {
    let built = |why: String| format!("authenticators: {id}: {why}");
    let accounts: Arc<dyn AccountKind> = Arc::new(
        registry
            .offer::<dyn AccountKind>(id)
            .ok_or_else(|| built("no provider has that id".to_owned()))?
            .map_err(|_| built("it keeps no accounts, so it cannot register passkeys".to_owned()))?
            .instantiate(config)
            .map_err(|error| built(error.to_string()))?,
    );
    let challenger: Arc<dyn Challenger> = Arc::new(
        registry
            .offer::<dyn Challenger>(id)
            .ok_or_else(|| built("no provider has that id".to_owned()))?
            .map_err(|_| built("it issues no challenges".to_owned()))?
            .instantiate(config)
            .map_err(|error| built(error.to_string()))?,
    );
    if link.directory.is_none() {
        return Err(built(
            "it has no directory, so it cannot say whose a passkey is".to_owned(),
        ));
    }
    Ok(PasskeyStore {
        link,
        challenger,
        accounts,
        files,
    })
}

/// The files a passkey provider's configuration keeps its tables in (`source`, and by default `pending` and `ceremonies` beside
/// it), the lock the table library keeps beside each, and the folder they are made in.
pub(super) fn files_of(
    config: &serde_json::Map<String, serde_json::Value>,
) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    let Some(source) = config
        .get("source")
        .and_then(|value| value.get("file"))
        .and_then(|value| value.get("path"))
        .and_then(serde_json::Value::as_str)
        .map(std::path::PathBuf::from)
    else {
        return files;
    };
    for suffix in [
        "",
        ".pending",
        ".ceremonies",
        ".lock",
        ".pending.lock",
        ".ceremonies.lock",
    ] {
        let mut path = source.clone().into_os_string();
        path.push(suffix);
        files.push(std::path::PathBuf::from(path));
    }
    files.push(match source.parent() {
        Some(folder) if !folder.as_os_str().is_empty() => folder.to_path_buf(),
        _ => std::path::PathBuf::from("."),
    });
    files
}
