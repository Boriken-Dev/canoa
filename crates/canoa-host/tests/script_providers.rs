//! Providers written with the Canoa SDK in Python and in JavaScript, loaded by the host from the examples under `examples/providers`
//! through Guatiao's sources: an `agent_vendor` the host lists as an agent, a `store_extension` that adds a Store type, and one
//! library that serves a `profile_item`, a `settings_section` and a `panel_tab` the host lists. Each
//! language builds its source from Cargo's checkout of Guatiao at the workspace's revision: Python against the interpreter
//! `CANOA_TEST_PYTHON` names (a Python 3.9 or later with its shared library), JavaScript with a C compiler for QuickJS
//! (`CANOA_TEST_JS` set to anything). Without a variable its test is ignored, and a run counts it as not run.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

#[cfg(not(canoa_test_python))]
#[test]
#[ignore = "NOT RUN: set CANOA_TEST_PYTHON to a Python 3.9 or later that has its shared library"]
fn python_providers_need_a_python() {
    panic!("CANOA_TEST_PYTHON names no Python");
}

#[cfg(not(canoa_test_js))]
#[test]
#[ignore = "NOT RUN: set CANOA_TEST_JS to anything, with a C compiler on the machine, to build the QuickJS source"]
fn javascript_providers_need_a_compiler() {
    panic!("CANOA_TEST_JS is not set");
}

#[cfg(any(canoa_test_python, canoa_test_js))]
mod linked;
#[cfg(any(canoa_test_python, canoa_test_js))]
mod support;

#[cfg(any(canoa_test_python, canoa_test_js))]
#[allow(dead_code)]
mod with_source {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use serde_json::{Value, json};

    use crate::linked::{Hosted, ask};
    use crate::support::{example, quoted};

    /// The root of the Cargo checkout of the package `name`, where its workspace is.
    pub fn checkout(name: &str) -> PathBuf {
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml");
        let output = Command::new(cargo)
            .args(["metadata", "--format-version", "1", "--manifest-path"])
            .arg(&manifest)
            .output()
            .expect("cargo metadata runs");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let metadata: Value = serde_json::from_slice(&output.stdout).expect("JSON");
        let package = metadata["packages"]
            .as_array()
            .expect("packages")
            .iter()
            .find(|package| package["name"] == name)
            .unwrap_or_else(|| panic!("{name} is in the graph"));
        // <checkout>/crates/<name>/Cargo.toml
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

    /// Builds the source crate `krate` of Guatiao from a copy of its checkout, since a build writes beside its manifest. The
    /// copy is named after the checkout's directory, which Cargo names by revision, and shared by every test of the run.
    pub fn build_source(krate: &str, library: &str, environment: &[(&str, String)]) -> PathBuf {
        let guatiao = checkout("guatiao");
        let revision = guatiao
            .file_name()
            .expect("Cargo's checkout is a directory of its own");
        let scratch = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(krate)
            .join(revision);
        let copy = scratch.join("guatiao");
        if !copy.join("Cargo.toml").is_file() {
            copy_tree(&guatiao.join("crates"), &copy.join("crates"));
            // The source's tests name the example kinds of Guatiao by path.
            copy_tree(&guatiao.join("examples"), &copy.join("examples"));
            std::fs::copy(guatiao.join("Cargo.toml"), copy.join("Cargo.toml")).expect("copied");
        }
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let mut command = Command::new(cargo);
        command
            .arg("build")
            .arg("--manifest-path")
            .arg(copy.join(format!("crates/{krate}/Cargo.toml")))
            .arg("--target-dir")
            .arg(scratch.join("target"));
        for (name, value) in environment {
            command.env(name, value);
        }
        let built = command.output().expect("cargo runs");
        assert!(
            built.status.success(),
            "the {krate} source did not build:\n{}",
            String::from_utf8_lossy(&built.stderr)
        );
        scratch.join("target/debug").join(format!(
            "{}{library}{}",
            std::env::consts::DLL_PREFIX,
            std::env::consts::DLL_SUFFIX
        ))
    }

    /// The SDK's folder for a language, and the folder of an example's library by its kind.
    pub fn repository() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    pub fn list(paths: &[PathBuf]) -> String {
        let quoted: Vec<String> = paths
            .iter()
            .map(|path| quoted(&path.to_string_lossy()))
            .collect();
        format!("[{}]", quoted.join(", "))
    }

    /// canoa.cfg with one store extension entry, whose source also holds the vendor example's library: the host lists a source's
    /// agents from every origin it loads, so one entry shows both kinds.
    pub fn config(source: &Path, provider: &str, settings: &str) -> String {
        // The host serves at least one entry; this agent is the one other tests use.
        let agent = example("stdio_agent", false);
        format!(
            "[host.agents.Probe]\nprogram = {}\nargs = [\"--scripted\"]\n\n\
             [host.extensions.examples]\nsource = {}\nprovider = {}\n\
             [host.extensions.examples.source_settings]\n{settings}",
            quoted(&agent.to_string_lossy()),
            quoted(&source.to_string_lossy()),
            quoted(provider),
        )
    }

    /// What the host says of the examples: the Store types its extension adds, and the agent row its vendor describes.
    pub fn check(config: String) {
        let hosted = Hosted::start_with_setup(|_| config, &[]);
        let link = hosted.local.open("@control").expect("a link");
        let (types, _) = ask(&link, 1, "store/extensions", json!({})).expect("the types");
        // The host's own extension lists its types too (Secret providers): this one is found by what it adds.
        let notes = types["types"]
            .as_array()
            .expect("types")
            .iter()
            .find(|each| each["id"] == "notes")
            .unwrap_or_else(|| panic!("the extension's type is listed: {types}"));
        assert_eq!(notes["extension_title"], "Notes");
        assert_eq!(notes["publisher"], "Example publisher");
        assert_eq!(notes["icon"], "extension");
        let (listed, _) = ask(&link, 2, "providers/list", json!({})).expect("the agents");
        let quill = listed["providers"]
            .as_array()
            .expect("rows")
            .iter()
            .find(|row| row["id"] == "quill")
            .unwrap_or_else(|| panic!("the vendor's agent is listed: {listed}"));
        assert_eq!(quill["title"], "Quill");
        assert_eq!(quill["program"], "Quill Agent");
        assert_eq!(quill["vendor"], "quill-labs");
        assert_eq!(quill["aos"], "quill");
        assert_eq!(quill["model_variable"], "QUILL_MODEL");
        assert_eq!(
            quill["standards"],
            json!([{"id": "quill-key", "title": "Quill API key"}])
        );
        assert_eq!(quill["names"], json!(["quill-agent"]));
        assert_eq!(
            quill["modes"],
            json!([{"id": "ask", "title": "Ask"}, {"id": "code", "title": "Code"}])
        );
        assert!(
            quill["mark"]
                .as_str()
                .is_some_and(|mark| mark.starts_with("<svg")),
            "{quill}"
        );
        assert_eq!(quill["origin"], "examples");
        // One library serves a profile item, a page of Settings and a tab of the panel, which the host lists as any extension's.
        let (items, _) = ask(&link, 3, "profile_items/list", json!({})).expect("the items");
        let region = items["items"]
            .as_array()
            .expect("items")
            .iter()
            .find(|row| row["id"] == "region")
            .unwrap_or_else(|| panic!("the item is listed: {items}"));
        assert_eq!(region["origin"], "examples");
        assert_eq!(region["harnesses"], json!(["claude-code"]));
        assert_eq!(region["schema"]["required"], json!(["name"]));
        assert_eq!(region["form"]["fields"]["name"]["placeholder"], "eu-west");
        let (sections, _) = ask(&link, 4, "settings/sections", json!({})).expect("the sections");
        let page = sections["sections"]
            .as_array()
            .expect("sections")
            .iter()
            .find(|row| row["id"] == "region")
            .unwrap_or_else(|| panic!("the section is listed: {sections}"));
        assert_eq!(page["title"], "Region");
        assert_eq!(page["group"], "Team");
        assert_eq!(page["stored"], "config");
        let (tabs, _) = ask(&link, 5, "panel_tabs/list", json!({})).expect("the tabs");
        let builds = tabs["tabs"]
            .as_array()
            .expect("tabs")
            .iter()
            .find(|row| row["id"] == "builds")
            .unwrap_or_else(|| panic!("the tab is listed: {tabs}"));
        assert_eq!(builds["shows"], "log");
        let (tail, _) = ask(
            &link,
            6,
            "panel_tabs/tail",
            json!({"id": "builds", "lines": 1}),
        )
        .expect("the tail");
        assert_eq!(tail["text"], "build 42 failed\n");
        drop(link);
        hosted.stop();
    }
}

#[cfg(canoa_test_python)]
mod python {
    use std::path::PathBuf;
    use std::process::Command;

    use super::with_source::{build_source, check, checkout, config, list, repository};

    const PROBE: &str = "import sys; print(sys.executable); print(sys.base_prefix)";

    #[test]
    fn a_vendor_and_a_store_extension_written_with_the_python_sdk_are_offered_by_the_host() {
        let named = env!("CANOA_TEST_PYTHON");
        let output = Command::new(named)
            .args(["-I", "-c", PROBE])
            .output()
            .unwrap_or_else(|error| panic!("CANOA_TEST_PYTHON={named} does not run: {error}"));
        assert!(output.status.success(), "CANOA_TEST_PYTHON={named} failed");
        let text = String::from_utf8(output.stdout).expect("UTF-8");
        let mut lines = text.lines();
        let executable = lines.next().expect("an executable").to_owned();
        let prefix = PathBuf::from(lines.next().expect("a prefix"));
        let source = build_source(
            "guatiao-source-py",
            "guatiao_source_py",
            &[("PYO3_PYTHON", executable)],
        );
        let examples = repository().join("examples/providers/python");
        let settings = format!(
            "dirs = {}\npaths = {}\nimport_dirs = {}\n",
            list(&[
                examples.join("vendor"),
                examples.join("extension"),
                examples.join("region")
            ]),
            list(&[
                checkout("guatiao").join("bindings/python/guatiao/src"),
                repository().join("bindings/python/canoa-sdk/src"),
            ]),
            list(&[prefix]),
        );
        check(config(&source, "notes_extension_notes", &settings));
    }
}

#[cfg(canoa_test_js)]
mod javascript {
    use super::with_source::{build_source, check, config, list, repository};

    #[test]
    fn a_vendor_and_a_store_extension_written_with_the_javascript_sdk_are_offered_by_the_host() {
        let source = build_source("guatiao-source-js", "guatiao_source_js", &[]);
        let examples = repository().join("examples/providers/js");
        let settings = format!(
            "dirs = {}\n",
            list(&[
                examples.join("vendor"),
                examples.join("extension"),
                examples.join("region")
            ]),
        );
        check(config(&source, "notes_extension_notes", &settings));
    }
}
