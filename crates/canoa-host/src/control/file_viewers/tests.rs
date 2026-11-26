use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{MAX_VIEW_BYTES, list, render};
use crate::control::Files;
use crate::file_viewers::standin::with_extension;

fn folder(name: &str) -> PathBuf {
    static NEXT: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, core::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("canoa-viewers-{name}-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("folder");
    dir
}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn row<'a>(answer: &'a Value, id: &str) -> &'a Value {
    answer["viewers"]
        .as_array()
        .expect("a list")
        .iter()
        .find(|each| each["id"] == id)
        .unwrap_or_else(|| panic!("no viewer {id}: {answer}"))
}

#[test]
fn an_extensions_viewers_are_listed_with_their_types_and_one_that_names_none_is_not_offered() {
    let answer = list(&with_extension());
    let ids: Vec<&str> = answer["viewers"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|each| each["id"].as_str().expect("text"))
        .collect();
    assert_eq!(ids, ["broken", "dump", "stamp"], "{answer}");
    let stamp = row(&answer, "stamp");
    assert_eq!(stamp["shows"], "surface");
    assert_eq!(stamp["extensions"], json!(["stamp"]));
    assert_eq!(stamp["mime_types"], json!([]));
    assert_eq!(stamp["priority"], 20);
    assert_eq!(stamp["origin"], "viewers");
    assert_eq!(stamp["help"], "Shows a stamp file.");
    let dump = row(&answer, "dump");
    assert_eq!(dump["shows"], "page");
    assert_eq!(dump["mime_types"], json!(["application/x-dump"]));
    assert_eq!(dump["opens_sections"], false);
}

#[test]
fn a_host_with_no_extension_lists_no_viewer() {
    let none = crate::file_viewers::ViewerCatalog::from_registries(Vec::new());
    assert_eq!(list(&none)["viewers"], json!([]));
}

#[test]
fn a_surface_viewer_draws_the_file_the_host_read_and_is_never_given_its_path() {
    let place = folder("surface");
    let file = place.join("a.stamp");
    fs::write(&file, "h\u{e9}llo").expect("file");
    let answer = render(
        &with_extension(),
        &Files::default(),
        &json!({"id": "stamp", "path": text(&file)}),
    )
    .expect("drawn");
    assert_eq!(answer["shows"], "surface");
    let shown = answer["messages"][0]["createSurface"]["components"][0]["text"]
        .as_str()
        .expect("text");
    assert_eq!(shown, "a.stamp: h\u{e9}llo (6 bytes)");
    let folder_name = place
        .file_name()
        .and_then(|name| name.to_str())
        .expect("a name");
    assert!(
        !answer.to_string().contains(folder_name),
        "the path never reaches the viewer: {answer}"
    );
    let _ = fs::remove_dir_all(&place);
}

#[test]
fn a_page_viewer_is_given_the_bytes_of_a_binary_file_and_its_page_comes_back_sealed() {
    let place = folder("page");
    let file = place.join("x.dump");
    fs::write(&file, [0u8, 1, 2, 3, 0, 255]).expect("file");
    let answer = render(
        &with_extension(),
        &Files::default(),
        &json!({"id": "dump", "path": text(&file)}),
    )
    .expect("drawn");
    assert_eq!(answer["shows"], "page");
    let document = answer["document"].as_str().expect("text");
    assert!(document.contains("<p id=n>6 bytes read</p>"), "{document}");
    let policy_at = document.find("Content-Security-Policy").expect("a policy");
    let script_at = document.find("<script>").expect("the page's own script");
    assert!(
        policy_at < script_at,
        "the policy comes before anything the page says"
    );
    assert!(document.contains("default-src 'none'"), "{document}");
    let _ = fs::remove_dir_all(&place);
}

#[test]
fn a_file_longer_than_a_viewer_takes_is_handed_over_cut() {
    let place = folder("long");
    let file = place.join("big.dump");
    let size = usize::try_from(MAX_VIEW_BYTES).expect("fits") + 1000;
    fs::write(&file, vec![7u8; size]).expect("file");
    let answer = render(
        &with_extension(),
        &Files::default(),
        &json!({"id": "dump", "path": text(&file)}),
    )
    .expect("drawn");
    let document = answer["document"].as_str().expect("text");
    assert!(
        document.contains(&format!("<p id=n>{MAX_VIEW_BYTES} bytes read</p>")),
        "a viewer reads up to {MAX_VIEW_BYTES} bytes of it"
    );
    let _ = fs::remove_dir_all(&place);
}

#[test]
fn a_viewer_that_is_none_a_file_that_is_none_or_an_answer_that_is_no_surface_is_refused_with_why() {
    let place = folder("refused");
    let file = place.join("a.brk");
    fs::write(&file, "x").expect("file");
    let catalog = with_extension();
    let ask = |params: Value| render(&catalog, &Files::default(), &params).expect_err("refused");
    let unknown = ask(json!({"id": "nothing", "path": text(&file)}));
    assert_eq!(unknown.0, crate::control::NOT_FOUND);
    assert!(unknown.1.contains("no file viewer"), "{}", unknown.1);
    let absent = ask(json!({"id": "stamp", "path": text(&place.join("gone.stamp"))}));
    assert!(absent.1.contains("no file"), "{}", absent.1);
    let relative = ask(json!({"id": "stamp", "path": "a.stamp"}));
    assert!(relative.1.contains("absolute"), "{}", relative.1);
    let missing = ask(json!({"path": text(&file)}));
    assert!(missing.1.contains("id is needed"), "{}", missing.1);
    let broken = ask(json!({"id": "broken", "path": text(&file)}));
    assert_eq!(broken.0, crate::control::REFUSED);
    assert!(broken.1.contains("list of A2UI messages"), "{}", broken.1);
    let _ = fs::remove_dir_all(&place);
}

#[test]
fn listing_needs_the_grant_that_reads_the_configuration_and_drawing_the_one_that_reads_files() {
    use crate::control::{
        Caller, Control, DEFAULT_MAX_RECORD_BYTES, DEFAULT_MAX_RECORDS, DENIED, Grant,
    };
    let control = Control::open(
        None,
        DEFAULT_MAX_RECORDS,
        DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens");
    let ask = |grants: &[Grant], method: &str, params: Value| {
        let caller = Caller::Peer {
            id: "sha256:aa".to_owned(),
            grants: grants.iter().copied().collect(),
        };
        let line = serde_json::to_vec(
            &json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}),
        )
        .expect("encodes");
        serde_json::from_slice::<Value>(&control.handle(&caller, &line).expect("answered"))
            .expect("JSON")
    };
    let refused = ask(&[Grant::StoreRead], "file_viewers/list", json!({}));
    assert_eq!(refused["error"]["code"], DENIED, "{refused}");
    let listed = ask(&[Grant::ConfigRead], "file_viewers/list", json!({}));
    assert_eq!(listed["result"]["viewers"], json!([]), "{listed}");
    let refused = ask(
        &[Grant::ConfigRead],
        "file_viewers/render",
        json!({"id": "stamp", "path": "/a"}),
    );
    assert_eq!(refused["error"]["code"], DENIED, "{refused}");
    let allowed = ask(
        &[Grant::Files],
        "file_viewers/render",
        json!({"id": "stamp", "path": "/a"}),
    );
    assert_eq!(
        allowed["error"]["code"],
        crate::control::NOT_FOUND,
        "a key that reads files gets as far as the viewer: {allowed}"
    );
}
