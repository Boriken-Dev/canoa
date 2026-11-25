//! Where Pi's releases are listed: the npm registry, whose answer for the package (`<registry>/<package>/latest`) names the newest
//! version with the integrity npm checks it by, and Node.js's own builds, which Pi runs on. Pi is installed by Node's npm, so the
//! catalog's files are Node's archives, which the host downloads and checks, and the release is Pi's version: npm fetches the
//! package itself, and checks each file against the registry's integrity, which the host does not see.

use alloc::collections::BTreeMap;

use canoa::store::catalog::{Release, ReleasePlatform, ReleasesCatalog};
use canoa::store::feed::{MEASURED, json_from, platform_key, text_of};
use canoa::store::recipe::pinned;
use canoa::store::source::{ReadRequest, SourceFetch};
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{FromValue, Schema, ToValue};
use guatiao_intake::Form;
use serde_json::Value;

/// The id of the source, which canoa.cfg names as `provider`.
pub const SOURCE: &str = "pi_releases";

/// The package Pi is published as.
pub const PACKAGE: &str = "@earendil-works/pi-coding-agent";

/// The registry the package is read from and installed from.
pub const REGISTRY: &str = "https://registry.npmjs.org";

/// What the source asks for.
#[derive(ToValue, FromValue, Schema, Form, Debug, Clone, Default, PartialEq, Eq)]
#[form(section(id = "source", label = "Node.js"))]
#[non_exhaustive]
pub struct PiSettings {
    /// Where the Node.js builds Pi runs on are served from: empty is nodejs.org's.
    #[schema(section = "source", title = "Node.js address")]
    #[form(placeholder = "https://nodejs.org/dist")]
    pub node: Option<String>,
}

/// The catalog of the version `package` (the registry's answer for the package's `latest`) names, over `node`, the catalog of the
/// Node.js releases it runs on: the files of Node's long-term-support release for each platform.
///
/// # Errors
///
/// `GUATIAO_ERR_UNSUPPORTED` when the registry names no version or Node lists no long-term-support build.
pub fn catalog_of(
    package: &Value,
    node: &ReleasesCatalog,
) -> Result<ReleasesCatalog, ProviderError> {
    let version = pinned(
        package["version"]
            .as_str()
            .ok_or_else(|| canoa::status::unsupported("the registry names no version"))?,
    )?;
    let lts = node
        .channels
        .get("lts")
        .and_then(|lts| node.releases.iter().find(|release| &release.version == lts))
        .ok_or_else(|| canoa::status::unsupported("Node lists no long-term-support release"))?;
    let mut release = Release::new(version.clone());
    release.channels = vec!["latest".to_owned()];
    if let Some(integrity) = package["dist"]["integrity"].as_str() {
        release.notices.push(format!(
            "npm installs {PACKAGE}@{version} from {REGISTRY} itself, checked against the integrity the registry lists ({integrity}); the host does not see those bytes. Node {} is checked against nodejs.org's SHASUMS256.txt, whose signature by the release team was verified.",
            lts.version
        ));
    }
    for (os, arch) in MEASURED {
        if let Some(on) = lts.platforms.get(&platform_key(os, arch)) {
            release.platforms.insert(
                platform_key(os, arch),
                ReleasePlatform::new(None, on.parts.clone()),
            );
        }
    }
    if release.platforms.is_empty() {
        return Err(canoa::status::unsupported(
            "Node lists no build for a platform Pi is installed on",
        ));
    }
    let channels = BTreeMap::from([("latest".to_owned(), version)]);
    Ok(ReleasesCatalog::new(channels, vec![release]))
}

fn read(request: &ReadRequest, fetch: &Object<dyn SourceFetch>) -> Result<String, ProviderError> {
    let node = text_of(
        &request.values()?,
        "node",
        canoa_node::NODE_ORIGIN.trim_end_matches('/'),
    );
    let package = json_from(fetch, &format!("{REGISTRY}/{PACKAGE}/latest"))?;
    let node = canoa_node::read_catalog(&node, fetch, &[])?;
    Ok(catalog_of(&package, &node)?.to_json())
}

canoa::catalog_source!(
    /// Pi's newest version from the npm registry, with the Node.js build it runs on, as a catalog of the Pi program.
    PiReleases,
    "pi_releases",
    "Pi releases from npm",
    "The newest version of the package in the npm registry, and the Node.js long-term-support build that installs and runs it.",
    "pi",
    PiSettings,
    places = ["https://registry.npmjs.org/", "https://nodejs.org/dist/"],
    keys = [],
    read
);

#[cfg(test)]
#[path = "source_tests.rs"]
mod tests;
