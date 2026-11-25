use canoa::store::catalog::{FileCheck, parse_releases};
use canoa::store::feed::FixedReleases;
use canoa::store::recipe::Recipe;
use canoa::store::{Check, Platform};

use super::*;
use crate::PowerShellRecipe;

/// What GitHub answered for the repository's releases, trimmed to the files this program takes: two stable releases and a
/// release candidate, as it gave them.
const LISTING: &str = include_str!("../tests/fixtures/github_releases.json");

fn catalog() -> canoa::store::catalog::ReleasesCatalog {
    let listing: Value = serde_json::from_str(LISTING).expect("JSON");
    let catalog = feed(REPO).catalog_of(&listing).expect("a catalog");
    parse_releases(&catalog.to_json()).expect("it reads back as a document")
}

#[test]
fn the_catalog_lists_the_stable_releases_newest_first_and_leaves_a_release_candidate_out() {
    let catalog = catalog();
    let versions: Vec<&str> = catalog
        .releases
        .iter()
        .map(|release| release.version.as_str())
        .collect();
    assert_eq!(versions, ["7.7.0", "7.6.10"]);
    assert_eq!(catalog.channels["stable"], "7.7.0");
    assert_eq!(catalog.releases[0].channels, ["stable"]);
    assert!(catalog.releases[1].channels.is_empty());
}

#[test]
fn every_file_is_at_the_releases_download_address_with_the_sha256_github_lists_for_it() {
    let catalog = catalog();
    let listing: Value = serde_json::from_str(LISTING).expect("JSON");
    let newest = &catalog.releases[0];
    assert_eq!(newest.platforms.len(), 4);
    let windows = &newest.platforms["windows-aarch64"];
    let [part] = windows.parts.as_slice() else {
        panic!("one file");
    };
    assert_eq!(part.file.file, "PowerShell-7.7.0-win-arm64.zip");
    assert_eq!(
        part.file.url,
        "https://github.com/PowerShell/PowerShell/releases/download/v7.7.0/PowerShell-7.7.0-win-arm64.zip"
    );
    let listed = listing[0]["assets"]
        .as_array()
        .and_then(|assets| {
            assets
                .iter()
                .find(|asset| asset["name"] == "PowerShell-7.7.0-win-arm64.zip")
        })
        .and_then(|asset| asset["digest"].as_str())
        .and_then(|digest| digest.strip_prefix("sha256:"))
        .expect("a digest in the listing");
    assert_eq!(part.file.check, Some(FileCheck::Sha256(listed.to_owned())));
    assert_eq!(
        newest.platforms["linux-x86_64"].parts[0].file.file,
        "powershell-7.7.0-linux-x64.tar.gz"
    );
}

#[test]
fn the_recipe_plans_from_the_catalog_and_says_how_the_archive_unpacks() {
    let tools = FixedReleases(catalog().to_json()).object();
    let release = PowerShellRecipe
        .release(None, None, &Platform::new("windows", "x86_64"), &tools)
        .expect("a release");
    assert_eq!(release.version, "7.7.0");
    assert_eq!(release.launch, "powershell/pwsh.exe");
    let [part] = release.parts.as_slice() else {
        panic!("one part");
    };
    assert!(matches!(part.download.check, Check::Sha256 { .. }));
    assert_eq!(part.unpack.as_ref().map(|step| step.format), Some("zip"));
    let older = PowerShellRecipe
        .release(
            Some("7.6.10"),
            None,
            &Platform::new("linux", "aarch64"),
            &tools,
        )
        .expect("a listed version");
    assert_eq!(older.launch, "powershell/pwsh");
    assert_eq!(
        older.parts[0].unpack.as_ref().map(|step| step.format),
        Some("tar.gz")
    );
    assert!(
        PowerShellRecipe
            .release(
                Some("7.0.0"),
                None,
                &Platform::new("linux", "x86_64"),
                &tools
            )
            .is_err_and(|why| why.message().contains("lists the release 7.0.0"))
    );
    assert_eq!(
        PowerShellRecipe.latest(None, &tools).version.as_deref(),
        Some("7.7.0")
    );
}

#[test]
fn a_listing_with_nothing_the_program_offers_is_refused() {
    let none: Value = serde_json::json!([
        {"tag_name": "v7.8.0-rc.1", "draft": false, "prerelease": true, "assets": []},
        {"tag_name": "v7.6.0", "draft": false, "prerelease": false, "assets": []}
    ]);
    assert!(feed(REPO).catalog_of(&none).is_err());
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
    for release in &catalog.releases {
        for platform in release.platforms.values() {
            assert!(
                platform
                    .parts
                    .iter()
                    .all(|part| matches!(part.file.check, Some(FileCheck::Sha256(_)))),
                "{}",
                release.version
            );
        }
    }
}
