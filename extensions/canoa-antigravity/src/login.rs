//! Antigravity's stored login: the settings it is read with, which are the account provider's and the usage reader's own
//! configuration (the entry's `account_settings`), and the one reader kept per set of settings in this process, so a token it
//! refreshed and the OAuth client that refreshed it are remembered from one reading to the next.

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::time::Duration;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, PoisonError};

use coqui_acp_api::guatiao::{FromValue, Schema, ToValue};
use coqui_acp_harness::antigravity::{self, AgyLogin, ClientSource, Keyring, LoginSettings};

/// How Antigravity's account provider and its usage reader are configured: the program, how its sign-in is run, and where and
/// how its stored login is read. Every address is Google's own by default and every limit has a default and no floor.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
pub struct AgySettings {
    /// The program it signs in and renews with, and whose strings hold the OAuth client a refresh uses: a path, or a name
    /// looked up on the `PATH`. `agy` when absent.
    pub executable: Option<String>,
    /// Where `agy` is looked for by name when `executable` names none: a `PATH`, which the host fills in with the person's own
    /// followed by the folders of the programs its Store installed. The process's own `PATH` when absent.
    pub search_path: Option<String>,
    /// How often, in milliseconds, a watch looks at the file an account keeps its login in.
    #[map(default = 30_000)]
    pub renew_poll_ms: u64,
    /// The longest, in milliseconds, a command of the program's own (a renewal) may run.
    #[map(default = 60_000)]
    pub command_timeout_ms: u64,
    /// The arguments that run the program's own sign-in.
    #[map(default = login_args())]
    pub login_args: Vec<String>,
    /// How often, in milliseconds, the stored login is looked at while the sign-in runs.
    #[map(default = 1_000)]
    pub login_poll_ms: u64,
    /// The hosts a page the sign-in prints may be on to be offered to the person; empty offers any page that is not this
    /// computer's.
    #[map(default = login_hosts())]
    pub login_hosts: Vec<String>,
    /// The home whose `.gemini` folder holds the login files. `USERPROFILE` on Windows, else `HOME`, when absent.
    pub home: Option<String>,
    /// The `.gemini` folder itself, in place of `<home>/.gemini`.
    pub gemini_dir: Option<String>,
    /// Whether the operating system's keyring is read for the login (Windows' Credential Manager, the Keychain, the Secret
    /// Service); the files are read either way.
    #[map(default = true)]
    pub keyring: bool,
    /// How long, in milliseconds, the keyring may take to answer before the files are read instead.
    #[map(default = 5_000)]
    pub keyring_timeout_ms: u64,
    /// The program whose strings hold the OAuth client an expired token is refreshed with, in place of `executable`.
    pub client_program: Option<String>,
    /// Google's token endpoint.
    #[map(default = token_url())]
    pub token_url: String,
    /// Google's userinfo endpoint.
    #[map(default = userinfo_url())]
    pub userinfo_url: String,
    /// The Cloud Code service, whatever the sign-in; when absent, `CLOUD_CODE_URL`, else the sign-in's own.
    pub service: Option<String>,
    /// The Cloud Code service of a personal account.
    #[map(default = consumer_service())]
    pub consumer_service: String,
    /// The Cloud Code service of a Google Cloud sign-in.
    #[map(default = gcp_service())]
    pub gcp_service: String,
    /// The `User-Agent` of each request.
    #[map(default = user_agent())]
    pub user_agent: String,
    /// How long, in milliseconds, each request may take, connection included.
    #[map(default = 15_000)]
    pub timeout_ms: u64,
    /// The most of an answer's body that is read, in bytes; an answer past it is malformed.
    #[map(default = 4_194_304)]
    pub max_answer_bytes: u64,
}

fn login_args() -> Vec<String> {
    alloc::vec!["acp".into(), "--login".into()]
}

fn login_hosts() -> Vec<String> {
    alloc::vec!["accounts.google.com".into()]
}

fn token_url() -> String {
    antigravity::TOKEN_URL.into()
}

fn userinfo_url() -> String {
    antigravity::USERINFO_URL.into()
}

fn consumer_service() -> String {
    antigravity::CONSUMER_SERVICE.into()
}

fn gcp_service() -> String {
    antigravity::GCP_SERVICE.into()
}

fn user_agent() -> String {
    "antigravity".into()
}

impl Default for AgySettings {
    fn default() -> Self {
        Self {
            executable: None,
            search_path: None,
            renew_poll_ms: 30_000,
            command_timeout_ms: 60_000,
            login_args: login_args(),
            login_poll_ms: 1_000,
            login_hosts: login_hosts(),
            home: None,
            gemini_dir: None,
            keyring: true,
            keyring_timeout_ms: 5_000,
            client_program: None,
            token_url: token_url(),
            userinfo_url: userinfo_url(),
            service: None,
            consumer_service: consumer_service(),
            gcp_service: gcp_service(),
            user_agent: user_agent(),
            timeout_ms: 15_000,
            max_answer_bytes: 4_194_304,
        }
    }
}

impl AgySettings {
    /// The program the sign-in runs: `executable`, else `agy` where the search path finds it, else its bare name for the system
    /// to look up.
    pub(crate) fn program(&self) -> String {
        self.executable
            .clone()
            .filter(|program| !program.is_empty())
            .or_else(|| self.found().map(|path| path.to_string_lossy().into_owned()))
            .unwrap_or_else(|| crate::PRESET.program.into())
    }

    /// `agy` as the search path the host gave finds it, when it gave one.
    fn found(&self) -> Option<PathBuf> {
        let search = self
            .search_path
            .as_deref()
            .filter(|path| !path.is_empty())?;
        canoa::search::find_in(
            search.as_ref(),
            &std::env::var("PATHEXT").unwrap_or_default(),
            crate::PRESET.program,
        )
    }

    /// How the stored login is read with these settings.
    fn reading(&self) -> LoginSettings {
        let mut settings = LoginSettings::new()
            .keyring(if self.keyring {
                Keyring::Os
            } else {
                Keyring::Off
            })
            .keyring_timeout(Duration::from_millis(self.keyring_timeout_ms))
            .token_url(self.token_url.as_str())
            .userinfo_url(self.userinfo_url.as_str())
            .consumer_service(self.consumer_service.as_str())
            .gcp_service(self.gcp_service.as_str())
            .user_agent(self.user_agent.as_str())
            .timeout(Duration::from_millis(self.timeout_ms))
            .body(usize::try_from(self.max_answer_bytes).unwrap_or(usize::MAX));
        if let Some(home) = &self.home {
            settings = settings.home(home);
        }
        if let Some(dir) = &self.gemini_dir {
            settings = settings.gemini_dir(dir);
        }
        if let Some(service) = &self.service {
            settings = settings.service(service.as_str());
        }
        let client = self
            .client_program
            .as_ref()
            .or(self.executable.as_ref())
            .filter(|program| !program.is_empty());
        settings.client(match client {
            Some(program) => ClientSource::Program(PathBuf::from(program)),
            None => self
                .found()
                .map_or(ClientSource::Find, ClientSource::Program),
        })
    }

    /// What tells one reader's settings from another's: every field the reading uses.
    fn key(&self) -> String {
        alloc::format!(
            "{:?}",
            (
                (&self.executable, &self.search_path),
                &self.home,
                &self.gemini_dir,
                self.keyring,
                self.keyring_timeout_ms,
                &self.client_program,
                (&self.token_url, &self.userinfo_url, &self.service),
                (&self.consumer_service, &self.gcp_service, &self.user_agent),
                (self.timeout_ms, self.max_answer_bytes),
            )
        )
    }
}

/// The readers of this process, one for each set of settings.
static READERS: Mutex<Vec<(String, Arc<Mutex<AgyLogin>>)>> = Mutex::new(Vec::new());

/// The reader kept in this process for `settings`, made the first time they are asked for.
pub(crate) fn reader(settings: &AgySettings) -> Arc<Mutex<AgyLogin>> {
    let key = settings.key();
    let mut readers = lock(&READERS);
    if let Some((_, reader)) = readers.iter().find(|(each, _)| *each == key) {
        return Arc::clone(reader);
    }
    let reader = Arc::new(Mutex::new(AgyLogin::new(settings.reading())));
    readers.push((key, Arc::clone(&reader)));
    reader
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use coqui_acp_api::guatiao::Value;

    #[test]
    fn an_empty_configuration_is_the_defaults_and_each_field_is_its_own() {
        let parsed = AgySettings::from_value(&Value::from(coqui_acp_api::guatiao::Map::new()))
            .expect("the defaults parse");
        assert_eq!(parsed, AgySettings::default());
        assert_eq!(parsed.login_args, ["acp", "--login"]);
        assert!(parsed.keyring);
        assert_eq!(parsed.token_url, antigravity::TOKEN_URL);
    }

    #[test]
    fn agy_is_found_on_the_search_path_the_host_gave_and_an_explicit_program_wins() {
        let folder =
            std::env::temp_dir().join(alloc::format!("canoa-agy-search-{}", std::process::id()));
        std::fs::create_dir_all(&folder).expect("a folder");
        let stand_in = folder.join(alloc::format!("agy{}", std::env::consts::EXE_SUFFIX));
        std::fs::write(&stand_in, b"stand-in").expect("written");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&stand_in, std::fs::Permissions::from_mode(0o755))
                .expect("runnable");
        }
        let searched = AgySettings {
            search_path: Some(folder.to_string_lossy().into_owned()),
            ..AgySettings::default()
        };
        // The control: no search path, and nothing in this process's own is `agy` here.
        assert_eq!(AgySettings::default().found(), None);
        assert_eq!(searched.found().as_deref(), Some(stand_in.as_path()));
        assert_eq!(searched.program(), stand_in.to_string_lossy());
        let explicit = AgySettings {
            executable: Some("/opt/agy".into()),
            ..searched
        };
        assert_eq!(explicit.program(), "/opt/agy");
        std::fs::remove_dir_all(&folder).expect("removed");
    }

    #[test]
    fn one_reader_is_kept_for_the_same_settings_and_another_for_others() {
        let first = AgySettings {
            home: Some("first-home".into()),
            keyring: false,
            ..AgySettings::default()
        };
        let other = AgySettings {
            home: Some("other-home".into()),
            ..first.clone()
        };
        assert!(Arc::ptr_eq(&reader(&first), &reader(&first.clone())));
        assert!(!Arc::ptr_eq(&reader(&first), &reader(&other)));
        // How the sign-in runs is no part of how the login is read.
        let polled = AgySettings {
            login_poll_ms: 5,
            ..first.clone()
        };
        assert!(Arc::ptr_eq(&reader(&first), &reader(&polled)));
    }
}
