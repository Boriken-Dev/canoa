//! A store shipped as a library: one library file offers both of its stores, `canoa-store-check` loads it and holds it to the
//! suite on a fixture file, and a library whose store breaks a rule fails the same command.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a test names what it uses from std"
)]

mod support;

use std::fs;
use std::process::Command;

use canoa::store::ProgramStore;
use coqui_acp_api::guatiao::library::{ImportSearch, Registry};
use support::example;

fn fixture(dir: &std::path::Path) -> std::path::PathBuf {
    let origin = "https://echo.example.test";
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let url = |version: &str| format!("{origin}/{version}/echo-{os}-{arch}");
    let mut world = serde_json::Map::new();
    world.insert(
        format!("{origin}/latest"),
        serde_json::json!({"text": "2.0.0"}),
    );
    for version in ["1.0.0", "2.0.0"] {
        let line = format!("echo {version}");
        world.insert(
            format!("{}.sha256", url(version)),
            serde_json::json!({"text": format!("{{{{sha256:{}}}}}", url(version))}),
        );
        world.insert(
            url(version),
            serde_json::json!({"standin": {"line": line, "exit": 0, "auto_update_var": "ECHO_NO_UPDATE"}}),
        );
    }
    let document = serde_json::json!({
        "old": "1.0.0", "new": "2.0.0", "auto_update_var": "ECHO_NO_UPDATE", "world": world,
    });
    let path = dir.join("fixture.json");
    fs::write(&path, document.to_string()).expect("fixture");
    path
}

#[test]
fn one_library_offers_both_stores_and_loads_once() {
    let library = example("echo_store", true);
    let mut registry = Registry::new("test", "0");
    registry.set_import_search(ImportSearch::Library);
    registry.load_file(&library).expect("the library loads");
    for id in ["echo_store", "echo_archive_store"] {
        let offer = registry
            .offer::<dyn ProgramStore>(id)
            .unwrap_or_else(|| panic!("{id} is offered"))
            .expect("a program_store");
        let store = offer.default_instance().expect("an instance");
        assert!(!store.describe().expect("describe").program.is_empty());
    }
}

#[test]
fn canoa_store_check_passes_a_library_that_keeps_every_rule() {
    let library = example("echo_store", true);
    let standin = example("version_standin", false);
    let checker = example("../canoa-store-check", false);
    let dir = std::env::temp_dir().join(format!("canoa-store-check-{}", std::process::id()));
    fs::create_dir_all(&dir).expect("dir");
    let fixture = fixture(&dir);
    let output = Command::new(&checker)
        .args([
            "--library",
            &library.to_string_lossy(),
            "--provider",
            "echo_store",
            "--fixture",
            &fixture.to_string_lossy(),
            "--standin",
            &standin.to_string_lossy(),
        ])
        .output()
        .expect("the checker runs");
    let text = String::from_utf8_lossy(&output.stdout).into_owned()
        + &String::from_utf8_lossy(&output.stderr);
    let _ = fs::remove_dir_all(&dir);
    assert!(output.status.success(), "{text}");
    assert!(text.contains("passes every check"), "{text}");
}

#[test]
fn canoa_store_check_exits_two_for_a_library_that_is_not_there_and_one_for_a_rule_broken() {
    let standin = example("version_standin", false);
    let checker = example("../canoa-store-check", false);
    let dir = std::env::temp_dir().join(format!("canoa-store-check-bad-{}", std::process::id()));
    fs::create_dir_all(&dir).expect("dir");
    let fixture = fixture(&dir);
    let missing = Command::new(&checker)
        .args([
            "--library",
            &dir.join("none").to_string_lossy(),
            "--provider",
            "echo_store",
            "--fixture",
            &fixture.to_string_lossy(),
            "--standin",
            &standin.to_string_lossy(),
        ])
        .output()
        .expect("runs");
    assert_eq!(missing.status.code(), Some(2));
    // A fixture whose digest document names the wrong file breaks `plan.*`/`apply.*`: the world is wrong, so a rule is.
    let broken = fs::read_to_string(&fixture)
        .expect("fixture")
        .replace("{{sha256:", "{{sha512:");
    fs::write(&fixture, broken).expect("write");
    let library = example("echo_store", true);
    let failing = Command::new(&checker)
        .args([
            "--library",
            &library.to_string_lossy(),
            "--provider",
            "echo_store",
            "--fixture",
            &fixture.to_string_lossy(),
            "--standin",
            &standin.to_string_lossy(),
        ])
        .output()
        .expect("runs");
    let _ = fs::remove_dir_all(&dir);
    assert_eq!(
        failing.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&failing.stdout)
    );
}
