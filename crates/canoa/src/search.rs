//! Where a library looks up a program by name: the `search_path` setting the host fills in for a provider that declares one.
//!
//! The host's search path is the person's own `PATH` followed by the folders of the programs the Store installed, so a library
//! finds a program the Store holds without the host changing the environment of the process it runs in.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// The setting a provider declares (an optional string holding a `PATH`) to be given the host's search path.
pub const SETTING: &str = "search_path";

/// The first runnable file called `name` in an absolute folder of `search_path`. On Windows a name with none of `pathext`'s
/// extensions takes each of them in turn (the usual four when `pathext` is empty).
#[must_use]
pub fn find_in(search_path: &OsStr, pathext: &str, name: &str) -> Option<PathBuf> {
    let suffixes: Vec<String> = if cfg!(windows) {
        let listed: Vec<String> = pathext
            .split(';')
            .filter(|ext| ext.starts_with('.') && ext.len() > 1)
            .map(str::to_ascii_lowercase)
            .collect();
        let listed = if listed.is_empty() {
            vec![".com".into(), ".exe".into(), ".bat".into(), ".cmd".into()]
        } else {
            listed
        };
        if listed
            .iter()
            .any(|ext| name.to_ascii_lowercase().ends_with(ext))
        {
            vec![String::new()]
        } else {
            listed
        }
    } else {
        vec![String::new()]
    };
    std::env::split_paths(search_path)
        .filter(|directory| directory.is_absolute())
        .flat_map(|directory| {
            suffixes
                .iter()
                .map(move |suffix| directory.join(format!("{name}{suffix}")))
        })
        .find(|candidate| runnable(candidate))
}

fn runnable(file: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(file) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}
