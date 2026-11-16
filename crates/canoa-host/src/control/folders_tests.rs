use std::path::{Path, PathBuf};
use std::process::Command;

use super::files::{Files, UploadLimits};
use super::*;

fn dir(name: &str) -> PathBuf {
    static NEXT: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("canoa-folders-{name}-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("directory");
    // Resolved, so that what the host answers compares with what the test made.
    fs::canonicalize(&dir)
        .map(|real| {
            let text = real.to_string_lossy().into_owned();
            PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(&text))
        })
        .expect("resolves")
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

fn opened(files: Files) -> Control {
    Control::open(
        None,
        DEFAULT_MAX_RECORDS,
        DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens")
    .with_files(files)
}

fn plain_files(home: &Path, roots: Vec<EntryRoots>) -> Files {
    Files::new(
        Some(home.to_path_buf()),
        roots,
        None,
        None,
        UploadLimits::default(),
    )
}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn write(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().expect("a parent")).expect("folder");
    fs::write(path, body).expect("file");
}

fn entries(answer: &Value) -> Vec<String> {
    answer["result"]["entries"]
        .as_array()
        .unwrap_or_else(|| panic!("{answer}"))
        .iter()
        .map(|entry| entry.as_str().expect("text").to_owned())
        .collect()
}

#[test]
fn every_folder_method_needs_the_folders_grant() {
    let tmp = dir("grants");
    let control = opened(plain_files(&tmp, Vec::new()));
    let nothing = key(&[Grant::Files, Grant::ConfigRead]);
    for (method, params) in [
        ("fs/info", json!({"path": text(&tmp)})),
        ("fs/mentions", json!({"path": text(&tmp)})),
        ("fs/resolve", json!({"path": text(&tmp), "name": "a"})),
        ("fs/scratch", json!({"session": "abc"})),
        ("fs/scratch_remove", json!({"session": "abc"})),
        ("fs/scratch_preview", json!({"template": "${session}"})),
    ] {
        let answer = ask(&control, &nothing, method, params);
        assert_eq!(answer["error"]["code"], DENIED, "{method}: {answer}");
    }
    let _ = fs::remove_dir_all(tmp);
}

#[test]
fn info_says_what_is_there_and_which_repository_it_is_in() {
    let tmp = dir("info");
    let repo = tmp.join("repo");
    write(&repo.join("src").join("lib.rs"), "x");
    fs::create_dir_all(repo.join(".git")).expect("git folder");
    fs::create_dir_all(tmp.join("plain")).expect("folder");
    let control = opened(plain_files(&tmp, Vec::new()));
    let said = ask(
        &control,
        &Caller::Owner,
        "fs/info",
        json!({"paths": [text(&repo.join("src")), text(&tmp.join("plain")), text(&tmp.join("gone")), text(&repo.join("src").join("lib.rs"))]}),
    );
    let found = said["result"]["folders"].as_array().expect("a list");
    assert_eq!(found.len(), 4, "{said}");
    assert_eq!(found[0]["exists"], true);
    assert_eq!(found[0]["folder"], true);
    assert_eq!(found[0]["repository"], true, "{said}");
    assert_eq!(found[0]["root"], text(&repo), "{said}");
    assert_eq!(found[1]["exists"], true);
    assert_eq!(found[1]["repository"], false, "{said}");
    assert!(found[1]["root"].is_null());
    assert_eq!(found[2]["exists"], false, "{said}");
    assert_eq!(found[3]["exists"], true);
    assert_eq!(found[3]["folder"], false, "a file is not a folder");
    let _ = fs::remove_dir_all(tmp);
}

#[test]
fn a_key_is_told_only_of_the_places_it_may_browse() {
    let tmp = dir("info-key");
    let home = tmp.join("home");
    let elsewhere = tmp.join("elsewhere");
    fs::create_dir_all(home.join("proj")).expect("folder");
    fs::create_dir_all(elsewhere.join(".git")).expect("folder");
    let control = opened(plain_files(&home, Vec::new()));
    let folders = key(&[Grant::Folders]);
    let said = ask(
        &control,
        &folders,
        "fs/info",
        json!({"paths": [text(&home.join("proj")), text(&elsewhere), r"\\server\share\x"]}),
    );
    let found = said["result"]["folders"].as_array().expect("a list");
    assert_eq!(found[0]["exists"], true, "{said}");
    assert_eq!(found[1]["exists"], false, "outside its places: {said}");
    assert!(found[1]["why"].is_string(), "the refusal says why: {said}");
    assert_eq!(
        found[2]["exists"], false,
        "a share outside its places is like any other place: {said}"
    );
    assert!(
        found[2]["why"]
            .as_str()
            .is_some_and(|why| !why.contains("login")),
        "no share is singled out for refusal: {said}"
    );
    // A key holding files is told of any place.
    let files = key(&[Grant::Folders, Grant::Files]);
    let said = ask(
        &control,
        &files,
        "fs/info",
        json!({"path": text(&elsewhere)}),
    );
    assert_eq!(said["result"]["folders"][0]["exists"], true, "{said}");
    assert_eq!(said["result"]["folders"][0]["repository"], true, "{said}");
    // More than the batch limit, and none at all, are asked wrongly.
    let many: Vec<String> = (0..65).map(|n| format!("/x{n}")).collect();
    let said = ask(&control, &Caller::Owner, "fs/info", json!({"paths": many}));
    assert_eq!(said["error"]["code"], BAD_PARAMS, "{said}");
    let said = ask(&control, &Caller::Owner, "fs/info", json!({}));
    assert_eq!(said["error"]["code"], BAD_PARAMS, "{said}");
    let _ = fs::remove_dir_all(tmp);
}

#[test]
fn mentions_walk_a_folder_nearest_first_within_the_bounds() {
    let tmp = dir("walk");
    let work = tmp.join("work");
    write(&work.join("notes.txt"), "x");
    write(&work.join("src").join("main.dart"), "x");
    write(&work.join("node_modules").join("dep").join("index.js"), "x");
    write(&work.join(".hidden").join("note.txt"), "x");
    write(
        &work
            .join("a")
            .join("b")
            .join("c")
            .join("d")
            .join("e")
            .join("deep.txt"),
        "x",
    );
    write(&work.join("bad\u{202e}name.txt"), "x");
    write(&tmp.join("outside.txt"), "x");
    let control = opened(plain_files(&tmp, Vec::new()));
    let said = ask(
        &control,
        &Caller::Owner,
        "fs/mentions",
        json!({"path": text(&work)}),
    );
    let all = entries(&said);
    for wanted in [
        "notes.txt",
        "src/",
        "src/main.dart",
        "node_modules/dep/index.js",
        ".hidden/note.txt",
    ] {
        assert!(
            all.iter().any(|entry| entry == wanted),
            "{wanted} in {all:?}"
        );
    }
    assert!(
        !all.iter().any(|entry| entry.contains("outside")),
        "{all:?}"
    );
    assert!(
        !all.iter().any(|entry| entry.contains('\u{202e}')),
        "a direction character hides text: {all:?}"
    );
    let top = all
        .iter()
        .position(|entry| entry == "notes.txt")
        .expect("top");
    let deep = all
        .iter()
        .position(|entry| entry == "src/main.dart")
        .expect("deeper");
    assert!(top < deep, "nearest first: {all:?}");
    let shallow = entries(&ask(
        &control,
        &Caller::Owner,
        "fs/mentions",
        json!({"path": text(&work), "max_depth": 2}),
    ));
    assert!(shallow.contains(&"a/b/c/".to_owned()), "{shallow:?}");
    assert!(
        !shallow.contains(&"a/b/c/d/e/".to_owned()),
        "deeper than the bound: {shallow:?}"
    );
    let few = entries(&ask(
        &control,
        &Caller::Owner,
        "fs/mentions",
        json!({"path": text(&work), "max_entries": 4}),
    ));
    assert_eq!(few.len(), 4);
    let file = ask(
        &control,
        &Caller::Owner,
        "fs/mentions",
        json!({"path": text(&work.join("notes.txt"))}),
    );
    assert_eq!(file["error"]["code"], NOT_FOUND, "{file}");
    let _ = fs::remove_dir_all(tmp);
}

fn git(folder: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(folder)
        .args(args)
        .output()
        .is_ok_and(|done| done.status.success())
}

#[test]
fn in_a_git_repository_the_names_are_gits_not_ignored_ones() {
    let tmp = dir("git");
    let work = tmp.join("work");
    fs::create_dir_all(&work).expect("folder");
    if !git(&work, &["init", "-q"]) {
        eprintln!("git is not here: skipped");
        let _ = fs::remove_dir_all(tmp);
        return;
    }
    write(&work.join("notes.txt"), "x");
    write(&work.join("src").join("main.dart"), "x");
    write(&work.join(".gitignore"), "dist/\n*.log\n");
    write(&work.join("dist").join("out.js"), "x");
    write(&work.join("build.log"), "x");
    write(&work.join("node_modules").join("dep").join("index.js"), "x");
    write(&work.join(".github").join("workflows").join("ci.yml"), "x");
    assert!(git(&work, &["add", "notes.txt"]));
    let control = opened(plain_files(&tmp, Vec::new()));
    let all = entries(&ask(
        &control,
        &Caller::Owner,
        "fs/mentions",
        json!({"path": text(&work)}),
    ));
    for wanted in [
        "notes.txt",
        "src/",
        "src/main.dart",
        ".gitignore",
        ".github/workflows/ci.yml",
        "node_modules/dep/index.js",
    ] {
        assert!(
            all.iter().any(|entry| entry == wanted),
            "{wanted} in {all:?}"
        );
    }
    assert!(
        !all.iter().any(|entry| entry.starts_with("dist")
            || entry.ends_with(".log")
            || entry.starts_with(".git/")),
        "{all:?}"
    );
    write(&work.join(".gitignore"), "dist/\n*.log\nnode_modules/\n");
    let again = entries(&ask(
        &control,
        &Caller::Owner,
        "fs/mentions",
        json!({"path": text(&work)}),
    ));
    assert!(
        !again.iter().any(|entry| entry.contains("node_modules")),
        "the repository's rule, read from it: {again:?}"
    );
    let _ = fs::remove_dir_all(tmp);
}

#[test]
fn a_name_resolves_only_inside_its_folder() {
    let tmp = dir("resolve");
    let work = tmp.join("work");
    write(&work.join("notes.txt"), "hello notes");
    write(&work.join("src").join("main.dart"), "void main() {}");
    write(&tmp.join("outside.txt"), "not in the folder");
    let control = opened(plain_files(&tmp, Vec::new()));
    let found = ask(
        &control,
        &Caller::Owner,
        "fs/resolve",
        json!({"path": text(&work), "name": "src/main.dart"}),
    );
    assert_eq!(found["result"]["folder"], false, "{found}");
    assert_eq!(found["result"]["size"], 14, "{found}");
    assert_eq!(
        found["result"]["path"],
        text(&work.join("src").join("main.dart")),
        "{found}"
    );
    let folder = ask(
        &control,
        &Caller::Owner,
        "fs/resolve",
        json!({"path": text(&work), "name": "src/"}),
    );
    assert_eq!(folder["result"]["folder"], true, "{folder}");
    for name in [
        "../outside.txt",
        "src/../../outside.txt",
        "/etc/passwd",
        "C:/Windows",
        "src\\main.dart",
        "",
        ".",
        "src//main.dart",
        "missing.txt",
        "bad\u{202e}.txt",
    ] {
        let said = ask(
            &control,
            &Caller::Owner,
            "fs/resolve",
            json!({"path": text(&work), "name": name}),
        );
        assert_eq!(said["error"]["code"], NOT_FOUND, "{name:?}: {said}");
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(tmp.join("outside.txt"), work.join("link.txt")).expect("link");
        let said = ask(
            &control,
            &Caller::Owner,
            "fs/resolve",
            json!({"path": text(&work), "name": "link.txt"}),
        );
        assert_eq!(said["error"]["code"], NOT_FOUND, "a link out: {said}");
    }
    // A key keeps to its places.
    let home = tmp.join("home");
    fs::create_dir_all(&home).expect("home");
    let control = opened(plain_files(&home, Vec::new()));
    let said = ask(
        &control,
        &key(&[Grant::Folders]),
        "fs/resolve",
        json!({"path": text(&work), "name": "notes.txt"}),
    );
    assert_eq!(said["error"]["code"], NOT_FOUND, "{said}");
    let _ = fs::remove_dir_all(tmp);
}

fn scratch_control(tmp: &Path, workspace: Option<&str>) -> Control {
    let config = tmp.join("canoa.cfg");
    if let Some(template) = workspace {
        fs::write(&config, format!("[sessions]\nworkspace = {template:?}\n")).expect("config");
    }
    opened(
        Files::new(
            Some(tmp.to_path_buf()),
            Vec::new(),
            Some(config),
            None,
            UploadLimits::default(),
        )
        .with_documents(tmp.join("Documents")),
    )
}

#[test]
fn a_chats_folder_is_made_from_the_persons_template_and_taken_back_when_empty() {
    let tmp = dir("scratch");
    let control = scratch_control(&tmp, None);
    let id = "3f2a9c1e-7b4d-4e58-9a16-0d5c2b8e6f40";
    let made = ask(
        &control,
        &Caller::Owner,
        "fs/scratch",
        json!({"session": id}),
    );
    let folder = tmp.join("Documents").join("Canoa").join("scratch").join(id);
    assert_eq!(made["result"]["made"], true, "{made}");
    assert_eq!(
        Path::new(made["result"]["path"].as_str().expect("a path")),
        folder
    );
    assert!(folder.is_dir());
    let again = ask(
        &control,
        &Caller::Owner,
        "fs/scratch",
        json!({"session": id}),
    );
    assert_eq!(again["result"]["made"], false, "it was there: {again}");
    write(&folder.join("kept.txt"), "x");
    let kept = ask(
        &control,
        &Caller::Owner,
        "fs/scratch_remove",
        json!({"session": id}),
    );
    assert_eq!(
        kept["result"]["removed"], false,
        "not while it holds a file: {kept}"
    );
    fs::remove_file(folder.join("kept.txt")).expect("removes");
    let gone = ask(
        &control,
        &Caller::Owner,
        "fs/scratch_remove",
        json!({"session": id}),
    );
    assert_eq!(gone["result"]["removed"], true, "{gone}");
    assert!(!folder.exists());
    // A key holding folders makes one too, and an id that is not one is refused whatever it names.
    let by_key = ask(
        &control,
        &key(&[Grant::Folders]),
        "fs/scratch",
        json!({"session": "k1"}),
    );
    assert_eq!(by_key["result"]["made"], true, "{by_key}");
    for bad_id in ["../x", "a/b", "", "a b", &"x".repeat(37)] {
        let said = ask(
            &control,
            &Caller::Owner,
            "fs/scratch",
            json!({"session": bad_id}),
        );
        assert_eq!(said["error"]["code"], BAD_PARAMS, "{bad_id:?}: {said}");
    }
    let none = ask(&control, &Caller::Owner, "fs/scratch", json!({}));
    assert_eq!(none["error"]["code"], BAD_PARAMS, "{none}");
    let _ = fs::remove_dir_all(tmp);
}

#[test]
fn the_template_takes_its_variables_as_single_segments() {
    let tmp = dir("template");
    let control = scratch_control(
        &tmp,
        Some("${documents}/chats/${handler}/${source}/${date}/${session}"),
    );
    let made = ask(
        &control,
        &Caller::Owner,
        "fs/scratch",
        json!({
            "session": "s1",
            "handler": "../claude:x",
            "source": "My Agent",
            "now": "2026-11-17T10:30:05",
        }),
    );
    let folder = tmp
        .join("Documents")
        .join("chats")
        .join("__claude_x")
        .join("My Agent")
        .join("2026-11-17")
        .join("s1");
    assert_eq!(made["result"]["made"], true, "{made}");
    assert_eq!(
        Path::new(made["result"]["path"].as_str().expect("a path")),
        folder,
        "{made}"
    );
    assert!(folder.is_dir());
    let bad_time = ask(
        &control,
        &Caller::Owner,
        "fs/scratch",
        json!({"session": "s2", "now": "yesterday"}),
    );
    assert_eq!(bad_time["error"]["code"], BAD_PARAMS, "{bad_time}");
    let _ = fs::remove_dir_all(tmp);
}

#[test]
fn a_template_that_is_not_a_workspace_is_refused_and_a_preview_makes_nothing() {
    let tmp = dir("preview");
    let control = scratch_control(&tmp, Some("${documents}/chats"));
    let refused = ask(
        &control,
        &Caller::Owner,
        "fs/scratch",
        json!({"session": "s1"}),
    );
    assert_eq!(
        refused["error"]["code"], BAD_PARAMS,
        "no ${{session}}: {refused}"
    );
    assert!(
        refused["error"]["message"]
            .as_str()
            .is_some_and(|why| why.contains("session")),
        "{refused}"
    );
    for template in [
        "",
        "${nope}/${session}",
        "${documents}/${session",
        "${documents}/x",
    ] {
        let said = ask(
            &control,
            &Caller::Owner,
            "fs/scratch_preview",
            json!({"template": template}),
        );
        assert_eq!(said["error"]["code"], BAD_PARAMS, "{template:?}: {said}");
    }
    let said = ask(
        &control,
        &key(&[Grant::Folders]),
        "fs/scratch_preview",
        json!({"template": "${documents}/Chats/${datetime}/${session}", "now": "2026-11-17T10:30:05", "session": "abc"}),
    );
    assert_eq!(
        Path::new(said["result"]["path"].as_str().expect("a path")),
        tmp.join("Documents")
            .join("Chats")
            .join("2026-11-17T103005")
            .join("abc"),
        "{said}"
    );
    assert!(!tmp.join("Documents").exists(), "a preview makes nothing");
    let example = ask(
        &control,
        &Caller::Owner,
        "fs/scratch_preview",
        json!({"template": "${documents}/c/${session}"}),
    );
    assert!(
        example["result"]["path"]
            .as_str()
            .is_some_and(|path| path.ends_with("9a16-0d5c2b8e6f40")),
        "an example session when none is named: {example}"
    );
    let _ = fs::remove_dir_all(tmp);
}
