//! [`Programs`]: what the host does with a store: asks it, shows the person its plan, and carries the plan out in a folder it
//! owns, checks that nothing outside it changed unannounced, and only then switches to the new copy.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files, paths and the environment are std's"
)]

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use canoa::store::{
    Action, ApplyRequest, DEFAULT_APPLY_TIMEOUT_MS, ELEVATION, InstallPlan, InstallPrompter,
    InstallTools, LatestQuery, LatestReport, ManagedCopy, PlanRequest, Platform, ProgramQuery,
    ProgramReport, ProgramStore, Progress, StoreInfo, StoreSettings,
};
use coqui_acp_api::guatiao::library::{ProviderError, Registry};
use coqui_acp_api::guatiao::{Status, Value};
use coqui_acp_api::{Environment, LoginAnswer, LoginForm, ProcessSpawner};
use coqui_acp_harness::LocalSpawner;

use super::exec::{GuardedSpawner, is_credential_name};
use super::footprint::{Difference, Footprint, Watch, uncovered};
use super::shelf::{Current, FileRecord, MadeBy, Shelf, ShelfError};
use super::tools::{HttpTransport, LocalTools, Lookup, Transport};

/// Why an operation on a program did not happen.
#[derive(Debug)]
#[non_exhaustive]
pub enum StoreError {
    /// No store has this id.
    Unknown(String),
    /// The store's settings were refused.
    Settings(String),
    /// The store refused, or failed.
    Store(ProviderError),
    /// The folder refused: held, not private, or a copy that does not check.
    Shelf(ShelfError),
    /// The host runs with elevated rights and was not allowed to install.
    Elevated,
    /// The install changed what its plan did not declare; the version folder was removed.
    Undeclared {
        /// Each change, named.
        items: Vec<Difference>,
        /// How many registry values were put back.
        restored: usize,
    },
    /// The store's answer does not hold what the host checks.
    Contract(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(id) => write!(f, "no program store {id}"),
            Self::Settings(why) => write!(f, "the store's settings were refused: {why}"),
            Self::Store(error) => write!(f, "{}", error.message()),
            Self::Shelf(error) => write!(f, "{error}"),
            Self::Elevated => f.write_str(
                "the host runs as an administrator or root, and installs only when told it may",
            ),
            Self::Undeclared { items, restored } => {
                f.write_str("the install changed what it did not declare:")?;
                for item in items {
                    write!(f, " {} ({});", item.what, item.kind)?;
                }
                write!(f, " {restored} registry values were put back")
            }
            Self::Contract(why) => write!(f, "the store broke its contract: {why}"),
        }
    }
}

impl core::error::Error for StoreError {}

impl From<ProviderError> for StoreError {
    fn from(error: ProviderError) -> Self {
        Self::Store(error)
    }
}

impl From<ShelfError> for StoreError {
    fn from(error: ShelfError) -> Self {
        Self::Shelf(error)
    }
}

/// One thing a running apply tells the person.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Event {
    /// A line of output.
    Say(String),
    /// How far a step has got.
    Progress(Progress),
}

/// How a person answers a question an install asks, and what receives its output.
pub trait Sink: Send + Sync {
    /// One thing the install said.
    fn event(&self, event: Event);

    /// A question; an error says why it could not be asked.
    ///
    /// # Errors
    ///
    /// Why no answer came.
    fn ask(&self, form: LoginForm) -> Result<LoginAnswer, String>;
}

/// A sink that keeps what it hears and answers no question.
#[derive(Debug, Default)]
pub struct Recorder {
    events: Mutex<Vec<Event>>,
}

impl Recorder {
    /// What it heard.
    #[must_use]
    pub fn events(&self) -> Vec<Event> {
        self.events.lock().map(|e| e.clone()).unwrap_or_default()
    }
}

impl Sink for Recorder {
    fn event(&self, event: Event) {
        if let Ok(mut events) = self.events.lock() {
            events.push(event);
        }
    }

    fn ask(&self, _form: LoginForm) -> Result<LoginAnswer, String> {
        Err("no one to ask".to_owned())
    }
}

/// The host's [`InstallPrompter`]: relays to a [`Sink`], and answers `GONE` once the person cancels.
#[must_use]
pub struct RelayPrompter {
    sink: Arc<dyn Sink>,
    cancel: Arc<AtomicBool>,
    lines: Arc<Mutex<Vec<String>>>,
}

impl fmt::Debug for RelayPrompter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RelayPrompter").finish_non_exhaustive()
    }
}

impl RelayPrompter {
    /// Relays to `sink`; `cancel` set makes every call answer `GONE`.
    pub fn new(sink: Arc<dyn Sink>, cancel: Arc<AtomicBool>) -> Self {
        Self {
            sink,
            cancel,
            lines: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn gone(&self) -> Result<(), ProviderError> {
        if self.cancel.load(Ordering::SeqCst) {
            return Err(ProviderError::new(Status::GUATIAO_ERR_GONE, "cancelled"));
        }
        Ok(())
    }
}

impl InstallPrompter for RelayPrompter {
    fn say(&self, line: &str) -> Result<(), ProviderError> {
        self.gone()?;
        let line = super::exec::clean(line);
        if let Ok(mut lines) = self.lines.lock() {
            lines.push(line.clone());
        }
        self.sink.event(Event::Say(line));
        Ok(())
    }

    fn progress(&self, progress: Progress) -> Result<(), ProviderError> {
        self.gone()?;
        self.sink.event(Event::Progress(progress));
        Ok(())
    }

    fn ask(&self, form: LoginForm) -> Result<LoginAnswer, ProviderError> {
        self.gone()?;
        self.sink
            .ask(form)
            .map_err(|why| ProviderError::new(Status::GUATIAO_ERR_UNSUPPORTED, &why))
    }

    fn cancelled(&self) -> Result<bool, ProviderError> {
        Ok(self.cancel.load(Ordering::SeqCst))
    }
}

/// What the host reads before it lets an install run.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Options {
    /// Lets an install run as an administrator or root, and a plan that needs one.
    pub allow_elevated: bool,
    /// Lets `http` reach a loopback address (a test's setting).
    pub loopback_http: bool,
    /// The environment installs and probes start from; the process's own, less every credential, by default.
    pub environment: Environment,
    /// The home whose profile files the footprint watches; the process's own by default.
    pub home: Option<PathBuf>,
    /// Whether the footprint reads the user's real registry keys (Windows).
    pub watch_registry: bool,
    /// How many versions to keep.
    pub keep_versions: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            allow_elevated: false,
            loopback_http: false,
            environment: process_environment(),
            home: std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                .map(PathBuf::from),
            watch_registry: cfg!(windows),
            keep_versions: super::shelf::DEFAULT_KEEP_VERSIONS,
        }
    }
}

/// This process's variables, without any that look like a credential.
#[must_use]
pub fn process_environment() -> Environment {
    std::env::vars()
        .filter(|(name, _)| !is_credential_name(name))
        .collect()
}

/// What a finished apply holds.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Applied {
    /// The copy the shelf now holds.
    pub current: Current,
    /// What the install said.
    pub lines: Vec<String>,
}

/// The programs this host keeps, through the stores a registry offers.
#[must_use]
pub struct Programs {
    shelf: Shelf,
    registry: Arc<Registry>,
    transport: Arc<dyn Transport>,
    options: Options,
}

impl fmt::Debug for Programs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Programs")
            .field("shelf", &self.shelf)
            .finish_non_exhaustive()
    }
}

impl Programs {
    /// The programs under `shelf`, through the stores `registry` offers, fetching over HTTPS.
    pub fn new(shelf: Shelf, registry: Arc<Registry>) -> Self {
        Self {
            shelf,
            registry,
            transport: Arc::new(HttpTransport::default()),
            options: Options::default(),
        }
    }

    /// Fetches through `transport`.
    pub fn transport(mut self, transport: Arc<dyn Transport>) -> Self {
        self.transport = transport;
        self
    }

    /// The options.
    pub fn options(mut self, options: Options) -> Self {
        self.shelf = self.shelf.keep_versions(options.keep_versions);
        self.options = options;
        self
    }

    /// The shelf.
    pub const fn shelf(&self) -> &Shelf {
        &self.shelf
    }

    /// The store offered under provider `id`, configured with `settings`.
    ///
    /// # Errors
    ///
    /// [`StoreError::Unknown`] for an id no store has, [`StoreError::Settings`] for settings its schema refuses.
    pub fn store(
        &self,
        id: &str,
        settings: &StoreSettings,
    ) -> Result<Arc<dyn ProgramStore>, StoreError> {
        use coqui_acp_api::guatiao::ToValue;
        let offer = self
            .registry
            .offer::<dyn ProgramStore>(id)
            .ok_or_else(|| StoreError::Unknown(id.to_owned()))?
            .map_err(|_| StoreError::Unknown(id.to_owned()))?;
        let config: Value = settings
            .to_value()
            .map_err(|e| StoreError::Settings(e.to_string()))?;
        let built = offer
            .instantiate(&config)
            .map_err(|e| StoreError::Settings(e.to_string()))?;
        Ok(Arc::new(built))
    }

    fn lookup(&self, environment: &Environment) -> Lookup {
        let path = environment
            .get("PATH")
            .or_else(|| environment.get("Path"))
            .unwrap_or("");
        Lookup {
            path: path.into(),
            home: self.options.home.clone(),
            variables: environment
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        }
    }

    fn tools(&self, info: &StoreInfo) -> LocalTools {
        LocalTools::new(Arc::clone(&self.transport), info.endpoints.clone())
            .lookup(self.lookup(&self.options.environment))
            .loopback_http(self.options.loopback_http)
    }

    fn managed(&self, info: &StoreInfo) -> Result<Option<ManagedCopy>, StoreError> {
        let dir = self.shelf.program(&info.program)?;
        Ok(dir
            .current()?
            .map(|c| ManagedCopy::new(c.dir.clone(), c.version.clone(), c.launch.launch())))
    }

    /// Every copy of the program the store can find, with no network and, unless `probe`, nothing run.
    ///
    /// # Errors
    ///
    /// What the store refuses.
    pub fn report(
        &self,
        store: &dyn ProgramStore,
        probe: bool,
    ) -> Result<ProgramReport, StoreError> {
        use coqui_acp_api::guatiao::Secret;
        let info = store.describe()?;
        let mut query = ProgramQuery::default();
        query.managed = self.managed(&info)?;
        query.environment = Secret::new(self.options.environment.clone());
        query.probe = probe;
        Ok(store.program(
            query,
            LocalSpawner::new().into_object(),
            self.tools(&info).into_object(),
        )?)
    }

    /// The newest version the store knows of.
    ///
    /// # Errors
    ///
    /// What the store refuses.
    pub fn latest(
        &self,
        store: &dyn ProgramStore,
        channel: Option<String>,
    ) -> Result<LatestReport, StoreError> {
        let info = store.describe()?;
        let mut query = LatestQuery::default();
        query.channel = channel;
        Ok(store.latest(query, self.tools(&info).into_object())?)
    }

    /// What an install, update or uninstall would do.
    ///
    /// # Errors
    ///
    /// What the store refuses.
    pub fn plan(
        &self,
        store: &dyn ProgramStore,
        action: Action,
        channel: Option<String>,
        version: Option<String>,
    ) -> Result<InstallPlan, StoreError> {
        let info = store.describe()?;
        let mut request = PlanRequest::new(
            action,
            Platform::new(std::env::consts::OS, std::env::consts::ARCH),
        );
        request.channel = channel;
        request.version = version;
        request.managed = self.managed(&info)?;
        Ok(store.plan(request, self.tools(&info).into_object())?)
    }

    /// Carries out `plan`, which the person confirmed, in a folder of its own; then switches to it.
    ///
    /// # Errors
    ///
    /// [`StoreError::Elevated`] when the host is elevated and not allowed to be; [`StoreError::Undeclared`] when the install
    /// changed what its plan did not declare (the folder is removed and `current.json` is as it was); what the store and the
    /// shelf refuse. A failure of any kind leaves the copy that was current as it was.
    pub fn apply(
        &self,
        provider: &str,
        settings: &StoreSettings,
        plan: &InstallPlan,
        sink: Arc<dyn Sink>,
        cancel: Arc<AtomicBool>,
    ) -> Result<Option<Applied>, StoreError> {
        let store = self.store(provider, settings)?;
        self.apply_store(store, provider, plan, sink, cancel)
    }

    /// [`apply`](Self::apply) for a store the caller already holds, recorded in `current.json` as `provider`.
    ///
    /// # Errors
    ///
    /// As [`apply`](Self::apply).
    pub fn apply_store(
        &self,
        store: Arc<dyn ProgramStore>,
        provider: &str,
        plan: &InstallPlan,
        sink: Arc<dyn Sink>,
        cancel: Arc<AtomicBool>,
    ) -> Result<Option<Applied>, StoreError> {
        let info = store.describe()?;
        if (!self.options.allow_elevated)
            && (running_elevated() || plan.changes.iter().any(|c| c.kind == ELEVATION))
        {
            return Err(StoreError::Elevated);
        }
        let program = self.shelf.program(&info.program)?;
        let lock = program.lock()?;
        let managed = self.managed(&info)?;
        if plan.action == Action::Uninstall {
            lock.remove()?;
            return Ok(None);
        }
        let version = plan
            .version
            .clone()
            .ok_or_else(|| StoreError::Contract("a plan with no version".to_owned()))?;
        if plan.downloads.is_empty() {
            // Already current: nothing to do and nothing to switch.
            return Ok(None);
        }
        let staging = lock.stage(&version)?;
        let result = self.run_apply(
            &*store, &info, provider, plan, managed, &staging, sink, cancel,
        );
        match result {
            Ok((outcome, records, lines, before, after)) => {
                let differences = before.diff(&after);
                let mut unexpected = uncovered(&differences, &plan.changes);
                if !unexpected.is_empty() {
                    // Compare once more a moment later: what is still changing is something else at work on the machine
                    // (an agent rewriting its own state), not the install, which has ended.
                    std::thread::sleep(core::time::Duration::from_millis(750));
                    let later = Footprint::take(&self.watch(&staging));
                    let moving = after.diff(&later);
                    unexpected.retain(|item| !moving.iter().any(|m| m.what == item.what));
                }
                if !unexpected.is_empty() {
                    lock.abandon(&staging);
                    let restored = before.restore_environment(&after);
                    return Err(StoreError::Undeclared {
                        items: unexpected,
                        restored,
                    });
                }
                let _ = fs::remove_dir_all(staging.join("home"));
                let _ = fs::remove_dir_all(staging.join("tmp"));
                if outcome.version.as_deref() != Some(version.as_str()) {
                    lock.abandon(&staging);
                    return Err(StoreError::Contract(format!(
                        "it read back {:?}, not {version}",
                        outcome.version
                    )));
                }
                let Some(launch) = outcome.launch else {
                    lock.abandon(&staging);
                    return Err(StoreError::Contract("no launch".to_owned()));
                };
                let current = lock
                    .commit(
                        &staging,
                        &version,
                        &launch,
                        records,
                        MadeBy {
                            provider: provider.to_owned(),
                            version: env!("CARGO_PKG_VERSION").to_owned(),
                        },
                    )
                    .map_err(StoreError::Shelf)?;
                Ok(Some(Applied { current, lines }))
            }
            Err(error) => {
                lock.abandon(&staging);
                Err(error)
            }
        }
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "one apply's inputs, named at the call"
    )]
    #[allow(clippy::type_complexity, reason = "what one apply returns")]
    fn run_apply(
        &self,
        store: &dyn ProgramStore,
        info: &StoreInfo,
        _provider: &str,
        plan: &InstallPlan,
        managed: Option<ManagedCopy>,
        staging: &Path,
        sink: Arc<dyn Sink>,
        cancel: Arc<AtomicBool>,
    ) -> Result<
        (
            canoa::store::ApplyOutcome,
            Vec<FileRecord>,
            Vec<String>,
            Footprint,
            Footprint,
        ),
        StoreError,
    > {
        use coqui_acp_api::guatiao::Secret;
        let staging_text = staging.to_string_lossy().into_owned();
        let environment = install_environment(&self.options.environment, staging);
        let watch = self.watch(staging);
        let before = Footprint::take(&watch);
        let spawner = GuardedSpawner::new(LocalSpawner::new().files(staging), staging_text.clone());
        let tools = self
            .tools(info)
            .granted(staging)
            .lookup(self.lookup(&environment))
            .cancelled_by(Arc::clone(&cancel))
            .deadline(super::exec::deadline_after(DEFAULT_APPLY_TIMEOUT_MS));
        let fetched = tools.fetch_log();
        let lines = Arc::new(Mutex::new(Vec::new()));
        let mut prompter = RelayPrompter::new(sink, cancel);
        prompter.lines = Arc::clone(&lines);
        let mut request = ApplyRequest::new(plan.clone(), staging_text);
        request.managed = managed;
        request.environment = Secret::new(environment);
        let outcome = store.apply(
            request,
            spawner.into_object(),
            tools.into_object(),
            prompter.into_object(),
        )?;
        let after = Footprint::take(&watch);
        let records = fetched
            .lock()
            .map(|f| {
                f.iter()
                    .map(|r| FileRecord {
                        path: r.path.clone(),
                        url: r.url.clone(),
                        sha256: r.sha256.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let lines = lines.lock().map(|l| l.clone()).unwrap_or_default();
        Ok((outcome, records, lines, before, after))
    }

    fn watch(&self, staging: &Path) -> Watch {
        let mut watch = Watch::new(self.options.home.clone().unwrap_or_default())
            .skip(self.shelf.root())
            .skip(staging)
            .registry(self.options.watch_registry);
        watch = watch.path_dirs(super::footprint::user_path_dirs(&std::ffi::OsString::from(
            self.options.environment.get("PATH").unwrap_or(""),
        )));
        watch
    }

    /// Swaps the current copy and the previous one.
    ///
    /// # Errors
    ///
    /// What the shelf refuses.
    pub fn rollback(&self, store: &dyn ProgramStore) -> Result<Current, StoreError> {
        let info = store.describe()?;
        let program = self.shelf.program(&info.program)?;
        let lock = program.lock()?;
        Ok(lock.rollback()?)
    }

    /// Removes the managed copy.
    ///
    /// # Errors
    ///
    /// What the shelf refuses.
    pub fn remove(&self, store: &dyn ProgramStore) -> Result<(), StoreError> {
        let info = store.describe()?;
        let program = self.shelf.program(&info.program)?;
        let lock = program.lock()?;
        Ok(lock.remove()?)
    }
}

/// The environment an install runs with: the base, with every home and temporary directory inside the staging folder.
#[must_use]
pub fn install_environment(base: &Environment, staging: &Path) -> Environment {
    let mut environment = base.clone();
    let home = staging.join("home");
    let temp = staging.join("tmp");
    let _ = fs::create_dir_all(&home);
    let _ = fs::create_dir_all(&temp);
    let (home, temp) = (
        home.to_string_lossy().into_owned(),
        temp.to_string_lossy().into_owned(),
    );
    for name in [
        "HOME",
        "USERPROFILE",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_CACHE_HOME",
        "XDG_STATE_HOME",
    ] {
        environment.insert(name, home.clone());
    }
    for name in ["APPDATA", "LOCALAPPDATA"] {
        environment.insert(name, home.clone());
    }
    for name in ["TEMP", "TMP", "TMPDIR"] {
        environment.insert(name, temp.clone());
    }
    environment
}

#[cfg(unix)]
fn running_elevated() -> bool {
    rustix::process::geteuid().is_root()
}

#[cfg(windows)]
fn running_elevated() -> bool {
    is_elevated::is_elevated()
}

#[cfg(not(any(unix, windows)))]
const fn running_elevated() -> bool {
    false
}

/// A store's id and what it says of itself, for a listing.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Listed {
    /// The provider id.
    pub provider: String,
    /// What `describe` answered.
    pub info: StoreInfo,
}

impl Programs {
    /// What each of the stores named says of itself.
    ///
    /// # Errors
    ///
    /// The first store that refuses to be described.
    pub fn list(&self, providers: &[&str]) -> Result<Vec<Listed>, StoreError> {
        providers
            .iter()
            .map(|id| {
                let store = self.store(id, &StoreSettings::default())?;
                Ok(Listed {
                    provider: (*id).to_owned(),
                    info: store.describe()?,
                })
            })
            .collect()
    }
}
