//! What the catalog sources of the program stores share: reading the values an entry's form holds, and turning a GitHub releases
//! listing into a `canoa.releases/1` catalog. A source knows where its publisher keeps releases and how its files are named; this
//! module knows the shapes the publishers have in common.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{FromValue, Schema, ToValue};
use guatiao_intake::Form;
use serde_json::{Map, Value};

use super::catalog::{
    CatalogFile, FileCheck, Release, ReleasePart, ReleasePlatform, ReleasesCatalog,
};
use super::recipe::{Published, github_assets};
use super::source::{ReadRequest, SourceFetch};

/// The most a listing or a document of a feed is read for.
pub const MOST_DOCUMENT: u64 = 8 << 20;

impl ReadRequest {
    /// What the entry's form holds, as a JSON object.
    ///
    /// # Errors
    ///
    /// `GUATIAO_ERR_BAD_VALUE` when the fields are not a JSON object.
    pub fn values(&self) -> Result<Map<String, Value>, ProviderError> {
        match serde_json::from_str(&self.fields) {
            Ok(Value::Object(map)) => Ok(map),
            _ => Err(crate::status::bad_value(format!(
                "the fields of the catalog {} are not a JSON object",
                self.label
            ))),
        }
    }
}

/// The text of `name` among `values`, or `default` when the form left it empty.
#[must_use]
pub fn text_of(values: &Map<String, Value>, name: &str, default: &str) -> String {
    values
        .get(name)
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .map_or_else(
            || default.to_owned(),
            |text| text.trim_end_matches('/').to_owned(),
        )
}

/// A JSON document, read through the host's fetch.
///
/// # Errors
///
/// What the fetch refuses, or `GUATIAO_ERR_BAD_VALUE` for text that is not JSON.
pub fn json_from(fetch: &Object<dyn SourceFetch>, url: &str) -> Result<Value, ProviderError> {
    let text = fetch.get_json(url, MOST_DOCUMENT)?;
    serde_json::from_str(&text).map_err(|_| crate::status::bad_value(format!("{url} is not JSON")))
}

/// The platforms the Store measures its programs on, as `(os, arch)`.
pub const MEASURED: [(&str, &str); 4] = [
    ("windows", "x86_64"),
    ("windows", "aarch64"),
    ("linux", "x86_64"),
    ("linux", "aarch64"),
];

/// What a source that reads a program's GitHub releases asks for.
#[derive(ToValue, FromValue, Schema, Form, Debug, Clone, Default, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[form(section(id = "source", label = "GitHub repository"))]
#[non_exhaustive]
pub struct GithubSettings {
    /// The repository, `owner/name`, whose releases are read: empty is the program's own.
    #[schema(section = "source", title = "Repository")]
    #[form(placeholder = "owner/name")]
    pub repo: Option<String>,
}

/// A platform's key in a catalog: `<os>-<arch>`.
#[must_use]
pub fn platform_key(os: &str, arch: &str) -> String {
    format!("{os}-{arch}")
}

/// The files of one GitHub repository's releases, and how a program names them.
#[derive(Debug)]
pub struct GithubFeed<'a, const N: usize> {
    /// The repository, `owner/name`.
    pub repo: &'a str,
    /// How many releases of the listing are read.
    pub listed: u32,
    /// The releases of a listing the program offers: a draft, a prerelease and any tag it does not take are left out.
    pub releases: fn(&Value) -> Vec<Published<N>>,
    /// The `(os, arch)` pairs the program is published for, each with the name of its file in a release, when it has one.
    pub platforms: &'a [(&'a str, &'a str)],
    /// The file a release carries for `os` and `arch`.
    pub file: fn(&Published<N>, &str, &str) -> Option<String>,
}

impl<const N: usize> GithubFeed<'_, N> {
    /// Where the listing is read from.
    #[must_use]
    pub fn api(&self) -> String {
        format!(
            "https://api.github.com/repos/{}/releases?per_page={}",
            self.repo, self.listed
        )
    }

    /// Where a release's files are served from.
    #[must_use]
    pub fn downloads(&self) -> String {
        format!("https://github.com/{}/releases/download/", self.repo)
    }

    /// The catalog of the repository's releases, newest first, each file checked by the SHA-256 GitHub lists for it. The newest is
    /// the `stable` channel. A release none of whose files is listed with a digest is left out.
    ///
    /// # Errors
    ///
    /// What the fetch refuses, or why the listing offers no release.
    pub fn catalog(
        &self,
        fetch: &Object<dyn SourceFetch>,
    ) -> Result<ReleasesCatalog, ProviderError> {
        let listing = json_from(fetch, &self.api())?;
        self.catalog_of(&listing)
    }

    /// The catalog of the releases in `listing`, which GitHub's releases API answered.
    ///
    /// # Errors
    ///
    /// Why the listing offers no release.
    pub fn catalog_of(&self, listing: &Value) -> Result<ReleasesCatalog, ProviderError> {
        let mut all = (self.releases)(listing);
        all.sort_by(|a, b| b.order.cmp(&a.order));
        let mut releases: Vec<Release> = Vec::new();
        let mut skipped: Option<String> = None;
        for published in &all {
            let mut platforms = BTreeMap::new();
            for (os, arch) in self.platforms {
                let Some(name) = (self.file)(published, os, arch) else {
                    continue;
                };
                let Some(digest) = published.digest_of(&name) else {
                    continue;
                };
                let file = CatalogFile::new(
                    format!("{}{}/{name}", self.downloads(), published.tag),
                    name.clone(),
                )
                .checked(FileCheck::Sha256(digest.to_owned()));
                platforms.insert(
                    platform_key(os, arch),
                    ReleasePlatform::new(None, vec![ReleasePart::new(file, name)]),
                );
            }
            if platforms.is_empty() {
                // A newer release the store cannot offer is said beside the one it offers instead.
                if releases.is_empty() && skipped.is_none() {
                    skipped = Some(published.tag.clone());
                }
                continue;
            }
            let mut release = Release::new(published.version.clone());
            if releases.is_empty() {
                release.channels = vec!["stable".to_owned()];
                if let Some(tag) = skipped.take() {
                    release.notices.push(format!(
                        "The newest release, {tag}, lists no file with a digest, so the store offers {} instead.",
                        published.version
                    ));
                }
            }
            release.platforms = platforms;
            releases.push(release);
        }
        let Some(newest) = releases.first().map(|release| release.version.clone()) else {
            return Err(crate::status::unsupported(format!(
                "{} lists no release with a file and a digest for this program",
                self.repo
            )));
        };
        let mut channels = BTreeMap::new();
        channels.insert("stable".to_owned(), newest);
        Ok(ReleasesCatalog::new(channels, releases))
    }
}

/// The releases of a GitHub listing whose tag `tag` turns into a version and its order, with each release's digested assets; a
/// draft and a prerelease are left out. For a program whose tags are `<prefix><dotted numbers>`.
#[must_use]
pub fn published_by_tag<const N: usize>(
    listing: &Value,
    version_of: impl Fn(&str) -> Option<String>,
    order_of: impl Fn(&str) -> Option<[u64; N]>,
) -> Vec<Published<N>> {
    listing
        .as_array()
        .into_iter()
        .flatten()
        .filter(|release| release["draft"] != true && release["prerelease"] != true)
        .filter_map(|release| {
            let tag = release["tag_name"].as_str()?;
            let version = version_of(tag)?;
            Some(Published {
                order: order_of(&version)?,
                version,
                tag: tag.to_string(),
                assets: github_assets(release),
            })
        })
        .collect()
}

/// Tools whose only answer is the releases document they were made with, for a test of a recipe: everything else is refused.
#[derive(Debug, Clone)]
pub struct FixedReleases(pub String);

impl FixedReleases {
    /// These tools as the object a recipe is handed.
    #[must_use]
    pub fn object(self) -> Object<dyn super::InstallTools> {
        super::InstallTools::into_object(self)
    }
}

impl super::InstallTools for FixedReleases {
    fn find(&self, _request: super::FindRequest) -> Result<Vec<super::Found>, ProviderError> {
        Err(crate::status::unsupported("these tools only list releases"))
    }

    fn releases(&self) -> Result<String, ProviderError> {
        Ok(self.0.clone())
    }

    fn fetch_text(&self, _url: &str, _max_bytes: u64) -> Result<String, ProviderError> {
        Err(crate::status::unsupported("these tools only list releases"))
    }

    fn fetch(&self, _request: super::FetchRequest) -> Result<super::Fetched, ProviderError> {
        Err(crate::status::unsupported("these tools only list releases"))
    }

    fn unpack(&self, _request: super::UnpackRequest) -> Result<super::Unpacked, ProviderError> {
        Err(crate::status::unsupported("these tools only list releases"))
    }
}
