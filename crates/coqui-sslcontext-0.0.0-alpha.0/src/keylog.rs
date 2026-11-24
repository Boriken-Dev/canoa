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
/// # Errors
///
/// [`Error::Io`] when the file cannot be opened for appending.
pub(crate) fn key_log(logging: &KeyLogging) -> Result<Option<Arc<dyn rustls::KeyLog>>, Error> {
    Ok(match logging {
        KeyLogging::Off => None,
        KeyLogging::Environment => Some(Arc::new(rustls::KeyLogFile::new())),
        KeyLogging::File(path) => {
            let file = OpenOptions::new().create(true).append(true).open(path)?;
            Some(Arc::new(FileKeyLog {
                file: Mutex::new(file),
            }))
        }
    })
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
