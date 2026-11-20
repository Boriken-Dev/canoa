//! Preparing an installed overlay as the specification's phases say: its dependencies installed into an environment of its own,
//! then its setup run, and the preparation recorded. Both run outside any agent's session, when the person installs or syncs the
//! overlay and says yes to what the plan lists.
//!
//! Every overlay gets an environment of its own (the specification lets a consumer share less than an overlay asks), kept beside
//! its folder and outside it, so the overlay's tree digest never covers it.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files, paths and processes are std's"
)]

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::time::Duration;
use std::ffi::OsString;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

use coqui_aos::{
    self as aos, Change, Dependency, Facts, OverlayTree, Phase, Runtime, Settings, SetupRecord,
    SetupRun, StdFs, Step,
};
use serde_json::{Value, json};

use super::needs::{Finder, Line, program_for};

/// How long an installer may run before it is taken for hung.
const INSTALL_TIME: Duration = Duration::from_secs(30 * 60);
/// How much of a failed step's output is kept, of each stream.
const TAIL_BYTES: usize = 8 * 1024;

/// Where an overlays folder keeps what preparing makes: beside the overlays, never inside one.
#[derive(Clone, Debug)]
pub(crate) struct Home {
    overlays: PathBuf,
}

impl Home {
    /// The preparations of the overlays in `overlays`.
    pub(crate) fn new(overlays: impl Into<PathBuf>) -> Self {
        Self {
            overlays: overlays.into(),
        }
    }

    fn environment(&self, identity: &str) -> PathBuf {
        self.overlays.join(".environments").join(identity)
    }

    /// The state directory of `identity`.
    pub(crate) fn state(&self, identity: &str) -> PathBuf {
        self.overlays.join(".state").join(identity)
    }

    /// The folder of the launchers of the commands `identity` exposes.
    pub(crate) fn commands(&self, identity: &str) -> PathBuf {
        self.environment(identity).join("commands")
    }

    fn record_file(&self, identity: &str) -> PathBuf {
        self.overlays
            .join(".prepared")
            .join(format!("{identity}.json"))
    }

    /// The last preparation of `identity` that succeeded.
    pub(crate) fn record(&self, identity: &str) -> Option<SetupRecord> {
        let value: Value =
            serde_json::from_slice(&std::fs::read(self.record_file(identity)).ok()?).ok()?;
        Some(SetupRecord::new(
            value.get("digest")?.as_str()?,
            Phase::named(value.get("phase")?.as_str()?)?,
        ))
    }

    pub(crate) fn write_record(&self, identity: &str, record: &SetupRecord) -> std::io::Result<()> {
        let text = json!({"digest": record.digest, "phase": record.phase.name()}).to_string();
        crate::replace::write_file(
            &self.record_file(identity),
            text.as_bytes(),
            crate::replace::SHARED,
        )
    }

    /// Each runtime's packages folder of `identity`, for the runtimes whose environment is built: what goes on `PYTHONPATH` and
    /// `NODE_PATH` after the overlay's own `lib/`.
    pub(crate) fn packages(&self, identity: &str) -> BTreeMap<String, String> {
        let environment = self.environment(identity);
        let mut found = BTreeMap::new();
        if let Some(dir) = site_packages(&environment.join("python")) {
            found.insert("python".to_owned(), dir.to_string_lossy().into_owned());
        }
        let modules = environment.join("node").join("node_modules");
        if modules.is_dir() {
            found.insert("node".to_owned(), modules.to_string_lossy().into_owned());
        }
        found
    }
}

/// The `site-packages` folder of a virtual environment at `venv`.
fn site_packages(venv: &Path) -> Option<PathBuf> {
    let windows = venv.join("Lib").join("site-packages");
    if windows.is_dir() {
        return Some(windows);
    }
    std::fs::read_dir(venv.join("lib"))
        .ok()?
        .flatten()
        .map(|entry| entry.path().join("site-packages"))
        .find(|candidate| candidate.is_dir())
}

/// What preparing an overlay would do at one digest, read from its installed files.
pub(crate) struct Plan {
    identity: String,
    root: PathBuf,
    digest: String,
    effective: aos::Effective,
    /// The commands its `scripts` expose, which installing makes available and runs nothing of.
    commands: Vec<super::commands::Declared>,
}

impl Plan {
    /// The plan for the overlay installed at `root`, whose files must be those `digest` names.
    ///
    /// # Errors
    ///
    /// Why the overlay cannot be read: its files are not those the digest names, its manifest is not one, or it is not
    /// preparable.
    pub(crate) fn of(identity: &str, root: &Path, os: &str, digest: &str) -> Result<Self, String> {
        let (tree, found) = aos::read_tree(&StdFs, root, &Settings::default())
            .map_err(|error| format!("its files cannot be read ({})", error.class))?;
        if !found.digest.to_string().eq_ignore_ascii_case(digest) {
            return Err("its files are not those its record's digest names".to_owned());
        }
        Self::over(identity, root, os, digest, tree)
    }

    /// The plan for an overlay that is not installed yet, whose files are `tree` as its source holds them: what installing it
    /// would run, for the person to read first.
    ///
    /// # Errors
    ///
    /// Why the overlay cannot be read.
    pub(crate) fn from_tree(
        identity: &str,
        os: &str,
        digest: &str,
        tree: OverlayTree,
    ) -> Result<Self, String> {
        Self::over(identity, Path::new(""), os, digest, tree)
    }

    fn over(
        identity: &str,
        root: &Path,
        os: &str,
        digest: &str,
        tree: OverlayTree,
    ) -> Result<Self, String> {
        let read = aos::read_overlay(&tree, &Settings::default())
            .map_err(|error| format!("its manifest cannot be read ({})", error.class))?;
        // An overlay Canoa would leave out whole prepares nothing, as one whose paths leave its folder would run nothing of it.
        let paths = OverlayTree::from_paths(tree.paths().map(str::to_owned));
        if let Some(issue) = aos::validate(&read.manifest, Some(&paths))
            .iter()
            .find(|issue| !aos::applies(core::slice::from_ref(issue)))
        {
            return Err(format!(
                "its manifest is not valid ({} {})",
                issue.at, issue.code
            ));
        }
        let mut facts = Facts::new(os);
        facts.root = root.to_string_lossy().into_owned();
        facts.identity = identity.to_owned();
        facts.pinned = true;
        facts.digest = Some(digest.to_owned());
        facts.tree = tree;
        // Preparing runs the setup it names, whatever env scripts are allowed.
        let effective = aos::effective(&read.manifest, &facts)
            .map_err(|error| format!("its manifest cannot be applied ({})", error.class))?;
        Ok(Self {
            identity: identity.to_owned(),
            root: root.to_path_buf(),
            digest: digest.to_owned(),
            effective,
            commands: super::commands::declared(&read.manifest, os),
        })
    }

    /// What preparing runs, one line each, in the order it runs them, for the person to read before saying yes.
    pub(crate) fn lines(&self) -> Vec<String> {
        self.effective
            .dependencies
            .iter()
            .map(dependency_line)
            .chain(self.effective.setup.as_ref().map(setup_line))
            .chain(self.commands.iter().map(command_line))
            .collect()
    }
}

/// What installing one dependency file runs.
fn dependency_line(dependency: &Dependency) -> String {
    let how = match (dependency.runtime, dependency.lock.is_some()) {
        (Runtime::Python, true) => {
            "pip install, hash-checked, no source builds, into an environment of its own"
        }
        (Runtime::Python, false) => {
            "pip install, not pinned (what the index serves that day), into an environment of its own"
        }
        (_, true) => "npm ci into an environment of its own",
        (_, false) => "npm install, not pinned, into an environment of its own",
    };
    format!(
        "dependencies: {} ({how}; fetches from the network)",
        dependency.file
    )
}

/// What exposing one command does: nothing runs until it is called.
fn command_line(command: &super::commands::Declared) -> String {
    format!(
        "scripts.{}: a command on the agent's PATH that runs {} under {} when it is called; nothing runs now",
        command.name, command.path, command.interpreter
    )
}

/// What running a setup does.
fn setup_line(setup: &SetupRun) -> String {
    let time = if setup.timeout == 0 {
        "no time limit".to_owned()
    } else {
        format!("up to {} s", setup.timeout)
    };
    let network = if setup.network {
        "says it needs the network"
    } else {
        "declares no network, which Canoa cannot keep it to"
    };
    format!(
        "setup: {} under {} ({time}; {network}), as install, then sync on each update",
        setup.path, setup.interpreter
    )
}

/// What preparing made of an overlay at `digest`, as report lines: each dependency file installed and the setup run, or not yet.
pub(crate) fn lines_for(
    home: &Home,
    identity: &str,
    digest: &str,
    dependencies: &[Dependency],
    setup: Option<&SetupRun>,
) -> Vec<Line> {
    let record = home
        .record(identity)
        .filter(|record| record.digest.eq_ignore_ascii_case(digest));
    let packages = home.packages(identity);
    let not_yet = "not done for this version: install or sync the overlay in Canoa, which lists what runs and asks first";
    let mut lines = Vec::new();
    for dependency in dependencies {
        let runtime = dependency.runtime.as_str();
        let held = record.as_ref().and(packages.get(runtime));
        lines.push(Line {
            key: format!("dependencies: {}", dependency.file),
            met: held.is_some(),
            how: held.map_or_else(|| not_yet.to_owned(), |dir| format!("installed in {dir}")),
            offer: None,
            runs: Some(dependency_line(dependency)),
        });
    }
    if let Some(setup) = setup {
        lines.push(Line {
            key: format!("setup: {}", setup.path),
            met: record.is_some(),
            how: record.as_ref().map_or_else(
                || not_yet.to_owned(),
                |record| format!("ran as {} for this version", record.phase.name()),
            ),
            offer: None,
            runs: Some(setup_line(setup)),
        });
    }
    lines
}

/// A preparation that failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Failure {
    /// The step that failed: `dependencies (python)`, `setup`, ...
    pub(crate) step: String,
    /// Why, with the end of what the step printed.
    pub(crate) what: String,
}

impl core::fmt::Display for Failure {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{} failed: {}", self.step, self.what)
    }
}

fn failed(step: impl Into<String>, what: impl Into<String>) -> Failure {
    Failure {
        step: step.into(),
        what: what.into(),
    }
}

/// What a successful preparation did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Prepared {
    /// The phase it ran as, none when there was nothing to prepare.
    pub(crate) phase: Option<Phase>,
    /// The steps taken, in order.
    pub(crate) steps: Vec<String>,
}

/// Prepares `plan`'s overlay for `change`: the steps of [`aos::preparation`] in order, the first that fails ending it and
/// leaving the record as it was. `environment` is added to the environment of every process a step starts (the person's own
/// package-manager settings come from this process's).
///
/// # Errors
///
/// The step that failed and why.
pub(crate) fn run(
    plan: &Plan,
    change: Change,
    home: &Home,
    finder: &Finder,
    environment: &[(OsString, OsString)],
) -> Result<Prepared, Failure> {
    let identity = plan.identity.as_str();
    let record = home.record(identity);
    let steps = aos::preparation(
        change,
        record.as_ref(),
        Some(&plan.digest),
        plan.effective.setup.as_ref(),
        &plan.effective.dependencies,
        None,
    );
    let mut taken = Vec::new();
    for step in &steps.steps {
        match step {
            Step::EmptyState => {
                let state = home.state(identity);
                let _ = std::fs::remove_dir_all(&state);
                std::fs::create_dir_all(&state)
                    .map_err(|error| failed("state directory", error.to_string()))?;
            }
            Step::Deps(runtime) => {
                std::fs::create_dir_all(home.state(identity))
                    .map_err(|error| failed("state directory", error.to_string()))?;
                for dependency in plan
                    .effective
                    .dependencies
                    .iter()
                    .filter(|each| each.runtime == *runtime)
                {
                    install(plan, dependency, home, finder, environment)?;
                }
            }
            Step::Setup => {
                let (Some(setup), Some(phase)) = (plan.effective.setup.as_ref(), steps.phase)
                else {
                    continue;
                };
                std::fs::create_dir_all(home.state(identity))
                    .map_err(|error| failed("state directory", error.to_string()))?;
                run_setup(plan, setup, phase, home, finder, environment)?;
            }
            Step::Record => {
                if let Some(record) = &steps.record {
                    home.write_record(identity, record)
                        .map_err(|error| failed("record", error.to_string()))?;
                }
            }
            Step::Delete => delete(home, identity),
            _ => {}
        }
        taken.push(step.to_string());
    }
    Ok(Prepared {
        phase: steps.phase,
        steps: taken,
    })
}

/// Removes what preparing made for `identity`: its environments, state directory, launchers and record.
pub(crate) fn delete(home: &Home, identity: &str) {
    for dir in [home.environment(identity), home.state(identity)] {
        let _ = std::fs::remove_dir_all(dir);
    }
    let _ = std::fs::remove_file(home.record_file(identity));
}

/// The variables a setup step and an installer's process get besides the person's own.
fn overlay_variables(plan: &Plan, home: &Home) -> Vec<(OsString, OsString)> {
    let root = plan.root.to_string_lossy().into_owned();
    let state = home.state(&plan.identity).to_string_lossy().into_owned();
    let name = aos::normalize_name(&plan.identity);
    vec![
        (
            aos::root_variable(&plan.identity).into(),
            root.clone().into(),
        ),
        (
            aos::state_variable(&plan.identity).into(),
            state.clone().into(),
        ),
        ("AOS_OVERLAY_ROOT".into(), root.into()),
        ("AOS_OVERLAY_NAME".into(), name.into()),
        ("AOS_OVERLAY_STATE".into(), state.into()),
    ]
}

fn run_setup(
    plan: &Plan,
    setup: &SetupRun,
    phase: Phase,
    home: &Home,
    finder: &Finder,
    environment: &[(OsString, OsString)],
) -> Result<(), Failure> {
    let step = "setup";
    let script = plan.root.join(&setup.path);
    if !script.is_file() {
        return Err(failed(
            step,
            format!("{} is not a file of the overlay", setup.path),
        ));
    }
    let program = program_for(finder, &setup.interpreter, None).map_err(|why| failed(step, why))?;
    let mut command = crate::spawn::Command::new(&program);
    if setup.interpreter == "powershell" {
        command.args(["-NoLogo", "-NoProfile", "-NonInteractive", "-File"]);
    }
    command.arg(&script).arg(phase.name()).args(&setup.args);
    command.current_dir(&plan.root);
    command.envs(environment.iter().cloned());
    command.envs(overlay_variables(plan, home));
    command.env("AOS_SETUP_PHASE", phase.name());
    let limit = (setup.timeout > 0).then(|| Duration::from_secs(setup.timeout));
    run_bounded(command, limit)
        .map(|_| ())
        .map_err(|what| failed(step, what))
}

fn install(
    plan: &Plan,
    dependency: &Dependency,
    home: &Home,
    finder: &Finder,
    environment: &[(OsString, OsString)],
) -> Result<(), Failure> {
    let (step, ours) = match dependency.runtime {
        Runtime::Python => ("dependencies (python)", "python"),
        _ => ("dependencies (node)", "node"),
    };
    let file = plan.root.join(&dependency.file);
    let text = std::fs::read(&file)
        .map_err(|error| failed(step, format!("{} cannot be read: {error}", dependency.file)))?;
    let next = home
        .environment(&plan.identity)
        .join(format!("{ours}.next"));
    let _ = std::fs::remove_dir_all(&next);
    std::fs::create_dir_all(&next).map_err(|error| failed(step, error.to_string()))?;
    let outcome = match dependency.runtime {
        Runtime::Python => python_environment(plan, dependency, &text, &next, finder, environment),
        _ => node_environment(plan, dependency, &file, &next, finder, environment),
    };
    if let Err(what) = outcome {
        let _ = std::fs::remove_dir_all(&next);
        return Err(failed(step, what));
    }
    // The old environment is replaced only now that the new one is built.
    let current = home.environment(&plan.identity).join(ours);
    let old = home.environment(&plan.identity).join(format!("{ours}.old"));
    let _ = std::fs::remove_dir_all(&old);
    if current.exists() {
        std::fs::rename(&current, &old).map_err(|error| failed(step, error.to_string()))?;
    }
    std::fs::rename(&next, &current).map_err(|error| failed(step, error.to_string()))?;
    let _ = std::fs::remove_dir_all(&old);
    Ok(())
}

fn python_environment(
    plan: &Plan,
    dependency: &Dependency,
    text: &[u8],
    next: &Path,
    finder: &Finder,
    environment: &[(OsString, OsString)],
) -> Result<(), String> {
    let locked = dependency.lock.is_some();
    aos::check_requirements(&String::from_utf8_lossy(text), locked).map_err(|error| {
        format!(
            "{} is not a requirements file ({})",
            dependency.file, error.class
        )
    })?;
    let minimum = plan
        .effective
        .python
        .as_ref()
        .and_then(|variant| variant.get("version"))
        .and_then(Value::as_str);
    let python = program_for(finder, "python", minimum)?;
    let venv = next;
    let mut make = crate::spawn::Command::new(&python);
    make.args(["-m", "venv"]).arg(venv);
    make.envs(environment.iter().cloned());
    run_bounded(make, Some(INSTALL_TIME))
        .map_err(|what| format!("making the environment: {what}"))?;
    let inside = if cfg!(windows) {
        venv.join("Scripts").join("python.exe")
    } else {
        venv.join("bin").join("python")
    };
    let mut pip = crate::spawn::Command::new(&inside);
    pip.args([
        "-m",
        "pip",
        "install",
        "--disable-pip-version-check",
        "--no-input",
    ]);
    if locked {
        pip.args(["--require-hashes", "--no-deps", "--only-binary=:all:"]);
    }
    pip.arg("-r").arg(plan.root.join(&dependency.file));
    pip.envs(environment.iter().cloned());
    run_bounded(pip, Some(INSTALL_TIME))?;
    Ok(())
}

fn node_environment(
    plan: &Plan,
    dependency: &Dependency,
    file: &Path,
    next: &Path,
    finder: &Finder,
    environment: &[(OsString, OsString)],
) -> Result<(), String> {
    let minimum = plan
        .effective
        .node
        .as_ref()
        .and_then(|variant| variant.get("version"))
        .and_then(Value::as_str);
    program_for(finder, "node", minimum)?;
    let npm = finder
        .find("npm")
        .ok_or_else(|| "npm is not on the PATH".to_owned())?;
    // npm reads the two files and nothing else of the overlay, so no `.npmrc` of it configures the install.
    std::fs::copy(file, next.join("package.json")).map_err(|error| error.to_string())?;
    if let Some(lock) = &dependency.lock {
        std::fs::copy(plan.root.join(lock), next.join("package-lock.json"))
            .map_err(|error| error.to_string())?;
    }
    let mut command = crate::spawn::Command::new(npm);
    command
        .arg(if dependency.lock.is_some() {
            "ci"
        } else {
            "install"
        })
        .args(["--no-audit", "--no-fund", "--prefix"])
        .arg(next);
    command.current_dir(next);
    command.envs(environment.iter().cloned());
    run_bounded(command, Some(INSTALL_TIME)).map(|_| ())
}

/// Runs `command` to its end in a process group of its own, ending everything it started when it ends or after `limit`. Its
/// output is read as it comes and only the end of each stream is kept.
fn run_bounded(
    mut command: crate::spawn::Command,
    limit: Option<Duration>,
) -> Result<String, String> {
    use crate::spawn::Stdio;
    command
        .stdin(Stdio::Null)
        .stdout(Stdio::Piped)
        .stderr(Stdio::Piped)
        .own_group();
    let mut child = command
        .spawn()
        .map_err(|error| format!("it could not start: {error}"))?;
    let tail = |stream: Option<Box<dyn std::io::Read + Send>>| {
        std::thread::spawn(move || {
            let mut kept: Vec<u8> = Vec::new();
            if let Some(mut stream) = stream {
                let mut chunk = [0_u8; 4096];
                while let Ok(read) = stream.read(&mut chunk) {
                    if read == 0 {
                        break;
                    }
                    kept.extend_from_slice(&chunk[..read]);
                    if kept.len() > TAIL_BYTES {
                        kept.drain(..kept.len() - TAIL_BYTES);
                    }
                }
            }
            String::from_utf8_lossy(&kept).into_owned()
        })
    };
    let out = tail(child.stdout.take().map(|s| Box::new(s) as _));
    let err = tail(child.stderr.take().map(|s| Box::new(s) as _));
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if limit.is_none_or(|limit| started.elapsed() < limit) => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => break None,
        }
    };
    // Nothing it started outlives it.
    match status {
        Some(_) => child.end_group_after_exit(),
        None => {
            let _ = child.kill();
            child.end_group();
            let _ = child.wait();
        }
    }
    let (out, err) = (
        out.join().unwrap_or_default(),
        err.join().unwrap_or_default(),
    );
    let shown = [err.trim(), out.trim()]
        .into_iter()
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    match status {
        Some(status) if status.success() => Ok(shown),
        Some(status) => Err(format!(
            "it ended with {status}{}",
            if shown.is_empty() {
                String::new()
            } else {
                format!(":\n{shown}")
            }
        )),
        None => Err(format!(
            "it was stopped after {} s{}",
            started.elapsed().as_secs(),
            if shown.is_empty() {
                String::new()
            } else {
                format!(":\n{shown}")
            }
        )),
    }
}

#[cfg(test)]
mod tests;
