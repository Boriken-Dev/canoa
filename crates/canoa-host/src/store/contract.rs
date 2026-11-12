//! The suite every `program_store` is held to, and the fakes it runs on: a world of documents in place of the network, a
//! stand-in program built at test time, and a throwaway home whose every change outside the version's folder is a failure.
//!
//! A store passes when [`run`] returns no [`Failure`]; each failure names the check it broke. The suite is strict and offline:
//! every spawn goes through a guarded spawner, every fetch through a world, and the registry keys an installer writes are the
//! real ones (a throwaway profile does not redirect them), restored whatever happened.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a suite names the files and clocks it runs on"
)]

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use canoa::store::{
    Action, ApplyRequest, InstallPlan, InstallPrompter, InstallTools, LatestQuery, ManagedCopy,
    PlanRequest, Platform, ProgramQuery, ProgramStore, Progress, StoreInfo, is_program_name,
};
use coqui_acp_api::guatiao::library::ProviderError;
use coqui_acp_api::guatiao::{Secret, Status};
use coqui_acp_api::{Environment, LoginAnswer, LoginForm, ProcessSpawner};
use coqui_acp_harness::LocalSpawner;

use super::exec::GuardedSpawner;
use super::footprint::{Footprint, Watch, uncovered};
use super::programs::install_environment;
use super::tools::{LocalTools, Lookup, Reply, ToolsLimits, Transport};

/// The variable the suite puts a canary secret in, which must appear nowhere a store writes.
pub const CANARY_NAME: &str = "COQUI_SUITE_CANARY_TOKEN";
/// The canary's value.
pub const CANARY_VALUE: &str = "canary-9f3c1d7e-not-a-real-secret";

/// A rule a store broke.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Failure {
    /// The named check, such as `apply.mismatch`.
    pub check: &'static str,
    /// What was seen.
    pub why: String,
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.check, self.why)
    }
}

/// How a fixture's world differs from the normal one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Variant {
    /// The publisher's documents and files, as they are.
    Normal,
    /// The file's bytes do not match the digest the publisher's document gives.
    Mismatch,
    /// Every file's URL fails to connect.
    DownloadFails,
    /// The program downloads, matches and will not run.
    WillNotRun,
    /// The same version, published again with other bytes: what a plan rested on has moved.
    Moved,
    /// Every request is slow.
    Slow,
}

/// What a store's own tests supply: the world its publisher's documents live in.
pub trait Fixture {
    /// The older version served by `Variant::Normal`.
    fn older(&self) -> String;
    /// The newer version.
    fn newer(&self) -> String;
    /// The platform the suite plans for.
    fn platform(&self) -> Platform;
    /// The documents and files, as `variant` says.
    fn world(&self, variant: Variant) -> World;
    /// The variable the launch environment must set to keep the program's own updater off; `None` when it has no switch.
    fn auto_update_var(&self) -> Option<String>;
}

/// URLs and what they answer.
#[derive(Debug, Clone, Default)]
pub struct World {
    docs: BTreeMap<String, Vec<u8>>,
    failing: BTreeSet<String>,
    slow: bool,
}

impl World {
    /// An empty world: every URL is `404`.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Serves `bytes` at `url`.
    pub fn put(&mut self, url: impl Into<String>, bytes: impl Into<Vec<u8>>) -> &mut Self {
        self.docs.insert(url.into(), bytes.into());
        self
    }

    /// Makes `url` fail to connect.
    pub fn fail(&mut self, url: impl Into<String>) -> &mut Self {
        self.failing.insert(url.into());
        self
    }

    /// Fails every URL that is not a version document: those holding more than `limit` bytes.
    pub fn fail_big(&mut self, limit: usize) -> &mut Self {
        let big: Vec<String> = self
            .docs
            .iter()
            .filter(|(_, v)| v.len() > limit)
            .map(|(k, _)| k.clone())
            .collect();
        self.failing.extend(big);
        self
    }

    /// Makes every body arrive slowly.
    pub const fn slow(&mut self) -> &mut Self {
        self.slow = true;
        self
    }
}

/// A copy of the stand-in program with its trailer: it prints `line` and exits `exit`; `auto_update_var` names the
/// variable that keeps its updater off.
#[must_use]
pub fn standin_bytes(
    standin: &Path,
    line: &str,
    exit: u32,
    auto_update_var: Option<&str>,
) -> Vec<u8> {
    let mut bytes = fs::read(standin).expect("the stand-in is built");
    let trailer =
        serde_json::json!({"line": line, "exit": exit, "auto_update_var": auto_update_var});
    bytes.extend_from_slice(b"\nCOQUI-STANDIN:");
    bytes.extend_from_slice(trailer.to_string().as_bytes());
    bytes.push(b'\n');
    bytes
}

/// The transport the suite fetches through: the world, and a record of every request.
#[derive(Debug)]
pub struct WorldTransport {
    world: Mutex<World>,
    requests: Mutex<Vec<String>>,
}

impl WorldTransport {
    /// Serves `world`.
    #[must_use]
    pub fn new(world: World) -> Arc<Self> {
        Arc::new(Self {
            world: Mutex::new(world),
            requests: Mutex::new(Vec::new()),
        })
    }

    /// Every URL asked for, in order.
    #[must_use]
    pub fn requests(&self) -> Vec<String> {
        self.requests.lock().map(|r| r.clone()).unwrap_or_default()
    }

    /// Replaces what is served.
    pub fn serve(&self, world: World) {
        if let Ok(mut current) = self.world.lock() {
            *current = world;
        }
    }
}

struct Slowly(Cursor<Vec<u8>>);

impl std::io::Read for Slowly {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        std::thread::sleep(Duration::from_millis(120));
        let take = buf.len().min(16 * 1024);
        self.0.read(&mut buf[..take])
    }
}

impl Transport for WorldTransport {
    fn get(&self, url: &str, _limits: &ToolsLimits) -> Result<Reply, String> {
        if let Ok(mut requests) = self.requests.lock() {
            requests.push(url.to_owned());
        }
        let world = self.world.lock().map_err(|_| "world poisoned".to_owned())?;
        if world.failing.contains(url) {
            return Err("connection refused".to_owned());
        }
        let (status, bytes) = world
            .docs
            .get(url)
            .map_or((404, Vec::new()), |bytes| (200, bytes.clone()));
        let body: Box<dyn std::io::Read + Send> = if world.slow {
            Box::new(Slowly(Cursor::new(bytes)))
        } else {
            Box::new(Cursor::new(bytes))
        };
        Ok(Reply {
            status,
            location: None,
            body,
        })
    }
}

/// A prompter that records and cancels when told to.
#[derive(Debug, Default)]
struct SuitePrompter {
    cancel_at_first_call: bool,
    lines: Mutex<Vec<String>>,
    calls: Mutex<u32>,
}

impl SuitePrompter {
    fn touch(&self) -> Result<(), ProviderError> {
        if let Ok(mut calls) = self.calls.lock() {
            *calls += 1;
        }
        if self.cancel_at_first_call {
            return Err(ProviderError::new(Status::GUATIAO_ERR_GONE, "cancelled"));
        }
        Ok(())
    }
}

impl InstallPrompter for SuitePrompter {
    fn say(&self, line: &str) -> Result<(), ProviderError> {
        self.touch()?;
        if let Ok(mut lines) = self.lines.lock() {
            lines.push(line.to_owned());
        }
        Ok(())
    }

    fn progress(&self, _progress: Progress) -> Result<(), ProviderError> {
        self.touch()
    }

    fn ask(&self, _form: LoginForm) -> Result<LoginAnswer, ProviderError> {
        self.touch()?;
        Err(ProviderError::new(
            Status::GUATIAO_ERR_UNSUPPORTED,
            "no one to ask",
        ))
    }

    fn cancelled(&self) -> Result<bool, ProviderError> {
        Ok(self.cancel_at_first_call)
    }
}

/// A throwaway home, programs folder and `PATH` directory, removed when dropped.
#[derive(Debug)]
pub struct Sandbox {
    root: PathBuf,
}

impl Sandbox {
    /// A new one under the system's temporary directory.
    ///
    /// # Panics
    ///
    /// When it cannot be made.
    #[must_use]
    pub fn new() -> Self {
        let mut nonce = [0u8; 6];
        getrandom::fill(&mut nonce).expect("random bytes");
        let name: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
        let root = std::env::temp_dir().join(format!("coqui-store-suite-{name}"));
        for dir in ["home", "bin", "work"] {
            fs::create_dir_all(root.join(dir)).expect("the sandbox is made");
        }
        Self { root }
    }

    /// The sandbox's home.
    #[must_use]
    pub fn home(&self) -> PathBuf {
        self.root.join("home")
    }

    /// The whole sandbox.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// A new empty version folder.
    fn staging(&self, name: &str) -> PathBuf {
        let dir = self.root.join("work").join(name);
        fs::create_dir_all(&dir).expect("a staging folder");
        dir
    }

    /// The environment a store is run with: every home inside the sandbox, `PATH` on its own `bin` and the system's, and
    /// the canary secret.
    fn environment(&self, staging: &Path) -> Environment {
        let mut env = Environment::new();
        let system = if cfg!(windows) {
            let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_owned());
            env.insert("SystemRoot", root.clone());
            format!("{root}\\System32")
        } else {
            "/usr/bin:/bin".to_owned()
        };
        let sep = if cfg!(windows) { ";" } else { ":" };
        env.insert(
            "PATH",
            format!("{}{sep}{system}", self.root.join("bin").display()),
        );
        let mut env = install_environment(&env, staging);
        let home = self.home().to_string_lossy().into_owned();
        for name in [
            "HOME",
            "USERPROFILE",
            "APPDATA",
            "LOCALAPPDATA",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "XDG_CACHE_HOME",
            "XDG_STATE_HOME",
        ] {
            env.insert(name, home.clone());
        }
        env.insert(CANARY_NAME, CANARY_VALUE);
        env
    }

    fn lookup(&self, env: &Environment) -> Lookup {
        Lookup {
            path: env.get("PATH").unwrap_or("").into(),
            home: Some(self.home()),
            variables: env.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        }
    }
}

impl Default for Sandbox {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// Held while a suite snapshots and restores the machine's registry keys.
static REAL_WORLD: Mutex<()> = Mutex::new(());

/// What one apply left behind.
struct Applied {
    result: Result<canoa::store::ApplyOutcome, ProviderError>,
    spawned: Vec<super::exec::Spawned>,
    violations: Vec<String>,
    differences: Vec<super::footprint::Difference>,
    urls: Vec<String>,
    lines: Vec<String>,
    elapsed: Duration,
    staging: PathBuf,
    secret_leak: bool,
}

struct Runner<'a> {
    store: Arc<dyn ProgramStore>,
    fixture: &'a dyn Fixture,
    info: StoreInfo,
    failures: Vec<Failure>,
    count: u32,
}

/// Holds `store` to every check, on the world `fixture` makes.
///
/// # Panics
///
/// When the sandbox cannot be made.
#[must_use]
pub fn run(store: Arc<dyn ProgramStore>, fixture: &dyn Fixture) -> Vec<Failure> {
    let Ok(info) = store.describe() else {
        return vec![Failure {
            check: "describe.answers",
            why: "describe failed".to_owned(),
        }];
    };
    let mut runner = Runner {
        store,
        fixture,
        info,
        failures: Vec::new(),
        count: 0,
    };
    runner.describe();
    runner.program();
    runner.latest();
    runner.plan();
    runner.apply();
    runner.failures
}

impl Runner<'_> {
    fn fail(&mut self, check: &'static str, why: impl Into<String>) {
        self.failures.push(Failure {
            check,
            why: why.into(),
        });
    }

    fn tools(&self, transport: Arc<WorldTransport>) -> LocalTools {
        LocalTools::new(transport, self.info.endpoints.clone()).allow_unverified(true)
    }

    fn describe(&mut self) {
        let again = self.store.describe();
        if again.as_ref().ok() != Some(&self.info) {
            self.fail(
                "describe.stable",
                "describe answered differently the second time",
            );
        }
        if !is_program_name(&self.info.program) {
            self.fail(
                "describe.name",
                format!("{:?} is not a program folder name", self.info.program),
            );
        }
        if self
            .info
            .endpoints
            .iter()
            .any(|e| !e.starts_with("https://"))
        {
            self.fail("describe.https", "an endpoint that is not an https origin");
        }
        if !self.info.platforms.contains(&self.fixture.platform()) {
            self.fail(
                "describe.platforms",
                "the fixture's platform is not one the store lists",
            );
        }
    }

    fn program(&mut self) {
        let sandbox = Sandbox::new();
        let staging = sandbox.staging("probe");
        let env = sandbox.environment(&staging);
        let transport = WorldTransport::new(World::new());
        let tools = || {
            LocalTools::new(transport.clone(), self.info.endpoints.clone())
                .allow_unverified(true)
                .lookup(sandbox.lookup(&env))
        };
        let spawner =
            GuardedSpawner::new(LocalSpawner::new(), staging.to_string_lossy()).lenient_cwd();
        let log = spawner.log();
        let mut query = ProgramQuery::default();
        query.environment = Secret::new(env.clone());
        let report = self
            .store
            .program(query, spawner.into_object(), tools().into_object());
        match report {
            Ok(report) if report.copies.is_empty() => {}
            Ok(report) => self.fail(
                "program.none_when_missing",
                format!("{} copies found in an empty sandbox", report.copies.len()),
            ),
            Err(error) => self.fail("program.none_when_missing", error.message().to_owned()),
        }
        if !log.spawned().is_empty() {
            self.fail(
                "program.no_spawn_without_probe",
                "it started a program with probe false",
            );
        }
        if !transport.requests().is_empty() {
            self.fail("program.no_network", "it opened a connection");
        }
    }

    fn latest(&mut self) {
        let transport = WorldTransport::new(self.fixture.world(Variant::Normal));
        let result = self.store.latest(
            LatestQuery::default(),
            self.tools(transport.clone()).into_object(),
        );
        match result {
            Ok(report) if report.known && report.version.is_some() => {}
            Ok(report) => self.fail(
                "latest.known",
                format!(
                    "the newest version was not read: {:?}",
                    report.unknown_because
                ),
            ),
            Err(error) => self.fail("latest.known", error.message().to_owned()),
        }
        for url in transport.requests() {
            if !self
                .info
                .endpoints
                .iter()
                .any(|e| url.starts_with(e.as_str()))
            {
                self.fail(
                    "latest.endpoints",
                    format!("fetched {url}, which is not in its endpoints"),
                );
            }
        }
        let mut world = self.fixture.world(Variant::Normal);
        world.fail_big(0);
        let transport = WorldTransport::new(world);
        if let Ok(report) = self
            .store
            .latest(LatestQuery::default(), self.tools(transport).into_object())
            && report.known
        {
            self.fail(
                "latest.failure_is_unknown",
                "a failing publisher still gave a version",
            );
        }
    }

    fn plan_for(
        &self,
        action: Action,
        version: Option<String>,
        managed: Option<ManagedCopy>,
    ) -> Result<InstallPlan, ProviderError> {
        self.plan_in(Variant::Normal, action, version, managed)
    }

    fn plan_in(
        &self,
        variant: Variant,
        action: Action,
        version: Option<String>,
        managed: Option<ManagedCopy>,
    ) -> Result<InstallPlan, ProviderError> {
        let transport = WorldTransport::new(self.fixture.world(variant));
        let mut request = PlanRequest::new(action, self.fixture.platform());
        request.version = version;
        request.managed = managed;
        self.store
            .plan(request, self.tools(transport).into_object())
    }

    fn plan(&mut self) {
        let new = self.fixture.newer();
        let plan = match self.plan_for(Action::Install, None, None) {
            Ok(plan) => plan,
            Err(error) => {
                self.fail("plan.answers", error.message().to_owned());
                return;
            }
        };
        if plan.version.as_deref() != Some(new.as_str()) {
            self.fail(
                "plan.version",
                format!("planned {:?}, the newest is {new}", plan.version),
            );
        }
        if plan.downloads.is_empty() {
            self.fail("plan.downloads", "an install that downloads nothing");
        }
        if let Ok(again) = self.plan_for(Action::Install, None, None)
            && again != plan
        {
            self.fail("plan.deterministic", "the same world gave another plan");
        }
        let launch = canoa::store::Launch::new("/not/used");
        let managed = ManagedCopy::new("/not/used", new.clone(), launch);
        match self.plan_for(Action::Update, None, Some(managed)) {
            Ok(update)
                if update.downloads.is_empty()
                    && update.steps.is_empty()
                    && update.version == update.from_version => {}
            Ok(_) => self.fail(
                "plan.current_is_empty",
                "an update to the current version does something",
            ),
            Err(error) => self.fail("plan.current_is_empty", error.message().to_owned()),
        }
    }

    /// One apply of the newest version, in a fresh sandbox, with every watch on.
    fn apply_with(
        &mut self,
        variant: Variant,
        mutate: impl FnOnce(&mut ApplyRequest, &mut SuitePrompter, &mut Duration),
    ) -> Option<(Applied, Sandbox, InstallPlan)> {
        // A program that will not run is published with a digest that fits it: planned in that world, not the normal one.
        let planned_in = if variant == Variant::WillNotRun {
            variant
        } else {
            Variant::Normal
        };
        let plan = self.plan_in(planned_in, Action::Install, None, None).ok()?;
        self.count += 1;
        let sandbox = Sandbox::new();
        let staging = sandbox.staging(&format!("v{}", self.count));
        let env = sandbox.environment(&staging);
        let transport = WorldTransport::new(self.fixture.world(variant));
        let cancel = Arc::new(AtomicBool::new(false));
        let mut request = ApplyRequest::new(plan.clone(), staging.to_string_lossy());
        request.environment = Secret::new(env.clone());
        let mut prompter = SuitePrompter::default();
        let mut timeout = Duration::from_millis(request.timeout_ms);
        mutate(&mut request, &mut prompter, &mut timeout);
        request.timeout_ms = u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX);
        let deadline = Instant::now() + timeout;
        let tools = LocalTools::new(transport.clone(), self.info.endpoints.clone())
            .allow_unverified(true)
            .granted(&staging)
            .lookup(sandbox.lookup(&env))
            .cancelled_by(cancel)
            .deadline(deadline);
        let urls = tools.url_log();
        let spawner = GuardedSpawner::new(
            LocalSpawner::new().files(&staging),
            staging.to_string_lossy(),
        );
        let log = spawner.log();
        let watch = Watch::new(sandbox.home())
            .tree(sandbox.root())
            .skip(&staging)
            .registry(cfg!(windows))
            .path_dirs([sandbox.root().join("bin")]);
        // The registry is the machine's: one apply at a time may snapshot it and put back what it finds changed.
        let machine = REAL_WORLD
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let before = Footprint::take(&watch);
        let prompter = Arc::new(prompter);
        let started = Instant::now();
        let result = self.store.apply(
            request,
            spawner.into_object(),
            tools.into_object(),
            ArcPrompter(Arc::clone(&prompter)).into_object(),
        );
        let elapsed = started.elapsed();
        let after = Footprint::take(&watch);
        let differences = uncovered(&before.diff(&after), &plan.changes);
        let _ = before.restore_registry(&after, &differences);
        drop(machine);
        let lines = prompter.lines.lock().map(|l| l.clone()).unwrap_or_default();
        let mut leaked = lines.iter().any(|l| l.contains(CANARY_VALUE));
        if let Err(error) = &result {
            leaked |= error.message().contains(CANARY_VALUE);
        }
        let spawned = log.spawned();
        leaked |= spawned
            .iter()
            .any(|s| s.args.iter().any(|a| a.contains(CANARY_VALUE)));
        let applied = Applied {
            result,
            spawned,
            violations: log.violations(),
            differences,
            urls: urls.lock().map(|u| u.clone()).unwrap_or_default(),
            lines,
            elapsed,
            staging,
            secret_leak: leaked,
        };
        Some((applied, sandbox, plan))
    }

    fn apply(&mut self) {
        // The control: the stand-in updates itself when its switch is not set, so the check below can fail.
        self.apply_normal();
        self.apply_failures();
    }

    fn apply_normal(&mut self) {
        let Some((applied, sandbox, plan)) = self.apply_with(Variant::Normal, |_, _, _| {}) else {
            self.fail("apply.plan", "no plan to apply");
            return;
        };
        let outcome = match &applied.result {
            Ok(outcome) => outcome.clone(),
            Err(error) => {
                self.fail("apply.succeeds", error.message().to_owned());
                self.report_side_effects(&applied);
                return;
            }
        };
        self.report_side_effects(&applied);
        if outcome.version != plan.version {
            self.fail(
                "apply.readback",
                format!(
                    "read back {:?}, planned {:?}",
                    outcome.version, plan.version
                ),
            );
        }
        let Some(launch) = outcome.launch.clone() else {
            self.fail("apply.launch", "no launch in the outcome");
            return;
        };
        if !Path::new(&launch.path).starts_with(&applied.staging)
            || !Path::new(&launch.path).is_file()
        {
            self.fail(
                "apply.launch_path",
                format!("{} is not a file inside the staging folder", launch.path),
            );
        }
        for name in launch.environment.keys() {
            if super::exec::is_credential_name(name) {
                self.fail(
                    "apply.launch_environment",
                    format!("{name} looks like a credential"),
                );
            }
        }
        if let Some(var) = self.fixture.auto_update_var() {
            if !launch.environment.contains_key(&var) {
                self.fail(
                    "apply.auto_update_off",
                    format!("the launch does not set {var}"),
                );
            }
            if applied.staging.join("auto-updated").exists()
                || Path::new(&launch.path)
                    .with_file_name("auto-updated")
                    .exists()
            {
                self.fail(
                    "apply.auto_update_off",
                    "the program updated itself while the store read its version",
                );
            }
        }
        // Read it back as a host would.
        let transport = WorldTransport::new(World::new());
        let env = sandbox.environment(&applied.staging);
        let mut query = ProgramQuery::default();
        query.managed = Some(ManagedCopy::new(
            applied.staging.to_string_lossy(),
            outcome.version.clone().unwrap_or_default(),
            launch,
        ));
        query.environment = Secret::new(env.clone());
        query.probe = true;
        let tools = LocalTools::new(transport, self.info.endpoints.clone())
            .allow_unverified(true)
            .lookup(sandbox.lookup(&env));
        match self.store.program(
            query,
            LocalSpawner::new().into_object(),
            tools.into_object(),
        ) {
            Ok(report) => {
                let first = report.copies.first();
                if !first.is_some_and(|c| c.managed) {
                    self.fail(
                        "program.managed_first",
                        "the managed copy is not listed first",
                    );
                }
                if first.and_then(|c| c.version.clone()) != plan.version {
                    self.fail(
                        "apply.launch_runs",
                        format!(
                            "a probe of the staged copy read {:?}",
                            first.and_then(|c| c.version.clone())
                        ),
                    );
                }
            }
            Err(error) => self.fail("apply.launch_runs", error.message().to_owned()),
        }
        // Uninstall: what the install declared is undone, and the program has nothing else to undo.
        if plan.changes.is_empty() {
            let transport = WorldTransport::new(self.fixture.world(Variant::Normal));
            let mut request = PlanRequest::new(Action::Uninstall, self.fixture.platform());
            request.managed = None;
            match self
                .store
                .plan(request, self.tools(transport).into_object())
            {
                Ok(plan) if plan.changes.is_empty() => {}
                Ok(_) => self.fail(
                    "uninstall.declares",
                    "an uninstall of an install that declared nothing declares changes",
                ),
                Err(error) => self.fail("uninstall.declares", error.message().to_owned()),
            }
        }
    }

    fn report_side_effects(&mut self, applied: &Applied) {
        if !applied.violations.is_empty() {
            self.fail("apply.spawns", applied.violations.join("; "));
        }
        for spawned in &applied.spawned {
            if !Path::new(&spawned.program).is_absolute() {
                self.fail(
                    "apply.spawns",
                    format!("started {}, which is not absolute", spawned.program),
                );
            }
        }
        for difference in &applied.differences {
            let check = match difference.kind {
                canoa::store::PROFILE_FILE => "apply.profile_file",
                canoa::store::REGISTRY | canoa::store::USER_PATH => "apply.registry",
                _ => "apply.outside_staging",
            };
            self.fail(
                check,
                format!(
                    "{} changed and the plan did not declare it",
                    difference.what
                ),
            );
        }
        if applied.secret_leak {
            self.fail(
                "apply.secrets",
                "the canary secret appeared in an argument, a line or an error",
            );
        }
        let mut seen = BTreeSet::new();
        for url in &applied.urls {
            if !self
                .info
                .endpoints
                .iter()
                .any(|e| url.starts_with(e.as_str()))
            {
                self.fail(
                    "apply.endpoints",
                    format!("fetched {url}, which is not in its endpoints"),
                );
            }
            seen.insert(url.clone());
        }
        let _ = &applied.lines;
    }

    fn apply_failures(&mut self) {
        // A download whose bytes do not match is refused, and never fetched again as unverified.
        let verifiable = self
            .plan_for(Action::Install, None, None)
            .is_ok_and(|plan| plan.downloads.iter().any(|d| !d.is_unverified()));
        if !verifiable {
            // The publisher gives nothing to check against, so a store can neither see a mismatch nor a moved file.
            return self.apply_failures_after_checks();
        }
        if let Some((applied, _, plan)) = self.apply_with(Variant::Mismatch, |_, _, _| {}) {
            match &applied.result {
                Err(error)
                    if error.status == Status::GUATIAO_ERR_BAD_VALUE
                        && error.message().contains("checksum mismatch") => {}
                Err(error)
                    if error.status == Status::GUATIAO_ERR_BAD_VALUE
                        && error.message().contains("plan changed") => {}
                Err(error) => self.fail(
                    "apply.mismatch",
                    format!("answered {:?} {}", error.status, error.message()),
                ),
                Ok(_) => self.fail(
                    "apply.mismatch",
                    "a download that does not match was accepted",
                ),
            }
            for download in &plan.downloads {
                let times = applied.urls.iter().filter(|u| **u == download.url).count();
                if times > 1 {
                    self.fail(
                        "apply.mismatch_no_retry",
                        format!("{} was fetched {times} times", download.url),
                    );
                }
            }
        }
        self.apply_failures_after_checks();
    }

    fn apply_failures_after_checks(&mut self) {
        if let Some((applied, _, _)) = self.apply_with(Variant::DownloadFails, |_, _, _| {}) {
            match &applied.result {
                Err(error)
                    if matches!(
                        error.status,
                        Status::GUATIAO_ERR_INTERNAL | Status::GUATIAO_ERR_BAD_VALUE
                    ) => {}
                Err(error) => self.fail(
                    "apply.download_error",
                    format!("answered {:?}", error.status),
                ),
                Ok(_) => self.fail(
                    "apply.download_error",
                    "an install whose download failed succeeded",
                ),
            }
        }
        if let Some((applied, _, _)) = self.apply_with(Variant::WillNotRun, |_, _, _| {}) {
            match &applied.result {
                Err(error) if error.status == Status::GUATIAO_ERR_INTERNAL => {}
                Err(error) => {
                    self.fail("apply.will_not_run", format!("answered {:?}", error.status))
                }
                Ok(_) => self.fail(
                    "apply.will_not_run",
                    "a program that will not run was installed",
                ),
            }
        }
        // A plan whose facts moved: planned in one world, applied in another.
        let verifiable = self
            .plan_for(Action::Install, None, None)
            .is_ok_and(|plan| plan.downloads.iter().any(|d| !d.is_unverified()));
        if let Some(plan) = self
            .plan_for(Action::Install, None, None)
            .ok()
            .filter(|_| verifiable)
        {
            let moved = self.fixture.world(Variant::Moved);
            let sandbox = Sandbox::new();
            let staging = sandbox.staging("moved");
            let env = sandbox.environment(&staging);
            let transport = WorldTransport::new(moved);
            let tools = LocalTools::new(transport, self.info.endpoints.clone())
                .allow_unverified(true)
                .granted(&staging)
                .lookup(sandbox.lookup(&env));
            let spawner = GuardedSpawner::new(
                LocalSpawner::new().files(&staging),
                staging.to_string_lossy(),
            );
            let mut request = ApplyRequest::new(plan, staging.to_string_lossy());
            request.environment = Secret::new(env);
            let watch = Watch::new(sandbox.home())
                .tree(sandbox.root())
                .skip(&staging)
                .registry(cfg!(windows));
            let machine = REAL_WORLD
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let before = Footprint::take(&watch);
            let answer = self.store.apply(
                request,
                spawner.into_object(),
                tools.into_object(),
                SuitePrompter::default().into_object(),
            );
            // A store that writes where it should not does so here too: put it back as the measured applies do.
            let after = Footprint::take(&watch);
            let changed = before.diff(&after);
            let _ = before.restore_registry(&after, &changed);
            drop(machine);
            match answer {
                Err(error)
                    if error.status == Status::GUATIAO_ERR_BAD_VALUE
                        && error.message().contains("plan changed") => {}
                Err(error) => self.fail(
                    "apply.stale_plan",
                    format!("answered {:?} {}", error.status, error.message()),
                ),
                Ok(_) => self.fail("apply.stale_plan", "a plan whose facts moved was applied"),
            }
        }
        // Cancelled at the first prompter call: GONE, within two seconds.
        if let Some((applied, _, _)) = self.apply_with(Variant::Normal, |_, prompter, _| {
            prompter.cancel_at_first_call = true
        }) {
            match &applied.result {
                Err(error) if error.status == Status::GUATIAO_ERR_GONE => {}
                Err(error) => self.fail("apply.cancel", format!("answered {:?}", error.status)),
                Ok(_) => self.fail("apply.cancel", "an install the person cancelled went on"),
            }
            if applied.elapsed > Duration::from_secs(2) {
                self.fail(
                    "apply.cancel",
                    format!("took {:?} to stop", applied.elapsed),
                );
            }
        }
        // A timeout shorter than the install: GONE.
        if let Some((applied, _, _)) = self.apply_with(Variant::Slow, |_, _, timeout| {
            *timeout = Duration::from_millis(300)
        }) {
            match &applied.result {
                Err(error) if error.status == Status::GUATIAO_ERR_GONE => {}
                Err(error) => self.fail(
                    "apply.timeout",
                    format!("answered {:?} {}", error.status, error.message()),
                ),
                Ok(_) => self.fail("apply.timeout", "an install past its timeout went on"),
            }
            if applied.elapsed > Duration::from_secs(3) {
                self.fail(
                    "apply.timeout",
                    format!("took {:?} to stop", applied.elapsed),
                );
            }
        }
    }
}

/// `SuitePrompter` behind an `Arc`, so the suite reads what it heard after the store has dropped its handle.
struct ArcPrompter(Arc<SuitePrompter>);

impl InstallPrompter for ArcPrompter {
    fn say(&self, line: &str) -> Result<(), ProviderError> {
        self.0.say(line)
    }

    fn progress(&self, progress: Progress) -> Result<(), ProviderError> {
        self.0.progress(progress)
    }

    fn ask(&self, form: LoginForm) -> Result<LoginAnswer, ProviderError> {
        self.0.ask(form)
    }

    fn cancelled(&self) -> Result<bool, ProviderError> {
        self.0.cancelled()
    }
}

/// A fixture read from a JSON file, for a store in a library the suite has no Rust code for.
///
/// ```json
/// { "old": "1.0.0", "new": "2.0.0", "auto_update_var": "NAME or null",
///   "platform": {"os": "linux", "arch": "x86_64"},
///   "world": {
///     "https://example.test/latest": {"text": "2.0.0"},
///     "https://example.test/2.0.0/x.sha256": {"text": "{{sha256:https://example.test/2.0.0/x}}"},
///     "https://example.test/2.0.0/x": {"standin": {"line": "x 2.0.0", "exit": 0, "auto_update_var": "NAME"}},
///     "https://example.test/doc": {"file": "relative/path"} } }
/// ```
///
/// `{{sha256:URL}}` and `{{sha512:URL}}` in a text stand for the digest of the bytes served at `URL`. The other worlds are
/// made from this one: a mismatch serves the file with a byte more than its digest says; a moved world serves different bytes
/// and a digest that fits them; a program that will not run exits 7; a failing download refuses every file over 4 KiB.
#[derive(Debug)]
pub struct FileFixture {
    standin: PathBuf,
    base: PathBuf,
    doc: serde_json::Value,
}

impl FileFixture {
    /// Reads `path`, with `standin` the built stand-in program.
    ///
    /// # Errors
    ///
    /// The reason the file cannot be read as a fixture.
    pub fn load(path: &Path, standin: PathBuf) -> Result<Self, String> {
        let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let doc: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        if !doc["world"].is_object()
            || doc["new"].as_str().is_none()
            || doc["old"].as_str().is_none()
        {
            return Err("the fixture needs `old`, `new` and a `world` object".to_owned());
        }
        Ok(Self {
            standin,
            base: path.parent().map(Path::to_path_buf).unwrap_or_default(),
            doc,
        })
    }
}

/// `{{kind:URL}}` markers in `text`: `(start, end, kind, url)`.
fn markers(text: &str) -> Vec<(usize, usize, String, String)> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(open) = text[from..].find("{{") {
        let start = from + open;
        let Some(close) = text[start..].find("}}") else {
            break;
        };
        let inner = &text[start + 2..start + close];
        if let Some((kind, url)) = inner.split_once(':')
            && (kind == "sha256" || kind == "sha512")
        {
            found.push((start, start + close + 2, kind.to_owned(), url.to_owned()));
        }
        from = start + close + 2;
    }
    found
}

fn digest(kind: &str, bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256, Sha512};
    if kind == "sha512" {
        format!("{:x}", Sha512::digest(bytes))
    } else {
        format!("{:x}", Sha256::digest(bytes))
    }
}

impl Fixture for FileFixture {
    fn older(&self) -> String {
        self.doc["old"].as_str().unwrap_or("1.0.0").to_owned()
    }

    fn newer(&self) -> String {
        self.doc["new"].as_str().unwrap_or("2.0.0").to_owned()
    }

    fn platform(&self) -> Platform {
        let p = &self.doc["platform"];
        match (p["os"].as_str(), p["arch"].as_str()) {
            (Some(os), Some(arch)) => Platform::new(os, arch),
            _ => Platform::new(std::env::consts::OS, std::env::consts::ARCH),
        }
    }

    fn auto_update_var(&self) -> Option<String> {
        self.doc["auto_update_var"].as_str().map(str::to_owned)
    }

    fn world(&self, variant: Variant) -> World {
        let entries = self.doc["world"].as_object().cloned().unwrap_or_default();
        let mut bytes: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        let mut texts: BTreeMap<String, String> = BTreeMap::new();
        for (url, spec) in &entries {
            if let Some(text) = spec["text"].as_str() {
                texts.insert(url.clone(), text.to_owned());
            } else if spec["standin"].is_object() {
                let s = &spec["standin"];
                let exit = if variant == Variant::WillNotRun {
                    7
                } else {
                    u32::try_from(s["exit"].as_u64().unwrap_or(0)).unwrap_or(0)
                };
                bytes.insert(
                    url.clone(),
                    standin_bytes(
                        &self.standin,
                        s["line"].as_str().unwrap_or(""),
                        exit,
                        s["auto_update_var"].as_str(),
                    ),
                );
            } else if let Some(file) = spec["file"].as_str() {
                bytes.insert(
                    url.clone(),
                    fs::read(self.base.join(file)).unwrap_or_default(),
                );
            }
        }
        let referenced: BTreeSet<String> = texts
            .values()
            .flat_map(|t| markers(t).into_iter().map(|(_, _, _, url)| url))
            .collect();
        let originals = bytes.clone();
        for url in &referenced {
            if let Some(served) = bytes.get_mut(url) {
                match variant {
                    Variant::Mismatch => served.extend_from_slice(b"tampered"),
                    Variant::Moved => served.extend_from_slice(b"republished"),
                    _ => {}
                }
            }
        }
        let mut world = World::new();
        for (url, text) in &texts {
            let mut out = String::new();
            let mut at = 0;
            for (start, end, kind, target) in markers(text) {
                out.push_str(&text[at..start]);
                let source = if variant == Variant::Mismatch {
                    originals.get(&target)
                } else {
                    bytes.get(&target)
                };
                out.push_str(&source.map(|b| digest(&kind, b)).unwrap_or_default());
                at = end;
            }
            out.push_str(&text[at..]);
            world.put(url.clone(), out.into_bytes());
        }
        for (url, served) in bytes {
            world.put(url, served);
        }
        if variant == Variant::DownloadFails {
            world.fail_big(4096);
        }
        if variant == Variant::Slow {
            world.slow();
        }
        world
    }
}
