//! The programs folder (`Shelf`) and the before/after snapshots (`Footprint`).
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a test names what it uses from std"
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use canoa::store::{Change, Launch, PATH, PROFILE_FILE, REGISTRY, STATE_DIR, USER_PATH};
use canoa_host::store::footprint::{Footprint, Watch, uncovered};
use canoa_host::store::shelf::{MadeBy, Shelf, ShelfError};

struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        let mut nonce = [0u8; 4];
        getrandom::fill(&mut nonce).expect("random");
        let hex: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
        let path = std::env::temp_dir().join(format!("canoa-shelf-{name}-{hex}"));
        fs::create_dir_all(&path).expect("a directory");
        Self(path)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn by() -> MadeBy {
    MadeBy {
        provider: "echo_store".to_owned(),
        version: "0".to_owned(),
    }
}

/// Stages `version` with a program file in it and commits it.
fn install(shelf: &Shelf, version: &str) -> PathBuf {
    let program = shelf.program("echo").expect("program");
    let lock = program.lock().expect("lock");
    let staging = lock.stage(version).expect("stage");
    let exe = staging.join("echo.bin");
    fs::write(&exe, version).expect("file");
    let launch = Launch::new(exe.to_string_lossy());
    lock.commit(&staging, version, &launch, Vec::new(), by())
        .expect("commit");
    staging
}

#[test]
fn a_commit_makes_the_copy_current_and_the_old_one_previous() {
    let dir = Dir::new("commit");
    let shelf = Shelf::new(dir.0.join("programs"));
    let one = install(&shelf, "1.0.0");
    let two = install(&shelf, "2.0.0");
    let current = shelf
        .program("echo")
        .expect("p")
        .current()
        .expect("read")
        .expect("current");
    assert_eq!(current.version, "2.0.0");
    assert_eq!(PathBuf::from(&current.dir), two);
    assert_eq!(current.previous.expect("previous").version, "1.0.0");
    assert!(one.is_dir(), "the previous version stays");
}

#[test]
fn rollback_swaps_and_a_third_version_prunes_the_oldest() {
    let dir = Dir::new("rollback");
    let shelf = Shelf::new(dir.0.join("programs"));
    let one = install(&shelf, "1.0.0");
    let two = install(&shelf, "2.0.0");
    {
        let program = shelf.program("echo").expect("p");
        let lock = program.lock().expect("lock");
        let back = lock.rollback().expect("rollback");
        assert_eq!(back.version, "1.0.0");
        assert_eq!(back.previous.expect("prev").version, "2.0.0");
    }
    install(&shelf, "3.0.0");
    assert!(
        one.exists(),
        "the rolled-back version is the new previous one"
    );
    assert!(
        !two.exists(),
        "a version that is neither current nor previous is pruned (keep two)"
    );
}

#[test]
fn nothing_to_roll_back_to_is_an_error() {
    let dir = Dir::new("noprev");
    let shelf = Shelf::new(dir.0.join("programs"));
    install(&shelf, "1.0.0");
    let program = shelf.program("echo").expect("p");
    let lock = program.lock().expect("lock");
    assert!(matches!(lock.rollback(), Err(ShelfError::Nothing(_))));
}

#[test]
fn a_crashed_operation_leaves_a_folder_the_next_locked_operation_removes() {
    let dir = Dir::new("crash");
    let shelf = Shelf::new(dir.0.join("programs"));
    install(&shelf, "1.0.0");
    let leftover = {
        let program = shelf.program("echo").expect("p");
        let lock = program.lock().expect("lock");
        let staging = lock.stage("2.0.0").expect("stage");
        fs::write(staging.join("half"), b"x").expect("partial");
        staging
        // the lock drops with no commit and no abandon: a crash
    };
    assert!(leftover.exists());
    let program = shelf.program("echo").expect("p");
    let _lock = program.lock().expect("a later lock");
    assert!(!leftover.exists(), "the next locked operation removes it");
}

#[test]
fn a_failed_commit_removes_the_staged_folder_and_keeps_current() {
    let dir = Dir::new("badlaunch");
    let shelf = Shelf::new(dir.0.join("programs"));
    install(&shelf, "1.0.0");
    let program = shelf.program("echo").expect("p");
    let lock = program.lock().expect("lock");
    let staging = lock.stage("2.0.0").expect("stage");
    let outside = dir.0.join("elsewhere.bin");
    fs::write(&outside, b"x").expect("file");
    let launch = Launch::new(outside.to_string_lossy());
    let error = lock
        .commit(&staging, "2.0.0", &launch, Vec::new(), by())
        .expect_err("outside");
    assert!(matches!(error, ShelfError::Refused(_)), "{error}");
    assert!(!staging.exists());
    assert_eq!(
        program.current().expect("read").expect("current").version,
        "1.0.0"
    );
}

#[test]
fn a_second_lock_on_one_program_is_refused_while_the_first_is_held() {
    let dir = Dir::new("held");
    let shelf = Shelf::new(dir.0.join("programs"));
    let program = shelf.program("echo").expect("p");
    let _first = program.lock().expect("lock");
    assert!(matches!(program.lock(), Err(ShelfError::Held)));
}

#[test]
fn current_json_is_always_whole_for_a_reader_while_versions_switch() {
    use std::sync::atomic::AtomicU32;
    let dir = Dir::new("atomic");
    let shelf = Shelf::new(dir.0.join("programs"));
    install(&shelf, "1.0.0");
    let stop = Arc::new(AtomicBool::new(false));
    let reads = Arc::new(AtomicU32::new(0));
    let reader = {
        let (stop, reads) = (Arc::clone(&stop), Arc::clone(&reads));
        let program = shelf.program("echo").expect("p");
        thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                let current = program.current().expect("a whole file").expect("present");
                assert!(current.version.ends_with(".0.0"));
                reads.fetch_add(1, Ordering::SeqCst);
            }
        })
    };
    // The switches start once the reader is reading, and the reader goes on until it has read after the last one.
    while reads.load(Ordering::SeqCst) == 0 {
        thread::yield_now();
    }
    for n in 2..8 {
        install(&shelf, &format!("{n}.0.0"));
    }
    let seen = reads.load(Ordering::SeqCst);
    while reads.load(Ordering::SeqCst) < seen + 2 {
        thread::yield_now();
    }
    stop.store(true, Ordering::SeqCst);
    reader.join().expect("the reader never read a partial file");
}

#[test]
fn removing_a_copy_deletes_current_first_and_then_every_folder() {
    let dir = Dir::new("remove");
    let shelf = Shelf::new(dir.0.join("programs"));
    let one = install(&shelf, "1.0.0");
    let program = shelf.program("echo").expect("p");
    program.lock().expect("lock").remove().expect("remove");
    assert!(program.current().expect("read").is_none());
    assert!(!one.exists());
}

#[test]
fn a_name_that_is_not_a_program_folder_name_is_refused() {
    let shelf = Shelf::new(std::env::temp_dir());
    for name in ["", "..", "a/b", "A", "x y", &"a".repeat(65)] {
        assert!(
            matches!(shelf.program(name), Err(ShelfError::Name)),
            "{name:?}"
        );
    }
}

#[cfg(unix)]
#[test]
fn a_folder_others_can_write_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    let dir = Dir::new("open");
    let shelf = Shelf::new(dir.0.join("programs"));
    let program = shelf.program("echo").expect("p");
    drop(program.lock().expect("lock"));
    fs::set_permissions(program.dir(), fs::Permissions::from_mode(0o777)).expect("chmod");
    assert!(matches!(program.lock(), Err(ShelfError::NotPrivate(_))));
}

/// The tests that plant values in the real registry take turns: each puts back what it finds changed, which would undo the other's.
#[cfg(windows)]
static REGISTRY_TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn watch(home: &Path, bin: &Path) -> Watch {
    Watch::new(home).path_dirs([bin.to_path_buf()])
}

#[test]
fn a_profile_append_a_new_file_in_a_path_directory_and_a_new_home_file_are_each_detected() {
    let dir = Dir::new("footprint");
    let (home, bin) = (dir.0.join("home"), dir.0.join("bin"));
    fs::create_dir_all(&home).expect("home");
    fs::create_dir_all(&bin).expect("bin");
    fs::write(home.join(".bashrc"), "# mine\n").expect("rc");
    let before = Footprint::take(&watch(&home, &bin));
    assert!(
        before
            .diff(&Footprint::take(&watch(&home, &bin)))
            .is_empty(),
        "no change, no difference"
    );
    fs::write(home.join(".bashrc"), "# mine\nexport PATH=$HOME/x:$PATH\n").expect("append");
    fs::write(bin.join("tool"), b"x").expect("tool");
    fs::write(home.join("stray"), b"x").expect("stray");
    let diff = before.diff(&Footprint::take(&watch(&home, &bin)));
    let kinds: Vec<_> = diff
        .iter()
        .map(|d| {
            (
                d.kind,
                d.what.rsplit(['/', '\\']).next().unwrap_or("").to_owned(),
            )
        })
        .collect();
    assert!(
        kinds.contains(&(PROFILE_FILE, ".bashrc".to_owned())),
        "{kinds:?}"
    );
    assert!(kinds.contains(&(PATH, "tool".to_owned())), "{kinds:?}");
    assert!(kinds.contains(&(PATH, "stray".to_owned())), "{kinds:?}");
}

#[test]
fn a_declared_change_covers_its_kind_and_nothing_else() {
    let dir = Dir::new("covers");
    let (home, bin) = (dir.0.join("home"), dir.0.join("bin"));
    fs::create_dir_all(&home).expect("home");
    fs::create_dir_all(&bin).expect("bin");
    let before = Footprint::take(&watch(&home, &bin));
    fs::write(home.join(".zshrc"), "export PATH=$HOME/x:$PATH\n").expect("rc");
    fs::write(bin.join("tool"), b"x").expect("tool");
    let diff = before.diff(&Footprint::take(&watch(&home, &bin)));
    assert_eq!(
        uncovered(&diff, &[]).len(),
        diff.len(),
        "nothing declared covers nothing"
    );
    let rc = Change::new(
        PROFILE_FILE,
        home.join(".zshrc").to_string_lossy(),
        "adds a line",
        true,
    );
    let left = uncovered(&diff, std::slice::from_ref(&rc));
    assert!(left.iter().all(|d| d.kind == PATH), "{left:?}");
    let dir_change = Change::new(PATH, dir.0.to_string_lossy(), "a launcher", false);
    assert!(uncovered(&diff, &[rc, dir_change]).is_empty());
    let state = Change::new(STATE_DIR, dir.0.to_string_lossy(), "state", false);
    assert!(
        uncovered(&diff, &[state])
            .iter()
            .all(|d| d.kind == PROFILE_FILE)
    );
    let user_path = Change::new(USER_PATH, "PATH", "adds a directory", true);
    assert!(
        uncovered(&diff, &[user_path])
            .iter()
            .all(|d| d.kind == PATH)
    );
    let registry = Change::new(REGISTRY, "HKCU\\Environment", "a value", true);
    assert_eq!(uncovered(&diff, &[registry]).len(), diff.len());
}

#[cfg(windows)]
#[test]
fn a_registry_value_a_suite_owns_is_detected_restored_and_a_path_rewrite_is_a_user_path_difference()
{
    use windows_registry::CURRENT_USER;
    let _turn = REGISTRY_TURN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = Dir::new("registry");
    let (home, bin) = (dir.0.join("home"), dir.0.join("bin"));
    fs::create_dir_all(&home).expect("home");
    fs::create_dir_all(&bin).expect("bin");
    let name = format!("COQUI_STORE_CHECK_{}", std::process::id());
    let before = Footprint::take(&watch(&home, &bin).registry(true));
    let key = CURRENT_USER.create("Environment").expect("key");
    key.set_string(&name, "planted").expect("set");
    let after = Footprint::take(&watch(&home, &bin).registry(true));
    let diff = before.diff(&after);
    assert!(
        diff.iter()
            .any(|d| d.kind == REGISTRY && d.what.ends_with(&name)),
        "{diff:?}"
    );
    assert_eq!(before.restore_registry(&after), 1);
    assert!(key.get_string(&name).is_err(), "the value was removed");
}

#[test]
fn the_places_a_login_or_a_session_start_reads_are_watched() {
    let dir = Dir::new("wide");
    let (home, bin) = (dir.0.join("home"), dir.0.join("bin"));
    fs::create_dir_all(home.join(".ssh")).expect("ssh");
    fs::create_dir_all(home.join(".config/autostart")).expect("autostart");
    fs::create_dir_all(&bin).expect("bin");
    let before = Footprint::take(&watch(&home, &bin));
    fs::write(home.join(".zshenv"), "x").expect("zshenv");
    fs::write(home.join(".gitconfig"), "[core]\n").expect("gitconfig");
    fs::write(home.join(".ssh/authorized_keys"), "ssh-ed25519 AAAA\n").expect("keys");
    fs::write(
        home.join(".config/autostart/x.desktop"),
        "[Desktop Entry]\n",
    )
    .expect("autostart");
    let names: Vec<String> = before
        .diff(&Footprint::take(&watch(&home, &bin)))
        .into_iter()
        .map(|d| d.what.replace('\\', "/"))
        .collect();
    for wanted in [
        ".zshenv",
        ".gitconfig",
        ".ssh/authorized_keys",
        ".config/autostart/x.desktop",
    ] {
        assert!(
            names.iter().any(|n| n.ends_with(wanted)),
            "{wanted} not seen in {names:?}"
        );
    }
}

#[test]
fn a_user_path_change_covers_a_profile_only_when_every_changed_line_is_about_path() {
    let dir = Dir::new("lines");
    let (home, bin) = (dir.0.join("home"), dir.0.join("bin"));
    fs::create_dir_all(&home).expect("home");
    fs::create_dir_all(&bin).expect("bin");
    fs::write(home.join(".bashrc"), "# mine\n").expect("rc");
    let before = Footprint::take(&watch(&home, &bin));
    fs::write(home.join(".bashrc"), "# mine\nexport PATH=$HOME/x:$PATH\n").expect("rc");
    let declared = [Change::new(USER_PATH, "PATH", "adds a directory", true)];
    let diff = before.diff(&Footprint::take(&watch(&home, &bin)));
    assert!(
        uncovered(&diff, &declared).is_empty(),
        "a PATH line is covered"
    );
    let before = Footprint::take(&watch(&home, &bin));
    fs::write(
        home.join(".bashrc"),
        "# mine\nexport PATH=$HOME/x:$PATH\nalias ll='ls -l'\n",
    )
    .expect("rc");
    let diff = before.diff(&Footprint::take(&watch(&home, &bin)));
    assert_eq!(uncovered(&diff, &declared).len(), 1, "an alias line is not");
}

#[test]
fn a_programs_root_outside_the_default_is_not_accepted() {
    let outside = std::env::temp_dir().join("canoa-programs-elsewhere");
    assert!(matches!(
        Shelf::configured(&outside),
        Err(ShelfError::NotPrivate(_))
    ));
    if let Some(default) = Shelf::default_root() {
        assert!(Shelf::configured(default.join("sub")).is_ok());
        assert!(Shelf::configured(&default).is_ok());
    }
}

#[test]
fn a_recorded_file_that_changed_is_found_before_a_launch() {
    let dir = Dir::new("verify");
    let shelf = Shelf::new(dir.0.join("programs"));
    let program = shelf.program("echo").expect("p");
    let lock = program.lock().expect("lock");
    let staging = lock.stage("1.0.0").expect("stage");
    let exe = staging.join("echo.bin");
    fs::write(&exe, b"the bytes that were checked").expect("file");
    let digest = {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(b"the bytes that were checked"))
    };
    let record = canoa_host::store::shelf::FileRecord {
        path: "echo.bin".to_owned(),
        url: "https://example.test/echo".to_owned(),
        sha256: digest,
        verified: true,
    };
    let launch = Launch::new(exe.to_string_lossy());
    let current = lock
        .commit(&staging, "1.0.0", &launch, vec![record], by())
        .expect("commit");
    program.verify(&current).expect("untouched");
    fs::write(&exe, b"swapped").expect("tamper");
    assert!(matches!(
        program.verify(&current),
        Err(ShelfError::Refused(_))
    ));
}

#[cfg(windows)]
#[test]
fn a_run_key_value_is_detected_and_put_back() {
    use windows_registry::CURRENT_USER;
    let _turn = REGISTRY_TURN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = Dir::new("runkey");
    let (home, bin) = (dir.0.join("home"), dir.0.join("bin"));
    fs::create_dir_all(&home).expect("home");
    fs::create_dir_all(&bin).expect("bin");
    let name = format!("COQUI_STORE_CHECK_RUN_{}", std::process::id());
    let path = r"Software\Microsoft\Windows\CurrentVersion\Run";
    let before = Footprint::take(&watch(&home, &bin).registry(true));
    let key = CURRENT_USER.create(path).expect("key");
    key.set_string(&name, "calc.exe").expect("set");
    let after = Footprint::take(&watch(&home, &bin).registry(true));
    assert!(before.diff(&after).iter().any(|d| d.what.ends_with(&name)));
    assert!(before.restore_registry(&after) >= 1);
    assert!(
        key.get_string(&name).is_err(),
        "the value was taken out again"
    );
}
