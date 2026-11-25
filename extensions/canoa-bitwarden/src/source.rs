//! Where Bitwarden's command line releases are listed: the repository that publishes it among the other Bitwarden clients, whose
//! release listing gives a SHA-256 for every file. A tag `cli-v2026.10.1` is the release `2026.10.1`; a draft, a prerelease and any
//! other client's tag are left out.

use canoa::store::feed::{GithubFeed, GithubSettings, json_from, published_by_tag, text_of};
use canoa::store::recipe::{Published, dotted_numbers};
use canoa::store::source::{ReadRequest, SourceFetch};
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use serde_json::Value;

/// The id of the source, which canoa.cfg names as `provider`.
pub const SOURCE: &str = "bitwarden_releases";

/// The repository whose releases the default catalog lists.
pub const REPO: &str = "bitwarden/clients";

/// The command line's releases in a listing.
fn releases(listing: &Value) -> Vec<Published<3>> {
    published_by_tag(
        listing,
        |tag| tag.strip_prefix("cli-v").map(str::to_owned),
        dotted_numbers::<3>,
    )
}

/// How the publisher spells a platform in the file's name, and the file a release has for it: `bw-<platform>-<version>.zip`, or, from
/// 2026.11.0, where the publisher names its two builds, the open-source one `bw-oss-<platform>-<version>.zip`. Windows has one
/// build, which an ARM64 Windows runs through emulation.
fn file(release: &Published<3>, os: &str, arch: &str) -> Option<String> {
    let spelled = match (os, arch) {
        ("windows", "x86_64" | "aarch64") => "windows",
        ("linux", "x86_64") => "linux",
        ("linux", "aarch64") => "linux-arm64",
        ("macos", "x86_64") => "macos",
        ("macos", "aarch64") => "macos-arm64",
        _ => return None,
    };
    let plain = format!("bw-{spelled}-{}.zip", release.version);
    if release.digest_of(&plain).is_some() {
        return Some(plain);
    }
    Some(format!("bw-oss-{spelled}-{}.zip", release.version))
}

/// The platforms the command line is published for.
const PLATFORMS: [(&str, &str); 6] = [
    ("windows", "x86_64"),
    ("windows", "aarch64"),
    ("linux", "x86_64"),
    ("linux", "aarch64"),
    ("macos", "x86_64"),
    ("macos", "aarch64"),
];

fn feed(repo: &str) -> GithubFeed<'_, 3> {
    // The repository publishes the web, browser and desktop clients too, and the command line's newest is among the latest hundred.
    GithubFeed {
        repo,
        listed: 100,
        releases,
        platforms: &PLATFORMS,
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
    /// Bitwarden's command line releases on GitHub, as a catalog of its program.
    BitwardenReleases,
    "bitwarden_releases",
    "Bitwarden command line releases on GitHub",
    "The command line's releases of a GitHub repository laid out like Bitwarden's clients', each file checked against the SHA-256 GitHub lists for it.",
    "bw",
    GithubSettings,
    places = ["https://api.github.com/repos/"],
    keys = [],
    read
);

#[cfg(test)]
#[path = "source_tests.rs"]
mod tests;
