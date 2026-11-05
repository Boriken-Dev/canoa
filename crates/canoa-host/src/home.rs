//! The directory of a host's identity and trust store, and the lock that keeps two hosts off it. The directory's layout is
//! [`coqui_agentic_peer::home`]'s; this names any directory rather than only the one this user's environment gives.
//!
//! The lock is the operating system's, on `host.lock`, so a holder that dies gives it up however it died and a process id
//! used again by something else proves nothing. Whoever holds it writes `host.holder` beside it: Windows refuses to read a
//! file another process has locked, and that file is only what a second host says, never what decides.
//!
//! Dropping a [`HomeLock`] gives it up; `host.lock` itself stays, since removing a file another host may be locking would
//! let two hold "the" lock at once.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;
use core::time::Duration;
use std::fs::{self, File, TryLockError};
use std::io::{self, Read as _, Write as _};
use std::path::{Path, PathBuf};

use coqui_agentic_peer::Error;
use coqui_agentic_peer::auth::{Identity, Method};
use coqui_agentic_peer::home::{self, METHOD_FILE, identity_file};
use coqui_peer_webcrypto::TrustStore;
use serde_json::{Value, json};

/// The file the operating system's lock is taken on.
const LOCK_FILE: &str = "host.lock";
/// The file saying who holds the lock.
const HOLDER_FILE: &str = "host.holder";
/// The most of [`HOLDER_FILE`] read; a holder's name is short.
const MAX_HOLDER: u64 = 4096;
/// The longest holder's name kept.
const MAX_NAME: usize = 64;
/// How long a host that found the lock taken waits for its holder to have said who it is.
const HOLDER_WAIT: Duration = Duration::from_millis(500);

/// A host's directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Home {
    dir: PathBuf,
}

/// Who holds a home.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Holder {
    /// What it calls itself: `canoa`, or `CanoaUI`.
    pub name: String,
    /// Its process id.
    pub pid: u32,
}

/// Why a home could not be locked.
#[derive(Debug)]
#[non_exhaustive]
pub enum LockError {
    /// Another host holds it; `holder` is `None` while it has not yet said who it is.
    Held {
        /// The home.
        dir: PathBuf,
        /// Who.
        holder: Option<Holder>,
    },
    /// The directory or the lock's file could not be used.
    Io {
        /// The home.
        dir: PathBuf,
        /// Why.
        source: io::Error,
    },
}

impl fmt::Display for LockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Held {
                dir,
                holder: Some(holder),
            } => write!(
                f,
                "{} (process {}) already hosts this home, {}: stop it, or give this one another COQUI_AGENTIC_PEER_HOME",
                holder.name,
                holder.pid,
                dir.display()
            ),
            Self::Held { dir, holder: None } => write!(
                f,
                "another host already hosts this home, {}: stop it, or give this one another COQUI_AGENTIC_PEER_HOME",
                dir.display()
            ),
            Self::Io { dir, source } => {
                write!(f, "the home {} cannot be locked: {source}", dir.display())
            }
        }
    }
}

impl core::error::Error for LockError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Held { .. } => None,
        }
    }
}

/// A home locked for one host; dropping it gives the home up.
#[derive(Debug)]
pub struct HomeLock {
    /// Held open for the lock, which ends when it closes.
    _file: File,
    holder_file: PathBuf,
}

impl Drop for HomeLock {
    fn drop(&mut self) {
        // Before the lock ends, so no later holder's file is removed.
        let _ = fs::remove_file(&self.holder_file);
    }
}

impl Home {
    /// The home at `dir`.
    #[must_use]
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// This user's home: `COQUI_AGENTIC_PEER_HOME`, else the platform's place.
    ///
    /// # Errors
    ///
    /// [`Error::Identity`] when no directory can be worked out for this user.
    pub fn of_user() -> Result<Self, Error> {
        home::dir()
            .map(Self::at)
            .ok_or_else(|| Error::Identity("no home directory for this user".to_owned()))
    }

    /// Its directory.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The identity of `method`, made when absent; with no method, the one the home records, else the one it holds, else a
    /// new one of the default method. It then records that method, as [`coqui_agentic_peer::home`] does for its own.
    ///
    /// # Errors
    ///
    /// [`Error::Identity`] when it cannot be read or written, when the recorded method names none, or when no method is
    /// named and the home holds identities of both.
    pub fn identity(&self, method: Option<Method>) -> Result<Identity, Error> {
        let chosen = self.chosen()?;
        let method = match (method, chosen) {
            (Some(method), _) | (None, Some(method)) => method,
            (None, None) => {
                let held: Vec<Method> = Method::ALL
                    .into_iter()
                    .filter(|method| self.dir.join(identity_file(*method)).exists())
                    .collect();
                match held.as_slice() {
                    [] => Method::DEFAULT,
                    [only] => *only,
                    _ => {
                        return Err(Error::Identity(format!(
                            "this home has an identity of each method: name the one to use, or write it in {METHOD_FILE}"
                        )));
                    }
                }
            }
        };
        let identity = Identity::open_or_create(&self.dir.join(identity_file(method)), method)?;
        if chosen != Some(method) {
            fs::create_dir_all(&self.dir)
                .and_then(|()| fs::write(self.dir.join(METHOD_FILE), format!("{method}\n")))
                .map_err(|error| {
                    Error::Identity(format!("{METHOD_FILE} could not be written: {error}"))
                })?;
        }
        Ok(identity)
    }

    fn chosen(&self) -> Result<Option<Method>, Error> {
        match fs::read_to_string(self.dir.join(METHOD_FILE)) {
            Ok(text) => text.trim().parse().map(Some).map_err(|_| {
                Error::Identity(format!(
                    "{METHOD_FILE} in this home names no method: write noise or webcrypto"
                ))
            }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(Error::Identity(format!(
                "{METHOD_FILE} could not be read: {error}"
            ))),
        }
    }

    /// The trust store.
    ///
    /// # Errors
    ///
    /// [`Error::Identity`] when it cannot be opened.
    pub fn store(&self) -> Result<TrustStore, Error> {
        home::store(Some(&self.dir.join("peers")))
    }

    /// Takes this home for one host, calling it `name` to any other that finds it held. A lock a dead process held is free,
    /// however that process ended.
    ///
    /// # Errors
    ///
    /// [`LockError::Held`] when another host holds it, in this process or another; [`LockError::Io`] when the directory or
    /// the file cannot be used.
    pub fn lock(&self, name: &str) -> Result<HomeLock, LockError> {
        let failed = |source| LockError::Io {
            dir: self.dir.clone(),
            source,
        };
        fs::create_dir_all(&self.dir).map_err(failed)?;
        let file = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.dir.join(LOCK_FILE))
            .map_err(failed)?;
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                return Err(LockError::Held {
                    dir: self.dir.clone(),
                    holder: self.holder_when_said(),
                });
            }
            Err(TryLockError::Error(source)) => return Err(failed(source)),
        }
        let holder_file = self.dir.join(HOLDER_FILE);
        // Only what another host says depends on it; a name left by a dead holder must not outlive a failed write.
        if write_holder(&holder_file, name).is_err() {
            let _ = fs::remove_file(&holder_file);
        }
        Ok(HomeLock {
            _file: file,
            holder_file,
        })
    }

    /// Who holds this home, by what it wrote; `None` while nobody says.
    #[must_use]
    pub fn holder(&self) -> Option<Holder> {
        read_holder(&self.dir.join(HOLDER_FILE))
    }

    /// The holder, given a moment to write: it writes just after it takes the lock.
    fn holder_when_said(&self) -> Option<Holder> {
        let until = std::time::Instant::now() + HOLDER_WAIT;
        loop {
            let holder = self.holder();
            if holder.is_some() || std::time::Instant::now() >= until {
                return holder;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

/// Writes who holds the home whole, beside its place and then over it, so that nobody reads half of it.
fn write_holder(file: &Path, name: &str) -> io::Result<()> {
    let name: String = name
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_NAME)
        .collect();
    let text = json!({ "pid": std::process::id(), "name": name }).to_string();
    let beside = file.with_extension(format!("holder.{}", std::process::id()));
    let written = File::create(&beside)
        .and_then(|mut out| out.write_all(text.as_bytes()))
        .and_then(|()| fs::rename(&beside, file));
    if written.is_err() {
        let _ = fs::remove_file(&beside);
    }
    written
}

fn read_holder(file: &Path) -> Option<Holder> {
    let mut text = String::new();
    File::open(file)
        .ok()?
        .take(MAX_HOLDER)
        .read_to_string(&mut text)
        .ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    let name: String = value
        .get("name")?
        .as_str()?
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_NAME)
        .collect();
    Some(Holder {
        name: if name.is_empty() {
            "another host".to_string()
        } else {
            name
        },
        pid: u32::try_from(value.get("pid")?.as_u64()?).ok()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("canoa-home-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_second_lock_on_a_home_says_who_holds_it() {
        let dir = fresh("held");
        let home = Home::at(&dir);
        let lock = home.lock("CanoaUI").expect("a free home locks");
        let refused = home.lock("canoa").expect_err("it is held");
        let LockError::Held { holder, .. } = &refused else {
            panic!("{refused}");
        };
        let holder = holder.as_ref().expect("it said who");
        assert_eq!(holder.name, "CanoaUI");
        assert_eq!(holder.pid, std::process::id());
        let said = refused.to_string();
        assert!(said.contains("CanoaUI"), "{said}");
        assert!(said.contains(&std::process::id().to_string()), "{said}");
        drop(lock);
        drop(home.lock("canoa").expect("a lock given up is free"));
        fs::remove_dir_all(&dir).expect("removed");
    }

    #[test]
    fn dropping_the_lock_leaves_nobody_named() {
        let dir = fresh("gone");
        let home = Home::at(&dir);
        drop(home.lock("canoa").expect("a free home locks"));
        assert_eq!(home.holder(), None);
        assert!(dir.join(LOCK_FILE).exists(), "the lock's file stays");
        fs::remove_dir_all(&dir).expect("removed");
    }

    #[test]
    fn a_holder_file_left_by_the_dead_does_not_hold_the_home() {
        let dir = fresh("dead");
        fs::create_dir_all(&dir).expect("made");
        fs::write(
            dir.join(HOLDER_FILE),
            r#"{"pid": 4294967295, "name": "dead"}"#,
        )
        .expect("written");
        let home = Home::at(&dir);
        let lock = home.lock("canoa").expect("nothing holds the lock");
        assert_eq!(
            home.holder().map(|holder| holder.name),
            Some("canoa".to_owned())
        );
        drop(lock);
        fs::remove_dir_all(&dir).expect("removed");
    }

    #[test]
    fn a_holders_name_cannot_write_on_the_terminal() {
        let dir = fresh("control");
        fs::create_dir_all(&dir).expect("made");
        fs::write(
            dir.join(HOLDER_FILE),
            r#"{"pid": 7, "name": "evil\u001b[2J\nname"}"#,
        )
        .expect("written");
        assert_eq!(
            Home::at(&dir).holder(),
            Some(Holder {
                name: "evil[2Jname".to_owned(),
                pid: 7,
            })
        );
        fs::remove_dir_all(&dir).expect("removed");
    }

    #[test]
    fn a_home_keeps_its_method_as_the_peer_crate_does() {
        let dir = fresh("method");
        let home = Home::at(&dir);
        let first = home.identity(None).expect("made");
        assert_eq!(first.method(), Method::DEFAULT);
        assert_eq!(
            home.identity(None).expect("read").peer_id(),
            first.peer_id()
        );
        let other = Method::ALL
            .into_iter()
            .find(|method| *method != Method::DEFAULT)
            .expect("two methods");
        let moved = home.identity(Some(other)).expect("made");
        assert_eq!(moved.method(), other);
        assert_ne!(moved.peer_id(), first.peer_id());
        assert_eq!(
            home.identity(None).expect("read").peer_id(),
            moved.peer_id()
        );
        fs::remove_file(dir.join(METHOD_FILE)).expect("removed");
        assert!(home.identity(None).is_err(), "both, and none named");
        fs::remove_dir_all(&dir).expect("removed");
    }
}
