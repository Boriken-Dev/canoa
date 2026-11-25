use canoa::store::catalog::{FileCheck, parse_releases};
use canoa::store::feed::FixedReleases;
use canoa::store::recipe::Recipe;
use canoa::store::source::CatalogSourceProvider as _;
use canoa::store::{Check, Platform};

use super::*;
use crate::NodeRecipe;

/// What nodejs.org answered for `index.json` (its first entries and the first long-term-support ones, trimmed to what the source
/// reads) and for the two release folders' `SHASUMS256.txt` (trimmed to the archives this program takes), as it answered.
const INDEX: &str = include_str!("../tests/fixtures/index.json");
const CURRENT: &str = include_str!("../tests/fixtures/SHASUMS256_v27.0.0-alpha.0.1.1.txt");
const LTS: &str = include_str!("../tests/fixtures/SHASUMS256_v26.13.1.txt");

fn sums() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("27.0.0-alpha.0.1.1".to_owned(), CURRENT.to_owned()),
        ("26.13.1".to_owned(), LTS.to_owned()),
    ])
}

fn catalog() -> canoa::store::catalog::ReleasesCatalog {
    let index: Value = serde_json::from_str(INDEX).expect("JSON");
    let made = catalog_of(ORIGIN, &heads(&index).expect("heads"), &sums()).expect("a catalog");
    parse_releases(&made.to_json()).expect("it reads back as a document")
}

#[test]
fn each_channel_is_at_the_release_the_index_lists_first_for_it() {
    let catalog = catalog();
    assert_eq!(catalog.channels["current"], "27.0.0-alpha.0.1.1");
    assert_eq!(catalog.channels["lts"], "26.13.1");
    let [lts, current] = catalog.releases.as_slice() else {
        panic!("two releases");
    };
    assert_eq!(current.channels, ["current"]);
    assert_eq!(lts.channels, ["lts"]);
}

#[test]
fn the_default_channel_comes_first_in_the_catalog_order_the_recipe_reads() {
    let index: Value = serde_json::from_str(INDEX).expect("JSON");
    let heads = heads(&index).expect("heads");
    assert_eq!(
        heads[0].0, "lts",
        "the store's first channel is its default"
    );
}

#[test]
fn every_archive_has_the_sha256_the_releases_checksums_give_at_its_release_folder() {
    let catalog = catalog();
    let lts = catalog
        .releases
        .iter()
        .find(|release| release.version == "26.13.1")
        .expect("the lts release");
    assert_eq!(lts.platforms.len(), 6);
    let part = &lts.platforms["windows-aarch64"].parts[0];
    assert_eq!(part.file.file, "node-v26.13.1-win-arm64.zip");
    assert_eq!(
        part.file.url,
        "https://nodejs.org/dist/v26.13.1/node-v26.13.1-win-arm64.zip"
    );
    let listed = LTS
        .lines()
        .find(|line| line.ends_with("node-v26.13.1-win-arm64.zip"))
        .and_then(|line| line.split_whitespace().next())
        .expect("a checksum");
    assert_eq!(part.file.check, Some(FileCheck::Sha256(listed.to_owned())));
    assert_eq!(
        lts.platforms["macos-aarch64"].parts[0].file.file,
        "node-v26.13.1-darwin-arm64.tar.gz"
    );
}

#[test]
fn a_release_whose_checksums_name_none_of_the_archives_is_refused() {
    let index: Value = serde_json::from_str(INDEX).expect("JSON");
    let heads = heads(&index).expect("heads");
    let mut none = sums();
    none.insert("26.13.1".to_owned(), "# nothing here\n".to_owned());
    assert!(catalog_of(ORIGIN, &heads, &none).is_err());
    assert!(super::heads(&serde_json::json!({})).is_err());
    assert!(super::heads(&serde_json::json!([{"version": "v1.0.0", "lts": false}])).is_err());
}

#[test]
fn the_recipe_plans_the_archive_unpacked_into_its_own_folder_for_each_system() {
    let tools = FixedReleases(catalog().to_json()).object();
    let windows = NodeRecipe
        .release(None, None, &Platform::new("windows", "x86_64"), &tools)
        .expect("a release");
    assert_eq!(windows.version, "26.13.1", "lts is the default channel");
    assert_eq!(windows.launch, "node/node.exe");
    let part = &windows.parts[0];
    assert_eq!(part.path, "node-archive");
    assert_eq!(
        part.unpack
            .as_ref()
            .map(|step| (step.format, step.into.as_str(), step.strip)),
        Some(("zip", "node", 1))
    );
    assert!(matches!(part.download.check, Check::Sha256 { .. }));
    let current = NodeRecipe
        .release(
            None,
            Some("current"),
            &Platform::new("linux", "aarch64"),
            &tools,
        )
        .expect("a release");
    assert_eq!(current.launch, "node/bin/node");
    assert_eq!(current.version, "27.0.0-alpha.0.1.1");
    assert!(
        current
            .steps
            .iter()
            .any(|step| step.contains("npm") && step.contains("npx"))
    );
    assert_eq!(
        NodeRecipe
            .latest(Some("current"), &tools)
            .version
            .as_deref(),
        Some("27.0.0-alpha.0.1.1")
    );
}

#[test]
#[ignore = "reaches nodejs.org"]
fn the_real_index_and_checksums_give_a_catalog() {
    let get = |url: &str| {
        let out = std::process::Command::new("curl")
            .args(["-sfL", "--retry", "2", url])
            .output()
            .expect("curl runs");
        assert!(out.status.success(), "{url} could not be fetched");
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let index: Value = serde_json::from_str(&get(&format!("{ORIGIN}/index.json"))).expect("JSON");
    let heads = heads(&index).expect("heads");
    let mut live = BTreeMap::new();
    for (_, version) in &heads {
        let message = get(&format!("{ORIGIN}/v{version}/SHASUMS256.txt.asc"));
        let (_, text) = signed(&message, &[]).expect("the release team's own signature");
        live.insert(version.clone(), text);
    }
    let catalog = catalog_of(ORIGIN, &heads, &live).expect("a catalog");
    assert_eq!(catalog.channels.len(), 2);
}

/// What nodejs.org answered for the clearsigned checksums of the two release folders, whole, as it answered.
const CURRENT_SIGNED: &str =
    include_str!("../tests/fixtures/SHASUMS256_v27.0.0-alpha.0.1.1.txt.asc");
const LTS_SIGNED: &str = include_str!("../tests/fixtures/SHASUMS256_v26.13.1.txt.asc");

/// A nodejs.org that answers from the recordings.
struct Recorded {
    lts: String,
}

impl SourceFetch for Recorded {
    fn get_text(&self, url: &str, _max: u64) -> Result<String, ProviderError> {
        match url {
            "https://nodejs.org/dist/index.json" => Ok(INDEX.to_owned()),
            "https://nodejs.org/dist/v26.13.1/SHASUMS256.txt.asc" => Ok(self.lts.clone()),
            "https://nodejs.org/dist/v27.0.0-alpha.0.1.1/SHASUMS256.txt.asc" => {
                Ok(CURRENT_SIGNED.to_owned())
            }
            other => Err(canoa::status::unsupported(format!(
                "no such address: {other}"
            ))),
        }
    }

    fn get_json(&self, url: &str, max: u64) -> Result<String, ProviderError> {
        self.get_text(url, max)
    }
}

#[test]
fn the_checksums_are_read_from_the_signed_text_and_the_catalog_says_who_signed_it() {
    let fetch = Recorded {
        lts: LTS_SIGNED.to_owned(),
    }
    .into_object();
    let catalog = read_catalog(ORIGIN, &fetch, &[]).expect("a signed catalog");
    assert_eq!(
        catalog.signed,
        Some((
            "5BE8A3F6C8A5C01D106C0AD820B1A390B168D356".to_owned(),
            "SHASUMS256.txt".to_owned()
        ))
    );
    let part = &catalog.releases[0].platforms["linux-x86_64"].parts[0].file;
    let listed = LTS
        .lines()
        .find(|line| line.ends_with("node-v26.13.1-linux-x64.tar.gz"))
        .and_then(|line| line.split_whitespace().next())
        .expect("a checksum");
    assert_eq!(part.check, Some(FileCheck::Sha256(listed.to_owned())));
}

#[test]
fn checksums_whose_signature_fails_or_is_by_a_key_the_entry_did_not_name_are_refused() {
    let edited = LTS_SIGNED.replacen(
        "node-v26.13.1-linux-x64.tar.gz",
        "node-v26.13.1-linux-x64.tar.xz",
        1,
    );
    let fetch = Recorded { lts: edited }.into_object();
    let refused = read_catalog(ORIGIN, &fetch, &[]).expect_err("an edited list");
    assert!(
        refused.message().contains("signature is refused"),
        "{refused:?}"
    );
    let fetch = Recorded {
        lts: LTS_SIGNED.to_owned(),
    }
    .into_object();
    let demanded = vec!["DD792F5973C6DE52C432CBDAC77ABFA00DDBF2B7".to_owned()];
    let refused = read_catalog(ORIGIN, &fetch, &demanded).expect_err("another key demanded");
    assert!(
        refused.message().contains("none of the keys"),
        "{refused:?}"
    );
    let plain = LTS_SIGNED
        .lines()
        .skip(3)
        .take_while(|line| !line.starts_with("-----"))
        .collect::<Vec<_>>()
        .join("\n");
    let fetch = Recorded { lts: plain }.into_object();
    assert!(
        read_catalog(ORIGIN, &fetch, &[]).is_err(),
        "unsigned text is not taken"
    );
}

#[test]
fn the_keys_shipped_are_the_release_teams_and_the_source_names_each_by_its_fingerprint() {
    let shipped = openpgp::fingerprints(&[KEYS]).expect("keys");
    for named in [
        "5BE8A3F6C8A5C01D106C0AD820B1A390B168D356",
        "C82FA3AE1CBEDC6BE46B9360C43CEC45C17AB93C",
        "A363A499291CBBC940DD62E41F10027AF002F8B0",
    ] {
        assert!(shipped.iter().any(|each| each == named), "{named}");
    }
    let info = NodeReleases.describe().expect("described");
    assert_eq!(info.keys.len(), 8);
    for key in &info.keys {
        assert!(shipped.iter().any(|each| each == key), "{key} is shipped");
    }
}

#[test]
fn a_version_may_be_named_with_the_v_a_tag_has() {
    let tools = FixedReleases(catalog().to_json()).object();
    let named = NodeRecipe
        .release(
            Some("v26.13.1"),
            None,
            &Platform::new("linux", "x86_64"),
            &tools,
        )
        .expect("the release the catalog lists as 26.13.1");
    assert_eq!(named.version, "26.13.1");
}
