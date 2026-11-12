//! Checks on the folders the launcher reads and writes: not a link, made by this user, and not writable by others.
//!
//! A folder that fails is refused, never repaired: something other than this launcher put it there.

use crate::error::{Error, Result};
use std::fs::{self, File, Metadata};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// The name probe files start with, which a crashed launch can leave behind.
pub const PROBE: &str = ".probe-";

/// A name no other call or process shares, for a probe or a temporary sibling.
pub fn unique(prefix: &str) -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!(
        "{prefix}{}-{nanos}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

/// Whether `meta` (from `symlink_metadata`) is a link: a symbolic link, or on Windows any reparse point, which covers
/// junctions and mount points.
pub fn is_link(meta: &Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    false
}

/// The folder at `path` when it exists, is a real directory (not a link) and is owned by this user; `Ok(None)` when absent.
///
/// `parent` is a folder already checked, in which the launcher makes a probe file to learn what "this user" owns, and
/// whose owner is trusted too.
pub fn check_dir(parent: &Path, path: &Path) -> Result<Option<()>> {
    check_dir_in(parent, parent, path)
}

/// [`check_dir`] with the probe made in `probe_dir` and `trust_dir`'s owner trusted; they differ for the base folder,
/// whose owner must not vouch for itself.
fn check_dir_in(probe_dir: &Path, trust_dir: &Path, path: &Path) -> Result<Option<()>> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(Error::io(format!("inspect {}", path.display()), e)),
    };
    if is_link(&meta) {
        return Err(Error::Refused(format!("{} is a link", path.display())));
    }
    if !meta.is_dir() {
        return Err(Error::Refused(format!(
            "{} is not a folder",
            path.display()
        )));
    }
    if !owned_by_me(probe_dir, trust_dir, path)? {
        return Err(Error::Refused(format!(
            "{} is not owned by the current user",
            path.display()
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o022 != 0 {
            return Err(Error::Refused(format!(
                "{} is writable by others",
                path.display()
            )));
        }
    }
    Ok(Some(()))
}

/// Makes `base` when absent and checks it, as [`check_dir`] does for a folder inside it.
///
/// `base` is under the user's own data folder, which the operating system keeps private to the user; its parents are
/// not checked, since a profile may be redirected on purpose.
pub fn ensure_base(base: &Path) -> Result<()> {
    if fs::symlink_metadata(base).is_err() {
        fs::create_dir_all(base).map_err(|e| Error::io(format!("create {}", base.display()), e))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(base, fs::Permissions::from_mode(0o700))
                .map_err(|e| Error::io(format!("restrict {}", base.display()), e))?;
        }
    }
    // The probe goes in `base`, which the user can write; the owner trusted besides is that of the user's data folder.
    let trust = base.parent().unwrap_or(base);
    check_dir_in(base, trust, base).map(|_| ())
}

/// Whether `path` has an owner this launch can trust: the owner of a file this process creates in `probe_dir`, and on
/// Windows also the owner of `trust_dir`, the Administrators group and the system account.
///
/// A process started elevated creates files owned by Administrators rather than by the user, so a folder made by the same
/// person's other kind of window would otherwise be refused for good. Who may make a file owned by those is already
/// someone who can replace the launcher.
fn owned_by_me(probe_dir: &Path, trust_dir: &Path, path: &Path) -> Result<bool> {
    let probe: PathBuf = probe_dir.join(unique(PROBE));
    File::options()
        .write(true)
        .create_new(true)
        .open(&probe)
        .map_err(|e| Error::io(format!("create a probe in {}", probe_dir.display()), e))?;
    let result = trusted_owner(&probe, trust_dir, path);
    let _ = fs::remove_file(&probe);
    result.map_err(|e| Error::io(format!("read the owner of {}", path.display()), e))
}

#[cfg(unix)]
fn trusted_owner(probe: &Path, _parent: &Path, path: &Path) -> io::Result<bool> {
    use std::os::unix::fs::MetadataExt;
    Ok(fs::symlink_metadata(probe)?.uid() == fs::symlink_metadata(path)?.uid())
}

#[cfg(windows)]
fn trusted_owner(probe: &Path, parent: &Path, path: &Path) -> io::Result<bool> {
    use windows_permissions::constants::{SeObjectType, SecurityInformation};
    use windows_permissions::wrappers::{CreateWellKnownSid, GetNamedSecurityInfo};

    // WELL_KNOWN_SID_TYPE: WinLocalSystemSid and WinBuiltinAdministratorsSid.
    const LOCAL_SYSTEM: u32 = 22;
    const ADMINISTRATORS: u32 = 26;
    let descriptor = |p: &Path| {
        GetNamedSecurityInfo(p, SeObjectType::SE_FILE_OBJECT, SecurityInformation::Owner)
    };
    let found = descriptor(path)?;
    let Some(owner) = found.owner() else {
        return Ok(false);
    };
    for known in [LOCAL_SYSTEM, ADMINISTRATORS] {
        if CreateWellKnownSid(known, None)?.as_ref() == owner {
            return Ok(true);
        }
    }
    for trusted in [probe, parent] {
        if descriptor(trusted)?.owner() == Some(owner) {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(not(any(unix, windows)))]
fn trusted_owner(_: &Path, _: &Path, _: &Path) -> io::Result<bool> {
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(unique(name));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_folder_this_user_made_passes_and_an_absent_one_is_none() {
        let base = scratch("guard-ok-");
        let sub = base.join("v1");
        assert!(check_dir(&base, &sub).unwrap().is_none());
        fs::create_dir(&sub).unwrap();
        assert!(check_dir(&base, &sub).unwrap().is_some());
        ensure_base(&base).unwrap();
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn a_file_where_a_folder_belongs_is_refused() {
        let base = scratch("guard-file-");
        let file = base.join("v1");
        fs::write(&file, b"x").unwrap();
        assert!(matches!(check_dir(&base, &file), Err(Error::Refused(_))));
        fs::remove_dir_all(&base).unwrap();
    }

    /// Hands the base folder to an owner nothing else vouches for (Everyone), which only a process holding the restore
    /// privilege may do; without it the test says so and ends.
    #[cfg(windows)]
    #[test]
    fn a_base_folder_cannot_vouch_for_its_own_owner() {
        use windows_permissions::constants::{SeObjectType, SecurityInformation};
        use windows_permissions::wrappers::{CreateWellKnownSid, SetNamedSecurityInfo};

        const WORLD: u32 = 1; // WinWorldSid
        let parent = scratch("guard-base-");
        let base = parent.join("Canoa");
        fs::create_dir(&base).unwrap();
        ensure_base(&base).expect("a base folder this process made is accepted");
        let everyone = CreateWellKnownSid(WORLD, None).unwrap();
        let set = SetNamedSecurityInfo(
            &base,
            SeObjectType::SE_FILE_OBJECT,
            SecurityInformation::Owner,
            Some(everyone.as_ref()),
            None,
            None,
            None,
        );
        if let Err(e) = set {
            eprintln!("skipped: this process may not give a folder to another owner ({e})");
        } else {
            assert!(matches!(ensure_base(&base), Err(Error::Refused(_))));
        }
        fs::remove_dir_all(&parent).unwrap();
    }

    #[test]
    fn names_do_not_repeat() {
        assert_ne!(unique("a-"), unique("a-"));
    }
}
