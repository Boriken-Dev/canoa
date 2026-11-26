use canoa::store::catalog::{FileCheck, parse_releases};
use canoa::store::feed::FixedReleases;
use canoa::store::recipe::Recipe;
use canoa::store::{Check, Platform};

use super::*;
use crate::GrokRecipe;

/// What the publisher's storage answered for each channel's file, as it answered (the body is the version and a newline).
fn answers() -> Vec<(&'static str, String)> {
    vec![
        ("stable", "1.0.92\n".to_owned()),
        ("alpha", "1.0.93\n".to_owned()),
        ("enterprise", "1.0.93\n".to_owned()),
    ]
}

fn catalog() -> canoa::store::catalog::ReleasesCatalog {
    let made = catalog_of(ORIGIN, &answers()).expect("a catalog");
    parse_releases(&made.to_json()).expect("it reads back as a document")
}

#[test]
fn each_channel_names_its_version_and_a_version_two_channels_share_is_listed_once() {
    let catalog = catalog();
    assert_eq!(catalog.channels["stable"], "1.0.92");
    assert_eq!(catalog.channels["alpha"], "1.0.93");
    let versions: Vec<&str> = catalog
        .releases
        .iter()
        .map(|release| release.version.as_str())
        .collect();
    assert_eq!(versions, ["1.0.92", "1.0.93"]);
    assert_eq!(catalog.releases[1].channels, ["alpha", "enterprise"]);
}

#[test]
fn every_file_is_one_nothing_checks_so_it_is_not_verified() {
    let catalog = catalog();
    let part = &catalog.releases[0].platforms["windows-aarch64"].parts[0];
    assert_eq!(
        part.file.url,
        "https://x.ai/cli/grok-1.0.92-windows-aarch64.exe"
    );
    assert_eq!(part.path, "grok.exe");
    assert!(part.executable);
    assert!(matches!(part.file.check, Some(FileCheck::Unverified(_))));
    assert_eq!(part.file.verified_with(), None);
    let linux = &catalog.releases[0].platforms["linux-x86_64"].parts[0];
    assert_eq!(linux.file.url, "https://x.ai/cli/grok-1.0.92-linux-x86_64");
}

#[test]
fn the_recipe_plans_the_default_channels_newest_and_one_the_person_asks_for() {
    let tools = FixedReleases(catalog().to_json()).object();
    let release = GrokRecipe
        .release(None, None, &Platform::new("linux", "x86_64"), &tools)
        .expect("a release");
    assert_eq!(release.version, "1.0.92", "stable is the default channel");
    assert_eq!(release.launch, "grok");
    assert!(release.parts[0].executable);
    assert!(matches!(
        release.parts[0].download.check,
        Check::Unverified { .. }
    ));
    let alpha = GrokRecipe
        .release(
            None,
            Some("alpha"),
            &Platform::new("windows", "x86_64"),
            &tools,
        )
        .expect("a release");
    assert_eq!(alpha.version, "1.0.93");
    assert_eq!(alpha.launch, "grok.exe");
    let latest = GrokRecipe.latest(Some("enterprise"), &tools);
    assert_eq!(latest.version.as_deref(), Some("1.0.93"));
    assert_eq!(latest.channel.as_deref(), Some("enterprise"));
    assert!(
        GrokRecipe
            .release(
                None,
                Some("beta"),
                &Platform::new("linux", "x86_64"),
                &tools
            )
            .is_err()
    );
}

#[test]
fn an_answer_that_is_no_version_is_refused() {
    let bad = vec![("stable", "<html>an error page</html>".to_owned())];
    assert!(catalog_of(ORIGIN, &bad).is_err());
}

#[test]
#[ignore = "reaches x.ai"]
fn the_real_channels_give_a_catalog_of_programs_that_are_there() {
    let mut live = Vec::new();
    for channel in CHANNELS {
        let out = std::process::Command::new("curl")
            .args(["-sfL", "--retry", "2", &format!("{ORIGIN}/{channel}")])
            .output()
            .expect("curl runs");
        assert!(out.status.success(), "{channel} could not be fetched");
        live.push((channel, String::from_utf8_lossy(&out.stdout).into_owned()));
    }
    let catalog = catalog_of(ORIGIN, &live).expect("a catalog");
    let newest = &catalog.releases[0].platforms["linux-x86_64"].parts[0]
        .file
        .url;
    let head = std::process::Command::new("curl")
        .args(["-sfIL", newest])
        .output()
        .expect("curl runs");
    assert!(head.status.success(), "{newest} is not there");
}
