use canoa::store::catalog::{FileCheck, parse_releases};
use canoa::store::feed::FixedReleases;
use canoa::store::recipe::Recipe;
use canoa::store::{Check, Platform};

use super::*;
use crate::PiRecipe;

/// What the npm registry answered for the package's `latest`, trimmed to the name, version and what npm checks it by.
const PACKAGE_DOCUMENT: &str = include_str!("../tests/fixtures/npm_latest.json");

/// What nodejs.org answered, as Node's own source keeps it: the index and the checksums of the two release folders.
const INDEX: &str = include_str!("../../canoa-node/tests/fixtures/index.json");
const CURRENT: &str =
    include_str!("../../canoa-node/tests/fixtures/SHASUMS256_v27.0.0-alpha.0.1.1.txt");
const LTS: &str = include_str!("../../canoa-node/tests/fixtures/SHASUMS256_v26.13.1.txt");

fn node() -> ReleasesCatalog {
    let index: Value = serde_json::from_str(INDEX).expect("JSON");
    let sums = BTreeMap::from([
        ("27.0.0-alpha.0.1.1".to_owned(), CURRENT.to_owned()),
        ("26.13.1".to_owned(), LTS.to_owned()),
    ]);
    canoa_node::catalog_of(
        canoa_node::NODE_ORIGIN.trim_end_matches('/'),
        &canoa_node::heads(&index).expect("heads"),
        &sums,
    )
    .expect("Node's catalog")
}

fn catalog() -> ReleasesCatalog {
    let package: Value = serde_json::from_str(PACKAGE_DOCUMENT).expect("JSON");
    let made = catalog_of(&package, &node()).expect("a catalog");
    parse_releases(&made.to_json()).expect("it reads back as a document")
}

#[test]
fn the_release_is_the_package_version_and_its_files_are_nodes_long_term_support_archives() {
    let catalog = catalog();
    let [release] = catalog.releases.as_slice() else {
        panic!("one release");
    };
    assert_eq!(release.version, "1.6.0");
    assert_eq!(catalog.channels["latest"], "1.6.0");
    assert_eq!(
        release.platforms.len(),
        4,
        "the platforms the Store is measured on"
    );
    let part = &release.platforms["windows-x86_64"].parts[0];
    assert_eq!(part.file.file, "node-v26.13.1-win-x64.zip");
    let listed = LTS
        .lines()
        .find(|line| line.ends_with("node-v26.13.1-win-x64.zip"))
        .and_then(|line| line.split_whitespace().next())
        .expect("a checksum");
    assert_eq!(part.file.check, Some(FileCheck::Sha256(listed.to_owned())));
    assert!(
        release
            .notices
            .iter()
            .any(|notice| notice.contains("sha512-zX1BVEss")),
        "the integrity npm checks the package by is said: {:?}",
        release.notices
    );
}

#[test]
fn a_registry_that_names_no_version_or_a_node_with_no_lts_is_refused() {
    assert!(catalog_of(&serde_json::json!({}), &node()).is_err());
    let package: Value = serde_json::from_str(PACKAGE_DOCUMENT).expect("JSON");
    let none = ReleasesCatalog::new(BTreeMap::new(), Vec::new());
    assert!(catalog_of(&package, &none).is_err());
}

#[test]
fn the_recipe_plans_node_unpacked_and_npm_run_over_it_for_each_system() {
    let tools = FixedReleases(catalog().to_json()).object();
    let windows = PiRecipe
        .release(None, None, &Platform::new("windows", "x86_64"), &tools)
        .expect("a release");
    assert_eq!(windows.version, "1.6.0");
    assert_eq!(
        windows.launch,
        "pi/node_modules/@earendil-works/pi-coding-agent/dist/bundle/cli.js"
    );
    assert_eq!(
        windows.facts.get("node").map(String::as_str),
        Some("node/node.exe")
    );
    let [part] = windows.parts.as_slice() else {
        panic!("one part: Node");
    };
    assert_eq!(part.path, "node-archive");
    assert_eq!(
        part.unpack
            .as_ref()
            .map(|step| (step.format, step.into.as_str(), step.strip)),
        Some(("zip", "node", 1))
    );
    assert!(matches!(part.download.check, Check::Sha256 { .. }));
    assert!(windows.steps.iter().any(|step| step.contains("npm")));
    let linux = PiRecipe
        .release(None, None, &Platform::new("linux", "aarch64"), &tools)
        .expect("a release");
    assert_eq!(
        linux.facts.get("node").map(String::as_str),
        Some("node/bin/node")
    );
    assert_eq!(
        PiRecipe.latest(None, &tools).version.as_deref(),
        Some("1.6.0")
    );
}

#[test]
#[ignore = "reaches registry.npmjs.org and nodejs.org"]
fn the_real_registry_and_node_give_a_catalog() {
    let get = |url: &str| {
        let out = std::process::Command::new("curl")
            .args(["-sfL", "--retry", "2", url])
            .output()
            .expect("curl runs");
        assert!(out.status.success(), "{url} could not be fetched");
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let package: Value =
        serde_json::from_str(&get(&format!("{REGISTRY}/{PACKAGE}/latest"))).expect("JSON");
    let index: Value =
        serde_json::from_str(&get("https://nodejs.org/dist/index.json")).expect("JSON");
    let heads = canoa_node::heads(&index).expect("heads");
    let mut live = BTreeMap::new();
    for (_, version) in &heads {
        live.insert(
            version.clone(),
            get(&format!(
                "https://nodejs.org/dist/v{version}/SHASUMS256.txt"
            )),
        );
    }
    let node = canoa_node::catalog_of("https://nodejs.org/dist", &heads, &live).expect("Node");
    let catalog = catalog_of(&package, &node).expect("a catalog");
    assert!(!catalog.releases[0].platforms.is_empty());
}
