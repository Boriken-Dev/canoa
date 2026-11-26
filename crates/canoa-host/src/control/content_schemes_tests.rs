//! `content/schemes`, and the generators `store/catalogs` lists, through the control channel.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::json;

use super::overlays_tests::{ask, dir, key};
use super::*;
use crate::store::programs::Programs;
use crate::store::shelf::Shelf;
use crate::store::sources::Sources;

struct Rig {
    tmp: PathBuf,
}

impl Rig {
    fn new(name: &str) -> Self {
        Self { tmp: dir(name) }
    }

    fn control(&self) -> Control {
        let mut sources = Sources::built_in();
        sources
            .add(Some("demo_ext"), crate::content::tests::demo_registry())
            .expect("added");
        let programs = Programs::new(
            Shelf::unchecked(self.tmp.join("programs")),
            crate::store::registry::built_in().expect("the built-in stores"),
        );
        Control::open(
            None,
            DEFAULT_MAX_RECORDS,
            DEFAULT_MAX_RECORD_BYTES,
            Vec::new(),
        )
        .expect("opens")
        .with_stores(Stores::new(Arc::new(programs), Vec::new()).with_sources(sources))
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.tmp);
    }
}

#[test]
fn the_schemes_the_host_reads_the_generators_and_what_was_refused_are_listed() {
    let rig = Rig::new("content-schemes");
    let control = rig.control();
    let answer = ask(&control, &Caller::Owner, "content/schemes", json!({}));
    let result = &answer["result"];
    let schemes = result["schemes"].as_array().expect("schemes");
    let row = |name: &str| {
        schemes
            .iter()
            .find(|row| row["scheme"] == name)
            .unwrap_or_else(|| panic!("no scheme {name} in {answer}"))
    };
    assert_eq!(row("file")["provider"], "file");
    assert_eq!(row("git+https")["lists"], true);
    assert_eq!(row("git+https")["resolves"], true);
    assert_eq!(row("https")["lists"], false);
    assert_eq!(row("demo")["extension"], "demo_ext");
    assert_eq!(row("demo")["state"], "on");
    let generators = result["generators"].as_array().expect("generators");
    let ids: Vec<&str> = generators
        .iter()
        .filter_map(|each| each["id"].as_str())
        .collect();
    assert!(
        ids.contains(&"command") && ids.contains(&"one_model"),
        "{ids:?}"
    );
    let command = generators
        .iter()
        .find(|each| each["id"] == "command")
        .expect("command");
    assert_eq!(command["source"], "absent");
    assert!(command["schema"].is_object(), "{command}");
    assert!(result["refused"].as_array().expect("refused").is_empty());

    // What `store/catalogs` lists for a type is the generators that serve it.
    let types = ask(
        &control,
        &Caller::Owner,
        "store/catalogs",
        json!({"type": "overlays"}),
    );
    let served: Vec<&str> = types["result"]["types"][0]["generators"]
        .as_array()
        .expect("generators")
        .iter()
        .filter_map(|each| each["id"].as_str())
        .collect();
    assert!(served.contains(&"command"), "{served:?}");
    assert!(
        !served.contains(&"one_model"),
        "one_model serves only whisper: {served:?}"
    );
}

#[test]
fn the_schemes_are_read_with_the_config_grant() {
    let rig = Rig::new("content-schemes-grant");
    let control = rig.control();
    let reader = key("sha256:aa", &[Grant::ConfigRead]);
    assert!(ask(&control, &reader, "content/schemes", json!({}))["result"].is_object());
    let none = key("sha256:bb", &[Grant::OverlayRead]);
    assert_eq!(
        ask(&control, &none, "content/schemes", json!({}))["error"]["code"],
        DENIED
    );
}
