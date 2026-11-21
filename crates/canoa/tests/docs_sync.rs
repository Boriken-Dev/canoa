//! Keeps the documented feature tables in `README.md` and `AGENTS.md` aligned with `Cargo.toml`, and keeps
//! `CHANGELOG.md` free of repeated subsection headings within one release section.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn feature_names_from_toml(text: &str) -> BTreeSet<String> {
    let mut in_features = false;
    let mut out = BTreeSet::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with('[') {
            in_features = line == "[features]";
            continue;
        }
        if !in_features || line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((name, _)) = line.split_once('=') {
            out.insert(name.trim().to_owned());
        }
    }
    out
}

fn feature_rows_from_markdown(text: &str) -> BTreeSet<String> {
    let mut in_table = false;
    let mut out = BTreeSet::new();
    for raw in text.lines() {
        let trimmed = raw.trim();
        if trimmed.starts_with("| Feature |") {
            in_table = true;
            continue;
        }
        if !in_table {
            continue;
        }
        if !trimmed.starts_with('|') {
            in_table = false;
            continue;
        }
        if !trimmed.starts_with("| `") {
            continue;
        }
        let rest = &trimmed[3..];
        let Some(end) = rest.find('`') else {
            continue;
        };
        let name = &rest[..end];
        if !name.is_empty() {
            out.insert(name.to_owned());
        }
    }
    out
}

#[test]
fn feature_tables_match_cargo_toml() {
    let root = manifest();
    let toml = fs::read_to_string(root.join("Cargo.toml")).expect("Cargo.toml");
    let expected = feature_names_from_toml(&toml);
    assert!(!expected.is_empty());

    let readme = fs::read_to_string(root.join("README.md")).expect("README.md");
    assert_eq!(
        feature_rows_from_markdown(&readme),
        expected,
        "crates/canoa/README.md feature table must match [features] in Cargo.toml"
    );

    let header = fs::read_to_string(root.join("AGENTS.md")).expect("AGENTS.md");
    assert_eq!(
        feature_rows_from_markdown(&header),
        expected,
        "crates/canoa/AGENTS.md feature table must match [features] in Cargo.toml"
    );
}

#[test]
fn changelog_unreleased_has_no_duplicate_subsections() {
    let changelog = manifest().join("..").join("..").join("CHANGELOG.md");
    let Ok(text) = fs::read_to_string(&changelog) else {
        // An unpacked crate tarball has no workspace changelog.
        return;
    };
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for line in text.lines() {
        if line.starts_with("## ") {
            seen.clear();
        } else if let Some(rest) = line.strip_prefix("### ") {
            let key = rest.trim().to_owned();
            assert!(
                seen.insert(key.clone()),
                "duplicate CHANGELOG subsection ### {key}"
            );
        }
    }
}
