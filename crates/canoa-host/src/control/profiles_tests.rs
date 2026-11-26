use std::path::{Path, PathBuf};

use super::files::{Files, UploadLimits};
use super::*;

fn dir(name: &str) -> PathBuf {
    static NEXT: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    let dir =
        std::env::temp_dir().join(format!("canoa-profiles-{name}-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("directory");
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

/// A host whose canoa.cfg is `config` (holding `cfg`), with `home` as the user's home.
fn host(home: &Path, config: &Path, cfg: &str) -> Control {
    write(config, cfg);
    Control::open(
        None,
        DEFAULT_MAX_RECORDS,
        DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens")
    .with_files(Files::new(
        Some(home.to_path_buf()),
        Vec::new(),
        Some(config.to_path_buf()),
        None,
        UploadLimits::default(),
    ))
}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn write(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().expect("a parent")).expect("folder");
    fs::write(path, body).expect("file");
}

fn sha(body: &str) -> String {
    use sha2::{Digest as _, Sha256};
    Sha256::digest(body.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[test]
fn each_method_needs_its_own_grant() {
    let tmp = dir("grants");
    let control = host(&tmp, &tmp.join("cfg").join("canoa.cfg"), "");
    let none = key(&[Grant::StoreRead]);
    for (method, params) in [
        ("profile/canonical", json!({"path": text(&tmp)})),
        ("profile/tree", json!({"directory": text(&tmp)})),
        ("profile/project", json!({"directory": text(&tmp)})),
        ("trust/list", json!({})),
        ("trust/set", json!({"directory": text(&tmp), "sha256": "a"})),
        ("trust/revoke", json!({"directory": text(&tmp)})),
    ] {
        let said = ask(&control, &none, method, params);
        assert_eq!(said["error"]["code"], DENIED, "{method}: {said}");
    }
    // Trusting changes what an agent does, so the grants to read and revoke do not reach it.
    let tames = key(&[Grant::ConfigRead, Grant::ConfigWrite, Grant::Folders]);
    let said = ask(
        &control,
        &tames,
        "trust/set",
        json!({"directory": text(&tmp), "sha256": "a"}),
    );
    assert_eq!(said["error"]["code"], DENIED, "{said}");
    let _ = fs::remove_dir_all(tmp);
}

#[test]
fn an_installed_overlay_is_read_whole_and_other_folders_are_not() {
    let tmp = dir("tree");
    let config = tmp.join("cfg").join("canoa.cfg");
    let control = host(&tmp, &config, "");
    let base = tmp.join("cfg").join("overlays").join("base");
    write(&base.join("overlay.toml"), "name = 1");
    write(&base.join("rules").join("a.md"), "hi");
    write(&base.join(".git").join("config"), "no");
    let reader = key(&[Grant::OverlayRead]);
    // The app names it as it lists it: relative to canoa.cfg's folder.
    let said = ask(
        &control,
        &reader,
        "profile/tree",
        json!({"directory": "overlays/base"}),
    );
    let files = said["result"]["files"]
        .as_object()
        .unwrap_or_else(|| panic!("{said}"));
    assert_eq!(files.len(), 2, "{said}");
    assert_eq!(files["rules/a.md"], "aGk=");
    assert!(!files.contains_key(".git/config"));
    let there = ask(
        &control,
        &reader,
        "profile/canonical",
        json!({"path": "overlays/base"}),
    );
    assert_eq!(there["result"]["directory"], text(&base), "{there}");
    let said = ask(
        &control,
        &reader,
        "profile/tree",
        json!({"directory": text(&tmp)}),
    );
    assert_eq!(said["error"]["code"], REFUSED, "{said}");
    let said = ask(
        &control,
        &Caller::Owner,
        "profile/tree",
        json!({"directory": "../.."}),
    );
    assert_eq!(said["error"]["code"], REFUSED, "{said}");
    let _ = fs::remove_dir_all(tmp);
}

#[test]
fn a_directory_overlay_canoa_cfg_names_is_read_in_place() {
    let tmp = dir("dirs");
    let mine = tmp.join("mine");
    write(&mine.join("overlay.toml"), "x = 1");
    let config = tmp.join("cfg").join("canoa.cfg");
    let control = host(
        &tmp,
        &config,
        &format!("[overlays.mine]\ndir = {:?}\n", text(&mine)),
    );
    let reader = key(&[Grant::OverlayRead]);
    let said = ask(
        &control,
        &reader,
        "profile/tree",
        json!({"directory": text(&mine)}),
    );
    assert_eq!(
        said["result"]["files"]["overlay.toml"], "eCA9IDE=",
        "{said}"
    );
    // A folder canoa.cfg does not name is no overlay.
    let other = tmp.join("other");
    write(&other.join("secret.txt"), "s");
    let said = ask(
        &control,
        &reader,
        "profile/tree",
        json!({"directory": text(&other)}),
    );
    assert_eq!(said["error"]["code"], REFUSED, "{said}");
    let _ = fs::remove_dir_all(tmp);
}

#[test]
fn a_working_folder_is_named_with_its_links_resolved_and_only_where_the_key_may_see() {
    let tmp = dir("canonical");
    let work = tmp.join("work");
    fs::create_dir_all(&work).expect("folder");
    let control = host(&tmp, &tmp.join("cfg").join("canoa.cfg"), "");
    let reader = key(&[Grant::OverlayRead, Grant::Folders]);
    let said = ask(
        &control,
        &reader,
        "profile/canonical",
        json!({"path": text(&work)}),
    );
    assert_eq!(said["result"]["directory"], text(&work), "{said}");
    let said = ask(
        &control,
        &reader,
        "profile/canonical",
        json!({"path": text(&tmp.join("nowhere"))}),
    );
    assert!(said["result"]["directory"].is_null(), "{said}");
    // Outside the home and every working root, a folder is not one this key may name.
    let outside = dir("outside");
    let said = ask(
        &control,
        &reader,
        "profile/canonical",
        json!({"path": text(&outside)}),
    );
    assert!(said["result"]["directory"].is_null(), "{said}");
    let _ = fs::remove_dir_all(outside);
    let _ = fs::remove_dir_all(tmp);
}

#[test]
fn a_project_profile_is_trusted_as_it_was_shown_and_revoked() {
    let tmp = dir("trust");
    let work = tmp.join("work");
    let body = "[mcp]\n";
    write(
        &work.join(canoa::brand::HOME_DIR).join("profile.toml"),
        body,
    );
    let control = host(&tmp, &tmp.join("cfg").join("canoa.cfg"), "");
    let owner = Caller::Owner;
    let said = ask(
        &control,
        &owner,
        "profile/project",
        json!({"directory": text(&work)}),
    );
    assert_eq!(said["result"]["directory"], text(&work), "{said}");
    assert_eq!(said["result"]["profile"], "W21jcF0K", "{said}");
    let said = ask(
        &control,
        &owner,
        "profile/project",
        json!({"directory": text(&tmp)}),
    );
    assert!(said["result"]["profile"].is_null(), "{said}");

    let said = ask(&control, &owner, "trust/list", json!({}));
    assert_eq!(said["result"]["projects"], json!([]), "{said}");

    // What was shown is what is trusted: a profile that is not it is refused.
    let said = ask(
        &control,
        &owner,
        "trust/set",
        json!({"directory": text(&work), "sha256": sha("other")}),
    );
    assert_eq!(said["error"]["code"], REFUSED, "{said}");
    let said = ask(
        &control,
        &owner,
        "trust/set",
        json!({"directory": text(&work), "sha256": sha(body)}),
    );
    let projects = said["result"]["projects"]
        .as_array()
        .unwrap_or_else(|| panic!("{said}"));
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0]["directory"], text(&work));
    assert_eq!(projects[0]["sha256"], sha(body));
    assert!(
        projects[0]["trusted"]
            .as_str()
            .is_some_and(|at| at.ends_with('Z'))
    );

    // Trusting again replaces the record, and a key holding the read grant sees it.
    let again = ask(
        &control,
        &owner,
        "trust/set",
        json!({"directory": text(&work), "sha256": sha(body)}),
    );
    assert_eq!(
        again["result"]["projects"].as_array().map(Vec::len),
        Some(1)
    );
    let said = ask(
        &control,
        &key(&[Grant::ConfigRead]),
        "trust/list",
        json!({}),
    );
    assert_eq!(
        said["result"]["projects"][0]["directory"],
        text(&work),
        "{said}"
    );

    let said = ask(
        &control,
        &key(&[Grant::ConfigWrite]),
        "trust/revoke",
        json!({"directory": text(&work)}),
    );
    assert_eq!(said["result"]["projects"], json!([]), "{said}");
    let said = ask(&control, &owner, "trust/list", json!({}));
    assert_eq!(said["result"]["projects"], json!([]), "{said}");
    let _ = fs::remove_dir_all(tmp);
}

#[test]
fn a_folder_with_no_profile_cannot_be_trusted_and_a_removed_one_can_be_revoked() {
    let tmp = dir("gone");
    let work = tmp.join("work");
    write(&work.join(canoa::brand::HOME_DIR).join("profile.toml"), "x");
    let control = host(&tmp, &tmp.join("cfg").join("canoa.cfg"), "");
    let owner = Caller::Owner;
    let said = ask(
        &control,
        &owner,
        "trust/set",
        json!({"directory": text(&tmp), "sha256": sha("x")}),
    );
    assert_eq!(said["error"]["code"], REFUSED, "{said}");
    let _ = ask(
        &control,
        &owner,
        "trust/set",
        json!({"directory": text(&work), "sha256": sha("x")}),
    );
    let _ = fs::remove_dir_all(&work);
    let said = ask(
        &control,
        &owner,
        "trust/revoke",
        json!({"directory": text(&work)}),
    );
    assert_eq!(said["result"]["projects"], json!([]), "{said}");
    let _ = fs::remove_dir_all(tmp);
}
