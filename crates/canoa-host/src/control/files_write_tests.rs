use std::path::{Path, PathBuf};

use super::files::{Files, UploadLimits};
use super::*;

fn dir(name: &str) -> PathBuf {
    static NEXT: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("canoa-write-{name}-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("directory");
    dir
}

fn ask(control: &Control, caller: &Caller, method: &str, params: Value) -> Value {
    let line =
        serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}))
            .expect("encodes");
    serde_json::from_slice(&control.handle(caller, &line).expect("answered")).expect("JSON")
}

fn key(grants: &[Grant]) -> Caller {
    Caller::Peer {
        id: "sha256:aa".to_owned(),
        grants: grants.iter().copied().collect(),
    }
}

fn writer() -> Caller {
    key(&[Grant::Files, Grant::FilesWrite])
}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn why(answer: &Value) -> String {
    answer["error"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("an error: {answer}"))
        .to_owned()
}

/// A host with a home, a canoa.cfg and a state folder inside `tmp`, and the protected list `protected` (none: the recommended one).
struct Host {
    tmp: PathBuf,
    work: PathBuf,
    cfg: PathBuf,
    state: PathBuf,
    control: Control,
}

fn host(name: &str, protected: Option<Vec<String>>) -> Host {
    let tmp = dir(name);
    let (home, work, state) = (tmp.join("home"), tmp.join("work"), tmp.join("state"));
    for folder in [&home, &work, &state] {
        fs::create_dir_all(folder).expect("folder");
    }
    let cfg = home.join("canoa.cfg");
    fs::write(&cfg, "[host]\n").expect("canoa.cfg");
    let control = Control::open(
        None,
        DEFAULT_MAX_RECORDS,
        DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens")
    .with_files(
        Files::new(
            Some(home),
            Vec::new(),
            Some(cfg.clone()),
            Some(state.join("control").join("uploads")),
            UploadLimits::default(),
        )
        .with_protected(protected),
    );
    Host {
        tmp,
        work,
        cfg,
        state,
        control,
    }
}

fn write(host: &Host, caller: &Caller, path: &Path, body: &str) -> Value {
    ask(
        &host.control,
        caller,
        "fs/write",
        json!({"path": text(path), "text": body}),
    )
}

#[test]
fn writing_needs_files_and_files_write_together() {
    let host = host("grants", Some(Vec::new()));
    let target = host.work.join("a.txt");
    for grants in [
        &[][..],
        &[Grant::Files],
        &[Grant::FilesWrite],
        &[Grant::Folders, Grant::Upload],
        &[Grant::FilesWriteHost],
    ] {
        let answer = write(&host, &key(grants), &target, "x");
        assert_eq!(answer["error"]["code"], DENIED, "{answer}");
        assert!(!target.exists(), "nothing was written");
    }
    let done = write(&host, &writer(), &target, "hello\n");
    assert_eq!(done["result"]["size"], 6, "{done}");
    assert_eq!(fs::read_to_string(&target).expect("read"), "hello\n");
    let again = write(&host, &Caller::Owner, &target, "again");
    assert_eq!(again["result"]["size"], 5, "{again}");
    assert_eq!(Grant::named("files.write"), Some(Grant::FilesWrite));
    assert_eq!(
        Grant::named("files.write.host"),
        Some(Grant::FilesWriteHost)
    );
    let _ = fs::remove_dir_all(&host.tmp);
}

#[test]
fn it_refuses_what_is_not_a_regular_file_in_a_place_that_exists() {
    let host = host("shapes", Some(Vec::new()));
    let missing = host.work.join("nowhere").join("a.txt");
    let cases = [
        (json!({"path": "a.txt", "text": "x"}), "absolute"),
        (
            json!({"path": text(&host.work.join("..").join("a.txt")), "text": "x"}),
            "absolute",
        ),
        (json!({"path": text(&host.work)}), "text is needed"),
        (json!({"path": text(&host.work), "text": "x"}), "folder"),
        (
            json!({"path": text(&missing), "text": "x"}),
            "parent folder",
        ),
    ];
    for (params, words) in cases {
        let answer = ask(&host.control, &writer(), "fs/write", params);
        assert!(why(&answer).contains(words), "{words}: {answer}");
    }
    assert!(!missing.exists());
    let _ = fs::remove_dir_all(&host.tmp);
}

#[test]
fn a_place_on_the_persons_protected_list_is_never_written_from_here() {
    let host = host(
        "protected",
        Some(vec!["secret.txt".to_owned(), "locked/".to_owned()]),
    );
    fs::create_dir_all(host.work.join("locked")).expect("folder");
    fs::write(host.work.join("secret.txt"), "kept").expect("file");
    for target in [
        host.work.join("secret.txt"),
        host.work.join("locked").join("new.txt"),
    ] {
        // Neither the grants of an ordinary key nor all of them lift the person's own list.
        for caller in [writer(), key(&Grant::all()), Caller::Owner] {
            let answer = write(&host, &caller, &target, "changed");
            assert!(why(&answer).contains("protected list"), "{answer}");
        }
    }
    assert_eq!(
        fs::read_to_string(host.work.join("secret.txt")).expect("read"),
        "kept"
    );
    assert!(!host.work.join("locked").join("new.txt").exists());
    let ordinary = host.work.join("other.txt");
    assert!(write(&host, &writer(), &ordinary, "fine")["result"].is_object());
    let _ = fs::remove_dir_all(&host.tmp);
}

#[test]
fn the_recommended_list_applies_when_the_person_wrote_none() {
    let host = host("recommended", None);
    let target = host.work.join("package.json");
    let answer = write(&host, &writer(), &target, "{}");
    assert!(why(&answer).contains("protected list"), "{answer}");
    assert!(!target.exists());
    let _ = fs::remove_dir_all(&host.tmp);
}

#[test]
fn a_link_to_a_protected_place_is_the_protected_place() {
    let host = host("link", Some(vec!["secret.txt".to_owned()]));
    let real = host.work.join("secret.txt");
    fs::write(&real, "kept").expect("file");
    let link = host.work.join("innocent.txt");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&real, &link).expect("link");
    #[cfg(windows)]
    if std::os::windows::fs::symlink_file(&real, &link).is_err() {
        // Creating links needs Developer Mode or an elevated process; the check above is the same code path.
        let _ = fs::remove_dir_all(&host.tmp);
        return;
    }
    let answer = write(&host, &writer(), &link, "changed");
    assert!(why(&answer).contains("protected list"), "{answer}");
    assert_eq!(fs::read_to_string(&real).expect("read"), "kept");
    let _ = fs::remove_dir_all(&host.tmp);
}

#[test]
fn canoa_cfg_and_the_state_folder_need_the_dedicated_grant() {
    let host = host("kept", Some(Vec::new()));
    fs::create_dir_all(host.state.join("control")).expect("folder");
    let records = host.state.join("ledger.json");
    fs::write(&records, "{}").expect("file");
    let fresh = host.state.join("control").join("new.json");
    for target in [&host.cfg, &records, &fresh] {
        let answer = write(&host, &writer(), target, "[host]\nshell = \"x\"\n");
        assert!(why(&answer).contains("files.write.host"), "{answer}");
    }
    assert_eq!(fs::read_to_string(&host.cfg).expect("read"), "[host]\n");
    assert_eq!(fs::read_to_string(&records).expect("read"), "{}");
    assert!(!fresh.exists());
    let host_writer = key(&[Grant::Files, Grant::FilesWrite, Grant::FilesWriteHost]);
    for target in [&host.cfg, &records, &fresh] {
        let answer = write(&host, &host_writer, target, "changed");
        assert!(answer["result"].is_object(), "{answer}");
        assert_eq!(fs::read_to_string(target).expect("read"), "changed");
    }
    let _ = fs::remove_dir_all(&host.tmp);
}

#[cfg(unix)]
#[test]
fn a_file_keeps_its_mode_and_a_new_one_takes_the_default_of_the_place() {
    use std::os::unix::fs::PermissionsExt as _;

    let host = host("mode", Some(Vec::new()));
    let existing = host.work.join("script.sh");
    fs::write(&existing, "old").expect("file");
    fs::set_permissions(&existing, fs::Permissions::from_mode(0o750)).expect("mode");
    assert!(write(&host, &writer(), &existing, "new")["result"].is_object());
    let mode = |path: &Path| fs::metadata(path).expect("meta").permissions().mode() & 0o777;
    assert_eq!(mode(&existing), 0o750);
    assert_eq!(fs::read_to_string(&existing).expect("read"), "new");

    // What any program's new file gets here, the process's umask deciding.
    let plain = host.work.join("plain.txt");
    fs::write(&plain, "x").expect("file");
    let made = host.work.join("made.txt");
    assert!(write(&host, &writer(), &made, "x")["result"].is_object());
    assert_eq!(mode(&made), mode(&plain));
    let _ = fs::remove_dir_all(&host.tmp);
}
