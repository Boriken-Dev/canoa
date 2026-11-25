//! Where Node.js's releases are listed: nodejs.org's distribution folder, whose `index.json` lists every release (the newest first,
//! and the long-term-support ones marked) and whose release folders each hold `SHASUMS256.txt` with the SHA-256 of every archive.
//! The catalog lists the release each channel (`lts`, `current`) is at, each archive checked against the checksums.
//!
//! The checksums are read from `SHASUMS256.txt.asc`, the clearsigned copy, and only when its signature verifies with one of the
//! release team's keys ([`KEYS`]) and, when the entry names fingerprints, one of those: the digests are taken from the text the
//! signature covers, so a mirror or a proxy cannot change one.

use std::collections::BTreeMap;

use canoa::store::catalog::{
    CatalogFile, FileCheck, Release, ReleasePart, ReleasePlatform, ReleasesCatalog,
};
use canoa::store::feed::{json_from, platform_key, text_of};
use canoa::store::openpgp;
use canoa::store::recipe::{pinned, sha256_from_sums};
use canoa::store::source::{ReadRequest, SourceFetch};
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{FromValue, Schema, ToValue};
use guatiao_intake::Form;
use serde_json::Value;

/// The id of the source, which canoa.cfg names as `provider`.
pub const SOURCE: &str = "node_releases";

/// Where Node's builds and checksums are served from, without the closing slash.
pub const ORIGIN: &str = "https://nodejs.org/dist";

/// The channels, the default first.
pub const CHANNELS: [&str; 2] = ["lts", "current"];

/// The public keys of the release team that signs `SHASUMS256.txt`, as the project's `release-keys` repository keeps them: the
/// active releasers'.
pub const KEYS: &str = include_str!("../keys/release-keys.asc");

/// The platforms Node publishes archives for.
const PLATFORMS: [(&str, &str); 6] = [
    ("windows", "x86_64"),
    ("windows", "aarch64"),
    ("linux", "x86_64"),
    ("linux", "aarch64"),
    ("macos", "x86_64"),
    ("macos", "aarch64"),
];

/// What the source asks for.
#[derive(ToValue, FromValue, Schema, Form, Debug, Clone, Default, PartialEq, Eq)]
#[form(section(id = "source", label = "Distribution"))]
#[non_exhaustive]
pub struct NodeSettings {
    /// Where the builds and their checksums are served from: empty is nodejs.org's.
    #[schema(section = "source", title = "Address")]
    #[form(placeholder = "https://nodejs.org/dist")]
    pub origin: Option<String>,
}

/// The archive of a release for `os` and `arch`, and how it is packed.
fn archive(version: &str, os: &str, arch: &str) -> Option<String> {
    let (os, format) = match os {
        "windows" => ("win", "zip"),
        "linux" => ("linux", "tar.gz"),
        "macos" => ("darwin", "tar.gz"),
        _ => return None,
    };
    let arch = match arch {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        _ => return None,
    };
    Some(format!("node-v{version}-{os}-{arch}.{format}"))
}

/// The version each channel is at, from `index.json`: `current` is its first entry, `lts` the first marked as long-term support.
///
/// # Errors
///
/// `GUATIAO_ERR_UNSUPPORTED` when the index lists none for a channel.
pub fn heads(index: &Value) -> Result<Vec<(&'static str, String)>, ProviderError> {
    let entries = index
        .as_array()
        .ok_or_else(|| canoa::status::bad_value("index.json is not a list"))?;
    let mut found = Vec::new();
    for channel in CHANNELS {
        let entry = entries
            .iter()
            .find(|entry| channel == "current" || entry["lts"].is_string())
            .and_then(|entry| entry["version"].as_str())
            .ok_or_else(|| {
                canoa::status::unsupported(format!("the index lists no release on {channel}"))
            })?;
        found.push((channel, pinned(entry.trim_start_matches('v'))?));
    }
    Ok(found)
}

/// The catalog of the releases `heads` names, whose checksums are `sums` (by version).
///
/// # Errors
///
/// `GUATIAO_ERR_UNSUPPORTED` when no checksum names an archive of a release.
pub fn catalog_of(
    origin: &str,
    heads: &[(&str, String)],
    sums: &BTreeMap<String, String>,
) -> Result<ReleasesCatalog, ProviderError> {
    let mut channels = BTreeMap::new();
    let mut releases: Vec<Release> = Vec::new();
    for (channel, version) in heads {
        channels.insert((*channel).to_owned(), version.clone());
        if let Some(have) = releases.iter_mut().find(|each| &each.version == version) {
            have.channels.push((*channel).to_owned());
            continue;
        }
        let text = sums.get(version).map_or("", String::as_str);
        let mut release = Release::new(version.clone());
        release.channels = vec![(*channel).to_owned()];
        for (os, arch) in PLATFORMS {
            let Some(name) = archive(version, os, arch) else {
                continue;
            };
            let Some(digest) = sha256_from_sums(text, &name) else {
                continue;
            };
            let file = CatalogFile::new(format!("{origin}/v{version}/{name}"), name.clone())
                .checked(FileCheck::Sha256(digest));
            release.platforms.insert(
                platform_key(os, arch),
                ReleasePlatform::new(None, vec![ReleasePart::new(file, name)]),
            );
        }
        if release.platforms.is_empty() {
            return Err(canoa::status::unsupported(format!(
                "the checksums of Node {version} name none of the archives"
            )));
        }
        releases.push(release);
    }
    Ok(ReleasesCatalog::new(channels, releases))
}

/// The checksums a clearsigned `SHASUMS256.txt.asc` holds, once its signature verifies with the release team's keys, and with one of
/// `demanded` when the entry names any.
///
/// # Errors
///
/// `GUATIAO_ERR_BAD_VALUE` with why the signature is refused.
pub fn signed(
    message: &str,
    demanded: &[String],
) -> Result<(openpgp::Signer, String), ProviderError> {
    let (who, text) = openpgp::verify_cleartext(&[KEYS], message).map_err(|why| {
        canoa::status::bad_value(format!("the checksums' signature is refused: {why}"))
    })?;
    if !demanded.is_empty()
        && !demanded
            .iter()
            .any(|each| each.eq_ignore_ascii_case(&who.fingerprint))
    {
        return Err(canoa::status::bad_value(format!(
            "the checksums are signed by {}, which is none of the keys the entry demands",
            who.fingerprint
        )));
    }
    Ok((who, text))
}

/// The catalog of the releases `origin` serves, read through `fetch`: what a source of Node's own and one that builds on a Node
/// release (Pi's) both read.
///
/// # Errors
///
/// What the fetch refuses, or why the index or the checksums name no release.
pub fn read_catalog(
    origin: &str,
    fetch: &Object<dyn SourceFetch>,
    demanded: &[String],
) -> Result<ReleasesCatalog, ProviderError> {
    let index = json_from(fetch, &format!("{origin}/index.json"))?;
    let heads = heads(&index)?;
    let mut sums = BTreeMap::new();
    let mut signer = None;
    for (_, version) in &heads {
        if !sums.contains_key(version) {
            let message = fetch.get_text(
                &format!("{origin}/v{version}/SHASUMS256.txt.asc"),
                64 * 1024,
            )?;
            let (who, text) = signed(&message, demanded)?;
            signer.get_or_insert(who);
            sums.insert(version.clone(), text);
        }
    }
    let mut catalog = catalog_of(origin, &heads, &sums)?;
    catalog.signed = signer.map(|who| (who.fingerprint, "SHASUMS256.txt".to_owned()));
    Ok(catalog)
}

fn read(request: &ReadRequest, fetch: &Object<dyn SourceFetch>) -> Result<String, ProviderError> {
    let origin = text_of(&request.values()?, "origin", ORIGIN);
    Ok(read_catalog(&origin, fetch, &request.keys)?.to_json())
}

canoa::catalog_source!(
    /// Node.js's releases from nodejs.org, as a catalog of the Node.js program.
    NodeReleases,
    "node_releases",
    "Node.js releases from nodejs.org",
    "The release each channel (lts, current) is at, each archive checked against the SHA-256 in the release's SHASUMS256.txt, whose signature by one of the release team's keys is verified first.",
    "node",
    NodeSettings,
    places = ["https://nodejs.org/dist/"],
    keys = [
        "5BE8A3F6C8A5C01D106C0AD820B1A390B168D356",
        "DD792F5973C6DE52C432CBDAC77ABFA00DDBF2B7",
        "CC68F5A3106FF448322E48ED27F5E38D5B0A215F",
        "890C08DB8579162FEE0DF9DB8BEAB4DFCF555EF4",
        "C82FA3AE1CBEDC6BE46B9360C43CEC45C17AB93C",
        "108F52B48DB57BB0CC439B2997B01419BD92F80A",
        "655F3B5C1FB3FA8D1A0CA6BDE4A7D232B936D2FD",
        "A363A499291CBBC940DD62E41F10027AF002F8B0"
    ],
    read
);

#[cfg(test)]
#[path = "source_tests.rs"]
mod tests;
