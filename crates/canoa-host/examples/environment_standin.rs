//! Two stand-in environments as a Guatiao library: the environments the host's tests give an agent. `standin_a` offers everything
//! (programs and a terminal started through this computer's spawner, files in a folder, and a capability of its own, `snapshot`);
//! `standin_b` only runs programs. Each is built from `{"root": folder, "log": file}`: paths are resolved under the folder, and
//! every request is appended to the log file, so a test can tell what reached it. This file is a `cdylib` and nothing else, as a
//! crate calling `providers!` has to be.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "an example names what it uses from std"
)]

use std::io::Write as _;
use std::path::{Path, PathBuf};

use canoa::environment::{
    Call, CallAnswer, Capability, DirEntry, Environment, EnvironmentInfo, FileContent, ListDir,
    Listing, ReadFile, WriteFile, bad_value, capability, internal, not_found, unsupported,
};
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{self, FromValue, Provider, Schema, Secret};
use coqui_acp_api::{
    FileInfo, HarnessNotice, ProcessSpawner, SecretBytes, SpawnSpec, SpawnedProcess, SpawnerInfo,
};
use coqui_acp_harness::LocalSpawner;

/// What both stand-ins are built from.
#[derive(FromValue, Schema)]
#[map(crate = ::coqui_acp_api::guatiao, deny_unknown_fields)]
struct Settings {
    /// The folder their files are in.
    root: String,
    /// The file each request is appended to.
    log: String,
}

/// What the two share.
struct Core {
    name: &'static str,
    root: PathBuf,
    log: PathBuf,
}

impl Core {
    fn new(name: &'static str, settings: Settings) -> Self {
        Self {
            name,
            root: PathBuf::from(settings.root),
            log: PathBuf::from(settings.log),
        }
    }

    fn note(&self, what: &str) {
        note(&self.log, &format!("{} {what}", self.name));
    }

    /// `path` under the folder; a path that leaves it is refused.
    fn resolve(&self, path: &str) -> Result<PathBuf, ProviderError> {
        let relative = path.trim_start_matches(['/', '\\']);
        let relative = Path::new(relative);
        if relative.components().any(|part| {
            matches!(
                part,
                std::path::Component::ParentDir | std::path::Component::Prefix(_)
            )
        }) {
            return Err(bad_value("the path leaves the folder"));
        }
        Ok(self.root.join(relative))
    }

    fn spawner(&self) -> Object<dyn ProcessSpawner> {
        Noting {
            name: self.name,
            log: self.log.clone(),
            inner: LocalSpawner::new(),
        }
        .into_object()
    }

    fn read(&self, request: &ReadFile) -> Result<FileContent, ProviderError> {
        self.note(&format!("read {}", request.path));
        let file = self.resolve(&request.path)?;
        let meta = std::fs::metadata(&file).map_err(|error| not_found(error.to_string()))?;
        if meta.len() > request.max_bytes {
            return Err(bad_value("the file is longer than asked for"));
        }
        std::fs::read(file)
            .map(FileContent::new)
            .map_err(|error| not_found(error.to_string()))
    }

    fn write(&self, request: &WriteFile) -> Result<(), ProviderError> {
        self.note(&format!("write {}", request.path));
        let file = self.resolve(&request.path)?;
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).map_err(|error| internal(error.to_string()))?;
        }
        std::fs::write(file, &request.bytes.0).map_err(|error| internal(error.to_string()))
    }

    fn list(&self, request: &ListDir) -> Result<Listing, ProviderError> {
        self.note(&format!("list {}", request.path));
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(self.resolve(&request.path)?)
            .map_err(|error| not_found(error.to_string()))?
            .flatten()
        {
            let name = entry.file_name().to_string_lossy().into_owned();
            match entry.metadata() {
                Ok(meta) if meta.is_dir() => entries.push(DirEntry::folder(name)),
                Ok(meta) => entries.push(DirEntry::file(name, meta.len())),
                Err(_) => {}
            }
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(Listing::new(entries))
    }
}

fn note(log: &Path, what: &str) {
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
    {
        let _ = writeln!(file, "{what}");
    }
}

/// This computer's spawner that notes every program started through it, and takes a terminal on pipes: a shell on pipes is what a
/// test types into.
struct Noting {
    name: &'static str,
    log: PathBuf,
    inner: LocalSpawner,
}

impl ProcessSpawner for Noting {
    fn describe(&self) -> Result<SpawnerInfo, ProviderError> {
        let mut info = SpawnerInfo::new(std::env::consts::OS, std::env::consts::ARCH, false);
        info.terminal = true;
        info.separate_error = true;
        Ok(info)
    }

    fn spawn(&self, mut spec: SpawnSpec) -> Result<Object<dyn SpawnedProcess>, ProviderError> {
        let terminal = if spec.terminal.take().is_some() {
            " (terminal)"
        } else {
            ""
        };
        note(
            &self.log,
            &format!("{} spawn {}{terminal}", self.name, spec.program),
        );
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

    fn notice(&self, notice: HarnessNotice) -> Result<(), ProviderError> {
        self.inner.notice(notice)
    }
}

/// The stand-in that offers everything.
#[derive(Provider)]
#[provider(Environment, id = "standin_a", config = Settings, crate = ::coqui_acp_api::guatiao)]
struct StandInA(Core);

impl From<Settings> for StandInA {
    fn from(settings: Settings) -> Self {
        Self(Core::new("standin_a", settings))
    }
}

impl Environment for StandInA {
    fn describe(&self) -> Result<EnvironmentInfo, ProviderError> {
        let shell = if cfg!(windows) { "cmd.exe" } else { "/bin/sh" };
        Ok(
            EnvironmentInfo::new("Stand-in A", std::env::consts::OS, std::env::consts::ARCH)
                .offering(Capability::new(capability::EXEC))
                .offering(Capability::new(capability::TERMINAL).fact("shell", shell))
                .offering(Capability::new(capability::FS_READ))
                .offering(Capability::new(capability::FS_WRITE))
                .offering(Capability::new(capability::FS_LIST))
                .offering(Capability::new("snapshot").help("Takes a snapshot of the machine.")),
        )
    }

    fn spawner(&self) -> Result<Object<dyn ProcessSpawner>, ProviderError> {
        Ok(self.0.spawner())
    }

    fn read_file(&self, request: ReadFile) -> Result<FileContent, ProviderError> {
        self.0.read(&request)
    }

    fn write_file(&self, request: WriteFile) -> Result<(), ProviderError> {
        self.0.write(&request)
    }

    fn list_dir(&self, request: ListDir) -> Result<Listing, ProviderError> {
        self.0.list(&request)
    }

    fn call(&self, request: Call) -> Result<CallAnswer, ProviderError> {
        self.0.note(&format!(
            "call {} {}",
            request.capability, request.arguments
        ));
        if request.capability == "snapshot" {
            Ok(CallAnswer::new("snapshot taken"))
        } else {
            Err(unsupported("no such capability"))
        }
    }
}

/// The stand-in that only runs programs.
#[derive(Provider)]
#[provider(Environment, id = "standin_b", config = Settings, crate = ::coqui_acp_api::guatiao)]
struct StandInB(Core);

impl From<Settings> for StandInB {
    fn from(settings: Settings) -> Self {
        Self(Core::new("standin_b", settings))
    }
}

impl Environment for StandInB {
    fn describe(&self) -> Result<EnvironmentInfo, ProviderError> {
        Ok(
            EnvironmentInfo::new("Stand-in B", std::env::consts::OS, std::env::consts::ARCH)
                .offering(Capability::new(capability::EXEC)),
        )
    }

    fn spawner(&self) -> Result<Object<dyn ProcessSpawner>, ProviderError> {
        Ok(self.0.spawner())
    }

    fn read_file(&self, request: ReadFile) -> Result<FileContent, ProviderError> {
        self.0.note(&format!("read {}", request.path));
        Err(unsupported("standin_b has no files"))
    }

    fn write_file(&self, request: WriteFile) -> Result<(), ProviderError> {
        self.0.note(&format!("write {}", request.path));
        Err(unsupported("standin_b has no files"))
    }

    fn list_dir(&self, request: ListDir) -> Result<Listing, ProviderError> {
        self.0.note(&format!("list {}", request.path));
        Err(unsupported("standin_b has no files"))
    }

    fn call(&self, request: Call) -> Result<CallAnswer, ProviderError> {
        self.0.note(&format!("call {}", request.capability));
        Err(unsupported("standin_b has no extra capabilities"))
    }
}

guatiao::providers!(StandInA, StandInB);
