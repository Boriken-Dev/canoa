use canoa::store::catalog::{FileCheck, parse_releases};
use canoa::store::feed::FixedReleases;
use canoa::store::recipe::{Recipe, newest_published};
use canoa::store::{Check, Platform};

use super::*;
use crate::BitwardenRecipe;

/// What GitHub answered for the repository's releases, trimmed to the command line's zips and the other clients' tags that sit among
/// them, as it gave them.
const LISTING: &str = include_str!("../tests/fixtures/github_releases.json");

fn listing() -> Value {
    serde_json::from_str(LISTING).expect("JSON")
}

fn catalog() -> canoa::store::catalog::ReleasesCatalog {
    let catalog = feed(REPO).catalog_of(&listing()).expect("a catalog");
    parse_releases(&catalog.to_json()).expect("it reads back as a document")
}

#[test]
fn the_command_lines_releases_are_the_cli_tags_that_are_not_drafts_or_prereleases() {
    let synthetic = serde_json::json!([
        {"tag_name": "web-v2026.11.0", "draft": false, "prerelease": false, "assets": []},
        {"tag_name": "cli-v2026.10.1", "draft": false, "prerelease": false, "assets": []},
        {"tag_name": "cli-v2026.9.0", "draft": false, "prerelease": false, "assets": []},
        {"tag_name": "cli-v2026.11.0-rc1", "draft": false, "prerelease": true, "assets": []},
        {"tag_name": "cli-v2026.12.0", "draft": true, "assets": []}
    ]);
    let found = releases(&synthetic);
    let versions: Vec<&str> = found.iter().map(|each| each.version.as_str()).collect();
    assert_eq!(versions, ["2026.10.1", "2026.9.0"]);
    assert_eq!(
        newest_published(found)
            .map(|release| release.version)
            .as_deref(),
        Some("2026.10.1")
    );
}

#[test]
fn a_platform_is_spelled_the_way_the_publisher_names_its_files() {
    let release = Published {
        version: "1.0.0".to_owned(),
        order: [1, 0, 0],
        tag: "cli-v1.0.0".to_owned(),
        assets: Vec::new(),
    };
    let name = |os: &str, arch: &str| file(&release, os, arch);
    assert_eq!(
        name("windows", "aarch64").as_deref(),
        Some("bw-oss-windows-1.0.0.zip"),
        "a release that lists neither is asked for the open-source build"
    );
    assert_eq!(
        name("linux", "aarch64").as_deref(),
        Some("bw-oss-linux-arm64-1.0.0.zip")
    );
    assert_eq!(
        name("macos", "aarch64").as_deref(),
        Some("bw-oss-macos-arm64-1.0.0.zip")
    );
    let older = Published {
        assets: vec![("bw-linux-arm64-1.0.0.zip".to_owned(), "a".repeat(64))],
        version: release.version.clone(),
        tag: release.tag.clone(),
        order: release.order,
    };
    assert_eq!(
        file(&older, "linux", "aarch64").as_deref(),
        Some("bw-linux-arm64-1.0.0.zip"),
        "the plain name while a release has it"
    );
    assert_eq!(name("freebsd", "x86_64"), None);
}

#[test]
fn the_catalog_lists_only_the_command_line_newest_first_with_the_digest_github_lists() {
    let catalog = catalog();
    let versions: Vec<&str> = catalog
        .releases
        .iter()
        .map(|release| release.version.as_str())
        .collect();
    assert_eq!(
        versions,
        ["2026.11.1", "2026.11.0"],
        "the other clients' tags are left out"
    );
    assert_eq!(catalog.channels["stable"], "2026.11.1");
    let newest = &catalog.releases[0];
    assert_eq!(newest.platforms.len(), 6);
    let part = &newest.platforms["linux-aarch64"].parts[0];
    assert_eq!(
        part.file.file, "bw-oss-linux-arm64-2026.11.1.zip",
        "from 2026.11.0 the publisher names its open-source build"
    );
    assert_eq!(
        catalog.releases[1].platforms["linux-aarch64"].parts[0]
            .file
            .file,
        "bw-oss-linux-arm64-2026.11.0.zip"
    );
    let listed = listing()[0]["assets"]
        .as_array()
        .and_then(|assets| {
            assets
                .iter()
                .find(|asset| asset["name"] == "bw-oss-linux-arm64-2026.11.1.zip")
        })
        .and_then(|asset| asset["digest"].as_str())
        .map(|digest| digest.trim_start_matches("sha256:").to_owned())
        .expect("a digest in the listing");
    assert_eq!(part.file.check, Some(FileCheck::Sha256(listed)));
}

#[test]
fn a_newer_release_without_a_digest_is_said_beside_the_newest_the_store_offers() {
    let digest = format!("sha256:{}", "a".repeat(64));
    let doc = serde_json::json!([
        {"tag_name": "cli-v2026.11.0", "draft": false, "prerelease": false, "assets": [
            {"name": "bw-windows-2026.11.0.zip"}
        ]},
        {"tag_name": "cli-v2026.10.1", "draft": false, "prerelease": false, "assets": [
            {"name": "bw-windows-2026.10.1.zip", "digest": digest}
        ]}
    ]);
    let made = feed(REPO).catalog_of(&doc).expect("falls back");
    assert_eq!(made.channels["stable"], "2026.10.1");
    assert_eq!(made.releases.len(), 1);
    assert!(
        made.releases[0]
            .notices
            .iter()
            .any(|notice| notice.contains("cli-v2026.11.0") && notice.contains("2026.10.1")),
        "{:?}",
        made.releases[0].notices
    );
    let none = serde_json::json!([
        {"tag_name": "cli-v2026.11.0", "draft": false, "prerelease": false, "assets": [
            {"name": "bw-windows-2026.11.0.zip"}
        ]}
    ]);
    assert!(feed(REPO).catalog_of(&none).is_err());
}

#[test]
fn a_platform_the_newest_lacks_is_offered_the_newest_release_that_has_it() {
    let digest = |seed: &str| format!("sha256:{}", seed.repeat(64)[..64].to_owned());
    let doc = serde_json::json!([
        {"tag_name": "cli-v2026.11.0", "draft": false, "prerelease": false, "assets": [
            {"name": "bw-windows-2026.11.0.zip", "digest": digest("b")}
        ]},
        {"tag_name": "cli-v2026.10.1", "draft": false, "prerelease": false, "assets": [
            {"name": "bw-windows-2026.10.1.zip", "digest": digest("a")},
            {"name": "bw-linux-2026.10.1.zip", "digest": digest("c")}
        ]}
    ]);
    let catalog = feed(REPO).catalog_of(&doc).expect("a catalog");
    let tools = FixedReleases(catalog.to_json()).object();
    let linux = BitwardenRecipe
        .release(None, None, &Platform::new("linux", "x86_64"), &tools)
        .expect("the older release that has it");
    assert_eq!(linux.version, "2026.10.1");
    assert!(
        linux
            .notices
            .iter()
            .any(|notice| notice.contains("2026.11.0") && notice.contains("linux-x86_64")),
        "{:?}",
        linux.notices
    );
    let windows = BitwardenRecipe
        .release(None, None, &Platform::new("windows", "x86_64"), &tools)
        .expect("the newest");
    assert_eq!(windows.version, "2026.11.0");
    assert!(
        BitwardenRecipe
            .release(
                Some("2026.11.0"),
                None,
                &Platform::new("linux", "x86_64"),
                &tools
            )
            .is_err(),
        "a version named is not swapped for another"
    );
}

#[test]
fn the_recipe_plans_the_zip_unpacked_into_its_own_folder() {
    let tools = FixedReleases(catalog().to_json()).object();
    let release = BitwardenRecipe
        .release(None, None, &Platform::new("windows", "x86_64"), &tools)
        .expect("a release");
    assert_eq!(release.launch, "bw/bw.exe");
    let [part] = release.parts.as_slice() else {
        panic!("one part");
    };
    assert_eq!(
        part.unpack
            .as_ref()
            .map(|step| (step.format, step.into.as_str())),
        Some(("zip", "bw"))
    );
    assert!(matches!(part.download.check, Check::Sha256 { .. }));
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
