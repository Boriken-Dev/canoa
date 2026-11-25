//! Whether the source of an installed overlay holds something newer: a repository the overlay was installed from, a commit
//! added to it, and what `overlay/updates` says before and after, with the git runs counted to show the cache answering.

use core::time::Duration;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::overlays::{OverlayLimits, Overlays};
use super::overlays_tests::{ask, change, dir, git, key, repo, source};
use super::*;

const HOUR: Duration = Duration::from_secs(3600);

struct Rig {
    overlays: Arc<Overlays>,
    upstream: PathBuf,
}

fn rig() -> Rig {
    let tmp = dir("updates");
    let upstream = repo(
        &tmp,
        "up",
        &[
            ("team/OVERLAY.md", "# team\n"),
            ("team/context/a.md", "alpha\n"),
            ("other/readme.md", "elsewhere\n"),
        ],
    );
    let overlays = Arc::new(Overlays::new(
        Some(tmp.join("cfg").join("overlays")),
        OverlayLimits::default(),
    ));
    Rig { overlays, upstream }
}

/// Installs `name` from `source`, as the app does: a check, then the apply of its plan.
fn install(overlays: &Overlays, name: &str, source: &Value, catalog: Option<&str>) -> Value {
    let mut params = source.clone();
    params["name"] = json!(name);
    if let Some(label) = catalog {
        params["catalog"] = json!(label);
    }
    let who = Caller::Owner;
    let plan = overlays.check(&who, &params, &|_| true).expect("checks");
    overlays
        .apply(&who, &json!({"token": plan["token"], "catalog": catalog}))
        .expect("applies")
}

fn head(repo: &Path) -> String {
    git(repo, &["rev-parse", "HEAD"])
}

fn updates(overlays: &Arc<Overlays>, params: &Value) -> Value {
    overlays.updates(params, HOUR).expect("answers")
}

fn state<'a>(answer: &'a Value, name: &str) -> &'a str {
    answer["overlays"][name]["state"].as_str().expect("a state")
}

#[test]
fn a_branch_that_moved_is_an_update_and_the_cache_answers_without_git() {
    let rig = rig();
    let source = source(&rig.upstream, "team");
    let installed = install(&rig.overlays, "team", &source, None);
    let first = head(&rig.upstream);
    assert_eq!(installed["commit"], first);

    let now = updates(&rig.overlays, &json!({"refresh": true}));
    assert_eq!(state(&now, "team"), "current", "{now}");
    assert_eq!(now["available"], 0);
    assert_eq!(now["overlays"]["team"]["follows"], "branch");

    change(&rig.upstream, &[("team/context/a.md", "beta\n")], "second");
    let second = head(&rig.upstream);
    let found = updates(&rig.overlays, &json!({"refresh": true}));
    assert_eq!(state(&found, "team"), "available", "{found}");
    assert_eq!(found["overlays"]["team"]["latest"], second);
    assert_eq!(found["overlays"]["team"]["installed"], first);
    assert_eq!(found["available"], 1);
    assert_eq!(found["stale"], false);

    // Within the time to live nothing is asked again, however often the page asks.
    let runs = rig.overlays.git_runs();
    for _ in 0..3 {
        let again = updates(&rig.overlays, &json!({}));
        assert_eq!(state(&again, "team"), "available", "{again}");
    }
    assert_eq!(rig.overlays.git_runs(), runs, "the cache ran no git");

    // What the look found, installed, is current at once with no new look.
    install(&rig.overlays, "team", &source, None);
    let installing = rig.overlays.git_runs();
    let after = updates(&rig.overlays, &json!({}));
    assert_eq!(state(&after, "team"), "current", "{after}");
    assert_eq!(after["available"], 0);
    assert_eq!(rig.overlays.git_runs(), installing, "no new look");
}

#[test]
fn a_commit_elsewhere_in_the_repository_is_no_update() {
    let rig = rig();
    install(&rig.overlays, "team", &source(&rig.upstream, "team"), None);
    change(&rig.upstream, &[("other/readme.md", "changed\n")], "other");
    let found = updates(&rig.overlays, &json!({"refresh": true}));
    assert_eq!(state(&found, "team"), "current", "{found}");
    assert_eq!(found["available"], 0);
}

#[test]
fn an_overlay_pinned_to_a_commit_never_has_one() {
    let rig = rig();
    let mut pinned = source(&rig.upstream, "team");
    pinned["ref"] = json!(head(&rig.upstream));
    install(&rig.overlays, "team", &pinned, None);
    change(&rig.upstream, &[("team/context/a.md", "beta\n")], "second");
    let found = updates(&rig.overlays, &json!({"refresh": true}));
    assert_eq!(state(&found, "team"), "pinned", "{found}");
    assert_eq!(found["overlays"]["team"]["follows"], "commit");
    assert_eq!(found["available"], 0);
}

#[test]
fn a_tag_that_names_another_commit_is_an_update() {
    let rig = rig();
    git(&rig.upstream, &["tag", "v1"]);
    let mut tagged = source(&rig.upstream, "team");
    tagged["ref"] = json!("v1");
    install(&rig.overlays, "team", &tagged, None);
    let found = updates(&rig.overlays, &json!({"refresh": true}));
    assert_eq!(state(&found, "team"), "current", "{found}");
    assert_eq!(found["overlays"]["team"]["follows"], "tag");
    change(&rig.upstream, &[("team/context/a.md", "beta\n")], "second");
    git(&rig.upstream, &["tag", "-f", "v1"]);
    let moved = updates(&rig.overlays, &json!({"refresh": true}));
    assert_eq!(state(&moved, "team"), "available", "{moved}");
}

#[test]
fn a_source_that_cannot_be_reached_says_so_and_is_no_update() {
    let rig = rig();
    install(&rig.overlays, "team", &source(&rig.upstream, "team"), None);
    std::fs::remove_dir_all(rig.upstream.join(".git")).expect("gone");
    let found = updates(&rig.overlays, &json!({"refresh": true}));
    assert_eq!(state(&found, "team"), "unreachable", "{found}");
    assert!(found["overlays"]["team"]["problem"].is_string(), "{found}");
    assert_eq!(found["available"], 0);
}

#[test]
fn an_overlay_from_a_catalog_is_looked_at_with_none_of_the_persons_git_settings() {
    let rig = rig();
    let source = source(&rig.upstream, "team");
    install(&rig.overlays, "team", &source, Some("theirs"));
    change(&rig.upstream, &[("team/context/a.md", "beta\n")], "second");
    let found = updates(&rig.overlays, &json!({"refresh": true}));
    assert_eq!(state(&found, "team"), "available", "{found}");
}

#[test]
fn an_overlay_never_looked_at_is_checking_and_then_looked_at_in_the_background() {
    let rig = rig();
    install(&rig.overlays, "team", &source(&rig.upstream, "team"), None);
    let first = updates(&rig.overlays, &json!({}));
    assert_eq!(state(&first, "team"), "checking", "{first}");
    assert_eq!(first["checking"], true);
    let mut answer = first;
    for _ in 0..600 {
        if state(&answer, "team") != "checking" {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
        answer = updates(&rig.overlays, &json!({}));
    }
    assert_eq!(state(&answer, "team"), "current", "{answer}");
}

#[test]
fn a_time_to_live_of_zero_looks_again_every_time() {
    let rig = rig();
    install(&rig.overlays, "team", &source(&rig.upstream, "team"), None);
    rig.overlays
        .look_at_all(Duration::ZERO, true)
        .expect("looks");
    let before = rig.overlays.git_runs();
    rig.overlays
        .look_at_all(Duration::ZERO, false)
        .expect("looks");
    assert!(
        rig.overlays.git_runs() > before,
        "a zero time keeps nothing"
    );
}

#[test]
fn the_method_needs_overlay_read_and_a_refresh_needs_overlay_write() {
    let tmp = dir("updates-wire");
    let overlays = tmp.join("cfg").join("overlays");
    let control = Control::open(
        None,
        DEFAULT_MAX_RECORDS,
        DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens")
    .with_overlays(Overlays::new(Some(overlays), OverlayLimits::default()));
    let reader = key("reader", &[Grant::OverlayRead]);
    let none = key("none", &[]);
    let ok = ask(&control, &reader, "overlay/updates", json!({}));
    assert_eq!(ok["result"]["overlays"], json!({}), "{ok}");
    assert_eq!(ok["result"]["available"], 0);
    let refused = ask(
        &control,
        &reader,
        "overlay/updates",
        json!({"refresh": true}),
    );
    assert_eq!(refused["error"]["code"], -32001, "{refused}");
    let nothing = ask(&control, &none, "overlay/updates", json!({}));
    assert_eq!(nothing["error"]["code"], -32001, "{nothing}");
    let writer = key("writer", &[Grant::OverlayRead, Grant::OverlayWrite]);
    let fine = ask(
        &control,
        &writer,
        "overlay/updates",
        json!({"refresh": true}),
    );
    assert!(fine["result"].is_object(), "{fine}");
}
