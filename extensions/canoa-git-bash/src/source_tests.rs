use canoa::store::catalog::{FileCheck, parse_releases};
use canoa::store::feed::FixedReleases;
use canoa::store::recipe::{Recipe, newest_published};
use canoa::store::{Check, Platform};

use super::*;
use crate::GitBashRecipe;

/// What GitHub answered for the repository's releases, trimmed to the portable archives: the stable releases, and the candidates
/// and drafts among them, as it gave them.
const LISTING: &str = include_str!("../tests/fixtures/github_releases.json");

fn listing() -> Value {
    serde_json::from_str(LISTING).expect("JSON")
}

fn catalog() -> canoa::store::catalog::ReleasesCatalog {
    let catalog = feed(REPO).catalog_of(&listing()).expect("a catalog");
    parse_releases(&catalog.to_json()).expect("it reads back as a document")
}

#[test]
fn git_for_windows_releases_are_named_the_way_their_files_are() {
    let synthetic = serde_json::json!([
        {"tag_name": "v2.56.1.windows.1", "draft": false, "prerelease": false, "assets": []},
        {"tag_name": "v2.56.0.windows.4", "draft": false, "prerelease": false, "assets": []},
        {"tag_name": "v2.56.0-rc2.windows.1", "draft": false, "prerelease": true, "assets": []},
        {"tag_name": "v2.99.0.windows.1", "draft": true, "assets": []},
        {"tag_name": "nothing", "draft": false, "assets": []}
    ]);
    let found = releases(&synthetic);
    let versions: Vec<&str> = found.iter().map(|each| each.version.as_str()).collect();
    assert_eq!(versions, ["2.56.1", "2.56.0.4"]);
    assert_eq!(
        newest_published(found).map(|each| each.version).as_deref(),
        Some("2.56.1"),
        "by number, so 2.56.0.4 is older than 2.56.1"
    );
}

#[test]
fn the_catalog_lists_the_stable_releases_newest_first_with_the_digest_github_lists_for_each_archive()
 {
    let catalog = catalog();
    let newest = &catalog.releases[0];
    assert_eq!(catalog.channels["stable"], newest.version);
    assert!(catalog.releases.len() >= 2);
    assert_eq!(
        newest
            .platforms
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["windows-aarch64", "windows-x86_64"],
        "Git for Windows publishes for Windows only"
    );
    let arm = &newest.platforms["windows-aarch64"].parts[0].file;
    assert_eq!(
        arm.file,
        format!("PortableGit-{}-arm64.7z.exe", newest.version)
    );
    let listed = listing()
        .as_array()
        .and_then(|all| {
            all.iter()
                .flat_map(|release| release["assets"].as_array().into_iter().flatten())
                .find(|asset| asset["name"] == arm.file.as_str())
                .and_then(|asset| asset["digest"].as_str())
                .map(|digest| digest.trim_start_matches("sha256:").to_owned())
        })
        .expect("a digest in the listing");
    assert_eq!(arm.check, Some(FileCheck::Sha256(listed)));
    assert!(
        arm.url
            .starts_with("https://github.com/git-for-windows/git/releases/download/v")
    );
}

#[test]
fn the_recipe_plans_the_self_extracting_archive_and_refuses_other_systems() {
    let tools = FixedReleases(catalog().to_json()).object();
    let release = GitBashRecipe
        .release(None, None, &Platform::new("windows", "x86_64"), &tools)
        .expect("a release");
    assert_eq!(release.launch, "git/bin/bash.exe");
    let [part] = release.parts.as_slice() else {
        panic!("one part");
    };
    assert_eq!(part.path, "PortableGit.7z.exe");
    assert!(part.executable);
    assert!(matches!(part.download.check, Check::Sha256 { .. }));
    assert!(
        GitBashRecipe
            .release(None, None, &Platform::new("linux", "x86_64"), &tools)
            .is_err_and(
                |why| why.message().contains("not published") || why.message().contains("Windows")
            )
    );
}

#[test]
#[ignore = "reaches api.github.com"]
fn the_real_listing_gives_a_catalog_with_a_digest_for_every_file() {
    let out = std::process::Command::new("curl")
        .args(["-sfL", "--retry", "2", &feed(REPO).api()])
        .output()
        .expect("curl runs");
    assert!(out.status.success(), "the listing could not be fetched");
    let listing: Value = serde_json::from_slice(&out.stdout).expect("JSON");
    let catalog = feed(REPO).catalog_of(&listing).expect("a catalog");
    assert!(!catalog.releases.is_empty());
}
