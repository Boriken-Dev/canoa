//! The one way the host writes a whole file (canoa.cfg, the overlay records, its state and caches): whole or not at all, and with
//! the mode (Unix) or the access list (Windows) the file had. A file that grows by appends, or arrives in pieces over several
//! requests, is not written here.

use core::sync::atomic::{AtomicU64, Ordering};
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

/// The mode of a new file only its owner reads: a secret, a sealed record, a list of keys.
pub(crate) const PRIVATE: u32 = 0o600;

/// The mode `fs::write` gives a new file, the process's umask deciding the rest: a record the person's other programs may read.
pub(crate) const SHARED: u32 = 0o666;

/// Writes `bytes` as the whole of `path`: a finished file is made beside it, synced, and put in its place, so a stop in the middle
/// leaves the old file. A file that exists keeps its mode on Unix and its access list on Windows, and when `path` is a link it is
/// the link's target that is replaced; a new file is made with `mode` on Unix (the access list of its folder on Windows). The
/// folder is made when it is missing.
///
/// # Errors
///
/// The file could not be written; what was made beside it is removed.
pub(crate) fn write_file(path: &Path, bytes: &[u8], mode: u32) -> io::Result<()> {
    write(path, bytes, mode, true)
}

/// As [`write_file`], but on Windows the file takes the access list of its folder, as a new one would, and is put in place with
/// one rename: a reader that opens the name meanwhile finds the old file or the new one, never none. For a folder that is made
/// private to its owner, whose files are read while they are rewritten.
///
/// # Errors
///
/// As [`write_file`].
pub(crate) fn write_file_in_folder(path: &Path, bytes: &[u8], mode: u32) -> io::Result<()> {
    write(path, bytes, mode, false)
}

fn write(path: &Path, bytes: &[u8], mode: u32, keep_access: bool) -> io::Result<()> {
    // A link is written through to its target, which is what the person pointed at.
    let target = fs::canonicalize(path).map_or_else(
        |_| path.to_path_buf(),
        |real| crate::share::plain(&real).into_owned(),
    );
    if let Some(parent) = target.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    // A name no other writer has, in this process or another, and none could have put a link at in advance: it is made only
    // where nothing is.
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    let mut beside = target.as_os_str().to_owned();
    beside.push(format!(
        ".new{}.{stamp}.{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let beside = PathBuf::from(beside);
    let done = make_beside(&beside, &target, bytes, mode)
        .and_then(|()| put_in_place(&beside, &target, keep_access));
    if done.is_err() {
        let _ = fs::remove_file(&beside);
    }
    done
}

/// Makes the finished file `beside` for `target`: with the mode `target` has on Unix when it exists.
fn make_beside(beside: &Path, target: &Path, bytes: &[u8], mode: u32) -> io::Result<()> {
    let existing = fs::metadata(target).ok().filter(fs::Metadata::is_file);
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(mode);
    }
    let _ = mode;
    let mut file = options.open(beside)?;
    #[cfg(unix)]
    if let Some(existing) = &existing {
        file.set_permissions(existing.permissions())?;
    }
    let _ = existing;
    file.write_all(bytes)?;
    file.sync_all()
}

/// Puts the finished file `beside` in the place of `target`, which is then the new text whole, never part of it. On Windows a file
/// that exists is replaced with its access list, which a file made beside it would not have.
fn put_in_place(beside: &Path, target: &Path, keep_access: bool) -> io::Result<()> {
    #[cfg(not(windows))]
    let _ = keep_access;
    #[cfg(windows)]
    if keep_access && target.is_file() {
        return native::replace(beside, target);
    }
    fs::rename(beside, target)
}

#[cfg(windows)]
#[allow(
    unsafe_code,
    reason = "ReplaceFileW, which swaps a file in with the access list of the one it replaces, has no safe form in std"
)]
mod native {
    use std::io;
    use std::os::windows::ffi::OsStrExt as _;
    use std::path::Path;

    use windows_sys::Win32::Storage::FileSystem::{REPLACEFILE_IGNORE_MERGE_ERRORS, ReplaceFileW};

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    /// `replacement` in the place of `replaced`, which keeps its access list; a failure to carry the list over is not a failure to
    /// write the file.
    pub(super) fn replace(replacement: &Path, replaced: &Path) -> io::Result<()> {
        let (replacement, replaced) = (wide(replacement), wide(replaced));
        // SAFETY: both names are NUL-terminated and live past the call; the backup, the exclusion and the reserved argument are
        // null, which the function documents as absent.
        let done = unsafe {
            ReplaceFileW(
                replaced.as_ptr(),
                replacement.as_ptr(),
                core::ptr::null(),
                REPLACEFILE_IGNORE_MERGE_ERRORS,
                core::ptr::null(),
                core::ptr::null(),
            )
        };
        if done == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

/// What the tests of the modules that write through [`write_file`] share.
#[cfg(test)]
pub(crate) mod proof {
    use std::fs;
    use std::path::{Path, PathBuf};

    /// A new empty folder of this process's, removed by the caller.
    pub(crate) fn folder(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("canoa-one-writer-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("a folder");
        dir
    }

    /// The names in `dir`, sorted.
    pub(crate) fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .expect("a folder")
            .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
            .collect();
        names.sort();
        names
    }

    /// Gives `path` the mode `mode`, which a write over it must keep.
    #[cfg(unix)]
    pub(crate) fn set_mode(path: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("a mode");
    }

    /// The permission bits of `path`.
    #[cfg(unix)]
    pub(crate) fn mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt as _;
        fs::metadata(path).expect("a file").permissions().mode() & 0o777
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dir(PathBuf);

    impl Dir {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("canoa-replace-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("a directory");
            Self(dir)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn leftovers(dir: &Path) -> Vec<String> {
        fs::read_dir(dir)
            .expect("a directory")
            .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
            .filter(|name| name.contains(".new"))
            .collect()
    }

    #[test]
    fn the_new_text_is_the_whole_file_and_nothing_is_left_beside_it() {
        let dir = Dir::new("text");
        let path = dir.0.join("canoa.cfg");
        fs::write(&path, "a much longer old text").expect("written");
        write_file(&path, b"new", 0o600).expect("replaced");
        assert_eq!(fs::read_to_string(&path).expect("read"), "new");
        let fresh = dir.0.join("deeper").join("fresh.cfg");
        write_file(&fresh, b"first", 0o600).expect("made");
        assert_eq!(fs::read_to_string(&fresh).expect("read"), "first");
        assert!(leftovers(&dir.0).is_empty(), "{:?}", leftovers(&dir.0));
        assert!(leftovers(&dir.0.join("deeper")).is_empty());
    }

    #[test]
    fn a_failed_write_leaves_the_old_file_and_removes_what_it_made() {
        let dir = Dir::new("failed");
        let path = dir.0.join("canoa.cfg");
        fs::write(&path, "old").expect("written");
        // A folder where the file goes: the rename cannot put a file over it.
        let blocked = dir.0.join("blocked");
        fs::create_dir_all(blocked.join("inside")).expect("a folder");
        assert!(write_file(&blocked, b"new", 0o600).is_err());
        assert_eq!(fs::read_to_string(&path).expect("read"), "old");
        assert!(leftovers(&dir.0).is_empty(), "{:?}", leftovers(&dir.0));
    }

    #[cfg(unix)]
    #[test]
    fn a_file_keeps_the_mode_it_had_and_a_new_one_has_the_mode_asked() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = Dir::new("mode");
        let path = dir.0.join("canoa.cfg");
        for mode in [0o600, 0o644, 0o640] {
            fs::write(&path, "old").expect("written");
            fs::set_permissions(&path, fs::Permissions::from_mode(mode)).expect("mode");
            write_file(&path, b"new", 0o666).expect("replaced");
            let now = fs::metadata(&path).expect("metadata").permissions().mode() & 0o777;
            assert_eq!(now, mode, "{mode:o} became {now:o}");
        }
        let fresh = dir.0.join("fresh.cfg");
        write_file(&fresh, b"x", 0o600).expect("made");
        let now = fs::metadata(&fresh).expect("metadata").permissions().mode() & 0o777;
        assert_eq!(now, 0o600);
    }

    /// A canoa.cfg that is a link is written through to what it points at, and stays a link.
    #[cfg(unix)]
    #[test]
    fn a_link_is_written_through_and_stays_a_link() {
        let dir = Dir::new("link");
        let target = dir.0.join("elsewhere.cfg");
        let link = dir.0.join("canoa.cfg");
        fs::write(&target, "old").expect("written");
        std::os::unix::fs::symlink(&target, &link).expect("a link");
        write_file(&link, b"new", 0o600).expect("replaced");
        assert!(
            fs::symlink_metadata(&link)
                .expect("metadata")
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_to_string(&target).expect("read"), "new");
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;
    use crate::spawn::Command;

    fn access_of(path: &Path) -> String {
        let out = Command::new("icacls")
            .arg(path)
            .output()
            .expect("icacls runs");
        String::from_utf8_lossy(&out.stdout).replace(&path.display().to_string(), "")
    }

    #[test]
    fn a_file_keeps_the_access_list_it_had() {
        let dir = std::env::temp_dir().join(format!("canoa-replace-acl-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("a directory");
        let path = dir.join("canoa.cfg");
        fs::write(&path, "old").expect("written");
        let user = std::env::var("USERNAME").expect("a user name");
        let set = Command::new("icacls")
            .arg(&path)
            .args(["/inheritance:r", "/grant:r"])
            .arg(format!("{user}:(R,W)"))
            .output()
            .expect("icacls runs");
        assert!(
            set.status.success(),
            "{}",
            String::from_utf8_lossy(&set.stdout)
        );
        let before = access_of(&path);
        write_file(&path, b"new", 0o600).expect("replaced");
        assert_eq!(fs::read_to_string(&path).expect("read"), "new");
        assert_eq!(access_of(&path), before);
        let _ = fs::remove_dir_all(&dir);
    }

    /// The old file is whole until the new one replaces it: a reader that opens it meanwhile sees one or the other, never a
    /// truncated file.
    #[test]
    fn the_old_file_is_whole_while_the_new_one_is_made() {
        let dir = std::env::temp_dir().join(format!("canoa-replace-whole-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("a directory");
        let path = dir.join("canoa.cfg");
        let old = "old text ".repeat(10_000);
        fs::write(&path, &old).expect("written");
        let reader = {
            let path = path.clone();
            std::thread::spawn(move || {
                let mut seen = Vec::new();
                for _ in 0..2_000 {
                    if let Ok(text) = fs::read_to_string(&path) {
                        seen.push(text.len());
                    }
                }
                seen
            })
        };
        for _ in 0..200 {
            write_file(&path, "new text ".repeat(10_000).as_bytes(), 0o600).expect("replaced");
        }

        let lengths = reader.join().expect("the reader finished");
        assert!(
            lengths.iter().all(|&length| length == old.len()),
            "a reader saw a file of {:?} bytes",
            lengths.iter().find(|&&length| length != old.len())
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
