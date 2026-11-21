//! Git Bash for Canoa: `git_bash_store`, Git for Windows' portable release holding `bin/bash.exe` with the tools that go
//! with it and `git`, checked against the SHA-256 GitHub lists. A publisher's signature is not checked, and the plan says so.
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

use canoa::status::{bad_value, internal, unsupported};
use canoa::store::recipe::{
    Finish, Part, Published, Recipe, Release, dotted_numbers, get_json, github_assets,
    github_endpoints, newest_published, path_has, pinned, spell,
};
use canoa::store::{
    Check, Download, InstallTools, LatestReport, MANAGER_UNKNOWN, Platform, StoreInfo, Updatable,
};
use canoa::store_provider;
use coqui_acp_api::guatiao::library::{Object, ProviderError};

/// The name this crate's library is registered under.
pub const LIBRARY: &str = "canoa-git-bash";

/// The release listing of Git for Windows.
pub const RELEASES_API: &str = "https://api.github.com/repos/git-for-windows/git/releases";

/// Where Git for Windows' release files are served from.
pub const DOWNLOADS: &str = "https://github.com/git-for-windows/git/releases/download/";

/// How many releases of a listing are read.
const LISTED: u32 = 30;

/// The name the self-extracting archive is kept under in the version's folder until it has been run.
const SELF_EXTRACTING: &str = "PortableGit.7z.exe";

store_provider!(
    /// The Git Bash store.
    GitBashStore, GitBashRecipe, "git_bash_store"
);

// What this crate offers as a library the host links: handed to `Registry::register_local`, the same way a loaded library is read.
coqui_acp_api::guatiao::local_providers!(
    id = "canoa-git-bash",
    version = env!("CARGO_PKG_VERSION"),
    providers = [GitBashStore]
);

/// The releases of Git for Windows in a listing, in no order: tag `v2.56.1.windows.1` is `2.56.1`, and `v2.56.0.windows.4` is
/// `2.56.0.4`, as the release's files spell them. A draft, a prerelease and any other tag are left out.
fn git_releases(listing: &serde_json::Value) -> Vec<Published<4>> {
    listing
        .as_array()
        .into_iter()
        .flatten()
        .filter(|release| release["draft"] != true && release["prerelease"] != true)
        .filter_map(|release| {
            let tag = release["tag_name"].as_str()?;
            let (base, again) = tag.strip_prefix('v')?.split_once(".windows.")?;
            let [major, minor, patch] = dotted_numbers::<3>(base)?;
            let again = again.parse::<u64>().ok()?;
            let version = if again == 1 {
                base.to_owned()
            } else {
                format!("{base}.{again}")
            };
            Some(Published {
                version,
                order: [major, minor, patch, again],
                tag: tag.to_owned(),
                assets: github_assets(release),
            })
        })
        .collect()
}

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

    fn latest(&self, _channel: Option<&str>, tools: &Object<dyn InstallTools>) -> LatestReport {
        match get_json(tools, &format!("{RELEASES_API}?per_page={LISTED}"))
            .map(|doc| newest_published(git_releases(&doc)))
            .and_then(|found| {
                found.ok_or_else(|| internal("the repository lists no portable release"))
            }) {
            Ok(release) => LatestReport::known(release.version),
            Err(error) => LatestReport::unknown(error.message().to_owned()),
        }
    }

    fn release(
        &self,
        version: Option<&str>,
        _channel: Option<&str>,
        platform: &Platform,
        tools: &Object<dyn InstallTools>,
    ) -> Result<Release, ProviderError> {
        if platform.os != "windows" {
            return Err(unsupported(
                "Git Bash is Git for Windows' own: other systems have a bash of their own",
            ));
        }
        let arch = spell(
            &platform.arch,
            &[("x86_64", "64-bit"), ("aarch64", "arm64")],
        )
        .ok_or_else(|| unsupported("Git for Windows is not published for this processor"))?;
        let doc = get_json(tools, &format!("{RELEASES_API}?per_page={LISTED}"))?;
        let all = git_releases(&doc);
        let release = match version {
            Some(wanted) => {
                let wanted = pinned(wanted)?;
                all.into_iter().find(|release| release.version == wanted)
            }
            None => newest_published(all),
        }
        .ok_or_else(|| unsupported("the repository lists no such release"))?;
        let name = format!("PortableGit-{}-{arch}.7z.exe", release.version);
        let digest = release.digest_of(&name).ok_or_else(|| {
            bad_value(format!(
                "release {} lists no {name} with a digest",
                release.tag
            ))
        })?;
        let download = Download::new(
            format!("{DOWNLOADS}{}/{name}", release.tag),
            None,
            Check::Sha256 {
                value: digest.to_owned(),
            },
        );
        let mut made = Release::new(
            release.version.clone(),
            "git/bin/bash.exe",
            vec![Part::new(download, SELF_EXTRACTING).executable()],
        );
        made.steps.push(
            "Run the downloaded self-extracting archive once, with `-y` and `-o` naming this version's own folder, and delete it"
                .to_owned(),
        );
        made.notices.push(format!(
            "Git for Windows {} is checked against the SHA-256 GitHub lists for the release's file, whose publisher signature is not checked by this store.",
            release.version
        ));
        made.notices.push(
            "The portable release is a self-extracting program of the publisher's: it is started once to unpack itself into this folder, and writes no registry entry (its own README says so). A Git for Windows the person installed stays as it is."
                .to_owned(),
        );
        Ok(made)
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

    fn digest(seed: &str) -> String {
        format!("sha256:{}", seed.repeat(64)[..64].to_owned())
    }

    fn git_listing() -> serde_json::Value {
        serde_json::json!([
            {"tag_name": "v2.56.1.windows.1", "draft": false, "prerelease": false, "assets": [
                {"name": "PortableGit-2.56.1-64-bit.7z.exe", "digest": digest("a")},
                {"name": "MinGit-2.56.1-64-bit.zip", "digest": digest("b")}]},
            {"tag_name": "v2.56.0.windows.4", "draft": false, "prerelease": false, "assets": [
                {"name": "PortableGit-2.56.0.4-64-bit.7z.exe", "digest": digest("c")}]},
            {"tag_name": "v2.56.0-rc2.windows.1", "draft": false, "prerelease": true, "assets": []},
            {"tag_name": "v2.99.0.windows.1", "draft": true, "assets": []},
            {"tag_name": "nothing", "draft": false, "assets": []}
        ])
    }

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
    fn git_for_windows_releases_are_named_the_way_their_files_are() {
        let found = git_releases(&git_listing());
        let versions: Vec<&str> = found.iter().map(|each| each.version.as_str()).collect();
        assert_eq!(versions, ["2.56.1", "2.56.0.4"]);
        assert_eq!(
            newest_published(found).map(|each| each.version).as_deref(),
            Some("2.56.1"),
            "by number, so 2.56.0.4 is older than 2.56.1"
        );
    }

    #[test]
    fn a_digest_is_the_one_github_lists_for_that_exact_file() {
        let found = git_releases(&git_listing());
        let first = &found[0];
        assert_eq!(
            first.digest_of("PortableGit-2.56.1-64-bit.7z.exe"),
            Some("a".repeat(64).as_str())
        );
        assert_eq!(first.digest_of("PortableGit-2.56.1-arm64.7z.exe"), None);
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
