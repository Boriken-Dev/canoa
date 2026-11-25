//! Git Bash for Canoa: `git_bash_store`, Git for Windows' portable release holding `bin/bash.exe` with the tools that go
//! with it and `git`, checked against the SHA-256 GitHub lists. A publisher's signature is not checked.
#![forbid(unsafe_code)]
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files and paths are std's"
)]

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use std::path::Path;

use canoa::status::{internal, unsupported};
use canoa::store::catalog::RELEASES_FORMAT;
use canoa::store::extension::{CatalogWords, DefaultCatalog};
use canoa::store::recipe::{Finish, Recipe, Release, dotted_numbers, github_endpoints, path_has};
use canoa::store::{MANAGER_UNKNOWN, Platform, StoreInfo, Updatable};
use canoa::store_provider;
use coqui_acp_api::guatiao::library::ProviderError;

/// The name this crate's library is registered under.
pub const LIBRARY: &str = "canoa-git-bash";

/// The release listing of Git for Windows.
pub const RELEASES_API: &str = "https://api.github.com/repos/git-for-windows/git/releases";

/// Where Git for Windows' release files are served from.
pub const DOWNLOADS: &str = "https://github.com/git-for-windows/git/releases/download/";

/// The name the self-extracting archive is kept under in the version's folder until it has been run.
const SELF_EXTRACTING: &str = "PortableGit.7z.exe";

mod source;

pub use source::{GitBashReleases, SOURCE, catalog_of};

store_provider!(
    /// The Git Bash store.
    GitBashStore, GitBashRecipe, "git_bash_store"
);

// What this crate offers as a library the host links: handed to `Registry::register_local`, the same way a loaded library is read.
coqui_acp_api::guatiao::local_providers!(
    id = "canoa-git-bash",
    version = env!("CARGO_PKG_VERSION"),
    providers = [GitBashStore, GitBashReleases]
);

/// The version in a line of `git --version`: `git version 2.56.1.windows.1` is `2.56.1`, and `...2.56.0.windows.4` is `2.56.0.4`,
/// the way the release's files spell them.
fn git_version(line: &str) -> Option<String> {
    let word = line.trim().strip_prefix("git version ")?.trim();
    let (base, again) = match word.split_once(".windows.") {
        Some((base, again)) => (base, again.parse::<u64>().ok()?),
        None => (word, 1),
    };
    dotted_numbers::<3>(base)?;
    Some(if again == 1 {
        base.to_owned()
    } else {
        format!("{base}.{again}")
    })
}

/// Git for Windows' portable release, which holds `bin/bash.exe` with the tools that go with it, and `git`.
#[derive(Debug, Default, Clone, Copy)]
pub struct GitBashRecipe;

impl Recipe for GitBashRecipe {
    fn info(&self) -> StoreInfo {
        let mut info = StoreInfo::new("git-bash", "Git Bash", "Git for Windows");
        info.homepage = Some("https://gitforwindows.org".to_owned());
        info.summary = Some(
            "The bash shell of Git for Windows, with the tools that go with it and git: what an agent such as Claude Code runs commands with on Windows, and a shell for the Terminal.".to_owned(),
        );
        info.icon = Some("branch".to_owned());
        info.commands = ["bash", "sh", "git"].map(str::to_owned).into();
        info.platforms = ["x86_64", "aarch64"]
            .map(|arch| Platform::new("windows", arch))
            .into();
        info.endpoints = github_endpoints(RELEASES_API, DOWNLOADS);
        info.catalogs = Some(
            CatalogWords::new("source", "sources", "releases")
                .in_format(RELEASES_FORMAT)
                .registering([DefaultCatalog::new("github", SOURCE)
                    .with_fields(serde_json::json!({"repo": source::REPO}).to_string())]),
        );
        info
    }

    fn names(&self) -> &'static [&'static str] {
        &["bash"]
    }

    fn dirs(&self) -> &'static [&'static str] {
        &[
            "%ProgramFiles%/Git/bin",
            "%ProgramFiles(x86)%/Git/bin",
            "%LocalAppData%/Programs/Git/bin",
        ]
    }

    /// What a found `bash` is asked: the version of the `git` it finds, which is the version a release is named by. A bash that
    /// has no git prints no version, and the copy is listed without one.
    fn version_args(&self) -> &'static [&'static str] {
        &["-c", "git --version"]
    }

    fn parse_version(&self, line: &str) -> Option<String> {
        git_version(line)
    }

    fn classify(&self, path: &str) -> (&'static str, Updatable) {
        let elsewhere = |how: &str| Updatable::Elsewhere {
            how: how.to_owned(),
        };
        if path_has(path, "/windows/system32/") || path_has(path, "/windows/sysnative/") {
            (
                MANAGER_UNKNOWN,
                elsewhere("the package manager of the Linux distribution it starts"),
            )
        } else if path_has(path, "msys") {
            (MANAGER_UNKNOWN, elsewhere("pacman -Syu, in MSYS2"))
        } else if path_has(path, "/git/") {
            (
                MANAGER_UNKNOWN,
                elsewhere("git update-git-for-windows, or Git for Windows' installer"),
            )
        } else {
            (
                MANAGER_UNKNOWN,
                Updatable::No {
                    why: "its origin is not known".to_owned(),
                },
            )
        }
    }

    fn launch_environment(&self) -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    fn adjust(&self, mut release: Release, platform: &Platform) -> Result<Release, ProviderError> {
        if platform.os != "windows" {
            return Err(unsupported(
                "Git Bash is Git for Windows' own: other systems have a bash of their own",
            ));
        }
        for part in &mut release.parts {
            part.path = SELF_EXTRACTING.to_owned();
            part.executable = true;
        }
        release.launch = "git/bin/bash.exe".to_owned();
        release.steps.push(
            "Run the downloaded self-extracting archive once, with `-y` and `-o` naming this version's own folder, and delete it"
                .to_owned(),
        );
        release.notices.push(format!(
            "Git for Windows {} is checked against the SHA-256 GitHub lists for the release's file, whose publisher signature is not checked by this store.",
            release.version
        ));
        release.notices.push(
            "The portable release is a self-extracting program of the publisher's: it is started once to unpack itself into this folder, and writes no registry entry (its own README says so). A Git for Windows the person installed stays as it is."
                .to_owned(),
        );
        Ok(release)
    }

    fn version_command(
        &self,
        _path: &str,
        staging: &str,
        _release: &Release,
    ) -> (String, Vec<String>) {
        // `bash --version` prints bash's own version; the release is named by git's, which sits in the same folder.
        let git = Path::new(staging).join("git/bin/git.exe");
        (
            git.to_string_lossy().into_owned(),
            vec!["--version".to_owned()],
        )
    }

    fn finish(&self, finish: &Finish<'_>) -> Result<(), ProviderError> {
        let staging = Path::new(finish.staging);
        let archive = staging.join(SELF_EXTRACTING);
        let into = staging.join("git");
        let output = finish.run.exec(
            &archive.to_string_lossy(),
            &["-y", &format!("-o{}", into.to_string_lossy())],
            &[],
        )?;
        if output.code != Some(0) {
            return Err(internal(
                "the self-extracting archive ended with an error: nothing was installed",
            ));
        }
        std::fs::remove_file(&archive)
            .map_err(|error| internal(format!("{SELF_EXTRACTING}: {error}")))
    }
}

#[cfg(test)]
mod tests {
    use canoa::store::ProgramStore;
    use coqui_acp_api::guatiao::library::Registry;

    use super::*;

    #[test]
    fn the_library_offers_the_git_bash_store() {
        let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
        let loading = registry
            .register_local(LIBRARY, library)
            .expect("registers");
        assert!(loading.loaded().is_some(), "accepted");
        assert!(
            registry
                .offer::<dyn ProgramStore>("git_bash_store")
                .is_some()
        );
    }

    #[test]
    fn a_version_is_what_the_program_prints_and_nothing_around_it() {
        assert_eq!(
            git_version("git version 2.56.1.windows.1\n").as_deref(),
            Some("2.56.1")
        );
        assert_eq!(
            git_version("git version 2.56.0.windows.4").as_deref(),
            Some("2.56.0.4")
        );
        assert_eq!(git_version("GNU bash, version 5.3.15(2)-release"), None);
    }
}
