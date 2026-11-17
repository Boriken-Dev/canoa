//! The shells of the Runtimes type: Git Bash (the Windows bash that agents such as Claude Code run commands with) and PowerShell.
//! Both are published on GitHub, and the release listing carries each file's SHA-256, which the download is checked against.
//!
//! [`GitBashRecipe`] is Git for Windows' own portable release, a self-extracting archive that the host runs once with `-y -o` in
//! the version's folder; its `PortableGit-*` files are the only ones that hold `bash.exe` and the tools a shell needs (MinGit
//! does not). [`PowerShellRecipe`] is the zip (Windows) or `tar.gz` (Linux) of Microsoft's PowerShell 7. A publisher's
//! signature is not checked, and each plan says so.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files and paths are std's"
)]

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use std::path::Path;

use canoa::store::recipe::{
    Finish, Part, Recipe, Release, UnpackStep, bad_value_error, get_json, internal_error, path_has,
    pinned, spell, unsupported_error,
};
use canoa::store::{
    Check, Download, InstallTools, LatestReport, MANAGER_UNKNOWN, Platform, StoreInfo, Updatable,
};
use coqui_acp_api::guatiao::library::{Object, ProviderError};

use super::builtin::store_provider;

/// The release listing of Git for Windows.
pub const GIT_RELEASES_API: &str = "https://api.github.com/repos/git-for-windows/git/releases";
/// Where Git for Windows' release files are served from.
pub const GIT_DOWNLOADS: &str = "https://github.com/git-for-windows/git/releases/download/";
/// The release listing of PowerShell.
pub const POWERSHELL_RELEASES_API: &str =
    "https://api.github.com/repos/PowerShell/PowerShell/releases";
/// Where PowerShell's release files are served from.
pub const POWERSHELL_DOWNLOADS: &str =
    "https://github.com/PowerShell/PowerShell/releases/download/";

/// The hosts GitHub sends a release download on to.
const ASSET_HOSTS: [&str; 2] = [
    "https://release-assets.githubusercontent.com",
    "https://objects.githubusercontent.com",
];

/// How many releases of a listing are read: both repositories publish many more than the newest.
const LISTED: u32 = 30;

/// The name the self-extracting archive is kept under in the version's folder until it has been run.
const SELF_EXTRACTING: &str = "PortableGit.7z.exe";

store_provider!(
    /// The Git Bash store.
    GitBashStore, GitBashRecipe, "git_bash_store"
);
store_provider!(
    /// The PowerShell store.
    PowerShellStore, PowerShellRecipe, "powershell_store"
);

fn digest_of(asset: &serde_json::Value) -> Option<String> {
    let digest = asset["digest"].as_str()?.strip_prefix("sha256:")?;
    (digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| digest.to_ascii_lowercase())
}

/// Each asset of a release that lists a plain SHA-256: its name and the digest.
fn assets_of(release: &serde_json::Value) -> Vec<(String, String)> {
    release["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|asset| Some((asset["name"].as_str()?.to_owned(), digest_of(asset)?)))
        .collect()
}

fn numbers<const N: usize>(text: &str) -> Option<[u64; N]> {
    let mut parts = text.split('.').map(|part| part.parse::<u64>().ok());
    let mut found = [0; N];
    for slot in &mut found {
        *slot = parts.next()??;
    }
    parts.next().is_none().then_some(found)
}

/// One published release: its version as the files name it, what orders it, its tag and its digested assets.
struct Published<const N: usize> {
    version: String,
    order: [u64; N],
    tag: String,
    assets: Vec<(String, String)>,
}

impl<const N: usize> Published<N> {
    fn digest_of(&self, name: &str) -> Option<&str> {
        self.assets
            .iter()
            .find(|(asset, _)| asset == name)
            .map(|(_, digest)| digest.as_str())
    }
}

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
            let [major, minor, patch] = numbers::<3>(base)?;
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
                assets: assets_of(release),
            })
        })
        .collect()
}

/// The releases of PowerShell in a listing: tag `v7.7.0` is `7.7.0`. A draft, a prerelease (`v7.7.0-rc.1`) and any other tag are
/// left out.
fn powershell_releases(listing: &serde_json::Value) -> Vec<Published<3>> {
    listing
        .as_array()
        .into_iter()
        .flatten()
        .filter(|release| release["draft"] != true && release["prerelease"] != true)
        .filter_map(|release| {
            let tag = release["tag_name"].as_str()?;
            let version = tag.strip_prefix('v')?;
            Some(Published {
                version: version.to_owned(),
                order: numbers::<3>(version)?,
                tag: tag.to_owned(),
                assets: assets_of(release),
            })
        })
        .collect()
}

fn newest<const N: usize>(all: Vec<Published<N>>) -> Option<Published<N>> {
    all.into_iter().max_by_key(|release| release.order)
}

fn listing(
    api: &str,
    tools: &Object<dyn InstallTools>,
) -> Result<serde_json::Value, ProviderError> {
    get_json(tools, &format!("{api}?per_page={LISTED}"))
}

fn endpoints(api: &str, downloads: &str) -> Vec<String> {
    [api.to_owned(), downloads.to_owned()]
        .into_iter()
        .chain(ASSET_HOSTS.iter().map(|host| (*host).to_owned()))
        .collect()
}

/// Git for Windows' portable release, which holds `bin/bash.exe` with the tools that go with it, and `git`.
#[derive(Debug, Default, Clone, Copy)]
pub struct GitBashRecipe;

/// The platforms Git for Windows publishes a portable release for.
fn git_platforms() -> Vec<Platform> {
    ["x86_64", "aarch64"]
        .map(|arch| Platform::new("windows", arch))
        .into()
}

/// The version in a line of `git --version`: `git version 2.56.1.windows.1` is `2.56.1`, and `...2.56.0.windows.4` is `2.56.0.4`,
/// the way the release's files spell them.
fn git_version(line: &str) -> Option<String> {
    let word = line.trim().strip_prefix("git version ")?.trim();
    let (base, again) = match word.split_once(".windows.") {
        Some((base, again)) => (base, again.parse::<u64>().ok()?),
        None => (word, 1),
    };
    numbers::<3>(base)?;
    Some(if again == 1 {
        base.to_owned()
    } else {
        format!("{base}.{again}")
    })
}

impl Recipe for GitBashRecipe {
    fn info(&self) -> StoreInfo {
        let mut info = StoreInfo::new("git-bash", "Git Bash", "Git for Windows");
        info.homepage = Some("https://gitforwindows.org".to_owned());
        info.summary = Some(
            "The bash shell of Git for Windows, with the tools that go with it and git: what an agent such as Claude Code runs commands with on Windows, and a shell for the Terminal.".to_owned(),
        );
        info.icon = Some("branch".to_owned());
        info.commands = ["bash", "sh", "git"].map(str::to_owned).into();
        info.platforms = git_platforms();
        info.endpoints = endpoints(GIT_RELEASES_API, GIT_DOWNLOADS);
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
        match listing(GIT_RELEASES_API, tools)
            .map(|doc| newest(git_releases(&doc)))
            .and_then(|found| {
                found.ok_or_else(|| internal_error("the repository lists no portable release"))
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
            return Err(unsupported_error(
                "Git Bash is Git for Windows' own: other systems have a bash of their own",
            ));
        }
        let arch = spell(
            &platform.arch,
            &[("x86_64", "64-bit"), ("aarch64", "arm64")],
        )
        .ok_or_else(|| unsupported_error("Git for Windows is not published for this processor"))?;
        let doc = listing(GIT_RELEASES_API, tools)?;
        let all = git_releases(&doc);
        let release = match version {
            Some(wanted) => {
                let wanted = pinned(wanted)?;
                all.into_iter().find(|release| release.version == wanted)
            }
            None => newest(all),
        }
        .ok_or_else(|| unsupported_error("the repository lists no such release"))?;
        let name = format!("PortableGit-{}-{arch}.7z.exe", release.version);
        let digest = release.digest_of(&name).ok_or_else(|| {
            bad_value_error(format!(
                "release {} lists no {name} with a digest",
                release.tag
            ))
        })?;
        let download = Download::new(
            format!("{GIT_DOWNLOADS}{}/{name}", release.tag),
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
            return Err(internal_error(
                "the self-extracting archive ended with an error: nothing was installed",
            ));
        }
        std::fs::remove_file(&archive)
            .map_err(|error| internal_error(format!("{SELF_EXTRACTING}: {error}")))
    }
}

/// PowerShell 7, from Microsoft's own releases.
#[derive(Debug, Default, Clone, Copy)]
pub struct PowerShellRecipe;

/// The version in a line of `pwsh --version`: `PowerShell 7.7.0` is `7.7.0`.
fn powershell_version(line: &str) -> Option<String> {
    let word = line.trim().strip_prefix("PowerShell ")?.trim();
    numbers::<3>(word).map(|_| word.to_owned())
}

impl Recipe for PowerShellRecipe {
    fn info(&self) -> StoreInfo {
        let mut info = StoreInfo::new("powershell", "PowerShell", "Microsoft");
        info.homepage = Some("https://github.com/PowerShell/PowerShell".to_owned());
        info.summary = Some(
            "PowerShell 7, on Windows and Linux: a shell for the Terminal and what an agent that runs PowerShell commands finds as pwsh.".to_owned(),
        );
        info.icon = Some("prompt".to_owned());
        info.commands = vec!["pwsh".to_owned()];
        info.platforms = ["windows", "linux"]
            .into_iter()
            .flat_map(|os| ["x86_64", "aarch64"].map(|arch| Platform::new(os, arch)))
            .collect();
        info.endpoints = endpoints(POWERSHELL_RELEASES_API, POWERSHELL_DOWNLOADS);
        info
    }

    fn names(&self) -> &'static [&'static str] {
        &["pwsh"]
    }

    fn dirs(&self) -> &'static [&'static str] {
        &["%ProgramFiles%/PowerShell/7", "~/.dotnet/tools"]
    }

    fn version_args(&self) -> &'static [&'static str] {
        &["--version"]
    }

    fn parse_version(&self, line: &str) -> Option<String> {
        powershell_version(line)
    }

    fn classify(&self, path: &str) -> (&'static str, Updatable) {
        let elsewhere = |how: &str| Updatable::Elsewhere {
            how: how.to_owned(),
        };
        if path_has(path, "windowsapps") || path_has(path, "/program files/powershell/") {
            (
                MANAGER_UNKNOWN,
                elsewhere("winget upgrade Microsoft.PowerShell"),
            )
        } else if path_has(path, "/opt/microsoft/powershell/") || path_has(path, "/usr/bin/") {
            (MANAGER_UNKNOWN, elsewhere("the system's package manager"))
        } else if path_has(path, ".dotnet/tools") {
            (
                MANAGER_UNKNOWN,
                elsewhere("dotnet tool update --global PowerShell"),
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

    /// PowerShell's own check for a newer release and its telemetry stay off in a copy the host keeps.
    fn launch_environment(&self) -> BTreeMap<String, String> {
        BTreeMap::from([
            ("POWERSHELL_UPDATECHECK".to_owned(), "Off".to_owned()),
            ("POWERSHELL_TELEMETRY_OPTOUT".to_owned(), "1".to_owned()),
        ])
    }

    fn latest(&self, _channel: Option<&str>, tools: &Object<dyn InstallTools>) -> LatestReport {
        match listing(POWERSHELL_RELEASES_API, tools)
            .map(|doc| newest(powershell_releases(&doc)))
            .and_then(|found| {
                found.ok_or_else(|| internal_error("the repository lists no stable release"))
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
        let os = spell(&platform.os, &[("windows", "win"), ("linux", "linux")])
            .ok_or_else(|| unsupported_error("PowerShell is not published for this system here"))?;
        let arch =
            spell(&platform.arch, &[("x86_64", "x64"), ("aarch64", "arm64")]).ok_or_else(|| {
                unsupported_error("PowerShell is not published for this processor here")
            })?;
        let windows = platform.os == "windows";
        let doc = listing(POWERSHELL_RELEASES_API, tools)?;
        let all = powershell_releases(&doc);
        let release = match version {
            Some(wanted) => {
                let wanted = pinned(wanted)?;
                all.into_iter().find(|release| release.version == wanted)
            }
            None => newest(all),
        }
        .ok_or_else(|| unsupported_error("the repository lists no such release"))?;
        // The Windows zip is spelled `PowerShell-`, the Linux archive `powershell-`.
        let (name, format) = if windows {
            (
                format!("PowerShell-{}-{os}-{arch}.zip", release.version),
                "zip",
            )
        } else {
            (
                format!("powershell-{}-{os}-{arch}.tar.gz", release.version),
                "tar.gz",
            )
        };
        let digest = release.digest_of(&name).ok_or_else(|| {
            bad_value_error(format!(
                "release {} lists no {name} with a digest",
                release.tag
            ))
        })?;
        let download = Download::new(
            format!("{POWERSHELL_DOWNLOADS}{}/{name}", release.tag),
            None,
            Check::Sha256 {
                value: digest.to_owned(),
            },
        );
        // Both archives hold every file at their root.
        let part = Part::new(download, "powershell-archive").unpacked(UnpackStep::new(
            format,
            "powershell",
            0,
            true,
        ));
        let launch = if windows {
            "powershell/pwsh.exe"
        } else {
            "powershell/pwsh"
        };
        let mut made = Release::new(release.version.clone(), launch, vec![part]);
        made.notices.push(format!(
            "PowerShell {} is checked against the SHA-256 GitHub lists for the release's file, whose publisher signature is not checked by this store.",
            release.version
        ));
        Ok(made)
    }

    fn finish(&self, finish: &Finish<'_>) -> Result<(), ProviderError> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            // The archive's own modes are kept when it carries them; the program is the one that has to run.
            let path = Path::new(finish.staging).join("powershell/pwsh");
            if path.is_file() {
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                    .map_err(|error| internal_error(format!("pwsh: {error}")))?;
            }
        }
        #[cfg(not(unix))]
        let _ = finish;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
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
    fn git_for_windows_releases_are_named_the_way_their_files_are() {
        let found = git_releases(&git_listing());
        let versions: Vec<&str> = found.iter().map(|each| each.version.as_str()).collect();
        assert_eq!(versions, ["2.56.1", "2.56.0.4"]);
        assert_eq!(
            newest(found).map(|each| each.version).as_deref(),
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
        assert_eq!(
            powershell_version("PowerShell 7.7.0\n").as_deref(),
            Some("7.7.0")
        );
        assert_eq!(powershell_version("PowerShell 7.7.0-rc.1"), None);
        assert_eq!(powershell_version("7.7.0"), None);
    }

    #[test]
    fn powershell_offers_no_prerelease() {
        let listing = serde_json::json!([
            {"tag_name": "v7.7.0", "draft": false, "prerelease": false, "assets": []},
            {"tag_name": "v7.8.0-rc.1", "draft": false, "prerelease": true, "assets": []},
            {"tag_name": "v7.6.10", "draft": false, "prerelease": false, "assets": []}
        ]);
        let found = powershell_releases(&listing);
        assert_eq!(found.len(), 2);
        assert_eq!(
            newest(found).map(|each| each.version).as_deref(),
            Some("7.7.0")
        );
    }
}
