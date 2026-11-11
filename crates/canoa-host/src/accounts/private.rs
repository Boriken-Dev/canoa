//! Directories and files only the owner can reach. On Unix that is mode 0700 and 0600; on Windows the accounts directory
//! gets a protected ACL naming this user and the system alone, which what is made inside it inherits.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files are std's"
)]

use std::fs;
use std::io;
use std::path::Path;

/// Makes `path` and the directories above it that are missing, each private to the owner where the system lets that be
/// said by a mode. On Windows what is made inherits its parent's ACL, which [`create_root`] has restricted.
pub(super) fn create_dir(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(path)
    }
}

/// As [`create_dir`] for the accounts directory itself, which on Windows is restricted to this user and the system the
/// moment it is made; one that is refused is not used. A directory that is already there is left as it is.
pub(super) fn create_root(path: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        let existed = path.is_dir();
        create_dir(path)?;
        if !existed && let Err(error) = restrict(path) {
            let _ = fs::remove_dir_all(path);
            return Err(error);
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        create_dir(path)
    }
}

/// A protected ACL, no inherited entry: full control for this user and for the system, passed to what is made inside.
#[cfg(windows)]
fn restrict(path: &Path) -> io::Result<()> {
    let user = match (std::env::var("USERDOMAIN"), std::env::var("USERNAME")) {
        (Ok(domain), Ok(name)) if !domain.is_empty() => format!("{domain}\\{name}"),
        (_, Ok(name)) => name,
        _ => return Err(io::Error::other("this user's name is not known")),
    };
    let granted = std::process::Command::new("icacls")
        .arg(path)
        .args(["/inheritance:r", "/grant:r"])
        .arg(format!("{user}:(OI)(CI)F"))
        .arg("*S-1-5-18:(OI)(CI)F")
        .output()?;
    if granted.status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "the accounts directory's access could not be restricted: {}",
            String::from_utf8_lossy(&granted.stdout).trim()
        )))
    }
}

/// A new file only this user can read, made only where nothing is.
pub(super) fn create_file(path: &Path) -> io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(path)
}

/// A file only this user can read, made or truncated and written whole.
pub(super) fn write_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write as _;
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(path)?.write_all(bytes)
}

/// The lease file, which is locked and never read.
pub(super) fn lease_file(path: &Path) -> io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(path)
}
