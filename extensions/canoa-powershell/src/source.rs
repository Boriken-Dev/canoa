//! Where PowerShell's releases are listed: Microsoft's repository on GitHub, whose release listing gives a SHA-256 for every file.
//! A tag `v7.7.0` is the release `7.7.0`; a draft, a prerelease (`v7.7.0-rc.1`) and any other tag are left out.

use canoa::store::feed::{
    GithubFeed, GithubSettings, MEASURED, json_from, published_by_tag, text_of,
};
use canoa::store::recipe::{Published, dotted_numbers};
use canoa::store::source::{ReadRequest, SourceFetch};
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use serde_json::Value;

/// The id of the source, which canoa.cfg names as `provider`.
pub const SOURCE: &str = "powershell_releases";

/// The repository whose releases the default catalog lists.
pub const REPO: &str = "PowerShell/PowerShell";

/// The releases of PowerShell in a listing.
fn releases(listing: &Value) -> Vec<Published<3>> {
    published_by_tag(
        listing,
        |tag| tag.strip_prefix('v').map(str::to_owned),
        dotted_numbers::<3>,
    )
}

/// The file of a release for `os` and `arch`: the Windows zip is spelled `PowerShell-`, the Linux archive `powershell-`.
fn file(release: &Published<3>, os: &str, arch: &str) -> Option<String> {
    let arch = match arch {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        _ => return None,
    };
    match os {
        "windows" => Some(format!("PowerShell-{}-win-{arch}.zip", release.version)),
        "linux" => Some(format!(
            "powershell-{}-linux-{arch}.tar.gz",
            release.version
        )),
        _ => None,
    }
}

fn feed(repo: &str) -> GithubFeed<'_, 3> {
    GithubFeed {
        repo,
        listed: 30,
        releases,
        platforms: &MEASURED,
        file,
    }
}

fn read(request: &ReadRequest, fetch: &Object<dyn SourceFetch>) -> Result<String, ProviderError> {
    let repo = text_of(&request.values()?, "repo", REPO);
    let feed = feed(&repo);
    let listing = json_from(fetch, &feed.api())?;
    Ok(feed.catalog_of(&listing)?.to_json())
}

canoa::catalog_source!(
    /// PowerShell's releases on GitHub, as a catalog of the PowerShell program.
    PowerShellReleases,
    "powershell_releases",
    "PowerShell releases on GitHub",
    "The stable releases of a GitHub repository laid out like PowerShell's, each file checked against the SHA-256 GitHub lists for it.",
    "powershell",
    GithubSettings,
    places = ["https://api.github.com/repos/"],
    keys = [],
    read
);

#[cfg(test)]
#[path = "source_tests.rs"]
mod tests;
