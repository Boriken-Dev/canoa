use serde_json::json;

use super::view;
use crate::store::catalog_run::Reading;
use crate::store::catalogs::{Entry, Origin, TypeCatalogs, merge};

const SHA: &str = "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe";

fn kind(format: &str) -> TypeCatalogs {
    TypeCatalogs {
        id: "scope".to_owned(),
        title: "Scope".to_owned(),
        noun: "source".to_owned(),
        plural: "sources".to_owned(),
        lists: "things".to_owned(),
        defaults: Vec::new(),
        fixed: None,
        format: Some(format.to_owned()),
    }
}

fn entry(table: serde_json::Value) -> Entry {
    merge(&kind("canoa.models/1"), &table)
        .into_iter()
        .find_map(|each| each.parsed.ok())
        .expect("an entry")
}

fn reading(text: String) -> Reading {
    Reading {
        text,
        fetched: 5,
        from: "fetched",
        stale: false,
        problem: None,
    }
}

fn models() -> String {
    json!({"format": "canoa.models/1", "revision": "r1", "items": [
        {"id": "checked", "label": "Checked", "files": [{"url": "https://h.example/a.bin", "check": {"sha256": SHA}}]},
        {"id": "bare", "label": "Bare", "files": [{"url": "https://h.example/b.bin"}]},
        {"id": "mixed", "label": "Mixed", "files": [
            {"url": "https://h.example/c.bin", "check": {"sha256": SHA}},
            {"url": "https://h.example/d.bin"}]},
    ]})
    .to_string()
}

#[test]
fn every_item_says_the_catalog_it_came_from_and_how_far_its_digests_can_be_believed() {
    let mine = entry(json!({"mine": {"provider": "hub"}}));
    let shown = view(&kind("canoa.models/1"), &mine, &reading(models())).expect("a view");
    assert_eq!(shown["revision"], "r1");
    let items = shown["items"].as_array().expect("items");
    assert_eq!(items.len(), 3);
    for item in items {
        assert_eq!(item["catalog"]["label"], "mine");
        assert_eq!(item["catalog"]["provider"], "hub");
        assert_eq!(item["catalog"]["origin"], "added");
        assert_eq!(item["catalog"]["revision"], "r1");
    }
    assert_eq!(items[0]["trust"], "verified");
    assert!(items[0].get("problem").is_none());
    for refused in [&items[1], &items[2]] {
        assert!(refused.get("trust").is_none(), "{refused}");
        assert!(
            refused["problem"]
                .as_str()
                .is_some_and(|why| why.contains("trusted = true")),
            "{refused}"
        );
    }
    assert_eq!(items[2]["files"][0]["trust"], "verified");
}

#[test]
fn an_entry_that_is_trusted_takes_files_with_no_digest_and_they_are_drawn_unverified() {
    let mine = entry(json!({"mine": {"provider": "hub", "trusted": true}}));
    let shown = view(&kind("canoa.models/1"), &mine, &reading(models())).expect("a view");
    let items = shown["items"].as_array().expect("items");
    assert_eq!(items[0]["trust"], "verified");
    assert_eq!(items[1]["trust"], "unverified");
    assert_eq!(items[2]["trust"], "unverified", "the weakest file decides");
    assert_eq!(shown["catalog"]["trust"], "unverified");
}

#[test]
fn a_releases_catalog_lists_each_version_with_its_platforms_and_the_weakest_trust() {
    let text = json!({"format": "canoa.releases/1", "channels": {"stable": "1.0.1"}, "releases": [
        {"version": "1.0.1", "channels": ["stable"], "platforms": {"windows-x86_64": {"launch": "p.exe", "parts": [
            {"url": "https://x.example/p.exe", "path": "p.exe", "executable": true, "check": {"sha256": SHA}}]}}},
        {"version": "1.0.0", "platforms": {"linux-x86_64": {"parts": [
            {"url": "https://x.example/p", "path": "p"}]}}},
    ], "signed": {"by": "5A3F", "over": "manifest.json"}})
    .to_string();
    let mut mirror = entry(json!({"internal": {"provider": "rel"}}));
    mirror.origin = Origin::Replaced;
    let shown = view(&kind("canoa.releases/1"), &mirror, &reading(text)).expect("a view");
    assert_eq!(shown["channels"], json!({"stable": "1.0.1"}));
    assert_eq!(
        shown["signed"],
        json!({"by": "5A3F", "over": "manifest.json"})
    );
    let releases = shown["releases"].as_array().expect("releases");
    assert_eq!(releases[0]["trust"], "verified");
    assert_eq!(
        releases[0]["platforms"]["windows-x86_64"]["parts"][0]["executable"],
        true
    );
    assert!(releases[1]["problem"].is_string(), "no digest, not trusted");
    assert_eq!(releases[1]["catalog"]["origin"], "replaced");
}

#[test]
fn a_text_of_the_wrong_format_or_a_scope_with_none_is_no_view() {
    let mine = entry(json!({"mine": {"provider": "hub"}}));
    assert!(view(&kind("canoa.releases/1"), &mine, &reading(models())).is_err());
    let mut plain = kind("canoa.models/1");
    plain.format = None;
    assert!(view(&plain, &mine, &reading(models())).is_err());
}
