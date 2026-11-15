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
use coqui_access::guatiao::{Alloc, Map, Value};
use coqui_access::{
    AccountPut, Accounts as AccountKind, Attempt as Credential, Authenticator, Directory, Lookup,
    Verdict, takes,
};
use coqui_access_signin::SignInRules;
use coqui_access_system::SystemAccounts;
use coqui_peer_tls::CertificateFields;

use super::settings::{Entry, Settings};
use super::signin::{
    Accounts, CertificateStatus, MAX_PASSWORD, Method, PinError, PinState, Presented, SignIn,
    Standing, valid_user,
};

/// The provider id of the operating system's accounts.
pub const SYSTEM: &str = "coqui_access_system";

/// The group a pinned certificate's record carries to name the user id its account had when it was pinned, so that an account
/// deleted and made again under the same name does not inherit the pin.
const UID_MARKER: &str = "uid:";

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

/// The portal's accounts: the authenticators of `Settings::authenticators`, in order, over the operating system's directory.
pub struct Access {
    /// Who the operating system admits (`min_uid`, `groups`): every method ends here.
    system: SystemAccounts,
    passwords: Vec<Link>,
    certificates: Vec<Link>,
    /// The header the proxy puts the verified-or-not client certificate in, lower case.
    header: String,
    pins: Option<PinStore>,
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
            _registry: Arc::new(registry),
        })
    }

    /// The account a provider's valid answer names, if the operating system admits it: its name and user id.
    fn resolve(&self, link: &Link, lookup: Lookup) -> SignIn {
        // A provider that names no account itself leaves it to a matcher over the credential's claims, which the portal does
        // not run: such a certificate is refused, and the log says what to set instead.
        if lookup.by == "claims" {
            tracing::warn!(
                "authenticators: {} answered by claims, which the portal has no matcher for; give it `account` picks that name an account",
                link.id
            );
            return SignIn::Refused;
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
        SignIn::Refused
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
        let Some(chain) = forwarded_chain(headers, &self.header) else {
            return SignIn::Refused;
        };
        // Pinned trust does not look at dates, and a certificate past them is not one to let in.
        if chain
            .first()
            .and_then(|leaf| leaf_fields(leaf))
            .is_none_or(|fields| outside_validity(&fields))
        {
            return SignIn::Refused;
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
        self.ask(&self.certificates, &credential, providers)
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
            SignIn::Refused
        );
        // Nothing, garbage, two headers, and a chain longer than the portal reads.
        assert_eq!(
            access.sign_in_certificate(&[], client(), &[]),
            SignIn::Refused
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
                    assert_eq!(signed, SignIn::Refused, "pin {other} after pin {index}");
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
                SignIn::Refused
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
            SignIn::Refused
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
            SignIn::Refused
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
            SignIn::Refused
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
}
