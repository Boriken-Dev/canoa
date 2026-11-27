//! Session secrets written for a traffic analyser, when asked.

use alloc::string::String;
use alloc::sync::Arc;
use core::fmt;
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::Mutex;

use crate::error::Error;
use crate::settings::KeyLogging;

/// The rustls key logger for `logging`, or `None` for [`KeyLogging::Off`].
///
/// A new file is private to its owner on Unix (mode 0600). On Windows it gets the directory's
/// inherited access: no owner-only ACL is set, so keep the file in a private directory. A file that
/// already exists with wider access is used and a warning is logged.
///
/// # Errors
///
/// [`Error::Io`] when the file cannot be opened for appending.
pub(crate) fn key_log(logging: &KeyLogging) -> Result<Option<Arc<dyn rustls::KeyLog>>, Error> {
    Ok(match logging {
        KeyLogging::Off => None,
        KeyLogging::Environment => Some(Arc::new(rustls::KeyLogFile::new())),
        KeyLogging::File(path) => {
            let file = open_private(path)?;
            Some(Arc::new(FileKeyLog {
                file: Mutex::new(file),
            }))
        }
    })
}

#[cfg(unix)]
fn open_private(path: &std::path::Path) -> Result<std::fs::File, Error> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)?;
    let mode = file.metadata()?.mode();
    if group_or_other_access(mode) {
        tracing::warn!(
            path = %path.display(),
            mode = format_args!("{:o}", mode & 0o777),
            "the key log is readable by others; anyone who can read it can read the traffic"
        );
    }
    Ok(file)
}

#[cfg(not(unix))]
fn open_private(path: &std::path::Path) -> Result<std::fs::File, Error> {
    Ok(OpenOptions::new().create(true).append(true).open(path)?)
}

/// Whether a Unix mode gives the group or others any access.
#[cfg(unix)]
fn group_or_other_access(mode: u32) -> bool {
    mode & 0o077 != 0
}

/// Appends lines in the NSS key log format: `LABEL <client random hex> <secret hex>`.
struct FileKeyLog {
    file: Mutex<std::fs::File>,
}

impl fmt::Debug for FileKeyLog {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("FileKeyLog").finish_non_exhaustive()
    }
}

fn hex(bytes: &[u8]) -> String {
    use core::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}

impl rustls::KeyLog for FileKeyLog {
    fn log(&self, label: &str, client_random: &[u8], secret: &[u8]) {
        let line = alloc::format!("{label} {} {}\n", hex(client_random), hex(secret));
        let mut file = self
            .file
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Err(error) = file.write_all(line.as_bytes()) {
            tracing::warn!(%error, "the key log could not be written");
        }
    }
}
