//! The secret providers written as scripts that extension crates carry, such as Bitwarden, kept where the host reads libraries from.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files are std's"
)]

use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;

use coqui_acp_api::guatiao::library::Registry;

use crate::scripts::Bundle;

/// The secret providers written as scripts that the extension crates carry: what a crate offers is its own, and the host only keeps
/// the files where its JavaScript source reads libraries.
static BUNDLE: Bundle = Bundle {
    name: "secrets",
    libraries: canoa_bitwarden::SCRIPT_LIBRARIES,
    files: canoa_bitwarden::SCRIPT_FILES,
};

/// Writes the bundled files into the host's home where they are missing or differ, and answers the folder.
///
/// # Errors
///
/// A file that could not be written.
#[cfg(test)]
pub(super) fn keep(home: &Path) -> Result<PathBuf, String> {
    BUNDLE.keep(home)
}

/// A registry of the bundled script providers, read by the source of JavaScript built into the host.
///
/// # Errors
///
/// Why they cannot be offered: their files, or the source.
pub(super) fn registry(home: &Path) -> Result<Registry, String> {
    BUNDLE.registry(home)
}
