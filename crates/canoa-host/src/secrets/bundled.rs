//! The secret providers written as scripts that ship inside the host: Bitwarden, with the SDK modules it imports.
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

/// The secret providers that ship in the host.
static BUNDLE: Bundle = Bundle {
    name: "secrets",
    libraries: &["bitwarden"],
    files: &[
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
    ],
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
