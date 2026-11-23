//! One-time code store, administrator factor management, and file ownership checks for the portal.

use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use std::path::PathBuf;

use coqui_access::guatiao::Status;
use coqui_access::guatiao::library::Registry;
use coqui_access::{
    AccountInfo, AccountPut, Accounts as AccountKind, Attempt as Credential, Verdict,
};

use super::super::signin::{CodeCheck, Enrolment, FactorError, FactorStanding, Method, valid_user};
use super::Link;

/// The longest one-time or recovery code a try takes, in bytes: a code is a few digits or a hyphenated group of ten.
pub(super) const MAX_CODE: usize = 32;

/// One account's one-time code as an administrator's listing shows it: never a secret.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FactorEntry {
    /// The record's key: the user id and the name, `1000:alice`. What `factor reset --id` takes.
    pub id: String,
    /// The account's name, when the system still has one for the id.
    pub user: Option<String>,
    /// A code is set up.
    pub enrolled: bool,
    /// An administrator disabled it.
    pub disabled: bool,
    /// A setup is waiting for its first code.
    pub waiting: bool,
    /// An administrator reset it: only an administrator sets one up again.
    pub reset: bool,
}

/// The provider that keeps one-time codes: it checks them, and its account kind sets them up, resets them and clears their failure
/// counts.
pub(super) struct CodeStore {
    pub(super) link: Link,
    pub(super) accounts: Arc<dyn AccountKind>,
    /// The files the provider's configuration keeps its tables in.
    pub(super) files: Vec<PathBuf>,
}

/// Why a code could not be taken away.
pub(super) enum KeyError {
    Unsupported,
    Failed(String),
}

/// What a code record is called in its table: the account's user id and name, so a user id made again for another account does
/// not find the code of the one before.
pub(super) fn code_key(user: &str, uid: u32) -> String {
    format!("{uid}:{user}")
}

/// The user id and name a code key holds, when it is one.
pub(super) fn split_key(key: &str) -> Option<(u32, &str)> {
    let (uid, user) = key.split_once(':')?;
    let uid = uid.parse().ok()?;
    valid_user(user).then_some((uid, user))
}

/// Whether the provider keeps the account's reset mark.
fn is_reset(account: &AccountInfo) -> bool {
    account.reset == Some(true)
}

impl CodeStore {
    /// When an administrator last cleared an account's failure counts, if one did, in seconds since the epoch.
    pub(super) fn unlock_stamp(&self, key: &str) -> Option<i64> {
        self.accounts
            .accounts()
            .ok()?
            .into_iter()
            .find(|account| account.name == key)?
            .unlocked_unix
    }

    pub(super) fn list_factors(&self) -> Result<Vec<FactorEntry>, String> {
        let enrolled = self
            .accounts
            .accounts()
            .map_err(|error| error.to_string())?;
        let waiting = self.accounts.waiting().map_err(|error| error.to_string())?;
        let mut ids: Vec<String> = enrolled
            .iter()
            .map(|account| account.name.clone())
            .chain(waiting.iter().map(|each| each.shown.clone()))
            .collect();
        ids.sort();
        ids.dedup();
        Ok(ids
            .into_iter()
            .map(|id| {
                let found = enrolled.iter().find(|account| account.name == id);
                FactorEntry {
                    user: split_key(&id).map(|(_, name)| name.to_owned()),
                    enrolled: found.is_some_and(|account| !is_reset(account)),
                    disabled: found.is_some_and(|account| account.disabled == Some(true)),
                    waiting: waiting.iter().any(|each| each.shown == id),
                    reset: found.is_some_and(is_reset),
                    id,
                }
            })
            .collect())
    }

    /// Takes the account's code away, leaving the provider's mark that only an administrator sets one up again. `false` when the
    /// provider held no code for the key: none ever, or one already reset.
    pub(super) fn reset_key(&self, key: &str) -> Result<bool, KeyError> {
        let held = self
            .accounts
            .accounts()
            .map_err(|error| KeyError::Failed(error.to_string()))?
            .iter()
            .any(|account| account.name == key && !is_reset(account));
        match self.accounts.put(AccountPut {
            name: key.to_owned(),
            reset: true,
            ..AccountPut::default()
        }) {
            Ok(_) => Ok(held),
            Err(error) if error.status == Status::GUATIAO_ERR_NOT_FOUND => Ok(false),
            Err(error) => Err(KeyError::Failed(error.to_string())),
        }
    }

    /// Lifts the reset marks of the user id under every name but the one just set up, which the provider lifted itself when its
    /// first code was confirmed.
    pub(super) fn clear_reset(&self, uid: u32) -> Result<(), String> {
        let all = self
            .accounts
            .accounts()
            .map_err(|error| error.to_string())?;
        for account in all.iter().filter(|account| {
            is_reset(account) && split_key(&account.name).is_some_and(|(id, _)| id == uid)
        }) {
            match self.accounts.remove(account.name.clone()) {
                Ok(()) => {}
                Err(error) if error.status == Status::GUATIAO_ERR_NOT_FOUND => {}
                Err(error) => return Err(error.to_string()),
            }
        }
        Ok(())
    }

    /// Ends the provider's delay of the account, which also stamps the time the portal reads. An account the provider holds no
    /// code for has no delay.
    pub(super) fn unlock(&self, user: &str, uid: u32) -> Result<(), String> {
        match self.accounts.put(AccountPut {
            name: code_key(user, uid),
            unlock: true,
            ..AccountPut::default()
        }) {
            Ok(_) => Ok(()),
            Err(error) if error.status == Status::GUATIAO_ERR_NOT_FOUND => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }

    pub(super) fn check_code(
        &self,
        user: &str,
        uid: u32,
        method: Method,
        code: &str,
        providers: &[String],
    ) -> CodeCheck {
        if !method.needs_account()
            || code.is_empty()
            || code.len() > MAX_CODE
            || (!providers.is_empty() && !providers.contains(&self.link.id))
        {
            return CodeCheck::Refused;
        }
        let name = code_key(user, uid);
        let credential = Credential {
            name: Some(name.clone()),
            secret: Some(code.to_owned()),
            method: Some(method.id().to_owned()),
            ..Credential::default()
        };
        match self.link.authenticator.authenticate(credential) {
            // The provider answers by the account key it was given; any other answer is not the account that was named.
            Ok(Verdict::Valid(lookup)) if lookup.by == "id" && lookup.value == name => {
                CodeCheck::Valid {
                    provider: self.link.id.clone(),
                }
            }
            Ok(_) => CodeCheck::Refused,
            Err(error) => CodeCheck::Failed(error.to_string()),
        }
    }

    pub(super) fn factor_record(&self, user: &str, uid: u32) -> (FactorStanding, Option<i64>) {
        let name = code_key(user, uid);
        let Ok(all) = self.accounts.accounts() else {
            return (FactorStanding::Unknown, None);
        };
        if let Some(account) = all.iter().find(|account| account.name == name) {
            return if is_reset(account) {
                (FactorStanding::Reset, None)
            } else if account.disabled == Some(true) {
                (FactorStanding::Disabled, account.made_unix)
            } else {
                (FactorStanding::Enrolled, account.made_unix)
            };
        }
        // The same user id under another name is the account renamed, or another made in its place: either way the code was
        // another account's, and the account is the administrator's to set up, not open to setting one up for itself.
        let renamed =
            |key: &str| split_key(key).is_some_and(|(id, each)| id == uid && each != user);
        if all.iter().any(|account| renamed(&account.name)) {
            (FactorStanding::Reset, None)
        } else {
            (FactorStanding::NotEnrolled, None)
        }
    }

    pub(super) fn begin_factor(
        &self,
        user: &str,
        uid: u32,
        proved_current: bool,
    ) -> Result<Enrolment, FactorError> {
        let outcome = self
            .accounts
            .put(AccountPut {
                name: code_key(user, uid),
                proved_current,
                display_name: Some(user.to_owned()),
                ..AccountPut::default()
            })
            .map_err(|error| FactorError::Failed(error.to_string()))?;
        match (outcome.secret, outcome.codes) {
            (Some(uri), codes) => Ok(Enrolment {
                uri,
                codes: codes.unwrap_or_default(),
            }),
            _ => Err(FactorError::Failed(
                "the provider returned no setup link".to_owned(),
            )),
        }
    }

    pub(super) fn confirm_factor(
        &self,
        user: &str,
        uid: u32,
        code: &str,
    ) -> Result<String, FactorError> {
        if code.is_empty() || code.len() > MAX_CODE {
            return Err(FactorError::Wrong);
        }
        self.accounts
            .put(AccountPut {
                name: code_key(user, uid),
                secret: Some(code.to_owned()),
                ..AccountPut::default()
            })
            .map(|_| self.link.id.clone())
            .map_err(|error| match error.status {
                Status::GUATIAO_ERR_NOT_FOUND => FactorError::NothingWaiting,
                Status::GUATIAO_ERR_BAD_VALUE => FactorError::Wrong,
                _ => FactorError::Failed(error.to_string()),
            })
    }
}

/// The files a provider's configuration keeps its tables in (`source`, `pending` and `waiting`), the lock the table library keeps
/// beside each, and the folder they are made in.
pub(super) fn code_files(config: &serde_json::Map<String, serde_json::Value>) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for key in ["source", "pending", "waiting"] {
        let Some(source) = config
            .get(key)
            .and_then(|value| value.get("file"))
            .and_then(|value| value.get("path"))
            .and_then(serde_json::Value::as_str)
            .map(PathBuf::from)
        else {
            continue;
        };
        for suffix in ["", ".pending", ".lock", ".pending.lock"] {
            let mut path = source.clone().into_os_string();
            path.push(suffix);
            files.push(PathBuf::from(path));
        }
        files.push(match source.parent() {
            Some(folder) if !folder.as_os_str().is_empty() => folder.to_path_buf(),
            _ => PathBuf::from("."),
        });
    }
    files
}

impl CodeStore {
    /// Whether the files the one-time codes are kept in, and the folder they are made in, belong to the account this process runs
    /// as. An administrator's command run as another account (root) would make files the portal's account cannot read, and every
    /// code step would then answer `503`.
    ///
    /// # Errors
    ///
    /// A message naming the path and whose it is.
    pub(super) fn check_files_owned(&self) -> Result<(), String> {
        use std::os::unix::fs::MetadataExt as _;
        let me = rustix::process::geteuid().as_raw();
        for path in &self.files {
            match std::fs::metadata(path) {
                Ok(meta) if meta.uid() != me => {
                    return Err(format!(
                        "{} belongs to user id {}, not to the account this command runs as ({me}): run it as the portal's account",
                        path.display(),
                        meta.uid()
                    ));
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("{}: {error}", path.display())),
            }
        }
        Ok(())
    }
}

pub(super) fn code_accounts(
    registry: &Registry,
    id: &str,
    config: &coqui_access::guatiao::Value,
) -> Result<Arc<dyn AccountKind>, String> {
    let built = |why: String| format!("authenticators: {id}: {why}");
    Ok(Arc::new(
        registry
            .offer::<dyn AccountKind>(id)
            .ok_or_else(|| built("no provider has that id".to_owned()))?
            .map_err(|_| built("it keeps no accounts, so it cannot set up codes".to_owned()))?
            .instantiate(config)
            .map_err(|error| built(error.to_string()))?,
    ))
}
