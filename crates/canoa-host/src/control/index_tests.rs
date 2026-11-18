//! `store/index` against bare repositories: what is fetched, how often, and what is served while a refresh runs.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::overlays_tests::{ask, change, dir, key, repo};
use super::*;
use crate::store::index::{Answer, CatalogSource, StoreIndex, parse_sources};

const CATALOG: &str = r#"name = "Team catalog"
description = "What the team shares"

[overlays.alpha]
source = "./overlays/alpha"
description = "Alpha overlay"
about = { summary = "The first one", tags = ["one", "two"] }

[overlays.beta]
source = "./overlays/beta"

[overlays.broken]
source = "./overlays/missing"

[overlays."Not A Name"]
source = "./overlays/alpha"
"#;

const ALPHA: &str = r#"name = "alpha"
requires = ["base"]
skills = ["review", { name = "lint", source = "./lint" }]

[hooks]
session_start = "echo hi"
"#;

fn url(repo: &Path) -> String {
    let text = repo.to_string_lossy().replace('\\', "/");
    if text.starts_with('/') {
        format!("file://{text}")
    } else {
        format!("file:///{text}")
    }
}

fn upstream(root: &Path) -> PathBuf {
    repo(
        root,
        "catalog",
        &[
            ("catalog.toml", CATALOG),
            ("overlays/alpha/overlay.cfg", ALPHA),
            ("overlays/beta/overlay.cfg", "name = \"beta\"\n"),
        ],
    )
}

struct Rig {
    tmp: PathBuf,
    upstream: PathBuf,
    overlays: Arc<Overlays>,
    cache: PathBuf,
}

impl Rig {
    fn new() -> Self {
        let tmp = dir("index");
        let upstream = upstream(&tmp);
        let overlays = Arc::new(Overlays::new(
            Some(tmp.join("cfg").join("overlays")),
            OverlayLimits::default(),
        ));
        let cache = tmp.join("home").join("cache").join("store");
        Self {
            tmp,
            upstream,
            overlays,
            cache,
        }
    }

    fn rows(&self) -> Vec<crate::store::index::Row> {
        parse_sources(&json!({"team": format!("{}@main#catalog.toml", url(&self.upstream))}))
    }

    fn index(&self, ttl: Duration) -> Arc<StoreIndex> {
        Arc::new(StoreIndex::new(Some(self.cache.clone()), ttl, 1 << 20))
    }

    fn ask(&self, index: &Arc<StoreIndex>, refresh: bool) -> Answer {
        index
            .catalogs(&self.overlays, &self.rows(), refresh)
            .pop()
            .expect("one source")
    }
}

fn names(answer: &Answer) -> Vec<String> {
    answer
        .entries
        .iter()
        .filter_map(|entry| entry["name"].as_str().map(str::to_owned))
        .collect()
}

fn entry<'a>(answer: &'a Answer, name: &str) -> &'a Value {
    answer
        .entries
        .iter()
        .find(|entry| entry["name"] == name)
        .unwrap_or_else(|| panic!("no entry {name} in {:?}", names(answer)))
}

fn settle(index: &StoreIndex) {
    let until = Instant::now() + Duration::from_secs(60);
    while index.refreshing() {
        assert!(Instant::now() < until, "the background refresh did not end");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_refresh_a_caller_asks_again_within_its_gap_is_answered_from_the_cache() {
    let rig = Rig::new();
    let index = rig.index(Duration::from_secs(3600));
    let gap = Duration::from_secs(60);
    let spaced = |refresh: bool, gap: Duration| {
        index
            .catalogs_spaced(&rig.overlays, &rig.rows(), refresh, gap)
            .pop()
            .expect("one source")
    };
    spaced(true, gap);
    let after_first = rig.overlays.git_runs();
    assert!(after_first > 0, "the first refresh fetched");
    for _ in 0..3 {
        let again = spaced(true, gap);
        assert_eq!(again.source["stale"], false, "{}", again.source);
    }
    assert_eq!(
        rig.overlays.git_runs(),
        after_first,
        "a repeat within the gap ran no git"
    );
    spaced(true, Duration::ZERO);
    assert!(
        rig.overlays.git_runs() > after_first,
        "with no gap a refresh fetches as it always did"
    );
}

#[test]
fn the_first_call_indexes_and_a_second_within_the_ttl_runs_no_git() {
    let rig = Rig::new();
    let index = rig.index(Duration::from_secs(3600));
    let first = rig.ask(&index, false);
    assert!(rig.overlays.git_runs() > 0, "the first call went to git");
    assert_eq!(first.source["stale"], false, "{}", first.source);
    assert_eq!(first.source["name"], "Team catalog");
    let alpha = entry(&first, "alpha");
    assert_eq!(alpha["kind"], "overlay");
    assert_eq!(alpha["summary"], "The first one");
    assert_eq!(alpha["tags"], json!(["one", "two"]));
    assert_eq!(alpha["requirements"], json!(["base"]));
    assert_eq!(alpha["contents"]["skills"], json!(["review", "lint"]));
    assert_eq!(alpha["contents"]["hooks"], json!(["session_start"]));
    assert_eq!(
        alpha["origin"],
        json!({"kind": "git", "location": url(&rig.upstream), "ref": "main", "path": "overlays/alpha"})
    );
    assert!(
        alpha["source"]
            .as_str()
            .is_some_and(|s| s.ends_with("@main#overlays/alpha")),
        "{alpha}"
    );

    let before = rig.overlays.git_runs();
    let second = rig.ask(&index, false);
    assert_eq!(
        rig.overlays.git_runs(),
        before,
        "a fresh index runs no git process"
    );
    assert_eq!(second.source["stale"], false);
    assert_eq!(names(&second), names(&first));
}

#[test]
fn a_stale_index_is_served_at_once_while_it_refreshes() {
    let rig = Rig::new();
    let index = rig.index(Duration::ZERO);
    let first = rig.ask(&index, false);
    assert!(!names(&first).contains(&"gamma".to_owned()));
    change(
        &rig.upstream,
        &[
            (
                "catalog.toml",
                &format!("{CATALOG}\n[overlays.gamma]\nsource = \"./overlays/gamma\"\n"),
            ),
            ("overlays/gamma/overlay.cfg", "name = \"gamma\"\n"),
        ],
        "gamma",
    );
    let stale = rig.ask(&index, false);
    assert!(
        !names(&stale).contains(&"gamma".to_owned()),
        "the old index answered"
    );
    assert_eq!(stale.source["stale"], true, "{}", stale.source);
    settle(&index);
    let fresh = rig.ask(&index, false);
    assert!(
        names(&fresh).contains(&"gamma".to_owned()),
        "{:?}",
        names(&fresh)
    );
    assert_ne!(fresh.source["commit"], first.source["commit"]);
    settle(&index);
}

#[test]
fn an_unchanged_remote_is_asked_and_not_fetched() {
    let rig = Rig::new();
    let index = rig.index(Duration::ZERO);
    let first = rig.ask(&index, false);
    let before = rig.overlays.git_runs();
    let again = rig.ask(&index, true);
    assert_eq!(
        rig.overlays.git_runs() - before,
        1,
        "one ls-remote and no fetch while the ref has not moved"
    );
    assert_eq!(again.source["commit"], first.source["commit"]);
    assert_eq!(again.source["stale"], false);
    assert!(again.source["checked"].as_u64() >= first.source["checked"].as_u64());
}

#[test]
fn a_broken_entry_becomes_a_card_with_its_reason_and_hides_none() {
    let rig = Rig::new();
    let index = rig.index(Duration::from_secs(3600));
    let answer = rig.ask(&index, false);
    assert_eq!(entry(&answer, "alpha")["problem"], Value::Null);
    assert_eq!(entry(&answer, "beta")["problem"], Value::Null);
    let broken = entry(&answer, "broken");
    assert!(
        broken["problem"]
            .as_str()
            .is_some_and(|why| why.contains("no OVERLAY.md")),
        "{broken}"
    );
    assert_eq!(broken["contents"], Value::Null);
    let refused = entry(&answer, "Not A Name");
    assert!(refused["problem"].as_str().is_some(), "{refused}");
}

#[test]
fn a_source_that_fails_keeps_its_last_index_marked_stale() {
    let rig = Rig::new();
    let index = rig.index(Duration::from_secs(3600));
    let first = rig.ask(&index, false);
    let moved = rig.tmp.join("catalog-gone");
    std::fs::rename(&rig.upstream, &moved).expect("moves away");
    let failed = rig.ask(&index, true);
    assert_eq!(failed.source["stale"], true, "{}", failed.source);
    assert!(
        failed.source["problem"].as_str().is_some(),
        "{}",
        failed.source
    );
    assert_eq!(
        names(&failed),
        names(&first),
        "the last index is still answered"
    );
    // A later request does not hammer a source that just failed: it is served stale with no attempt before the ttl.
    let again = index_runs(&rig, &index);
    assert_eq!(again, 0, "no attempt while the failure is fresh");
}

fn index_runs(rig: &Rig, index: &Arc<StoreIndex>) -> usize {
    let before = rig.overlays.git_runs();
    let _ = rig.ask(index, false);
    settle(index);
    rig.overlays.git_runs() - before
}

#[test]
fn an_index_is_kept_under_its_own_home_and_read_back_after_a_restart() {
    let rig = Rig::new();
    let index = rig.index(Duration::from_secs(3600));
    let first = rig.ask(&index, false);
    let commit = first.source["commit"].as_str().expect("commit").to_owned();
    let file = rig
        .cache
        .join("s1-catalog-team")
        .join(&commit)
        .join("index.json");
    assert!(file.is_file(), "{}", file.display());
    // Nothing of it is anywhere else: another home's cache is empty, and the source's repository was not written to.
    let other = rig.tmp.join("other").join("cache").join("store");
    assert!(!other.exists());
    assert!(!rig.upstream.join("index.json").exists());
    assert!(
        std::fs::read_dir(rig.cache.join("s1-catalog-team").join(&commit))
            .expect("folder")
            .flatten()
            .all(|found| found.file_name() != "index.json.new"),
        "written through a temporary file that was renamed over"
    );

    // A host started again answers from the disk with no network.
    let restarted = Arc::new(Overlays::new(
        Some(rig.tmp.join("cfg2").join("overlays")),
        OverlayLimits::default(),
    ));
    let index = rig.index(Duration::from_secs(3600));
    let answer = index
        .catalogs(&restarted, &rig.rows(), false)
        .pop()
        .expect("one");
    assert_eq!(restarted.git_runs(), 0);
    assert_eq!(names(&answer), names(&first));
    assert_eq!(answer.source["commit"], commit);
}

#[test]
fn only_the_newest_commit_stays_in_the_cache() {
    let rig = Rig::new();
    let index = rig.index(Duration::from_secs(3600));
    let first = rig.ask(&index, false);
    change(
        &rig.upstream,
        &[("overlays/beta/extra.md", "more\n")],
        "more",
    );
    let second = rig.ask(&index, true);
    assert_ne!(first.source["commit"], second.source["commit"]);
    let kept: Vec<String> = std::fs::read_dir(rig.cache.join("s1-catalog-team"))
        .expect("source folder")
        .flatten()
        .filter_map(|found| found.file_name().to_str().map(str::to_owned))
        .collect();
    assert_eq!(kept, [second.source["commit"].as_str().expect("commit")]);
}

#[test]
fn a_directory_of_overlays_is_a_catalog_in_its_minimal_form() {
    let rig = Rig::new();
    let index = rig.index(Duration::from_secs(3600));
    let rows = parse_sources(&json!({"dirs": format!("{}@main#overlays", url(&rig.upstream))}));
    let answer = index
        .catalogs(&rig.overlays, &rows, false)
        .pop()
        .expect("one");
    assert_eq!(names(&answer), ["alpha", "beta"], "{}", answer.source);
    assert_eq!(
        entry(&answer, "alpha")["contents"]["skills"],
        json!(["review", "lint"])
    );
}

#[test]
fn a_row_that_is_no_source_shows_with_its_reason() {
    let rows = parse_sources(
        &json!({"ok": "https://example.com/c.git@main#c.toml", "nope": 3, "Bad Label": "x@main#c"}),
    );
    assert_eq!(rows.iter().filter(|row| row.is_ok()).count(), 1);
    let bad: Vec<&(String, String)> = rows.iter().filter_map(|row| row.as_ref().err()).collect();
    assert_eq!(bad.len(), 2, "{bad:?}");
    let source: &CatalogSource = rows.iter().find_map(|row| row.as_ref().ok()).expect("one");
    assert_eq!(source.reference, "main");
    assert_eq!(source.path, "c.toml");
}

fn configured(rig: &Rig, cfg_catalogs: &str) -> Control {
    let config = rig.tmp.join("cfg").join("canoa.cfg");
    std::fs::create_dir_all(config.parent().expect("parent")).expect("dir");
    std::fs::write(
        &config,
        format!("[store.overlays.catalogs]\nboriken = false\n{cfg_catalogs}\n"),
    )
    .expect("config");
    Control::open(
        None,
        DEFAULT_MAX_RECORDS,
        DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens")
    .with_files(Files::new(
        None,
        Vec::new(),
        Some(config),
        None,
        UploadLimits::default(),
    ))
    .with_overlays(Overlays::new(
        Some(rig.tmp.join("cfg").join("overlays")),
        OverlayLimits::default(),
    ))
    .with_index(StoreIndex::new(
        Some(rig.cache.clone()),
        Duration::from_secs(3600),
        1 << 20,
    ))
}

#[test]
fn the_grants_are_the_ones_the_catalog_and_program_calls_already_need() {
    let rig = Rig::new();
    let control = configured(
        &rig,
        &format!("team = \"{}@main#catalog.toml\"", url(&rig.upstream)),
    );
    let owner = ask(&control, &Caller::Owner, "store/index", json!({}));
    let sources = owner["result"]["sources"].as_array().expect("sources");
    assert_eq!(sources[0]["label"], "team", "{owner}");
    assert_eq!(sources[0]["entries"], 4, "{owner}");
    assert!(
        owner["result"]["entries"]
            .as_array()
            .is_some_and(|all| all.len() >= 4)
    );

    let reader = key("sha256:aa", &[Grant::OverlayRead]);
    let read = ask(&control, &reader, "store/index", json!({}));
    assert_eq!(read["result"]["sources"][0]["id"], "catalog:team", "{read}");
    assert!(
        read["result"]["sources"]
            .as_array()
            .is_some_and(|all| all.iter().all(|s| s["kind"] != "programs")),
        "a reader of catalogs is not shown programs: {read}"
    );
    // Forcing a fetch is the writer's.
    let refused = ask(&control, &reader, "store/index", json!({"refresh": true}));
    assert_eq!(refused["error"]["code"], DENIED, "{refused}");
    let writer = key("sha256:bb", &[Grant::OverlayRead, Grant::OverlayWrite]);
    let forced = ask(&control, &writer, "store/index", json!({"refresh": true}));
    assert!(forced["result"]["entries"].is_array(), "{forced}");

    for grants in [&[][..], &[Grant::StoreRead, Grant::LedgerRead][..]] {
        let nothing = key("sha256:cc", grants);
        let answer = ask(&control, &nothing, "store/index", json!({}));
        assert_eq!(answer["error"]["code"], DENIED, "{answer}");
    }
    let bad = ask(
        &control,
        &Caller::Owner,
        "store/index",
        json!({"refresh": "yes"}),
    );
    assert_eq!(bad["error"]["code"], -32602, "{bad}");
}

#[test]
fn program_entries_need_the_programs_grant_and_catalog_entries_do_not_leak_to_it() {
    let rig = Rig::new();
    let shelf = crate::store::shelf::Shelf::unchecked(rig.tmp.join("programs"));
    let registry = crate::store::registry::built_in().expect("stores");
    let stores = Stores::new(
        Arc::new(crate::store::programs::Programs::new(shelf, registry)),
        vec!["claude_store".to_owned()],
    );
    let control = configured(
        &rig,
        &format!("team = \"{}@main#catalog.toml\"", url(&rig.upstream)),
    )
    .with_stores(stores);
    let installer = key("sha256:dd", &[Grant::StorePrograms]);
    let answer = ask(&control, &installer, "store/index", json!({}));
    let entries = answer["result"]["entries"].as_array().expect("entries");
    assert!(!entries.is_empty(), "{answer}");
    assert!(
        entries.iter().all(|entry| entry["kind"] == "program"),
        "{answer}"
    );
    assert_eq!(entries[0]["provider"], "claude_store");
    // No network and no catalog fetch for a key that reads programs only.
    assert_eq!(control.overlays.git_runs(), 0);

    let both = ask(&control, &Caller::Owner, "store/index", json!({}));
    let kinds: std::collections::BTreeSet<&str> = both["result"]["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .filter_map(|entry| entry["kind"].as_str())
        .collect();
    assert!(
        kinds.contains("overlay") && kinds.contains("program"),
        "{kinds:?}"
    );
}

#[test]
fn the_tools_read_the_index_with_the_grants_they_were_given_and_never_more() {
    use crate::store::mcp::Backend as _;
    let rig = Rig::new();
    let control = Arc::new(configured(
        &rig,
        &format!("team = \"{}@main#catalog.toml\"", url(&rig.upstream)),
    ));
    let backend = ToolsBackend(Arc::clone(&control));
    let reader: std::collections::BTreeSet<Grant> = [Grant::OverlayRead].into();
    let read = backend.index(&reader, false).expect("a reader reads");
    assert_eq!(read["sources"][0]["id"], "catalog:team", "{read}");
    // The sources are canoa.cfg's, and fetching them again is the grant that fetches.
    let refused = backend
        .index(&reader, true)
        .expect_err("a refresh needs overlay.write");
    assert!(refused.contains("overlay.write"), "{refused}");
    let none: std::collections::BTreeSet<Grant> = [Grant::StoreRead].into();
    assert!(
        backend.index(&none, false).is_err(),
        "a key that reads neither half is refused"
    );
    let fetching: std::collections::BTreeSet<Grant> =
        [Grant::OverlayRead, Grant::OverlayWrite].into();
    assert!(backend.index(&fetching, true).is_ok());
    assert!(
        backend.installed().overlays.is_empty(),
        "nothing is named in this canoa.cfg"
    );
}

#[test]
fn the_default_catalog_is_offered_first_and_a_false_label_removes_it() {
    use crate::store::index::{DEFAULT_CATALOG_LABEL, DEFAULT_CATALOG_SPEC, catalog_rows};
    let labels = |section: serde_json::Value| -> Vec<String> {
        catalog_rows(&section)
            .into_iter()
            .map(|row| row.map_or_else(|(label, _)| label, |source| source.label))
            .collect()
    };
    assert_eq!(labels(json!({})), [DEFAULT_CATALOG_LABEL]);
    assert_eq!(labels(json!(null)), [DEFAULT_CATALOG_LABEL]);
    assert_eq!(
        labels(json!({"aaa": "https://example.org/a.git@main#c.toml"})),
        [DEFAULT_CATALOG_LABEL, "aaa"],
        "a person's own catalog sits beside it, after it"
    );
    assert!(labels(json!({"Boriken": false})).is_empty());
    assert_eq!(
        labels(json!({"boriken": false, "mine": "https://example.org/a.git@main#c.toml"})),
        ["mine"]
    );
    let own = catalog_rows(&json!({"boriken": "https://example.org/b.git@dev#x.toml"}));
    assert_eq!(own.len(), 1, "{own:?}");
    assert_eq!(own[0].as_ref().expect("source").reference, "dev");
    let default = catalog_rows(&json!({}));
    let default = default[0].as_ref().expect("source");
    assert_eq!(default.spec, DEFAULT_CATALOG_SPEC);
    assert_eq!(default.path, "catalog.toml");
}
