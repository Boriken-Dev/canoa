//! The add-in's log.

use std::io::Write;

/// One line of metadata per event, so a person can see whether and when Office loaded the DLL. No content is ever written.
/// The variable `brand::LOG` names the file; otherwise it is `office-addin.log` in the brand's folder under `%LOCALAPPDATA%`.
pub(crate) fn log(event: &str) {
    let path = match std::env::var_os(crate::brand::LOG) {
        Some(path) => std::path::PathBuf::from(path),
        None => match std::env::var_os("LOCALAPPDATA") {
            Some(base) => std::path::Path::new(&base)
                .join(crate::brand::FOLDER)
                .join("office-addin.log"),
            None => return,
        },
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
    let _ = writeln!(
        file,
        "{ms} pid={} exe={} arch={} {event}",
        std::process::id(),
        exe.as_deref().unwrap_or("?"),
        std::env::consts::ARCH
    );
}
