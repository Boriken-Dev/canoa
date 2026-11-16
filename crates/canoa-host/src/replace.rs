//! Putting a finished file in the place of another that people may have set the access of: a canoa.cfg rewritten by the host keeps
//! the mode (Unix) or the access list (Windows) it had.

use std::fs;
use std::io;
use std::path::Path;

/// Replaces `path` with the finished file `beside`, which is then gone. When `path` exists it keeps its mode on Unix, and on
/// Windows its access list, which a file made beside it would not have.
///
/// # Errors
///
/// The file could not be put in place; `beside` is left for the caller to remove.
pub(crate) fn replace_file(beside: &Path, path: &Path) -> io::Result<()> {
    let existing = fs::metadata(path).ok();
    #[cfg(unix)]
    if let Some(existing) = &existing {
        fs::set_permissions(beside, existing.permissions())?;
    }
    // CopyFile onto a file that exists writes into it and leaves its access list; a rename would bring the new file's own.
    #[cfg(windows)]
    if existing.is_some() {
        fs::copy(beside, path)?;
        return fs::remove_file(beside);
    }
    let _ = existing;
    fs::rename(beside, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dir(std::path::PathBuf);

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

    #[test]
    fn the_new_text_is_in_place_and_the_finished_file_is_gone() {
        let dir = Dir::new("text");
        let (beside, path) = (dir.0.join("canoa.cfg.new"), dir.0.join("canoa.cfg"));
        fs::write(&path, "old").expect("written");
        fs::write(&beside, "new").expect("written");
        replace_file(&beside, &path).expect("replaced");
        assert_eq!(fs::read_to_string(&path).expect("read"), "new");
        assert!(!beside.exists());
        fs::write(&beside, "first").expect("written");
        let fresh = dir.0.join("fresh.cfg");
        replace_file(&beside, &fresh).expect("made");
        assert_eq!(fs::read_to_string(&fresh).expect("read"), "first");
    }

    #[cfg(unix)]
    #[test]
    fn a_file_keeps_the_mode_it_had() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = Dir::new("mode");
        let (beside, path) = (dir.0.join("canoa.cfg.new"), dir.0.join("canoa.cfg"));
        for mode in [0o600, 0o644, 0o640] {
            fs::write(&path, "old").expect("written");
            fs::set_permissions(&path, fs::Permissions::from_mode(mode)).expect("mode");
            fs::write(&beside, "new").expect("written");
            fs::set_permissions(&beside, fs::Permissions::from_mode(0o666)).expect("mode");
            replace_file(&beside, &path).expect("replaced");
            let now = fs::metadata(&path).expect("metadata").permissions().mode() & 0o777;
            assert_eq!(now, mode, "{mode:o} became {now:o}");
        }
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;
    use std::process::Command;

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
        let (beside, path) = (dir.join("canoa.cfg.new"), dir.join("canoa.cfg"));
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
        fs::write(&beside, "new").expect("written");
        replace_file(&beside, &path).expect("replaced");
        assert_eq!(fs::read_to_string(&path).expect("read"), "new");
        assert_eq!(access_of(&path), before);
        let _ = fs::remove_dir_all(&dir);
    }
}
