//! The changes the store tools make, against a repository on disk and a canoa.cfg beside it: what is kept for a session, what the
//! person is shown, and that only that is applied.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::files::{Files, UploadLimits};
use super::overlays::{OverlayLimits, Overlays};
use super::overlays_tests::{dir, repo};
use super::*;
use crate::store::mcp::{Action, Change, Prepared};

fn grants() -> BTreeSet<Grant> {
    [
        Grant::OverlayRead,
        Grant::OverlayWrite,
        Grant::ConfigWrite,
        Grant::ConfigWriteSessions,
        Grant::Folders,
    ]
    .into()
}

struct Rig {
    tmp: PathBuf,
    control: Control,
    upstream: PathBuf,
    config: PathBuf,
}

impl Rig {
    fn new(config_text: &str) -> Self {
        let tmp = dir("changes");
        let upstream = repo(
            &tmp,
            "alpha",
            &[
                ("overlay.cfg", "name = \"alpha\"\n"),
                (
                    "skills/review/SKILL.md",
                    "---\nname: review\n---\nReview.\n",
                ),
            ],
        );
        let config = tmp.join("cfg").join("canoa.cfg");
        fs::create_dir_all(config.parent().expect("parent")).expect("dir");
        fs::write(&config, config_text).expect("config");
        let control = Control::open(
            None,
            DEFAULT_MAX_RECORDS,
            DEFAULT_MAX_RECORD_BYTES,
            Vec::new(),
        )
        .expect("opens")
        .with_files(Files::new(
            None,
            vec![EntryRoots {
                peers: None,
                roots: vec![tmp.clone()],
            }],
            Some(config.clone()),
            None,
            UploadLimits::default(),
        ))
        .with_overlays(Overlays::new(
            Some(tmp.join("cfg").join("overlays")),
            OverlayLimits::default(),
        ));
        Self {
            tmp,
            control,
            upstream,
            config,
        }
    }

    fn overlays(&self) -> PathBuf {
        self.tmp.join("cfg").join("overlays")
    }

    fn entry(&self) -> Value {
        let text = self.upstream.to_string_lossy().replace('\\', "/");
        let url = if text.starts_with('/') {
            format!("file://{text}")
        } else {
            format!("file:///{text}")
        };
        json!({"kind": "overlay", "id": "team:overlay:alpha", "name": "alpha", "catalog": "team",
            "origin": {"kind": "git", "location": url, "ref": "main", "path": null},
            "contents": {"skills": ["review"], "hooks": ["PreToolUse"], "env_scripts": [], "mcp_servers": []}})
    }

    fn change(&self, action: Action) -> Change {
        Change {
            action,
            kind: "overlay".to_owned(),
            id: "alpha".to_owned(),
            source: None,
            version: None,
            entry: Some(self.entry()),
        }
    }

    fn section(&self, name: &str) -> Value {
        self.control
            .files
            .config_get(&json!({"section": name}))
            .expect("config")["value"]
            .clone()
    }
}

fn path_exists(path: &Path) -> bool {
    path.exists()
}

fn digest_of(prepared: &Prepared) -> &str {
    &prepared.digest
}

#[test]
fn an_install_is_kept_for_the_session_shown_in_full_and_written_only_on_commit() {
    let rig = Rig::new("# the person's own\n");
    let prepared = rig
        .control
        .tool_prepare("s1", &grants(), &rig.change(Action::Install))
        .expect("prepared");
    assert!(prepared.nothing.is_none());
    assert!(digest_of(&prepared).starts_with("sha256:"), "{prepared:?}");
    let card = prepared.lines.join("\n");
    for expected in [
        "Install the overlay alpha",
        "Tree digest: sha256:",
        "Files: 2 in all",
        "+ overlay.cfg",
        "+ skills/review/SKILL.md",
        "The catalog says it holds hooks: PreToolUse",
        "stay off until you allow them at their digest",
    ] {
        assert!(card.contains(expected), "{expected}: {card}");
    }
    assert!(
        !path_exists(&rig.overlays().join("alpha")) && rig.section("overlays").is_null(),
        "nothing is written before the person says yes"
    );
    let done = rig
        .control
        .tool_commit("s1", &grants(), &prepared)
        .expect("applied");
    assert_eq!(done["digest"], digest_of(&prepared));
    assert!(path_exists(
        &rig.overlays().join("alpha").join("overlay.cfg")
    ));
    let entry = &rig.section("overlays")["alpha"];
    assert_eq!(entry["pin"], digest_of(&prepared), "{entry}");
    assert!(
        entry["source"]
            .as_str()
            .is_some_and(|source| source.starts_with("git+file://") && source.ends_with("@main"))
    );
    assert!(
        fs::read_to_string(&rig.config)
            .expect("config")
            .contains("alpha")
    );
    // Asked again, there is nothing to do.
    let again = rig
        .control
        .tool_prepare("s1", &grants(), &rig.change(Action::Update))
        .expect("prepared");
    assert!(
        again
            .nothing
            .as_deref()
            .is_some_and(|why| why.contains("already installed")),
        "{again:?}"
    );
}

#[test]
fn a_plan_is_its_sessions_alone_and_a_digest_other_than_the_one_shown_applies_nothing() {
    let rig = Rig::new("");
    let prepared = rig
        .control
        .tool_prepare("s1", &grants(), &rig.change(Action::Install))
        .expect("prepared");
    // Another session cannot apply it.
    let other = rig
        .control
        .tool_commit("s2", &grants(), &prepared)
        .expect_err("not s2's plan");
    assert!(other.contains("not one this host made"), "{other}");
    // A digest other than the one the host kept is refused, and the plan is given up.
    let mut forged = prepared.clone();
    forged.digest = format!("sha256:{}", "0".repeat(64));
    let moved = rig
        .control
        .tool_commit("s1", &grants(), &forged)
        .expect_err("not what was shown");
    assert!(moved.contains("not what the person was shown"), "{moved}");
    assert!(!path_exists(&rig.overlays().join("alpha")));
    assert!(rig.section("overlays").is_null());
    let gone = rig
        .control
        .tool_commit("s1", &grants(), &prepared)
        .expect_err("the plan was given up");
    assert!(gone.contains("not one this host made"), "{gone}");
    assert!(!path_exists(&rig.overlays().join("alpha")));
}

#[test]
fn a_session_changes_only_what_its_key_may_and_a_refused_change_leaves_nothing_fetched() {
    let rig = Rig::new("");
    for (missing, name) in [
        (Grant::OverlayWrite, "overlay.write"),
        (Grant::ConfigWrite, "config.write"),
        (Grant::ConfigWriteSessions, "config.write.sessions"),
    ] {
        let mut held = grants();
        held.remove(&missing);
        let refused = rig
            .control
            .tool_prepare("s1", &held, &rig.change(Action::Install))
            .expect_err("a grant is missing");
        assert!(refused.contains(name), "{refused}");
    }
    assert!(!path_exists(&rig.overlays().join(".catalogs")));
    // A source on this computer outside what the key may browse is refused by the same check as any caller's.
    let mut held = grants();
    held.remove(&Grant::Folders);
    let refused = rig
        .control
        .tool_prepare("s1", &held, &rig.change(Action::Install))
        .expect_err("no folders grant");
    assert!(refused.contains("folder"), "{refused}");
    // The entry is the Store's: with none there is nothing to fetch, and a source is never taken from the agent.
    let mut nameless = rig.change(Action::Install);
    nameless.entry = None;
    assert!(
        rig.control
            .tool_prepare("s1", &grants(), &nameless)
            .is_err()
    );
}

#[test]
fn an_overlay_the_person_named_by_hand_or_from_elsewhere_is_never_taken_over_or_removed() {
    let rig = Rig::new(
        "[overlays.alpha]\nsource = \"git+https://example.test/other.git@main\"\npin = \"sha256:00\"\n",
    );
    let refused = rig
        .control
        .tool_prepare("s1", &grants(), &rig.change(Action::Install))
        .expect_err("another source");
    assert!(refused.contains("another source or by hand"), "{refused}");
    assert!(!path_exists(&rig.overlays().join("alpha")));
    // Nor is one installed under that name by something other than the store removed.
    let removal = rig
        .control
        .tool_prepare("s1", &grants(), &rig.change(Action::Remove))
        .expect("nothing to do");
    assert!(
        removal
            .nothing
            .as_deref()
            .is_some_and(|why| why.contains("not installed")),
        "{removal:?}"
    );
    assert!(
        rig.section("overlays")["alpha"]["source"]
            .as_str()
            .is_some_and(|source| source.contains("other.git"))
    );
}

#[test]
fn an_update_moves_the_pin_the_store_wrote_and_never_one_the_person_set_by_hand() {
    let rig = Rig::new("");
    let first = rig
        .control
        .tool_prepare("s1", &grants(), &rig.change(Action::Install))
        .expect("prepared");
    rig.control
        .tool_commit("s1", &grants(), &first)
        .expect("installed");
    super::overlays_tests::change(
        &rig.upstream,
        &[(
            "overlay.cfg",
            "name = \"alpha\"
# two
",
        )],
        "two",
    );
    let update = rig
        .control
        .tool_prepare("s1", &grants(), &rig.change(Action::Update))
        .expect("the store's own pin may move, shown");
    let card = update.lines.join(
        "
",
    );
    assert!(card.contains("Update the overlay alpha"), "{card}");
    assert!(
        card.contains("It moves the pin in canoa.cfg from"),
        "{card}"
    );
    assert!(card.contains("~ overlay.cfg"), "{card}");
    assert_ne!(update.digest, first.digest);
    rig.control
        .tool_commit("s1", &grants(), &update)
        .expect("updated");
    assert_eq!(rig.section("overlays")["alpha"]["pin"], update.digest);
    let kept = fs::read_to_string(rig.overlays().join("alpha").join("overlay.cfg")).expect("file");
    assert!(kept.contains("two"));
    // A pin the person wrote by hand is theirs: the next change is refused, and nothing is fetched into place.
    let text = fs::read_to_string(&rig.config).expect("config");
    let by_hand = text.replace(&update.digest, &format!("sha256:{}", "0".repeat(64)));
    assert_ne!(text, by_hand);
    fs::write(&rig.config, by_hand).expect("config");
    super::overlays_tests::change(
        &rig.upstream,
        &[(
            "overlay.cfg",
            "name = \"alpha\"
# three
",
        )],
        "three",
    );
    let refused = rig
        .control
        .tool_prepare("s1", &grants(), &rig.change(Action::Update))
        .expect_err("the pin is the person's");
    assert!(
        refused.contains("a pin set by hand is the person's"),
        "{refused}"
    );
    let kept = fs::read_to_string(rig.overlays().join("alpha").join("overlay.cfg")).expect("file");
    assert!(!kept.contains("three"));
}

#[test]
fn a_removal_is_shown_first_takes_the_overlay_and_its_entry_and_follows_the_state_it_was_shown() {
    let rig = Rig::new("");
    let installed = rig
        .control
        .tool_prepare("s1", &grants(), &rig.change(Action::Install))
        .expect("prepared");
    rig.control
        .tool_commit("s1", &grants(), &installed)
        .expect("installed");
    let prepared = rig
        .control
        .tool_prepare("s1", &grants(), &rig.change(Action::Remove))
        .expect("prepared");
    assert!(
        prepared
            .lines
            .join("\n")
            .contains("Remove the overlay alpha")
    );
    assert!(
        path_exists(&rig.overlays().join("alpha")),
        "shown, not done"
    );
    let mut stale = prepared.clone();
    stale.digest = "sha256:11".to_owned();
    assert!(rig.control.tool_commit("s1", &grants(), &stale).is_err());
    assert!(path_exists(&rig.overlays().join("alpha")));
    rig.control
        .tool_commit("s1", &grants(), &prepared)
        .expect("removed");
    assert!(!path_exists(&rig.overlays().join("alpha")));
    assert!(!path_exists(&rig.overlays().join("alpha.json")));
    assert!(rig.section("overlays").is_null());
}

#[test]
fn the_loadout_turns_an_overlay_off_and_on_for_new_sessions_and_skills_only_when_chosen() {
    let rig = Rig::new(
        "[overlays.alpha]\nsource = \"git+https://example.test/a.git@main\"\n\n[loadout]\nregistries = [\"https://example.test/r\"]\n",
    );
    let off = Change {
        action: Action::Disable,
        kind: "overlay".to_owned(),
        id: "alpha".to_owned(),
        source: None,
        version: None,
        entry: None,
    };
    let prepared = rig
        .control
        .tool_prepare("s1", &grants(), &off)
        .expect("prepared");
    assert!(
        prepared
            .lines
            .join("\n")
            .contains("Turn off the overlay alpha")
    );
    assert!(
        rig.section("loadout").get("off").is_none(),
        "shown, not done"
    );
    rig.control
        .tool_commit("s1", &grants(), &prepared)
        .expect("applied");
    let loadout = rig.section("loadout");
    assert_eq!(loadout["off"], json!(["alpha"]), "{loadout}");
    assert_eq!(
        loadout["registries"],
        json!(["https://example.test/r"]),
        "the rest of the table stays"
    );
    // Turned off already: nothing to ask.
    assert!(
        rig.control
            .tool_prepare("s1", &grants(), &off)
            .expect("prepared")
            .nothing
            .is_some()
    );
    // An overlay [overlays] does not name, and a skill nobody chose, are refused.
    let mut unknown = off.clone();
    unknown.id = "ghost".to_owned();
    assert!(rig.control.tool_prepare("s1", &grants(), &unknown).is_err());
    let skill = Change {
        action: Action::Enable,
        kind: "skill".to_owned(),
        id: "../escape".to_owned(),
        source: None,
        version: None,
        entry: None,
    };
    let refused = rig
        .control
        .tool_prepare("s1", &grants(), &skill)
        .expect_err("not a skill name");
    assert!(refused.contains("not a skill name"), "{refused}");
    let mut chosen = skill.clone();
    chosen.id = "review".to_owned();
    let refused = rig
        .control
        .tool_prepare("s1", &grants(), &chosen)
        .expect_err("not chosen");
    assert!(refused.contains("chose"), "{refused}");
    // A skill's install is the Store app's.
    let mut install = chosen.clone();
    install.action = Action::Install;
    let app = rig
        .control
        .tool_prepare("s1", &grants(), &install)
        .expect_err("the app's");
    assert!(app.contains("Store in the Canoa app"), "{app}");
}
