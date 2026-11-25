//! Where Antigravity's releases are listed: its publisher's updater service, which answers for each platform with a manifest
//! (`<origin>/<os>_<arch>.json`) naming the one build it serves, its address and its SHA-512. Only the newest build is served, so
//! the catalog lists one release for each version the platforms' manifests name.

use alloc::collections::BTreeMap;

use canoa::store::catalog::{
    CatalogFile, FileCheck, Release, ReleasePart, ReleasePlatform, ReleasesCatalog,
};
use canoa::store::feed::{MEASURED, compare_versions, json_from, platform_key, text_of};
use canoa::store::recipe::{exe_suffix, hex_digest, pinned};
use canoa::store::source::{ReadRequest, SourceFetch};
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{FromValue, Schema, ToValue};
use guatiao_intake::Form;
use serde_json::Value;

/// The id of the source, which canoa.cfg names as `provider`.
pub const SOURCE: &str = "agy_releases";

/// Where the publisher's updater answers.
pub const ORIGIN: &str =
    "https://antigravity-cli-auto-updater-974169037036.us-central1.run.app/manifests";

/// What the source asks for.
#[derive(ToValue, FromValue, Schema, Form, Debug, Clone, Default, PartialEq, Eq)]
#[form(section(id = "source", label = "Publisher"))]
#[non_exhaustive]
pub struct AgyReleasesSettings {
    /// Where the manifests are served from: empty is the publisher's.
    #[schema(section = "source", title = "Address")]
    #[form(placeholder = "https://host/manifests")]
    pub origin: Option<String>,
}

/// How the publisher spells a platform in a manifest's name.
fn spelled(os: &str, arch: &str) -> Option<String> {
    let os = match os {
        "windows" | "linux" => os,
        _ => return None,
    };
    let arch = match arch {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        _ => return None,
    };
    Some(format!("{os}_{arch}"))
}

/// The catalog of the builds `manifests` name, each keyed by the platform it answered for as `<os>_<arch>`.
///
/// # Errors
///
/// `GUATIAO_ERR_UNSUPPORTED` when no manifest names a build that can be taken.
pub fn catalog_of(manifests: &BTreeMap<String, Value>) -> Result<ReleasesCatalog, ProviderError> {
    let mut releases: Vec<Release> = Vec::new();
    for (os, arch) in MEASURED {
        let Some(manifest) = spelled(os, arch).and_then(|name| manifests.get(&name)) else {
            continue;
        };
        let (Some(version), Some(url), Some(sha512)) = (
            manifest["version"].as_str(),
            manifest["url"].as_str(),
            manifest["sha512"].as_str(),
        ) else {
            continue;
        };
        let Some(sha512) = hex_digest(sha512, 128) else {
            continue;
        };
        if !url.starts_with("https://") {
            continue;
        }
        let Ok(version) = pinned(version) else {
            continue;
        };
        let windows = os == "windows";
        let place = if windows {
            format!("agy{}", exe_suffix(os))
        } else {
            "agy.tar.gz".to_owned()
        };
        let file = CatalogFile::new(url, place.clone()).checked(FileCheck::Sha512(sha512));
        let mut part = ReleasePart::new(file, place.clone());
        if windows {
            part = part.executable();
        }
        let platform = ReleasePlatform::new(windows.then_some(place), vec![part]);
        match releases.iter_mut().find(|each| each.version == version) {
            Some(have) => {
                have.platforms.insert(platform_key(os, arch), platform);
            }
            None => {
                let mut release = Release::new(version);
                release.platforms.insert(platform_key(os, arch), platform);
                releases.push(release);
            }
        }
    }
    releases.sort_by(|a, b| compare_versions(&b.version, &a.version));
    if releases.is_empty() {
        return Err(canoa::status::unsupported(
            "no manifest names a build with an address and a SHA-512",
        ));
    }
    let newest = releases[0].version.clone();
    releases[0].channels = vec!["stable".to_owned()];
    let channels = BTreeMap::from([("stable".to_owned(), newest)]);
    Ok(ReleasesCatalog::new(channels, releases))
}

fn read(request: &ReadRequest, fetch: &Object<dyn SourceFetch>) -> Result<String, ProviderError> {
    let origin = text_of(&request.values()?, "origin", ORIGIN);
    let mut manifests = BTreeMap::new();
    let mut last = None;
    for (os, arch) in MEASURED {
        let Some(name) = spelled(os, arch) else {
            continue;
        };
        match json_from(fetch, &format!("{origin}/{name}.json")) {
            Ok(manifest) => {
                manifests.insert(name, manifest);
            }
            Err(error) => last = Some(error),
        }
    }
    match catalog_of(&manifests) {
        Ok(catalog) => Ok(catalog.to_json()),
        Err(error) => Err(last.unwrap_or(error)),
    }
}

canoa::catalog_source!(
    /// Antigravity's newest build from its publisher's updater, as a catalog of the Antigravity program.
    AgyReleases,
    "agy_releases",
    "Antigravity builds from Google",
    "The newest build the publisher's updater serves for each platform, checked against the SHA-512 its manifest gives.",
    "agy",
    AgyReleasesSettings,
    places = ["https://antigravity-cli-auto-updater-974169037036.us-central1.run.app/manifests/"],
    keys = [],
    read
);

#[cfg(test)]
#[path = "source_tests.rs"]
mod tests;
