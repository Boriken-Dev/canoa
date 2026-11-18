//! `store/catalogs`, and the catalogs a provider answers with, through the control channel.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use serde_json::{Value, json};

use super::overlays_tests::{ask, dir, key};
use super::*;
use crate::store::index::StoreIndex;
use crate::store::programs::Programs;
use crate::store::shelf::Shelf;
use crate::store::sources::tests::{STANDIN_FAILS, with_standin};

struct Rig {
    tmp: PathBuf,
    config: PathBuf,
}

impl Rig {
    fn new(name: &str) -> Self {
        let tmp = dir(name);
        let config = tmp.join("cfg").join("canoa.cfg");
        std::fs::create_dir_all(config.parent().expect("parent")).expect("dir");
        Self { tmp, config }
    }

    fn control(&self, cfg: &str, with_sources: bool) -> Control {
        std::fs::write(&self.config, cfg).expect("config");
        let control = Control::open(
            None,
            DEFAULT_MAX_RECORDS,
            DEFAULT_MAX_RECORD_BYTES,
            Vec::new(),
        )
        .expect("opens")
        .with_files(Files::new(
            None,
            Vec::new(),
            Some(self.config.clone()),
            None,
            UploadLimits::default(),
        ))
        .with_overlays(Overlays::new(
            Some(self.tmp.join("cfg").join("overlays")),
            OverlayLimits::default(),
        ))
        .with_index(StoreIndex::new(
            Some(self.tmp.join("home").join("cache").join("store")),
            Duration::from_secs(3600),
            1 << 20,
        ));
        if with_sources {
            let programs = Programs::new(
                Shelf::unchecked(self.tmp.join("programs")),
                crate::store::registry::built_in().expect("the built-in stores"),
            );
            control.with_stores(
                Stores::new(Arc::new(programs), Vec::new()).with_sources(with_standin()),
            )
        } else {
            control
        }
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.tmp);
    }
}

fn labels(answer: &Value, kind: &str) -> Vec<String> {
    answer["result"]["types"]
        .as_array()
        .expect("types")
        .iter()
        .find(|each| each["type"] == kind)
        .unwrap_or_else(|| panic!("no type {kind} in {answer}"))["catalogs"]
        .as_array()
        .expect("catalogs")
        .iter()
        .map(|row| row["label"].as_str().expect("label").to_owned())
        .collect()
}

#[test]
fn each_type_lists_only_its_own_catalogs() {
    let rig = Rig::new("catalog-types");
    let control = rig.control(
        "[store.overlays.catalogs]\nboriken = false\nteam = \"https://example.org/o.git@main#catalog.toml\"\n\n\
         [store.skills.catalogs]\nanthropics = false\nmine = \"owner/repo\"\n\n\
         [store.mcps.catalogs]\ncorp = \"https://registry.example/\"\n\n\
         [store.extensions.catalogs]\nplugins = \"https://example.org/e.git@main#extensions.toml\"\n",
        false,
    );
    let all = ask(&control, &Caller::Owner, "store/catalogs", json!({}));
    assert_eq!(labels(&all, "overlays"), ["team"], "{all}");
    assert_eq!(labels(&all, "skills"), ["mine"], "{all}");
    assert_eq!(labels(&all, "mcps"), ["official", "corp"], "{all}");
    assert_eq!(labels(&all, "extensions"), ["plugins"], "{all}");

    let one = ask(
        &control,
        &Caller::Owner,
        "store/catalogs",
        json!({"type": "skills"}),
    );
    let types = one["result"]["types"].as_array().expect("types");
    assert_eq!(types.len(), 1, "{one}");
    assert_eq!(types[0]["words"]["noun"], "source");
    assert_eq!(types[0]["words"]["lists"], "skills");
    assert_eq!(types[0]["catalogs"][0]["source"], "owner/repo");

    let unknown = ask(
        &control,
        &Caller::Owner,
        "store/catalogs",
        json!({"type": "toys"}),
    );
    assert!(unknown["error"].is_object(), "{unknown}");
    let reader = key("sha256:aa", &[Grant::OverlayRead]);
    assert!(ask(&control, &reader, "store/catalogs", json!({}))["result"].is_object());
    let none = key("sha256:bb", &[Grant::Hosting]);
    assert_eq!(
        ask(&control, &none, "store/catalogs", json!({}))["error"]["code"],
        DENIED
    );
}

#[test]
fn a_catalog_written_under_one_type_is_not_listed_under_another() {
    let rig = Rig::new("catalog-types-apart");
    let control = rig.control(
        "[store.skills.catalogs]\nonly_skills = \"owner/repo\"\n",
        false,
    );
    let answer = ask(&control, &Caller::Owner, "store/catalogs", json!({}));
    assert_eq!(labels(&answer, "skills"), ["anthropics", "only_skills"]);
    assert_eq!(labels(&answer, "overlays"), ["boriken"], "only its default");
    assert_eq!(
        labels(&answer, "mcps"),
        ["official"],
        "only the official registry"
    );
    assert!(labels(&answer, "extensions").is_empty());
}

#[test]
fn a_source_is_offered_in_the_forms_of_the_types_it_serves_with_the_fields_it_describes() {
    let rig = Rig::new("catalog-types-sources");
    let control = rig.control("", true);
    let answer = ask(&control, &Caller::Owner, "store/catalogs", json!({}));
    let sources = |kind: &str| -> Vec<String> {
        answer["result"]["types"]
            .as_array()
            .expect("types")
            .iter()
            .find(|each| each["type"] == kind)
            .expect("type")["sources"]
            .as_array()
            .expect("sources")
            .iter()
            .map(|each| each["id"].as_str().expect("id").to_owned())
            .collect()
    };
    assert_eq!(
        sources("skills"),
        ["read", "command", "everywhere", "lister", "standin"],
        "{answer}"
    );
    assert_eq!(sources("overlays"), ["read", "command", "everywhere"]);
    assert_eq!(sources("mcps"), ["read", "command", "everywhere", "lister"]);
    assert_eq!(sources("extensions"), ["read", "command", "everywhere"]);

    let skills = answer["result"]["types"]
        .as_array()
        .expect("types")
        .iter()
        .find(|each| each["type"] == "skills")
        .expect("skills");
    let standin = &skills["sources"][4];
    assert_eq!(standin["extension"], "standin_ext");
    let schema = &standin["schema"];
    assert_eq!(schema["required"], json!(["team"]));
    assert_eq!(schema["properties"]["team"]["title"], "Team name");
    assert_eq!(
        schema["properties"]["team"]["description"],
        "Which team's list"
    );
    assert!(
        schema["properties"]["seats"]["type"]
            .to_string()
            .contains("integer")
    );
    assert!(
        standin["form"].is_object(),
        "the form the source declares: {standin}"
    );
    assert!(
        skills["sources"][0]["schema"].is_object(),
        "read asks for its source"
    );
}

#[cfg(windows)]
const SHOW: &str = "[\"cmd\", \"/C\", \"type\", \"catalog.toml\"]";
#[cfg(not(windows))]
const SHOW: &str = "[\"cat\", \"catalog.toml\"]";

const MADE: &str = "name = \"Made by a script\"\n\n[overlays.alpha]\nsource = \"git+https://example.org/alpha.git@main\"\n";

fn made_names(index: &Value) -> Vec<String> {
    index["result"]["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .filter(|entry| entry["catalog"] != "programs")
        .map(|entry| entry["name"].as_str().expect("name").to_owned())
        .collect()
}

#[test]
fn a_program_generates_a_catalog_the_type_lists_and_a_failing_one_keeps_the_last_good() {
    let rig = Rig::new("catalog-command");
    let folder = rig.tmp.join("gen");
    std::fs::create_dir_all(&folder).expect("dir");
    std::fs::write(folder.join("catalog.toml"), MADE).expect("catalog");
    let cwd = folder.to_string_lossy().replace('\\', "/");
    let control = rig.control(
        &format!(
            "[store.overlays.catalogs]\nboriken = false\n\n[store.overlays.catalogs.made]\nprovider = \"command\"\ncommand = {SHOW}\ncwd = \"{cwd}\"\n"
        ),
        false,
    );
    let refresh = json!({"refresh": true});
    let first = ask(&control, &Caller::Owner, "store/index", refresh.clone());
    assert_eq!(made_names(&first), ["alpha"], "{first}");
    let source = &first["result"]["sources"][0];
    assert_eq!(source["label"], "made");
    assert_eq!(source["provider"], "command");
    assert_eq!(source["type"], "overlays");
    assert_eq!(source["name"], "Made by a script");
    assert!(source.get("problem").is_none(), "{source}");

    // The script now fails: the list stays, and the reason is the end of what it said.
    std::fs::remove_file(folder.join("catalog.toml")).expect("removed");
    let failed = ask(&control, &Caller::Owner, "store/index", refresh.clone());
    assert_eq!(
        made_names(&failed),
        ["alpha"],
        "the last good list stays: {failed}"
    );
    let problem = failed["result"]["sources"][0]["problem"]
        .as_str()
        .expect("a problem");
    assert!(problem.contains("ended with"), "{problem}");
    assert_eq!(failed["result"]["sources"][0]["stale"], true);

    // And a script that works again replaces it, read each time the catalog is refreshed.
    std::fs::write(folder.join("catalog.toml"), MADE.replace("alpha", "beta")).expect("catalog");
    let again = ask(&control, &Caller::Owner, "store/index", refresh);
    assert_eq!(made_names(&again), ["beta"], "{again}");
    assert!(again["result"]["sources"][0].get("problem").is_none());
}

#[test]
fn an_extensions_source_generates_a_catalog_the_same_way_and_keeps_the_last_good_on_failure() {
    let rig = Rig::new("catalog-extension-source");
    let control = rig.control(
        "[store.overlays.catalogs]\nboriken = false\n\n[store.overlays.catalogs.theirs]\nprovider = \"everywhere\"\n",
        true,
    );
    STANDIN_FAILS.store(false, Ordering::SeqCst);
    let refresh = json!({"refresh": true});
    let first = ask(&control, &Caller::Owner, "store/index", refresh.clone());
    assert_eq!(made_names(&first), ["made"], "{first}");
    assert_eq!(first["result"]["sources"][0]["provider"], "everywhere");
    assert_eq!(first["result"]["sources"][0]["name"], "From theirs");

    STANDIN_FAILS.store(true, Ordering::SeqCst);
    let failed = ask(&control, &Caller::Owner, "store/index", refresh);
    STANDIN_FAILS.store(false, Ordering::SeqCst);
    assert_eq!(made_names(&failed), ["made"], "{failed}");
    assert!(
        failed["result"]["sources"][0]["problem"]
            .as_str()
            .is_some_and(|why| why.contains("the stand-in is down")),
        "{failed}"
    );
}

#[test]
fn a_catalog_naming_a_provider_this_host_does_not_offer_says_so_and_lists_nothing() {
    let rig = Rig::new("catalog-no-provider");
    let control = rig.control(
        "[store.overlays.catalogs]\nboriken = false\n\n[store.overlays.catalogs.lost]\nprovider = \"nobody\"\n",
        false,
    );
    let listed = ask(
        &control,
        &Caller::Owner,
        "store/index",
        json!({"refresh": true}),
    );
    assert!(made_names(&listed).is_empty(), "{listed}");
    assert!(
        listed["result"]["sources"][0]["problem"]
            .as_str()
            .is_some_and(|why| why.contains("no catalog source")),
        "{listed}"
    );
}

#[test]
fn a_key_without_hosting_cannot_name_a_source_that_runs_something_but_may_write_a_plain_catalog() {
    let rig = Rig::new("catalog-guard");
    let control = rig.control("[store.overlays.catalogs]\nboriken = false\n", false);
    let writer = key("sha256:aa", &[Grant::ConfigWrite, Grant::ConfigRead]);
    let program = json!({"section": "store", "value": {"overlays": {"catalogs": {
        "boriken": false,
        "made": {"provider": "command", "command": ["gen"]},
    }}}});
    for method in ["config/set", "config/check"] {
        let refused = ask(&control, &writer, method, program.clone());
        assert_eq!(refused["error"]["code"], DENIED, "{method}: {refused}");
    }
    let plain = json!({"section": "store", "value": {"overlays": {"catalogs": {
        "boriken": false,
        "mine": "https://example.org/o.git@main#c.toml",
    }}}});
    let written = ask(&control, &writer, "config/set", plain);
    assert!(written["result"].is_object(), "{written}");

    let trusted = key(
        "sha256:bb",
        &[Grant::ConfigWrite, Grant::ConfigRead, Grant::Hosting],
    );
    let allowed = ask(&control, &trusted, "config/set", program);
    assert!(allowed["result"].is_object(), "{allowed}");
}
