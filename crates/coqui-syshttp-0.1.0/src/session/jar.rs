//! Cookie and token jars, keyed by host: in memory, or in files under one root.
//!
//! The root is [`store_root`]: `SYSHTTP_HOME` (under the prefixes of [`HttpEnv`]), else the
//! platform's per-user state directory. Cookies go under `<root>/cookies/<host>.txt` (Netscape
//! format), tokens under `<root>/tokens/<host>.token`: the files Python syshttp reads and writes.
//!
//! Token values are secrets: they are [`Secret`]s and this module never logs or prints them. Files
//! are written `0600` (directories `0700`) and replaced whole, with LF line ends; a directory is
//! created at the first write, not when a jar is built.

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use coqui_proxy::Secret;

use super::cookies::{Cookie, load_netscape, save_netscape};
use super::env::HttpEnv;
use super::fileio::{read_text, write_private};
#[cfg(feature = "session-reqwest")]
use super::settings::CookieSettings;
use super::state::Cookies;
#[cfg(feature = "session-reqwest")]
use super::state::JarKey;

/// The setting naming the jar root.
pub const HOME: &str = "SYSHTTP_HOME";

const UNSAFE: [char; 9] = ['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

/// A jar file name for a host: characters no filesystem takes (the `:` of an IPv6 literal on
/// Windows, a `/`) become `_`.
#[must_use]
pub fn safe_name(key: &str) -> String {
    key.chars()
        .map(|character| {
            if UNSAFE.contains(&character) {
                '_'
            } else {
                character
            }
        })
        .collect()
}

/// The per-user state directory for jars when `SYSHTTP_HOME` is unset, a directory `syshttp`
/// under `%LOCALAPPDATA%` on Windows, `~/Library/Application Support` on macOS, and
/// `$XDG_STATE_HOME` (else `~/.local/state`) elsewhere.
#[must_use]
pub fn default_root(env: &HttpEnv) -> PathBuf {
    let home = std::env::home_dir().unwrap_or_default();
    let unprefixed = |name: &str| {
        env.raw(name)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    let base = if cfg!(windows) {
        unprefixed("LOCALAPPDATA").map_or_else(|| home.join("AppData").join("Local"), PathBuf::from)
    } else if cfg!(target_os = "macos") {
        home.join("Library").join("Application Support")
    } else {
        unprefixed("XDG_STATE_HOME")
            .map_or_else(|| home.join(".local").join("state"), PathBuf::from)
    };
    base.join("syshttp")
}

/// The jar root: `SYSHTTP_HOME` (a leading `~` is the home directory), else [`default_root`].
#[must_use]
pub fn store_root(env: &HttpEnv) -> PathBuf {
    match env.get(&[HOME]).ok().flatten() {
        Some(home) => expand_user(&home),
        None => default_root(env),
    }
}

fn expand_user(path: &str) -> PathBuf {
    let rest = match path {
        "~" => Some(""),
        _ => path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")),
    };
    match (rest, std::env::home_dir()) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(path),
    }
}

/// Cookies kept by key, one list per host (or one for every host).
pub trait CookieJar: fmt::Debug + Send + Sync {
    /// The cookies stored under `key`, `None` when there are none.
    fn get(&self, key: &str) -> Option<Vec<Cookie>>;
    /// Stores `cookies` under `key`, replacing what was there.
    ///
    /// # Errors
    ///
    /// What writing the store reports.
    fn set(&self, key: &str, cookies: &[Cookie]) -> io::Result<()>;
}

/// Tokens kept by key. A token is a secret.
pub trait TokenJar: fmt::Debug + Send + Sync {
    /// The token stored under `key`, `None` when there is none.
    fn get(&self, key: &str) -> Option<Secret>;
    /// Stores `token` under `key`, replacing what was there.
    ///
    /// # Errors
    ///
    /// What writing the store reports.
    fn set(&self, key: &str, token: &Secret) -> io::Result<()>;
}

/// The cookie state a jar keeps for `reqwest`, made when it is first used.
#[derive(Clone, Default)]
#[cfg_attr(not(feature = "session-reqwest"), allow(dead_code))]
struct Engine(Arc<Mutex<Option<Arc<Cookies>>>>);

impl fmt::Debug for Engine {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Engine")
    }
}

impl Engine {
    /// The state over `backing`, one entry per origin host.
    #[cfg(feature = "session-reqwest")]
    fn get(&self, backing: impl FnOnce() -> Arc<dyn CookieJar>) -> Option<Arc<Cookies>> {
        let mut slot = self.0.lock().ok()?;
        let engine = slot.get_or_insert_with(|| {
            Arc::new(Cookies::new(
                Some(backing()),
                JarKey::ByHost,
                super::cookies::HostAliases::default(),
                CookieSettings::default(),
            ))
        });
        Some(Arc::clone(engine))
    }
}

/// A cookie jar in memory. Clones share the cookies.
#[derive(Debug, Default, Clone)]
pub struct MemoryCookieJar {
    entries: Arc<Mutex<BTreeMap<String, Vec<Cookie>>>>,
    #[cfg_attr(not(feature = "session-reqwest"), allow(dead_code))]
    engine: Engine,
}

impl MemoryCookieJar {
    /// An empty jar.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl CookieJar for MemoryCookieJar {
    fn get(&self, key: &str) -> Option<Vec<Cookie>> {
        self.entries.lock().ok()?.get(key).cloned()
    }

    fn set(&self, key: &str, cookies: &[Cookie]) -> io::Result<()> {
        self.entries
            .lock()
            .map_err(|_| io::Error::other("the jar is poisoned"))?
            .insert(key.into(), cookies.to_vec());
        Ok(())
    }
}

/// A token jar in memory.
#[derive(Debug, Default)]
pub struct MemoryTokenJar {
    entries: Mutex<BTreeMap<String, Secret>>,
}

impl MemoryTokenJar {
    /// An empty jar.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl TokenJar for MemoryTokenJar {
    fn get(&self, key: &str) -> Option<Secret> {
        self.entries.lock().ok()?.get(key).cloned()
    }

    fn set(&self, key: &str, token: &Secret) -> io::Result<()> {
        self.entries
            .lock()
            .map_err(|_| io::Error::other("the jar is poisoned"))?
            .insert(key.into(), token.clone());
        Ok(())
    }
}

/// Makes a [`FileCookieJar`] or a [`FileTokenJar`].
#[derive(Debug, Default, Clone)]
#[must_use]
pub struct FileJarBuilder {
    root: Option<PathBuf>,
    env: Option<HttpEnv>,
}

impl FileJarBuilder {
    /// The directory the jar keeps its files under (default: [`store_root`]).
    pub fn root(mut self, root: impl Into<PathBuf>) -> Self {
        self.root = Some(root.into());
        self
    }

    /// Where `SYSHTTP_HOME` is read from (default: the process, through `SYSHTTP_PREFIX`).
    pub fn env(mut self, env: HttpEnv) -> Self {
        self.env = Some(env);
        self
    }

    fn root_dir(&self) -> PathBuf {
        match (&self.root, &self.env) {
            (Some(root), _) => root.clone(),
            (None, Some(env)) => store_root(env),
            (None, None) => store_root(&HttpEnv::process()),
        }
    }

    /// The cookie jar: Netscape files under `<root>/cookies`.
    #[must_use]
    pub fn build_cookies(self) -> FileCookieJar {
        FileCookieJar {
            dir: self.root_dir().join("cookies"),
            engine: Engine::default(),
        }
    }

    /// The token jar: files under `<root>/tokens`.
    #[must_use]
    pub fn build_tokens(self) -> FileTokenJar {
        FileTokenJar {
            dir: self.root_dir().join("tokens"),
        }
    }
}

/// A cookie jar of Netscape files, one per host, under `<root>/cookies/<host>.txt`.
#[derive(Debug, Clone)]
pub struct FileCookieJar {
    dir: PathBuf,
    #[cfg_attr(not(feature = "session-reqwest"), allow(dead_code))]
    engine: Engine,
}

impl FileCookieJar {
    /// Starts a jar; call [`FileJarBuilder::build_cookies`] on the result, or use
    /// [`FileCookieJar::new`].
    pub fn builder() -> FileJarBuilder {
        FileJarBuilder::default()
    }

    /// A jar under `root`.
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            dir: root.as_ref().join("cookies"),
            engine: Engine::default(),
        }
    }

    /// The file that holds `key`.
    #[must_use]
    pub fn path(&self, key: &str) -> PathBuf {
        self.dir.join(alloc::format!("{}.txt", safe_name(key)))
    }
}

impl CookieJar for FileCookieJar {
    fn get(&self, key: &str) -> Option<Vec<Cookie>> {
        let path = self.path(key);
        path.exists().then(|| load_netscape(&path))
    }

    fn set(&self, key: &str, cookies: &[Cookie]) -> io::Result<()> {
        save_netscape(cookies, &self.path(key))
    }
}

/// A token jar of files, one per host, under `<root>/tokens/<host>.token`.
#[derive(Debug, Clone)]
pub struct FileTokenJar {
    dir: PathBuf,
}

impl FileTokenJar {
    /// Starts a jar; call [`FileJarBuilder::build_tokens`] on the result, or use
    /// [`FileTokenJar::new`].
    pub fn builder() -> FileJarBuilder {
        FileJarBuilder::default()
    }

    /// A jar under `root`.
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            dir: root.as_ref().join("tokens"),
        }
    }

    /// The file that holds `key`.
    #[must_use]
    pub fn path(&self, key: &str) -> PathBuf {
        self.dir.join(alloc::format!("{}.token", safe_name(key)))
    }
}

impl TokenJar for FileTokenJar {
    fn get(&self, key: &str) -> Option<Secret> {
        let text = read_text(&self.path(key)).ok()?;
        let token = text.trim();
        (!token.is_empty()).then(|| Secret::new(token))
    }

    fn set(&self, key: &str, token: &Secret) -> io::Result<()> {
        write_private(
            &self.path(key),
            alloc::format!("{}\n", token.expose().trim()).as_bytes(),
        )
    }
}

#[cfg(feature = "session-reqwest")]
mod reqwest_store {
    use alloc::string::String;
    use alloc::sync::Arc;
    use std::time::SystemTime;

    use ::reqwest::header::HeaderValue;

    use super::{CookieJar, Engine, FileCookieJar, MemoryCookieJar};

    fn set(
        engine: &Engine,
        backing: impl FnOnce() -> Arc<dyn CookieJar>,
        headers: &mut dyn Iterator<Item = &HeaderValue>,
        url: &url::Url,
    ) {
        let Some(state) = engine.get(backing) else {
            return;
        };
        let values: alloc::vec::Vec<String> = headers
            .map(|value| {
                value
                    .as_bytes()
                    .iter()
                    .map(|byte| char::from(*byte))
                    .collect()
            })
            .collect();
        state.before(url);
        if !values.is_empty() {
            state.store(url, &values, SystemTime::now());
        }
    }

    fn get(
        engine: &Engine,
        backing: impl FnOnce() -> Arc<dyn CookieJar>,
        url: &url::Url,
    ) -> Option<HeaderValue> {
        let state = engine.get(backing)?;
        state.before(url);
        let secret = state.header(url, SystemTime::now())?;
        let mut value = HeaderValue::from_str(secret.expose()).ok()?;
        value.set_sensitive(true);
        Some(value)
    }

    impl ::reqwest::cookie::CookieStore for FileCookieJar {
        fn set_cookies(&self, headers: &mut dyn Iterator<Item = &HeaderValue>, url: &url::Url) {
            let backing = || -> Arc<dyn CookieJar> {
                Arc::new(Self {
                    dir: self.dir.clone(),
                    engine: Engine::default(),
                })
            };
            set(&self.engine, backing, headers, url);
        }

        fn cookies(&self, url: &url::Url) -> Option<HeaderValue> {
            let backing = || -> Arc<dyn CookieJar> {
                Arc::new(Self {
                    dir: self.dir.clone(),
                    engine: Engine::default(),
                })
            };
            get(&self.engine, backing, url)
        }
    }

    impl ::reqwest::cookie::CookieStore for MemoryCookieJar {
        fn set_cookies(&self, headers: &mut dyn Iterator<Item = &HeaderValue>, url: &url::Url) {
            let backing = || -> Arc<dyn CookieJar> {
                Arc::new(Self {
                    entries: Arc::clone(&self.entries),
                    engine: Engine::default(),
                })
            };
            set(&self.engine, backing, headers, url);
        }

        fn cookies(&self, url: &url::Url) -> Option<HeaderValue> {
            let backing = || -> Arc<dyn CookieJar> {
                Arc::new(Self {
                    entries: Arc::clone(&self.entries),
                    engine: Engine::default(),
                })
            };
            get(&self.engine, backing, url)
        }
    }
}
