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
use core::time::Duration;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError, RwLock};
use std::time::Instant;

use canoa::store::models::ModelFile;
use canoa::store::{
    Action, ApplyRequest, Change, DEFAULT_APPLY_TIMEOUT_MS, ELEVATION, InstallPlan,
    InstallPrompter, InstallTools, LatestQuery, LatestReport, Launch, ManagedCopy, PATH,
    PROFILE_FILE, PlanRequest, Platform, ProgramQuery, ProgramReport, ProgramStore, Progress,
    REGISTRY, STATE_DIR, SYSTEM_PATH, StoreInfo, StoreSettings, USER_PATH,
};
use coqui_acp_api::guatiao::library::{ProviderError, Registry};
use coqui_acp_api::guatiao::{Status, Value};
use coqui_acp_api::{Environment, LoginAnswer, LoginForm, ProcessSpawner};
use coqui_acp_harness::LocalSpawner;
use sha2::{Digest, Sha256};

use super::footprint::{Difference, Footprint, Watch, is_profile_file, uncovered};
use super::shelf::{Current, FileRecord, MadeBy, Shelf, ShelfError};
use super::tools::{HttpTransport, LocalTools, Lookup, ToolsLimits, Transport};
use canoa::store::exec::{GuardedSpawner, is_credential_name};
use canoa::store::protect::{Protected, RECOMMENDED_PLACES, RECOMMENDED_VARIABLES};

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
        /// The values left as they are because the person changed them since.
        kept: Vec<String>,
    },
    /// The install failed after the plan declared changes: the host cannot say that they were not made.
    LeftBehind {
        /// Why the install failed.
        reason: String,
        /// What the plan declared.
        changes: Vec<Change>,
    },
    /// The store's answer does not hold what the host checks.
    Contract(String),
    /// The plan has downloads nothing checks, and the host was not told to take those.
    Unverified(Vec<String>),
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
            Self::Undeclared {
                items,
                restored,
                kept,
            } => {
                f.write_str("the install changed what it did not declare:")?;
                for item in items {
                    write!(f, " {} ({});", item.what, item.kind)?;
                }
                write!(f, " {restored} registry values were put back")?;
                if !kept.is_empty() {
                    write!(
                        f,
                        "; left as they are, the person's own change since: {}",
                        kept.join(", ")
                    )?;
                }
                f.write_str(". Something else writing at the same time looks the same: try again")
            }
            Self::LeftBehind { reason, changes } => {
                write!(f, "{reason}; what the plan declared may be left behind:")?;
                for change in changes {
                    write!(f, " {} {} ({});", change.kind, change.target, change.detail)?;
                }
                Ok(())
            }
            Self::Unverified(urls) => write!(
                f,
                "the publisher gives nothing to check these downloads against: {}; they are taken only when allowed",
                urls.join(", ")
            ),
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
        let line = canoa::store::exec::clean(line);
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
    /// The places a declared change may not be at, under or hold, and the folders it may only enter a place of its own in: lines
    /// in gitignore syntax (see [`protect`](canoa::store::protect)). canoa.cfg's `[host.store] protected`; the recommended list by default.
    pub protected: Vec<String>,
    /// The variables an install is never given and a launch never sets, in the same syntax, matched against a name.
    /// canoa.cfg's `[host.store] credential_variables`; the recommended list by default.
    pub credential_variables: Vec<String>,
    /// Whether the footprint reads the user's real registry keys (Windows).
    pub watch_registry: bool,
    /// Whether [`launch`](Programs::launch) reads every file of the managed copy against what was recorded when it was installed.
    /// canoa.cfg's `[host.store] verify_copies`; off by default, when only the launch file must exist. An install's own check of
    /// each download against its publisher's SHA-256 does not depend on it.
    pub verify_copies: bool,
    /// How many versions to keep.
    pub keep_versions: usize,
    /// How long the host goes on watching after a store's `apply` has returned, before it measures what changed: a change in that
    /// time is undeclared. A writer outside the apply that waits longer than this is not seen.
    pub settle: Duration,
    /// How long a plan the host made stays good.
    pub plan_ttl: Duration,
    /// How many plans the host keeps at once.
    pub plan_limit: usize,
    /// How many plans one caller keeps at once; its oldest goes first.
    pub plans_per_owner: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            allow_elevated: false,
            loopback_http: false,
            environment: process_environment(),
            home: std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                .map(PathBuf::from),
            protected: lines(RECOMMENDED_PLACES),
            credential_variables: lines(RECOMMENDED_VARIABLES),
            watch_registry: cfg!(windows),
            verify_copies: false,
            keep_versions: super::shelf::DEFAULT_KEEP_VERSIONS,
            settle: Duration::from_secs(2),
            plan_ttl: Duration::from_secs(15 * 60),
            plan_limit: 32,
            plans_per_owner: 8,
        }
    }
}

fn lines(list: &[&str]) -> Vec<String> {
    list.iter().map(|line| (*line).to_owned()).collect()
}

impl Options {
    /// The defaults, with the lists `config` names in `[host.store]` in place of the recommended ones, and the environment less the
    /// variables the list protects.
    #[must_use]
    pub fn from_config(config: &crate::config::Config) -> Self {
        let mut options = Self {
            verify_copies: config.store.verify_copies.unwrap_or(false),
            ..Self::default()
        };
        if let Some(list) = &config.store.protected {
            options.protected.clone_from(list);
        }
        if let Some(list) = &config.store.credential_variables {
            options.credential_variables.clone_from(list);
            let protect = Protected::new(list);
            options.environment = std::env::vars()
                .filter(|(name, _)| !protect.protects_name(name))
                .collect();
        }
        options
    }
}

/// This process's variables, without any that look like a credential.
#[must_use]
pub fn process_environment() -> Environment {
    std::env::vars()
        .filter(|(name, _)| !is_credential_name(name))
        .collect()
}

/// What one run of a store's `apply` left: its answer, and the machine before and after.
struct Ran {
    result: Result<canoa::store::ApplyOutcome, ProviderError>,
    records: Vec<FileRecord>,
    lines: Vec<String>,
    before: Footprint,
    after: Footprint,
}

/// What a store's launch has to look like before the host records it: the files it names lie inside the version's folder,
/// since the host verifies that copy and runs nothing outside it. The variables it sets are the store's own and are not
/// checked.
fn check_launch_shape(launch: &Launch, staging: &Path) -> Result<(), StoreError> {
    let settings = super::shelf::LaunchRecord::of(launch).settings;
    for key in ["executable", "node", "script"] {
        if let Some(path) = settings.get(key).and_then(serde_json::Value::as_str) {
            let inside = canoa::store::exec::normalize(Path::new(path))
                .zip(canoa::store::exec::normalize(staging))
                .is_some_and(|(path, staging)| path.starts_with(staging));
            if !inside {
                return Err(StoreError::Contract(format!(
                    "its launch setting {key} names a file outside the version's folder"
                )));
            }
        }
    }
    Ok(())
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

/// What reads and merges the releases catalogs of a program's scope, by the program's id: the text of a `canoa.releases/1`
/// document, or why none.
pub(crate) type Releases = Arc<dyn Fn(&str) -> Result<String, String> + Send + Sync>;

/// The programs this host keeps, through the stores a registry offers.
#[must_use]
pub struct Programs {
    shelf: Shelf,
    registry: Arc<Registry>,
    /// The registries of extensions' libraries, asked for a store the first one does not offer.
    extras: Mutex<Vec<Arc<Registry>>>,
    transport: Arc<dyn Transport>,
    options: Options,
    /// The programs whose scope has a catalog entry that says `trusted = true` (`[store.<program>.catalogs.<label>]`): an install of
    /// one may take a download nothing checks. The plan names each such download, and `current.json` records that a file was not
    /// verified.
    trusted: RwLock<BTreeSet<String>>,
    releases: RwLock<Option<Releases>>,
    /// How long a lookup waits to connect and for an answer; `[host.store] connect_timeout` and `lookup_timeout`.
    lookups: Mutex<(Duration, Duration)>,
    /// The plans this host made, by token: it applies its own copy of a plan, never one a client sends.
    plans: Mutex<Vec<StoredPlan>>,
}

/// A plan the host made, for the provider whose store made it.
struct StoredPlan {
    /// Who asked for it: only that caller applies it.
    owner: String,
    provider: String,
    made: Instant,
    plan: InstallPlan,
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
        crate::vendors::note(&registry);
        Self {
            shelf,
            registry,
            extras: Mutex::new(Vec::new()),
            transport: Arc::new(HttpTransport::default()),
            options: Options::default(),
            trusted: RwLock::new(BTreeSet::new()),
            releases: RwLock::new(None),
            lookups: Mutex::new((
                ToolsLimits::default().lookup_connect,
                ToolsLimits::default().lookup_response,
            )),
            plans: Mutex::new(Vec::new()),
        }
    }

    /// Also offers the stores of `registry`, which an extension's library was loaded into; a store the first registry offers
    /// is never replaced by one of these.
    pub fn add_registry(&self, registry: Arc<Registry>) {
        crate::vendors::note(&registry);
        self.extras
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(registry);
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
        if crate::disabled::off::<dyn ProgramStore>(id) {
            return Err(StoreError::Unknown(id.to_owned()));
        }
        let extras = self
            .extras
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let offer = core::iter::once(&self.registry)
            .chain(&extras)
            .find_map(|registry| registry.offer::<dyn ProgramStore>(id))
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

    /// The name of the library that offers the store `id`, when a registry says.
    #[must_use]
    pub fn library_of(&self, id: &str) -> Option<String> {
        let extras = self
            .extras
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        core::iter::once(&self.registry)
            .chain(&extras)
            .find_map(|registry| registry.offer::<dyn ProgramStore>(id))?
            .ok()?
            .library()
            .map(str::to_owned)
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

    /// Waits `connect` to connect and `response` for an answer when a store reads a document (a release list, a checksum file), and
    /// keeps what publishers say of them under `cache` so a later run asks whether it changed.
    pub fn lookups(&self, connect: Duration, response: Duration, cache: Option<&Path>) {
        *self.lookups.lock().unwrap_or_else(PoisonError::into_inner) = (connect, response);
        if let Some(cache) = cache {
            self.transport.keep_in(cache);
        }
    }

    /// Takes `programs` as the ones whose installs may take a download nothing checks, in place of the ones named before: the
    /// programs whose scope in canoa.cfg has a trusted catalog entry.
    pub fn trust(&self, programs: impl IntoIterator<Item = String>) {
        *self.trusted.write().unwrap_or_else(PoisonError::into_inner) =
            programs.into_iter().collect();
    }

    fn is_trusted(&self, program: &str) -> bool {
        self.trusted
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(program)
    }

    /// The transport every fetch of this store goes through, which a source's `fetch` shares.
    pub(crate) fn shared_transport(&self) -> Arc<dyn Transport> {
        Arc::clone(&self.transport)
    }

    /// Takes `reader` as what reads the catalogs of a program's scope, by the program's id, in place of the one named before.
    pub(crate) fn read_releases_with(&self, reader: Releases) {
        *self
            .releases
            .write()
            .unwrap_or_else(PoisonError::into_inner) = Some(reader);
    }

    fn tools(&self, info: &StoreInfo) -> LocalTools {
        let reader = info
            .catalogs
            .as_ref()
            .and(
                self.releases
                    .read()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone(),
            )
            .map(|reader| {
                let program = info.program.clone();
                let read: super::tools::ReleasesReader = Arc::new(move || reader(&program));
                read
            });
        self.tools_at(&info.endpoints)
            .allow_unverified(self.is_trusted(&info.program))
            .releases_from(reader)
    }

    fn tools_at(&self, endpoints: &[String]) -> LocalTools {
        let (lookup_connect, lookup_response) =
            *self.lookups.lock().unwrap_or_else(PoisonError::into_inner);
        let limits = ToolsLimits {
            lookup_connect,
            lookup_response,
            ..ToolsLimits::default()
        };
        LocalTools::new(Arc::clone(&self.transport), endpoints.to_vec())
            .limits(limits)
            .lookup(self.lookup(&self.options.environment))
            .loopback_http(self.options.loopback_http)
    }

    fn managed(&self, info: &StoreInfo) -> Result<Option<ManagedCopy>, StoreError> {
        let dir = self.shelf.program(&info.program)?;
        Ok(dir
            .current()?
            .map(|c| ManagedCopy::new(c.dir.clone(), c.version.clone(), c.launch.launch())))
    }

    /// Every copy of the program the store can find, with no network and, unless `probe`, nothing run. With `probe` the host's own
    /// copy is run, and of the others only `pick`, the one the person chose.
    ///
    /// # Errors
    ///
    /// What the store refuses.
    pub fn report(
        &self,
        store: &dyn ProgramStore,
        probe: bool,
        pick: Option<&str>,
    ) -> Result<ProgramReport, StoreError> {
        use coqui_acp_api::guatiao::Secret;
        let info = store.describe()?;
        let mut query = ProgramQuery::default();
        query.managed = self.managed(&info)?;
        query.environment = Secret::new(self.options.environment.clone());
        query.probe = probe;
        query.probe_copy = pick.map(str::to_owned);
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

    /// What an install, update or uninstall would do. The host keeps the plan it is given, for `provider` and by its token, and
    /// applies that copy: what a client sends back only names it.
    ///
    /// # Errors
    ///
    /// What the store refuses.
    pub fn plan(
        &self,
        provider: &str,
        store: &dyn ProgramStore,
        action: Action,
        channel: Option<String>,
        version: Option<String>,
    ) -> Result<InstallPlan, StoreError> {
        self.plan_for("", provider, store, action, channel, version)
    }

    /// As [`plan`](Self::plan), kept for `owner`: only a caller that names the same owner applies it, and one owner keeps at most
    /// [`Options::plans_per_owner`] plans, so a caller cannot push out another one's.
    ///
    /// # Errors
    ///
    /// What the store refuses.
    pub fn plan_for(
        &self,
        owner: &str,
        provider: &str,
        store: &dyn ProgramStore,
        action: Action,
        channel: Option<String>,
        version: Option<String>,
    ) -> Result<InstallPlan, StoreError> {
        self.plan_kept(Some(owner), provider, store, action, channel, version)
    }

    /// As [`plan`](Self::plan), for a caller that only reads what an apply would do: the host keeps nothing, so it neither applies
    /// the plan nor lets it replace or push out one it holds for someone else.
    ///
    /// # Errors
    ///
    /// What the store refuses.
    pub fn plan_to_read(
        &self,
        provider: &str,
        store: &dyn ProgramStore,
        action: Action,
        channel: Option<String>,
        version: Option<String>,
    ) -> Result<InstallPlan, StoreError> {
        self.plan_kept(None, provider, store, action, channel, version)
    }

    fn plan_kept(
        &self,
        keep: Option<&str>,
        provider: &str,
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
        let managed = self.managed(&info)?;
        request.managed.clone_from(&managed);
        let mut plan = store.plan(request, self.tools(&info).into_object())?;
        plan.from_version = managed.as_ref().map(|m| m.version.clone());
        if plan.token.is_empty() && plan.action == Action::Uninstall {
            let mut nonce = [0u8; 16];
            getrandom::fill(&mut nonce).map_err(|e| StoreError::Contract(e.to_string()))?;
            plan.token = data_encoding::HEXLOWER.encode(&Sha256::digest(
                [provider.as_bytes(), b"|uninstall|", &nonce]
                    .concat()
                    .as_slice(),
            ));
        }
        if let Some(owner) = keep
            && !plan.token.is_empty()
        {
            self.keep_plan(owner, provider, &plan);
        }
        Ok(plan)
    }

    fn keep_plan(&self, owner: &str, provider: &str, plan: &InstallPlan) {
        if let Ok(mut plans) = self.plans.lock() {
            let now = Instant::now();
            plans.retain(|p| now.duration_since(p.made) < self.options.plan_ttl);
            // The same plan again replaces the caller's own, never another caller's.
            plans.retain(|p| {
                !(p.owner == owner && p.provider == provider && p.plan.token == plan.token)
            });
            let own = self.options.plans_per_owner.max(1);
            while plans.iter().filter(|p| p.owner == owner).count() >= own {
                if let Some(oldest) = plans.iter().position(|p| p.owner == owner) {
                    plans.remove(oldest);
                }
            }
            plans.push(StoredPlan {
                owner: owner.to_owned(),
                provider: provider.to_owned(),
                made: now,
                plan: plan.clone(),
            });
            // Past the whole host's limit, the caller that made this plan gives up its own oldest first.
            while plans.len() > self.options.plan_limit.max(1) {
                let at = plans.iter().position(|p| p.owner == owner).unwrap_or(0);
                plans.remove(at);
            }
        }
    }

    /// The plan made for `provider` under `token`, taken out so that it is applied once.
    fn take_plan(&self, owner: &str, provider: &str, token: &str) -> Option<InstallPlan> {
        let mut plans = self.plans.lock().ok()?;
        let now = Instant::now();
        plans.retain(|p| now.duration_since(p.made) < self.options.plan_ttl);
        let at = plans
            .iter()
            .position(|p| p.owner == owner && p.provider == provider && p.plan.token == token)?;
        Some(plans.remove(at).plan)
    }

    /// Whether `path` is, is inside or holds a place on the protected list (`[host.store] protected`). A place that cannot be
    /// worked out counts as protected.
    #[must_use]
    pub fn protects_place(&self, path: &Path) -> bool {
        let protected = Protected::new(&self.options.protected);
        let home = self
            .options
            .home
            .as_deref()
            .and_then(canoa::store::exec::normalize);
        let Some(place) = canoa::store::exec::normalize(path) else {
            return true;
        };
        protected.protects(&place, home.as_deref()) || protected.holds(&place, home.as_deref())
    }

    /// The version the managed copy would go back to, if it has a previous one.
    ///
    /// # Errors
    ///
    /// What the store or the shelf refuses.
    pub fn previous_version(&self, store: &dyn ProgramStore) -> Result<Option<String>, StoreError> {
        let info = store.describe()?;
        let program = self.shelf.program(&info.program)?;
        Ok(program
            .current()?
            .and_then(|current| current.previous)
            .map(|previous| previous.version))
    }

    /// The plan this host holds for `owner` under `token`, as it would be applied, left in place.
    #[must_use]
    pub fn kept_plan(&self, owner: &str, provider: &str, token: &str) -> Option<InstallPlan> {
        let plans = self.plans.lock().ok()?;
        let now = Instant::now();
        plans
            .iter()
            .find(|p| {
                p.owner == owner
                    && p.provider == provider
                    && p.plan.token == token
                    && now.duration_since(p.made) < self.options.plan_ttl
            })
            .map(|p| p.plan.clone())
    }

    /// Gives up the plan this host holds for `owner` under `token`, if it holds one.
    pub fn forget_plan(&self, owner: &str, provider: &str, token: &str) {
        let _ = self.take_plan(owner, provider, token);
    }

    /// The version of the copy the host manages for the program `store` offers, if it has one.
    ///
    /// # Errors
    ///
    /// What the store or the shelf refuses.
    pub fn managed_version(&self, store: &dyn ProgramStore) -> Result<Option<String>, StoreError> {
        let info = store.describe()?;
        Ok(self.managed(&info)?.map(|managed| managed.version))
    }

    /// Whether an install, update or removal holds the shelf now, in this process or another.
    #[must_use]
    pub fn installing(&self) -> bool {
        matches!(self.shelf.apply_lock(), Err(ShelfError::Held))
    }

    /// The start of the managed copy as `current.json` names it, with nothing checked and nothing hashed: what a listing needs to
    /// know where the program is. Whatever runs the copy goes through [`launch`](Self::launch).
    ///
    /// # Errors
    ///
    /// What the store or the shelf refuses.
    pub fn managed_launch(&self, store: &dyn ProgramStore) -> Result<Option<String>, StoreError> {
        let info = store.describe()?;
        Ok(self.managed(&info)?.map(|managed| managed.launch.path))
    }

    /// The start of the managed copy, after `current.json` is checked against the digest kept outside the program's folder, the launch
    /// is found to lie inside the copy and the copy inside the programs folder and, with [`Options::verify_copies`], every file of the
    /// copy is checked against what was recorded when it was installed (a file added since, one gone, one changed). Reading every file
    /// costs time in proportion to the copy: it is for the moment before a session starts. Without it the launch file must exist.
    ///
    /// # Errors
    ///
    /// [`StoreError::Shelf`] when any of that does not hold, or there is no managed copy.
    pub fn launch(&self, store: &dyn ProgramStore) -> Result<Launch, StoreError> {
        let info = store.describe()?;
        let program = self.shelf.program(&info.program)?;
        let current = program
            .current_checked()?
            .ok_or(StoreError::Shelf(ShelfError::Nothing("there is no copy")))?;
        let launch = current.launch.launch();
        let inside = |path: &Path, root: &Path| {
            canoa::store::exec::normalize(path)
                .zip(canoa::store::exec::normalize(root))
                .is_some_and(|(path, root)| path.starts_with(root))
        };
        if !inside(Path::new(&current.dir), self.shelf.root())
            || !inside(Path::new(&launch.path), Path::new(&current.dir))
        {
            return Err(StoreError::Shelf(ShelfError::Refused(
                "the launch is not inside the managed copy".to_owned(),
            )));
        }
        if self.options.verify_copies {
            program.verify(&current)?;
        } else if !Path::new(&launch.path).is_file() {
            return Err(StoreError::Shelf(ShelfError::Refused(
                "the launch file of the managed copy is gone".to_owned(),
            )));
        }
        Ok(launch)
    }

    /// Refuses a plan that declares a change broader than one program's own places: the system's places a login or a tool reads,
    /// the home or what holds it, a registry key outside the few a program may have.
    fn check_changes(&self, plan: &InstallPlan, program: &str) -> Result<(), StoreError> {
        let refuse = |change: &Change, why: &str| {
            StoreError::Contract(format!(
                "the plan declares a change of {} ({}) {why}",
                change.target, change.kind
            ))
        };
        let protected = Protected::new(&self.options.protected);
        for change in &plan.changes {
            match change.kind.as_str() {
                PATH | STATE_DIR | PROFILE_FILE | USER_PATH => {
                    let Some(home) = &self.options.home else {
                        return Err(refuse(change, "with no home to place it in"));
                    };
                    if !Path::new(&change.target).is_absolute() {
                        return Err(refuse(change, "that is not an absolute path"));
                    }
                    let parts = parts_under_home(&change.target, home)
                        .ok_or_else(|| refuse(change, "that is not under the home"))?;
                    if parts.is_empty() {
                        return Err(refuse(change, "that is the home itself"));
                    }
                    let place = canoa::store::exec::normalize(Path::new(&change.target));
                    let at_home = canoa::store::exec::normalize(home);
                    let (Some(place), Some(at_home)) = (place, at_home) else {
                        return Err(refuse(change, "that cannot be placed"));
                    };
                    if protected.protects(&place, Some(&at_home)) {
                        return Err(refuse(
                            change,
                            "that is, or is inside, a place on the protected list ([host.store] protected)",
                        ));
                    }
                    if protected.holds(&place, Some(&at_home)) {
                        return Err(refuse(
                            change,
                            "that holds a place on the protected list ([host.store] protected)",
                        ));
                    }
                    match change.kind.as_str() {
                        STATE_DIR if !named_for(&parts, program) => {
                            return Err(refuse(
                                change,
                                "that is not a folder named for the program",
                            ));
                        }
                        PROFILE_FILE if !is_profile_file(home, &change.target) => {
                            return Err(refuse(change, "that is not a profile file"));
                        }
                        _ => {}
                    }
                }
                REGISTRY if !registry_allowed(&change.target) => {
                    return Err(refuse(change, "that is not a key a program may declare"));
                }
                _ => {}
            }
        }
        Ok(())
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
        self.apply_store_for("", store, provider, plan, sink, cancel)
    }

    /// [`apply_store`](Self::apply_store) of a plan made for `owner` by [`plan_for`](Self::plan_for).
    ///
    /// # Errors
    ///
    /// As [`apply`](Self::apply).
    pub fn apply_store_for(
        &self,
        owner: &str,
        store: Arc<dyn ProgramStore>,
        provider: &str,
        plan: &InstallPlan,
        sink: Arc<dyn Sink>,
        cancel: Arc<AtomicBool>,
    ) -> Result<Option<Applied>, StoreError> {
        let info = store.describe()?;
        // The plan applied is the host's own copy, found by its provider and token, and used once.
        if plan.token.is_empty() && plan.action != Action::Uninstall {
            // An update to the version already current: nothing to do and nothing to switch.
            return Ok(None);
        }
        let made = self
            .take_plan(owner, provider, &plan.token)
            .ok_or_else(|| {
                StoreError::Contract(
                    "this host made no such plan, or it has expired: plan again".to_owned(),
                )
            })?;
        let plan = &made;
        self.check_changes(plan, &info.program)?;
        if (!self.options.allow_elevated)
            && (running_elevated()
                || plan
                    .changes
                    .iter()
                    .any(|c| c.kind == ELEVATION || c.kind == SYSTEM_PATH))
        {
            return Err(StoreError::Elevated);
        }
        let unchecked: Vec<String> = plan
            .downloads
            .iter()
            .filter(|d| d.is_unverified())
            .map(|d| d.url.clone())
            .collect();
        if !unchecked.is_empty() && !self.is_trusted(&info.program) {
            return Err(StoreError::Unverified(unchecked));
        }
        // One install at a time on the shelf: two measured together would each see the other's changes.
        let _one_at_a_time = self.shelf.apply_lock()?;
        let program = self.shelf.program(&info.program)?;
        let lock = program.lock()?;
        let managed = self.managed(&info)?;
        if plan.from_version != managed.as_ref().map(|m| m.version.clone()) {
            return Err(StoreError::Contract(
                "the copy was installed, updated or removed since this plan was made: plan again"
                    .to_owned(),
            ));
        }
        if plan.action == Action::Uninstall {
            lock.remove()?;
            return Ok(None);
        }
        let version = plan
            .version
            .clone()
            .ok_or_else(|| StoreError::Contract("a plan with no version".to_owned()))?;
        if plan.downloads.is_empty() {
            return Ok(None);
        }
        let staging = lock.stage(&version)?;
        let ran = self.run_apply(&*store, &info, plan, managed, &staging, sink, cancel);
        // Whatever the install did, it is measured: a failed, cancelled or timed-out one may have changed the machine too.
        let differences = ran.before.diff(&ran.after);
        let unexpected = uncovered(&differences, &plan.changes);
        if !unexpected.is_empty() {
            lock.abandon(&staging);
            let done = ran.before.restore_registry(&ran.after, &unexpected);
            return Err(StoreError::Undeclared {
                items: unexpected,
                restored: done.restored,
                kept: done.left,
            });
        }
        let outcome = match ran.result {
            Ok(outcome) => outcome,
            Err(error) => {
                lock.abandon(&staging);
                if plan.changes.is_empty() {
                    return Err(StoreError::Store(error));
                }
                return Err(StoreError::LeftBehind {
                    reason: error.message().to_owned(),
                    changes: plan.changes.clone(),
                });
            }
        };
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
        if let Err(error) = check_launch_shape(&launch, &staging) {
            lock.abandon(&staging);
            return Err(error);
        }
        // The whole copy is recorded, the files an archive unpacked into it too, so a launch can find what changed.
        let mut records = ran.records;
        if let Err(error) = super::shelf::record_tree(&staging, &mut records) {
            lock.abandon(&staging);
            return Err(StoreError::Shelf(error));
        }
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
        Ok(Some(Applied {
            current,
            lines: ran.lines,
        }))
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "one apply's inputs, named at the call"
    )]
    fn run_apply(
        &self,
        store: &dyn ProgramStore,
        info: &StoreInfo,
        plan: &InstallPlan,
        managed: Option<ManagedCopy>,
        staging: &Path,
        sink: Arc<dyn Sink>,
        cancel: Arc<AtomicBool>,
    ) -> Ran {
        use coqui_acp_api::guatiao::Secret;
        let staging_text = staging.to_string_lossy().into_owned();
        let environment = install_environment(&self.options.environment, staging);
        let watch = self.watch(staging);
        let before = Footprint::take(&watch);
        let spawner = GuardedSpawner::new(LocalSpawner::new().files(staging), staging_text.clone())
            .credentials(Arc::new(Protected::new(&self.options.credential_variables)));
        let tools = self
            .tools(info)
            .granted(staging)
            .lookup(self.lookup(&environment))
            .cancelled_by(Arc::clone(&cancel))
            .deadline(canoa::store::exec::deadline_after(DEFAULT_APPLY_TIMEOUT_MS));
        let fetched = tools.fetch_log();
        let lines = Arc::new(Mutex::new(Vec::new()));
        let mut prompter = RelayPrompter::new(sink, cancel);
        prompter.lines = Arc::clone(&lines);
        let mut request = ApplyRequest::new(plan.clone(), staging_text);
        request.managed = managed;
        request.environment = Secret::new(environment);
        let result = store.apply(
            request,
            spawner.into_object(),
            tools.into_object(),
            prompter.into_object(),
        );
        // A writer the install left behind gets this long to show itself; one that waits longer is not seen.
        std::thread::sleep(self.options.settle);
        let after = Footprint::take(&watch);
        let records = fetched
            .lock()
            .map(|f| {
                f.iter()
                    .map(|r| FileRecord {
                        path: r.path.clone(),
                        url: r.url.clone(),
                        sha256: r.sha256.clone(),
                        verified: plan
                            .downloads
                            .iter()
                            .find(|d| d.url == r.url)
                            .is_none_or(|d| !d.is_unverified()),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let lines = lines.lock().map(|l| l.clone()).unwrap_or_default();
        Ran {
            result,
            records,
            lines,
            before,
            after,
        }
    }

    fn watch(&self, staging: &Path) -> Watch {
        // The programs folder is watched whole but for this apply's own folder: another program's copy changing is not this one's.
        let mut watch = Watch::new(self.options.home.clone().unwrap_or_default())
            .tree(self.shelf.root())
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

    /// The folder the models of the type whose program `store` installs are kept in, beside the program's versions: a program's own
    /// install, update or removal never moves what is in it. It may not exist yet.
    ///
    /// # Errors
    ///
    /// What the shelf refuses.
    pub fn models_folder(&self, store: &dyn ProgramStore) -> Result<PathBuf, StoreError> {
        let info = store.describe()?;
        self.models_folder_of(&info.program)
    }

    /// The folder the models of the type that keeps its files under `program` are kept in: a type with no program store names its own
    /// id, so a type that gains or loses a program keeps its models where they are. It may not exist yet.
    ///
    /// # Errors
    ///
    /// What the shelf refuses.
    pub fn models_folder_of(&self, program: &str) -> Result<PathBuf, StoreError> {
        Ok(self.shelf.program(program)?.dir().join(MODELS_DIR))
    }

    /// Downloads `files` into the models folder of the type `store`'s program belongs to, each from an endpoint the store names and
    /// checked against its SHA-256, so that either every file is kept or none is. A file already there is replaced.
    ///
    /// # Errors
    ///
    /// A name that is not a plain relative path, the shelf's refusals (another install running), and what the download refuses
    /// (an address outside the store's endpoints, a checksum that does not match, a cancel).
    pub fn fetch_models(
        &self,
        store: &dyn ProgramStore,
        files: &[ModelFile],
        checks: Option<&[canoa::store::Check]>,
        cancel: Arc<AtomicBool>,
    ) -> Result<(), StoreError> {
        let info = store.describe()?;
        self.fetch_models_in(&info.program, &info.endpoints, files, checks, cancel)
    }

    /// As [`fetch_models`](Self::fetch_models) for the type whose files are kept under `program`, from the origins `endpoints`: the
    /// way a type with no program store is served.
    ///
    /// # Errors
    ///
    /// As [`fetch_models`](Self::fetch_models).
    pub fn fetch_models_in(
        &self,
        program: &str,
        endpoints: &[String],
        files: &[ModelFile],
        checks: Option<&[canoa::store::Check]>,
        cancel: Arc<AtomicBool>,
    ) -> Result<(), StoreError> {
        let folder = self.models_folder_of(program)?;
        let names = model_names(files)?;
        if checks.is_some_and(|checks| checks.len() != files.len()) {
            return Err(StoreError::Contract(
                "a model's files and the checks that go with them differ in number".to_owned(),
            ));
        }
        // A file nothing checks is taken only from a catalog entry that says it is trusted: the checks name it, and the plan did.
        let unchecked = checks.is_some_and(|checks| {
            checks
                .iter()
                .any(|check| matches!(check, canoa::store::Check::Unverified { .. }))
        });
        // One at a time with the installs: a program's install measures this folder's tree, and a download in it is not its change.
        let _one_at_a_time = self.shelf.apply_lock()?;
        let kept = self.shelf.program(program)?;
        let _lock = kept.lock()?;
        fs::create_dir_all(&folder).map_err(ShelfError::from)?;
        let tools = self
            .tools_at(endpoints)
            .allow_unverified(unchecked)
            .granted(&folder)
            .cancelled_by(cancel)
            .into_object();
        let part = |name: &Path| {
            let mut kept = name.as_os_str().to_owned();
            kept.push(".part");
            PathBuf::from(kept)
        };
        let mut fetched = Vec::new();
        let outcome = (|| -> Result<(), StoreError> {
            for (at, (file, name)) in files.iter().zip(&names).enumerate() {
                let partial = part(name);
                let _ = fs::remove_file(folder.join(&partial));
                fetched.push(partial.clone());
                let check = checks.map_or_else(
                    || canoa::store::Check::Sha256 {
                        value: file.sha256.to_ascii_lowercase(),
                    },
                    |checks| checks[at].clone(),
                );
                let request = canoa::store::FetchRequest::new(
                    file.url.clone(),
                    partial.to_string_lossy().replace('\\', "/"),
                    check,
                    file.bytes.unwrap_or(1 << 30).max(1),
                );
                tools.fetch(request)?;
            }
            Ok(())
        })();
        if let Err(error) = outcome {
            for partial in &fetched {
                let _ = fs::remove_file(folder.join(partial));
            }
            return Err(error);
        }
        for (partial, name) in fetched.iter().zip(&names) {
            let target = folder.join(name);
            let _ = fs::remove_file(&target);
            fs::rename(folder.join(partial), &target).map_err(ShelfError::from)?;
        }
        Ok(())
    }

    /// Takes `files` out of the models folder of the type `store`'s program belongs to; a file that is not there is already gone.
    ///
    /// # Errors
    ///
    /// A name that is not a plain relative path, and what the shelf or the system refuses.
    pub fn delete_models(
        &self,
        store: &dyn ProgramStore,
        files: &[ModelFile],
    ) -> Result<(), StoreError> {
        let info = store.describe()?;
        self.delete_models_in(&info.program, files)
    }

    /// As [`delete_models`](Self::delete_models) for the type whose files are kept under `program`.
    ///
    /// # Errors
    ///
    /// As [`delete_models`](Self::delete_models).
    pub fn delete_models_in(&self, program: &str, files: &[ModelFile]) -> Result<(), StoreError> {
        let folder = self.models_folder_of(program)?;
        let names = model_names(files)?;
        let _one_at_a_time = self.shelf.apply_lock()?;
        for name in names {
            match fs::remove_file(folder.join(name)) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(ShelfError::from(error).into()),
            }
        }
        Ok(())
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

/// The folder beside a program's versions that holds the models installed on their own.
const MODELS_DIR: &str = "models";

/// The relative paths of `files`, refusing one that leaves the models folder.
fn model_names(files: &[ModelFile]) -> Result<Vec<PathBuf>, StoreError> {
    files
        .iter()
        .map(|file| {
            super::tools::relative_path(&file.file).map_err(|_| {
                StoreError::Contract(format!(
                    "the model file {:?} is not a plain path in the models folder",
                    file.file
                ))
            })
        })
        .collect()
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

/// The parts of `target` below `home`, lower case, when it is under it; resolved by name (`.` and `..` gone).
fn parts_under_home(target: &str, home: &Path) -> Option<Vec<String>> {
    let target = canoa::store::exec::normalize(Path::new(target))?;
    let home = canoa::store::exec::normalize(home)?;
    let lower = |p: &Path| -> Vec<String> {
        p.components()
            .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
            .collect()
    };
    let (target, home) = (lower(&target), lower(&home));
    (target.len() >= home.len() && target.iter().zip(&home).all(|(a, b)| a == b))
        .then(|| target[home.len()..].to_vec())
}

/// Whether a state folder, as the parts below the home, is the program's own: `~/.name`, or `name` under the places a program
/// keeps its state.
fn named_for(parts: &[String], program: &str) -> bool {
    let program = program.to_lowercase();
    let starts = |prefix: &[&str]| {
        parts.len() > prefix.len()
            && prefix.iter().zip(parts).all(|(a, b)| a == b)
            && parts[prefix.len()] == program
    };
    parts
        .first()
        .is_some_and(|first| first.trim_start_matches('.') == program)
        || starts(&[".config"])
        || starts(&[".local", "share"])
        || starts(&["appdata", "roaming"])
        || starts(&["appdata", "local"])
}

/// Whether a registry key may be declared: the user's `Path`, a program's own `Software\<vendor>\<program>` (not Microsoft's, not
/// `Classes`) or its entry under `Uninstall`. Resolved by name first (`/` and `\`, `.` and `..`), so no spelling widens it.
fn registry_allowed(target: &str) -> bool {
    let mut parts: Vec<String> = Vec::new();
    for part in target.split(['\\', '/']) {
        match part {
            "" | "." => {}
            ".." => return false,
            other => parts.push(other.to_lowercase()),
        }
    }
    if parts.first().map(String::as_str) != Some("hkcu")
        && parts.first().map(String::as_str) != Some("hkey_current_user")
    {
        return false;
    }
    let below: Vec<&str> = parts[1..].iter().map(String::as_str).collect();
    match below.as_slice() {
        [
            "software",
            "microsoft",
            "windows",
            "currentversion",
            "uninstall",
            _,
            ..,
        ] => true,
        ["software", vendor, _, ..] => !matches!(
            *vendor,
            "microsoft" | "classes" | "policies" | "wow6432node"
        ),
        _ => false,
    }
}

#[cfg(test)]
mod plan_tests {
    use super::*;

    fn programs(per_owner: usize) -> Programs {
        let options = Options {
            plans_per_owner: per_owner,
            ..Options::default()
        };
        Programs::new(
            Shelf::unchecked(std::env::temp_dir().join("canoa-plan-tests")),
            Arc::new(Registry::new("test", "0.0.0")),
        )
        .options(options)
    }

    fn plan(token: &str) -> InstallPlan {
        let mut plan = InstallPlan::new(Action::Install, Some("1.0.0".to_owned()));
        plan.token = token.to_owned();
        plan
    }

    #[test]
    fn a_plan_is_applied_only_by_the_caller_it_was_made_for() {
        let programs = programs(8);
        programs.keep_plan("key-a", "echo_store", &plan("t1"));
        assert!(programs.take_plan("key-b", "echo_store", "t1").is_none());
        assert!(programs.take_plan("", "echo_store", "t1").is_none());
        assert!(programs.take_plan("key-a", "echo_store", "t1").is_some());
        assert!(
            programs.take_plan("key-a", "echo_store", "t1").is_none(),
            "once"
        );
    }

    #[test]
    fn a_plan_with_a_token_another_caller_holds_does_not_replace_that_callers() {
        let programs = programs(8);
        programs.keep_plan("owner", "echo_store", &plan("same"));
        programs.keep_plan("key", "echo_store", &plan("same"));
        assert!(programs.take_plan("owner", "echo_store", "same").is_some());
        assert!(programs.take_plan("key", "echo_store", "same").is_some());
    }

    #[test]
    fn past_the_hosts_limit_the_caller_loses_its_own_plans_first() {
        let options = Options {
            plans_per_owner: 4,
            plan_limit: 4,
            ..Options::default()
        };
        let programs = Programs::new(
            Shelf::unchecked(std::env::temp_dir().join("canoa-plan-tests")),
            Arc::new(Registry::new("test", "0.0.0")),
        )
        .options(options);
        programs.keep_plan("owner", "echo_store", &plan("mine"));
        for token in ["a", "b", "c", "d"] {
            programs.keep_plan("key", "echo_store", &plan(token));
        }
        assert!(
            programs.take_plan("owner", "echo_store", "mine").is_some(),
            "the owner's plan, the oldest of all, stays"
        );
        assert!(programs.take_plan("key", "echo_store", "a").is_none());
    }

    #[test]
    fn one_caller_cannot_push_out_the_plans_of_another() {
        let programs = programs(2);
        programs.keep_plan("owner", "echo_store", &plan("mine"));
        for token in ["a", "b", "c", "d", "e"] {
            programs.keep_plan("key", "echo_store", &plan(token));
        }
        assert!(
            programs.take_plan("owner", "echo_store", "mine").is_some(),
            "the owner's plan stays"
        );
        assert!(programs.take_plan("key", "echo_store", "a").is_none());
        assert!(programs.take_plan("key", "echo_store", "c").is_none());
        assert!(programs.take_plan("key", "echo_store", "d").is_some());
        assert!(programs.take_plan("key", "echo_store", "e").is_some());
    }
}
