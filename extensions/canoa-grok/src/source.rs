//! Where Grok's releases are listed: its publisher's own storage, which names the version each channel is at in a one-line file
//! (`<origin>/stable`, `alpha`, `enterprise`) and serves one program for each platform at `<origin>/grok-<version>-<os>-<arch>`.
//! The publisher gives no checksum for any of it, so every file is listed as one nothing checks, and is offered as not verified.

use alloc::collections::BTreeMap;

use canoa::store::catalog::{
    CatalogFile, FileCheck, Release, ReleasePart, ReleasePlatform, ReleasesCatalog,
};
use canoa::store::feed::{MEASURED, platform_key, text_of};
use canoa::store::recipe::{exe_suffix, pinned};
use canoa::store::source::{ReadRequest, SourceFetch};
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{FromValue, Schema, ToValue};
use guatiao_intake::Form;

/// The id of the source, which canoa.cfg names as `provider`.
pub const SOURCE: &str = "grok_releases";

/// Where the publisher keeps the channels' files and the programs.
pub const ORIGIN: &str = "https://x.ai/cli";

/// The channels, the default first.
pub const CHANNELS: [&str; 3] = ["stable", "alpha", "enterprise"];

/// Why no checksum is given.
const WHY: &str = "xAI publishes no checksum for Grok";

/// What the source asks for.
#[derive(ToValue, FromValue, Schema, Form, Debug, Clone, Default, PartialEq, Eq)]
#[form(section(id = "source", label = "Publisher"))]
#[non_exhaustive]
pub struct GrokSettings {
    /// Where the channels' files and the programs are served from: empty is the publisher's.
    #[schema(section = "source", title = "Address")]
    #[form(placeholder = "https://x.ai/cli")]
    pub origin: Option<String>,
}

/// The catalog of the releases the channels name at `origin`, where `versions` are what each channel's file answered.
///
/// # Errors
///
/// `GUATIAO_ERR_BAD_VALUE` for a channel that answers with something that is no version.
pub fn catalog_of(
    origin: &str,
    versions: &[(&str, String)],
) -> Result<ReleasesCatalog, ProviderError> {
    let mut channels = BTreeMap::new();
    let mut releases: Vec<Release> = Vec::new();
    for (channel, answer) in versions {
        let version = pinned(answer.trim())?;
        channels.insert((*channel).to_owned(), version.clone());
        if let Some(have) = releases.iter_mut().find(|each| each.version == version) {
            have.channels.push((*channel).to_owned());
            continue;
        }
        let mut release = Release::new(version.clone());
        release.channels = vec![(*channel).to_owned()];
        for (os, arch) in MEASURED {
            let suffix = exe_suffix(os);
            let file = CatalogFile::new(
                format!("{origin}/grok-{version}-{os}-{arch}{suffix}"),
                format!("grok{suffix}"),
            )
            .checked(FileCheck::Unverified(WHY.to_owned()));
            release.platforms.insert(
                platform_key(os, arch),
                ReleasePlatform::new(
                    Some(format!("grok{suffix}")),
                    vec![ReleasePart::new(file, format!("grok{suffix}")).executable()],
                ),
            );
        }
        releases.push(release);
    }
    Ok(ReleasesCatalog::new(channels, releases))
}

fn read(request: &ReadRequest, fetch: &Object<dyn SourceFetch>) -> Result<String, ProviderError> {
    let origin = text_of(&request.values()?, "origin", ORIGIN);
    let mut versions = Vec::new();
    for channel in CHANNELS {
        versions.push((
            channel,
            fetch.get_text(&format!("{origin}/{channel}"), 256)?,
        ));
    }
    Ok(catalog_of(&origin, &versions)?.to_json())
}

canoa::catalog_source!(
    /// Grok's releases from its publisher's storage, as a catalog of the Grok program.
    GrokReleases,
    "grok_releases",
    "Grok releases from xAI",
    "The version each channel is at and the program for each platform, from the publisher's storage. It gives no checksum, so what it lists is offered as not verified.",
    "grok",
    GrokSettings,
    places = ["https://x.ai/cli/"],
    keys = [],
    read
);

#[cfg(test)]
#[path = "source_tests.rs"]
mod tests;
