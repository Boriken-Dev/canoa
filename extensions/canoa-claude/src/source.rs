//! Where Claude Code's releases are listed: its publisher's release storage, which names the version of each channel in a one-line
//! file (`<origin>/latest`, `<origin>/stable`) and keeps for each version a `manifest.json` with the SHA-256 and size of the
//! program of every platform, beside a detached signature (`manifest.json.sig`) by the publisher's release key.
//!
//! The key is shipped with this crate ([`KEY`]). A manifest whose signature does not verify with it, or is by another key than the
//! one the entry demands, makes the whole catalog refuse: nothing is taken as a digest-only fallback.

use std::collections::BTreeMap;

use canoa::store::catalog::{
    CatalogFile, FileCheck, Release, ReleasePart, ReleasePlatform, ReleasesCatalog,
};
use canoa::store::feed::{platform_key, text_of};
use canoa::store::openpgp::{self, Signer};
use canoa::store::recipe::{hex_digest, pinned};
use canoa::store::source::{ReadRequest, SourceFetch};
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{FromValue, Schema, ToValue};
use guatiao_intake::Form;
use serde_json::Value;

/// The id of the source, which canoa.cfg names as `provider`.
pub const SOURCE: &str = "claude_releases";

/// Where the publisher keeps its releases, without the closing slash.
pub const ORIGIN: &str = "https://downloads.claude.ai/claude-code-releases";

/// The channels, the default first.
pub const CHANNELS: [&str; 2] = ["latest", "stable"];

/// The publisher's release key, as `<origin>/../keys/claude-code.asc` serves it.
pub const KEY: &str = include_str!("../keys/claude-code.asc");

/// The fingerprint of [`KEY`].
pub const FINGERPRINT: &str = "31DDDE24DDFAB679F42D7BD2BAA929FF1A7ECACE";

/// How the publisher spells each platform, with the platform's own key.
const PLATFORMS: [(&str, &str, &str); 6] = [
    ("win32-x64", "windows", "x86_64"),
    ("win32-arm64", "windows", "aarch64"),
    ("linux-x64", "linux", "x86_64"),
    ("linux-arm64", "linux", "aarch64"),
    ("darwin-x64", "macos", "x86_64"),
    ("darwin-arm64", "macos", "aarch64"),
];

/// What the source asks for.
#[derive(ToValue, FromValue, Schema, Form, Debug, Clone, Default, PartialEq, Eq)]
#[form(section(id = "source", label = "Publisher"))]
#[non_exhaustive]
pub struct ClaudeSettings {
    /// Where the releases are kept: empty is the publisher's. A mirror that relays the publisher's files keeps its signature.
    #[schema(section = "source", title = "Address")]
    #[form(placeholder = "https://downloads.claude.ai/claude-code-releases")]
    pub origin: Option<String>,
}

/// A version's manifest, the signature over it, and who signed it.
#[derive(Debug, Clone)]
pub struct Signed {
    /// The manifest as the publisher wrote it.
    pub manifest: String,
    /// Who made the signature.
    pub signer: Signer,
}

/// Checks `signature` over `manifest` against the publisher's key, and against `demanded` fingerprints when the entry names any.
///
/// # Errors
///
/// `GUATIAO_ERR_BAD_VALUE` with why the signature is refused.
pub fn verified(
    manifest: &str,
    signature: &str,
    demanded: &[String],
) -> Result<Signed, ProviderError> {
    let signer =
        openpgp::verify_detached(&[KEY], manifest.as_bytes(), signature).map_err(|why| {
            canoa::status::bad_value(format!("the manifest's signature is refused: {why}"))
        })?;
    if !demanded.is_empty()
        && !demanded
            .iter()
            .any(|each| each.eq_ignore_ascii_case(&signer.fingerprint))
    {
        return Err(canoa::status::bad_value(format!(
            "the manifest is signed by {}, which is none of the keys the entry demands",
            signer.fingerprint
        )));
    }
    Ok(Signed {
        manifest: manifest.to_owned(),
        signer,
    })
}

/// The catalog of the releases `channels` name (`(channel, version)`), whose signed manifests are `manifests` (by version).
///
/// # Errors
///
/// A manifest for another version, or one that lists no program with a SHA-256.
pub fn catalog_of(
    origin: &str,
    channels: &[(&str, String)],
    manifests: &BTreeMap<String, Signed>,
) -> Result<ReleasesCatalog, ProviderError> {
    let mut named = BTreeMap::new();
    let mut releases: Vec<Release> = Vec::new();
    let mut signer = None;
    for (channel, version) in channels {
        named.insert((*channel).to_owned(), version.clone());
        if let Some(have) = releases.iter_mut().find(|each| &each.version == version) {
            have.channels.push((*channel).to_owned());
            continue;
        }
        let signed = manifests.get(version).ok_or_else(|| {
            canoa::status::internal(format!("no manifest was read for {version}"))
        })?;
        let manifest: Value = serde_json::from_str(&signed.manifest).map_err(|_| {
            canoa::status::internal(format!("the manifest of {version} is not JSON"))
        })?;
        if manifest["version"].as_str() != Some(version.as_str()) {
            return Err(canoa::status::internal(
                "the manifest is for another version",
            ));
        }
        let mut release = Release::new(version.clone());
        release.channels = vec![(*channel).to_owned()];
        for (spelled, os, arch) in PLATFORMS {
            let Some(entry) = manifest["platforms"].get(spelled) else {
                continue;
            };
            let binary = entry["binary"].as_str().unwrap_or(if os == "windows" {
                "claude.exe"
            } else {
                "claude"
            });
            if binary.contains(['/', '\\']) || binary.contains("..") {
                return Err(canoa::status::internal(
                    "the manifest names a binary with a path",
                ));
            }
            let checksum = entry["checksum"]
                .as_str()
                .and_then(|checksum| hex_digest(checksum, 64))
                .ok_or_else(|| {
                    canoa::status::internal("the manifest gives no SHA-256 for a build")
                })?;
            let mut file = CatalogFile::new(
                format!("{origin}/{version}/{spelled}/{binary}"),
                binary.to_owned(),
            )
            .checked(FileCheck::Sha256(checksum));
            if let Some(size) = entry["size"].as_u64() {
                file = file.sized(size);
            }
            release.platforms.insert(
                platform_key(os, arch),
                ReleasePlatform::new(
                    Some(binary.to_owned()),
                    vec![ReleasePart::new(file, binary.to_owned()).executable()],
                ),
            );
        }
        if release.platforms.is_empty() {
            return Err(canoa::status::unsupported(format!(
                "the manifest of {version} lists no program for a platform this is installed on"
            )));
        }
        signer = Some(signed.signer.clone());
        releases.push(release);
    }
    let mut catalog = ReleasesCatalog::new(named, releases);
    catalog.signed = signer.map(|signer| (signer.fingerprint, "manifest.json".to_owned()));
    Ok(catalog)
}

fn read(request: &ReadRequest, fetch: &Object<dyn SourceFetch>) -> Result<String, ProviderError> {
    let origin = text_of(&request.values()?, "origin", ORIGIN);
    let mut channels = Vec::new();
    let mut manifests = BTreeMap::new();
    for channel in CHANNELS {
        let version = pinned(fetch.get_text(&format!("{origin}/{channel}"), 256)?.trim())?;
        if !manifests.contains_key(&version) {
            let manifest = fetch.get_text(&format!("{origin}/{version}/manifest.json"), 1 << 20)?;
            let signature =
                fetch.get_text(&format!("{origin}/{version}/manifest.json.sig"), 16 << 10)?;
            manifests.insert(
                version.clone(),
                verified(&manifest, &signature, &request.keys)?,
            );
        }
        channels.push((channel, version));
    }
    Ok(catalog_of(&origin, &channels, &manifests)?.to_json())
}

canoa::catalog_source!(
    /// Claude Code's releases from its publisher's storage, as a catalog of the Claude Code program.
    ClaudeReleases,
    "claude_releases",
    "Claude Code releases from Anthropic",
    "The version of each channel and the program for each platform, checked against the SHA-256 in the release's manifest, whose signature by the publisher's release key is verified first.",
    "claude",
    ClaudeSettings,
    places = ["https://downloads.claude.ai/claude-code-releases/"],
    keys = ["31DDDE24DDFAB679F42D7BD2BAA929FF1A7ECACE"],
    read
);

#[cfg(test)]
#[path = "source_tests.rs"]
mod tests;
