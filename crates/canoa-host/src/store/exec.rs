//! Running programs for a store: the spawner it is handed, which refuses what a store must never start, and [`Run`], which a
//! store runs a program with and gets its output, its cancel and its deadline kept for it.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "paths and clocks are std's"
)]

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::time::Duration;
use std::path::Path;
use std::sync::Mutex;
use std::time::Instant;

use canoa::store::InstallPrompter;
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{Secret, Status};
use coqui_acp_api::{
    Environment, ErrorOutput, FileInfo, ProcessExit, ProcessSpawner, SecretBytes, SpawnSpec,
    SpawnedProcess, SpawnerInfo,
};

/// What a started program was asked to be, as the spawner saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Spawned {
    /// The program.
    pub program: String,
    /// Its arguments.
    pub args: Vec<String>,
    /// Its working directory.
    pub cwd: Option<String>,
    /// The names of the variables it was given.
    pub variables: Vec<String>,
}

/// Whether `name` looks like a credential: a variable a store must never pass on.
#[must_use]
pub fn is_credential_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    upper.ends_with("_API_KEY")
        || upper.ends_with("_TOKEN")
        || upper.ends_with("_SECRET")
        || upper.ends_with("PASSWORD")
        || upper.ends_with("_KEY")
        || upper == "ANTHROPIC_AUTH_TOKEN"
}

/// What a guarded spawner kept.
#[derive(Debug, Default)]
pub struct SpawnLog {
    spawned: Mutex<Vec<Spawned>>,
    violations: Mutex<Vec<String>>,
}

impl SpawnLog {
    /// Every program started, in order.
    #[must_use]
    pub fn spawned(&self) -> Vec<Spawned> {
        self.spawned.lock().map(|g| g.clone()).unwrap_or_default()
    }

    /// Every spawn that was refused, and why.
    #[must_use]
    pub fn violations(&self) -> Vec<String> {
        self.violations
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default()
    }
}

/// A [`ProcessSpawner`] that starts only what a store may: an absolute program that is not a `.cmd`, `.bat` or `.ps1`,
/// in a working directory inside the folder the store was given, with no credential variable, on a search path that never
/// reads the current directory. It refuses the rest and records why.
#[derive(Debug)]
#[must_use]
pub struct GuardedSpawner<S> {
    inner: S,
    staging: String,
    log: Arc<SpawnLog>,
    lenient_cwd: bool,
}

impl<S: ProcessSpawner> GuardedSpawner<S> {
    /// Guards `inner` for a store given `staging`.
    pub fn new(inner: S, staging: impl Into<String>) -> Self {
        Self {
            inner,
            staging: staging.into(),
            log: Arc::new(SpawnLog::default()),
            lenient_cwd: false,
        }
    }

    /// Lets a program start in any working directory: for a probe of a copy that is not in the staging folder.
    pub const fn lenient_cwd(mut self) -> Self {
        self.lenient_cwd = true;
        self
    }

    /// What it keeps.
    #[must_use]
    pub fn log(&self) -> Arc<SpawnLog> {
        Arc::clone(&self.log)
    }

    fn refuse(&self, why: String) -> ProviderError {
        if let Ok(mut violations) = self.log.violations.lock() {
            violations.push(why.clone());
        }
        ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, &why)
    }

    fn check(&self, spec: &SpawnSpec) -> Result<(), String> {
        let path = Path::new(&spec.program);
        if !path.is_absolute() {
            return Err(format!(
                "a program that is not an absolute path: {}",
                spec.program
            ));
        }
        let lower = spec.program.to_ascii_lowercase();
        if [".cmd", ".bat", ".ps1"]
            .iter()
            .any(|ext| lower.ends_with(ext))
        {
            return Err(format!("a script started directly: {}", spec.program));
        }
        match &spec.cwd {
            _ if self.lenient_cwd => {}
            Some(cwd) if Path::new(cwd).starts_with(&self.staging) => {}
            Some(cwd) => {
                return Err(format!(
                    "a working directory outside the staging folder: {cwd}"
                ));
            }
            None => return Err("no working directory".to_owned()),
        }
        for name in spec.environment.expose().names() {
            if is_credential_name(name) {
                return Err(format!("a credential variable passed on: {name}"));
            }
        }
        Ok(())
    }
}

impl<S: ProcessSpawner> ProcessSpawner for GuardedSpawner<S> {
    fn describe(&self) -> Result<SpawnerInfo, ProviderError> {
        self.inner.describe()
    }

    fn spawn(&self, mut spec: SpawnSpec) -> Result<Object<dyn SpawnedProcess>, ProviderError> {
        if let Err(why) = self.check(&spec) {
            return Err(self.refuse(why));
        }
        let mut environment = spec.environment.expose().clone();
        environment.insert("NoDefaultCurrentDirectoryInExePath", "1");
        if let Ok(mut spawned) = self.log.spawned.lock() {
            spawned.push(Spawned {
                program: spec.program.clone(),
                args: spec.args.clone(),
                cwd: spec.cwd.clone(),
                variables: environment.names().cloned().collect(),
            });
        }
        spec.environment = Secret::new(environment);
        self.inner.spawn(spec)
    }

    fn write_file(
        &self,
        path: &str,
        bytes: Secret<SecretBytes>,
        private: bool,
    ) -> Result<(), ProviderError> {
        self.inner.write_file(path, bytes, private)
    }

    fn read_file(&self, path: &str, max_bytes: u64) -> Result<Secret<SecretBytes>, ProviderError> {
        self.inner.read_file(path, max_bytes)
    }

    fn file_info(&self, path: &str) -> Result<Option<FileInfo>, ProviderError> {
        self.inner.file_info(path)
    }

    fn remove_file(&self, path: &str) -> Result<(), ProviderError> {
        self.inner.remove_file(path)
    }
}

/// What a program printed and how it ended.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Output {
    /// Its exit code, when it exited by itself.
    pub code: Option<i64>,
    /// Everything it printed, standard error included.
    pub text: String,
}

/// Runs programs for a store, keeping the person's cancel and the apply's deadline.
#[must_use]
pub struct Run<'a> {
    spawner: &'a Object<dyn ProcessSpawner>,
    prompter: Option<&'a Object<dyn InstallPrompter>>,
    cwd: String,
    environment: Environment,
    deadline: Instant,
}

impl core::fmt::Debug for Run<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Run")
            .field("cwd", &self.cwd)
            .finish_non_exhaustive()
    }
}

impl<'a> Run<'a> {
    /// Runs through `spawner` in `cwd` with `environment`, ending at `deadline`.
    pub fn new(
        spawner: &'a Object<dyn ProcessSpawner>,
        cwd: impl Into<String>,
        environment: Environment,
        deadline: Instant,
    ) -> Self {
        let environment: Environment = environment
            .iter()
            .filter(|(name, _)| !is_credential_name(name))
            .collect();
        Self {
            spawner,
            prompter: None,
            cwd: cwd.into(),
            environment,
            deadline,
        }
    }

    /// Relays each line a program prints to `prompter`, and stops when it cancels.
    pub const fn prompter(mut self, prompter: &'a Object<dyn InstallPrompter>) -> Self {
        self.prompter = Some(prompter);
        self
    }

    /// Runs `program` with `args` and `extra` variables to the end.
    ///
    /// # Errors
    ///
    /// `GUATIAO_ERR_GONE` with `install cancelled` or `install timed out` once the person cancels or the deadline passes
    /// (the program's whole tree is killed first); what the spawner refuses, as it does.
    pub fn exec(
        &self,
        program: &str,
        args: &[&str],
        extra: &[(&str, &str)],
    ) -> Result<Output, ProviderError> {
        let mut environment = self.environment.clone();
        for (name, value) in extra {
            environment.insert(*name, *value);
        }
        let mut spec = SpawnSpec::new(program)
            .cwd(self.cwd.clone())
            .environment(environment)
            .error_output(ErrorOutput::Merged);
        for arg in args {
            spec = spec.arg(*arg);
        }
        let process = self.spawner.spawn(spec)?;
        let mut text = String::new();
        let mut pending: Vec<u8> = Vec::new();
        let mut buffer = vec![0u8; 8192];
        loop {
            if let Some(why) = self.interrupted() {
                let _ = process.kill();
                return Err(ProviderError::new(Status::GUATIAO_ERR_GONE, why));
            }
            let count = process.read(&mut buffer, 200)?;
            if count == coqui_acp_api::READ_TIMED_OUT || count == coqui_acp_api::READ_LINK_DOWN {
                continue;
            }
            if count <= 0 {
                break;
            }
            pending.extend_from_slice(&buffer[..usize::try_from(count).unwrap_or(0)]);
            while let Some(end) = pending.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = pending.drain(..=end).collect();
                let line = clean(&String::from_utf8_lossy(&line));
                self.say(&line);
                text.push_str(&line);
                text.push('\n');
            }
        }
        if !pending.is_empty() {
            let line = clean(&String::from_utf8_lossy(&pending));
            self.say(&line);
            text.push_str(&line);
        }
        let exit = loop {
            if let Some(why) = self.interrupted() {
                let _ = process.kill();
                return Err(ProviderError::new(Status::GUATIAO_ERR_GONE, why));
            }
            let exit: ProcessExit = process.wait(200)?;
            if !exit.running {
                break exit;
            }
        };
        Ok(Output {
            code: exit.code,
            text,
        })
    }

    fn say(&self, line: &str) {
        if let Some(prompter) = self.prompter
            && !line.is_empty()
        {
            let _ = prompter.say(line);
        }
    }

    fn interrupted(&self) -> Option<&'static str> {
        if Instant::now() >= self.deadline {
            return Some("install timed out");
        }
        if let Some(prompter) = self.prompter
            && prompter.cancelled().unwrap_or(false)
        {
            return Some("install cancelled");
        }
        None
    }
}

/// A line the person may read: control and bidirectional characters out, at most 4 KiB.
#[must_use]
pub fn clean(line: &str) -> String {
    let mut out: String = line
        .trim_end_matches(['\r', '\n'])
        .chars()
        .filter(|c| {
            !c.is_control() || *c == '\t'
        })
        .filter(|c| !matches!(*c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{200e}' | '\u{200f}'))
        .collect();
    if out.len() > 4096 {
        let mut end = 4096;
        while !out.is_char_boundary(end) {
            end -= 1;
        }
        out.truncate(end);
    }
    out
}

/// The time `timeout_ms` from now.
#[must_use]
pub fn deadline_after(timeout_ms: u64) -> Instant {
    Instant::now() + Duration::from_millis(timeout_ms)
}
