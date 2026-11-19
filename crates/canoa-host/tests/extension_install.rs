//! An extension a catalog lists is installed through the control channel, named in canoa.cfg by the host, and loaded by the host
//! when it starts again: the catalog is a local repository holding the sample extension's library, and nothing is fetched from the
//! network. A library that changed after it was installed is not loaded.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod linked;
mod support;

use std::path::{Path, PathBuf};
use std::process::Command;

use linked::{Hosted, ask, entry};
use serde_json::json;
use support::{Scratch, example};

fn git(repo: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.autocrlf=false",
        ])
        .args(args)
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn url(repo: &Path) -> String {
    let text = repo.to_string_lossy().replace('\\', "/");
    if text.starts_with('/') {
        format!("file://{text}")
    } else {
        format!("file:///{text}")
    }
}

/// A catalog repository on `main` that lists the sample extension, whose library it holds, with the digest of its folder.
fn catalog(scratch: &Scratch) -> (PathBuf, String) {
    let repo = scratch.0.join("catalog");
    let folder = repo.join("ext").join("sample");
    std::fs::create_dir_all(&folder).expect("a folder");
    git(&repo, &["init", "--quiet", "-b", "main"]);
    let library = example("sample_extension", true);
    let name = library
        .file_name()
        .expect("a file name")
        .to_string_lossy()
        .into_owned();
    std::fs::copy(&library, folder.join(&name)).expect("the library");
    // A debug build on Linux carries about 500 MB of debug information, past what any extension may hold; what a catalog
    // ships has none. Windows keeps it in a separate file.
    #[cfg(unix)]
    let _ = Command::new("strip")
        .arg("-S")
        .arg(folder.join(&name))
        .status();
    // A native library may be past an overlay's limits, so the pin is computed with the limits the installer reads with.
    let limits = coqui_aos::Settings::default()
        .tree_bytes(coqui_aos::Limit::at(256 * 1024 * 1024))
        .file_bytes(coqui_aos::Limit::at(256 * 1024 * 1024));
    let pin = coqui_aos::tree_digest(&coqui_aos::StdFs, &folder, &limits)
        .expect("a digest")
        .digest
        .to_string();
    std::fs::write(
        repo.join("extensions.toml"),
        format!(
            "[extensions.sample]\nsource = \"./ext/sample\"\npin = \"{pin}\"\nload = \"{name}\"\n\
             provider = \"sample_extension\"\nneeds = {{ kind = \"store_extension\", version = \"1\" }}\n"
        ),
    )
    .expect("the catalog");
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "--quiet", "-m", "the catalog"]);
    (repo, name)
}

fn host(scratch: Scratch, repo: &Path) -> Hosted {
    let catalog = url(repo);
    Hosted::start_in_setup(
        scratch,
        move |dir| {
            let kept = std::fs::read_to_string(dir.join("canoa.cfg")).unwrap_or_default();
            if kept.contains("extensions.catalogs") {
                return kept;
            }
            format!(
                "[store.extensions.catalogs]\nteam = \"{catalog}@main#extensions.toml\"\n\n{}",
                entry("fixture", "")
            )
        },
        &[],
    )
}

/// Installs the sample extension through a host and stops it, giving its scratch directory back.
fn installed() -> (Scratch, PathBuf, PathBuf) {
    let scratch = Scratch::new();
    let (repo, library) = catalog(&scratch);
    let hosted = host(scratch, &repo);
    let link = hosted.local.open("@control").expect("a link");
    let (plan, _) = ask(
        &link,
        1,
        "extension/plan",
        json!({"catalog": "team", "name": "sample"}),
    )
    .expect("a plan");
    assert_eq!(plan["action"], "install", "{plan}");
    assert!(
        std::fs::read_to_string(hosted.scratch.0.join("canoa.cfg"))
            .is_ok_and(|text| !text.contains("host.extensions")),
        "a plan writes nothing"
    );
    let (applied, _) = ask(&link, 2, "extension/apply", json!({"plan": plan})).expect("applied");
    assert_eq!(applied["installed"], true, "{applied}");
    assert_eq!(applied["restart"], true, "the host reads it when it starts");
    let config = std::fs::read_to_string(hosted.scratch.0.join("canoa.cfg")).expect("canoa.cfg");
    assert!(config.contains("[host.extensions.sample]"), "{config}");
    assert!(config.contains("pin = \"sha256:"), "{config}");
    drop(link);
    hosted.stop();
    let linked::Hosted { scratch, ended, .. } = hosted;
    ended.join().expect("it returned").expect("it ended well");
    let kept = scratch
        .0
        .join("home")
        .join("extensions")
        .join("sample")
        .join(&library);
    assert!(
        kept.is_file(),
        "the library is in the host's home: {}",
        kept.display()
    );
    (scratch, repo, kept)
}

#[test]
fn an_extension_installed_from_a_catalog_is_loaded_when_the_host_starts_again() {
    let (scratch, repo, _kept) = installed();
    let hosted = host(scratch, &repo);
    let link = hosted.local.open("@control").expect("a link");
    let (types, _) = ask(&link, 1, "store/extensions", json!({})).expect("the types");
    // The host's own Secret providers type is listed beside what an extension adds.
    let ids: Vec<&str> = types["types"]
        .as_array()
        .expect("a list")
        .iter()
        .filter(|each| each["extension"] != "built-in")
        .filter_map(|each| each["id"].as_str())
        .collect();
    assert_eq!(
        ids,
        ["echoes", "pictured"],
        "the installed copy loaded: {types}"
    );
    assert!(types["types"].as_array().is_some_and(|types| {
        types
            .iter()
            .any(|each| each["id"] == "echoes" && each["extension"] == "sample")
    }));
    drop(link);
    hosted.stop();
}

#[test]
fn a_library_that_changed_after_it_was_installed_is_not_loaded() {
    let (scratch, repo, kept) = installed();
    let mut bytes = std::fs::read(&kept).expect("the library");
    bytes.push(0);
    std::fs::write(&kept, bytes).expect("changed");
    let hosted = host(scratch, &repo);
    let link = hosted.local.open("@control").expect("a link");
    let (types, _) = ask(&link, 1, "store/extensions", json!({})).expect("the types");
    assert!(
        types["types"]
            .as_array()
            .is_some_and(|types| types.iter().all(|each| each["extension"] == "built-in")),
        "the host checked the digest it recorded before it loaded the file: {types}"
    );
    drop(link);
    hosted.stop();
}

// A package of scripts: a folder with a manifest and JavaScript modules, the same bytes on every platform and no build. It is
// installed as a library is, and the host runs it with the QuickJS built into the program, which this test process carries.

/// The SDK's modules a package imports, copied beside it so the package is the whole of what is shipped.
const SDK: [&str; 4] = ["store.js", "profile.js", "kinds.js", "model.js"];

const PACKAGE_MODULE: &str = "\
import { StoreExtension } from './sdk/store.js';
import { EnvProfileItem } from './sdk/profile.js';

export class Notes extends StoreExtension {
  static title = 'Team notes';
  static publisher = 'Example publisher';
  static types = [{ id: 'notes', title: 'Notes', icon: 'extension' }];
}

export class Region extends EnvProfileItem {
  static item = 'region';
  static title = 'Region';
  static values = { type: 'object', properties: { name: { type: 'string', title: 'Name' } }, required: ['name'] };
  static variables = { name: ['REGION'] };
}
";

/// A catalog repository on `main` that lists the package, which it holds as a folder: its pin is the digest of that folder, which a
/// host recomputes over what it fetched.
fn script_catalog(scratch: &Scratch) -> PathBuf {
    let repo = scratch.0.join("script_catalog");
    let folder = repo.join("pkg").join("team_notes");
    std::fs::create_dir_all(folder.join("sdk")).expect("a folder");
    git(&repo, &["init", "--quiet", "-b", "main"]);
    let sdk = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bindings/js/canoa-sdk/src");
    for file in SDK {
        std::fs::copy(sdk.join(file), folder.join("sdk").join(file)).expect("an SDK module");
    }
    std::fs::write(folder.join("team_notes.js"), PACKAGE_MODULE).expect("the module");
    std::fs::write(
        folder.join("team_notes.guatiao"),
        "type = \"js\"\nname = \"team_notes\"\nkinds = [\"store_extension\", \"profile_item\"]\npaths = [\".\"]\n",
    )
    .expect("the manifest");
    std::fs::write(
        folder.join("package.json"),
        "{\"name\": \"team_notes\", \"version\": \"1.0.0\", \"type\": \"module\", \"exports\": \"./team_notes.js\"}\n",
    )
    .expect("the package");
    let pin = coqui_aos::tree_digest(&coqui_aos::StdFs, &folder, &coqui_aos::Settings::default())
        .expect("a digest")
        .digest
        .to_string();
    std::fs::write(
        repo.join("extensions.toml"),
        format!(
            "[extensions.team_notes]\nsource = \"./pkg/team_notes\"\npin = \"{pin}\"\nruntime = \"js\"\n\
             provider = \"team_notes_notes\"\nneeds = {{ kind = \"store_extension\", version = \"1\" }}\n"
        ),
    )
    .expect("the catalog");
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "--quiet", "-m", "the catalog"]);
    repo
}

/// Installs the package through a host that carries QuickJS and stops it, giving its scratch directory back and the digest the
/// host recorded.
fn script_installed(repo: &Path, scratch: Scratch) -> (Scratch, String) {
    canoa_host::bundle_js!();
    let hosted = host(scratch, repo);
    let link = hosted.local.open("@control").expect("a link");
    let (plan, _) = ask(
        &link,
        1,
        "extension/plan",
        json!({"catalog": "team", "name": "team_notes"}),
    )
    .expect("a plan");
    assert_eq!(plan["action"], "install", "{plan}");
    let (applied, _) = ask(&link, 2, "extension/apply", json!({"plan": plan})).expect("applied");
    assert_eq!(applied["installed"], true, "{applied}");
    let config = std::fs::read_to_string(hosted.scratch.0.join("canoa.cfg")).expect("canoa.cfg");
    assert!(config.contains("source = \"bundled:js\""), "{config}");
    let pin = config
        .lines()
        .find_map(|line| line.trim().strip_prefix("pin = \""))
        .and_then(|rest| rest.strip_suffix('"'))
        .expect("the recorded digest")
        .to_owned();
    drop(link);
    hosted.stop();
    let linked::Hosted { scratch, ended, .. } = hosted;
    ended.join().expect("it returned").expect("it ended well");
    (scratch, pin)
}

/// What a host that loaded the package lists of it: the Store type it adds and the profile item it offers.
fn script_listed(hosted: &Hosted) {
    let link = hosted.local.open("@control").expect("a link");
    let (types, _) = ask(&link, 1, "store/extensions", json!({})).expect("the types");
    let notes = types["types"]
        .as_array()
        .expect("a list")
        .iter()
        .find(|each| each["id"] == "notes")
        .unwrap_or_else(|| panic!("the package's type is listed: {types}"));
    assert_eq!(notes["extension"], "team_notes");
    assert_eq!(notes["extension_title"], "Team notes");
    let (items, _) = ask(&link, 2, "profile_items/list", json!({})).expect("the items");
    let region = items["items"]
        .as_array()
        .expect("a list")
        .iter()
        .find(|each| each["id"] == "region")
        .unwrap_or_else(|| panic!("the package's item is listed: {items}"));
    assert_eq!(region["origin"], "team_notes");
    drop(link);
}

#[test]
fn a_package_of_scripts_installed_from_a_catalog_is_listed_by_the_host_when_it_starts_again() {
    let scratch = Scratch::new();
    let repo = script_catalog(&scratch);
    let (scratch, _) = script_installed(&repo, scratch);
    let kept = scratch.0.join("home").join("extensions").join("team_notes");
    assert!(
        kept.join("team_notes.guatiao").is_file() && kept.join("sdk").join("store.js").is_file()
    );
    let hosted = host(scratch, &repo);
    script_listed(&hosted);
    hosted.stop();
}

#[test]
fn a_second_host_given_the_same_catalog_entry_ends_with_the_same_package() {
    let first = Scratch::new();
    let repo = script_catalog(&first);
    let (first, first_pin) = script_installed(&repo, first);
    // Another host, in a home of its own, is told the same catalog; it fetches the package itself and checks the same digest.
    let (second, second_pin) = script_installed(&repo, Scratch::new());
    assert_eq!(
        first_pin, second_pin,
        "both recorded the digest the catalog gave"
    );
    let folder = |scratch: &Scratch| scratch.0.join("home").join("extensions").join("team_notes");
    let digest = |scratch: &Scratch| {
        coqui_aos::tree_digest(
            &coqui_aos::StdFs,
            &folder(scratch),
            &coqui_aos::Settings::default(),
        )
        .expect("a digest")
        .digest
        .to_string()
    };
    assert_eq!(digest(&first), digest(&second));
    assert_eq!(digest(&first), first_pin);
    for scratch in [first, second] {
        let hosted = host(scratch, &repo);
        script_listed(&hosted);
        hosted.stop();
    }
}

#[test]
fn a_script_that_changed_after_it_was_installed_is_not_loaded() {
    let scratch = Scratch::new();
    let repo = script_catalog(&scratch);
    let (scratch, _) = script_installed(&repo, scratch);
    let module = scratch
        .0
        .join("home")
        .join("extensions")
        .join("team_notes")
        .join("team_notes.js");
    let mut text = std::fs::read_to_string(&module).expect("the module");
    text.push_str("// changed\n");
    std::fs::write(&module, text).expect("changed");
    let hosted = host(scratch, &repo);
    let link = hosted.local.open("@control").expect("a link");
    let (types, _) = ask(&link, 1, "store/extensions", json!({})).expect("the types");
    assert!(
        types["types"]
            .as_array()
            .is_some_and(|types| types.iter().all(|each| each["id"] != "notes")),
        "the host checked the digest it recorded before it ran any of the script: {types}"
    );
    drop(link);
    hosted.stop();
}
