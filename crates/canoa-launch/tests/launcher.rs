//! The launcher program itself: built with a payload that carries a stand-in for the app, copied alone into an empty folder
//! and started. Needs the example built (`cargo test` builds it; a narrower run needs `--examples`), and builds the launcher
//! once more, with the payload, into its own target folder.

use canoa_launch::pack;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const EXE: &str = if cfg!(windows) {
    "CanoaUI.exe"
} else {
    "CanoaUI"
};
const VERSION: &str = "9.8.7-test";

fn scratch(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("canoa-launcher-test-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn stand_in() -> PathBuf {
    let test = std::env::current_exe().unwrap();
    let examples = test.parent().unwrap().parent().unwrap().join("examples");
    let name = if cfg!(windows) {
        "stand_in.exe"
    } else {
        "stand_in"
    };
    let path = examples.join(name);
    assert!(
        path.is_file(),
        "{} is missing: build the examples (cargo test -p canoa-launch builds them)",
        path.display()
    );
    path
}

/// A launcher carrying a payload made of the stand-in under the app's name.
fn carrying_launcher(root: &Path) -> PathBuf {
    let app = root.join("app");
    fs::create_dir_all(app.join("data")).unwrap();
    // The launcher starts `CanoaUI.exe` by that name on every platform it is built for.
    fs::copy(stand_in(), app.join(canoa_launch::APP_PROGRAM)).unwrap();
    fs::write(app.join("canoa_ffi.dll"), "library").unwrap();
    fs::write(app.join("data/icudtl.dat"), vec![1u8; 100_000]).unwrap();
    let payload = root.join("payload.zip");
    fs::write(&payload, pack(&app, VERSION).unwrap()).unwrap();

    let target = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("launcher-with-payload");
    let built = Command::new(std::env::var_os("CARGO").unwrap())
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("CANOA_LAUNCH_PAYLOAD", &payload)
        .args([
            "build",
            "--locked",
            "-p",
            "canoa-launch",
            "--bin",
            "canoa-launch",
            "--target-dir",
        ])
        .arg(&target)
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    target.join("debug").join(if cfg!(windows) {
        "canoa-launch.exe"
    } else {
        "canoa-launch"
    })
}

fn start(launcher: &Path, data_home: &Path, args: &[&str]) -> Output {
    let mut command = Command::new(launcher);
    command.args(args).env("CANOA_LAUNCH_NO_DIALOG", "1");
    if cfg!(windows) {
        command.env("LOCALAPPDATA", data_home);
    } else {
        command.env("XDG_DATA_HOME", data_home);
    }
    command.output().unwrap()
}

fn text(out: &[u8]) -> String {
    String::from_utf8_lossy(out).into_owned()
}

#[test]
fn a_lone_launcher_unpacks_runs_the_app_and_reuses_the_copy() {
    let root = scratch("lone");
    let built = carrying_launcher(&root);
    // Only the launcher, alone in an empty folder.
    let only = root.join("only");
    fs::create_dir(&only).unwrap();
    let launcher = only.join(EXE);
    fs::copy(&built, &launcher).unwrap();
    let home = root.join("home");
    fs::create_dir(&home).unwrap();
    let unpacked = home.join("Canoa").join(VERSION);

    let first = start(&launcher, &home, &["--canoa-child", "peers"]);
    assert_eq!(
        first.status.code(),
        Some(2),
        "the app's status is the launcher's: {}",
        text(&first.stderr)
    );
    let out = text(&first.stdout);
    assert!(out.contains("library=true"), "{out}");
    assert!(out.contains(r#"args=["--canoa-child", "peers"]"#), "{out}");
    assert!(
        out.contains(&unpacked.display().to_string()),
        "ran from {} : {out}",
        unpacked.display()
    );
    assert!(unpacked.join("canoa_ffi.dll").is_file() && unpacked.join("data/icudtl.dat").is_file());
    assert_eq!(
        fs::read_dir(&only).unwrap().count(),
        1,
        "nothing is written beside the launcher"
    );

    let stamp = fs::metadata(unpacked.join("canoa_ffi.dll"))
        .unwrap()
        .modified()
        .unwrap();
    let second = start(&launcher, &home, &["--exit", "5"]);
    assert_eq!(second.status.code(), Some(5));
    assert_eq!(
        fs::metadata(unpacked.join("canoa_ffi.dll"))
            .unwrap()
            .modified()
            .unwrap(),
        stamp,
        "reused, not unpacked again"
    );

    // A library changed after unpacking is put back, and a planted one is removed, before the app starts.
    fs::write(unpacked.join("canoa_ffi.dll"), "evil").unwrap();
    fs::write(unpacked.join("planted.dll"), "evil").unwrap();
    let third = start(&launcher, &home, &[]);
    assert_eq!(third.status.code(), Some(0), "{}", text(&third.stderr));
    assert_eq!(
        fs::read_to_string(unpacked.join("canoa_ffi.dll")).unwrap(),
        "library"
    );
    assert!(!unpacked.join("planted.dll").exists());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_launcher_started_where_the_version_folder_is_a_link_runs_nothing() {
    let root = scratch("linked");
    let launcher = carrying_launcher(&root);
    let home = root.join("home");
    let base = home.join("Canoa");
    fs::create_dir_all(&base).unwrap();
    let victim = root.join("victim");
    fs::create_dir(&victim).unwrap();
    // Plants the stand-in where a link would lead, so a launcher that followed it would run it.
    fs::copy(stand_in(), victim.join(canoa_launch::APP_PROGRAM)).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&victim, base.join(VERSION)).unwrap();
    #[cfg(windows)]
    assert!(
        Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(base.join(VERSION))
            .arg(&victim)
            .output()
            .unwrap()
            .status
            .success()
    );

    let out = start(&launcher, &home, &[]);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("link"), "{}", text(&out.stderr));
    assert!(
        !text(&out.stdout).contains("stand-in"),
        "the app ran through the link"
    );
    assert_eq!(fs::read_dir(&victim).unwrap().count(), 1);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_launcher_built_without_a_payload_says_so_and_runs_nothing() {
    let root = scratch("empty");
    let out = start(Path::new(env!("CARGO_BIN_EXE_canoa-launch")), &root, &[]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        text(&out.stderr).contains("carries no app"),
        "{}",
        text(&out.stderr)
    );
    assert!(!root.join("Canoa").exists());
    fs::remove_dir_all(&root).unwrap();
}
