use std::path::{Path, PathBuf};

use super::files::{DEFAULT_LIST, Files, READ_PAGE_BYTES, UploadLimits};
use super::*;

fn dir(name: &str) -> PathBuf {
    static NEXT: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("canoa-read-{name}-{}-{n}", std::process::id()));
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

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// A host that names one working root and a home, and a key that holds `files` alone: the roots are no part of what it may read.
fn control_over(home: &Path, root: &Path) -> Control {
    Control::open(
        None,
        DEFAULT_MAX_RECORDS,
        DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens")
    .with_files(Files::new(
        Some(home.to_owned()),
        vec![EntryRoots {
            peers: Some(vec!["sha256:aa".to_owned()]),
            roots: vec![root.to_owned()],
        }],
        None,
        None,
        UploadLimits::default(),
    ))
}

struct Place {
    tmp: PathBuf,
    home: PathBuf,
    work: PathBuf,
    elsewhere: PathBuf,
}

fn place(name: &str) -> Place {
    let tmp = dir(name);
    let (home, work, elsewhere) = (tmp.join("home"), tmp.join("work"), tmp.join("elsewhere"));
    for folder in [&home, &work, &elsewhere] {
        fs::create_dir_all(folder).expect("folder");
    }
    Place {
        tmp,
        home,
        work,
        elsewhere,
    }
}

fn reader() -> Caller {
    key(&[Grant::Files])
}

fn read(control: &Control, caller: &Caller, path: &Path) -> Value {
    ask(control, caller, "fs/read", json!({"path": text(path)}))
}

fn bytes_of(answer: &Value) -> Vec<u8> {
    data_encoding::BASE64
        .decode(answer["result"]["data"].as_str().expect("data").as_bytes())
        .expect("base64")
}

fn why(answer: &Value) -> String {
    answer["error"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("an error: {answer}"))
        .to_owned()
}

#[test]
fn reading_takes_its_own_grant_and_nothing_else() {
    let place = place("grant");
    fs::write(place.work.join("a.txt"), "hello").expect("file");
    let control = control_over(&place.home, &place.work);
    for grants in [
        &[][..],
        &[Grant::Folders],
        &[Grant::ConfigRead, Grant::Upload],
    ] {
        let answer = read(&control, &key(grants), &place.work.join("a.txt"));
        assert_eq!(answer["error"]["code"], DENIED, "{answer}");
    }
    let granted = read(&control, &reader(), &place.work.join("a.txt"));
    assert_eq!(granted["result"]["kind"], "text", "{granted}");
    assert_eq!(Grant::named("files"), Some(Grant::Files));
    let held = ask(&control, &reader(), "host/grants", json!({}));
    assert_eq!(held["result"]["grants"], json!(["files"]));
    let _ = fs::remove_dir_all(&place.tmp);
}

#[test]
fn a_key_with_the_grant_reads_what_the_account_can_read_anywhere() {
    let place = place("anywhere");
    // Outside every working root and the home, in a dotted folder, and under names an agent would be kept from.
    let hidden = place.elsewhere.join(".ssh");
    fs::create_dir_all(&hidden).expect("folder");
    let files = [
        (hidden.join("id_rsa"), "private"),
        (place.elsewhere.join(".env"), "TOKEN=1"),
        (place.elsewhere.join("server.pem"), "pem"),
        (place.elsewhere.join("package.json"), "{}"),
        (place.work.join("canoa.cfg"), "[host]\n"),
    ];
    for (path, body) in &files {
        fs::write(path, body).expect("file");
    }
    let control = control_over(&place.home, &place.work);
    for caller in [reader(), Caller::Owner] {
        for (path, body) in &files {
            let answer = read(&control, &caller, path);
            assert_eq!(bytes_of(&answer), body.as_bytes(), "{path:?}: {answer}");
        }
    }
    // A path that says `..` is still a path.
    let roundabout = place.work.join("..").join("elsewhere").join(".env");
    assert_eq!(
        bytes_of(&read(&control, &reader(), &roundabout)),
        b"TOKEN=1"
    );
    let _ = fs::remove_dir_all(&place.tmp);
}

#[test]
fn a_key_reads_text_with_its_size_name_and_kind() {
    let place = place("text");
    fs::create_dir_all(place.work.join("src")).expect("folder");
    let body = "fn main() {}\nunicode: \u{e9}\u{4e2d}\n";
    fs::write(place.work.join("src").join("main.rs"), body).expect("file");
    let control = control_over(&place.home, &place.work);
    let answer = read(&control, &reader(), &place.work.join("src").join("main.rs"));
    let got = &answer["result"];
    assert_eq!(got["kind"], "text", "{answer}");
    assert_eq!(got["name"], "main.rs");
    assert_eq!(got["size"], body.len());
    assert_eq!(got["truncated"], false);
    assert_eq!(bytes_of(&answer), body.as_bytes());
    let _ = fs::remove_dir_all(&place.tmp);
}

#[test]
fn a_refusal_says_why() {
    let place = place("why");
    fs::write(place.work.join("in.txt"), "in").expect("file");
    let control = control_over(&place.home, &place.work);
    let missing = read(&control, &reader(), &place.work.join("nothing.txt"));
    assert_eq!(missing["error"]["code"], NOT_FOUND, "{missing}");
    assert!(why(&missing).contains("no file"), "{missing}");
    let relative = ask(&control, &reader(), "fs/read", json!({"path": "in.txt"}));
    assert_eq!(relative["error"]["code"], BAD_PARAMS, "{relative}");
    assert!(why(&relative).contains("absolute"), "{relative}");
    let none = ask(&control, &reader(), "fs/read", json!({}));
    assert_eq!(none["error"]["code"], BAD_PARAMS, "{none}");
    assert!(why(&none).contains("path"), "{none}");
    let folder = read(&control, &reader(), &place.work);
    assert_eq!(folder["error"]["code"], BAD_PARAMS, "{folder}");
    assert!(why(&folder).contains("folder"), "{folder}");
    let _ = fs::remove_dir_all(&place.tmp);
}

#[test]
fn a_read_stops_at_the_limit_and_says_so() {
    let place = place("limit");
    fs::write(place.work.join("big.txt"), vec![b'a'; 2000]).expect("file");
    let control = control_over(&place.home, &place.work);
    let big_path = text(&place.work.join("big.txt"));
    let cut = ask(
        &control,
        &reader(),
        "fs/read",
        json!({"path": big_path, "max_bytes": 100}),
    );
    assert_eq!(cut["result"]["truncated"], true, "{cut}");
    assert_eq!(cut["result"]["size"], 2000);
    assert_eq!(bytes_of(&cut).len(), 100);
    let whole = read(&control, &reader(), &place.work.join("big.txt"));
    assert_eq!(whole["result"]["truncated"], false, "{whole}");
    assert_eq!(bytes_of(&whole).len(), 2000);
    // One answer carries a page the transport takes, whatever the request asks; the rest is read with `offset`.
    let huge = vec![b'b'; usize::try_from(READ_PAGE_BYTES).expect("fits") + 10];
    fs::write(place.work.join("huge.txt"), &huge).expect("file");
    let capped = ask(
        &control,
        &reader(),
        "fs/read",
        json!({"path": text(&place.work.join("huge.txt")), "max_bytes": u64::MAX}),
    );
    assert_eq!(capped["result"]["truncated"], true, "{capped}");
    assert_eq!(bytes_of(&capped).len() as u64, READ_PAGE_BYTES);
    assert_eq!(capped["result"]["next"], READ_PAGE_BYTES, "{capped}");
    let rest = ask(
        &control,
        &reader(),
        "fs/read",
        json!({"path": text(&place.work.join("huge.txt")), "offset": READ_PAGE_BYTES}),
    );
    assert_eq!(bytes_of(&rest), vec![b'b'; 10], "{rest}");
    assert_eq!(rest["result"]["truncated"], false, "{rest}");
    assert!(rest["result"]["next"].is_null(), "{rest}");
    assert_eq!(
        rest["result"]["kind"], "text",
        "a page far in is told by the file's start"
    );
    for bad in [json!(0), json!(-1), json!("9"), json!(1.5)] {
        let answer = ask(
            &control,
            &reader(),
            "fs/read",
            json!({"path": big_path, "max_bytes": bad}),
        );
        assert_eq!(answer["error"]["code"], BAD_PARAMS, "{bad}: {answer}");
        assert!(why(&answer).contains("max_bytes"), "{answer}");
    }
    let answer = ask(
        &control,
        &reader(),
        "fs/read",
        json!({"path": big_path, "offset": -1}),
    );
    assert!(why(&answer).contains("offset"), "{answer}");
    let _ = fs::remove_dir_all(&place.tmp);
}

#[test]
fn a_cut_character_is_still_text_and_an_image_is_sent_in_pages_to_put_together() {
    let place = place("kinds");
    fs::write(place.work.join("e.txt"), "\u{e9}\u{e9}\u{e9}").expect("file");
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend_from_slice(&[0; 40]);
    fs::write(place.work.join("p.png"), &png).expect("file");
    fs::write(place.work.join("b.bin"), [1u8, 2, 0, 3]).expect("file");
    fs::write(place.work.join("bad.txt"), [0xffu8, 0xfe, b'a']).expect("file");
    fs::write(place.work.join("i.svg"), "<svg onload=\"x()\"></svg>").expect("file");
    let control = control_over(&place.home, &place.work);
    let ask_of = |name: &str, max: u64| {
        ask(
            &control,
            &reader(),
            "fs/read",
            json!({"path": text(&place.work.join(name)), "max_bytes": max}),
        )
    };
    let cut = ask_of("e.txt", 3);
    assert_eq!(cut["result"]["kind"], "text", "{cut}");
    assert_eq!(cut["result"]["truncated"], true);
    let image = ask_of("p.png", 1000);
    assert_eq!(image["result"]["kind"], "image", "{image}");
    assert_eq!(image["result"]["mime"], "image/png");
    assert_eq!(bytes_of(&image), png);
    // An image the page cuts is not withheld: its pages are read by offset and put together.
    let first = ask_of("p.png", 20);
    assert_eq!(first["result"]["kind"], "image");
    assert_eq!(first["result"]["truncated"], true);
    assert_eq!(first["result"]["next"], 20, "{first}");
    assert_eq!(bytes_of(&first), png[..20]);
    let second = ask(
        &control,
        &reader(),
        "fs/read",
        json!({"path": text(&place.work.join("p.png")), "offset": 20, "max_bytes": 1000}),
    );
    assert_eq!(second["result"]["kind"], "image", "{second}");
    assert_eq!(second["result"]["mime"], "image/png", "{second}");
    assert_eq!(bytes_of(&second), png[20..], "{second}");
    assert!(second["result"]["next"].is_null());
    for name in ["b.bin", "bad.txt"] {
        let answer = ask_of(name, 1000);
        assert_eq!(answer["result"]["kind"], "binary", "{answer}");
        assert!(answer["result"]["data"].is_null());
    }
    // SVG is text, never an image to draw: it can carry script.
    let svg = ask_of("i.svg", 1000);
    assert_eq!(svg["result"]["kind"], "text", "{svg}");
    let _ = fs::remove_dir_all(&place.tmp);
}

#[test]
fn raw_asks_for_the_bytes_of_a_binary_file_too() {
    let place = place("raw");
    fs::write(place.work.join("b.bin"), [1u8, 2, 0, 3]).expect("file");
    let control = control_over(&place.home, &place.work);
    let raw = ask(
        &control,
        &reader(),
        "fs/read",
        json!({"path": text(&place.work.join("b.bin")), "raw": true}),
    );
    assert_eq!(raw["result"]["kind"], "binary", "{raw}");
    assert_eq!(bytes_of(&raw), [1u8, 2, 0, 3]);
    let plain = read(&control, &reader(), &place.work.join("b.bin"));
    assert!(plain["result"]["data"].is_null(), "{plain}");
    let _ = fs::remove_dir_all(&place.tmp);
}

/// A pipe is not a file, and opening one for reading would wait for a writer.
#[cfg(unix)]
#[test]
fn a_named_pipe_is_refused_and_never_waited_on() {
    let place = place("fifo");
    let pipe = place.work.join("pipe");
    let made = std::process::Command::new("mkfifo")
        .arg(&pipe)
        .status()
        .expect("mkfifo runs");
    assert!(made.success());
    let control = control_over(&place.home, &place.work);
    let answer = read(&control, &reader(), &pipe);
    assert_eq!(answer["error"]["code"], BAD_PARAMS, "{answer}");
    assert!(why(&answer).contains("not a regular file"), "{answer}");
    let _ = fs::remove_dir_all(&place.tmp);
}

#[cfg(unix)]
#[test]
fn a_device_is_not_read() {
    let place = place("device");
    let control = control_over(&place.home, &place.work);
    for device in ["/dev/zero", "/dev/null"] {
        let answer = read(&control, &reader(), Path::new(device));
        assert_eq!(answer["error"]["code"], BAD_PARAMS, "{device}: {answer}");
        assert!(why(&answer).contains("not a regular file"), "{answer}");
    }
    let _ = fs::remove_dir_all(&place.tmp);
}

/// A link made where the platform lets this user make one.
fn link(target: &Path, at: &Path) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, at).is_ok()
    }
    #[cfg(windows)]
    {
        if target.is_dir() {
            std::os::windows::fs::symlink_dir(target, at).is_ok()
        } else {
            std::os::windows::fs::symlink_file(target, at).is_ok()
        }
    }
}

#[test]
fn a_link_is_followed_wherever_it_goes() {
    let place = place("links");
    fs::write(place.elsewhere.join("secret.txt"), "there").expect("file");
    let (out, dir_out) = (place.work.join("out.txt"), place.work.join("outdir"));
    if !link(&place.elsewhere.join("secret.txt"), &out) || !link(&place.elsewhere, &dir_out) {
        eprintln!(
            "skipped: this account cannot make links here (Windows needs Developer Mode or elevation)"
        );
        let _ = fs::remove_dir_all(&place.tmp);
        return;
    }
    let control = control_over(&place.home, &place.work);
    assert_eq!(bytes_of(&read(&control, &reader(), &out)), b"there");
    let through = read(&control, &reader(), &dir_out.join("secret.txt"));
    assert_eq!(bytes_of(&through), b"there", "{through}");
    let _ = fs::remove_dir_all(&place.tmp);
}

fn names(answer: &Value) -> Vec<String> {
    answer["result"]["entries"]
        .as_array()
        .unwrap_or_else(|| panic!("{answer}"))
        .iter()
        .map(|entry| entry["name"].as_str().expect("a name").to_owned())
        .collect()
}

#[test]
fn a_key_with_the_grant_lists_anywhere_and_starts_from_the_home() {
    let place = place("list");
    fs::create_dir_all(place.elsewhere.join("deep")).expect("folder");
    fs::write(place.elsewhere.join("a.txt"), "a").expect("file");
    fs::write(place.elsewhere.join(".dot"), "d").expect("file");
    let control = control_over(&place.home, &place.work);
    let lister = key(&[Grant::Files, Grant::Folders]);
    let listed = ask(
        &control,
        &lister,
        "fs/list",
        json!({"path": text(&place.elsewhere), "files": true, "hidden": true}),
    );
    assert_eq!(names(&listed), ["deep", ".dot", "a.txt"], "{listed}");
    // The parent of a place is offered wherever it is.
    assert_eq!(listed["result"]["parent"], text(&place.tmp), "{listed}");
    let places = ask(&control, &lister, "fs/list", json!({}));
    let first = places["result"]["entries"][0]["path"]
        .as_str()
        .expect("a path");
    assert!(Path::new(first).ends_with("home"), "{places}");
    // A key with `folders` alone is still kept to the home and its working roots.
    let only_folders = key(&[Grant::Folders]);
    let outside = ask(
        &control,
        &only_folders,
        "fs/list",
        json!({"path": text(&place.elsewhere)}),
    );
    assert_eq!(outside["error"]["code"], NOT_FOUND, "{outside}");
    let relative = ask(&control, &lister, "fs/list", json!({"path": "elsewhere"}));
    assert!(why(&relative).contains("absolute"), "{relative}");
    let a_file = ask(
        &control,
        &lister,
        "fs/list",
        json!({"path": text(&place.elsewhere.join("a.txt"))}),
    );
    assert!(why(&a_file).contains("file"), "{a_file}");
    let _ = fs::remove_dir_all(&place.tmp);
}

/// A folder of more than a page of entries loses none: the page is cut from the sorted whole and the rest is asked for by offset.
#[test]
fn a_folder_longer_than_a_page_is_listed_in_pages_with_none_lost() {
    let place = place("pages");
    let total = DEFAULT_LIST + 150;
    for number in 0..total {
        fs::write(place.work.join(format!("f{number:05}.txt")), "").expect("file");
    }
    let control = control_over(&place.home, &place.work);
    let lister = key(&[Grant::Files, Grant::Folders]);
    let ask_page = |params: Value| {
        let mut asked = json!({"path": text(&place.work), "files": true});
        asked
            .as_object_mut()
            .expect("an object")
            .extend(params.as_object().cloned().expect("an object"));
        ask(&control, &lister, "fs/list", asked)
    };
    let first = ask_page(json!({}));
    assert_eq!(names(&first).len(), DEFAULT_LIST, "{first}");
    assert_eq!(first["result"]["more"], true);
    assert_eq!(first["result"]["next"], DEFAULT_LIST);
    assert_eq!(
        names(&first)[0],
        "f00000.txt",
        "the page is of the sorted whole"
    );
    let rest = ask_page(json!({"offset": DEFAULT_LIST}));
    assert_eq!(names(&rest).len(), 150, "{rest}");
    assert_eq!(rest["result"]["more"], false);
    assert!(rest["result"]["next"].is_null());
    assert_eq!(names(&rest).last().map(String::as_str), Some("f01149.txt"));
    // A request may ask for a page of its own size: one answer, no ceiling of the host's.
    let all = ask_page(json!({"limit": total}));
    assert_eq!(names(&all).len(), total, "{all}");
    assert_eq!(all["result"]["more"], false);
    let small = ask_page(json!({"limit": 5, "offset": 5}));
    assert_eq!(names(&small)[0], "f00005.txt", "{small}");
    assert_eq!(small["result"]["next"], 10);
    for bad in [
        json!({"limit": 0}),
        json!({"offset": -1}),
        json!({"limit": "x"}),
    ] {
        let answer = ask_page(bad.clone());
        assert_eq!(answer["error"]["code"], BAD_PARAMS, "{bad}: {answer}");
    }
    let _ = fs::remove_dir_all(&place.tmp);
}

/// A name with a direction mark or a control character is listed, shown as what it is, and its path still opens it.
#[cfg(unix)]
#[test]
fn every_name_is_listed_with_what_hides_in_it_shown() {
    let place = place("names");
    let odd = place.work.join("a\u{202e}txt.exe");
    fs::write(&odd, "x").expect("file");
    let control = control_over(&place.home, &place.work);
    let lister = key(&[Grant::Files, Grant::Folders]);
    let listed = ask(
        &control,
        &lister,
        "fs/list",
        json!({"path": text(&place.work), "files": true}),
    );
    assert_eq!(names(&listed), ["a<U+202E>txt.exe"], "{listed}");
    let path = listed["result"]["entries"][0]["path"]
        .as_str()
        .expect("a path");
    let read = read(&control, &reader(), Path::new(path));
    assert_eq!(read["result"]["name"], "a<U+202E>txt.exe", "{read}");
    assert_eq!(bytes_of(&read), b"x");
    let _ = fs::remove_dir_all(&place.tmp);
}
