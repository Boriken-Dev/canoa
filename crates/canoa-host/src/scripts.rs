//! The providers written in JavaScript that ship inside the host: a bundle is the text of a few libraries (a manifest, a package, the
//! module and the SDK files it imports) built into the program, kept as files in a folder of the user's home because the source of
//! JavaScript reads libraries from folders, and rewritten when a file differs from what the program carries, so what runs is
//! always what shipped. Each kind that has such providers has a bundle of its own; the registry a bundle makes is read like the
//! registry of any extension, so the host treats a bundled provider as it treats one an extension adds.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files are std's"
)]

use std::path::{Path, PathBuf};

use coqui_acp_api::guatiao::library::Registry;
use coqui_acp_api::guatiao::{List, Map};

use crate::provider::add_bundled_js;

/// A set of libraries and the files they are made of.
pub(crate) struct Bundle {
    /// The name of the folder under `bundled/` they are kept in; the host's version and a digest of the files follow it.
    pub(crate) name: &'static str,
    /// The folders of the libraries, relative to the bundle's: each holds a manifest.
    pub(crate) libraries: &'static [&'static str],
    /// Each file, by its path under the bundle's folder, and its text.
    pub(crate) files: &'static [(&'static str, &'static str)],
}

/// Process-wide home override set by `Serving::bind` when no explicit home directory is threaded into bundled script lookups.
static ROOT: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

/// Keeps the bundled files in `home` from now on, as the host that serves `home` does, so a host started on a home of its own writes
/// nothing to another's.
pub(crate) fn use_home(home: &Path) {
    *ROOT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(home.to_path_buf());
}

/// Where bundled files are kept: the home of the host being served, else this user's own, which no other user can write.
pub(crate) fn root() -> PathBuf {
    if let Some(home) = ROOT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
    {
        return home;
    }
    // A test of this crate keeps them in the temporary folder, never in the person's own home.
    #[cfg(test)]
    {
        std::env::temp_dir().join("canoa-bundled-tests")
    }
    #[cfg(not(test))]
    {
        crate::home::Home::of_user().map_or_else(
            |_| std::env::temp_dir().join("canoa-bundled"),
            |home| home.dir().to_path_buf(),
        )
    }
}

impl Bundle {
    /// The folder this bundle is kept in under `home`, named by the host's version and a digest of the files it carries: a host
    /// built with other files reads a folder of its own, so two programs sharing a home never rewrite each other's.
    pub(crate) fn folder(&self, home: &Path) -> PathBuf {
        use sha2::Digest as _;

        let mut digest = sha2::Sha256::new();
        for (path, text) in self.files {
            digest.update((path.len() as u64).to_le_bytes());
            digest.update(path.as_bytes());
            digest.update((text.len() as u64).to_le_bytes());
            digest.update(text.as_bytes());
        }
        let digest = data_encoding::HEXLOWER.encode(&digest.finalize());
        home.join("bundled").join(format!(
            "{}-{}-{}",
            self.name,
            env!("CARGO_PKG_VERSION"),
            &digest[..16]
        ))
    }

    /// Writes the files into `home` where they are missing or differ, and answers the bundle's folder.
    ///
    /// # Errors
    ///
    /// A file that could not be written.
    pub(crate) fn keep(&self, home: &Path) -> Result<PathBuf, String> {
        let folder = self.folder(home);
        for (path, text) in self.files {
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

    /// A registry of the bundle's libraries, read by the source of JavaScript built into the host.
    ///
    /// # Errors
    ///
    /// Why they cannot be offered: their files, or the source.
    pub(crate) fn registry(&self, home: &Path) -> Result<Registry, String> {
        let folder = self.keep(home)?;
        let text = |path: PathBuf| path.to_string_lossy().into_owned();
        let mut config = Map::new();
        config.set(
            "dirs",
            List::from_iter(
                self.libraries
                    .iter()
                    .map(|library| text(folder.join(library))),
            ),
        );
        // The descriptions of every kind a script may serve, as the host gives any source.
        config.set(
            "kinds",
            List::from_iter([text(crate::descriptions::file()?)]),
        );
        let mut registry = Registry::new(canoa::brand::PROGRAM, env!("CARGO_PKG_VERSION"));
        add_bundled_js(&mut registry, &config).map_err(|error| error.to_string())?;
        Ok(registry)
    }
}
