//! The built-in exporter `file`: OTLP's JSON, one document a line, appended to `spans-YYYY-MM-DD.jsonl`, `metrics-…` and `logs-…`
//! in a folder (UTC days). The folder is `telemetry/` in the host's home unless `folder` says another; it is made readable by this
//! user alone where the system has such a thing. Files older than `keep_days` days are removed when the exporter is made and once a
//! day after, and only files this exporter names.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files and locks are std's"
)]

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError, RwLock};

use canoa::telemetry::{Batch, ExporterError, ExporterInfo, TelemetryExporter, otlp};
use coqui_acp_api::guatiao::library::ProviderError;
use coqui_acp_api::guatiao::{FromValue, Provider, Schema, ToValue};

/// How many days of files the exporter keeps unless `keep_days` says another number.
pub(crate) const DEFAULT_KEEP_DAYS: u32 = 14;

const GROUPS: [&str; 3] = ["spans", "metrics", "logs"];
const SECONDS_A_DAY: i64 = 86_400;

static HOME: RwLock<Option<PathBuf>> = RwLock::new(None);

/// The host's home, which `telemetry/` of an exporter that names no folder is in.
pub(crate) fn set_home(home: Option<PathBuf>) {
    *HOME.write().unwrap_or_else(PoisonError::into_inner) = home;
}

fn home() -> Option<PathBuf> {
    HOME.read().unwrap_or_else(PoisonError::into_inner).clone()
}

/// What the `file` exporter is configured with.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Default, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct FileSettings {
    /// The folder the files are written in; `telemetry` in the host's home when none is named.
    #[schema(title = "Folder")]
    pub folder: Option<String>,
    /// How many days of files are kept, at least 1; 14 when none is named.
    #[schema(title = "Keep days")]
    pub keep_days: Option<u32>,
}

/// Writes batches to files.
#[derive(Provider)]
#[provider(TelemetryExporter, id = "file", name = "Files", config = FileSettings, crate = ::coqui_acp_api::guatiao)]
pub struct FileExporter {
    folder: Option<PathBuf>,
    keep_days: u32,
    swept: Mutex<String>,
}

impl core::fmt::Debug for FileExporter {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FileExporter")
            .field("keep_days", &self.keep_days)
            .finish_non_exhaustive()
    }
}

impl From<FileSettings> for FileExporter {
    fn from(settings: FileSettings) -> Self {
        let exporter = Self {
            folder: settings
                .folder
                .filter(|folder| !folder.trim().is_empty())
                .map(PathBuf::from),
            keep_days: settings.keep_days.unwrap_or(DEFAULT_KEEP_DAYS).max(1),
            swept: Mutex::new(String::new()),
        };
        if let Some(folder) = exporter.folder() {
            exporter.sweep(&folder, now_seconds());
        }
        exporter
    }
}

fn now_seconds() -> i64 {
    super::hub::now_ns() / 1_000_000_000
}

/// `YYYY-MM-DD` of `seconds`, UTC.
fn day_of(seconds: i64) -> String {
    crate::utc::rfc3339_unix(seconds)[..10].to_owned()
}

impl FileExporter {
    /// An exporter writing to `folder`.
    #[cfg(test)]
    pub(crate) fn in_folder(folder: &Path, keep_days: u32) -> Self {
        Self::from(FileSettings {
            folder: Some(folder.to_string_lossy().into_owned()),
            keep_days: Some(keep_days),
        })
    }

    fn folder(&self) -> Option<PathBuf> {
        self.folder
            .clone()
            .or_else(|| home().map(|home| home.join("telemetry")))
    }

    /// Removes this exporter's files that are older than `keep_days` days before `now`, once a day.
    fn sweep(&self, folder: &Path, now: i64) {
        let today = day_of(now);
        {
            let mut swept = self.swept.lock().unwrap_or_else(PoisonError::into_inner);
            if *swept == today {
                return;
            }
            swept.clone_from(&today);
        }
        let cutoff = day_of(now - i64::from(self.keep_days) * SECONDS_A_DAY);
        let Ok(entries) = std::fs::read_dir(folder) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if ours(&name).is_some_and(|day| day < cutoff.as_str()) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

/// The day a file of this exporter is of, when `name` is one of its files.
fn ours(name: &str) -> Option<&str> {
    let rest = GROUPS
        .iter()
        .find_map(|group| name.strip_prefix(group)?.strip_prefix('-'))?;
    let day = rest.strip_suffix(".jsonl")?;
    let bytes = day.as_bytes();
    let shaped = bytes.len() == 10
        && bytes.iter().enumerate().all(|(at, byte)| match at {
            4 | 7 => *byte == b'-',
            _ => byte.is_ascii_digit(),
        });
    shaped.then_some(day)
}

fn append(path: &Path, line: &str) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(crate::replace::SHARED);
    }
    // One write, so a line is never cut by another writer's.
    options.open(path)?.write_all(line.as_bytes())
}

impl TelemetryExporter for FileExporter {
    fn describe(&self) -> Result<ExporterInfo, ProviderError> {
        let mut info = ExporterInfo::new("Files");
        info.help = Some(
            "Writes what is recorded to files in a folder on this computer, as OTLP JSON lines."
                .to_owned(),
        );
        Ok(info)
    }

    fn export(&self, batch: Batch) -> Result<(), ProviderError> {
        let folder = self.folder().ok_or_else(|| {
            ExporterError::Bad("there is no folder to write in: name one in `folder`".to_owned())
                .into_provider()
        })?;
        crate::control::private_dir(&folder).map_err(|error| {
            ExporterError::Unavailable(format!("the folder could not be made: {}", error.kind()))
                .into_provider()
        })?;
        let now = now_seconds();
        self.sweep(&folder, now);
        let day = day_of(now);
        let documents = otlp::render(&batch);
        for (group, document) in [
            ("spans", documents.traces),
            ("metrics", documents.metrics),
            ("logs", documents.logs),
        ] {
            let Some(document) = document else { continue };
            let mut line = serde_json::to_string(&document).map_err(|_| {
                ExporterError::Bad("a batch could not be written".to_owned()).into_provider()
            })?;
            line.push('\n');
            append(&folder.join(format!("{group}-{day}.jsonl")), &line).map_err(|error| {
                ExporterError::Unavailable(format!("a file could not be written: {}", error.kind()))
                    .into_provider()
            })?;
        }
        Ok(())
    }

    fn flush(&self) -> Result<(), ProviderError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_this_exporters_files_are_taken_for_its_own() {
        assert_eq!(ours("spans-2026-01-02.jsonl"), Some("2026-01-02"));
        assert_eq!(ours("logs-2026-01-02.jsonl"), Some("2026-01-02"));
        for name in [
            "notes-2026-01-02.jsonl",
            "spans-2026-01-02.txt",
            "spans-2026-1-2.jsonl",
            "spans-today.jsonl",
            "spans.jsonl",
        ] {
            assert_eq!(ours(name), None, "{name}");
        }
    }
}
