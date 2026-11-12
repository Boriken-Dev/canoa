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
fn the_app_starts_in_its_own_folder_and_keeps_the_config_path_it_was_given() {
    let root = scratch("cwd");
    let launcher = carrying_launcher(&root);
    let home = root.join("home");
    fs::create_dir(&home).unwrap();
    // Started from a folder a download may have filled, with a config path relative to it.
    let downloads = root.join("downloads");
    fs::create_dir(&downloads).unwrap();
    let mut command = Command::new(&launcher);
    command
        .current_dir(&downloads)
        .env("CANOA_CONFIG", "mine.cfg")
        .env("CANOA_LAUNCH_NO_DIALOG", "1");
    command.env(
        if cfg!(windows) {
            "LOCALAPPDATA"
        } else {
            "XDG_DATA_HOME"
        },
        &home,
    );
    let out = command.output().unwrap();
    let said = text(&out.stdout);
    let unpacked = home.join("Canoa").join(VERSION);
    assert!(
        said.contains(&format!("cwd={}", unpacked.display())),
        "{said} {}",
        text(&out.stderr)
    );
    let wanted = downloads.join("mine.cfg");
    assert!(
        said.contains(&format!("config={}", wanted.display())),
        "{said}"
    );
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

/// The names of the DLLs the executable imports, lower-cased.
#[cfg(windows)]
fn imported_libraries(exe: &Path) -> Vec<String> {
    use object::Object;
    let data = fs::read(exe).unwrap();
    let file = object::File::parse(&*data).unwrap();
    file.imports()
        .unwrap()
        .filter_map(Result::ok)
        .map(|import| String::from_utf8_lossy(import.library()).to_lowercase())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// A start that is given 60 seconds: a loader failure in a program with no console can wait on a dialog nobody answers.
#[cfg(windows)]
fn start_within_a_minute(launcher: &Path, data_home: &Path, args: &[&str]) -> Option<i32> {
    let mut child = Command::new(launcher)
        .args(args)
        .env("CANOA_LAUNCH_NO_DIALOG", "1")
        .env("LOCALAPPDATA", data_home)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    for _ in 0..600 {
        if let Some(status) = child.try_wait().unwrap() {
            return status.code();
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
    None
}

#[cfg(windows)]
#[test]
fn a_dll_planted_beside_the_launcher_is_not_loaded() {
    let root = scratch("planted");
    let built = carrying_launcher(&root);
    let only = root.join("only");
    fs::create_dir(&only).unwrap();
    let launcher = only.join(EXE);
    fs::copy(&built, &launcher).unwrap();

    // No runtime library to plant: the C runtime is linked in.
    let imports = imported_libraries(&launcher);
    let runtime = |name: &String| {
        [
            "vcruntime",
            "msvcp",
            "ucrtbase",
            "api-ms-win-crt",
            "concrt",
            "vcomp",
        ]
        .iter()
        .any(|p| name.starts_with(p))
    };
    let found: Vec<_> = imports.iter().filter(|name| runtime(name)).collect();
    assert!(
        found.is_empty(),
        "the launcher imports {found:?} (all: {imports:?})"
    );

    // What it still imports is looked for in System32 alone: the load configuration's DependentLoadFlags is
    // LOAD_LIBRARY_SEARCH_SYSTEM32.
    let data = fs::read(&launcher).unwrap();
    let pe = object::read::pe::PeFile64::parse(&*data).unwrap();
    let config = pe
        .data_directories()
        .get(object::pe::IMAGE_DIRECTORY_ENTRY_LOAD_CONFIG)
        .unwrap();
    let config = config.data(&*data, &pe.section_table()).unwrap();
    assert_eq!(
        u16::from_le_bytes([config[0x4e], config[0x4f]]),
        0x800,
        "DependentLoadFlags"
    );

    // Junk under the name of every library it does import, and of the runtime's, where a download could have put it.
    let mut names: Vec<String> = imports.clone();
    names.extend(["vcruntime140.dll", "vcruntime140_1.dll", "msvcp140.dll"].map(String::from));
    for name in &names {
        fs::write(only.join(name), b"MZ this is not a library").unwrap();
    }
    let home = root.join("home");
    fs::create_dir(&home).unwrap();
    assert_eq!(
        start_within_a_minute(&launcher, &home, &["--exit", "5"]),
        Some(5),
        "imports: {imports:?}"
    );
    fs::remove_dir_all(&root).unwrap();
}
