use canoa::store::catalog::{FileCheck, parse_releases};
use canoa::store::feed::FixedReleases;
use canoa::store::recipe::Recipe;
use canoa::store::{Check, Platform};

use super::*;
use crate::CodexRecipe;

/// What the publisher answered for `channels/latest`, trimmed to the archives this program takes, and for the release's
/// `codex-package_SHA256SUMS`, as it answered.
const DOCUMENT: &str = include_str!("../tests/fixtures/channel_latest.json");
const SUMS: &str = include_str!("../tests/fixtures/codex-package_SHA256SUMS");

fn document() -> Value {
    serde_json::from_str(DOCUMENT).expect("JSON")
}

fn catalog() -> canoa::store::catalog::ReleasesCatalog {
    let made = catalog_of(ORIGIN, &document(), SUMS).expect("a catalog");
    parse_releases(&made.to_json()).expect("it reads back as a document")
}

#[test]
fn the_catalog_lists_the_newest_release_with_the_digest_the_checksums_and_the_channel_agree_on() {
    let catalog = catalog();
    let [release] = catalog.releases.as_slice() else {
        panic!("one release");
    };
    assert_eq!(release.version, "0.172.0");
    assert_eq!(catalog.channels["latest"], "0.172.0");
    assert_eq!(release.platforms.len(), 4);
    let part = &release.platforms["windows-x86_64"].parts[0];
    assert_eq!(
        part.file.file,
        "codex-package-x86_64-pc-windows-msvc.tar.gz"
    );
    assert_eq!(
        part.file.url,
        "https://releases.openai.com/codex/releases/0.172.0/codex-package-x86_64-pc-windows-msvc.tar.gz"
    );
    assert_eq!(
        part.file.check,
        Some(FileCheck::Sha256(
            "b723674d7caf26a3f55121a02d6cad8f3ac8ddf5860747a6edde1c904cf593af".to_owned()
        ))
    );
    let linux = &release.platforms["linux-aarch64"].parts[0].file;
    assert_eq!(
        linux.file,
        "codex-package-aarch64-unknown-linux-musl.tar.gz"
    );
}

#[test]
fn a_digest_the_channel_and_the_checksums_disagree_on_is_refused() {
    let wrong = SUMS.replace("b723674d7caf26a3", "0000000000000000");
    let refused = catalog_of(ORIGIN, &document(), &wrong).expect_err("they disagree");
    assert!(refused.message().contains("disagree"), "{refused:?}");
    assert!(catalog_of(ORIGIN, &serde_json::json!({}), SUMS).is_err());
    assert!(catalog_of(ORIGIN, &document(), "").is_err());
}

#[test]
fn the_recipe_plans_the_package_unpacked_beside_its_program_and_says_it_updates_itself() {
    let tools = FixedReleases(catalog().to_json()).object();
    let release = CodexRecipe
        .release(None, None, &Platform::new("windows", "aarch64"), &tools)
        .expect("a release");
    assert_eq!(release.version, "0.172.0");
    assert_eq!(release.launch, "pkg/bin/codex.exe");
    let [part] = release.parts.as_slice() else {
        panic!("one part");
    };
    assert_eq!(part.path, "codex-package.tar.gz");
    assert_eq!(
        part.unpack
            .as_ref()
            .map(|step| (step.format, step.into.as_str(), step.skip_links)),
        Some(("tar.gz", "pkg", false))
    );
    assert!(matches!(part.download.check, Check::Sha256 { .. }));
    assert!(
        release
            .notices
            .iter()
            .any(|notice| notice.contains("replace itself"))
    );
    assert_eq!(
        CodexRecipe.latest(None, &tools).version.as_deref(),
        Some("0.172.0")
    );
    assert!(
        CodexRecipe
            .release(
                Some("0.100.0"),
                None,
                &Platform::new("linux", "x86_64"),
                &tools
            )
            .is_err_and(|why| why.message().contains("lists the release 0.100.0"))
    );
}

#[test]
#[ignore = "reaches releases.openai.com"]
fn the_real_channel_and_checksums_give_a_catalog() {
    let get = |url: &str| {
        let out = std::process::Command::new("curl")
            .args(["-sfL", "--retry", "2", url])
            .output()
            .expect("curl runs");
        assert!(out.status.success(), "{url} could not be fetched");
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let document: Value =
        serde_json::from_str(&get(&format!("{ORIGIN}/codex/channels/latest"))).expect("JSON");
    let tag = document["tag_name"].as_str().expect("a tag").to_owned();
    let version = tag.trim_start_matches("rust-v");
    let sums = get(&format!(
        "{ORIGIN}/codex/releases/{version}/codex-package_SHA256SUMS"
    ));
    let catalog = catalog_of(ORIGIN, &document, &sums).expect("a catalog");
    assert_eq!(catalog.releases[0].version, version);
}
