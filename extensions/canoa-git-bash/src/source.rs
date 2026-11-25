//! Where Git for Windows' portable releases are listed: its repository on GitHub, whose release listing gives a SHA-256 for every
//! file. The tag `v2.56.1.windows.1` is the release `2.56.1`, and `v2.56.0.windows.4` is `2.56.0.4`, as the release's files spell
//! them; a draft, a prerelease and any other tag are left out.

use canoa::store::feed::{GithubFeed, GithubSettings, json_from, text_of};
use canoa::store::recipe::{Published, dotted_numbers, github_assets};
use canoa::store::source::{ReadRequest, SourceFetch};
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use serde_json::Value;

/// The id of the source, which canoa.cfg names as `provider`.
pub const SOURCE: &str = "git_bash_releases";

/// The repository whose releases the default catalog lists.
pub const REPO: &str = "git-for-windows/git";

/// The releases of Git for Windows in a listing, in no order.
fn releases(listing: &Value) -> Vec<Published<4>> {
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

/// The portable archive of a release: only Windows has one.
fn file(release: &Published<4>, os: &str, arch: &str) -> Option<String> {
    let arch = match arch {
        "x86_64" => "64-bit",
        "aarch64" => "arm64",
        _ => return None,
    };
    (os == "windows").then(|| format!("PortableGit-{}-{arch}.7z.exe", release.version))
}

fn feed(repo: &str) -> GithubFeed<'_, 4> {
    GithubFeed {
        repo,
        listed: 30,
        releases,
        platforms: &[("windows", "x86_64"), ("windows", "aarch64")],
        file,
    }
}

/// The catalog of the releases in `listing`, which GitHub's releases API answered for the default repository.
///
/// # Errors
///
/// Why the listing offers no release.
pub fn catalog_of(
    listing: &Value,
) -> Result<canoa::store::catalog::ReleasesCatalog, ProviderError> {
    feed(REPO).catalog_of(listing)
}

fn read(request: &ReadRequest, fetch: &Object<dyn SourceFetch>) -> Result<String, ProviderError> {
    let repo = text_of(&request.values()?, "repo", REPO);
    let feed = feed(&repo);
    let listing = json_from(fetch, &feed.api())?;
    Ok(feed.catalog_of(&listing)?.to_json())
}

canoa::catalog_source!(
    /// Git for Windows' portable releases on GitHub, as a catalog of the Git Bash program.
    GitBashReleases,
    "git_bash_releases",
    "Git for Windows releases on GitHub",
    "The portable releases of a GitHub repository laid out like Git for Windows', each file checked against the SHA-256 GitHub lists for it.",
    "git-bash",
    GithubSettings,
    places = ["https://api.github.com/repos/"],
    keys = [],
    read
);

#[cfg(test)]
#[path = "source_tests.rs"]
mod tests;
