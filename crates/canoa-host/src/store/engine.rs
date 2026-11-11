//! [`Engine`]: the plan, apply and find that every built-in store shares, driven by a [`Recipe`] that holds what is specific
//! to one program: where it is published, how its download is checked, and how its version is read.
//!
//! A recipe describes a release as [`Part`]s: files to fetch into the version's folder, each perhaps unpacked, and a path
//! the program starts from. The engine never runs an installer the publisher wrote: it downloads, checks, places and reads
//! the version back.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "paths and clocks are std's"
)]

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write as _;
use std::path::Path;
use std::time::Instant;

use canoa::store::{
    Action, ApplyOutcome, ApplyRequest, Check, Download, FetchRequest, FindRequest, InstallPlan,
    InstallPrompter, InstallTools, LatestQuery, LatestReport, Launch, MANAGER_CANOA,
    PROBE_TIMEOUT_MS, PlanRequest, Platform, ProgramCopy, ProgramQuery, ProgramReport, Progress,
    StoreInfo, StoreSettings, UnpackRequest, Updatable,
};
use coqui_acp_api::Environment;
use coqui_acp_api::ProcessSpawner;
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{Map, Status, Value};
use sha2::{Digest, Sha256};

use super::exec::{Run, deadline_after};

/// What to unpack a fetched file with.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct UnpackStep {
    /// `zip`, `tar` or `tar.gz`.
    pub format: &'static str,
    /// The folder, relative to the version's folder.
    pub into: String,
    /// How many leading parts to drop from each entry.
    pub strip: u32,
    /// Whether to leave links out.
    pub skip_links: bool,
}

/// One file of a release.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Part {
    /// What is fetched.
    pub download: Download,
    /// Where it goes, relative to the version's folder.
    pub path: String,
    /// Unpacked after, and the archive then removed.
    pub unpack: Option<UnpackStep>,
    /// Whether to make it executable (a standalone program).
    pub executable: bool,
}

impl Part {
    /// A file fetched to `path`.
    #[must_use]
    pub fn new(download: Download, path: impl Into<String>) -> Self {
        Self {
            download,
            path: path.into(),
            unpack: None,
            executable: false,
        }
    }

    /// Made executable once checked.
    #[must_use]
    pub const fn executable(mut self) -> Self {
        self.executable = true;
        self
    }

    /// Unpacked after the fetch.
    #[must_use]
    pub fn unpacked(mut self, step: UnpackStep) -> Self {
        self.unpack = Some(step);
        self
    }
}

/// A release: what to fetch and where the program starts.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Release {
    /// The version.
    pub version: String,
    /// Its files.
    pub parts: Vec<Part>,
    /// The path the program starts from, relative to the version's folder.
    pub launch: String,
    /// Extra steps in words, after the downloads.
    pub steps: Vec<String>,
    /// Things the person should read before confirming.
    pub notices: Vec<String>,
    /// What the recipe itself needs again at apply time, such as the relative path of a runtime it unpacks.
    pub facts: BTreeMap<String, String>,
}

impl Release {
    /// A release of `version` started from `launch`.
    #[must_use]
    pub fn new(version: impl Into<String>, launch: impl Into<String>, parts: Vec<Part>) -> Self {
        Self {
            version: version.into(),
            parts,
            launch: launch.into(),
            steps: Vec::new(),
            notices: Vec::new(),
            facts: BTreeMap::new(),
        }
    }
}

/// What a recipe's extra step is given.
#[non_exhaustive]
pub struct Finish<'a> {
    /// The absolute path of the version's folder.
    pub staging: &'a str,
    /// The release being applied.
    pub release: &'a Release,
    /// Runs a program in the folder, keeping the cancel and the deadline.
    pub run: &'a Run<'a>,
    /// The tools.
    pub tools: &'a Object<dyn InstallTools>,
}

impl core::fmt::Debug for Finish<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Finish")
            .field("staging", &self.staging)
            .finish_non_exhaustive()
    }
}

/// What is specific to one program.
pub trait Recipe: Send + Sync + 'static {
    /// Who the program is.
    fn info(&self) -> StoreInfo;

    /// File names to look for: `claude`, and on Windows `claude.exe` is tried too.
    fn names(&self) -> &'static [&'static str];

    /// Directories beside the `PATH` where a publisher's installer puts it.
    fn dirs(&self) -> &'static [&'static str];

    /// The arguments that make it print its version.
    fn version_args(&self) -> &'static [&'static str];

    /// The version in a line of `version_args`'s output.
    fn parse_version(&self, line: &str) -> Option<String>;

    /// Who put a copy at `path` there, and who can update it.
    fn classify(&self, path: &str) -> (&'static str, Updatable);

    /// Whether a copy replaces itself and no setting stops it.
    fn self_updates(&self) -> bool {
        false
    }

    /// Variables that turn the program's own updater off, none of them a credential.
    fn launch_environment(&self) -> BTreeMap<String, String>;

    /// The newest version, from the publisher's own endpoint.
    fn latest(&self, channel: Option<&str>, tools: &Object<dyn InstallTools>) -> LatestReport;

    /// The release of `version`, or the newest on `channel`, for `platform`.
    ///
    /// # Errors
    ///
    /// `GUATIAO_ERR_UNSUPPORTED` with the reason for a version or a platform the publisher does not serve.
    fn release(
        &self,
        version: Option<&str>,
        channel: Option<&str>,
        platform: &Platform,
        tools: &Object<dyn InstallTools>,
    ) -> Result<Release, ProviderError>;

    /// How to start the staged program whose release launches at `path`: the path, the settings that hand it to a harness
    /// provider and the variables that keep its updater off.
    fn launch(&self, path: &str, _staging: &str, _release: &Release) -> Launch {
        let mut launch = Launch::new(path);
        launch.settings = executable_settings(path);
        launch.environment = self.launch_environment();
        launch
    }

    /// The program and arguments that print the staged copy's version: its launch path and `version_args`.
    fn version_command(
        &self,
        path: &str,
        _staging: &str,
        _release: &Release,
    ) -> (String, Vec<String>) {
        (
            path.to_owned(),
            self.version_args()
                .iter()
                .map(|a| (*a).to_owned())
                .collect(),
        )
    }

    /// What runs after the files are placed: nothing, for a program that is one file.
    ///
    /// # Errors
    ///
    /// Whatever the step fails with.
    fn finish(&self, _finish: &Finish<'_>) -> Result<(), ProviderError> {
        Ok(())
    }
}

/// A store over a recipe.
#[derive(Debug)]
pub struct Engine<R> {
    recipe: R,
    settings: StoreSettings,
}

fn refuse(status: Status, message: &str) -> ProviderError {
    ProviderError::new(status, message)
}

/// The `executable` setting that hands a copy to a harness provider.
#[must_use]
pub fn executable_settings(path: &str) -> Value {
    let mut map = Map::new();
    map.set("executable", Value::from(path));
    Value::from(map)
}

fn launch_for<R: Recipe>(recipe: &R, path: &str) -> Launch {
    let mut launch = Launch::new(path);
    launch.settings = executable_settings(path);
    launch.environment = recipe.launch_environment();
    launch
}

/// The line of `text` that holds a version, by `parse`.
fn version_in(text: &str, parse: impl Fn(&str) -> Option<String>) -> Option<(String, String)> {
    text.lines()
        .map(str::trim)
        .find_map(|line| parse(line).map(|version| (version, line.to_owned())))
}

impl<R: Recipe> Engine<R> {
    /// A store of `recipe` configured as `settings` say.
    pub const fn new(recipe: R, settings: StoreSettings) -> Self {
        Self { recipe, settings }
    }

    /// Who the program is.
    ///
    /// # Errors
    ///
    /// Never.
    pub fn describe(&self) -> Result<StoreInfo, ProviderError> {
        Ok(self.recipe.info())
    }

    fn probe(
        &self,
        spawner: &Object<dyn ProcessSpawner>,
        path: &str,
        environment: &Environment,
    ) -> Option<(String, String)> {
        let lower = path.to_ascii_lowercase();
        if [".cmd", ".bat", ".ps1"]
            .iter()
            .any(|ext| lower.ends_with(ext))
        {
            return None;
        }
        let dir = Path::new(path).parent()?.to_string_lossy().into_owned();
        let run = Run::new(
            spawner,
            dir,
            environment.clone(),
            deadline_after(PROBE_TIMEOUT_MS),
        );
        let launch = self.recipe.launch_environment();
        let extra: Vec<(&str, &str)> = launch
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let output = run.exec(path, self.recipe.version_args(), &extra).ok()?;
        version_in(&output.text, |line| self.recipe.parse_version(line))
    }

    /// The copies of the program found.
    ///
    /// # Errors
    ///
    /// What `tools.find` refuses.
    pub fn program(
        &self,
        query: &ProgramQuery,
        spawner: &Object<dyn ProcessSpawner>,
        tools: &Object<dyn InstallTools>,
    ) -> Result<ProgramReport, ProviderError> {
        let environment = query.environment.expose();
        let mut report = ProgramReport::default();
        if let Some(managed) = &query.managed {
            let mut copy = ProgramCopy::new(
                &managed.launch.path,
                MANAGER_CANOA,
                managed.launch.clone(),
                Updatable::ByStore,
            );
            copy.version = Some(managed.version.clone());
            copy.managed = true;
            copy.self_updates = self.recipe.self_updates();
            if query.probe
                && let Some((version, line)) =
                    self.probe(spawner, &managed.launch.path, environment)
            {
                copy.version = Some(version);
                copy.version_line = Some(line);
            }
            report.copies.push(copy);
        }
        let found = tools.find(FindRequest::new(
            self.recipe
                .names()
                .iter()
                .map(|n| (*n).to_owned())
                .collect(),
            self.recipe.dirs().iter().map(|d| (*d).to_owned()).collect(),
        ))?;
        for file in found {
            if report.copies.iter().any(|c| c.path == file.path) {
                continue;
            }
            let (manager, update) = self.recipe.classify(&file.path);
            let mut copy = ProgramCopy::new(
                &file.path,
                manager,
                launch_for(&self.recipe, &file.path),
                update,
            );
            copy.self_updates = self.recipe.self_updates();
            if query.probe
                && let Some((version, line)) = self.probe(spawner, &file.path, environment)
            {
                copy.version = Some(version);
                copy.version_line = Some(line);
            }
            report.copies.push(copy);
        }
        Ok(report)
    }

    /// The newest version.
    ///
    /// # Errors
    ///
    /// Never: a failure is `known: false`.
    pub fn latest(
        &self,
        query: &LatestQuery,
        tools: &Object<dyn InstallTools>,
    ) -> Result<LatestReport, ProviderError> {
        let channel = query
            .channel
            .as_deref()
            .or(self.settings.channel.as_deref());
        Ok(self.recipe.latest(channel, tools))
    }

    fn check_platform(&self, platform: &Platform) -> Result<(), ProviderError> {
        let info = self.recipe.info();
        if info.platforms.iter().any(|p| p == platform) {
            return Ok(());
        }
        Err(refuse(
            Status::GUATIAO_ERR_UNSUPPORTED,
            &format!(
                "{} is not published for {} on {}",
                info.title, platform.os, platform.arch
            ),
        ))
    }

    /// What `apply` would do.
    ///
    /// # Errors
    ///
    /// `GUATIAO_ERR_UNSUPPORTED` for a platform or version the publisher does not serve; what the publisher's endpoints
    /// refuse.
    pub fn plan(
        &self,
        request: &PlanRequest,
        tools: &Object<dyn InstallTools>,
    ) -> Result<InstallPlan, ProviderError> {
        let from = request.managed.as_ref().map(|m| m.version.clone());
        if request.action == Action::Uninstall {
            let mut plan = InstallPlan::new(Action::Uninstall, None);
            plan.from_version = from;
            return Ok(plan);
        }
        self.check_platform(&request.platform)?;
        let version = request
            .version
            .as_deref()
            .or(self.settings.version.as_deref());
        let channel = request
            .channel
            .as_deref()
            .or(self.settings.channel.as_deref());
        let release = self
            .recipe
            .release(version, channel, &request.platform, tools)?;
        let mut plan = InstallPlan::new(request.action, Some(release.version.clone()));
        plan.from_version = from.clone();
        if request.action == Action::Update && from.as_deref() == Some(release.version.as_str()) {
            return Ok(plan);
        }
        plan.bytes = release
            .parts
            .iter()
            .try_fold(0u64, |sum, p| p.download.bytes.map(|b| sum + b));
        for part in &release.parts {
            let how = match &part.download.check {
                Check::Sha256 { .. } => "SHA-256 checked".to_owned(),
                Check::Sha512 { .. } => "SHA-512 checked".to_owned(),
                Check::Unverified { why } => format!("not verified: {why}"),
                _ => "checked".to_owned(),
            };
            plan.steps
                .push(format!("Download {} ({how})", part.download.url));
            if part.download.is_unverified() {
                plan.notices
                    .push(format!("Not verified: {}", part.download.url));
            }
            if let Some(step) = &part.unpack {
                plan.steps.push(format!("Unpack it as {}", step.format));
            }
        }
        plan.steps.extend(release.steps.iter().cloned());
        plan.steps
            .push("Run the downloaded program once to read its version".to_owned());
        plan.notices.extend(release.notices.iter().cloned());
        plan.downloads = release.parts.iter().map(|p| p.download.clone()).collect();
        plan.token = token_of(&release, &request.platform);
        Ok(plan)
    }

    /// Carries out `request.plan` in `request.staging`.
    ///
    /// # Errors
    ///
    /// `GUATIAO_ERR_BAD_VALUE` with `plan changed` when what the plan rests on moved or a download does not match;
    /// `GUATIAO_ERR_GONE` when cancelled or timed out; `GUATIAO_ERR_INTERNAL` for a download that failed or a program that
    /// will not run.
    pub fn apply(
        &self,
        request: &ApplyRequest,
        spawner: &Object<dyn ProcessSpawner>,
        tools: &Object<dyn InstallTools>,
        prompter: &Object<dyn InstallPrompter>,
    ) -> Result<ApplyOutcome, ProviderError> {
        let plan = &request.plan;
        if plan.action == Action::Uninstall {
            return Ok(ApplyOutcome::default());
        }
        let Some(version) = plan.version.as_deref() else {
            return Err(refuse(
                Status::GUATIAO_ERR_BAD_VALUE,
                "a plan with no version",
            ));
        };
        if plan.downloads.is_empty() {
            let managed = request.managed.as_ref().ok_or_else(|| {
                refuse(Status::GUATIAO_ERR_BAD_VALUE, "an empty plan and no copy")
            })?;
            return Ok(ApplyOutcome::new(
                Some(managed.version.clone()),
                Some(managed.launch.clone()),
            ));
        }
        let info = spawner.describe()?;
        let platform = Platform::new(info.os, info.arch);
        let release = self.recipe.release(Some(version), None, &platform, tools)?;
        if token_of(&release, &platform) != plan.token {
            return Err(refuse(Status::GUATIAO_ERR_BAD_VALUE, "plan changed"));
        }
        let deadline = Instant::now() + core::time::Duration::from_millis(request.timeout_ms);
        let cancelled = || -> Result<(), ProviderError> {
            if prompter.cancelled().unwrap_or(false) {
                return Err(refuse(Status::GUATIAO_ERR_GONE, "install cancelled"));
            }
            if Instant::now() >= deadline {
                return Err(refuse(Status::GUATIAO_ERR_GONE, "install timed out"));
            }
            Ok(())
        };
        let total = release.parts.len() as u64;
        for (index, part) in release.parts.iter().enumerate() {
            cancelled()?;
            prompter.progress(Progress::new(
                format!("Downloading {}", part.download.url),
                index as u64,
                Some(total),
            ))?;
            prompter.say(&format!("Downloading {}", part.download.url))?;
            let mut fetch = FetchRequest::new(
                part.download.url.clone(),
                part.path.clone(),
                part.download.check.clone(),
                part.download.bytes.unwrap_or(1 << 30).max(1),
            );
            fetch.executable = part.executable;
            tools.fetch(fetch)?;
            if let Some(step) = &part.unpack {
                cancelled()?;
                prompter.say(&format!("Unpacking {}", part.path))?;
                let mut unpack = UnpackRequest::new(
                    part.path.clone(),
                    step.into.clone(),
                    step.format,
                    4 << 30,
                    100_000,
                );
                unpack.strip = step.strip;
                unpack.remove_archive = true;
                unpack.skip_links = step.skip_links;
                tools.unpack(unpack)?;
            }
        }
        cancelled()?;
        let run = Run::new(
            spawner,
            request.staging.clone(),
            request.environment.expose().clone(),
            deadline,
        )
        .prompter(prompter);
        self.recipe.finish(&Finish {
            staging: &request.staging,
            release: &release,
            run: &run,
            tools,
        })?;
        cancelled()?;
        let program = Path::new(&request.staging)
            .join(&release.launch)
            .to_string_lossy()
            .into_owned();
        prompter.say("Reading the version of the downloaded program")?;
        let launch_env = self.recipe.launch_environment();
        let extra: Vec<(&str, &str)> = launch_env
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let (version_program, version_args) =
            self.recipe
                .version_command(&program, &request.staging, &release);
        let version_args: Vec<&str> = version_args.iter().map(String::as_str).collect();
        let output = run
            .exec(&version_program, &version_args, &extra)
            .map_err(|error| {
                if error.status == Status::GUATIAO_ERR_GONE {
                    error
                } else {
                    refuse(
                        Status::GUATIAO_ERR_INTERNAL,
                        "the downloaded program will not run",
                    )
                }
            })?;
        if output.code != Some(0) {
            return Err(refuse(
                Status::GUATIAO_ERR_INTERNAL,
                "the downloaded program will not run: it exited with an error",
            ));
        }
        let Some((read, _)) = version_in(&output.text, |l| self.recipe.parse_version(l)) else {
            return Err(refuse(
                Status::GUATIAO_ERR_INTERNAL,
                "the downloaded program will not run: it printed no version",
            ));
        };
        if read != release.version {
            return Err(refuse(
                Status::GUATIAO_ERR_INTERNAL,
                &format!("the program reports {read}, not {}", release.version),
            ));
        }
        Ok(ApplyOutcome::new(
            Some(read),
            Some(self.recipe.launch(&program, &request.staging, &release)),
        ))
    }
}

/// The opaque token of a release: what the plan rests on.
fn token_of(release: &Release, platform: &Platform) -> String {
    let mut text = format!(
        "{}|{}|{}|{}",
        release.version, platform.os, platform.arch, release.launch
    );
    for part in &release.parts {
        let check = match &part.download.check {
            Check::Sha256 { value } => format!("sha256:{value}"),
            Check::Sha512 { value } => format!("sha512:{value}"),
            _ => "unverified".to_owned(),
        };
        let _ = write!(text, "|{}|{}|{check}", part.download.url, part.path);
    }
    let digest = Sha256::digest(text.as_bytes());
    format!("{digest:x}")
}
