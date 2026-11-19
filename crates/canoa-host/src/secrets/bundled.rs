//! The secret providers written as scripts that ship inside the host: their files are kept in the host's home, so the source of
//! JavaScript that is built into the host reads them, and are rewritten when they differ from what the host carries, so what runs
//! is always what shipped.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files are std's"
)]

use std::path::{Path, PathBuf};

use coqui_acp_api::guatiao::library::Registry;
use coqui_acp_api::guatiao::{List, Map};

use crate::provider::add_bundled_js;

/// Each file of the bundled libraries, by its path under the folder they are kept in.
const FILES: [(&str, &str); 6] = [
    (
        "bitwarden/bitwarden.js",
        include_str!("../../extensions/bitwarden/bitwarden.js"),
    ),
    (
        "bitwarden/package.json",
        include_str!("../../extensions/bitwarden/package.json"),
    ),
    (
        "bitwarden/canoa_bitwarden.guatiao",
        include_str!("../../extensions/bitwarden/canoa_bitwarden.guatiao"),
    ),
    // The SDK's modules the script imports, kept beside it.
    (
        "bitwarden/sdk/secret.js",
        include_str!("../../../../bindings/js/canoa-sdk/src/secret.js"),
    ),
    (
        "bitwarden/sdk/kinds.js",
        include_str!("../../../../bindings/js/canoa-sdk/src/kinds.js"),
    ),
    (
        "bitwarden/sdk/model.js",
        include_str!("../../../../bindings/js/canoa-sdk/src/model.js"),
    ),
];

/// Where the files are kept under the host's home, and so what a library there is told apart from an older host's.
fn folder(home: &Path) -> PathBuf {
    home.join("bundled")
        .join(format!("secrets-{}", env!("CARGO_PKG_VERSION")))
}

/// Writes the bundled files into the host's home where they are missing or differ, and answers the folder.
///
/// # Errors
///
/// A file that could not be written.
pub(super) fn keep(home: &Path) -> Result<PathBuf, String> {
    let folder = folder(home);
    for (path, text) in FILES {
        let file = folder.join(path);
        if std::fs::read(&file).is_ok_and(|have| have == text.as_bytes()) {
            continue;
        }
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("{}: {error}", parent.display()))?;
        }
        std::fs::write(&file, text).map_err(|error| format!("{}: {error}", file.display()))?;
    }
    Ok(folder)
}

/// A registry of the bundled script providers, read by the source of JavaScript built into the host.
///
/// # Errors
///
/// Why they cannot be offered: their files, or the source.
pub(super) fn registry(home: &Path) -> Result<Registry, String> {
    let folder = keep(home)?;
    let text = |path: PathBuf| path.to_string_lossy().into_owned();
    let mut config = Map::new();
    config.set("dirs", List::from_iter([text(folder.join("bitwarden"))]));
    // The descriptions of every kind a script may serve, as the host gives any source.
    config.set(
        "kinds",
        List::from_iter([text(crate::descriptions::file()?)]),
    );
    let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
    add_bundled_js(&mut registry, &config).map_err(|error| error.to_string())?;
    Ok(registry)
}
