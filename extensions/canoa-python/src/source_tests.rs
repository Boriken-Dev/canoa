use canoa::store::catalog::{FileCheck, parse_releases};
use canoa::store::feed::FixedReleases;
use canoa::store::recipe::Recipe;
use canoa::store::{Check, Platform};

use super::*;
use crate::PythonRecipe;

/// What GitHub answered for the newest release's `SHA256SUMS`, trimmed to the `install_only` archives of the four platforms and the
/// minor releases offered, as it answered.
const SUMS: &str = include_str!("../tests/fixtures/SHA256SUMS");

fn catalog() -> canoa::store::catalog::ReleasesCatalog {
    let made = catalog_of(ORIGIN, SUMS).expect("a catalog");
    parse_releases(&made.to_json()).expect("it reads back as a document")
}

#[test]
fn each_minor_series_is_a_channel_at_its_newest_final_build_and_the_stripped_build_is_not_taken() {
    let catalog = catalog();
    assert_eq!(catalog.channels["3.13"], "3.13.18");
    assert_eq!(catalog.channels["3.11"], "3.11.18");
    let versions: Vec<&str> = catalog
        .releases
        .iter()
        .map(|release| release.version.as_str())
        .collect();
    assert_eq!(
        versions,
        ["3.14.10", "3.13.18", "3.12.16", "3.11.18"],
        "by number"
    );
    let part = &catalog.releases[1].platforms["linux-x86_64"].parts[0];
    assert!(
        part.file.file.ends_with("-install_only.tar.gz"),
        "{}",
        part.file.file
    );
}

#[test]
fn every_archive_has_the_sha256_the_digest_list_gives_at_the_builds_own_tag() {
    let catalog = catalog();
    let release = catalog
        .releases
        .iter()
        .find(|release| release.version == "3.13.18")
        .expect("3.13");
    assert_eq!(release.platforms.len(), 4);
    let part = &release.platforms["windows-aarch64"].parts[0];
    assert_eq!(
        part.file.url,
        "https://github.com/astral-sh/python-build-standalone/releases/download/20261124/cpython-3.13.18%2B20261124-aarch64-pc-windows-msvc-install_only.tar.gz"
    );
    let listed = SUMS
        .lines()
        .find(|line| {
            line.ends_with("cpython-3.13.18+20261124-aarch64-pc-windows-msvc-install_only.tar.gz")
        })
        .and_then(|line| line.split_whitespace().next())
        .expect("a digest");
    assert_eq!(part.file.check, Some(FileCheck::Sha256(listed.to_owned())));
}

#[test]
fn a_prerelease_and_a_list_with_no_final_build_are_never_offered() {
    let with_alpha = format!(
        "{}{}  cpython-3.15.0a1+20261124-x86_64-unknown-linux-gnu-install_only.tar.gz\n",
        SUMS,
        "a".repeat(64)
    );
    let catalog = catalog_of(ORIGIN, &with_alpha).expect("a catalog");
    assert!(
        catalog
            .releases
            .iter()
            .all(|release| !release.version.contains('a'))
    );
    assert!(!catalog.channels.contains_key("3.15"));
    assert!(catalog_of(ORIGIN, "").is_err());
}

#[test]
fn the_recipe_plans_the_archive_unpacked_with_the_interpreters_names_for_each_system() {
    let tools = FixedReleases(catalog().to_json()).object();
    let windows = PythonRecipe
        .release(None, None, &Platform::new("windows", "x86_64"), &tools)
        .expect("a release");
    assert_eq!(windows.version, "3.13.18", "3.13 is the default channel");
    assert_eq!(windows.launch, "python/python.exe");
    let part = &windows.parts[0];
    assert_eq!(part.path, "python-archive");
    assert_eq!(
        part.unpack
            .as_ref()
            .map(|step| (step.format, step.into.as_str(), step.strip)),
        Some(("tar.gz", "python", 1))
    );
    assert!(matches!(part.download.check, Check::Sha256 { .. }));
    let linux = PythonRecipe
        .release(
            None,
            Some("3.11"),
            &Platform::new("linux", "aarch64"),
            &tools,
        )
        .expect("a release");
    assert_eq!(linux.version, "3.11.18");
    assert_eq!(linux.launch, "python/bin/python3.11");
    assert_eq!(linux.facts.get("minor").map(String::as_str), Some("3.11"));
    assert!(linux.steps.iter().any(|step| step.contains("python3")));
    let named = PythonRecipe
        .release(
            Some("3.12.16"),
            None,
            &Platform::new("linux", "x86_64"),
            &tools,
        )
        .expect("a listed version");
    assert_eq!(named.launch, "python/bin/python3.12");
    assert_eq!(
        PythonRecipe.latest(Some("3.14"), &tools).version.as_deref(),
        Some("3.14.10")
    );
}

#[test]
#[ignore = "reaches github.com"]
fn the_real_digest_list_gives_a_catalog() {
    let out = std::process::Command::new("curl")
        .args([
            "-sfL",
            "--retry",
            "2",
            &format!("{ORIGIN}/latest/download/SHA256SUMS"),
        ])
        .output()
        .expect("curl runs");
    assert!(out.status.success(), "the list could not be fetched");
    let catalog = catalog_of(ORIGIN, &String::from_utf8_lossy(&out.stdout)).expect("a catalog");
    assert!(catalog.channels.contains_key("3.13"));
}
