//! A directory only this user can reach, made once for a connection and removed when it ends.
//!
//! It is created non-recursively and refuses a path that already exists, so a directory another user made first (or made
//! again after this one went) is never taken. Everything the host writes for a harness goes under such a directory.

use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Where private directories are made: the user's runtime directory where there is one, which only the user can write, else
/// the temporary directory.
pub(super) fn base() -> PathBuf {
    base_from(std::env::var_os("XDG_RUNTIME_DIR"))
}

/// [`base`] given what `XDG_RUNTIME_DIR` holds.
fn base_from(runtime: Option<std::ffi::OsString>) -> PathBuf {
    #[cfg(unix)]
    if let Some(runtime) = runtime
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute() && dir.is_dir())
    {
        return runtime;
    }
    #[cfg(not(unix))]
    let _ = runtime;
    std::env::temp_dir()
}

/// The user id this process runs as.
#[cfg(unix)]
fn own_uid() -> u32 {
    rustix::process::geteuid().as_raw()
}

/// Makes `path` as a directory of this user's alone. It must not exist, and its parent must.
///
/// # Errors
///
/// `AlreadyExists` when something is at `path`, whoever it belongs to; any other error when the directory cannot be made or
/// is not, once made, this user's alone.
pub(super) fn make(path: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(false);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(path)?;
    #[cfg(unix)]
    check(path)?;
    #[cfg(windows)]
    protect(path)?;
    Ok(())
}

/// Makes `path` inside a directory made by [`make`]: it must not exist, and only this user can enter it.
///
/// # Errors
///
/// As [`make`].
pub(super) fn make_inside(path: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(false);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(path)?;
    #[cfg(unix)]
    check(path)?;
    Ok(())
}

/// Whether what is at `path` is a directory (not a link) of this user's that no one else can enter.
#[cfg(unix)]
pub(super) fn check(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::MetadataExt as _;
    let meta = std::fs::symlink_metadata(path)?;
    if !meta.is_dir() || meta.uid() != own_uid() || meta.mode() & 0o077 != 0 {
        return Err(std::io::Error::other(
            "a directory that is not this user's alone",
        ));
    }
    Ok(())
}

/// Takes every right to `path` away from everyone but this user and the system, and from what it would inherit.
#[cfg(windows)]
fn protect(path: &Path) -> std::io::Result<()> {
    let root =
        std::env::var_os("SystemRoot").map_or_else(|| PathBuf::from("C:\\Windows"), PathBuf::from);
    let status = std::process::Command::new(root.join("System32").join("icacls.exe"))
        .arg(path)
        .args(["/inheritance:r", "/grant:r"])
        // OWNER RIGHTS, so no name has to be looked up: the directory's creator is its owner.
        .arg("*S-1-3-4:(OI)(CI)F")
        .arg("*S-1-5-18:(OI)(CI)F")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(
            "the directory's rights could not be limited to this user",
        ))
    }
}

/// A private directory with a name of its own, made at its first use and removed, with all it holds, when this is dropped.
#[derive(Debug)]
pub(super) struct Private {
    path: PathBuf,
    made: OnceLock<Result<(), String>>,
    #[cfg(windows)]
    held: OnceLock<Option<std::fs::File>>,
}

impl Private {
    /// A directory named `PREFIX-PID-RANDOM` in [`base`]; nothing is made until [`Self::ensure`].
    pub(super) fn new(prefix: &str) -> Self {
        Self::under(&base(), prefix)
    }

    /// As [`Self::new`], in `dir`.
    pub(super) fn under(dir: &Path, prefix: &str) -> Self {
        let mut random = [0_u8; 16];
        let _ = getrandom::fill(&mut random);
        let name: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        Self {
            path: dir.join(format!("{prefix}-{}-{name}", std::process::id())),
            made: OnceLock::new(),
            #[cfg(windows)]
            held: OnceLock::new(),
        }
    }

    /// Where it is, made or not.
    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    /// The directory, made on the first call; every call after gives that call's answer.
    ///
    /// # Errors
    ///
    /// As [`make`]: nothing is written under a directory that was not made here.
    pub(super) fn ensure(&self) -> std::io::Result<&Path> {
        let made = self.made.get_or_init(|| {
            make(&self.path).map_err(|error| error.to_string())?;
            self.lock();
            Ok(())
        });
        match made {
            Ok(()) => Ok(&self.path),
            Err(why) => Err(std::io::Error::other(why.clone())),
        }
    }

    /// Marks the directory as a live connection's, so a sweep leaves it: a file held open where the system forbids deleting
    /// it, and on Unix the process id in the name.
    fn lock(&self) {
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt as _;
            let _ = self.held.set(
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    // Read and write may be shared, deleting may not.
                    .share_mode(0x3)
                    .open(self.path.join(LIVE))
                    .ok(),
            );
        }
    }
}

impl Drop for Private {
    fn drop(&mut self) {
        if matches!(self.made.get(), Some(Ok(()))) {
            #[cfg(windows)]
            drop(self.held.take());
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

/// The file a live connection holds open on Windows.
#[cfg(windows)]
const LIVE: &str = ".live";

/// How old a directory of a host that did not name its process must be before it is taken for a dead host's.
#[cfg(not(windows))]
const UNNAMED: Duration = Duration::from_secs(24 * 3600);
/// How old a directory must be, whoever made it, before it is looked at.
const STALE: Duration = Duration::from_secs(3600);

/// Whether the process `pid` runs, where the system can say without asking it anything.
#[cfg(unix)]
fn running(pid: u32) -> bool {
    let Some(pid) = i32::try_from(pid)
        .ok()
        .and_then(rustix::process::Pid::from_raw)
    else {
        return true;
    };
    // Not permitted means it runs, as another user's.
    !matches!(
        rustix::process::test_kill_process(pid),
        Err(rustix::io::Errno::SRCH)
    )
}

/// Whether the directory `entry` is one `prefix` names that a dead host left and this user owns.
fn is_stale(prefix: &str, entry: &std::fs::DirEntry) -> bool {
    let name = entry.file_name().to_string_lossy().into_owned();
    let Some(rest) = name
        .strip_prefix(prefix)
        .and_then(|rest| rest.strip_prefix('-'))
    else {
        return false;
    };
    let Ok(meta) = entry.path().symlink_metadata() else {
        return false;
    };
    if !meta.is_dir() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if meta.uid() != own_uid() {
            return false;
        }
    }
    let old = meta
        .modified()
        .ok()
        .and_then(|made| made.elapsed().ok())
        .is_some_and(|age| age > STALE);
    // `PID-RANDOM` names the host that made it; a name with the random part only is an older host's.
    let named = rest
        .split_once('-')
        .and_then(|(pid, _)| pid.parse::<u32>().ok());
    // Old as well as dead: a host in another process namespace that shares this directory is not dead for having no process
    // here, and a process id that has been used again does not keep a dead host's directory for ever.
    if !old {
        return false;
    }
    #[cfg(unix)]
    {
        let ancient = meta
            .modified()
            .ok()
            .and_then(|made| made.elapsed().ok())
            .is_some_and(|age| age > UNNAMED);
        // A process id that was used again does not keep a directory a day old.
        if let Some(pid) = named {
            return ancient || !running(pid);
        }
    }
    #[cfg(windows)]
    let _ = named;
    #[cfg(windows)]
    {
        // A live connection holds `.live` open without leave to delete it.
        let live = entry.path().join(LIVE);
        if live.exists() && std::fs::remove_file(&live).is_err() {
            return false;
        }
        true
    }
    #[cfg(not(windows))]
    {
        meta.modified()
            .ok()
            .and_then(|made| made.elapsed().ok())
            .is_some_and(|age| age > UNNAMED)
    }
}

/// Removes what a host that was killed left in `dir`: the directories named `PREFIX-...` for each of `prefixes` that its
/// owner (this user) made, whose host no longer runs.
pub(super) fn sweep(dir: &Path, prefixes: &[&str]) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let entries: Vec<_> = entries.flatten().collect();
    for entry in entries {
        if prefixes.iter().any(|prefix| is_stale(prefix, &entry)) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(test)]
mod tests;
