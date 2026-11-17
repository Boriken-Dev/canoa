//! The folder methods against a folder on a Windows share: this machine's own administrative share, so nothing leaves it.
use std::path::{Path, PathBuf};
use std::process::Command;

use super::files::{Files, UploadLimits};
use super::*;

/// A new folder on the administrative share of the drive the temporary folder is on, as `\localhost\C$\...`.
fn shared(name: &str) -> PathBuf {
    static NEXT: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    let local = std::env::temp_dir().join(format!("canoa-unc-{name}-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&local);
    fs::create_dir_all(&local).expect("directory");
    let text = local.to_string_lossy().into_owned();
    let (drive, rest) = text.split_at(2);
    let share = PathBuf::from(format!(r"\\localhost\{}${rest}", &drive[..1]));
    assert!(
        share.is_dir(),
        "{} needs the administrative share",
        share.display()
    );
    share
}

fn local_of(share: &Path) -> PathBuf {
    let text = share.to_string_lossy().into_owned();
    let rest = text.strip_prefix(r"\\localhost\").expect("a share");
    PathBuf::from(format!("{}:{}", &rest[..1], &rest[2..]))
}

fn ask(control: &Control, method: &str, params: Value) -> Value {
    let line =
        serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}))
            .expect("encodes");
    serde_json::from_slice(&control.handle(&Caller::Owner, &line).expect("answered")).expect("JSON")
}

fn control(home: &Path) -> Control {
    let files = Files::new(
        Some(home.to_path_buf()),
        Vec::new(),
        None,
        None,
        UploadLimits::default(),
    );
    Control::open(
        None,
        DEFAULT_MAX_RECORDS,
        DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens")
    .with_files(files)
}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn git(folder: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(folder)
        .args(args)
        .output()
        .is_ok_and(|done| done.status.success())
}

fn clean(share: &Path) {
    let _ = fs::remove_dir_all(local_of(share));
}

#[test]
fn a_share_folder_is_listed_and_named_as_written() {
    let work = shared("list");
    fs::create_dir_all(work.join("src")).expect("folder");
    fs::write(work.join("notes.txt"), "x").expect("file");
    let c = control(&std::env::temp_dir());
    let said = ask(&c, "fs/list", json!({"path": text(&work), "files": true}));
    assert_eq!(said["result"]["path"], text(&work), "{said}");
    let names: Vec<&str> = said["result"]["entries"]
        .as_array()
        .unwrap_or_else(|| panic!("{said}"))
        .iter()
        .filter_map(|entry| entry["name"].as_str())
        .collect();
    assert!(
        names.contains(&"src") && names.contains(&"notes.txt"),
        "{said}"
    );
    let verbatim = format!(r"\\?\UNC\{}", &text(&work)[2..]);
    let said = ask(&c, "fs/list", json!({"path": verbatim}));
    assert_eq!(said["result"]["path"], text(&work), "{said}");
    let said = ask(&c, "fs/list", json!({"path": text(&work.join("src"))}));
    assert_eq!(said["result"]["path"], text(&work.join("src")), "{said}");
    clean(&work);
}

#[test]
fn a_share_folder_is_a_repository_and_mentions_its_files() {
    let work = shared("repo");
    fs::create_dir_all(work.join("src")).expect("folder");
    fs::write(work.join("src").join("lib.rs"), "x").expect("file");
    fs::write(work.join("notes.txt"), "x").expect("file");
    fs::write(work.join(".gitignore"), "ignored.log\n").expect("file");
    fs::write(work.join("ignored.log"), "x").expect("file");
    assert!(git(&local_of(&work), &["init", "-q"]), "git init");
    let c = control(&std::env::temp_dir());
    let said = ask(&c, "fs/info", json!({"paths": [text(&work.join("src"))]}));
    let found = &said["result"]["folders"][0];
    assert_eq!(found["exists"], true, "{said}");
    assert_eq!(found["repository"], true, "{said}");
    assert_eq!(found["root"], text(&work), "{said}");
    let said = ask(
        &c,
        "fs/repo_root",
        json!({"paths": [text(&work.join("src"))]}),
    );
    assert_eq!(said["result"]["folders"][0]["root"], text(&work), "{said}");
    let said = ask(
        &c,
        "fs/realpath",
        json!({"paths": [text(&work.join("src"))]}),
    );
    assert_eq!(
        said["result"]["paths"][0]["real"],
        text(&work.join("src")),
        "{said}"
    );
    let said = ask(&c, "fs/mentions", json!({"path": text(&work)}));
    let all: Vec<&str> = said["result"]["entries"]
        .as_array()
        .unwrap_or_else(|| panic!("{said}"))
        .iter()
        .filter_map(|entry| entry.as_str())
        .collect();
    assert!(
        all.contains(&"notes.txt") && all.contains(&"src/lib.rs"),
        "{said}"
    );
    assert!(
        !all.contains(&"ignored.log"),
        "git names the files, so an ignored one is not: {said}"
    );
    let said = ask(
        &c,
        "fs/resolve",
        json!({"path": text(&work), "name": "src/lib.rs"}),
    );
    assert!(said.get("error").is_none(), "{said}");
    clean(&work);
}

#[test]
fn a_chats_folder_is_made_and_taken_back_on_a_share() {
    let parent = shared("scratch");
    let local = local_of(&parent);
    let template = format!("{}\\${{session}}", text(&parent));
    let config = local.join("canoa.cfg");
    fs::write(&config, format!("[sessions]\nworkspace = {template:?}\n")).expect("config");
    let files = Files::new(
        Some(local.clone()),
        Vec::new(),
        Some(config),
        None,
        UploadLimits::default(),
    );
    let c = Control::open(
        None,
        DEFAULT_MAX_RECORDS,
        DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens")
    .with_files(files);
    let made = ask(&c, "fs/scratch", json!({"session": "unc1"}));
    assert_eq!(made["result"]["made"], true, "{made}");
    let folder = local.join("unc1");
    assert!(folder.is_dir(), "{made}");
    let gone = ask(&c, "fs/scratch_remove", json!({"session": "unc1"}));
    assert_eq!(gone["result"]["removed"], true, "{gone}");
    clean(&parent);
}

fn overlay_host(tmp: &Path) -> Control {
    Control::open(
        None,
        DEFAULT_MAX_RECORDS,
        DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens")
    .with_files(Files::new(
        Some(tmp.to_path_buf()),
        Vec::new(),
        None,
        None,
        UploadLimits::default(),
    ))
    .with_overlays(super::overlays::Overlays::new(
        Some(tmp.join("cfg").join("overlays")),
        super::overlays::OverlayLimits::default(),
    ))
}

/// A repository made on this machine's disk and named through the administrative share.
fn shared_repo(files: &[(&str, &str)]) -> (PathBuf, PathBuf) {
    let share = shared("repo-source");
    let local = local_of(&share);
    super::overlays_tests::repo(&local, "tools", files);
    (share.join("tools"), share)
}

#[test]
fn a_skill_is_fetched_from_a_git_source_on_a_share() {
    let (upstream, repo) = shared_repo(&[
        (
            "skills/review/SKILL.md",
            "---\nname: review\ndescription: Reviews.\n---\nbody\n",
        ),
        ("skills/review/run.sh", "echo hi\n"),
    ]);
    let tmp = std::env::temp_dir().join(format!("canoa-unc-skill-{}", std::process::id()));
    fs::create_dir_all(&tmp).expect("folder");
    let c = overlay_host(&tmp);
    let said = ask(
        &c,
        "skill/fetch",
        super::overlays_tests::source(&upstream, "skills/review"),
    );
    let files = said["result"]["files"]
        .as_object()
        .unwrap_or_else(|| panic!("{said}"));
    assert!(
        files.contains_key("SKILL.md") && files.contains_key("run.sh"),
        "{said}"
    );
    let _ = fs::remove_dir_all(&tmp);
    clean(&repo);
}

#[test]
fn an_overlay_is_checked_from_a_git_source_on_a_share() {
    let (upstream, repo) = shared_repo(&[("OVERLAY.md", "# team\n")]);
    let tmp = std::env::temp_dir().join(format!("canoa-unc-overlay-{}", std::process::id()));
    fs::create_dir_all(&tmp).expect("folder");
    let c = overlay_host(&tmp);
    let mut params = super::overlays_tests::source(&upstream, "");
    params["name"] = json!("team");
    let said = ask(&c, "overlay/check", params);
    assert!(said["result"]["token"].is_string(), "{said}");
    let _ = fs::remove_dir_all(&tmp);
    clean(&repo);
}

#[test]
fn an_upload_is_kept_in_a_folder_on_a_share() {
    let uploads = shared("uploads");
    let local = local_of(&uploads);
    let files = Files::new(
        Some(local.clone()),
        Vec::new(),
        None,
        Some(uploads.clone()),
        UploadLimits::default(),
    );
    let c = Control::open(
        None,
        DEFAULT_MAX_RECORDS,
        DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens")
    .with_files(files);
    let put = ask(
        &c,
        "upload/put",
        json!({"name": "report.pdf", "data": data_encoding::BASE64.encode(b"hello")}),
    );
    let path = PathBuf::from(
        put["result"]["path"]
            .as_str()
            .unwrap_or_else(|| panic!("{put}")),
    );
    assert_eq!(fs::read(&path).expect("kept"), b"hello", "{put}");
    assert!(
        path.to_string_lossy().starts_with(r"\\localhost\"),
        "written plain, on the share: {put}"
    );
    let id = put["result"]["id"].as_str().expect("an id").to_owned();
    let gone = ask(&c, "upload/remove", json!({"id": id}));
    assert_eq!(gone["result"]["removed"], true, "{gone}");
    clean(&uploads);
}

#[test]
fn an_overlay_dir_on_a_share_is_read_and_named_plain() {
    let folder = shared("overlay-dir");
    let local = local_of(&folder);
    fs::create_dir_all(local.join("team").join("rules")).expect("folder");
    fs::write(local.join("team").join("rules").join("a.md"), "hi").expect("file");
    let config = local.join("canoa.cfg");
    let team = folder.join("team");
    fs::write(
        &config,
        format!("[overlays.team]\ndir = {:?}\n", team.to_string_lossy()),
    )
    .expect("config");
    let files = Files::new(
        Some(local.clone()),
        Vec::new(),
        Some(config),
        None,
        UploadLimits::default(),
    );
    let c = Control::open(
        None,
        DEFAULT_MAX_RECORDS,
        DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens")
    .with_files(files);
    let reader = Caller::Peer {
        id: "sha256:aa".to_owned(),
        grants: [Grant::OverlayRead].into_iter().collect(),
    };
    let ask_as = |method: &str, params: Value| {
        let line = serde_json::to_vec(
            &json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}),
        )
        .expect("encodes");
        serde_json::from_slice::<Value>(&c.handle(&reader, &line).expect("answered")).expect("JSON")
    };
    let verbatim = format!(r"\\?\UNC\{}", &text(&team)[2..]);
    for written in [text(&team), verbatim] {
        let said = ask_as("profile/canonical", json!({"path": written}));
        assert_eq!(said["result"]["directory"], text(&team), "{said}");
        let said = ask_as("profile/tree", json!({"directory": written}));
        assert!(said["result"]["files"]["rules/a.md"].is_string(), "{said}");
    }
    clean(&folder);
}
