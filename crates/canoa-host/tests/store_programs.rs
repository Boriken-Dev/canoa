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
    let programs = Programs::new(Shelf::new(dir.0.join("programs")), registry())
        .transport(transport.clone())
        .options(options);
    Setup {
        dir,
        programs,
        transport,
        fixture,
    }
}

fn install(
    s: &Setup,
    version: Option<&str>,
    action: Action,
) -> Result<Option<canoa_host::store::programs::Applied>, StoreError> {
    let settings = StoreSettings::default();
    let store = s.programs.store("echo_store", &settings)?;
    let plan = s
        .programs
        .plan(&*store, action, None, version.map(str::to_owned))?;
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
    let report = s.programs.report(&*store, true).expect("report");
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
        .plan(&*store, Action::Update, None, None)
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
        .plan(&*store, Action::Install, None, None)
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

/// A store beside which something else keeps rewriting the profile for a while, as a running agent rewrites its own state.
struct Noisy {
    engine: Engine<EchoRecipe>,
    rc: PathBuf,
}

impl ProgramStore for Noisy {
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
        let rc = self.rc.clone();
        std::thread::spawn(move || {
            for n in 0..40 {
                let _ = fs::write(&rc, format!("# rewritten {n}\n"));
                std::thread::sleep(std::time::Duration::from_millis(60));
            }
        });
        self.engine.apply(&r, &s, &t, &p)
    }
}

#[test]
fn a_change_still_going_on_after_the_install_ends_is_not_blamed_on_it() {
    let s = setup("noisy", Variant::Normal);
    let store: Arc<dyn ProgramStore> = Arc::new(Noisy {
        engine: Engine::new(EchoRecipe, StoreSettings::default()),
        rc: s.dir.0.join("home/.bashrc"),
    });
    let plan = s
        .programs
        .plan(&*store, Action::Install, None, None)
        .expect("plan");
    let applied = s
        .programs
        .apply_store(
            store,
            "echo_store",
            &plan,
            Arc::new(Recorder::default()),
            Arc::new(AtomicBool::new(false)),
        )
        .expect("something else's ongoing writes do not stop the install");
    assert!(applied.is_some());
}

#[test]
fn an_elevated_host_installs_only_when_told_it_may() {
    let s = setup("elevated", Variant::Normal);
    let mut options = Options::default();
    options.home = Some(s.dir.0.join("home"));
    options.watch_registry = false;
    options.allow_elevated = false;
    let programs = Programs::new(Shelf::new(s.dir.0.join("strict")), registry())
        .transport(s.transport.clone())
        .options(options);
    let settings = StoreSettings::default();
    let store = programs.store("echo_store", &settings).expect("store");
    let plan = programs
        .plan(&*store, Action::Install, None, None)
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
