//! Where Python's releases are listed: the standalone CPython builds the Astral project publishes on GitHub, whose newest release
//! has a `SHA256SUMS` naming every build (`cpython-<version>+<tag>-<triple>-install_only.tar.gz`) with its SHA-256. The catalog
//! lists each version of it a final release has, and the newest of each minor series is that series' channel.

use std::collections::BTreeMap;

use canoa::store::catalog::{
    CatalogFile, FileCheck, Release, ReleasePart, ReleasePlatform, ReleasesCatalog,
};
use canoa::store::feed::{compare_versions, platform_key, text_of};
use canoa::store::recipe::sha256_from_sums;
use canoa::store::source::{ReadRequest, SourceFetch};
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{FromValue, Schema, ToValue};
use guatiao_intake::Form;

/// The id of the source, which canoa.cfg names as `provider`.
pub const SOURCE: &str = "python_releases";

/// Where the builds are listed and served from, without the closing slash.
pub const ORIGIN: &str = "https://github.com/astral-sh/python-build-standalone/releases";

/// The minor releases offered as channels, the first being the default.
pub const CHANNELS: [&str; 4] = ["3.13", "3.14", "3.12", "3.11"];

/// The platforms the builds are published for, with the triple the publisher names each by.
const TRIPLES: [(&str, &str, &str); 4] = [
    ("windows", "x86_64", "x86_64-pc-windows-msvc"),
    ("windows", "aarch64", "aarch64-pc-windows-msvc"),
    ("linux", "x86_64", "x86_64-unknown-linux-gnu"),
    ("linux", "aarch64", "aarch64-unknown-linux-gnu"),
];

/// What the source asks for.
#[derive(ToValue, FromValue, Schema, Form, Debug, Clone, Default, PartialEq, Eq)]
#[form(section(id = "source", label = "Releases"))]
#[non_exhaustive]
pub struct PythonSettings {
    /// The releases page the builds are listed and served from: empty is the Astral project's.
    #[schema(section = "source", title = "Address")]
    #[form(placeholder = "https://github.com/astral-sh/python-build-standalone/releases")]
    pub origin: Option<String>,
}

/// A build a line of the digest list names.
struct Build {
    version: String,
    file: String,
    tag: String,
}

fn final_version(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    parts.len() == 3 && parts.iter().all(|part| part.parse::<u64>().is_ok())
}

/// The builds `sums` names for `triple`, by version.
fn builds(sums: &str, triple: &str) -> Vec<Build> {
    let suffix = format!("-{triple}-install_only.tar.gz");
    sums.lines()
        .filter_map(|line| line.split_whitespace().last())
        .map(|name| name.trim_start_matches('*'))
        .filter_map(|file| {
            let rest = file
                .strip_prefix("cpython-")?
                .strip_suffix(suffix.as_str())?;
            let (version, tag) = rest.split_once('+')?;
            (tag.bytes().all(|byte| byte.is_ascii_digit()) && !tag.is_empty()).then(|| Build {
                version: version.to_owned(),
                file: file.to_owned(),
                tag: tag.to_owned(),
            })
        })
        .filter(|build| final_version(&build.version))
        .collect()
}

/// The catalog of the builds the digest list `sums` of the release folder `origin` names.
///
/// # Errors
///
/// `GUATIAO_ERR_UNSUPPORTED` when the list names no final build.
pub fn catalog_of(origin: &str, sums: &str) -> Result<ReleasesCatalog, ProviderError> {
    let mut releases: BTreeMap<String, Release> = BTreeMap::new();
    for (os, arch, triple) in TRIPLES {
        for build in builds(sums, triple) {
            let Some(digest) = sha256_from_sums(sums, &build.file) else {
                continue;
            };
            let url = format!(
                "{origin}/download/{}/{}",
                build.tag,
                build.file.replace('+', "%2B")
            );
            let file = CatalogFile::new(url, build.file.clone()).checked(FileCheck::Sha256(digest));
            releases
                .entry(build.version.clone())
                .or_insert_with(|| Release::new(build.version.clone()))
                .platforms
                .insert(
                    platform_key(os, arch),
                    ReleasePlatform::new(None, vec![ReleasePart::new(file, build.file)]),
                );
        }
    }
    let mut releases: Vec<Release> = releases.into_values().collect();
    releases.sort_by(|a, b| compare_versions(&b.version, &a.version));
    if releases.is_empty() {
        return Err(canoa::status::unsupported(
            "the digest list names no final build of Python",
        ));
    }
    let mut channels = BTreeMap::new();
    for channel in CHANNELS {
        let newest = releases
            .iter_mut()
            .find(|release| release.version.starts_with(&format!("{channel}.")));
        if let Some(release) = newest {
            release.channels.push(channel.to_owned());
            channels.insert(channel.to_owned(), release.version.clone());
        }
    }
    Ok(ReleasesCatalog::new(channels, releases))
}

fn read(request: &ReadRequest, fetch: &Object<dyn SourceFetch>) -> Result<String, ProviderError> {
    let origin = text_of(&request.values()?, "origin", ORIGIN);
    let sums = fetch.get_text(&format!("{origin}/latest/download/SHA256SUMS"), 4 << 20)?;
    Ok(catalog_of(&origin, &sums)?.to_json())
}

canoa::catalog_source!(
    /// Python's standalone builds on GitHub, as a catalog of the Python program.
    PythonReleases,
    "python_releases",
    "Python builds from the Astral project",
    "The standalone CPython builds of the newest release, the newest of each minor series a channel, each archive checked against the SHA-256 in the release's SHA256SUMS.",
    "python",
    PythonSettings,
    places = [
        "https://github.com/astral-sh/python-build-standalone/releases/",
        "https://release-assets.githubusercontent.com",
        "https://objects.githubusercontent.com"
    ],
    keys = [],
    read
);

#[cfg(test)]
#[path = "source_tests.rs"]
mod tests;
