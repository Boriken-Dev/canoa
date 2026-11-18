//! The Bitwarden provider, a Python library, loaded through Guatiao's source of Python libraries into the host's catalog and asked
//! through the vault, with a stand-in `bw` as the program it runs. It proves the `secret_provider` kind is one a script can serve:
//! its description, the values it answers, the flags that say what it needs, and the errors it raises.
//!
//! The source is a `cdylib` built against a Python, so this builds it from Cargo's checkout of Guatiao at the workspace's revision,
//! with the Python `CANOA_TEST_PYTHON` names; without that variable the test is not built, and a run counts it as not run.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};

use super::catalog::Catalog;
use super::vault::Vault;

const PROBE: &str = r"
import os, sys
print(sys.executable)
print(sys.base_prefix)
";

fn python() -> (String, PathBuf) {
    let named = env!("CANOA_TEST_PYTHON");
    let output = Command::new(named)
        .args(["-I", "-c", PROBE])
        .output()
        .unwrap_or_else(|error| panic!("CANOA_TEST_PYTHON={named} does not run: {error}"));
    assert!(output.status.success(), "CANOA_TEST_PYTHON={named} failed");
    let text = String::from_utf8(output.stdout).expect("UTF-8");
    let mut lines = text.lines();
    (
        lines.next().expect("an executable").to_owned(),
        PathBuf::from(lines.next().expect("a prefix")),
    )
}

fn checkout(name: &str) -> PathBuf {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml");
    let output = Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--manifest-path"])
        .arg(&manifest)
        .output()
        .expect("cargo metadata runs");
    assert!(output.status.success());
    let metadata: Value = serde_json::from_slice(&output.stdout).expect("JSON");
    let package = metadata["packages"]
        .as_array()
        .expect("packages")
        .iter()
        .find(|package| package["name"] == name)
        .unwrap_or_else(|| panic!("{name} is in the graph"));
    PathBuf::from(package["manifest_path"].as_str().expect("a path"))
        .ancestors()
        .nth(3)
        .expect("a crate under <checkout>/crates")
        .to_path_buf()
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("a directory");
    for entry in std::fs::read_dir(from).expect("a readable checkout") {
        let entry = entry.expect("a readable entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("a file type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("a file copies");
        }
    }
}

/// Builds guatiao-source-py from a copy of Guatiao's checkout, since a build writes beside its manifest.
fn build_source(python: &str, guatiao: &Path) -> PathBuf {
    let revision = guatiao.file_name().expect("a directory of its own");
    let scratch = std::env::temp_dir()
        .join("canoa-secret-script-tests")
        .join(revision);
    let copy = scratch.join("guatiao");
    if !copy.join("Cargo.toml").is_file() {
        copy_tree(&guatiao.join("crates"), &copy.join("crates"));
        copy_tree(&guatiao.join("examples"), &copy.join("examples"));
        std::fs::copy(guatiao.join("Cargo.toml"), copy.join("Cargo.toml")).expect("copied");
    }
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let built = Command::new(cargo)
        .arg("build")
        .arg("--manifest-path")
        .arg(copy.join("crates/guatiao-source-py/Cargo.toml"))
        .arg("--target-dir")
        .arg(scratch.join("target"))
        .env("PYO3_PYTHON", python)
        .output()
        .expect("cargo runs");
    assert!(
        built.status.success(),
        "the Python source did not build:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
    scratch.join("target/debug").join(format!(
        "{}guatiao_source_py{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ))
}

fn extension() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("extensions/bitwarden")
}

/// canoa.cfg with the script as `[host.extensions.bitwarden]`, as a person adds one: the source, the folder of its library and where it
/// finds Guatiao's Python binding. The provider it names is not a store extension, since it adds no type to the Store.
fn script_config(python: &str, prefix: &Path) -> crate::config::Config {
    let guatiao = checkout("guatiao");
    let source = build_source(python, &guatiao);
    let quoted = |path: &Path| format!("{:?}", path.to_string_lossy().replace(r"\", "/"));
    let text = format!(
        "[host.extensions.bitwarden]
source = {}
provider = \"bitwarden\"
import_search = \"path\"
         [host.extensions.bitwarden.source_settings]
dirs = [{}]
paths = [{}]
import_dirs = [{}]
",
        quoted(&source),
        quoted(&extension()),
        quoted(&guatiao.join("bindings/python/guatiao/src")),
        quoted(prefix),
    );
    crate::config::Config::from_str_in(&text, &std::env::temp_dir()).expect("canoa.cfg parses")
}

/// The catalog of the providers the host offers with that extension loaded.
fn script_catalog(python: &str, prefix: &Path) -> Catalog {
    Catalog::of(&script_config(python, prefix))
}

const VALUE: &str = "gh-pw-zq9-made-up";

#[test]
fn a_script_is_a_secret_provider_the_host_lists_describes_and_asks_like_any_other() {
    let (python, prefix) = python();
    let catalog = script_catalog(&python, &prefix);
    let listed = catalog.list();
    let bitwarden = listed
        .iter()
        .find(|each| each.id == "bitwarden")
        .expect("the script's provider is offered beside the built-in ones");
    assert_eq!(
        bitwarden.origin, "bitwarden",
        "the origin is the extension that loads it"
    );
    // What the host reads of a script is what it reads of a Rust provider: a description with the flags, and a settings schema that
    // marks the session key as a secret so a form hides it.
    assert_eq!(bitwarden.info.title, "Bitwarden");
    assert!(bitwarden.info.can_list && !bitwarden.info.can_store && !bitwarden.info.needs_person);
    let schema = bitwarden.schema.as_ref().expect("a schema");
    let session = &schema["properties"]["session"];
    assert!(
        session["writeOnly"] == true || session["x-sensitive"] == true,
        "{session}"
    );
    assert_eq!(listed.iter().filter(|each| each.id == "form").count(), 1);

    let command = python.clone();
    let standin = extension().join("tests/bw_standin.py");
    let clock = std::sync::Arc::new(AtomicU64::new(1_000_000));
    let read = std::sync::Arc::clone(&clock);
    let vault = Vault::with_clock(
        catalog,
        std::sync::Arc::new(move || {
            Ok(Some(json!({
                "names": {"github": "bw", "ghost": "bw"},
                "providers": {"bw": {
                    "provider": "bitwarden", "keep": "close",
                    "command": command.clone(), "arguments": [standin.to_string_lossy()],
                    "session": {"from": "form", "name": "bw_session"}
                }}
            })))
        }),
        std::sync::Arc::new(move || read.load(Ordering::SeqCst)),
    );
    let recall = |session: &str, name: &str| {
        vault
            .recall(session, name)
            .map(|value| String::from_utf8(value.to_vec()).expect("text"))
    };
    // Locked: the key is not held, so the vault is not even opened and the person is asked for the secret itself.
    assert_eq!(recall("s1", "github"), None);
    // A wrong key is the provider's `GONE`, which the host reads as nothing held.
    vault
        .remember(None, "bw_session", b"not-the-key", "close")
        .unwrap();
    assert_eq!(recall("s1", "github"), None);
    // The right key opens it, and an item that is not there is no value.
    vault
        .remember(None, "bw_session", b"good-session-key-made-up", "close")
        .unwrap();
    assert_eq!(recall("s1", "github").as_deref(), Some(VALUE));
    assert_eq!(recall("s1", "ghost"), None);
    let rows = vault.rows();
    let row = rows
        .iter()
        .find(|row| row.name == "github")
        .expect("what the script gave is in the table");
    assert_eq!(row.provider, "bw");
    assert_eq!(
        row.expires_in, None,
        "a Bitwarden item has no end of its own"
    );
    // A setting the script does not declare is refused by the host before the script is asked.
    let mut surprise = serde_json::Map::new();
    surprise.insert("surprise".to_owned(), json!(1));
    assert!(
        script_catalog(&python, &prefix)
            .instance("bitwarden", &surprise)
            .is_err()
    );
}

/// The library's own tests, run by the test Python with Guatiao's Python binding: the provider against a stand-in `bw`, no value in a
/// failure, the session key in the environment and never on a command line.
#[test]
fn the_scripts_own_tests_pass() {
    let (python, _) = python();
    let guatiao = checkout("guatiao");
    let output = Command::new(&python)
        .args(["-m", "unittest", "discover", "-s", "tests"])
        .current_dir(extension())
        .env("PYTHONPATH", guatiao.join("bindings/python/guatiao/src"))
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env_remove("BW_SESSION")
        .output()
        .expect("python runs");
    let said = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "the script's tests failed:
{said}"
    );
    assert!(said.contains("OK"), "{said}");
}

/// An extension whose library offers no store type is no error and adds none: the Store is as it was.
#[test]
fn a_library_that_adds_no_store_type_is_loaded_for_its_other_kinds_and_leaves_the_store_alone() {
    use crate::control::Stores;
    use crate::store::programs::Programs;
    use crate::store::shelf::Shelf;

    let (python, prefix) = python();
    let config = script_config(&python, &prefix);
    let root = std::env::temp_dir().join(format!("canoa-secret-stores-{}", std::process::id()));
    let programs = Programs::new(
        Shelf::unchecked(&root),
        crate::store::registry::built_in().expect("the built-in stores"),
    );
    let plain = Stores::new(Arc::new(programs), Vec::new());
    let extended = plain.extended(&config.extensions);
    let types = extended
        .read("store/extensions", &json!({}), None)
        .expect("the types");
    assert_eq!(types["types"], json!([]), "{types}");
    let _ = std::fs::remove_dir_all(&root);
}
