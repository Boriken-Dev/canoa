//! One-time code store, administrator factor management, and file ownership checks for the portal.

use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use std::path::{Path, PathBuf};

use coqui_access::guatiao::Status;
use coqui_access::guatiao::library::Registry;
use coqui_access::{AccountPut, Accounts as AccountKind, Attempt as Credential, Verdict};

use super::super::settings::{Entry, Settings};
use super::super::signin::{CodeCheck, Enrolment, FactorError, FactorStanding, Method, valid_user};
use super::{Link, PROXY, SYSTEM};

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

/// The provider that keeps one-time codes: it checks them, and its account kind sets them up and takes them away.
pub(super) struct CodeStore {
    pub(super) link: Link,
    pub(super) accounts: Arc<dyn AccountKind>,
    /// The file that says which accounts were reset, beside the table.
    pub(super) resets: Option<PathBuf>,
    /// The file that says when an administrator last cleared an account's failure counts.
    pub(super) unlocks: Option<PathBuf>,
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

impl CodeStore {
    /// The accounts an administrator reset, which only an administrator sets a code up for again: one key a line in a file
    /// beside the table. A file that is not there is none.
    pub(super) fn reset_marks(&self) -> Result<Vec<String>, String> {
        match &self.resets {
            Some(path) => read_lines(path),
            None => Ok(Vec::new()),
        }
    }

    /// Changes the reset marks while holding the lock beside them, so that two commands do not lose each other's changes.
    pub(super) fn update_marks(&self, change: impl FnOnce(&mut Vec<String>)) -> Result<(), String> {
        let Some(path) = &self.resets else {
            return Ok(());
        };
        locked(path, || {
            let mut marks = read_lines(path)?;
            let before = marks.clone();
            change(&mut marks);
            if marks != before {
                write_lines(path, &marks)?;
            }
            Ok(())
        })
    }

    /// The time an administrator last cleared an account's failure counts, if one did.
    pub(super) fn unlock_stamp(&self, key: &str) -> Option<i64> {
        let path = self.unlocks.as_ref()?;
        read_lines(path).ok()?.into_iter().find_map(|line| {
            let (found, millis) = line.rsplit_once(' ')?;
            (found == key).then(|| millis.parse().ok())?
        })
    }

    pub(super) fn write_unlock(&self, key: &str, millis: i64) -> Result<(), String> {
        let Some(path) = &self.unlocks else {
            return Ok(());
        };
        locked(path, || {
            let mut lines = read_lines(path)?;
            lines.retain(|line| line.rsplit_once(' ').is_none_or(|(found, _)| found != key));
            lines.push(format!("{key} {millis}"));
            write_lines(path, &lines)
        })
    }

    pub(super) fn list_factors(&self) -> Result<Vec<FactorEntry>, String> {
        let enrolled = self
            .accounts
            .accounts()
            .map_err(|error| error.to_string())?;
        let waiting = self.accounts.waiting().map_err(|error| error.to_string())?;
        let marks = self.reset_marks()?;
        let mut ids: Vec<String> = enrolled
            .iter()
            .map(|account| account.name.clone())
            .chain(waiting.iter().map(|each| each.shown.clone()))
            .chain(marks.iter().cloned())
            .collect();
        ids.sort();
        ids.dedup();
        Ok(ids
            .into_iter()
            .map(|id| {
                let found = enrolled.iter().find(|account| account.name == id);
                FactorEntry {
                    user: split_key(&id).map(|(_, name)| name.to_owned()),
                    enrolled: found.is_some(),
                    disabled: found.is_some_and(|account| account.disabled == Some(true)),
                    waiting: waiting.iter().any(|each| each.shown == id),
                    reset: marks.contains(&id),
                    id,
                }
            })
            .collect())
    }

    pub(super) fn reset_key(&self, key: &str) -> Result<bool, KeyError> {
        // The mark is written first: a reset that is interrupted leaves an account that is marked and still has its code, not
        // one that has neither and is open to setting one up for itself. It is made even when there is nothing to remove.
        self.update_marks(|marks| {
            if !marks.iter().any(|mark| mark == key) {
                marks.push(key.to_owned());
            }
        })
        .map_err(KeyError::Failed)?;
        match self.accounts.remove(key.to_owned()) {
            Ok(()) => Ok(true),
            Err(error) if error.status == Status::GUATIAO_ERR_NOT_FOUND => Ok(false),
            Err(error) => Err(KeyError::Failed(error.to_string())),
        }
    }

    pub(super) fn unlock(&self, user: &str, uid: u32) -> Result<(), String> {
        let key = code_key(user, uid);
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| {
                i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
            });
        self.write_unlock(&key, millis)?;
        // The provider's own delay ends with an administrator's `disabled` toggle, which is passed back as it stands: a code an
        // administrator disabled stays disabled. An account with no code has no delay.
        let all = self
            .accounts
            .accounts()
            .map_err(|error| error.to_string())?;
        let Some(record) = all.iter().find(|account| account.name == key) else {
            return Ok(());
        };
        match self.accounts.put(AccountPut {
            name: key,
            disabled: Some(record.disabled == Some(true)),
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
            let standing = if account.disabled == Some(true) {
                FactorStanding::Disabled
            } else {
                FactorStanding::Enrolled
            };
            return (standing, account.made_unix);
        }
        let Ok(marks) = self.reset_marks() else {
            return (FactorStanding::Unknown, None);
        };
        // The same user id under another name is the account renamed, or another made in its place: either way the code was
        // another account's, and the account is the administrator's to set up, not open to setting one up for itself.
        let renamed =
            |key: &str| split_key(key).is_some_and(|(id, each)| id == uid && each != user);
        if marks.contains(&name)
            || marks.iter().any(|mark| renamed(mark))
            || all.iter().any(|account| renamed(&account.name))
        {
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

fn read_lines(path: &Path) -> Result<Vec<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text.lines().map(str::to_owned).collect()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error.to_string()),
    }
}

/// Writes `lines` to `path`, privately and in one step.
fn write_lines(path: &Path, lines: &[String]) -> Result<(), String> {
    let mut text = String::new();
    for line in lines {
        text.push_str(line);
        text.push('\n');
    }
    crate::replace::write_file(path, text.as_bytes(), crate::replace::PRIVATE)
        .map_err(|error| error.to_string())
}

/// Runs `body` holding an exclusive lock on the file `<path>.lock`, which every process that changes `path` takes.
fn locked<T>(path: &Path, body: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    use std::os::unix::fs::OpenOptionsExt as _;
    let mut lock = path.to_path_buf().into_os_string();
    lock.push(".lock");
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(PathBuf::from(lock))
        .map_err(|error| error.to_string())?;
    rustix::fs::flock(&file, rustix::fs::FlockOperation::LockExclusive)
        .map_err(|error| error.to_string())?;
    // Released when `file` is closed, at the end of this call.
    body()
}

/// Whether an `authenticators` entry is the one-time code provider, by its id or its library's file name.
pub(super) fn keeps_codes(entry: &Entry) -> bool {
    let says = |text: &str| text.to_ascii_lowercase().contains("totp");
    says(&entry.provider)
        || entry
            .library
            .as_ref()
            .and_then(|path| path.file_name())
            .is_some_and(|name| says(&name.to_string_lossy()))
}

/// A file named `<source><suffix>`, for a provider that keeps its table in a file.
pub(super) fn beside(
    config: &serde_json::Map<String, serde_json::Value>,
    suffix: &str,
) -> Option<PathBuf> {
    let mut file = source_path(config)?.into_os_string();
    file.push(suffix);
    Some(PathBuf::from(file))
}

/// The file a provider's `source` names, if it names one.
fn source_path(config: &serde_json::Map<String, serde_json::Value>) -> Option<PathBuf> {
    config
        .get("source")?
        .get("file")?
        .get("path")?
        .as_str()
        .map(PathBuf::from)
}

/// Whether the files the one-time codes are kept in, and the folder they are made in, belong to the account this process runs
/// as. An administrator's command run as another account (root) would make files the portal's account cannot read, and every
/// code step would then answer `503`. Asked before anything is opened.
///
/// # Errors
///
/// A message naming the path and whose it is.
pub fn check_code_files_owned(settings: &Settings) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt as _;
    let _ = (PROXY, SYSTEM);
    let me = rustix::process::geteuid().as_raw();
    // The code provider's files only: a password table under a folder root owns is no reason to refuse. The provider is found
    // before anything is opened, by its id or its library's file name saying `totp`, whatever else it is called.
    for entry in settings
        .authenticators
        .iter()
        .filter(|entry| keeps_codes(entry))
    {
        for key in ["source", "pending", "waiting"] {
            let Some(source) = entry
                .config
                .get(key)
                .and_then(|value| value.get("file"))
                .and_then(|value| value.get("path"))
                .and_then(serde_json::Value::as_str)
                .map(PathBuf::from)
            else {
                continue;
            };
            let mut paths = vec![source.clone()];
            for suffix in [
                ".pending",
                ".lock",
                ".pending.lock",
                ".reset",
                ".reset.lock",
                ".unlock",
                ".unlock.lock",
            ] {
                let mut path = source.clone().into_os_string();
                path.push(suffix);
                paths.push(PathBuf::from(path));
            }
            paths.push(match source.parent() {
                Some(folder) if !folder.as_os_str().is_empty() => folder.to_path_buf(),
                _ => PathBuf::from("."),
            });
            for path in paths {
                match std::fs::metadata(&path) {
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
        }
    }
    Ok(())
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

#[cfg(test)]
mod write_tests {
    use crate::replace::proof;

    #[test]
    fn a_list_of_lines_is_whole_private_and_keeps_the_mode_it_had() {
        let dir = proof::folder("access-lines");
        let file = dir.join("deeper").join("resets");
        super::write_lines(&file, &["one".to_owned(), "two".to_owned()]).expect("written");
        assert_eq!(std::fs::read_to_string(&file).expect("read"), "one\ntwo\n");
        assert_eq!(proof::mode(&file), 0o600);
        proof::set_mode(&file, 0o640);
        super::write_lines(&file, &["three".to_owned()]).expect("written");
        assert_eq!(proof::mode(&file), 0o640);
        assert_eq!(proof::names(&dir.join("deeper")), ["resets"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
