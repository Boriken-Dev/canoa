use core::sync::atomic::{AtomicU64, Ordering};

use super::*;

/// A directory of the test's own, removed afterwards.
struct Under(PathBuf);

impl Under {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "canoa-private-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a directory");
        Self(dir)
    }
}

impl Drop for Under {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_directory_is_made_only_where_nothing_is_and_never_with_its_parents() {
    let under = Under::new();
    let once = under.0.join("a");
    make(&once).expect("made");
    assert_eq!(
        make(&once).expect_err("taken").kind(),
        std::io::ErrorKind::AlreadyExists,
        "a directory that is already there is not taken, whoever made it"
    );
    assert!(
        make(&under.0.join("missing").join("b")).is_err(),
        "no parent is made"
    );
    assert!(make_inside(&under.0.join("missing").join("b")).is_err());
    make_inside(&once.join("inner")).expect("made inside");
    assert!(make_inside(&once.join("inner")).is_err());
}

#[test]
fn a_private_directory_is_not_used_when_something_is_already_at_its_path_and_that_is_left_alone() {
    let under = Under::new();
    let private = Private::new("canoa-test");
    // Another user made it first.
    std::fs::create_dir_all(private.path()).expect("made first");
    std::fs::write(private.path().join("theirs"), b"x").expect("written");
    assert!(private.ensure().is_err());
    assert!(
        private.ensure().is_err(),
        "the answer is the same every time"
    );
    drop(private);
    let _ = under;
    // What was not made here is not removed here.
    let leftover = std::fs::read_dir(base())
        .expect("read")
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(&format!("canoa-test-{}-", std::process::id()))
        })
        .count();
    assert_eq!(
        leftover, 1,
        "the directory someone else made is still there"
    );
    for entry in std::fs::read_dir(base()).expect("read").flatten() {
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with(&format!("canoa-test-{}-", std::process::id()))
        {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

#[test]
fn a_private_directory_is_made_once_and_removed_with_all_it_holds() {
    let private = Private::new("canoa-test");
    let path = private.path().to_owned();
    assert!(!path.exists());
    private.ensure().expect("made");
    std::fs::write(path.join("file"), b"x").expect("written");
    assert_eq!(private.ensure().expect("again"), path);
    drop(private);
    assert!(!path.exists());
}

#[cfg(unix)]
#[test]
fn a_directory_is_this_users_alone_by_its_mode_and_not_a_link() {
    use std::os::unix::fs::PermissionsExt as _;
    let under = Under::new();
    let dir = under.0.join("d");
    make(&dir).expect("made");
    assert_eq!(
        std::fs::metadata(&dir).expect("meta").permissions().mode() & 0o777,
        0o700
    );
    check(&dir).expect("this user's alone");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).expect("changed");
    assert!(check(&dir).is_err(), "others can enter it");
    let link = under.0.join("l");
    std::os::unix::fs::symlink(&dir, &link).expect("linked");
    assert!(check(&link).is_err(), "a link is not a directory made here");
}

#[cfg(unix)]
#[test]
fn the_runtime_directory_is_preferred_where_there_is_one() {
    let under = Under::new();
    assert_eq!(
        base_from(Some(under.0.clone().into_os_string())),
        under.0,
        "a user's runtime directory, which only the user can write"
    );
    assert_eq!(
        base_from(Some("relative".into())),
        std::env::temp_dir(),
        "a path that is not absolute is not used"
    );
    assert_eq!(
        base_from(Some(under.0.join("none").into_os_string())),
        std::env::temp_dir()
    );
    assert_eq!(base_from(None), std::env::temp_dir());
}

#[cfg(windows)]
#[test]
fn a_directory_gives_no_right_to_anyone_but_this_user_and_the_system() {
    let under = Under::new();
    let dir = under.0.join("d");
    make(&dir).expect("made");
    let root = std::env::var_os("SystemRoot").expect("SystemRoot");
    let shown = std::process::Command::new(PathBuf::from(root).join("System32").join("icacls.exe"))
        .arg(&dir)
        .output()
        .expect("icacls");
    let rights = String::from_utf8_lossy(&shown.stdout).into_owned();
    for other in [
        "Everyone",
        "BUILTIN\\Users",
        "Authenticated Users",
        "BUILTIN\\Administrators",
    ] {
        assert!(!rights.contains(other), "{other} has rights: {rights}");
    }
    assert!(rights.contains("NT AUTHORITY\\SYSTEM"), "{rights}");
    // What is made inside inherits that.
    make_inside(&dir.join("inner")).expect("inside");
    let shown = std::process::Command::new(
        PathBuf::from(std::env::var_os("SystemRoot").expect("root"))
            .join("System32")
            .join("icacls.exe"),
    )
    .arg(dir.join("inner"))
    .output()
    .expect("icacls");
    let rights = String::from_utf8_lossy(&shown.stdout).into_owned();
    assert!(
        !rights.contains("BUILTIN\\Users") && !rights.contains("Everyone"),
        "{rights}"
    );
}
