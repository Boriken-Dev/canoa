//! What the host's home remembers of its last run (`run.json`: `version`, `started_at`, `ended`), so the next start can say whether
//! the host restarted or was updated. A home with no file is a first run and fires nothing.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files are std's"
)]

use std::path::Path;

use serde_json::{Value, json};

/// The file in the host's home.
pub(crate) const FILE: &str = "run.json";

/// How this start compares with the last run in the same home.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Start {
    /// No earlier run is recorded.
    First,
    /// The last run had the same version.
    Restarted,
    /// The last run had another version.
    Updated,
}

fn seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

fn write(home: &Path, version: &str, started: u64, ended: bool) {
    let text = json!({"version": version, "started_at": started, "ended": ended}).to_string();
    if let Err(error) =
        std::fs::create_dir_all(home).and_then(|()| std::fs::write(home.join(FILE), text))
    {
        tracing::warn!("the run record could not be written: {error}");
    }
}

/// Records this start in `home` and says how it compares with the last one.
pub(crate) fn begin(home: &Path, version: &str) -> Start {
    let last = std::fs::read(home.join(FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|value| {
            value
                .get("version")
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
    write(home, version, seconds(), false);
    match last {
        None => Start::First,
        Some(last) if last == version => Start::Restarted,
        Some(_) => Start::Updated,
    }
}

/// Records that the run that began at `started` ended in order.
pub(crate) fn end(home: &Path, version: &str, started: u64) {
    write(home, version, started, true);
}

/// The run this host is: recorded as ended in order when it is dropped.
#[derive(Debug)]
pub(crate) struct Record {
    home: std::path::PathBuf,
    version: String,
    started: u64,
}

impl Record {
    /// Records the start of a run in `home`, and says how it compares with the last one.
    pub(crate) fn begin(home: &Path, version: &str) -> (Self, Start) {
        let start = begin(home, version);
        (
            Self {
                home: home.to_owned(),
                version: version.to_owned(),
                started: now(),
            },
            start,
        )
    }
}

impl Drop for Record {
    fn drop(&mut self) {
        end(&self.home, &self.version, self.started);
    }
}

/// The time a run begun now is recorded with.
pub(crate) fn now() -> u64 {
    seconds()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Home(std::path::PathBuf);

    impl Drop for Home {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_first_start_a_restart_and_an_update_are_told_apart() {
        let folder = std::env::temp_dir().join(format!("canoa-run-record-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("a folder");
        let home = Home(folder);
        assert_eq!(begin(home.0.as_path(), "1.0.0"), Start::First);
        assert_eq!(begin(home.0.as_path(), "1.0.0"), Start::Restarted);
        end(home.0.as_path(), "1.0.0", now());
        assert_eq!(begin(home.0.as_path(), "1.0.0"), Start::Restarted);
        assert_eq!(begin(home.0.as_path(), "1.1.0"), Start::Updated);
        std::fs::write(home.0.as_path().join(FILE), "not json").expect("written");
        assert_eq!(begin(home.0.as_path(), "1.1.0"), Start::First);
    }
}
