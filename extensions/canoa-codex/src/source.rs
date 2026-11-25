//! Where Codex's releases are listed: its publisher's release host, whose channel document (`<origin>/codex/channels/latest`)
//! names the newest release and gives a SHA-256 for each of its files, and whose release folder holds `codex-package_SHA256SUMS`.
//! A file is listed with the digest the checksums give, and only when the channel document says the same.

use alloc::collections::BTreeMap;

use canoa::store::catalog::{
    CatalogFile, FileCheck, Release, ReleasePart, ReleasePlatform, ReleasesCatalog,
};
use canoa::store::feed::{MEASURED, json_from, platform_key, text_of};
use canoa::store::recipe::{pinned, sha256_from_sums};
use canoa::store::source::{ReadRequest, SourceFetch};
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{FromValue, Schema, ToValue};
use guatiao_intake::Form;
use serde_json::Value;

/// The id of the source, which canoa.cfg names as `provider`.
pub const SOURCE: &str = "codex_releases";

/// Where the publisher serves its releases.
pub const ORIGIN: &str = "https://releases.openai.com";

/// What the source asks for.
#[derive(ToValue, FromValue, Schema, Form, Debug, Clone, Default, PartialEq, Eq)]
#[form(section(id = "source", label = "Publisher"))]
#[non_exhaustive]
pub struct CodexSettings {
    /// Where the releases are served from: empty is the publisher's.
    #[schema(section = "source", title = "Address")]
    #[form(placeholder = "https://releases.openai.com")]
    pub origin: Option<String>,
}

/// How the publisher spells a platform in a file's name, as `(os, arch)`.
fn triple(os: &str, arch: &str) -> Option<String> {
    let os = match os {
        "windows" => "pc-windows-msvc",
        "linux" => "unknown-linux-musl",
        "macos" => "apple-darwin",
        _ => return None,
    };
    let arch = match arch {
        "x86_64" | "aarch64" => arch,
        _ => return None,
    };
    Some(format!("codex-package-{arch}-{os}.tar.gz"))
}

/// The catalog of the release `document` (the channel document) names, whose files `sums` (its `codex-package_SHA256SUMS`) checks.
///
/// # Errors
///
/// A document that names no release, or whose digest for a file is not the one the checksums give.
pub fn catalog_of(
    origin: &str,
    document: &Value,
    sums: &str,
) -> Result<ReleasesCatalog, ProviderError> {
    let tag = document["tag_name"]
        .as_str()
        .ok_or_else(|| canoa::status::internal("the channel document names no release"))?;
    let version = pinned(tag.strip_prefix("rust-v").unwrap_or(tag))?;
    let base = format!("{origin}/codex/releases/{version}");
    let mut release = Release::new(version.clone());
    release.channels = vec!["latest".to_owned()];
    for (os, arch) in MEASURED {
        let Some(asset) = triple(os, arch) else {
            continue;
        };
        let Some(digest) = sha256_from_sums(sums, &asset) else {
            continue;
        };
        let published = document["assets"]
            .as_array()
            .and_then(|assets| {
                assets
                    .iter()
                    .find(|each| each["name"].as_str() == Some(asset.as_str()))
            })
            .and_then(|each| each["digest"].as_str())
            .and_then(|digest| digest.strip_prefix("sha256:"));
        if published.is_some_and(|published| !published.eq_ignore_ascii_case(&digest)) {
            return Err(canoa::status::internal(
                "the channel's digest and the release's checksums disagree",
            ));
        }
        let file = CatalogFile::new(format!("{base}/{asset}"), asset.clone())
            .checked(FileCheck::Sha256(digest));
        release.platforms.insert(
            platform_key(os, arch),
            ReleasePlatform::new(None, vec![ReleasePart::new(file, asset)]),
        );
    }
    if release.platforms.is_empty() {
        return Err(canoa::status::unsupported(format!(
            "release {version} lists no codex-package the checksums vouch for"
        )));
    }
    let channels = BTreeMap::from([("latest".to_owned(), version)]);
    Ok(ReleasesCatalog::new(channels, vec![release]))
}

fn read(request: &ReadRequest, fetch: &Object<dyn SourceFetch>) -> Result<String, ProviderError> {
    let origin = text_of(&request.values()?, "origin", ORIGIN);
    let document = json_from(fetch, &format!("{origin}/codex/channels/latest"))?;
    let tag = document["tag_name"]
        .as_str()
        .ok_or_else(|| canoa::status::internal("the channel document names no release"))?;
    let version = pinned(tag.strip_prefix("rust-v").unwrap_or(tag))?;
    let sums = fetch.get_text(
        &format!("{origin}/codex/releases/{version}/codex-package_SHA256SUMS"),
        64 * 1024,
    )?;
    Ok(catalog_of(&origin, &document, &sums)?.to_json())
}

canoa::catalog_source!(
    /// Codex's newest release from its publisher's release host, as a catalog of the Codex program.
    CodexReleases,
    "codex_releases",
    "Codex releases from OpenAI",
    "The newest release of the publisher's channel, each file checked against the SHA-256 its checksums give and its channel document repeats.",
    "codex",
    CodexSettings,
    places = ["https://releases.openai.com/"],
    keys = [],
    read
);

#[cfg(test)]
#[path = "source_tests.rs"]
mod tests;
