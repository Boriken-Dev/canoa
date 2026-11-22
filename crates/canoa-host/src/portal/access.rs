//! The portal's sign-in providers: an ordered list of coqui-access authenticators, each credential going to the provider that
//! claims it, and every method ending at an account of the operating system.
//!
//! A name and a password go, in list order, to the providers that take a password: `Unknown` passes the credential on and
//! `Invalid` ends the try. A certificate the trusted proxy forwarded (DER in base64, in one header) goes to the providers that
//! take a request, as the chain a transport would have verified the key of; they decide whether to trust it (pinned, or signed
//! by a CA) and which account it is. Whoever a provider names is then looked up in the operating system's own directory under the portal's `min_uid` and `groups`, so a
//! method adds a way in and never a person.

mod certs;
mod codes;

#[cfg(test)]
mod tests;

use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use core::net::IpAddr;
use std::path::{Path, PathBuf};

use coqui_access::guatiao::library::{ImportSearch, Registry};
use coqui_access::guatiao::{Alloc, Map, Value};
use coqui_access::{
    Accounts as AccountKind, Attempt as Credential, Authenticator, Directory, Lookup, Verdict,
    takes,
};
use coqui_access_kit::matcher::{Match, Matcher};
use coqui_access_signin::SignInRules;
use coqui_access_system::SystemAccounts;

use certs::{
    PinStore, UID_MARKER, build_matcher, forwarded_chain, leaf_fields, outside_validity, pin_store,
};
use codes::{CodeStore, KeyError, beside, code_accounts};
pub use codes::{FactorEntry, check_code_files_owned};

use super::settings::{Entry, Settings};
use super::signin::{
    Accounts, CertificateStatus, CodeCheck, Enrolment, FactorError, FactorStanding, MAX_PASSWORD,
    Method, PinError, SignIn, Standing, valid_user,
};

#[cfg(test)]
use certs::{MAX_CHAIN, MAX_HEADER, marker_fits};
#[cfg(test)]
use codes::{MAX_CODE, code_key, keeps_codes, split_key};

/// The provider id of the operating system's accounts.
pub const SYSTEM: &str = "coqui_access_system";

/// The provider that reads a certificate a front proxy forwarded and answers by its claims. The portal refuses it: it trusts what
/// the proxy says, and a proxy that ignores certificate errors makes every claim a stranger's.
const PROXY: &str = "coqui_access_proxy";

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
        self.codes
            .as_ref()
            .ok_or_else(|| "this portal keeps no one-time codes".to_owned())?
            .list_factors()
    }

    /// Removes the code whose record is called `key` (`1000:alice`, as the listing shows it) and marks it reset: for an account
    /// that is gone, whose name no command can give.
    ///
    /// # Errors
    ///
    /// A message for a key that is not one, a portal that keeps no codes or a store that cannot be written.
    pub fn reset_by_id(&self, key: &str) -> Result<bool, String> {
        if codes::split_key(key).is_none() {
            return Err("not a code id: it is a user id, a colon and a user name".to_owned());
        }
        self.reset_key(key).map_err(|error| match error {
            KeyError::Unsupported => "this portal keeps no one-time codes".to_owned(),
            KeyError::Failed(why) => why,
        })
    }

    fn reset_key(&self, key: &str) -> Result<bool, KeyError> {
        self.codes
            .as_ref()
            .ok_or(KeyError::Unsupported)?
            .reset_key(key)
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
            marks.retain(|mark| codes::split_key(mark).is_none_or(|(id, _)| id != uid))
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
        self.codes
            .as_ref()
            .ok_or_else(|| "this portal keeps no one-time codes".to_owned())?
            .unlock(user, uid)
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
        Some(self.pins.as_ref()?.status(&self.header, user, uid, headers))
    }

    fn pin(&self, user: &str, uid: u32, headers: &[(String, String)]) -> Result<(), PinError> {
        self.pins
            .as_ref()
            .ok_or(PinError::Unsupported)?
            .pin(&self.header, user, uid, headers)
    }

    fn unpin(&self, user: &str) -> Result<bool, PinError> {
        self.pins.as_ref().ok_or(PinError::Unsupported)?.unpin(user)
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
        store.check_code(user, uid, method, code, providers)
    }

    fn factor_record(&self, user: &str, uid: u32) -> (FactorStanding, Option<i64>) {
        let Some(store) = &self.codes else {
            return (FactorStanding::Unsupported, None);
        };
        store.factor_record(user, uid)
    }

    fn unlock_stamp(&self, user: &str, uid: u32) -> Option<i64> {
        self.codes
            .as_ref()?
            .unlock_stamp(&codes::code_key(user, uid))
    }

    fn begin_factor(
        &self,
        user: &str,
        uid: u32,
        proved_current: bool,
    ) -> Result<Enrolment, FactorError> {
        self.codes
            .as_ref()
            .ok_or(FactorError::Unsupported)?
            .begin_factor(user, uid, proved_current)
    }

    fn confirm_factor(&self, user: &str, uid: u32, code: &str) -> Result<String, FactorError> {
        self.codes
            .as_ref()
            .ok_or(FactorError::Unsupported)?
            .confirm_factor(user, uid, code)
    }

    fn reset_factor(&self, user: &str, uid: u32) -> Result<bool, FactorError> {
        self.reset_key(&codes::code_key(user, uid))
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
