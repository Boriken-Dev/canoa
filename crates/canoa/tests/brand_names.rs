//! The rules a brand file's names obey, and that a build carries the names of the brand file it was made from.

#[allow(dead_code, reason = "the build script reads what these tests read")]
#[path = "../build/names.rs"]
mod names;

use std::path::{Path, PathBuf};

use names::Names;

fn fixture(key: &str, value: serde_json::Value) -> String {
    let mut brand = serde_json::json!({
        "name": "Pirogue",
        "short_name": "Pirogue",
        "executable": "PirogueUI",
        "id": "org.example.pirogue",
        "env_prefix": "PIROGUE",
        "icon": "mark.svg",
        "accent": "#C0392B",
    });
    brand[key] = value;
    brand.to_string()
}

#[test]
fn a_brand_that_names_nothing_else_takes_its_names_from_its_name() {
    let brand = Names::read(&fixture("name", "Pirogue".into())).expect("a brand");
    assert_eq!(brand.program, "pirogue");
    assert_eq!(brand.config_file, "pirogue.cfg");
    assert_eq!(brand.folder, "Pirogue");
    assert_eq!(brand.home_dir, ".pirogue");
}

#[test]
fn the_example_brand_names_each_place_itself() {
    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/brands/agentui/brand.json");
    let text = std::fs::read_to_string(&file).expect("the example brand");
    let brand = Names::read(&text).expect("a brand");
    assert_eq!(
        (
            brand.program.as_str(),
            brand.config_file.as_str(),
            brand.folder.as_str(),
            brand.home_dir.as_str(),
            brand.env_prefix.as_str(),
        ),
        ("agents", "agents.cfg", "agents", ".agentui", "AGENTUI")
    );
    assert!(brand.icon_path(&file).is_file());
}

#[test]
fn a_name_that_is_not_one_path_component_is_refused() {
    for (key, bad) in [
        ("config_file", ""),
        ("config_file", "a/b.cfg"),
        ("config_file", "a\\b.cfg"),
        ("config_file", "..cfg"),
        ("config_file", "x:y.cfg"),
        ("config_file", "nul.cfg"),
        ("folder", "agents."),
        ("folder", " agents"),
        ("folder", "CON"),
        ("folder", "a*b"),
        ("home_dir", ".a\nb"),
        ("home_dir", "../x"),
        ("program", "Agents"),
        ("program", "1agents"),
        ("program", "has space"),
        ("env_prefix", "lower"),
        ("id", "NoDots"),
        ("executable", "has space"),
        ("accent", "red"),
        ("name", "say \"hi\""),
        ("extra", "x"),
    ] {
        let error = Names::read(&fixture(key, bad.into())).expect_err(bad);
        assert!(error.contains(key), "{key} {bad:?}: {error}");
    }
}

#[test]
fn a_name_that_is_one_path_component_is_taken() {
    for (key, good) in [
        ("config_file", "agents.cfg"),
        ("config_file", "agents.json"),
        ("folder", "agents"),
        ("folder", "Agent UI"),
        ("home_dir", ".agentui"),
        ("home_dir", "agentui"),
        ("program", "agent-ui"),
    ] {
        Names::read(&fixture(key, good.into()))
            .unwrap_or_else(|error| panic!("{key} {good}: {error}"));
    }
}

/// With `BRAND_FILE` in the environment a build carries that file's names, and the crate's copy of the app's brand without it.
#[test]
fn a_build_carries_the_names_of_the_brand_file_it_was_made_from() {
    let file = std::env::var_os("BRAND_FILE")
        .filter(|path| !path.is_empty())
        .map_or_else(
            || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("brand/brand.json"),
            PathBuf::from,
        );
    let brand =
        Names::read(&std::fs::read_to_string(&file).expect("the brand file")).expect("a brand");
    assert_eq!(canoa::brand::NAME, brand.name);
    assert_eq!(canoa::brand::PROGRAM, brand.program);
    assert_eq!(canoa::brand::CONFIG_FILE, brand.config_file);
    assert_eq!(canoa::brand::FOLDER, brand.folder);
    assert_eq!(canoa::brand::HOME_DIR, brand.home_dir);
    assert_eq!(canoa::brand::ID, brand.id);
    assert_eq!(canoa::brand::ENV_PREFIX, brand.env_prefix);
    assert_eq!(canoa::brand::CONFIG, format!("{}_CONFIG", brand.env_prefix));
}
