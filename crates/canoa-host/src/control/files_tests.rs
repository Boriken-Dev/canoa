use std::path::{Path, PathBuf};

use super::files::{Files, UploadLimits};
use super::*;

fn dir(name: &str) -> PathBuf {
    static NEXT: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("canoa-files-{name}-{}-{n}", std::process::id()));
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

fn key(id: &str, grants: &[Grant]) -> Caller {
    Caller::Peer {
        id: id.to_owned(),
        grants: grants.iter().copied().collect(),
    }
}

fn control(files: Files) -> Control {
    Control::open(
        None,
        DEFAULT_MAX_RECORDS,
        DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens")
    .with_files(files)
}

fn names(answer: &Value) -> Vec<String> {
    answer["result"]["entries"]
        .as_array()
        .unwrap_or_else(|| panic!("{answer}"))
        .iter()
        .map(|entry| entry["name"].as_str().expect("a name").to_owned())
        .collect()
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[test]
fn every_method_needs_its_own_grant() {
    let tmp = dir("grants");
    let control = control(Files::new(
        Some(tmp.clone()),
        Vec::new(),
        Some(tmp.join("canoa.cfg")),
        Some(tmp.join("uploads")),
        UploadLimits::default(),
    ));
    let nothing = key("sha256:aa", &[Grant::LedgerRead]);
    for (method, params) in [
        ("fs/list", json!({})),
        ("config/get", json!({"section": "sessions"})),
        ("config/set", json!({"section": "sessions", "value": {}})),
        ("upload/put", json!({"name": "a", "data": ""})),
        ("upload/remove", json!({"path": "x"})),
        ("host/restart", json!({})),
    ] {
        let answer = ask(&control, &nothing, method, params);
        assert_eq!(answer["error"]["code"], DENIED, "{method}: {answer}");
    }
    // A grant for one is not a grant for another.
    let reader = key("sha256:aa", &[Grant::ConfigRead]);
    let answer = ask(
        &control,
        &reader,
        "config/set",
        json!({"section": "sessions", "value": {}}),
    );
    assert_eq!(answer["error"]["code"], DENIED, "{answer}");
    let _ = fs::remove_dir_all(tmp);
}

#[test]
fn a_key_lists_folders_under_the_home_and_its_roots_and_nowhere_else() {
    let tmp = dir("browse");
    let home = tmp.join("home");
    let work = tmp.join("work");
    let elsewhere = tmp.join("elsewhere");
    for folder in [
        home.join("projects").join("app"),
        home.join(".hidden"),
        work.join("repo"),
        elsewhere.join("secret"),
    ] {
        fs::create_dir_all(folder).expect("folder");
    }
    fs::write(home.join("notes.txt"), "x").expect("file");
    let roots = vec![
        EntryRoots {
            peers: Some(vec!["sha256:aa".to_owned()]),
            roots: vec![work.clone()],
        },
        EntryRoots {
            peers: Some(vec!["sha256:bb".to_owned()]),
            roots: vec![elsewhere.clone()],
        },
    ];
    let control = control(Files::new(
        Some(home.clone()),
        roots,
        None,
        None,
        UploadLimits::default(),
    ));
    let a = key("sha256:aa", &[Grant::Folders]);

    // With no path: the places to start from, the home first.
    let places = ask(&control, &a, "fs/list", json!({}));
    assert_eq!(names(&places), ["home", "work"], "{places}");

    let inside = ask(&control, &a, "fs/list", json!({"path": path_text(&home)}));
    assert_eq!(
        names(&inside),
        ["projects"],
        "dirs only, nothing hidden: {inside}"
    );
    assert_eq!(inside["result"]["more"], false);
    let hidden = ask(
        &control,
        &a,
        "fs/list",
        json!({"path": path_text(&home), "hidden": true, "files": true}),
    );
    assert_eq!(
        names(&hidden),
        [".hidden", "projects", "notes.txt"],
        "{hidden}"
    );

    // A folder's parent is offered only while it is still inside a place the key may browse.
    let nested = ask(
        &control,
        &a,
        "fs/list",
        json!({"path": path_text(&home.join("projects"))}),
    );
    assert_eq!(
        nested["result"]["parent"],
        json!(path_text(&home)),
        "{nested}"
    );
    assert_eq!(inside["result"]["parent"], Value::Null, "{inside}");

    // Not another entry's root, not the parent of the home, not a path that climbs.
    for path in [
        path_text(&elsewhere),
        path_text(&tmp),
        format!("{}/../elsewhere", path_text(&home)),
        "relative/path".to_owned(),
    ] {
        let denied = ask(&control, &a, "fs/list", json!({"path": path}));
        assert!(denied["error"].is_object(), "{path}: {denied}");
    }
    // What is not there reads the same as what is not allowed.
    let missing = ask(
        &control,
        &a,
        "fs/list",
        json!({"path": path_text(&home.join("nope"))}),
    );
    let forbidden = ask(
        &control,
        &a,
        "fs/list",
        json!({"path": path_text(&elsewhere)}),
    );
    assert_eq!(missing["error"]["code"], forbidden["error"]["code"]);
    let _ = fs::remove_dir_all(tmp);
}

#[cfg(unix)]
#[test]
fn a_link_out_of_the_roots_is_not_listed_or_followed() {
    let tmp = dir("links");
    let home = tmp.join("home");
    let outside = tmp.join("outside");
    fs::create_dir_all(home.join("real")).expect("folder");
    fs::create_dir_all(outside.join("inner")).expect("folder");
    std::os::unix::fs::symlink(&outside, home.join("escape")).expect("link");
    std::os::unix::fs::symlink(home.join("real"), home.join("fine")).expect("link");
    let control = control(Files::new(
        Some(home.clone()),
        Vec::new(),
        None,
        None,
        UploadLimits::default(),
    ));
    let a = key("sha256:aa", &[Grant::Folders]);
    let listed = ask(&control, &a, "fs/list", json!({"path": path_text(&home)}));
    assert_eq!(names(&listed), ["fine", "real"], "{listed}");
    let through = ask(
        &control,
        &a,
        "fs/list",
        json!({"path": path_text(&home.join("escape"))}),
    );
    assert!(through["error"].is_object(), "{through}");
    let _ = fs::remove_dir_all(tmp);
}

#[test]
fn the_owner_browses_the_whole_computer() {
    let tmp = dir("owner");
    let control = control(Files::default());
    let answer = ask(
        &control,
        &Caller::Owner,
        "fs/list",
        json!({"path": path_text(&tmp)}),
    );
    assert_eq!(answer["result"]["entries"], json!([]), "{answer}");
    let _ = fs::remove_dir_all(tmp);
}

fn editing(file: &str, text: &str) -> (PathBuf, Control, Caller) {
    let tmp = dir("config");
    let path = tmp.join(file);
    if !text.is_empty() {
        fs::write(&path, text).expect("canoa.cfg");
    }
    let control = control(Files::new(
        Some(tmp),
        Vec::new(),
        Some(path.clone()),
        None,
        UploadLimits::default(),
    ));
    (
        path,
        control,
        key("sha256:aa", &[Grant::ConfigRead, Grant::ConfigWrite]),
    )
}

#[test]
fn a_section_is_written_in_the_files_own_format_and_read_back() {
    for (file, text, marker) in [
        ("canoa.cfg", "[desktop]\nstart_hidden = true\n", None),
        (
            "canoa.cfg",
            "#!yaml\ndesktop:\n  start_hidden: true\n",
            Some("#!yaml"),
        ),
        (
            "canoa.json",
            "{\"desktop\": {\"start_hidden\": true}}",
            None,
        ),
        ("canoa.cfg", "", None),
    ] {
        let (path, control, who) = editing(file, text);
        let set = ask(
            &control,
            &who,
            "config/set",
            json!({"section": "sessions", "value": {"keep": 12, "workspace": "~/work"}}),
        );
        assert_eq!(set["result"]["restart"], false, "{file}: {set}");
        let written = fs::read_to_string(&path).expect("written");
        if let Some(marker) = marker {
            assert!(written.starts_with(marker), "{written}");
        }
        assert!(
            written.contains("sessions") && written.contains("keep"),
            "{written}"
        );
        if !text.is_empty() {
            assert!(
                written.contains("start_hidden"),
                "the other tables stay: {written}"
            );
        }
        let got = ask(&control, &who, "config/get", json!({"section": "sessions"}));
        assert_eq!(
            got["result"]["value"],
            json!({"keep": 12, "workspace": "~/work"}),
            "{file}: {got}"
        );
        let absent = ask(
            &control,
            &who,
            "config/get",
            json!({"section": "appearance"}),
        );
        assert_eq!(absent["result"]["value"], Value::Null);
        // Null takes the section out.
        ask(
            &control,
            &who,
            "config/set",
            json!({"section": "sessions", "value": null}),
        );
        let gone = ask(&control, &who, "config/get", json!({"section": "sessions"}));
        assert_eq!(gone["result"]["value"], Value::Null, "{file}: {gone}");
        let _ = fs::remove_dir_all(path.parent().expect("dir"));
    }
}

#[test]
fn only_the_apps_own_sections_are_written() {
    let (path, control, who) = editing("canoa.cfg", "[desktop]\nstart_hidden = true\n");
    let before = fs::read_to_string(&path).expect("canoa.cfg");
    for section in [
        "host",
        "agents",
        "profile",
        "desktop",
        "skill_pins",
        "nonsense",
        "",
    ] {
        for method in ["config/set", "config/get"] {
            let answer = ask(
                &control,
                &who,
                method,
                json!({"section": section, "value": {"x": "y"}}),
            );
            assert!(answer["error"].is_object(), "{method} {section}: {answer}");
        }
    }
    // The sections the host reads say they take effect on its next start.
    let overlays = ask(
        &control,
        &who,
        "config/set",
        json!({"section": "overlays", "value": {"team": {}}}),
    );
    assert_eq!(overlays["result"]["restart"], true, "{overlays}");
    let other = fs::read_to_string(&path).expect("canoa.cfg");
    assert!(
        other.contains("start_hidden") && other.contains("team"),
        "{other}"
    );
    assert!(!before.contains("team"));
    // A value that is not a table, or that is too large, changes nothing.
    let size_before = fs::read_to_string(&path).expect("canoa.cfg");
    for value in [json!("text"), json!([1]), json!(7)] {
        let answer = ask(
            &control,
            &who,
            "config/set",
            json!({"section": "sessions", "value": value}),
        );
        assert_eq!(answer["error"]["code"], BAD_PARAMS, "{answer}");
    }
    let huge = "x".repeat(super::files::MAX_SECTION_BYTES);
    let answer = ask(
        &control,
        &who,
        "config/set",
        json!({"section": "sessions", "value": {"big": huge}}),
    );
    assert_eq!(answer["error"]["code"], REFUSED, "{answer}");
    assert_eq!(fs::read_to_string(&path).expect("canoa.cfg"), size_before);
    let _ = fs::remove_dir_all(path.parent().expect("dir"));
}

#[test]
fn a_section_that_would_leave_the_file_refused_is_not_written() {
    let (path, control, who) = editing("canoa.cfg", "[desktop]\nstart_hidden = true\n");
    // `[overlays]` names are overlay names; the host refuses anything else when it starts.
    let before = fs::read_to_string(&path).expect("canoa.cfg");
    let answer = ask(
        &control,
        &who,
        "config/set",
        json!({"section": "overlays", "value": {"../escape": {}}}),
    );
    assert_eq!(answer["error"]["code"], REFUSED, "{answer}");
    assert_eq!(fs::read_to_string(&path).expect("canoa.cfg"), before);
    // And no file is left beside it.
    let leftovers: Vec<_> = fs::read_dir(path.parent().expect("dir"))
        .expect("dir")
        .flatten()
        .map(|entry| entry.file_name())
        .collect();
    assert_eq!(leftovers.len(), 1, "{leftovers:?}");
    let _ = fs::remove_dir_all(path.parent().expect("dir"));
}

#[test]
fn a_host_with_no_file_to_write_refuses() {
    let control = control(Files::default());
    let who = key("sha256:aa", &[Grant::ConfigWrite, Grant::ConfigRead]);
    for method in ["config/get", "config/set"] {
        let answer = ask(
            &control,
            &who,
            method,
            json!({"section": "sessions", "value": {}}),
        );
        assert_eq!(answer["error"]["code"], REFUSED, "{answer}");
    }
}

fn uploading(limits: UploadLimits) -> (PathBuf, Control) {
    let tmp = dir("upload");
    let control = control(Files::new(
        Some(tmp.clone()),
        Vec::new(),
        None,
        Some(tmp.join("uploads")),
        limits,
    ));
    (tmp, control)
}

fn encoded(bytes: &[u8]) -> String {
    data_encoding::BASE64.encode(bytes)
}

#[test]
fn an_upload_is_kept_under_a_safe_name_in_a_folder_of_its_keys_own() {
    let (tmp, control) = uploading(UploadLimits::default());
    let a = key("sha256:aa", &[Grant::Upload]);
    let b = key("sha256:bb", &[Grant::Upload]);
    let put = ask(
        &control,
        &a,
        "upload/put",
        json!({"name": "..\\..\\evil/../report final.pdf", "data": encoded(b"hello")}),
    );
    let path = PathBuf::from(
        put["result"]["path"]
            .as_str()
            .unwrap_or_else(|| panic!("{put}")),
    );
    assert_eq!(fs::read(&path).expect("kept"), b"hello");
    assert_eq!(put["result"]["size"], 5);
    assert!(path.starts_with(tmp.join("uploads")), "{path:?}");
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("a name");
    assert!(
        name.ends_with("-report final.pdf") && !name.contains(".."),
        "{name}"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = fs::metadata(&path).expect("meta").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
    // Two uploads of one name do not meet.
    let again = ask(
        &control,
        &a,
        "upload/put",
        json!({"name": "report final.pdf", "data": encoded(b"other")}),
    );
    assert_ne!(again["result"]["path"], put["result"]["path"]);
    // Another key can neither remove it nor see it.
    let theirs = ask(
        &control,
        &b,
        "upload/remove",
        json!({"path": path_text(&path)}),
    );
    assert_eq!(theirs["result"], json!({"removed": false}), "{theirs}");
    assert!(path.exists());
    let mine = ask(
        &control,
        &a,
        "upload/remove",
        json!({"path": path_text(&path)}),
    );
    assert_eq!(mine["result"], json!({"removed": true}), "{mine}");
    assert!(!path.exists());
    // Nothing outside its own folder is removed, however it is named.
    let outside = tmp.join("keep.txt");
    fs::write(&outside, "x").expect("file");
    for target in [
        path_text(&outside),
        path_text(&tmp.join("uploads").join("..").join("keep.txt")),
    ] {
        let answer = ask(&control, &a, "upload/remove", json!({"path": target}));
        assert_eq!(answer["result"], json!({"removed": false}), "{answer}");
    }
    assert!(outside.exists());
    let _ = fs::remove_dir_all(tmp);
}

#[test]
fn uploads_are_held_to_the_settings() {
    let (tmp, control) = uploading(UploadLimits {
        file_bytes: 10,
        total_bytes: 15,
        files: 2,
    });
    let a = key("sha256:aa", &[Grant::Upload]);
    let put = |bytes: &[u8]| {
        ask(
            &control,
            &a,
            "upload/put",
            json!({"name": "f", "data": encoded(bytes)}),
        )
    };
    assert_eq!(
        put(&[0; 11])["error"]["code"],
        REFUSED,
        "a file over its limit"
    );
    assert!(put(&[0; 10])["result"].is_object());
    // 10 of 15 bytes are held: another 6 would pass the total, 5 would not.
    assert_eq!(put(&[0; 6])["error"]["code"], REFUSED, "past the total");
    assert!(put(&[0; 5])["result"].is_object());
    assert_eq!(put(&[0; 1])["error"]["code"], REFUSED, "past the count");
    // Another key has room of its own.
    let b = key("sha256:bb", &[Grant::Upload]);
    let theirs = ask(
        &control,
        &b,
        "upload/put",
        json!({"name": "f", "data": encoded(b"ok")}),
    );
    assert!(theirs["result"].is_object(), "{theirs}");
    // Not base64, or no data at all.
    let bad_data = ask(
        &control,
        &b,
        "upload/put",
        json!({"name": "f", "data": "***"}),
    );
    assert_eq!(bad_data["error"]["code"], BAD_PARAMS, "{bad_data}");
    let none = ask(&control, &b, "upload/put", json!({"name": "f"}));
    assert_eq!(none["error"]["code"], BAD_PARAMS, "{none}");
    let _ = fs::remove_dir_all(tmp);
}

#[test]
fn a_host_that_keeps_no_uploads_refuses_them() {
    let control = control(Files::default());
    let a = key("sha256:aa", &[Grant::Upload]);
    let answer = ask(&control, &a, "upload/put", json!({"name": "f", "data": ""}));
    assert_eq!(answer["error"]["code"], REFUSED, "{answer}");
}

#[test]
fn a_restart_raises_the_flag_and_then_stops_the_host() {
    let stop = Arc::new(AtomicBool::new(false));
    let control = Control::open(
        None,
        DEFAULT_MAX_RECORDS,
        DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens")
    .with_stop(Arc::clone(&stop));
    let flag = control.restart_flag();
    let a = key("sha256:aa", &[Grant::HostRestart]);
    assert!(!flag.load(Ordering::SeqCst));
    let answer = ask(&control, &a, "host/restart", json!({}));
    assert_eq!(answer["result"], json!({"restarting": true}), "{answer}");
    assert!(flag.load(Ordering::SeqCst));
    let until = std::time::Instant::now() + Duration::from_secs(5);
    while !stop.load(Ordering::SeqCst) && std::time::Instant::now() < until {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(stop.load(Ordering::SeqCst));
    // A control that cannot stop its host says so, and promises nothing.
    let bare = ask(&memory_control(), &a, "host/restart", json!({}));
    assert_eq!(bare["error"]["code"], REFUSED, "{bare}");
}

fn memory_control() -> Control {
    Control::open(
        None,
        DEFAULT_MAX_RECORDS,
        DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens")
}

#[test]
fn a_large_file_is_sent_in_chunks_in_order_and_is_whole_only_when_done() {
    let (tmp, control) = uploading(UploadLimits {
        file_bytes: 30,
        total_bytes: 1000,
        files: 4,
    });
    let a = key("sha256:aa", &[Grant::Upload]);
    let b = key("sha256:bb", &[Grant::Upload]);
    let first = ask(
        &control,
        &a,
        "upload/put",
        json!({"name": "big.bin", "data": encoded(b"0123456789"), "done": false}),
    );
    let id = first["result"]["id"].as_str().expect("an id").to_owned();
    assert_eq!(first["result"]["done"], false, "{first}");
    assert_eq!(first["result"]["size"], 10);
    let path = PathBuf::from(first["result"]["path"].as_str().expect("a path"));
    assert!(!path.exists(), "not whole yet");
    assert!(PathBuf::from(format!("{}.part", path.display())).exists());

    // A chunk is accepted only where the upload stands.
    for offset in [0, 9, 11] {
        let wrong = ask(
            &control,
            &a,
            "upload/put",
            json!({"id": id, "offset": offset, "data": encoded(b"x")}),
        );
        assert_eq!(wrong["error"]["code"], REFUSED, "{offset}: {wrong}");
    }
    let nothing = ask(
        &control,
        &a,
        "upload/put",
        json!({"id": id, "data": encoded(b"x")}),
    );
    assert_eq!(nothing["error"]["code"], REFUSED, "{nothing}");
    // Another key has no such upload.
    let theirs = ask(
        &control,
        &b,
        "upload/put",
        json!({"id": id, "offset": 10, "data": encoded(b"x")}),
    );
    assert_eq!(theirs["error"]["code"], REFUSED, "{theirs}");
    // A name that is not one this host made is no id.
    for bad_id in ["../x", "a", "0123456789ab-x.part", "0123456789ab-../x"] {
        let wrong = ask(
            &control,
            &a,
            "upload/put",
            json!({"id": bad_id, "offset": 0, "data": encoded(b"x")}),
        );
        assert!(wrong["error"].is_object(), "{bad_id}: {wrong}");
    }

    let more = ask(
        &control,
        &a,
        "upload/put",
        json!({"id": id, "offset": 10, "data": encoded(b"ABCDEFGHIJ"), "done": false}),
    );
    assert_eq!(more["result"]["size"], 20, "{more}");
    let last = ask(
        &control,
        &a,
        "upload/put",
        json!({"id": id, "offset": 20, "data": encoded(b"abc")}),
    );
    assert_eq!(last["result"]["done"], true, "{last}");
    assert_eq!(fs::read(&path).expect("whole"), b"0123456789ABCDEFGHIJabc");
    assert!(!PathBuf::from(format!("{}.part", path.display())).exists());

    // Past the file limit mid-way, what was kept goes and the upload ends.
    let start = ask(
        &control,
        &a,
        "upload/put",
        json!({"name": "toolong", "data": encoded(&[1; 20]), "done": false}),
    );
    let id = start["result"]["id"].as_str().expect("an id").to_owned();
    let over = ask(
        &control,
        &a,
        "upload/put",
        json!({"id": id, "offset": 20, "data": encoded(&[1; 11])}),
    );
    assert_eq!(over["error"]["code"], REFUSED, "{over}");
    let gone = ask(
        &control,
        &a,
        "upload/put",
        json!({"id": id, "offset": 20, "data": encoded(b"x")}),
    );
    assert_eq!(gone["error"]["code"], REFUSED, "{gone}");

    // An unfinished upload is removed by its id, and so is a finished one.
    let unfinished = ask(
        &control,
        &a,
        "upload/put",
        json!({"name": "later", "data": encoded(b"zz"), "done": false}),
    );
    let removed = ask(
        &control,
        &a,
        "upload/remove",
        json!({"id": unfinished["result"]["id"]}),
    );
    assert_eq!(removed["result"], json!({"removed": true}), "{removed}");
    let removed = ask(
        &control,
        &a,
        "upload/remove",
        json!({"path": path_text(&path)}),
    );
    assert_eq!(removed["result"], json!({"removed": true}), "{removed}");
    let _ = fs::remove_dir_all(tmp);
}

#[test]
fn a_chunk_is_at_most_what_a_page_can_carry() {
    let (tmp, control) = uploading(UploadLimits::default());
    let a = key("sha256:aa", &[Grant::Upload]);
    let over = vec![0_u8; super::files::MAX_CHUNK + 1];
    let refused = ask(
        &control,
        &a,
        "upload/put",
        json!({"name": "f", "data": encoded(&over)}),
    );
    assert_eq!(refused["error"]["code"], REFUSED, "{refused}");
    let exact = vec![0_u8; super::files::MAX_CHUNK];
    let kept = ask(
        &control,
        &a,
        "upload/put",
        json!({"name": "f", "data": encoded(&exact)}),
    );
    assert_eq!(kept["result"]["size"], super::files::MAX_CHUNK, "{kept}");
    let _ = fs::remove_dir_all(tmp);
}
