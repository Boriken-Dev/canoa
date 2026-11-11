//! [`Shelf`]: the folder Canoa owns for the programs it installs, and the one truth about each, `current.json`.
//!
//! ```text
//! <programs>/<program>/
//!   lock                       held by every operation below
//!   current.json               {version, dir, launch, files, by, installed_at, previous}
//!   versions/<version>-<nonce>/  a version's files; the staging folder is this folder, so paths never move
//!   logs/<utc>-<action>.log      the streamed output, the last few kept
//! ```
//!
//! The switch from one version to the next is one rename of `current.json`. A session already running keeps the path it
//! started with; a new one reads the new file. Nothing here runs at startup: a crashed install leaves a folder `current.json`
//! does not name, which the next locked operation removes.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files, paths and the lock are std's"
)]

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;
use std::fs::{self, File, TryLockError};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use canoa::store::{Launch, is_program_name};
use coqui_acp_api::guatiao::Alloc;
use serde::{Deserialize, Serialize};

/// How many versions are kept by default: the current one and the previous.
pub const DEFAULT_KEEP_VERSIONS: usize = 2;

/// Why a shelf operation did not happen.
#[derive(Debug)]
#[non_exhaustive]
pub enum ShelfError {
    /// Another process holds the program's lock.
    Held,
    /// Not a program folder name.
    Name,
    /// The program's folder is not owned by this user alone.
    NotPrivate(String),
    /// What the staged copy claims does not hold.
    Refused(String),
    /// Nothing to roll back to, or no copy at all.
    Nothing(&'static str),
    /// A file could not be used.
    Io(io::Error),
}

impl fmt::Display for ShelfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Held => f.write_str("another operation holds this program"),
            Self::Name => f.write_str("not a program folder name"),
            Self::NotPrivate(why) => write!(f, "the programs folder is not private: {why}"),
            Self::Refused(why) => f.write_str(why),
            Self::Nothing(what) => f.write_str(what),
            Self::Io(error) => write!(f, "{error}"),
        }
    }
}

impl core::error::Error for ShelfError {}

impl From<io::Error> for ShelfError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// A launch, as `current.json` writes it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LaunchRecord {
    /// The absolute path.
    pub path: String,
    /// The harness settings, as JSON.
    #[serde(default)]
    pub settings: serde_json::Value,
    /// The variables.
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
}

impl LaunchRecord {
    /// The record of `launch`.
    #[must_use]
    pub fn of(launch: &Launch) -> Self {
        let settings = serde_json::to_value(guatiao_serde::Serializable::from(&launch.settings))
            .unwrap_or_else(|_| serde_json::json!({}));
        Self {
            path: launch.path.clone(),
            settings,
            environment: launch.environment.clone(),
        }
    }

    /// The launch it records.
    #[must_use]
    pub fn launch(&self) -> Launch {
        let mut launch = Launch::new(self.path.clone());
        if let Ok(value) = guatiao_serde::adapt::to_value_in(&self.settings, Alloc::rust()) {
            launch.settings = value;
        }
        launch.environment = self.environment.clone();
        launch
    }
}

/// A file an install fetched.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileRecord {
    /// The path inside the version's folder.
    pub path: String,
    /// Where it came from.
    pub url: String,
    /// Its SHA-256.
    pub sha256: String,
    /// Whether the publisher gave something to check it against. A copy with a file where this is false was installed only
    /// because the person allowed a download nothing checks.
    #[serde(default = "yes")]
    pub verified: bool,
}

const fn yes() -> bool {
    true
}

/// The store that made a copy.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MadeBy {
    /// The provider id.
    pub provider: String,
    /// Its library's version.
    pub version: String,
}

/// The copy before the current one, for a rollback.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Previous {
    /// Its version.
    pub version: String,
    /// Its folder.
    pub dir: String,
    /// How to start it.
    pub launch: LaunchRecord,
}

/// `current.json`: the one truth about a program's managed copy.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Current {
    /// The version.
    pub version: String,
    /// The absolute path of its folder.
    pub dir: String,
    /// How to start it.
    pub launch: LaunchRecord,
    /// Every file the install fetched, with its digest: the audit trail.
    #[serde(default)]
    pub files: Vec<FileRecord>,
    /// Who installed it.
    pub by: MadeBy,
    /// When, in Unix seconds.
    pub installed_at: u64,
    /// The copy before this one.
    #[serde(default)]
    pub previous: Option<Previous>,
}

/// The folder of all programs.
#[derive(Debug, Clone)]
#[must_use]
pub struct Shelf {
    root: PathBuf,
    keep: usize,
}

impl Shelf {
    /// A shelf at `root`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            keep: DEFAULT_KEEP_VERSIONS,
        }
    }

    /// How many versions to keep, current included; no floor but one.
    pub fn keep_versions(mut self, keep: usize) -> Self {
        self.keep = keep.max(1);
        self
    }

    /// Where programs live by default: `%LOCALAPPDATA%\Canoa\programs`, `~/Library/Application Support/Canoa/programs`,
    /// else `$XDG_DATA_HOME/Canoa/programs` or `~/.local/share/Canoa/programs`. `None` when no home is known.
    #[must_use]
    pub fn default_root() -> Option<PathBuf> {
        let env = |name: &str| {
            std::env::var_os(name)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        if cfg!(windows) {
            return env("LOCALAPPDATA").map(|d| d.join("Canoa").join("programs"));
        }
        if cfg!(target_os = "macos") {
            return env("HOME").map(|d| {
                d.join("Library")
                    .join("Application Support")
                    .join("Canoa")
                    .join("programs")
            });
        }
        env("XDG_DATA_HOME")
            .or_else(|| env("HOME").map(|d| d.join(".local").join("share")))
            .map(|d| d.join("Canoa").join("programs"))
    }

    /// A shelf at `root`, which has to be the default folder or below it: no check yet reads who may write another place, so the
    /// host takes no other.
    ///
    /// # Errors
    ///
    /// [`ShelfError::NotPrivate`] for a root outside [`default_root`](Self::default_root), or when there is no default.
    pub fn configured(root: impl Into<PathBuf>) -> Result<Self, ShelfError> {
        let root = root.into();
        let default = Self::default_root()
            .ok_or_else(|| ShelfError::NotPrivate("this user has no data folder".into()))?;
        if !super::footprint::path_under(&root.to_string_lossy(), &default.to_string_lossy()) {
            return Err(ShelfError::NotPrivate(
                "a programs folder outside the user's data folder is not accepted until the access list of the place can be read"
                    .into(),
            ));
        }
        Ok(Self::new(root))
    }

    /// The root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The folder of `program`.
    ///
    /// # Errors
    ///
    /// [`ShelfError::Name`] for a name that is not a program folder name.
    pub fn program(&self, program: &str) -> Result<ProgramDir, ShelfError> {
        if !is_program_name(program) {
            return Err(ShelfError::Name);
        }
        Ok(ProgramDir {
            dir: self.root.join(program),
            keep: self.keep,
        })
    }
}

/// One program's folder.
#[derive(Debug, Clone)]
pub struct ProgramDir {
    dir: PathBuf,
    keep: usize,
}

impl ProgramDir {
    /// The folder.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The managed copy, when there is one. Reading takes no lock: `current.json` is only ever replaced whole.
    ///
    /// # Errors
    ///
    /// [`ShelfError::Io`] for a file that cannot be read or is not a `current.json`.
    pub fn current(&self) -> Result<Option<Current>, ShelfError> {
        let path = self.dir.join("current.json");
        match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|e| ShelfError::Io(io::Error::new(io::ErrorKind::InvalidData, e))),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// Checks the files recorded for `current` against their digests, before a launch. A recorded file that is not there any more
    /// was an archive the install unpacked and removed, whose files are not recorded one by one, and is not checked.
    ///
    /// # Errors
    ///
    /// [`ShelfError::Refused`] naming the first file that no longer matches.
    pub fn verify(&self, current: &Current) -> Result<(), ShelfError> {
        use sha2::{Digest, Sha256};
        use std::io::Read;
        for record in &current.files {
            let path = Path::new(&current.dir).join(&record.path);
            let Ok(mut file) = File::open(&path) else {
                continue;
            };
            let mut hash = Sha256::new();
            let mut buffer = vec![0u8; 64 * 1024];
            loop {
                let count = file.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                hash.update(&buffer[..count]);
            }
            let digest = format!("{:x}", hash.finalize());
            if !digest.eq_ignore_ascii_case(&record.sha256) {
                return Err(ShelfError::Refused(format!(
                    "{} no longer matches the digest recorded when it was installed",
                    record.path
                )));
            }
        }
        Ok(())
    }

    /// Takes the program's lock, making its folder (private to the user) as needed, and removes the folders left by a
    /// crashed operation.
    ///
    /// # Errors
    ///
    /// [`ShelfError::Held`] while another holds it, [`ShelfError::NotPrivate`] for a folder others may write.
    pub fn lock(&self) -> Result<Locked<'_>, ShelfError> {
        make_private(&self.dir)?;
        check_private(&self.dir)?;
        let file = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.dir.join("lock"))?;
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Err(ShelfError::Held),
            Err(TryLockError::Error(error)) => return Err(error.into()),
        }
        let locked = Locked {
            program: self,
            _file: file,
        };
        locked.sweep()?;
        Ok(locked)
    }
}

/// An operation holding the program's lock.
#[derive(Debug)]
pub struct Locked<'a> {
    program: &'a ProgramDir,
    _file: File,
}

impl Locked<'_> {
    fn versions(&self) -> PathBuf {
        self.program.dir.join("versions")
    }

    /// Makes the empty folder an install writes into: `versions/<version>-<nonce>`.
    ///
    /// # Errors
    ///
    /// [`ShelfError::Refused`] for a version that is not a plain name.
    pub fn stage(&self, version: &str) -> Result<PathBuf, ShelfError> {
        if version.is_empty()
            || version.len() > 64
            || !version
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b'+'))
        {
            return Err(ShelfError::Refused(
                "a version that is not a plain name".into(),
            ));
        }
        let mut nonce = [0u8; 4];
        getrandom::fill(&mut nonce).map_err(|e| io::Error::other(e.to_string()))?;
        let name = format!(
            "{version}-{}",
            nonce.iter().map(|b| format!("{b:02x}")).collect::<String>()
        );
        let dir = self.versions().join(name);
        make_private(&dir)?;
        Ok(dir)
    }

    /// Removes a staged folder after a failure or a cancel; `current.json` is untouched.
    pub fn abandon(&self, staging: &Path) {
        if staging.starts_with(self.versions()) {
            let _ = fs::remove_dir_all(staging);
        }
    }

    /// Switches to the staged copy: checks it, writes `current.json` over the old one in one rename, makes the old one
    /// `previous`, and prunes.
    ///
    /// # Errors
    ///
    /// [`ShelfError::Refused`] when the launch path is not a regular file inside the staging folder, or leads out of it
    /// by a link; the staged folder is then removed.
    pub fn commit(
        &self,
        staging: &Path,
        version: &str,
        launch: &Launch,
        files: Vec<FileRecord>,
        by: MadeBy,
    ) -> Result<Current, ShelfError> {
        if let Err(error) = check_launch(staging, &launch.path) {
            self.abandon(staging);
            return Err(error);
        }
        let old = self.program.current()?;
        let current = Current {
            version: version.to_owned(),
            dir: staging.to_string_lossy().into_owned(),
            launch: LaunchRecord::of(launch),
            files,
            by,
            installed_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
            previous: old.map(|old| Previous {
                version: old.version,
                dir: old.dir,
                launch: old.launch,
            }),
        };
        self.write(&current)?;
        self.sweep()?;
        Ok(current)
    }

    /// Swaps the current copy and the previous one. No extension is asked.
    ///
    /// # Errors
    ///
    /// [`ShelfError::Nothing`] while there is no previous copy.
    pub fn rollback(&self) -> Result<Current, ShelfError> {
        let current = self
            .program
            .current()?
            .ok_or(ShelfError::Nothing("there is no copy"))?;
        let previous = current
            .previous
            .clone()
            .ok_or(ShelfError::Nothing("there is no previous version"))?;
        if !Path::new(&previous.dir).is_dir() {
            return Err(ShelfError::Nothing(
                "the previous version is no longer there",
            ));
        }
        let swapped = Current {
            version: previous.version,
            dir: previous.dir,
            launch: previous.launch,
            files: Vec::new(),
            by: current.by.clone(),
            installed_at: current.installed_at,
            previous: Some(Previous {
                version: current.version,
                dir: current.dir,
                launch: current.launch,
            }),
        };
        self.write(&swapped)?;
        Ok(swapped)
    }

    /// Removes the managed copy: `current.json` first, so nothing points at it any more, then the folders.
    ///
    /// # Errors
    ///
    /// [`ShelfError::Io`] for a file that could not be removed.
    pub fn remove(&self) -> Result<(), ShelfError> {
        match fs::remove_file(self.program.dir.join("current.json")) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        self.sweep()
    }

    fn write(&self, current: &Current) -> Result<(), ShelfError> {
        let temporary = self.program.dir.join("current.json.tmp");
        let mut file = File::create(&temporary)?;
        file.write_all(&serde_json::to_vec_pretty(current).map_err(io::Error::other)?)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, self.program.dir.join("current.json"))?;
        Ok(())
    }

    /// Removes the version folders `current.json` does not name (past `keep`), retrying later any the system says is in use.
    fn sweep(&self) -> Result<(), ShelfError> {
        let current = self.program.current()?;
        let mut keep: Vec<PathBuf> = Vec::new();
        if let Some(current) = &current {
            keep.push(PathBuf::from(&current.dir));
            if self.program.keep > 1
                && let Some(previous) = &current.previous
            {
                keep.push(PathBuf::from(&previous.dir));
            }
        }
        let Ok(entries) = fs::read_dir(self.versions()) else {
            return Ok(());
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !keep.iter().any(|k| same_path(k, &path)) {
                // A folder Windows reports in use stays, and goes at the next locked operation.
                let _ = fs::remove_dir_all(&path);
            }
        }
        Ok(())
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        a.to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy())
    } else {
        a == b
    }
}

/// The launch path is a regular file inside the folder, reached by no link that leads out.
fn check_launch(staging: &Path, launch: &str) -> Result<(), ShelfError> {
    let refused = |why: &str| Err(ShelfError::Refused(why.to_owned()));
    let path = Path::new(launch);
    if !path.is_absolute() {
        return refused("the launch path is not absolute");
    }
    let (Ok(root), Ok(real)) = (fs::canonicalize(staging), fs::canonicalize(path)) else {
        return refused("the launch path is not there");
    };
    if !real.starts_with(&root) {
        return refused("the launch path is outside the version's folder");
    }
    if !real.is_file() {
        return refused("the launch path is not a regular file");
    }
    Ok(())
}

#[cfg(unix)]
fn make_private(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

#[cfg(not(unix))]
fn make_private(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)
}

/// The folder belongs to this user and no one else can write it. On Windows the folder inherits its parent's access list
/// (`%LOCALAPPDATA%`'s, which is the user's alone); the list itself is not read here.
#[cfg(unix)]
fn check_private(dir: &Path) -> Result<(), ShelfError> {
    use std::os::unix::fs::MetadataExt;
    let meta = fs::symlink_metadata(dir)?;
    if !meta.is_dir() {
        return Err(ShelfError::NotPrivate("it is not a folder".into()));
    }
    if meta.uid() != rustix::process::geteuid().as_raw() {
        return Err(ShelfError::NotPrivate("another user owns it".into()));
    }
    if meta.mode() & 0o022 != 0 {
        return Err(ShelfError::NotPrivate("others can write it".into()));
    }
    Ok(())
}

#[cfg(not(unix))]
fn check_private(dir: &Path) -> Result<(), ShelfError> {
    let meta = fs::symlink_metadata(dir)?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(ShelfError::NotPrivate("it is a link, not a folder".into()));
    }
    Ok(())
}
