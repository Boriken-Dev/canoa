//! [`Engine`]: the plan, apply and find that every built-in store shares, driven by a [`Recipe`] that holds what is specific
//! to one program: where it is published, how its download is checked, and how its version is read.
//!
//! A recipe describes a release as [`Part`](crate::store::recipe::Part)s: files to fetch into the version's folder, each perhaps unpacked, and a path
//! the program starts from. The engine never runs an installer the publisher wrote: it downloads, checks, places and reads
//! the version back.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "paths and clocks are std's"
)]

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write as _;
use std::path::Path;
use std::time::Instant;

use crate::store::recipe::executable_settings;
use crate::store::recipe::{Finish, Recipe, Release};
use crate::store::{
    Action, ApplyOutcome, ApplyRequest, Check, FetchRequest, FindRequest, InstallPlan,
    InstallPrompter, InstallTools, LatestQuery, LatestReport, Launch, MANAGER_CANOA,
    PROBE_TIMEOUT_MS, PlanRequest, Platform, ProgramCopy, ProgramQuery, ProgramReport, Progress,
    StoreInfo, StoreSettings, UnpackRequest, Updatable,
};
use coqui_acp_api::Environment;
use coqui_acp_api::ProcessSpawner;
use coqui_acp_api::guatiao::Status;
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use sha2::{Digest, Sha256};

use super::exec::{Run, deadline_after};

/// A store over a recipe.
#[derive(Debug)]
pub struct Engine<R> {
    recipe: R,
    settings: StoreSettings,
}

fn refuse(status: Status, message: &str) -> ProviderError {
    ProviderError::new(status, message)
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
            // Only the copy the person picked is run: an outside copy is whatever happens to be on the `PATH`.
            if query.probe
                && query.probe_copy.as_deref() == Some(file.path.as_str())
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
        plan.changes.clone_from(&release.changes);
        if let Some(from) = from.as_deref()
            && !numeric(from).is_empty()
            && numeric(&release.version) < numeric(from)
        {
            let words = format!(
                "This installs {}, which is older than {from}: a downgrade.",
                release.version
            );
            plan.notices.push(words.clone());
            plan.steps.push(words);
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
        plan.token = token_of(&release, &request.platform, request.action, from.as_deref());
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
        let channel = self.recipe.channel_of(plan);
        let release = self
            .recipe
            .release(Some(version), channel.as_deref(), &platform, tools)?;
        if token_of(
            &release,
            &platform,
            plan.action,
            plan.from_version.as_deref(),
        ) != plan.token
        {
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
        self.recipe
            .finish(&Finish::new(&request.staging, &release, &run, tools))?;
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
                        &format!("the downloaded program will not run: {error}"),
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
        if !crate::store::recipe::valid_version(&read) || read != release.version {
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

/// The numbers a version starts with, `1.5.2-beta` being `[1, 5, 2]`.
fn numeric(version: &str) -> Vec<u64> {
    version
        .split(['.', '-', '+'])
        .map_while(|part| part.parse().ok())
        .collect()
}

/// The opaque token of a release: what the plan rests on.
fn token_of(
    release: &Release,
    platform: &Platform,
    action: Action,
    replaces: Option<&str>,
) -> String {
    let mut text = format!(
        "{}|{}|{}|{}",
        release.version, platform.os, platform.arch, release.launch
    );
    let _ = write!(text, "|{action:?}|replaces:{}", replaces.unwrap_or("-"));
    let _ = write!(text, "|{:?}", release.changes.len());
    for change in &release.changes {
        let _ = write!(
            text,
            "|change:{}:{}:{}:{}",
            change.kind, change.target, change.detail, change.undone_by_uninstall
        );
    }
    for part in &release.parts {
        let check = match &part.download.check {
            Check::Sha256 { value } => format!("sha256:{value}"),
            Check::Sha512 { value } => format!("sha512:{value}"),
            _ => "unverified".to_owned(),
        };
        let _ = write!(text, "|{}|{}|{check}", part.download.url, part.path);
    }
    Sha256::digest(text.as_bytes())
        .iter()
        .fold(String::new(), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}
