use canoa::store::catalog::{FileCheck, parse_releases};
use canoa::store::feed::FixedReleases;
use canoa::store::recipe::Recipe;
use canoa::store::{Check, Platform};

use super::*;
use crate::ClaudeRecipe;

/// What the publisher's storage answered for the manifests of the two channels' versions and their signatures, as it answered.
const LATEST: &str = include_str!("../tests/fixtures/manifest_2.1.332.json");
const LATEST_SIGNATURE: &str = include_str!("../tests/fixtures/manifest_2.1.332.json.sig");
const STABLE: &str = include_str!("../tests/fixtures/manifest_2.1.326.json");
const STABLE_SIGNATURE: &str = include_str!("../tests/fixtures/manifest_2.1.326.json.sig");

fn manifests() -> BTreeMap<String, Signed> {
    BTreeMap::from([
        (
            "2.1.332".to_owned(),
            verified(LATEST, LATEST_SIGNATURE, &[]).expect("signed"),
        ),
        (
            "2.1.326".to_owned(),
            verified(STABLE, STABLE_SIGNATURE, &[]).expect("signed"),
        ),
    ])
}

fn channels() -> Vec<(&'static str, String)> {
    vec![
        ("latest", "2.1.332".to_owned()),
        ("stable", "2.1.326".to_owned()),
    ]
}

fn catalog() -> canoa::store::catalog::ReleasesCatalog {
    let made = catalog_of(ORIGIN, &channels(), &manifests()).expect("a catalog");
    parse_releases(&made.to_json()).expect("it reads back as a document")
}

#[test]
fn the_key_shipped_is_the_one_the_source_names() {
    assert_eq!(openpgp::fingerprints(&[KEY]).expect("a key"), [FINGERPRINT]);
}

#[test]
fn each_channel_names_its_version_and_every_program_has_the_sha256_the_signed_manifest_gives() {
    let catalog = catalog();
    assert_eq!(catalog.channels["latest"], "2.1.332");
    assert_eq!(catalog.channels["stable"], "2.1.326");
    assert_eq!(
        catalog.signed,
        Some((FINGERPRINT.to_owned(), "manifest.json".to_owned())),
        "the catalog says who signed what it was made from"
    );
    let latest = &catalog.releases[0];
    assert_eq!(latest.platforms.len(), 6);
    let windows = &latest.platforms["windows-x86_64"];
    assert_eq!(windows.launch.as_deref(), Some("claude.exe"));
    let part = &windows.parts[0];
    assert!(part.executable);
    assert_eq!(
        part.file.url,
        "https://downloads.claude.ai/claude-code-releases/2.1.332/win32-x64/claude.exe"
    );
    assert_eq!(
        part.file.check,
        Some(FileCheck::Sha256(
            "e6652f9170a07df45f1db3e0520f1df0f7b385181ff74f70f49398a67b5fd320".to_owned()
        ))
    );
    assert_eq!(part.file.bytes, Some(403_083_424));
    assert_eq!(
        latest.platforms["linux-aarch64"].parts[0].file.url,
        "https://downloads.claude.ai/claude-code-releases/2.1.332/linux-arm64/claude"
    );
}

#[test]
fn a_manifest_that_was_changed_or_signed_by_another_key_makes_the_whole_catalog_refuse() {
    let changed = LATEST.replacen("e6652f9170a07df4", "0000000000000000", 1);
    let refused = verified(&changed, LATEST_SIGNATURE, &[]).expect_err("refused");
    assert!(
        refused.message().contains("signature is refused"),
        "{refused:?}"
    );
    let other = verified(LATEST, STABLE_SIGNATURE, &[]).expect_err("another document's");
    assert!(other.message().contains("refused"), "{other:?}");
    let demanded = vec!["5BE8A3F6C8A5C01D106C0AD820B1A390B168D356".to_owned()];
    let strangers =
        verified(LATEST, LATEST_SIGNATURE, &demanded).expect_err("a key the entry did not name");
    assert!(
        strangers.message().contains("none of the keys"),
        "{strangers:?}"
    );
    let named = vec![FINGERPRINT.to_ascii_lowercase()];
    assert!(verified(LATEST, LATEST_SIGNATURE, &named).is_ok());
}

#[test]
fn a_manifest_for_another_version_or_with_no_sha256_is_refused() {
    let mut wrong = manifests();
    wrong.insert(
        "2.1.332".to_owned(),
        verified(STABLE, STABLE_SIGNATURE, &[]).expect("signed"),
    );
    assert!(catalog_of(ORIGIN, &channels(), &wrong).is_err());
    assert!(catalog_of(ORIGIN, &channels(), &BTreeMap::new()).is_err());
}

#[test]
fn the_recipe_plans_the_default_channel_and_the_one_the_person_asks_for() {
    let tools = FixedReleases(catalog().to_json()).object();
    let release = ClaudeRecipe
        .release(None, None, &Platform::new("windows", "aarch64"), &tools)
        .expect("a release");
    assert_eq!(release.version, "2.1.332", "latest is the default channel");
    assert_eq!(release.launch, "claude.exe");
    assert!(release.parts[0].executable);
    assert!(matches!(
        release.parts[0].download.check,
        Check::Sha256 { .. }
    ));
    let stable = ClaudeRecipe
        .release(
            None,
            Some("stable"),
            &Platform::new("linux", "x86_64"),
            &tools,
        )
        .expect("a release");
    assert_eq!(stable.version, "2.1.326");
    assert_eq!(stable.launch, "claude");
    let latest = ClaudeRecipe.latest(Some("stable"), &tools);
    assert_eq!(latest.version.as_deref(), Some("2.1.326"));
    assert_eq!(latest.channel.as_deref(), Some("stable"));
}

#[test]
#[ignore = "reaches downloads.claude.ai"]
fn the_real_channels_manifests_and_signatures_give_a_catalog() {
    let get = |url: &str| {
        let out = std::process::Command::new("curl")
            .args(["-sfL", "--retry", "2", url])
            .output()
            .expect("curl runs");
        assert!(out.status.success(), "{url} could not be fetched");
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let key = get("https://downloads.claude.ai/keys/claude-code.asc");
    assert_eq!(
        openpgp::fingerprints(&[key.as_str()]).expect("a key"),
        [FINGERPRINT],
        "the key the publisher serves is the one shipped"
    );
    let mut channels = Vec::new();
    let mut live = BTreeMap::new();
    for channel in CHANNELS {
        let version = get(&format!("{ORIGIN}/{channel}")).trim().to_owned();
        let manifest = get(&format!("{ORIGIN}/{version}/manifest.json"));
        let signature = get(&format!("{ORIGIN}/{version}/manifest.json.sig"));
        live.insert(
            version.clone(),
            verified(&manifest, &signature, &[]).expect("the publisher's own signature"),
        );
        channels.push((channel, version));
    }
    let catalog = catalog_of(ORIGIN, &channels, &live).expect("a catalog");
    assert_eq!(catalog.channels.len(), 2);
}
