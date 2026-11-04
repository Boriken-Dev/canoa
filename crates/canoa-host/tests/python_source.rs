//! A provider written in Python, coqui-acp's `examples/provider-python` echo, served by the built `canoa-host` from one
//! Canoa.cfg entry through Guatiao's source of Python libraries. The source is a `cdylib` built against a Python, so this
//! test builds it from Cargo's checkout of Guatiao at the workspace's revision, with the Python `CANOA_TEST_PYTHON` names.
//! Without that variable the test is ignored, and a run counts it as not run.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

#[cfg(not(canoa_test_python))]
#[test]
#[ignore = "NOT RUN: set CANOA_TEST_PYTHON to a Python 3.9 or later that has its shared library"]
fn a_python_source_entry_needs_a_python() {
    panic!("CANOA_TEST_PYTHON names no Python");
}

#[cfg(canoa_test_python)]
mod support;

#[cfg(canoa_test_python)]
mod with_python {
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use coqui_agentic_peer::auth::Identity;
    use serde_json::{Value, json};

    use crate::support::{Host, Scratch, cwd, log_holding, quoted, with_link};

    /// Where the interpreter is and what a host needs to find its shared library.
    struct Python {
        executable: String,
        prefix: PathBuf,
        library: PathBuf,
    }

    const PROBE: &str = r#"
import os, sys, sysconfig
if os.name == "nt":
    lib = os.path.join(sys.base_prefix, "python3.dll")
else:
    name = sysconfig.get_config_var("LDLIBRARY") or ""
    lib = os.path.join(sysconfig.get_config_var("LIBDIR") or "", name)
    if not os.path.isfile(lib):
        lib = os.path.join(sys.base_prefix, "lib", name)
print(sys.executable)
print(sys.base_prefix)
print(lib)
print(sys.version_info[0] * 100 + sys.version_info[1])
"#;

    fn python() -> Python {
        let named = env!("CANOA_TEST_PYTHON");
        let output = Command::new(named)
            .args(["-I", "-c", PROBE])
            .output()
            .unwrap_or_else(|error| panic!("CANOA_TEST_PYTHON={named} does not run: {error}"));
        assert!(output.status.success(), "CANOA_TEST_PYTHON={named} failed");
        let text = String::from_utf8(output.stdout).expect("UTF-8");
        let lines: Vec<&str> = text.lines().collect();
        let version: u32 = lines[3].parse().expect("a version");
        assert!(
            version >= 309,
            "CANOA_TEST_PYTHON={named} is older than 3.9"
        );
        let python = Python {
            executable: lines[0].to_owned(),
            prefix: PathBuf::from(lines[1]),
            library: PathBuf::from(lines[2]),
        };
        assert!(
            python.library.is_file(),
            "CANOA_TEST_PYTHON={named} has no shared library at {}, which embedding needs",
            python.library.display()
        );
        python
    }

    /// The checkout Cargo made of the package `name` from git, as its workspace root.
    fn checkout(name: &str) -> PathBuf {
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

    /// Builds guatiao-source-py from a copy of Guatiao's checkout, since a build writes beside its manifest. The copy is
    /// named after the checkout's directory, which Cargo names by revision, so another pin builds from a copy of its own.
    fn build_source(python: &Python, guatiao: &Path) -> PathBuf {
        let revision = guatiao
            .file_name()
            .expect("Cargo's checkout is a directory of its own");
        let scratch = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join("guatiao-source-py")
            .join(revision);
        let copy = scratch.join("guatiao");
        if !copy.join("Cargo.toml").is_file() {
            copy_tree(&guatiao.join("crates"), &copy.join("crates"));
            std::fs::copy(guatiao.join("Cargo.toml"), copy.join("Cargo.toml")).expect("copied");
        }
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let mut command = Command::new(cargo);
        command
            .arg("build")
            .arg("--manifest-path")
            .arg(copy.join("crates/guatiao-source-py/Cargo.toml"))
            .arg("--target-dir")
            .arg(scratch.join("target"))
            .env("PYO3_PYTHON", &python.executable);
        if !cfg!(windows) {
            // A Python unpacked anywhere may report the prefix it was built for, not its own.
            let lib = python.library.parent().expect("a directory");
            command.env("RUSTFLAGS", format!("-L native={}", lib.display()));
        }
        let built = command.output().expect("cargo runs");
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

    /// The environment in which the host's provider process finds the interpreter.
    fn environment(python: &Python) -> Vec<(&'static str, OsString)> {
        let (name, dir) = if cfg!(windows) {
            ("PATH", python.prefix.clone())
        } else if cfg!(target_os = "macos") {
            (
                "DYLD_FALLBACK_LIBRARY_PATH",
                python.library.parent().expect("a directory").to_path_buf(),
            )
        } else {
            (
                "LD_LIBRARY_PATH",
                python.library.parent().expect("a directory").to_path_buf(),
            )
        };
        let mut dirs = vec![dir];
        // An empty entry would name the working directory to the loader.
        dirs.extend(
            std::env::split_paths(&std::env::var_os(name).unwrap_or_default())
                .filter(|dir| !dir.as_os_str().is_empty()),
        );
        vec![
            (name, std::env::join_paths(dirs).expect("paths join")),
            ("PYTHONDONTWRITEBYTECODE", OsString::from("1")),
        ]
    }

    /// A Canoa.cfg entry `python` serving `provider` from the manifests in `dirs`, with the source built against `python`.
    fn source_entry(python: &Python, provider: &str, dirs: &[PathBuf]) -> String {
        let guatiao = checkout("guatiao");
        let coqui_acp = checkout("coqui-acp-provider");
        let source = build_source(python, &guatiao);
        let list = |paths: &[PathBuf]| {
            let quoted: Vec<String> = paths
                .iter()
                .map(|path| quoted(&path.to_string_lossy()))
                .collect();
            format!("[{}]", quoted.join(", "))
        };
        format!(
            "[host.providers.python]\nsource = {}\nprovider = {}\n\
             [host.providers.python.source_settings]\ndirs = {}\npaths = {}\n",
            quoted(&source.to_string_lossy()),
            quoted(provider),
            list(dirs),
            list(&[
                guatiao.join("bindings/python/guatiao/src"),
                coqui_acp.join("crates/coqui-acp-provider/python"),
            ]),
        )
    }

    /// The text of each prompt's turn, in one session.
    fn turns(host: &Host, client: &Identity, prompts: &[&str]) -> Vec<String> {
        let dir = cwd(&host.scratch);
        with_link(host.url("python"), client, host.peer, |rpc| {
            rpc.initialize().expect("initialize is answered");
            let session = rpc
                .request("session/new", json!({"cwd": dir, "mcpServers": []}))
                .expect("a session opens");
            let id = session["sessionId"].as_str().expect("an id").to_owned();
            prompts
                .iter()
                .map(|prompt| rpc.prompt(&id, prompt).expect("the turn ends"))
                .collect()
        })
        .expect("a paired key is served")
    }

    #[test]
    fn a_python_provider_answers_through_one_source_entry() {
        let python = python();
        let coqui_acp = checkout("coqui-acp-provider");
        let config = source_entry(
            &python,
            "acp_echo_echo",
            &[coqui_acp.join("examples/provider-python/echo")],
        );
        let host = Host::start_in(Scratch::new(), &config, 1, &environment(&python));
        let client = crate::support::key();
        host.pair(&client);
        assert_eq!(
            turns(&host, &client, &["hello", "again"]),
            ["echo: hello", "echo: again"]
        );
    }

    #[test]
    fn what_a_python_provider_prints_goes_to_the_hosts_log_bounded_and_never_to_the_client() {
        let python = python();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/python/printing");
        let config = source_entry(&python, "printing_echo_echo", &[fixture]);
        let (host, log) = Host::start_logged(Scratch::new(), &config, 1, &environment(&python));
        let client = crate::support::key();
        host.pair(&client);
        assert_eq!(
            turns(&host, &client, &["hello", "flood", "again"]),
            ["printing: hello", "printing: flood", "printing: again"]
        );
        let printed = [
            "printed while importing",
            "printed while starting",
            "printed during a prompt",
            "forged by a print",
            "written to file descriptor 1",
            "printed to standard error",
            "the rest is dropped",
        ];
        let written = log_holding(&log, &printed);
        for printed in printed {
            assert!(written.contains(printed), "{printed} is in the log");
        }
        assert!(
            written.len() < 2 * 1024 * 1024,
            "the host kept {} bytes of a 4 MiB flood",
            written.len()
        );
    }
}
