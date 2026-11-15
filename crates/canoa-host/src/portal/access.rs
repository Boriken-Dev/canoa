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
use std::path::Path;

use coqui_access::guatiao::library::{ImportSearch, Registry};
use coqui_access::guatiao::{Alloc, Map, Value};
use coqui_access::{
    AccountPut, Accounts as AccountKind, Attempt as Credential, Authenticator, Directory, Lookup,
    Verdict, takes,
};
use coqui_access_system::SystemAccounts;
use coqui_peer_tls::CertificateFields;

use super::settings::{Entry, Settings};
use super::signin::{
    Accounts, CertificateStatus, MAX_PASSWORD, Method, PinError, PinState, Presented, SignIn,
    Standing, valid_user,
};

/// The provider id of the operating system's accounts.
pub const SYSTEM: &str = "coqui_access_system";

/// What a certificate attempt presents as the connection's peer: the proxy's socket has no address, and the providers that
/// record who asked want one source. A connection from anywhere else never makes a certificate attempt.
pub const PROXY_PEER: &str = "127.0.0.1";

/// The most certificates (leaf first) and bytes a forwarded header may carry.
const MAX_CHAIN: usize = 8;
const MAX_HEADER: usize = 16 * 1024;

coqui_access::guatiao::local_providers!(
    id = "canoa-portal",
    version = env!("CARGO_PKG_VERSION"),
    providers = [SystemAccounts]
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

fn value_of(map: &serde_json::Map<String, serde_json::Value>) -> Result<Value, String> {
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

fn open_library(registry: &mut Registry, library: &Path) -> Result<(), String> {
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
        let mut registry = Registry::new("canoa-portal", env!("CARGO_PKG_VERSION"));
        registry
            .register_local("canoa-portal", library)
            .map_err(|error| error.to_string())?;
        let built_in = |id: &str| id == SYSTEM;
        for entry in &settings.authenticators {
            match (&entry.library, built_in(&entry.provider)) {
                (Some(path), false) => open_library(&mut registry, path)
                    .map_err(|why| format!("authenticators: {}: {why}", entry.provider))?,
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
        let named = if link.id == SYSTEM {
            self.system.lookup(lookup.clone())
        } else if link.directory.is_none() && lookup.by == "name" {
            // A provider with no directory of its own names the account outright.
            return self.admit(&lookup.value);
        } else if let Some(directory) = &link.directory {
            directory.lookup(lookup.clone())
        } else {
            return SignIn::Refused;
        };
        match named {
            Ok(Some(person)) => self.admit(&person.name),
            // A certificate a CA signed names its account by a field of the certificate, which no table of the provider holds.
            Ok(None) if link.takes_request && lookup.by == "name" => self.admit(&lookup.value),
            Ok(None) => SignIn::Refused,
            Err(error) => SignIn::Failed(error.to_string()),
        }
    }

    /// The certificate a request carries: the leaf's subject and fingerprint, when the header holds readable certificates.
    fn presented(&self, headers: &[(String, String)]) -> Option<Presented> {
        let chain = forwarded_chain(headers, &self.header)?;
        let fields = CertificateFields::parse(chain.first()?).ok()?;
        let map = fields.to_map();
        let first = |key: &str| map.get(key).and_then(|values| values.first()).cloned();
        Some(Presented {
            subject: first("subject").unwrap_or_default(),
            fingerprint: first("fingerprint")?,
        })
    }

    /// `name` as the operating system admits it: under `min_uid` and `groups`, spelled as the system spells it.
    fn admit(&self, name: &str) -> SignIn {
        if !valid_user(name) {
            return SignIn::Refused;
        }
        let lookup = Lookup {
            by: "name".to_owned(),
            value: name.to_owned(),
        };
        match self.system.lookup(lookup) {
            Ok(Some(person)) if valid_user(&person.name) => match person.id.parse() {
                Ok(uid) => SignIn::Admitted {
                    user: person.name,
                    uid,
                },
                Err(_) => SignIn::Refused,
            },
            Ok(_) => SignIn::Refused,
            Err(error) => SignIn::Failed(error.to_string()),
        }
    }

    /// Asks `links` in order: the first that does not pass the credential on decides.
    fn ask(&self, links: &[Link], credential: &Credential) -> SignIn {
        for link in links {
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

    fn sign_in(&self, user: &str, password: &str) -> SignIn {
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
        self.ask(&self.passwords, &credential)
    }

    fn sign_in_certificate(&self, headers: &[(String, String)]) -> SignIn {
        if self.certificates.is_empty() {
            return SignIn::Refused;
        }
        let Some(chain) = forwarded_chain(headers, &self.header) else {
            return SignIn::Refused;
        };
        // The proxy proved the browser holds the leaf's key, and the portal believes the proxy only on its own socket; whether
        // the certificate is trusted is for the provider to say.
        let credential = Credential {
            certificates: Some(chain.into_iter().map(Into::into).collect()),
            secure: Some(true),
            peer: Some(PROXY_PEER.to_owned()),
            ..Credential::default()
        };
        self.ask(&self.certificates, &credential)
    }

    fn certificate_status(
        &self,
        user: &str,
        headers: &[(String, String)],
    ) -> Option<CertificateStatus> {
        let pins = self.pins.as_ref()?;
        let presented = self.presented(headers).map(|presented| {
            let state = match pins.directory.lookup(Lookup {
                by: "certificate".to_owned(),
                value: presented.fingerprint.clone(),
            }) {
                Ok(Some(person)) if person.name == user => PinState::Yours,
                Ok(Some(_)) => PinState::Another,
                _ => PinState::Unpinned,
            };
            (presented, state)
        });
        let pinned = pins.accounts.accounts().is_ok_and(|all| {
            all.iter()
                .any(|account| account.name == user && account.disabled != Some(true))
        });
        Some(CertificateStatus { presented, pinned })
    }

    fn pin(&self, user: &str, headers: &[(String, String)]) -> Result<(), PinError> {
        let pins = self.pins.as_ref().ok_or(PinError::Unsupported)?;
        let presented = self.presented(headers).ok_or(PinError::NoCertificate)?;
        if !valid_user(user) {
            return Err(PinError::Failed("not an account name".to_owned()));
        }
        match pins.directory.lookup(Lookup {
            by: "certificate".to_owned(),
            value: presented.fingerprint.clone(),
        }) {
            Ok(Some(person)) if person.name != user => return Err(PinError::Taken),
            Err(error) => return Err(PinError::Failed(error.to_string())),
            _ => {}
        }
        pins.accounts
            .put(AccountPut {
                name: user.to_owned(),
                secret: Some(presented.fingerprint),
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
        if !all.iter().any(|account| account.name == user) {
            return Ok(false);
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
            })
        };
        // A directory that fails once may answer the next time; only a definite answer ends a session.
        let found = ask().or_else(|_| {
            std::thread::sleep(core::time::Duration::from_millis(200));
            ask()
        });
        match found {
            Ok(Some(person)) if person.name == user => Standing::Present,
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
            ..Settings::default()
        }
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
        assert_eq!(
            access.sign_in(&me, "file-pw"),
            SignIn::Admitted {
                user: me.clone(),
                uid: mine()
            }
        );
        assert_eq!(access.sign_in(&me, "wrong"), SignIn::Refused);
        assert_eq!(access.sign_in("nobody-here", "file-pw"), SignIn::Refused);
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
            Access::new(&outside).unwrap().sign_in(&me, "file-pw"),
            SignIn::Refused
        );
        let mut above = settings(vec![password_entry(&file)]);
        above.min_uid = mine() + 1;
        assert_eq!(
            Access::new(&above).unwrap().sign_in(&me, "file-pw"),
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
            both.sign_in(&me, "pw-one"),
            SignIn::Admitted { .. }
        ));
        assert_eq!(both.sign_in(&me, "pw-two"), SignIn::Refused);
        // Unknown: the system provider, limited to a group this account is not in, says it is not its to judge.
        let narrow = entry(
            SYSTEM,
            None,
            &serde_json::json!({"groups": ["canoa-no-such-group"]}),
        );
        let passed_on = Access::new(&settings(vec![narrow, password_entry(&second)])).unwrap();
        assert!(matches!(
            passed_on.sign_in(&me, "pw-two"),
            SignIn::Admitted { .. }
        ));
        std::fs::remove_dir_all(dir).unwrap();
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
        assert_eq!(
            access.sign_in_certificate(&header(&b64)),
            SignIn::Admitted {
                user: me.clone(),
                uid: mine()
            }
        );
        // A certificate nobody pinned, however well formed.
        let (other, _) = certificate();
        let other = data_encoding::BASE64.encode(&other);
        assert_eq!(access.sign_in_certificate(&header(&other)), SignIn::Refused);
        // Nothing, garbage, two headers, and a chain longer than the portal reads.
        assert_eq!(access.sign_in_certificate(&[]), SignIn::Refused);
        assert_eq!(
            access.sign_in_certificate(&header("not base64!")),
            SignIn::Refused
        );
        let twice = vec![
            ("x-client-cert".to_owned(), b64.clone()),
            ("x-client-cert".to_owned(), b64.clone()),
        ];
        assert_eq!(access.sign_in_certificate(&twice), SignIn::Refused);
        let long = [b64.as_str(); 9].join(",");
        assert_eq!(access.sign_in_certificate(&header(&long)), SignIn::Refused);
        // A password goes to no certificate provider.
        assert_eq!(access.sign_in(&me, "anything"), SignIn::Refused);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_pinned_certificate_of_an_account_outside_the_groups_is_refused() {
        let dir = scratch("pinned-groups");
        let (access, _, der) = pinned(&dir.join("peers.toml"), &["canoa-no-such-group"]);
        assert_eq!(
            access.sign_in_certificate(&header(&data_encoding::BASE64.encode(&der))),
            SignIn::Refused
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn enrolling(file: &Path) -> Access {
        let mut entry = mtls_entry(file);
        entry.enrol = true;
        Access::new(&settings(vec![entry])).unwrap()
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
        assert_eq!(access.sign_in_certificate(&mine_header), SignIn::Refused);
        let status = access.certificate_status(&me, &mine_header).unwrap();
        let (presented, state) = status.presented.unwrap();
        assert_eq!(
            (presented.fingerprint.as_str(), state),
            (fingerprint.as_str(), PinState::Unpinned)
        );
        assert!(!status.pinned);
        // Pinned, the certificate alone signs the account in.
        access.pin(&me, &mine_header).unwrap();
        assert_eq!(
            access.sign_in_certificate(&mine_header),
            SignIn::Admitted {
                user: me.clone(),
                uid: mine()
            }
        );
        let status = access.certificate_status(&me, &mine_header).unwrap();
        assert_eq!(status.presented.unwrap().1, PinState::Yours);
        assert!(status.pinned);
        // One pinned to another account is that account's, and cannot be taken.
        assert_eq!(
            access
                .certificate_status(&me, &other_header)
                .unwrap()
                .presented
                .unwrap()
                .1,
            PinState::Another
        );
        assert_eq!(access.pin(&me, &other_header), Err(PinError::Taken));
        assert_eq!(access.pin(&me, &[]), Err(PinError::NoCertificate));
        assert_eq!(
            access.pin(&me, &header("not base64!")),
            Err(PinError::NoCertificate)
        );
        // Removed, it signs no one in, and removing again finds nothing.
        assert_eq!(access.unpin(&me), Ok(true));
        assert_eq!(access.sign_in_certificate(&mine_header), SignIn::Refused);
        assert_eq!(access.unpin(&me), Ok(false));
        assert!(!access.certificate_status(&me, &mine_header).unwrap().pinned);
        // The other account's pin was never touched.
        assert_eq!(
            access
                .certificate_status(&me, &other_header)
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
        assert_eq!(access.certificate_status(&me, &h), None);
        assert_eq!(access.pin(&me, &h), Err(PinError::Unsupported));
        assert_eq!(access.unpin(&me), Err(PinError::Unsupported));
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
