use canoa::store::catalog::{FileCheck, parse_releases};
use canoa::store::feed::FixedReleases;
use canoa::store::recipe::Recipe;
use canoa::store::{Check, Platform};

use super::*;
use crate::OpenCodeRecipe;

/// What GitHub answered for the repository's releases, trimmed to the archives this program takes, as it gave them.
const LISTING: &str = include_str!("../tests/fixtures/github_releases.json");

fn listing() -> Value {
    serde_json::from_str(LISTING).expect("JSON")
}

fn catalog() -> canoa::store::catalog::ReleasesCatalog {
    let catalog = feed(REPO).catalog_of(&listing()).expect("a catalog");
    parse_releases(&catalog.to_json()).expect("it reads back as a document")
}

#[test]
fn the_catalog_lists_the_releases_newest_first_with_an_archive_for_each_platform() {
    let catalog = catalog();
    let newest = &catalog.releases[0];
    assert_eq!(catalog.channels["stable"], newest.version);
    assert_eq!(catalog.releases.len(), 2);
    assert_eq!(newest.platforms.len(), 4);
    assert_eq!(
        newest.platforms["windows-x86_64"].parts[0].file.file,
        "opencode-windows-x64.zip"
    );
    assert_eq!(
        newest.platforms["linux-aarch64"].parts[0].file.file,
        "opencode-linux-arm64.tar.gz"
    );
}

#[test]
fn every_file_has_the_sha256_github_lists_for_it_at_its_tags_download_address() {
    let catalog = catalog();
    let newest = &catalog.releases[0];
    let part = &newest.platforms["linux-x86_64"].parts[0];
    let listed = listing()[0]["assets"]
        .as_array()
        .and_then(|assets| {
            assets
                .iter()
                .find(|asset| asset["name"] == "opencode-linux-x64.tar.gz")
        })
        .and_then(|asset| asset["digest"].as_str())
        .map(|digest| digest.trim_start_matches("sha256:").to_owned())
        .expect("a digest in the listing");
    assert_eq!(part.file.check, Some(FileCheck::Sha256(listed)));
    assert_eq!(
        part.file.url,
        format!(
            "https://github.com/anomalyco/opencode/releases/download/v{}/opencode-linux-x64.tar.gz",
            newest.version
        )
    );
}

#[test]
fn the_recipe_plans_from_the_catalog_and_says_where_the_archive_unpacks() {
    let tools = FixedReleases(catalog().to_json()).object();
    let windows = OpenCodeRecipe
        .release(None, None, &Platform::new("windows", "aarch64"), &tools)
        .expect("a release");
    assert_eq!(windows.launch, "pkg/opencode.exe");
    let [part] = windows.parts.as_slice() else {
        panic!("one part");
    };
    assert_eq!(part.path, "opencode.zip");
    assert_eq!(
        part.unpack
            .as_ref()
            .map(|step| (step.format, step.into.as_str())),
        Some(("zip", "pkg"))
    );
    assert!(matches!(part.download.check, Check::Sha256 { .. }));
    let linux = OpenCodeRecipe
        .release(None, None, &Platform::new("linux", "x86_64"), &tools)
        .expect("a release");
    assert_eq!(linux.launch, "pkg/opencode");
    assert_eq!(
        linux.parts[0].unpack.as_ref().map(|step| step.format),
        Some("tar.gz")
    );
    assert_eq!(
        OpenCodeRecipe.latest(None, &tools).version.as_deref(),
        Some(catalog().releases[0].version.as_str())
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
