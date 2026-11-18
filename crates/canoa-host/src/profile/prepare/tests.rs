//! Preparing an overlay against a real Python (`CANOA_TEST_PYTHON`, a program or a name on the `PATH`) and a folder of wheels
//! standing in for the package index, so nothing leaves this computer.

use std::io::Write as _;

use super::*;

/// The Python the tests install with, or none: a test without one says so and returns.
fn python() -> Option<PathBuf> {
    let named = std::env::var_os("CANOA_TEST_PYTHON")?;
    let path = PathBuf::from(&named);
    if path.is_absolute() {
        return Some(path);
    }
    let lookup = |name: &str| std::env::var(name).ok();
    Finder::new(&lookup, &[]).find(&named.to_string_lossy())
}

macro_rules! python_or_return {
    () => {
        match python() {
            Some(python) => python,
            None => {
                eprintln!(
                    "NOT RUN: set CANOA_TEST_PYTHON to a Python 3.9 or later with venv and pip"
                );
                return;
            }
        }
    };
}

/// A finder whose path holds only the folder of `python`.
fn finder_for(python: &Path) -> Finder {
    let folder = python
        .parent()
        .expect("a folder")
        .to_string_lossy()
        .into_owned();
    let lookup = move |name: &str| match name {
        "PATH" => Some(folder.clone()),
        other => std::env::var(other).ok(),
    };
    Finder::new(&lookup, &[])
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("canoa-prepare-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a folder");
        Self(dir)
    }

    fn overlays(&self) -> PathBuf {
        self.0.join("overlays")
    }

    fn write(&self, relative: &str, text: &str) {
        let path = self.0.join(relative);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("its folder");
        std::fs::write(path, text).expect("written");
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A wheel of a package `name` at `version` whose module holds `VALUE = "<value>"`, in `folder`: the index a pip run is pointed
/// at with `PIP_FIND_LINKS`.
fn wheel(folder: &Path, name: &str, version: &str, value: &str) {
    std::fs::create_dir_all(folder).expect("a folder");
    let file = folder.join(format!("{name}-{version}-py3-none-any.whl"));
    let mut zip = zip::ZipWriter::new(std::fs::File::create(file).expect("a file"));
    let options = zip::write::SimpleFileOptions::default();
    let dist = format!("{name}-{version}.dist-info");
    let members = [
        (
            format!("{name}/__init__.py"),
            format!("VALUE = \"{value}\"\n"),
        ),
        (
            format!("{dist}/METADATA"),
            format!("Metadata-Version: 2.1\nName: {name}\nVersion: {version}\n"),
        ),
        (
            format!("{dist}/WHEEL"),
            "Wheel-Version: 1.0\nGenerator: test\nRoot-Is-Purelib: true\nTag: py3-none-any\n"
                .to_owned(),
        ),
    ];
    let mut record = String::new();
    for (path, text) in &members {
        zip.start_file(path, options).expect("a member");
        zip.write_all(text.as_bytes()).expect("its bytes");
        record.push_str(&format!("{path},,\n"));
    }
    record.push_str(&format!("{dist}/RECORD,,\n"));
    zip.start_file(format!("{dist}/RECORD"), options)
        .expect("record");
    zip.write_all(record.as_bytes()).expect("its bytes");
    zip.finish().expect("closed");
}

fn index_env(index: &Path) -> Vec<(OsString, OsString)> {
    vec![
        ("PIP_NO_INDEX".into(), "1".into()),
        ("PIP_FIND_LINKS".into(), index.as_os_str().to_owned()),
        ("PIP_DISABLE_PIP_VERSION_CHECK".into(), "1".into()),
    ]
}

/// Installs `files` as the overlay `name` under `scratch` and gives its plan.
fn plan_for(scratch: &Scratch, name: &str, files: &[(&str, &str)]) -> Plan {
    for (path, text) in files {
        scratch.write(&format!("overlays/{name}/{path}"), text);
    }
    let root = scratch.overlays().join(name);
    let digest = aos::tree_digest(&StdFs, &root, &Settings::default())
        .expect("digest")
        .digest
        .to_string();
    Plan::of(name, &root, aos::current_os(), &digest).expect("a plan")
}

const SETUP: &str = r#"import os, sys
state = os.environ['AOS_OVERLAY_STATE']
with open(os.path.join(state, 'phases.log'), 'a') as log:
    log.write(sys.argv[1] + ' ' + os.environ['AOS_SETUP_PHASE'] + ' ' + os.path.basename(os.environ['AOS_OVERLAY_ROOT']) + '\n')
"#;

#[test]
fn a_plan_lists_what_will_run_before_anything_does() {
    let scratch = Scratch::new("lines");
    let plan = plan_for(
        &scratch,
        "needy",
        &[
            (
                "overlay.cfg",
                "name = \"needy\"\n[scripts]\ncheck = \"bin/check.py\"\n",
            ),
            ("lib/python/requirements.txt", "canoa-test-pkg==1.0\n"),
            ("lib/node/package.json", "{\"dependencies\": {}}\n"),
            ("setup.py", "pass\n"),
            ("bin/check.py", "pass\n"),
        ],
    );
    let lines = plan.lines();
    assert_eq!(lines.len(), 4, "{lines:?}");
    assert!(
        lines[0].starts_with("dependencies: lib/python/requirements.txt (pip install, not pinned"),
        "{}",
        lines[0]
    );
    assert!(lines[0].contains("fetches from the network"));
    assert!(lines[1].starts_with("dependencies: lib/node/package.json (npm install, not pinned"));
    assert!(
        lines[2]
            .starts_with("setup: setup.py under python (up to 300 s; says it needs the network)"),
        "{}",
        lines[2]
    );
    assert!(
        !scratch.0.join("overlays/.environments").exists(),
        "planning made nothing"
    );
    assert_eq!(
        lines[3],
        "scripts.check: a command on the agent's PATH that runs bin/check.py under python when it is called; nothing runs now"
    );
    let nothing = plan_for(&scratch, "plain", &[("overlay.cfg", "name = \"plain\"\n")]);
    assert!(nothing.lines().is_empty());
}

#[test]
fn dependencies_install_into_an_environment_of_the_overlays_own_and_setup_runs_in_its_phases() {
    let python = python_or_return!();
    let scratch = Scratch::new("deps");
    let index = scratch.0.join("index");
    wheel(&index, "canoa_test_pkg", "1.0", "from the first wheel");
    let plan = plan_for(
        &scratch,
        "needy",
        &[
            ("overlay.cfg", "name = \"needy\"\n"),
            ("lib/python/requirements.txt", "canoa-test-pkg==1.0\n"),
            ("setup.py", SETUP),
        ],
    );
    let home = Home::new(scratch.overlays());
    let finder = finder_for(&python);
    let env = index_env(&index);
    assert_eq!(home.record("needy"), None);
    assert!(home.packages("needy").is_empty());

    let first = run(&plan, Change::Add, &home, &finder, &env).expect("prepared");
    assert_eq!(first.phase, Some(Phase::Install));
    assert_eq!(
        first.steps,
        ["empty-state", "deps:python", "setup", "record"]
    );
    let record = home.record("needy").expect("recorded");
    assert_eq!(
        (record.digest.as_str(), record.phase),
        (plan.digest.as_str(), Phase::Install)
    );
    let packages = PathBuf::from(&home.packages("needy")["python"]);
    assert!(
        packages
            .join("canoa_test_pkg")
            .join("__init__.py")
            .is_file(),
        "{packages:?}"
    );
    // The environment is beside the overlay, never inside it, so the overlay's digest is as it was.
    assert!(!plan.root.join(".venv").exists());
    let again = aos::tree_digest(&StdFs, &plan.root, &Settings::default()).expect("digest");
    assert_eq!(again.digest.to_string(), plan.digest);
    // Setup ran as `install`, with the overlay's root and a state directory it could write to.
    let log = std::fs::read_to_string(home.state("needy").join("phases.log"))
        .expect("setup wrote its log");
    assert_eq!(log, "install install needy\n");

    // A sync rebuilds the environment and runs setup again as `sync`, in the state the first left.
    let second = run(&plan, Change::Sync, &home, &finder, &env).expect("synced");
    assert_eq!(second.phase, Some(Phase::Sync));
    assert_eq!(second.steps, ["deps:python", "setup", "record"]);
    let log = std::fs::read_to_string(home.state("needy").join("phases.log")).expect("log");
    assert_eq!(log, "install install needy\nsync sync needy\n");
    assert!(
        PathBuf::from(&home.packages("needy")["python"])
            .join("canoa_test_pkg")
            .is_dir()
    );

    // Removing runs setup as `remove` and takes the environment, the state and the record away.
    let removed = run(&plan, Change::Remove, &home, &finder, &env).expect("removed");
    assert_eq!(removed.phase, Some(Phase::Remove));
    assert_eq!(removed.steps, ["setup", "delete"]);
    assert!(home.packages("needy").is_empty());
    assert!(!home.state("needy").exists());
    assert_eq!(home.record("needy"), None);
}

#[test]
fn a_failed_install_names_its_step_and_output_and_leaves_what_was_there() {
    let python = python_or_return!();
    let scratch = Scratch::new("fail");
    let index = scratch.0.join("index");
    wheel(&index, "canoa_test_pkg", "1.0", "kept");
    let home = Home::new(scratch.overlays());
    let finder = finder_for(&python);
    let env = index_env(&index);
    let good = plan_for(
        &scratch,
        "needy",
        &[
            ("overlay.cfg", "name = \"needy\"\n"),
            ("lib/python/requirements.txt", "canoa-test-pkg==1.0\n"),
        ],
    );
    run(&good, Change::Add, &home, &finder, &env).expect("prepared");
    let before = home.packages("needy");
    let recorded = home.record("needy").expect("recorded");

    // The next version asks for a package the index does not hold.
    let worse = plan_for(
        &scratch,
        "needy",
        &[(
            "lib/python/requirements.txt",
            "canoa-test-pkg==1.0\ncanoa-test-absent==9.9\n",
        )],
    );
    assert_ne!(worse.digest, good.digest);
    let failure = run(&worse, Change::Sync, &home, &finder, &env).expect_err("it fails");
    assert_eq!(failure.step, "dependencies (python)");
    assert!(
        failure.what.contains("canoa-test-absent"),
        "{}",
        failure.what
    );
    assert_eq!(
        home.record("needy"),
        Some(recorded),
        "the record still names the last good digest"
    );
    assert_eq!(
        home.packages("needy"),
        before,
        "the environment is as it was"
    );
    assert!(
        PathBuf::from(&before["python"])
            .join("canoa_test_pkg")
            .is_dir()
    );
    assert!(
        !scratch
            .overlays()
            .join(".environments/needy/python.next")
            .exists()
    );

    // A requirements file with an option in it is refused before pip runs.
    let option = plan_for(
        &scratch,
        "optioned",
        &[
            ("overlay.cfg", "name = \"optioned\"\n"),
            (
                "lib/python/requirements.txt",
                "--index-url http://example.invalid/simple\ncanoa-test-pkg\n",
            ),
        ],
    );
    let failure = run(&option, Change::Add, &home, &finder, &env).expect_err("refused");
    assert!(
        failure.what.contains("not a requirements file"),
        "{}",
        failure.what
    );
    assert_eq!(home.record("optioned"), None);
}

#[test]
fn a_setup_that_fails_or_overruns_is_stopped_and_reported_with_what_it_printed() {
    let python = python_or_return!();
    let scratch = Scratch::new("setup");
    let home = Home::new(scratch.overlays());
    let finder = finder_for(&python);
    let fails = plan_for(
        &scratch,
        "fails",
        &[
            ("overlay.cfg", "name = \"fails\"\n"),
            (
                "setup.py",
                "import sys\nprint('boom from setup')\nsys.exit(3)\n",
            ),
        ],
    );
    let failure = run(&fails, Change::Add, &home, &finder, &[]).expect_err("fails");
    assert_eq!(failure.step, "setup");
    assert!(failure.what.contains("boom from setup"), "{}", failure.what);
    assert!(failure.what.contains('3'), "{}", failure.what);
    assert_eq!(
        home.record("fails"),
        None,
        "nothing is recorded, so the next run installs again"
    );

    let slow = plan_for(
        &scratch,
        "slow",
        &[
            (
                "overlay.cfg",
                "name = \"slow\"\n[[setup]]\ninterpreter = \"python\"\npath = \"setup.py\"\ntimeout = 1\n",
            ),
            ("setup.py", "import time\ntime.sleep(60)\n"),
        ],
    );
    let started = Instant::now();
    let failure = run(&slow, Change::Add, &home, &finder, &[]).expect_err("overruns");
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "it was ended at its limit"
    );
    assert!(
        failure.what.contains("was stopped after"),
        "{}",
        failure.what
    );
    assert_eq!(home.record("slow"), None);
}

#[test]
fn an_overlay_with_nothing_to_prepare_runs_nothing_and_records_nothing() {
    let scratch = Scratch::new("nothing");
    let plan = plan_for(&scratch, "plain", &[("overlay.cfg", "name = \"plain\"\n")]);
    let home = Home::new(scratch.overlays());
    let finder = Finder::new(&|_| None, &[]);
    let done = run(&plan, Change::Add, &home, &finder, &[]).expect("nothing to do");
    assert_eq!(
        done,
        Prepared {
            phase: None,
            steps: Vec::new()
        }
    );
    assert_eq!(home.record("plain"), None);
    assert!(!scratch.overlays().join(".state").exists());
}
