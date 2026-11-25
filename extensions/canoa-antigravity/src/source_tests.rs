use canoa::store::catalog::{FileCheck, parse_releases};
use canoa::store::feed::FixedReleases;
use canoa::store::recipe::Recipe;
use canoa::store::{Check, Platform};

use super::*;
use crate::AgyRecipe;

/// What the publisher's updater answered for each platform's manifest, as it answered.
const MANIFESTS: &str = include_str!("../tests/fixtures/manifests.json");

fn manifests() -> BTreeMap<String, Value> {
    serde_json::from_str(MANIFESTS).expect("JSON")
}

fn catalog() -> canoa::store::catalog::ReleasesCatalog {
    let made = catalog_of(&manifests()).expect("a catalog");
    parse_releases(&made.to_json()).expect("it reads back as a document")
}

#[test]
fn the_catalog_lists_the_one_version_the_manifests_name_with_a_build_for_each_platform() {
    let catalog = catalog();
    let [release] = catalog.releases.as_slice() else {
        panic!("one release: {:?}", catalog.releases);
    };
    assert_eq!(release.version, "1.5.16");
    assert_eq!(catalog.channels["stable"], "1.5.16");
    assert_eq!(release.platforms.len(), 4);
    let windows = &release.platforms["windows-aarch64"];
    assert_eq!(windows.launch.as_deref(), Some("agy.exe"));
    let part = &windows.parts[0];
    assert!(part.executable);
    assert!(
        part.file
            .url
            .ends_with("/windows-arm/cli_windows_arm64.exe")
    );
    assert!(
        matches!(part.file.check, Some(FileCheck::Sha512(ref hex)) if hex.starts_with("a3d96173"))
    );
    let linux = &release.platforms["linux-x86_64"];
    assert_eq!(
        linux.launch, None,
        "an archive: the recipe says where the program is in it"
    );
    assert_eq!(linux.parts[0].path, "agy.tar.gz");
}

#[test]
fn platforms_that_name_different_versions_make_a_release_each_newest_first() {
    let mut all = manifests();
    all.get_mut("linux_arm64").expect("a manifest")["version"] = serde_json::json!("1.10.0");
    let catalog = catalog_of(&all).expect("a catalog");
    let versions: Vec<&str> = catalog
        .releases
        .iter()
        .map(|release| release.version.as_str())
        .collect();
    assert_eq!(versions, ["1.10.0", "1.5.16"], "by number, not by text");
    assert_eq!(catalog.releases[0].platforms.len(), 1);
    assert_eq!(catalog.channels["stable"], "1.10.0");
    // The newest asked for on a platform the newest does not serve is the newest that does.
    let tools = FixedReleases(catalog.to_json()).object();
    let on_windows = AgyRecipe
        .release(None, None, &Platform::new("windows", "x86_64"), &tools)
        .expect("a release");
    assert_eq!(on_windows.version, "1.5.16");
}

#[test]
fn a_manifest_with_no_digest_or_a_file_that_is_not_https_is_left_out() {
    let mut all = manifests();
    all.get_mut("windows_amd64").expect("a manifest")["sha512"] = serde_json::json!("abc");
    all.get_mut("windows_arm64").expect("a manifest")["url"] = serde_json::json!("http://x/y.exe");
    let catalog = catalog_of(&all).expect("a catalog");
    let keys: Vec<&str> = catalog.releases[0]
        .platforms
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, ["linux-aarch64", "linux-x86_64"]);
    assert!(catalog_of(&BTreeMap::new()).is_err());
}

#[test]
fn the_recipe_plans_the_windows_program_alone_and_the_linux_archive_unpacked() {
    let tools = FixedReleases(catalog().to_json()).object();
    let windows = AgyRecipe
        .release(None, None, &Platform::new("windows", "x86_64"), &tools)
        .expect("a release");
    assert_eq!(windows.launch, "agy.exe");
    assert!(windows.parts[0].unpack.is_none() && windows.parts[0].executable);
    assert!(matches!(
        windows.parts[0].download.check,
        Check::Sha512 { .. }
    ));
    let linux = AgyRecipe
        .release(None, None, &Platform::new("linux", "aarch64"), &tools)
        .expect("a release");
    assert_eq!(linux.launch, "bin/antigravity");
    assert_eq!(
        linux.parts[0]
            .unpack
            .as_ref()
            .map(|step| (step.format, step.into.as_str())),
        Some(("tar.gz", "bin"))
    );
    assert!(
        AgyRecipe
            .release(
                Some("1.0.0"),
                None,
                &Platform::new("linux", "x86_64"),
                &tools
            )
            .is_err(),
        "only what a catalog lists can be named"
    );
}

#[test]
#[ignore = "reaches the publisher's updater"]
fn the_real_manifests_give_a_catalog() {
    let mut live = BTreeMap::new();
    for (os, arch) in MEASURED {
        let name = spelled(os, arch).expect("a name");
        let out = std::process::Command::new("curl")
            .args(["-sfL", "--retry", "2", &format!("{ORIGIN}/{name}.json")])
            .output()
            .expect("curl runs");
        assert!(out.status.success(), "{name} could not be fetched");
        live.insert(name, serde_json::from_slice(&out.stdout).expect("JSON"));
    }
    let catalog = catalog_of(&live).expect("a catalog");
    assert!(!catalog.releases.is_empty());
}
