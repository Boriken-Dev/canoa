//! The portal's sign-in providers: an ordered list of coqui-access authenticators, each credential going to the provider that
//! claims it, and every method ending at an account of the operating system.
//!
//! A name and a password go, in list order, to the providers that take a password: `Unknown` passes the credential on and
//! `Invalid` ends the try. A certificate the trusted proxy forwarded (DER in base64, in one header) goes to the providers that
//! take a request, as the chain a transport would have verified the key of; they decide whether to trust it (pinned, or signed
//! by a CA) and which account it is. Whoever a provider names is then looked up in the operating system's own directory under the portal's `min_uid` and `groups`, so a
//! method adds a way in and never a person.

use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use core::net::IpAddr;
use std::path::{Path, PathBuf};

use coqui_access::guatiao::library::{ImportSearch, Registry};
use coqui_access::guatiao::{Alloc, Map, Status, Value};
use coqui_access::{
    AccountPut, Accounts as AccountKind, Attempt as Credential, Authenticator, Directory, Lookup,
    Verdict, takes,
};
use coqui_access_kit::matcher::{Match, MatchRule, Matcher, MatcherConfig};
use coqui_access_signin::SignInRules;
use coqui_access_system::SystemAccounts;
use coqui_peer_tls::CertificateFields;

use super::settings::{Entry, Settings};
use super::signin::{
    Accounts, CertificateStatus, CodeCheck, Enrolment, FactorError, FactorStanding, MAX_PASSWORD,
    Method, PinError, PinState, Presented, SignIn, Standing, valid_user,
};

/// The provider id of the operating system's accounts.
pub const SYSTEM: &str = "coqui_access_system";

/// The provider that reads a certificate a front proxy forwarded and answers by its claims. The portal refuses it: it trusts what
/// the proxy says, and a proxy that ignores certificate errors makes every claim a stranger's.
const PROXY: &str = "coqui_access_proxy";

/// The group a pinned certificate's record carries to name the user id its account had when it was pinned, so that an account
/// deleted and made again under the same name does not inherit the pin.
const UID_MARKER: &str = "uid:";

/// The matcher `settings` describe: its rules, or coqui-access's defaults when it gives none.
fn build_matcher(settings: &Settings) -> Result<Matcher, String> {
    let Some(rules) = &settings.matcher else {
        return Ok(Matcher::default());
    };
    let mut config = MatcherConfig::default();
    config.rule = rules
        .iter()
        .map(|rule| {
            let mut built = MatchRule::new(rule.by.clone(), rule.template.clone());
            if let Some(domain) = &rule.domain {
                built = built.domain(domain.clone());
            }
            if let Some(pins) = &rule.pins {
                built = built.pins(pins.clone());
            }
            built
        })
        .collect();
    Matcher::new(config).map_err(|error| format!("matcher: {error}"))
}

/// What a code record is called in its table: the account's user id and name, so a user id made again for another account does
/// not find the code of the one before.
fn code_key(user: &str, uid: u32) -> String {
    format!("{uid}:{user}")
}

/// The user id and name a code key holds, when it is one.
fn split_key(key: &str) -> Option<(u32, &str)> {
    let (uid, user) = key.split_once(':')?;
    let uid = uid.parse().ok()?;
    valid_user(user).then_some((uid, user))
}

/// Why a code could not be taken away.
enum KeyError {
    Unsupported,
    Failed(String),
}

impl CodeStore {
    /// The accounts an administrator reset, which only an administrator sets a code up for again: one key a line in a file
    /// beside the table. A file that is not there is none.
    fn reset_marks(&self) -> Result<Vec<String>, String> {
        match &self.resets {
            Some(path) => read_lines(path),
            None => Ok(Vec::new()),
        }
    }

    /// Changes the reset marks while holding the lock beside them, so that two commands do not lose each other's changes.
    fn update_marks(&self, change: impl FnOnce(&mut Vec<String>)) -> Result<(), String> {
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
    fn unlock_stamp(&self, key: &str) -> Option<i64> {
        let path = self.unlocks.as_ref()?;
        read_lines(path).ok()?.into_iter().find_map(|line| {
            let (found, millis) = line.rsplit_once(' ')?;
            (found == key).then(|| millis.parse().ok())?
        })
    }

    fn write_unlock(&self, key: &str, millis: i64) -> Result<(), String> {
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

/// The longest one-time or recovery code a try takes, in bytes: a code is a few digits or a hyphenated group of ten.
const MAX_CODE: usize = 32;

/// The most certificates (leaf first) and bytes a forwarded header may carry.
const MAX_CHAIN: usize = 8;
const MAX_HEADER: usize = 16 * 1024;

coqui_access::guatiao::local_providers!(
    id = "canoa-portal",
    version = env!("CARGO_PKG_VERSION"),
    providers = [SystemAccounts, SignInRules]
);

/// One authenticator of the list, built.
struct Link {
    id: String,
    authenticator: Arc<dyn Authenticator>,
    /// The provider's own directory, for naming whoever it says is valid; none for a provider that offers no directory.
    directory: Option<Arc<dyn Directory>>,
    /// Whether it takes the forwarded certificate rather than a password.
    takes_request: bool,
}

/// Where enrolled certificates are kept: a certificate provider's account kind, and its directory to say whose one is.
struct PinStore {
    accounts: Arc<dyn AccountKind>,
    directory: Arc<dyn Directory>,
}

/// The provider that keeps one-time codes: it checks them, and its account kind sets them up and takes them away.
struct CodeStore {
    link: Link,
    accounts: Arc<dyn AccountKind>,
    /// The file that says which accounts were reset, beside the table.
    resets: Option<PathBuf>,
    /// The file that says when an administrator last cleared an account's failure counts.
    unlocks: Option<PathBuf>,
}

/// Whether an `authenticators` entry is the one-time code provider, by its id or its library's file name.
fn keeps_codes(entry: &Entry) -> bool {
    let says = |text: &str| text.to_ascii_lowercase().contains("totp");
    says(&entry.provider)
        || entry
            .library
            .as_ref()
            .and_then(|path| path.file_name())
            .is_some_and(|name| says(&name.to_string_lossy()))
}

/// A file named `<source><suffix>`, for a provider that keeps its table in a file.
fn beside(config: &serde_json::Map<String, serde_json::Value>, suffix: &str) -> Option<PathBuf> {
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

/// The portal's accounts: the authenticators of `Settings::authenticators`, in order, over the operating system's directory.
pub struct Access {
    /// Who the operating system admits (`min_uid`, `groups`): every method ends here.
    system: SystemAccounts,
    passwords: Vec<Link>,
    certificates: Vec<Link>,
    /// The header the proxy puts the verified-or-not client certificate in, lower case.
    header: String,
    pins: Option<PinStore>,
    /// Ties the claims of a certificate to an account for a provider that names none.
    matcher: Matcher,
    /// The directory the matcher asks, in place of the operating system's: only a test sets it, since only the operating
    /// system's accounts give the numeric user id the match is compared with.
    matcher_directory: Option<Arc<dyn Directory>>,
    /// The one-time codes, when a provider keeps them.
    codes: Option<CodeStore>,
    /// Dropped after the links, whose instances point into what it loaded.
    _registry: Arc<Registry>,
}

impl fmt::Debug for Access {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Access")
            .field("passwords", &self.passwords.len())
            .field("certificates", &self.certificates.len())
            .finish_non_exhaustive()
    }
}

/// A registry holding the providers the portal has built in.
pub(super) fn new_registry() -> Result<Registry, String> {
    let mut registry = Registry::new("canoa-portal", env!("CARGO_PKG_VERSION"));
    registry
        .register_local("canoa-portal", library)
        .map_err(|error| error.to_string())?;
    Ok(registry)
}

pub(super) fn value_of(map: &serde_json::Map<String, serde_json::Value>) -> Result<Value, String> {
    let mut out = Map::new();
    for (key, value) in map {
        let value = guatiao_serde::adapt::to_value_in(value, Alloc::rust())
            .map_err(|_| format!("the setting {key:?} cannot be a Guatiao value"))?;
        out.set(key, value);
    }
    Ok(Value::from(out))
}

/// The configuration an entry builds from. The operating system's provider takes the portal's own `pam_service`, `min_uid`
/// and `groups` for each key the entry leaves out, so the default list signs in as the portal always has.
fn config_of(entry: &Entry, settings: &Settings) -> serde_json::Map<String, serde_json::Value> {
    let mut config = entry.config.clone();
    if entry.provider == SYSTEM {
        config
            .entry("pam_service")
            .or_insert_with(|| settings.pam_service.clone().into());
        config
            .entry("min_uid")
            .or_insert_with(|| settings.min_uid.into());
        config
            .entry("groups")
            .or_insert_with(|| settings.groups.clone().into());
    }
    config
}

/// Refuses a library unless it and every directory above it belong to root and nobody else may write them: the portal runs its
/// code, so whoever can replace the file can run as the portal.
pub(super) fn check_library(library: &Path) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt as _;
    let real = std::fs::canonicalize(library).map_err(|error| error.to_string())?;
    for each in real.ancestors() {
        let meta =
            std::fs::metadata(each).map_err(|error| format!("{}: {error}", each.display()))?;
        if meta.uid() != 0 {
            return Err(format!("{} is not owned by root", each.display()));
        }
        if meta.mode() & 0o022 != 0 {
            return Err(format!(
                "{} can be written by a group or others",
                each.display()
            ));
        }
    }
    Ok(())
}

/// The files a provider's configuration keeps accounts in (`source` and `waiting`) that a group or others can read.
fn loose_sources(config: &serde_json::Map<String, serde_json::Value>) -> Vec<PathBuf> {
    use std::os::unix::fs::MetadataExt as _;
    ["source", "waiting"]
        .iter()
        .filter_map(|key| config.get(*key)?.get("file")?.get("path")?.as_str())
        .map(PathBuf::from)
        .filter(|path| {
            std::fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.mode() & 0o077 != 0)
        })
        .collect()
}

pub(super) fn open_library(registry: &mut Registry, library: &Path) -> Result<(), String> {
    registry.set_import_search(ImportSearch::Library);
    registry
        .load_file(library)
        .map(drop)
        .map_err(|error| error.to_string())
}

impl Access {
    /// The providers `settings.authenticators` name, built, over the system directory `settings` describes.
    ///
    /// # Errors
    ///
    /// A message naming the provider, for one that is not there, does not load, refuses its configuration, takes only bearer
    /// keys, or leaves the portal with no way to sign anyone in.
    pub fn new(settings: &Settings) -> Result<Self, String> {
        let system = SystemAccounts::builder()
            .pam_service(&settings.pam_service)
            .min_uid(settings.min_uid)
            .groups(settings.groups.iter().cloned())
            .build()
            .map_err(|error| error.to_string())?;
        let mut registry = new_registry()?;
        if settings.libraries_any_owner {
            tracing::warn!(
                "libraries_any_owner: provider libraries are loaded whoever owns or may write them"
            );
        }
        let built_in = |id: &str| id == SYSTEM;
        if settings
            .authenticators
            .iter()
            .any(|entry| entry.provider == PROXY)
        {
            return Err(format!(
                "authenticators: {PROXY} is not used by the portal: a certificate it takes from a header the proxy sets is believed whatever the proxy checked; use `coqui_access_mtls` behind the portal's certificate header"
            ));
        }
        for entry in &settings.authenticators {
            match (&entry.library, built_in(&entry.provider)) {
                (Some(path), false) => {
                    if !settings.libraries_any_owner {
                        check_library(path)
                            .map_err(|why| format!("authenticators: {}: {why}", entry.provider))?;
                    }
                    open_library(&mut registry, path)
                        .map_err(|why| format!("authenticators: {}: {why}", entry.provider))?;
                }
                (Some(_), true) => {
                    return Err(format!(
                        "authenticators: {} is built in; leave its `library` out",
                        entry.provider
                    ));
                }
                (None, false) => {
                    return Err(format!(
                        "authenticators: {} is not built in; name the `library` that provides it",
                        entry.provider
                    ));
                }
                (None, true) => {}
            }
        }
        let mut passwords = Vec::new();
        let mut certificates = Vec::new();
        let mut pins = None;
        let mut codes = None;
        let matcher = build_matcher(settings)?;
        for entry in &settings.authenticators {
            for path in loose_sources(&entry.config) {
                tracing::warn!(
                    "authenticators: {}: {} can be read by a group or others; restrict it to the portal's account",
                    entry.provider,
                    path.display()
                );
            }
            let config = value_of(&config_of(entry, settings))
                .map_err(|why| format!("authenticators: {}: {why}", entry.provider))?;
            let link = link(&registry, &entry.provider, &config)?;
            let description = link
                .authenticator
                .describe()
                .map_err(|error| format!("authenticators: {}: {error}", entry.provider))?;
            if entry.enrol && description.takes.as_deref() != Some(takes::REQUEST) {
                return Err(format!(
                    "authenticators: {} does not take a certificate, so it cannot keep enrolled ones",
                    entry.provider
                ));
            }
            match description.takes.as_deref() {
                Some(takes::KEY) => {
                    tracing::info!(
                        "authenticators: {} takes bearer keys, which are not a sign-in for the page; skipped",
                        entry.provider
                    );
                }
                Some(takes::REQUEST) => {
                    if entry.enrol {
                        pins = Some(pin_store(&registry, &entry.provider, &config, &link)?);
                    }
                    certificates.push(Link {
                        takes_request: true,
                        ..link
                    });
                }
                Some(takes::OTP) => {
                    if codes.is_some() {
                        return Err(
                            "authenticators: at most one provider keeps one-time codes".to_owned()
                        );
                    }
                    // The authenticator and the account kind are two instances of the provider; only a file they share makes
                    // a setup the one visible to the other.
                    if !entry.config.contains_key("source") {
                        return Err(format!(
                            "authenticators: {} needs a `source` file to keep one-time codes in",
                            entry.provider
                        ));
                    }
                    codes = Some(CodeStore {
                        accounts: code_accounts(&registry, &entry.provider, &config)?,
                        link,
                        resets: beside(&entry.config, ".reset"),
                        unlocks: beside(&entry.config, ".unlock"),
                    });
                }
                // A provider that says nothing of what it takes is asked for a name and a password.
                Some(takes::PASSWORD) | None => passwords.push(link),
                Some(other) => {
                    return Err(format!(
                        "authenticators: {} takes {other:?}, which the portal does not know",
                        entry.provider
                    ));
                }
            }
        }
        if passwords.is_empty() && certificates.is_empty() {
            return Err("authenticators: none signs anyone in".to_owned());
        }
        Ok(Self {
            system,
            passwords,
            certificates,
            header: settings.certificate_header.clone(),
            pins,
            matcher,
            matcher_directory: None,
            codes,
            _registry: Arc::new(registry),
        })
    }

    /// The account called `name` as the portal admits one (`min_uid`, `groups`), for an administrator's command: its name as the
    /// system spells it and its user id.
    ///
    /// # Errors
    ///
    /// A message when the system does not have it, the portal would not admit it, or the directory failed.
    pub fn admitted(&self, name: &str) -> Result<(String, u32), String> {
        match self.admit(name, "") {
            SignIn::Admitted { user, uid, .. } => Ok((user, uid)),
            SignIn::Failed(why) => Err(why),
            _ => Err("not an account this portal signs in".to_owned()),
        }
    }

    /// Who has a one-time code set up, being set up or reset, for an administrator's listing.
    ///
    /// # Errors
    ///
    /// A message when no provider keeps codes or its store cannot be read.
    pub fn list_factors(&self) -> Result<Vec<FactorEntry>, String> {
        let store = self
            .codes
            .as_ref()
            .ok_or_else(|| "this portal keeps no one-time codes".to_owned())?;
        let enrolled = store
            .accounts
            .accounts()
            .map_err(|error| error.to_string())?;
        let waiting = store
            .accounts
            .waiting()
            .map_err(|error| error.to_string())?;
        let marks = store.reset_marks()?;
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

    /// Removes the code whose record is called `key` (`1000:alice`, as the listing shows it) and marks it reset: for an account
    /// that is gone, whose name no command can give.
    ///
    /// # Errors
    ///
    /// A message for a key that is not one, a portal that keeps no codes or a store that cannot be written.
    pub fn reset_by_id(&self, key: &str) -> Result<bool, String> {
        if split_key(key).is_none() {
            return Err("not a code id: it is a user id, a colon and a user name".to_owned());
        }
        self.reset_key(key).map_err(|error| match error {
            KeyError::Unsupported => "this portal keeps no one-time codes".to_owned(),
            KeyError::Failed(why) => why,
        })
    }

    fn reset_key(&self, key: &str) -> Result<bool, KeyError> {
        let store = self.codes.as_ref().ok_or(KeyError::Unsupported)?;
        // The mark is written first: a reset that is interrupted leaves an account that is marked and still has its code, not
        // one that has neither and is open to setting one up for itself. It is made even when there is nothing to remove.
        store
            .update_marks(|marks| {
                if !marks.iter().any(|mark| mark == key) {
                    marks.push(key.to_owned());
                }
            })
            .map_err(KeyError::Failed)?;
        match store.accounts.remove(key.to_owned()) {
            Ok(()) => Ok(true),
            Err(error) if error.status == Status::GUATIAO_ERR_NOT_FOUND => Ok(false),
            Err(error) => Err(KeyError::Failed(error.to_string())),
        }
    }

    /// Lifts the reset mark of an account an administrator has just set a code up for, and the marks of the same user id under
    /// other names, which were the accounts it was before.
    ///
    /// # Errors
    ///
    /// A message when the marks cannot be written.
    pub fn clear_reset(&self, user: &str, uid: u32) -> Result<(), String> {
        let Some(store) = &self.codes else {
            return Ok(());
        };
        let _ = user;
        store.update_marks(|marks| {
            marks.retain(|mark| split_key(mark).is_none_or(|(id, _)| id != uid))
        })
    }

    /// Clears what the portal counted of an account's failed second-step tries, and the provider's delay of it, for a person who
    /// was locked out by someone else's guesses and has changed their password. A portal that is running reads the time written
    /// here at the account's next try.
    ///
    /// # Errors
    ///
    /// A message when the time cannot be written or the provider cannot be asked.
    pub fn unlock(&self, user: &str, uid: u32) -> Result<(), String> {
        let store = self
            .codes
            .as_ref()
            .ok_or_else(|| "this portal keeps no one-time codes".to_owned())?;
        let key = code_key(user, uid);
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| {
                i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
            });
        store.write_unlock(&key, millis)?;
        // The provider's own delay ends with an administrator's `disabled` toggle, which is passed back as it stands: a code an
        // administrator disabled stays disabled. An account with no code has no delay.
        let all = store
            .accounts
            .accounts()
            .map_err(|error| error.to_string())?;
        let Some(record) = all.iter().find(|account| account.name == key) else {
            return Ok(());
        };
        match store.accounts.put(AccountPut {
            name: key,
            disabled: Some(record.disabled == Some(true)),
            ..AccountPut::default()
        }) {
            Ok(_) => Ok(()),
            Err(error) if error.status == Status::GUATIAO_ERR_NOT_FOUND => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }

    /// The account a provider's valid answer names, if the operating system admits it: its name and user id.
    fn resolve(&self, link: &Link, lookup: Lookup) -> SignIn {
        // A provider that names no account itself answers by the credential's claims, and the matcher ties them to an account in
        // the directory. Only a certificate the proxy forwarded reaches here, and the directory, not the certificate, decides
        // the account: it is then admitted by the operating system like any other.
        if lookup.by == "claims" {
            return self.match_claims(&link.id, &lookup);
        }
        let named = if link.id == SYSTEM {
            self.system.lookup(lookup.clone())
        } else if link.directory.is_none() && lookup.by == "name" {
            // A provider with no directory of its own names the account outright.
            return self.admit(&lookup.value, &link.id);
        } else if let Some(directory) = &link.directory {
            directory.lookup(lookup.clone())
        } else {
            return SignIn::Refused;
        };
        match named {
            Ok(Some(person)) => {
                let marker = person
                    .groups
                    .iter()
                    .find_map(|group| group.strip_prefix(UID_MARKER))
                    .map(str::to_owned);
                match self.admit(&person.name, &link.id) {
                    // A pin made for another user id than this name has now is not this account's.
                    SignIn::Admitted { uid, .. }
                        if link.takes_request
                            && marker
                                .as_deref()
                                .is_some_and(|marker| marker != uid.to_string()) =>
                    {
                        SignIn::Refused
                    }
                    other => other,
                }
            }
            // A certificate a CA signed names its account by a field of the certificate, which no table of the provider holds.
            Ok(None) if link.takes_request && lookup.by == "name" => {
                self.admit(&lookup.value, &link.id)
            }
            Ok(None) => SignIn::Refused,
            Err(error) => SignIn::Failed(error.to_string()),
        }
    }

    /// The account the matcher finds for the claims `link` answered; refused when none matches or more than one does.
    fn match_claims(&self, provider: &str, lookup: &Lookup) -> SignIn {
        let directory: &dyn Directory = match &self.matcher_directory {
            Some(directory) => directory.as_ref(),
            None => &self.system,
        };
        match self.matcher.resolve(lookup, directory) {
            Ok(matched) => match matched.result {
                Match::Found(person) => {
                    let admitted = self.admit(&person.name, provider);
                    // The directory that found the person and the operating system that admits the account must mean one
                    // account: the directory gives the account's numeric user id as the person's id.
                    match &admitted {
                        SignIn::Admitted { uid, .. }
                            if person.id.parse::<u32>().ok() != Some(*uid) =>
                        {
                            tracing::info!(
                                "a certificate from {provider} matched a directory account that is not the operating system's account of that name: refused"
                            );
                            SignIn::Refused
                        }
                        _ => admitted,
                    }
                }
                Match::Ambiguous => {
                    tracing::info!(
                        "a certificate from {} matched more than one account: refused",
                        provider
                    );
                    SignIn::Refused
                }
                _ => {
                    tracing::info!(
                        "a certificate from {} matched no account: refused",
                        provider
                    );
                    SignIn::Refused
                }
            },
            Err(error) => SignIn::Failed(error.to_string()),
        }
    }

    /// The certificate a request carries: the leaf's subject and fingerprint, when the header holds readable certificates.
    fn presented(&self, headers: &[(String, String)]) -> Option<Presented> {
        let chain = forwarded_chain(headers, &self.header)?;
        let fields = leaf_fields(chain.first()?)?;
        let expired = outside_validity(&fields);
        let map = fields.to_map();
        let first = |key: &str| map.get(key).and_then(|values| values.first()).cloned();
        Some(Presented {
            subject: first("subject").unwrap_or_default(),
            fingerprint: first("fingerprint")?,
            expired,
        })
    }

    /// `name` as the operating system admits it: under `min_uid` and `groups`, spelled as the system spells it; `provider` is the
    /// one that accepted the credential.
    fn admit(&self, name: &str, provider: &str) -> SignIn {
        if !valid_user(name) {
            return SignIn::Refused;
        }
        let lookup = Lookup {
            by: "name".to_owned(),
            value: name.to_owned(),
            claims: None,
        };
        match self.system.lookup(lookup) {
            Ok(Some(person)) if valid_user(&person.name) => match person.id.parse() {
                Ok(uid) => SignIn::Admitted {
                    user: person.name,
                    uid,
                    groups: person.groups,
                    provider: provider.to_owned(),
                },
                Err(_) => SignIn::Refused,
            },
            Ok(_) => SignIn::Refused,
            Err(error) => SignIn::Failed(error.to_string()),
        }
    }

    /// Asks `links` in order, or only the ones `providers` names, in the order it names them: the first that does not pass the
    /// credential on decides. A name no link has is skipped, so a policy naming a provider that is not here asks nobody.
    fn ask(&self, links: &[Link], credential: &Credential, providers: &[String]) -> SignIn {
        self.ask_or(links, credential, providers, SignIn::Refused)
    }

    /// [`Self::ask`], answering `unknown` when every provider asked said it does not know the credential.
    fn ask_or(
        &self,
        links: &[Link],
        credential: &Credential,
        providers: &[String],
        unknown: SignIn,
    ) -> SignIn {
        let chosen: Vec<&Link> = if providers.is_empty() {
            links.iter().collect()
        } else {
            providers
                .iter()
                .filter_map(|id| links.iter().find(|link| &link.id == id))
                .collect()
        };
        for link in chosen {
            match link.authenticator.authenticate(credential.clone()) {
                Ok(Verdict::Unknown) => {}
                Ok(Verdict::Valid(lookup)) => return self.resolve(link, lookup),
                Ok(_) => return SignIn::Refused,
                Err(error) => return SignIn::Failed(error.to_string()),
            }
        }
        unknown
    }
}

/// Whether a pin's groups name `uid` as the account's user id, or name none (a pin made by hand).
fn marker_fits(groups: &[String], uid: u32) -> bool {
    groups
        .iter()
        .find_map(|group| group.strip_prefix(UID_MARKER))
        .is_none_or(|marker| marker == uid.to_string())
}

fn pin_store(
    registry: &Registry,
    id: &str,
    config: &Value,
    link: &Link,
) -> Result<PinStore, String> {
    let built = |why: String| format!("authenticators: {id}: {why}");
    let accounts: Arc<dyn AccountKind> = Arc::new(
        registry
            .offer::<dyn AccountKind>(id)
            .ok_or_else(|| built("no provider has that id".to_owned()))?
            .map_err(|_| {
                built("it keeps no accounts, so it cannot hold enrolled certificates".to_owned())
            })?
            .instantiate(config)
            .map_err(|error| built(error.to_string()))?,
    );
    let directory = link.directory.clone().ok_or_else(|| {
        built("it has no directory, so it cannot say whose a certificate is".to_owned())
    })?;
    Ok(PinStore {
        accounts,
        directory,
    })
}

fn code_accounts(
    registry: &Registry,
    id: &str,
    config: &Value,
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

fn link(registry: &Registry, id: &str, config: &Value) -> Result<Link, String> {
    let built = |why: String| format!("authenticators: {id}: {why}");
    let offer = registry
        .offer::<dyn Authenticator>(id)
        .ok_or_else(|| built("no provider has that id".to_owned()))?
        .map_err(|_| built("it is not an authenticator".to_owned()))?;
    offer.available().map_err(|why| built(why.to_string()))?;
    let authenticator: Arc<dyn Authenticator> = Arc::new(
        offer
            .instantiate(config)
            .map_err(|error| built(error.to_string()))?,
    );
    let directory = if id == SYSTEM {
        None
    } else {
        match registry.offer::<dyn Directory>(id) {
            Some(Ok(offer)) => {
                let directory: Arc<dyn Directory> = Arc::new(
                    offer
                        .instantiate(config)
                        .map_err(|error| built(error.to_string()))?,
                );
                Some(directory)
            }
            _ => None,
        }
    };
    Ok(Link {
        id: id.to_owned(),
        authenticator,
        directory,
        takes_request: false,
    })
}

fn leaf_fields(der: &[u8]) -> Option<CertificateFields> {
    CertificateFields::parse(der).ok()
}

/// Whether now is before the certificate's first day or after its last.
fn outside_validity(fields: &CertificateFields) -> bool {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
        });
    now < fields.not_before || now > fields.not_after
}

/// The certificates (DER, leaf first) in the one `header` of `headers`: base64, comma-separated. `None` for no header, two,
/// one too large, one that is not base64 or holds too many certificates.
fn forwarded_chain(headers: &[(String, String)], header: &str) -> Option<Vec<Vec<u8>>> {
    let mut found = headers.iter().filter(|(name, _)| name == header);
    let (_, value) = found.next()?;
    if found.next().is_some() || value.len() > MAX_HEADER {
        return None;
    }
    let chain: Vec<_> = value
        .split(',')
        .map(|part| data_encoding::BASE64.decode(part.trim().as_bytes()).ok())
        .collect::<Option<_>>()?;
    if chain.is_empty() || chain.len() > MAX_CHAIN || chain.iter().any(Vec::is_empty) {
        return None;
    }
    Some(chain)
}

impl Accounts for Access {
    fn methods(&self) -> Vec<Method> {
        let mut methods = Vec::new();
        if !self.passwords.is_empty() {
            methods.push(Method::Password);
        }
        if !self.certificates.is_empty() {
            methods.push(Method::Certificate);
        }
        if self.codes.is_some() {
            methods.extend([Method::Totp, Method::RecoveryCode]);
        }
        methods
    }

    fn sign_in(&self, user: &str, password: &str, providers: &[String]) -> SignIn {
        if self.passwords.is_empty()
            || !valid_user(user)
            || password.is_empty()
            || password.len() > MAX_PASSWORD
        {
            return SignIn::Refused;
        }
        let credential = Credential {
            name: Some(user.to_owned()),
            secret: Some(password.to_owned()),
            ..Credential::default()
        };
        self.ask(&self.passwords, &credential, providers)
    }

    fn sign_in_certificate(
        &self,
        headers: &[(String, String)],
        client: IpAddr,
        providers: &[String],
    ) -> SignIn {
        if self.certificates.is_empty() {
            return SignIn::Refused;
        }
        if !self.carries_certificate(headers) {
            return SignIn::NoCertificate;
        }
        let Some(chain) = forwarded_chain(headers, &self.header) else {
            return SignIn::Refused;
        };
        // Pinned trust does not look at dates, and a certificate past them is not one to let in.
        match chain.first().and_then(|leaf| leaf_fields(leaf)) {
            None => return SignIn::Refused,
            Some(fields) if outside_validity(&fields) => return SignIn::Expired,
            Some(_) => {}
        }
        // The proxy proved the browser holds the leaf's key, and the portal believes the proxy only on its own socket; whether
        // the certificate is trusted is for the provider to say.
        let credential = Credential {
            certificates: Some(chain.into_iter().map(Into::into).collect()),
            secure: Some(true),
            // Where the browser is, which the providers that keep a waiting list of unknown certificates record.
            peer: Some(client.to_string()),
            ..Credential::default()
        };
        self.ask_or(&self.certificates, &credential, providers, SignIn::Unpinned)
    }

    fn carries_certificate(&self, headers: &[(String, String)]) -> bool {
        headers.iter().any(|(name, _)| *name == self.header)
    }

    fn certificate_status(
        &self,
        user: &str,
        uid: u32,
        headers: &[(String, String)],
    ) -> Option<CertificateStatus> {
        let pins = self.pins.as_ref()?;
        let presented = self.presented(headers).map(|presented| {
            let state = match pins.directory.lookup(Lookup {
                by: "certificate".to_owned(),
                value: presented.fingerprint.clone(),
                claims: None,
            }) {
                Ok(Some(person)) if person.name == user && marker_fits(&person.groups, uid) => {
                    PinState::Yours
                }
                // Another name's, or this name's from before the account was made again: neither signs this account in.
                Ok(Some(person)) if person.name != user => PinState::Another,
                _ => PinState::Unpinned,
            };
            (presented, state)
        });
        let (pinned, disabled) = pins.accounts.accounts().map_or((false, false), |all| {
            all.iter()
                .find(|account| account.name == user)
                .map_or((false, false), |account| {
                    let disabled = account.disabled == Some(true);
                    (!disabled && marker_fits(&account.groups, uid), disabled)
                })
        });
        Some(CertificateStatus {
            presented,
            pinned,
            disabled,
        })
    }

    fn pin(&self, user: &str, uid: u32, headers: &[(String, String)]) -> Result<(), PinError> {
        let pins = self.pins.as_ref().ok_or(PinError::Unsupported)?;
        let presented = self.presented(headers).ok_or(PinError::NoCertificate)?;
        if presented.expired {
            return Err(PinError::Expired);
        }
        if !valid_user(user) {
            return Err(PinError::Failed("not an account name".to_owned()));
        }
        let all = pins
            .accounts
            .accounts()
            .map_err(|error| PinError::Failed(error.to_string()))?;
        // A pin an administrator disabled is theirs to lift: removing it and enrolling again would lift it from here.
        if all
            .iter()
            .any(|account| account.name == user && account.disabled == Some(true))
        {
            return Err(PinError::Disabled);
        }
        match pins.directory.lookup(Lookup {
            by: "certificate".to_owned(),
            value: presented.fingerprint.clone(),
            claims: None,
        }) {
            Ok(Some(person)) if person.name != user => return Err(PinError::Taken),
            Err(error) => return Err(PinError::Failed(error.to_string())),
            _ => {}
        }
        // The provider adds a pin to an account that has one. Enrolling means this certificate is the account's only one, so the
        // record goes first: a failure after that leaves the account with no pin, never with the old one.
        if all.iter().any(|account| account.name == user) {
            pins.accounts
                .remove(user.to_owned())
                .map_err(|error| PinError::Failed(error.to_string()))?;
        }
        pins.accounts
            .put(AccountPut {
                name: user.to_owned(),
                secret: Some(presented.fingerprint),
                groups: Some(vec![format!("{UID_MARKER}{uid}")]),
                ..AccountPut::default()
            })
            .map(drop)
            .map_err(|error| PinError::Failed(error.to_string()))
    }

    fn unpin(&self, user: &str) -> Result<bool, PinError> {
        let pins = self.pins.as_ref().ok_or(PinError::Unsupported)?;
        let all = pins
            .accounts
            .accounts()
            .map_err(|error| PinError::Failed(error.to_string()))?;
        let Some(found) = all.iter().find(|account| account.name == user) else {
            return Ok(false);
        };
        if found.disabled == Some(true) {
            return Err(PinError::Disabled);
        }
        pins.accounts
            .remove(user.to_owned())
            .map(|()| true)
            .map_err(|error| PinError::Failed(error.to_string()))
    }

    fn check_code(
        &self,
        user: &str,
        uid: u32,
        method: Method,
        code: &str,
        providers: &[String],
    ) -> CodeCheck {
        let Some(store) = &self.codes else {
            return CodeCheck::Refused;
        };
        if !method.needs_account()
            || code.is_empty()
            || code.len() > MAX_CODE
            || (!providers.is_empty() && !providers.contains(&store.link.id))
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
        match store.link.authenticator.authenticate(credential) {
            // The provider answers by the account key it was given; any other answer is not the account that was named.
            Ok(Verdict::Valid(lookup)) if lookup.by == "id" && lookup.value == name => {
                CodeCheck::Valid {
                    provider: store.link.id.clone(),
                }
            }
            Ok(_) => CodeCheck::Refused,
            Err(error) => CodeCheck::Failed(error.to_string()),
        }
    }

    fn factor_record(&self, user: &str, uid: u32) -> (FactorStanding, Option<i64>) {
        let Some(store) = &self.codes else {
            return (FactorStanding::Unsupported, None);
        };
        let name = code_key(user, uid);
        let Ok(all) = store.accounts.accounts() else {
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
        let Ok(marks) = store.reset_marks() else {
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

    fn unlock_stamp(&self, user: &str, uid: u32) -> Option<i64> {
        self.codes.as_ref()?.unlock_stamp(&code_key(user, uid))
    }

    fn begin_factor(
        &self,
        user: &str,
        uid: u32,
        proved_current: bool,
    ) -> Result<Enrolment, FactorError> {
        let store = self.codes.as_ref().ok_or(FactorError::Unsupported)?;
        let outcome = store
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

    fn confirm_factor(&self, user: &str, uid: u32, code: &str) -> Result<String, FactorError> {
        let store = self.codes.as_ref().ok_or(FactorError::Unsupported)?;
        if code.is_empty() || code.len() > MAX_CODE {
            return Err(FactorError::Wrong);
        }
        store
            .accounts
            .put(AccountPut {
                name: code_key(user, uid),
                secret: Some(code.to_owned()),
                ..AccountPut::default()
            })
            .map(|_| store.link.id.clone())
            .map_err(|error| match error.status {
                Status::GUATIAO_ERR_NOT_FOUND => FactorError::NothingWaiting,
                Status::GUATIAO_ERR_BAD_VALUE => FactorError::Wrong,
                _ => FactorError::Failed(error.to_string()),
            })
    }

    fn reset_factor(&self, user: &str, uid: u32) -> Result<bool, FactorError> {
        self.reset_key(&code_key(user, uid))
            .map_err(|error| match error {
                KeyError::Unsupported => FactorError::Unsupported,
                KeyError::Failed(why) => FactorError::Failed(why),
            })
    }

    fn still(&self, user: &str, uid: u32) -> Standing {
        let ask = || {
            self.system.lookup(Lookup {
                by: "id".to_owned(),
                value: uid.to_string(),
                claims: None,
            })
        };
        // A directory that fails once may answer the next time; only a definite answer ends a session.
        let found = ask().or_else(|_| {
            std::thread::sleep(core::time::Duration::from_millis(200));
            ask()
        });
        match found {
            Ok(Some(person)) if person.name == user => Standing::Present(person.groups),
            Ok(_) => Standing::Gone,
            Err(_) => Standing::Unknown,
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]
mod tests {
    use std::path::PathBuf;

    use coqui_access::{AccountPut, Accounts as AccountKind};
    use sha2::{Digest as _, Sha256};

    use super::*;

    fn exe_dir() -> PathBuf {
        std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf()
    }

    /// The library cargo built beside the test binary for the dev-dependency `name`; its file name carries a hash.
    fn plugin(name: &str) -> PathBuf {
        let prefix = format!("lib{name}_plugin-");
        std::fs::read_dir(exe_dir())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|file| {
                let name = file.file_name().to_string_lossy().into_owned();
                name.starts_with(&prefix) && name.ends_with(".so")
            })
            .max_by_key(|file| file.metadata().and_then(|meta| meta.modified()).ok())
            .map(|file| file.path())
            .unwrap_or_else(|| panic!("{name}'s plugin is built beside the tests"))
    }

    /// This process's user id: an account every machine has, so the system directory has someone to admit.
    fn mine() -> u32 {
        std::os::unix::fs::MetadataExt::uid(&std::fs::metadata("/proc/self").unwrap())
    }

    fn my_name() -> String {
        let system = SystemAccounts::builder().min_uid(0).build().unwrap();
        system
            .lookup(Lookup {
                by: "id".to_owned(),
                value: mine().to_string(),
                claims: None,
            })
            .unwrap()
            .expect("this process's user is in the directory")
            .name
    }

    fn file_source(path: &Path) -> serde_json::Value {
        serde_json::json!({"source": {"file": {"path": path, "format": "toml"}}})
    }

    /// Writes `name` into the provider's file with `secret`, through the provider's own account kind.
    fn seed(library: &str, id: &str, config: &serde_json::Value, name: &str, secret: &str) {
        let mut registry = Registry::new("seed", "0");
        open_library(&mut registry, &plugin(library)).unwrap();
        let config = value_of(config.as_object().unwrap()).unwrap();
        let accounts: Arc<dyn AccountKind> = Arc::new(
            registry
                .offer::<dyn AccountKind>(id)
                .unwrap()
                .unwrap()
                .instantiate(&config)
                .unwrap(),
        );
        accounts
            .put(AccountPut {
                name: name.to_owned(),
                secret: Some(secret.to_owned()),
                ..AccountPut::default()
            })
            .unwrap();
    }

    fn entry(provider: &str, library: Option<PathBuf>, config: &serde_json::Value) -> Entry {
        let mut entry = Entry::new(provider);
        entry.library = library;
        entry.config = config.as_object().cloned().unwrap_or_default();
        entry
    }

    fn settings(entries: Vec<Entry>) -> Settings {
        Settings {
            min_uid: mine().max(1),
            authenticators: entries,
            // The libraries are built under the tester's own target directory.
            libraries_any_owner: true,
            ..Settings::default()
        }
    }

    fn client() -> IpAddr {
        "198.51.100.7".parse().unwrap()
    }

    fn password_entry(file: &Path) -> Entry {
        entry(
            "coqui_access_password",
            Some(plugin("coqui_access_password")),
            &file_source(file),
        )
    }

    fn mtls_entry(file: &Path) -> Entry {
        entry(
            "coqui_access_mtls",
            Some(plugin("coqui_access_mtls")),
            &file_source(file),
        )
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("canoa-access-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn seed_password(file: &Path, name: &str, password: &str) {
        seed(
            "coqui_access_password",
            "coqui_access_password",
            &file_source(file),
            name,
            password,
        );
    }

    #[test]
    fn a_password_from_a_file_signs_in_the_system_account_of_that_name() {
        let dir = scratch("password");
        let file = dir.join("accounts.toml");
        let me = my_name();
        seed_password(&file, &me, "file-pw");
        let access = Access::new(&settings(vec![password_entry(&file)])).unwrap();
        assert_eq!(access.methods(), [Method::Password]);
        admits(
            access.sign_in(&me, "file-pw", &[]),
            &me,
            "coqui_access_password",
        );
        assert_eq!(access.sign_in(&me, "wrong", &[]), SignIn::Refused);
        assert_eq!(
            access.sign_in("nobody-here", "file-pw", &[]),
            SignIn::Refused
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_account_a_method_names_must_pass_the_systems_groups_and_uid() {
        let dir = scratch("admit");
        let file = dir.join("accounts.toml");
        let me = my_name();
        seed_password(&file, &me, "file-pw");
        let mut outside = settings(vec![password_entry(&file)]);
        outside.groups = vec!["canoa-no-such-group".to_owned()];
        assert_eq!(
            Access::new(&outside).unwrap().sign_in(&me, "file-pw", &[]),
            SignIn::Refused
        );
        let mut above = settings(vec![password_entry(&file)]);
        above.min_uid = mine() + 1;
        assert_eq!(
            Access::new(&above).unwrap().sign_in(&me, "file-pw", &[]),
            SignIn::Refused
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unknown_passes_the_credential_on_and_invalid_ends_the_try() {
        let dir = scratch("order");
        let (first, second) = (dir.join("first.toml"), dir.join("second.toml"));
        let me = my_name();
        seed_password(&first, &me, "pw-one");
        seed_password(&second, &me, "pw-two");
        // Invalid: the first provider owns the name and refuses `pw-two`, so the second is never asked.
        let both = Access::new(&settings(vec![
            password_entry(&first),
            password_entry(&second),
        ]))
        .unwrap();
        assert!(matches!(
            both.sign_in(&me, "pw-one", &[]),
            SignIn::Admitted { .. }
        ));
        assert_eq!(both.sign_in(&me, "pw-two", &[]), SignIn::Refused);
        // Unknown: the system provider, limited to a group this account is not in, says it is not its to judge.
        let narrow = entry(
            SYSTEM,
            None,
            &serde_json::json!({"groups": ["canoa-no-such-group"]}),
        );
        let passed_on = Access::new(&settings(vec![narrow, password_entry(&second)])).unwrap();
        assert!(matches!(
            passed_on.sign_in(&me, "pw-two", &[]),
            SignIn::Admitted { .. }
        ));
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// `result` is a sign-in of `me`, by this process's user id, accepted by `provider`.
    fn admits(result: SignIn, me: &str, provider: &str) {
        match result {
            SignIn::Admitted {
                user,
                uid,
                provider: by,
                ..
            } => assert_eq!((user.as_str(), uid, by.as_str()), (me, mine(), provider)),
            other => panic!("not admitted: {other:?}"),
        }
    }

    fn certificate() -> (Vec<u8>, String) {
        let made = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
        let der = made.cert.der().to_vec();
        let fingerprint = format!(
            "sha256:{}",
            data_encoding::HEXLOWER.encode(&Sha256::digest(&der))
        );
        (der, fingerprint)
    }

    fn header(value: &str) -> Vec<(String, String)> {
        vec![("x-client-cert".to_owned(), value.to_owned())]
    }

    fn pinned(file: &Path, groups: &[&str]) -> (Access, String, Vec<u8>) {
        let me = my_name();
        let (der, fingerprint) = certificate();
        seed(
            "coqui_access_mtls",
            "coqui_access_mtls",
            &file_source(file),
            &me,
            &fingerprint,
        );
        let mut settings = settings(vec![mtls_entry(file)]);
        settings.groups = groups.iter().map(|group| (*group).to_owned()).collect();
        (Access::new(&settings).unwrap(), me, der)
    }

    #[test]
    fn a_pinned_certificate_signs_in_its_account_and_any_other_is_refused() {
        let dir = scratch("pinned");
        let (access, me, der) = pinned(&dir.join("peers.toml"), &[]);
        assert_eq!(access.methods(), [Method::Certificate]);
        let b64 = data_encoding::BASE64.encode(&der);
        admits(
            access.sign_in_certificate(&header(&b64), client(), &[]),
            &me,
            "coqui_access_mtls",
        );
        // A certificate nobody pinned, however well formed.
        let (other, _) = certificate();
        let other = data_encoding::BASE64.encode(&other);
        assert_eq!(
            access.sign_in_certificate(&header(&other), client(), &[]),
            SignIn::Unpinned
        );
        // Nothing is told apart from garbage, two headers, and a chain longer than the portal reads.
        assert!(!access.carries_certificate(&[]));
        assert_eq!(
            access.sign_in_certificate(&[], client(), &[]),
            SignIn::NoCertificate
        );
        assert_eq!(
            access.sign_in_certificate(&header("not base64!"), client(), &[]),
            SignIn::Refused
        );
        let twice = vec![
            ("x-client-cert".to_owned(), b64.clone()),
            ("x-client-cert".to_owned(), b64.clone()),
        ];
        assert_eq!(
            access.sign_in_certificate(&twice, client(), &[]),
            SignIn::Refused
        );
        let long = [b64.as_str(); 9].join(",");
        assert_eq!(
            access.sign_in_certificate(&header(&long), client(), &[]),
            SignIn::Refused
        );
        // A password goes to no certificate provider.
        assert_eq!(access.sign_in(&me, "anything", &[]), SignIn::Refused);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_pinned_certificate_of_an_account_outside_the_groups_is_refused() {
        let dir = scratch("pinned-groups");
        let (access, _, der) = pinned(&dir.join("peers.toml"), &["canoa-no-such-group"]);
        assert_eq!(
            access.sign_in_certificate(&header(&data_encoding::BASE64.encode(&der)), client(), &[]),
            SignIn::Refused
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn enrolling(file: &Path) -> Access {
        let mut entry = mtls_entry(file);
        entry.enrol = true;
        Access::new(&settings(vec![entry])).unwrap()
    }

    /// The provider adds a pin to an account that has one; enrolling a certificate means it is the account's only one, so the
    /// one it replaces stops signing in, and removing takes the whole record.
    #[test]
    fn pinning_a_certificate_replaces_the_one_before_it_in_the_real_provider() {
        let dir = scratch("replace");
        let file = dir.join("peers.toml");
        let me = my_name();
        let (other_der, other_fingerprint) = certificate();
        seed(
            "coqui_access_mtls",
            "coqui_access_mtls",
            &file_source(&file),
            "someone-else",
            &other_fingerprint,
        );
        let access = enrolling(&file);
        let (first, _) = certificate();
        let (second, _) = certificate();
        let (third, _) = certificate();
        let headers: Vec<_> = [&first, &second, &third]
            .into_iter()
            .map(|der| header(&data_encoding::BASE64.encode(der)))
            .collect();
        for (index, enrolled) in headers.iter().enumerate() {
            access.pin(&me, mine(), enrolled).unwrap();
            for (other, probe) in headers.iter().enumerate().take(index + 1) {
                let signed = access.sign_in_certificate(probe, client(), &[]);
                if other == index {
                    admits(signed, &me, "coqui_access_mtls");
                } else {
                    assert_eq!(signed, SignIn::Unpinned, "pin {other} after pin {index}");
                }
            }
        }
        // Pinning the certificate it already has changes nothing.
        access.pin(&me, mine(), &headers[2]).unwrap();
        admits(
            access.sign_in_certificate(&headers[2], client(), &[]),
            &me,
            "coqui_access_mtls",
        );
        // Removing takes the whole record, and another account's is untouched.
        assert_eq!(access.unpin(&me), Ok(true));
        for probe in &headers {
            assert_eq!(
                access.sign_in_certificate(probe, client(), &[]),
                SignIn::Unpinned
            );
        }
        assert_eq!(access.unpin(&me), Ok(false));
        let _ = other_der;
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_certificate_is_pinned_and_removed_through_the_providers_accounts() {
        let dir = scratch("enrol");
        let file = dir.join("peers.toml");
        let me = my_name();
        // Someone else's certificate, pinned first: it also makes the file.
        let (other_der, other_fingerprint) = certificate();
        seed(
            "coqui_access_mtls",
            "coqui_access_mtls",
            &file_source(&file),
            "someone-else",
            &other_fingerprint,
        );
        let access = enrolling(&file);
        let (der, fingerprint) = certificate();
        let mine_header = header(&data_encoding::BASE64.encode(&der));
        let other_header = header(&data_encoding::BASE64.encode(&other_der));
        assert_eq!(
            access.sign_in_certificate(&mine_header, client(), &[]),
            SignIn::Unpinned
        );
        let status = access
            .certificate_status(&me, mine(), &mine_header)
            .unwrap();
        let (presented, state) = status.presented.unwrap();
        assert_eq!(
            (presented.fingerprint.as_str(), state),
            (fingerprint.as_str(), PinState::Unpinned)
        );
        assert!(!status.pinned);
        // Pinned, the certificate alone signs the account in.
        access.pin(&me, mine(), &mine_header).unwrap();
        admits(
            access.sign_in_certificate(&mine_header, client(), &[]),
            &me,
            "coqui_access_mtls",
        );
        let status = access
            .certificate_status(&me, mine(), &mine_header)
            .unwrap();
        assert_eq!(status.presented.unwrap().1, PinState::Yours);
        assert!(status.pinned);
        // One pinned to another account is that account's, and cannot be taken.
        assert_eq!(
            access
                .certificate_status(&me, mine(), &other_header)
                .unwrap()
                .presented
                .unwrap()
                .1,
            PinState::Another
        );
        assert_eq!(access.pin(&me, mine(), &other_header), Err(PinError::Taken));
        assert_eq!(access.pin(&me, mine(), &[]), Err(PinError::NoCertificate));
        assert_eq!(
            access.pin(&me, mine(), &header("not base64!")),
            Err(PinError::NoCertificate)
        );
        // Removed, it signs no one in, and removing again finds nothing.
        assert_eq!(access.unpin(&me), Ok(true));
        assert_eq!(
            access.sign_in_certificate(&mine_header, client(), &[]),
            SignIn::Unpinned
        );
        assert_eq!(access.unpin(&me), Ok(false));
        assert!(
            !access
                .certificate_status(&me, mine(), &mine_header)
                .unwrap()
                .pinned
        );
        // The other account's pin was never touched.
        assert_eq!(
            access
                .certificate_status(&me, mine(), &other_header)
                .unwrap()
                .presented
                .unwrap()
                .1,
            PinState::Another
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_portal_that_keeps_no_enrolled_certificates_pins_none() {
        let dir = scratch("no-enrol");
        let (access, me, der) = pinned(&dir.join("peers.toml"), &[]);
        let h = header(&data_encoding::BASE64.encode(&der));
        assert_eq!(access.certificate_status(&me, mine(), &h), None);
        assert_eq!(access.pin(&me, mine(), &h), Err(PinError::Unsupported));
        assert_eq!(access.unpin(&me), Err(PinError::Unsupported));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_password_file_passes_on_a_name_it_does_not_hold_and_refuses_a_wrong_password_for_one_it_does()
     {
        // The password provider answers Unknown for a name its file lacks, so the providers after it are asked, and Invalid for
        // a wrong password of a name it holds, which ends the try.
        let dir = scratch("lockout");
        let (first, second) = (dir.join("first.toml"), dir.join("second.toml"));
        let me = my_name();
        seed_password(&first, "someone-else", "pw-x");
        seed_password(&second, &me, "pw-two");
        let first_listed = Access::new(&settings(vec![
            password_entry(&first),
            password_entry(&second),
        ]))
        .unwrap();
        admits(
            first_listed.sign_in(&me, "pw-two", &[]),
            &me,
            "coqui_access_password",
        );
        assert_eq!(
            first_listed.sign_in("someone-else", "pw-x", &[]),
            SignIn::Refused
        );
        // A name the first file holds with another password is refused there, and the second is not asked.
        seed_password(&first, &me, "pw-one");
        let held = Access::new(&settings(vec![
            password_entry(&first),
            password_entry(&second),
        ]))
        .unwrap();
        assert_eq!(held.sign_in(&me, "pw-two", &[]), SignIn::Refused);
        // Only the providers a policy names are asked, in the order it names them.
        assert_eq!(
            held.sign_in(&me, "pw-two", &["coqui_access_password".to_owned()]),
            SignIn::Refused
        );
        assert_eq!(
            held.sign_in(&me, "pw-one", &["nobody".to_owned()]),
            SignIn::Refused
        );
        // Listed after a provider that leaves the name alone, the same file is reached.
        let narrow = entry(
            SYSTEM,
            None,
            &serde_json::json!({"groups": ["canoa-no-such-group"]}),
        );
        let after = Access::new(&settings(vec![narrow, password_entry(&second)])).unwrap();
        assert!(matches!(
            after.sign_in(&me, "pw-two", &[]),
            SignIn::Admitted { .. }
        ));
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn expired_certificate() -> (Vec<u8>, String) {
        let mut params = rcgen::CertificateParams::new(vec!["localhost".to_owned()]).unwrap();
        params.not_before = rcgen::date_time_ymd(2019, 1, 1);
        params.not_after = rcgen::date_time_ymd(2020, 1, 1);
        let key = rcgen::KeyPair::generate().unwrap();
        let der = params.self_signed(&key).unwrap().der().to_vec();
        let fingerprint = format!(
            "sha256:{}",
            data_encoding::HEXLOWER.encode(&Sha256::digest(&der))
        );
        (der, fingerprint)
    }

    #[test]
    fn an_expired_certificate_signs_no_one_in_even_when_it_is_pinned_and_is_never_pinned() {
        let dir = scratch("expired");
        let me = my_name();
        let (der, fingerprint) = expired_certificate();
        let file = dir.join("peers.toml");
        seed(
            "coqui_access_mtls",
            "coqui_access_mtls",
            &file_source(&file),
            &me,
            &fingerprint,
        );
        let access = enrolling(&file);
        let h = header(&data_encoding::BASE64.encode(&der));
        assert_eq!(
            access.sign_in_certificate(&h, client(), &[]),
            SignIn::Expired
        );
        let (cert, _) = access
            .certificate_status(&me, mine(), &h)
            .unwrap()
            .presented
            .unwrap();
        assert!(cert.expired);
        assert_eq!(access.pin(&me, mine(), &h), Err(PinError::Expired));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_pin_made_for_another_user_id_is_not_the_accounts_and_can_be_pinned_over() {
        let dir = scratch("uid");
        let file = dir.join("peers.toml");
        let me = my_name();
        let (other_der, other_fp) = certificate();
        seed(
            "coqui_access_mtls",
            "coqui_access_mtls",
            &file_source(&file),
            "someone-else",
            &other_fp,
        );
        let access = enrolling(&file);
        let (der, _) = certificate();
        let h = header(&data_encoding::BASE64.encode(&der));
        // Pinned for the user id this name had before it was made again.
        access.pin(&me, mine() + 1, &h).unwrap();
        assert_eq!(
            access.sign_in_certificate(&h, client(), &[]),
            SignIn::Refused
        );
        let status = access.certificate_status(&me, mine(), &h).unwrap();
        assert!(!status.pinned);
        assert_eq!(status.presented.unwrap().1, PinState::Unpinned);
        // The person pins again, and the pin is theirs now.
        access.pin(&me, mine(), &h).unwrap();
        assert!(matches!(
            access.sign_in_certificate(&h, client(), &[]),
            SignIn::Admitted { .. }
        ));
        let _ = other_der;
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_pin_an_administrator_disabled_cannot_be_removed_or_replaced_from_here() {
        let dir = scratch("disabled");
        let file = dir.join("peers.toml");
        let me = my_name();
        let (_, fingerprint) = certificate();
        seed(
            "coqui_access_mtls",
            "coqui_access_mtls",
            &file_source(&file),
            &me,
            &fingerprint,
        );
        let mut registry = Registry::new("seed", "0");
        open_library(&mut registry, &plugin("coqui_access_mtls")).unwrap();
        let config = value_of(file_source(&file).as_object().unwrap()).unwrap();
        let accounts: Arc<dyn AccountKind> = Arc::new(
            registry
                .offer::<dyn AccountKind>("coqui_access_mtls")
                .unwrap()
                .unwrap()
                .instantiate(&config)
                .unwrap(),
        );
        accounts
            .put(AccountPut {
                name: me.clone(),
                disabled: Some(true),
                ..AccountPut::default()
            })
            .unwrap();
        let access = enrolling(&file);
        let (der, _) = certificate();
        let h = header(&data_encoding::BASE64.encode(&der));
        assert_eq!(access.pin(&me, mine(), &h), Err(PinError::Disabled));
        assert_eq!(access.unpin(&me), Err(PinError::Disabled));
        let status = access.certificate_status(&me, mine(), &h).unwrap();
        assert!(status.disabled && !status.pinned);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_library_is_loaded_only_from_files_root_owns_unless_the_setting_says_otherwise() {
        let dir = scratch("owner");
        let copy = dir.join("libcopy.so");
        std::fs::copy(plugin("coqui_access_mtls"), &copy).unwrap();
        let file = dir.join("peers.toml");
        let make = |any_owner: bool| {
            let mut settings = settings(vec![entry(
                "coqui_access_mtls",
                Some(copy.clone()),
                &file_source(&file),
            )]);
            settings.libraries_any_owner = any_owner;
            Access::new(&settings)
        };
        // Owned by whoever runs the test, which is not root: refused (a test run as root cannot show this).
        if mine() != 0 {
            let refused = make(false).unwrap_err();
            assert!(refused.contains("not owned by root"), "{refused}");
        }
        assert!(make(true).is_ok());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn source_files_a_group_or_others_can_read_are_found() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = scratch("loose");
        let (tight, loose) = (dir.join("tight.toml"), dir.join("loose.toml"));
        for (path, mode) in [(&tight, 0o600), (&loose, 0o644)] {
            std::fs::write(path, "").unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
        }
        let config = serde_json::json!({
            "source": {"file": {"path": tight}},
            "waiting": {"file": {"path": loose}},
        });
        assert_eq!(loose_sources(config.as_object().unwrap()), vec![loose]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_list_the_portal_cannot_build_does_not_start() {
        for entries in [
            vec![entry("coqui_access_nothing", None, &serde_json::json!({}))],
            vec![entry(
                SYSTEM,
                Some(PathBuf::from("/x.so")),
                &serde_json::json!({}),
            )],
            vec![entry(SYSTEM, None, &serde_json::json!({"no_such_key": 1}))],
            vec![{
                let mut system = Entry::new(SYSTEM);
                system.enrol = true;
                system
            }],
        ] {
            assert!(Access::new(&settings(entries)).is_err());
        }
    }

    /// A directory of the test's own for the matcher to ask: `alice@example.org` is this process's account, `twin@example.org`
    /// is two accounts, and the filter and id lookups find nobody. It counts the lookups it was asked.
    struct Book {
        asked: std::sync::atomic::AtomicUsize,
    }

    impl Directory for Book {
        fn lookup(
            &self,
            lookup: Lookup,
        ) -> Result<Option<coqui_access::Person>, coqui_access::guatiao::library::ProviderError>
        {
            self.asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            match (lookup.by.as_str(), lookup.value.as_str()) {
                ("email", "alice@example.org") => Ok(Some(coqui_access::Person {
                    id: mine().to_string(),
                    name: my_name(),
                    ..coqui_access::Person::default()
                })),
                ("email", "twin@example.org") => Err(coqui_access::error::ambiguous()),
                // A directory that finds an account by a name that holds an `@`, and one that means another user id.
                ("name", name) if name == format!("{}@evil", my_name()) => {
                    Ok(Some(coqui_access::Person {
                        id: mine().to_string(),
                        name: my_name(),
                        ..coqui_access::Person::default()
                    }))
                }
                ("email", "mismatch@example.org") => Ok(Some(coqui_access::Person {
                    id: (mine() + 7).to_string(),
                    name: my_name(),
                    ..coqui_access::Person::default()
                })),
                ("name", name) if name == my_name() => Ok(Some(coqui_access::Person {
                    id: mine().to_string(),
                    name: my_name(),
                    ..coqui_access::Person::default()
                })),
                _ => Ok(None),
            }
        }
    }

    fn claims(pairs: &[(&str, &str)]) -> Lookup {
        let mut values = std::collections::BTreeMap::<String, Vec<String>>::new();
        for (name, value) in pairs {
            values
                .entry((*name).to_owned())
                .or_default()
                .push((*value).to_owned());
        }
        Lookup {
            by: "claims".to_owned(),
            value: "a certificate".to_owned(),
            claims: Some(coqui_access::Claims {
                values,
                certificate: None,
            }),
        }
    }

    fn matching(change: impl FnOnce(&mut Settings)) -> (Access, Arc<Book>) {
        let dir = scratch("matcher");
        let file = dir.join("accounts.toml");
        seed_password(&file, &my_name(), "x");
        let mut own = settings(vec![password_entry(&file)]);
        change(&mut own);
        let mut access = Access::new(&own).unwrap();
        let book = Arc::new(Book {
            asked: std::sync::atomic::AtomicUsize::new(0),
        });
        access.matcher_directory = Some(book.clone());
        (access, book)
    }

    #[test]
    fn a_upn_claim_that_is_an_account_name_signs_that_account_in() {
        let (access, _) = matching(|_| {});
        let me = my_name();
        admits(
            access.match_claims("cert", &claims(&[("san.upn", &me)])),
            &me,
            "cert",
        );
    }

    #[test]
    fn an_email_claim_finds_the_account_the_directory_gives_it() {
        let (access, book) = matching(|_| {});
        let me = my_name();
        admits(
            access.match_claims("cert", &claims(&[("san.email", "alice@example.org")])),
            &me,
            "cert",
        );
        assert!(book.asked.load(std::sync::atomic::Ordering::SeqCst) > 0);
    }

    #[test]
    fn an_ambiguous_claim_and_one_nobody_has_are_refused() {
        let (access, _) = matching(|_| {});
        for lookup in [
            claims(&[("san.email", "twin@example.org")]),
            claims(&[("san.email", "nobody@example.org")]),
            claims(&[("subject.cn", &my_name())]),
            claims(&[]),
        ] {
            assert_eq!(access.match_claims("cert", &lookup), SignIn::Refused);
        }
    }

    #[test]
    fn a_rule_may_strip_the_realm_only_when_it_names_it() {
        let rule = |domain: Option<&str>| {
            vec![crate::portal::MatcherRule::new(
                "name",
                "{san.upn|local}",
                domain,
            )]
        };
        let dir = scratch("realm");
        let file = dir.join("accounts.toml");
        seed_password(&file, &my_name(), "x");
        let mut bare = settings(vec![password_entry(&file)]);
        bare.matcher = Some(rule(None));
        assert!(
            Access::new(&bare).is_err(),
            "`local` without a realm does not start"
        );
        let (access, _) = matching(|own| own.matcher = Some(rule(Some("corp.example"))));
        let me = my_name();
        admits(
            access.match_claims(
                "cert",
                &claims(&[("san.upn", &format!("{me}@corp.example"))]),
            ),
            &me,
            "cert",
        );
        // The same name in another realm is not this account.
        assert_eq!(
            access.match_claims(
                "cert",
                &claims(&[("san.upn", &format!("{me}@evil.example"))])
            ),
            SignIn::Refused
        );
    }

    fn totp_entry(file: &Path) -> Entry {
        let mut config = file_source(file);
        config["skew"] = 10.into();
        entry(
            "coqui_access_totp",
            Some(plugin("coqui_access_totp")),
            &config,
        )
    }

    #[test]
    fn a_code_store_needs_a_source_file_and_only_one_provider_keeps_codes() {
        let dir = scratch("totp-source");
        let file = dir.join("accounts.toml");
        seed_password(&file, &my_name(), "x");
        let memory = entry(
            "coqui_access_totp",
            Some(plugin("coqui_access_totp")),
            &serde_json::json!({}),
        );
        assert!(Access::new(&settings(vec![password_entry(&file), memory])).is_err());
        let twice = settings(vec![
            password_entry(&file),
            totp_entry(&dir.join("a.toml")),
            totp_entry(&dir.join("b.toml")),
        ]);
        assert!(Access::new(&twice).is_err());
    }

    #[test]
    fn an_administrators_listing_names_who_has_a_code_and_never_shows_a_secret() {
        let dir = scratch("totp-list");
        let file = dir.join("accounts.toml");
        let me = my_name();
        seed_password(&file, &me, "x");
        let access = Access::new(&settings(vec![
            password_entry(&file),
            totp_entry(&dir.join("totp.toml")),
        ]))
        .unwrap();
        assert!(access.methods().contains(&Method::Totp));
        assert_eq!(access.admitted(&me), Ok((me.clone(), mine())));
        assert!(access.admitted("nobody-here").is_err());
        assert!(access.admitted("../x").is_err());
        assert_eq!(access.list_factors(), Ok(Vec::new()));
        let begun = access.begin_factor(&me, mine(), false).unwrap();
        let waiting = access.list_factors().unwrap();
        assert_eq!(waiting.len(), 1);
        assert_eq!(waiting[0].user.as_deref(), Some(me.as_str()));
        assert!((waiting[0].waiting, waiting[0].enrolled) == (true, false));
        let shown = format!("{waiting:?}{begun:?}");
        assert!(!shown.contains("otpauth") && !shown.contains("secret="));
        assert_eq!(
            access.factor_standing(&me, mine()),
            FactorStanding::NotEnrolled
        );
        assert_eq!(access.reset_factor(&me, mine()), Ok(true));
        // Reset leaves the account marked for an administrator, with nothing else of it.
        let after = access.list_factors().unwrap();
        assert_eq!(
            (after.len(), after[0].reset, after[0].enrolled),
            (1, true, false)
        );
        // A code is checked for the account named, whatever the portal's other methods say.
        assert_eq!(
            access.check_code(&me, mine(), Method::Totp, "000000", &[]),
            CodeCheck::Refused
        );
        assert_eq!(
            access.check_code(&me, mine(), Method::Password, "000000", &[]),
            CodeCheck::Refused
        );
        assert_eq!(
            access.check_code(&me, mine(), Method::Totp, "000000", &["another".to_owned()]),
            CodeCheck::Refused,
            "a provider the policy did not name is not asked"
        );
    }

    #[test]
    fn a_value_with_two_ats_is_refused_before_the_realm_is_stripped() {
        let (access, _) = matching(|own| {
            own.matcher = Some(vec![crate::portal::MatcherRule::new(
                "name",
                "{san.upn|local}",
                Some("corp.example"),
            )]);
        });
        let me = my_name();
        // One `@` is the realm and strips to the account; two leave a name the directory above finds as the same account.
        admits(
            access.match_claims(
                "cert",
                &claims(&[("san.upn", &format!("{me}@corp.example"))]),
            ),
            &me,
            "cert",
        );
        for value in [
            format!("{me}@evil@corp.example"),
            format!("@{me}@corp.example"),
        ] {
            assert_eq!(
                access.match_claims("cert", &claims(&[("san.upn", &value)])),
                SignIn::Refused,
                "{value}"
            );
        }
        assert_eq!(
            access.match_claims(
                "cert",
                &claims(&[("san.email", &format!("{me}@evil@corp.example"))])
            ),
            SignIn::Refused
        );
    }

    #[test]
    fn a_directory_that_means_another_user_id_than_the_system_does_is_not_the_account() {
        let (access, _) = matching(|_| {});
        assert_eq!(
            access.match_claims("cert", &claims(&[("san.email", "mismatch@example.org")])),
            SignIn::Refused
        );
    }

    #[test]
    fn the_proxy_provider_is_refused_and_so_is_a_listing_that_names_it() {
        let dir = scratch("proxy");
        let file = dir.join("accounts.toml");
        seed_password(&file, &my_name(), "x");
        let mut proxy = Entry::new("coqui_access_proxy");
        proxy.library = Some(PathBuf::from("/does/not/exist.so"));
        let error = Access::new(&settings(vec![password_entry(&file), proxy])).unwrap_err();
        assert!(error.contains("not used by the portal"), "{error}");
    }

    #[test]
    fn code_files_made_by_another_account_are_refused_before_anything_is_opened() {
        let dir = scratch("owned");
        let ours = settings(vec![totp_entry(&dir.join("totp.toml"))]);
        assert_eq!(check_code_files_owned(&ours), Ok(()));
        // `/tmp` belongs to root: files made there as this account would be root's to remove, and root's as the other way round.
        let name = format!("canoa-not-made-{}.toml", std::process::id());
        let rooted = settings(vec![totp_entry(&PathBuf::from("/tmp").join(&name))]);
        let error = check_code_files_owned(&rooted).unwrap_err();
        assert!(error.contains("belongs to user id 0"), "{error}");
        assert!(!PathBuf::from("/tmp").join(&name).exists());
        // Under another id, and in a key the provider is given for a second file, it is the same.
        let other = entry(
            "codes_under_another_name",
            Some(PathBuf::from("/opt/libcoqui_access_totp_plugin.so")),
            &serde_json::json!({"source": {"file": {"path": dir.join("t.toml"), "format": "toml"}},
                "pending": {"file": {"path": PathBuf::from("/tmp").join(&name), "format": "toml"}}}),
        );
        let error = check_code_files_owned(&settings(vec![other])).unwrap_err();
        assert!(error.contains("belongs to user id 0"), "{error}");
    }

    fn access_with_codes(name: &str) -> (Access, PathBuf, String) {
        let dir = scratch(name);
        let file = dir.join("accounts.toml");
        let me = my_name();
        seed_password(&file, &me, "x");
        let totp = dir.join("totp.toml");
        let access =
            Access::new(&settings(vec![password_entry(&file), totp_entry(&totp)])).unwrap();
        (access, totp, me)
    }

    #[test]
    fn a_code_table_that_cannot_be_read_is_unknown_and_never_a_code_that_is_not_there() {
        let (access, table, me) = access_with_codes("unreadable");
        let begun = access.begin_factor(&me, mine(), false).unwrap();
        access
            .confirm_factor(&me, mine(), &totp_in(&begun.uri))
            .unwrap();
        let (standing, made) = access.factor_record(&me, mine());
        assert_eq!(standing, FactorStanding::Enrolled);
        assert!(made.is_some());
        let kept = std::fs::read(&table).unwrap();
        std::fs::write(&table, b"[[accounts\nnot a table").unwrap();
        assert_eq!(
            access.factor_record(&me, mine()),
            (FactorStanding::Unknown, None)
        );
        assert_eq!(access.factor_standing(&me, mine()), FactorStanding::Unknown);
        std::fs::write(&table, kept).unwrap();
        assert_eq!(
            access.factor_record(&me, mine()).0,
            FactorStanding::Enrolled
        );
    }

    /// The code an app shows now for the secret in a setup link.
    fn totp_in(link: &str) -> String {
        use hmac::{Hmac, KeyInit, Mac as _};
        let secret = link
            .split(['?', '&'])
            .find_map(|part| part.strip_prefix("secret="))
            .unwrap();
        let key = data_encoding::BASE32_NOPAD
            .decode(secret.as_bytes())
            .unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let mut mac = <Hmac<sha1::Sha1> as KeyInit>::new_from_slice(&key).unwrap();
        mac.update(&(now / 30).to_be_bytes());
        let digest = mac.finalize().into_bytes();
        let offset = usize::from(digest[19] & 0x0f);
        let number = u32::from_be_bytes([
            digest[offset] & 0x7f,
            digest[offset + 1],
            digest[offset + 2],
            digest[offset + 3],
        ]);
        format!("{:06}", number % 1_000_000)
    }

    #[test]
    fn marks_are_written_before_the_code_goes_and_two_commands_do_not_lose_each_others() {
        let (access, table, me) = access_with_codes("marks");
        let begun = access.begin_factor(&me, mine(), false).unwrap();
        access
            .confirm_factor(&me, mine(), &totp_in(&begun.uri))
            .unwrap();
        // A table that cannot be written: the removal fails, and the mark is already there.
        let key = format!("{}:{me}", mine());
        let mut lock = table.clone().into_os_string();
        lock.push(".lock");
        let lock = PathBuf::from(lock);
        let mut permissions = std::fs::metadata(&lock).unwrap().permissions();
        let writable = permissions.clone();
        std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o400);
        std::fs::set_permissions(&lock, permissions).unwrap();
        let outcome = access.reset_by_id(&key);
        std::fs::set_permissions(&lock, writable).unwrap();
        assert!(outcome.is_err(), "the removal could not be made");
        let listed = access.list_factors().unwrap();
        assert!(
            listed.iter().any(|entry| entry.id == key && entry.reset),
            "{listed:?}"
        );
        // Many commands at once each leave their mark.
        std::thread::scope(|scope| {
            for index in 0..16_u32 {
                let access = &access;
                scope.spawn(move || {
                    access
                        .reset_by_id(&format!("{}:person{index}", 5000 + index))
                        .unwrap();
                });
            }
        });
        let marked = access
            .list_factors()
            .unwrap()
            .into_iter()
            .filter(|entry| entry.reset && entry.id.starts_with(|c: char| c.is_ascii_digit()))
            .count();
        assert_eq!(marked, 17, "a mark was lost to another command's write");
    }

    #[test]
    fn an_account_renamed_or_made_again_under_the_same_user_id_is_reset_and_not_open_to_a_first_code()
     {
        let (access, _, me) = access_with_codes("renamed");
        let begun = access.begin_factor(&me, mine(), false).unwrap();
        access
            .confirm_factor(&me, mine(), &totp_in(&begun.uri))
            .unwrap();
        // The same user id under another name finds no code, and is not an account with none to set up.
        assert_eq!(
            access.factor_standing("renamed-account", mine()),
            FactorStanding::Reset
        );
        // A reset recorded for the old name carries to the new one, and an administrator's setup lifts both.
        access.reset_factor(&me, mine()).unwrap();
        assert_eq!(
            access.factor_standing("renamed-account", mine()),
            FactorStanding::Reset
        );
        access.clear_reset("renamed-account", mine()).unwrap();
        assert_eq!(
            access.factor_standing("renamed-account", mine()),
            FactorStanding::NotEnrolled
        );
        assert_eq!(
            access.factor_standing("elsewhere", mine() + 1),
            FactorStanding::NotEnrolled,
            "another user id is not this one"
        );
    }

    #[test]
    fn an_unlock_is_a_time_the_portal_reads_and_does_not_need_a_code_to_exist() {
        let (access, _, me) = access_with_codes("unlock");
        assert_eq!(access.unlock_stamp(&me, mine()), None);
        access.unlock(&me, mine()).unwrap();
        let first = access.unlock_stamp(&me, mine()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        access.unlock(&me, mine()).unwrap();
        assert!(access.unlock_stamp(&me, mine()).unwrap() > first);
        assert_eq!(access.unlock_stamp("someone-else", mine()), None);
    }

    #[test]
    fn an_account_with_a_code_is_given_a_new_one_only_when_its_current_code_was_proved() {
        let (access, _, me) = access_with_codes("rotate-proof");
        let begun = access.begin_factor(&me, mine(), false).unwrap();
        access
            .confirm_factor(&me, mine(), &totp_in(&begun.uri))
            .unwrap();
        let refused = access.begin_factor(&me, mine(), false);
        let Err(FactorError::Failed(why)) = refused else {
            panic!("a rotation with no proof was not refused: {refused:?}");
        };
        assert!(!why.is_empty() && !why.contains("secret="), "{why}");
        assert!(
            access
                .list_factors()
                .unwrap()
                .iter()
                .all(|each| !each.waiting)
        );
        let fresh = access.begin_factor(&me, mine(), true).unwrap();
        assert_ne!(fresh.uri, begun.uri);
        assert!(
            access
                .list_factors()
                .unwrap()
                .iter()
                .any(|each| each.waiting)
        );
    }

    #[test]
    fn an_unlock_leaves_a_code_an_administrator_disabled_disabled() {
        let (access, _, me) = access_with_codes("unlock-disabled");
        let begun = access.begin_factor(&me, mine(), false).unwrap();
        access
            .confirm_factor(&me, mine(), &totp_in(&begun.uri))
            .unwrap();
        let store = access.codes.as_ref().unwrap();
        store
            .accounts
            .put(AccountPut {
                name: format!("{}:{me}", mine()),
                disabled: Some(true),
                ..AccountPut::default()
            })
            .unwrap();
        assert_eq!(
            access.factor_standing(&me, mine()),
            FactorStanding::Disabled
        );
        access.unlock(&me, mine()).unwrap();
        assert_eq!(
            access.factor_standing(&me, mine()),
            FactorStanding::Disabled,
            "an unlock is not an enable"
        );
        // And an enabled one stays enabled.
        store
            .accounts
            .put(AccountPut {
                name: format!("{}:{me}", mine()),
                disabled: Some(false),
                ..AccountPut::default()
            })
            .unwrap();
        access.unlock(&me, mine()).unwrap();
        assert_eq!(
            access.factor_standing(&me, mine()),
            FactorStanding::Enrolled
        );
    }

    #[test]
    fn the_owner_check_looks_at_the_code_providers_files_and_no_others() {
        let dir = scratch("owner-scope");
        // A password table under a folder root owns, and a code table under ours: nothing to refuse.
        let name = format!("canoa-owner-scope-{}.toml", std::process::id());
        let password = entry(
            "coqui_access_password",
            None,
            &serde_json::json!({"source": {"file": {"path": PathBuf::from("/tmp").join(&name), "format": "toml"}}}),
        );
        let ours = settings(vec![password.clone(), totp_entry(&dir.join("totp.toml"))]);
        assert_eq!(check_code_files_owned(&ours), Ok(()));
        // A code provider under another id is found by its library's name, and its files are the ones that count.
        let mut codes = totp_entry(&PathBuf::from("/tmp").join(&name));
        codes.provider = "second_factor".to_owned();
        let error = check_code_files_owned(&settings(vec![password, codes])).unwrap_err();
        assert!(error.contains("belongs to user id 0"), "{error}");
    }
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
