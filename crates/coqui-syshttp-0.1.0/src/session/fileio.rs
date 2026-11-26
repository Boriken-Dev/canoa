//! Reading and writing files that hold secrets: owner-only and replaced whole.

use alloc::string::String;
use core::time::Duration;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// How often a replace or a read is tried again when another handle holds the file: on Windows a
/// file being replaced refuses to open for an instant.
const TRIES: u32 = 10;
const WAIT: Duration = Duration::from_millis(10);

/// The UTF-8 text of `path`, undecodable bytes dropped.
pub(crate) fn read_text(path: &Path) -> io::Result<String> {
    let mut attempt = 0;
    let bytes = loop {
        match fs::read(path) {
            Err(error)
                if error.kind() == io::ErrorKind::PermissionDenied && attempt + 1 < TRIES =>
            {
                attempt += 1;
                std::thread::sleep(WAIT);
            }
            other => break other?,
        }
    };
    Ok(bytes.utf8_chunks().map(|chunk| chunk.valid()).collect())
}

fn make_directories(directory: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(directory)
}

/// Writes `data` to `path` so that a reader sees the old file or the new one, never part of
/// either, and only the owner can read it (mode `0600`; missing parent directories `0700`). The
/// bytes are written as given: no newline translation.
pub(crate) fn write_private(path: &Path, data: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    if let Some(parent) = parent {
        make_directories(parent)?;
    }
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let directory = parent.unwrap_or_else(|| Path::new("."));
    let (temporary, mut file) = create_temporary(directory, &name)?;
    let result = (|| {
        file.write_all(data)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        replace(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn create_temporary(directory: &Path, name: &str) -> io::Result<(std::path::PathBuf, fs::File)> {
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let mut last = io::Error::from(io::ErrorKind::AlreadyExists);
    for round in 0..TRIES {
        let temporary = directory.join(alloc::format!(
            "{name}.{:x}-{:x}-{round}.tmp",
            std::process::id(),
            seed
        ));
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&temporary) {
            Ok(file) => return Ok((temporary, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => last = error,
            Err(error) => return Err(error),
        }
    }
    Err(last)
}

fn replace(from: &Path, to: &Path) -> io::Result<()> {
    let mut attempt = 0;
    loop {
        match fs::rename(from, to) {
            Err(error)
                if error.kind() == io::ErrorKind::PermissionDenied && attempt + 1 < TRIES =>
            {
                attempt += 1;
                std::thread::sleep(WAIT);
            }
            other => return other,
        }
    }
}
