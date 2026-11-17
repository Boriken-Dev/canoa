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
            if kept.contains("extension_catalogs") {
                return kept;
            }
            format!(
                "[store.extension_catalogs]\nteam = \"{catalog}@main#extensions.toml\"\n\n{}",
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
    let ids: Vec<&str> = types["types"]
        .as_array()
        .expect("a list")
        .iter()
        .filter_map(|each| each["id"].as_str())
        .collect();
    assert_eq!(
        ids,
        ["echoes", "pictured"],
        "the installed copy loaded: {types}"
    );
    assert_eq!(types["types"][0]["extension"], "sample");
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
    assert_eq!(
        types["types"].as_array().map(Vec::len),
        Some(0),
        "the host checked the digest it recorded before it loaded the file: {types}"
    );
    drop(link);
    hosted.stop();
}
