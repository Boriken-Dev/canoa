//! The two formats a catalog source answers with for a Store type that lists models or programs: `canoa.models/1` and
//! `canoa.releases/1`. Both are JSON, versioned by their `format` member, and what a source read upstream (a Hugging Face tree, a
//! publisher's release feed) is normalised into, so the host and the application know the formats and never a model or program.
//!
//! The JSON Schema of each is [`MODELS_SCHEMA`] and [`RELEASES_SCHEMA`]; the parsers below check what a schema cannot (a digest's
//! length, a revision in a URL). A file with no digest parses, with no check, and is offered as not verified.

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde_json::{Map, Value};

/// The `format` of a list of models.
pub const MODELS_FORMAT: &str = "canoa.models/1";

/// The `format` of a list of a program's releases.
pub const RELEASES_FORMAT: &str = "canoa.releases/1";

/// The formats an extension's catalogs may declare.
pub const FORMATS: [&str; 2] = [MODELS_FORMAT, RELEASES_FORMAT];

/// The JSON Schema (2020-12) of `canoa.models/1`.
pub const MODELS_SCHEMA: &str = include_str!("../../include/canoa.models.schema.json");

/// The JSON Schema (2020-12) of `canoa.releases/1`.
pub const RELEASES_SCHEMA: &str = include_str!("../../include/canoa.releases.schema.json");

/// How far a catalog's own list of files or items is read: more is refused, so a source cannot hand the host a document it cannot hold.
const MAX_ITEMS: usize = 5000;

/// What checks one file: the digest the publisher states, named by its algorithm.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum FileCheck {
    /// Its SHA-256, in lower-case hexadecimal.
    Sha256(String),
    /// Its SHA-512, in lower-case hexadecimal.
    Sha512(String),
    /// Its MD5, in lower-case hexadecimal.
    Md5(String),
    /// A git SHA-1 blob id, which says what the file holds only because its URL names a full commit.
    GitBlob(String),
    /// The publisher gives nothing to check it against, for the reason given.
    Unverified(String),
}

impl FileCheck {
    /// The name of the algorithm the publisher's digest is made with, as people write it; none for a file nothing checks.
    #[must_use]
    pub const fn algorithm(&self) -> Option<&'static str> {
        match self {
            Self::Sha256(_) => Some("SHA-256"),
            Self::Sha512(_) => Some("SHA-512"),
            Self::Md5(_) => Some("MD5"),
            Self::GitBlob(_) => Some("git blob id"),
            Self::Unverified(_) => None,
        }
    }
}

/// One file of a model or a release: where it is, and what says it is the one meant.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CatalogFile {
    /// Where it is downloaded from.
    pub url: String,
    /// Its name inside the folder it is kept in.
    pub file: String,
    /// Its size, when known.
    pub bytes: Option<u64>,
    /// What checks it; none when the catalog gives no digest.
    pub check: Option<FileCheck>,
    /// Whether the digest was computed from content at a pinned address, not given by the publisher.
    pub derived: bool,
}

impl CatalogFile {
    /// The algorithm of the digest this file is downloaded against; none when the publisher states no digest.
    #[must_use]
    pub fn verified_with(&self) -> Option<&'static str> {
        self.check.as_ref().and_then(FileCheck::algorithm)
    }
}

/// The algorithm each of `files` is checked with, each name once and joined by `, `; none when there is no file or one has no digest.
#[must_use]
pub fn verified_with<'a>(files: impl IntoIterator<Item = &'a CatalogFile>) -> Option<String> {
    let mut names: Vec<&'static str> = Vec::new();
    for file in files {
        let name = file.verified_with()?;
        if !names.contains(&name) {
            names.push(name);
        }
    }
    (!names.is_empty()).then(|| names.join(", "))
}

/// One model, voice or other file set a models catalog lists.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ModelItem {
    /// What it is called, unique in its catalog.
    pub id: String,
    /// Its name for people.
    pub label: String,
    /// Who made it.
    pub by: Option<String>,
    /// Words that narrow a list of them.
    pub tags: Vec<String>,
    /// Its licence.
    pub license: Option<String>,
    /// Facts shown beside it, a label and a value each.
    pub facts: Vec<(String, String)>,
    /// What is downloaded for it.
    pub files: Vec<CatalogFile>,
}

impl ModelItem {
    /// The algorithms its files' digests are made with; none when a file has no digest.
    #[must_use]
    pub fn verified_with(&self) -> Option<String> {
        verified_with(&self.files)
    }
}

/// A parsed `canoa.models/1` document.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ModelsCatalog {
    /// What the source read it at: a commit, a version.
    pub revision: Option<String>,
    /// Its items, in the order the source gave them.
    pub items: Vec<ModelItem>,
}

/// One file of a platform's release, and where it goes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReleasePart {
    /// What is downloaded, with the digest that checks it.
    pub file: CatalogFile,
    /// Where it ends up inside the copy.
    pub path: String,
    /// Whether it is started.
    pub executable: bool,
}

/// What one platform downloads for a release.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReleasePlatform {
    /// The file that is started, inside the copy.
    pub launch: Option<String>,
    /// What is downloaded.
    pub parts: Vec<ReleasePart>,
}

/// One version of a program.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Release {
    /// The version.
    pub version: String,
    /// The channels it is the answer of.
    pub channels: Vec<String>,
    /// What the person is told before installing it.
    pub notices: Vec<String>,
    /// What each platform downloads, by `<os>-<arch>`.
    pub platforms: BTreeMap<String, ReleasePlatform>,
}

/// A parsed `canoa.releases/1` document.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReleasesCatalog {
    /// The version each channel is at.
    pub channels: BTreeMap<String, String>,
    /// The releases the catalog lists.
    pub releases: Vec<Release>,
    /// The key that signed the document the source read, and which document that was, when the source verified one.
    pub signed: Option<(String, String)>,
}

fn object<'a>(value: &'a Value, what: &str) -> Result<&'a Map<String, Value>, String> {
    value
        .as_object()
        .ok_or_else(|| format!("{what} is not an object"))
}

fn text<'a>(map: &'a Map<String, Value>, key: &str, what: &str) -> Result<&'a str, String> {
    map.get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| format!("{what} has no {key}"))
}

fn optional(map: &Map<String, Value>, key: &str, what: &str) -> Result<Option<String>, String> {
    match map.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(format!("{what}: {key} is not text")),
    }
}

fn words(map: &Map<String, Value>, key: &str, what: &str) -> Result<Vec<String>, String> {
    match map.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(each)) => each
            .iter()
            .map(|word| {
                word.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| format!("{what}: {key} holds something that is not text"))
            })
            .collect(),
        Some(_) => Err(format!("{what}: {key} is not a list")),
    }
}

fn hex(text: &str, length: usize) -> Option<String> {
    (text.len() == length && text.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| text.to_ascii_lowercase())
}

/// Whether `url` has a path segment that is a full 40-digit commit id, which is what makes a git blob id say what a file holds.
fn names_commit(url: &str) -> bool {
    url.split(['/', '?', '#'])
        .any(|part| hex(part, 40).is_some())
}

fn check_of(
    map: &Map<String, Value>,
    url: &str,
    what: &str,
) -> Result<(Option<FileCheck>, bool), String> {
    let derived = matches!(map.get("derived"), Some(Value::Bool(true)));
    let Some(check) = map.get("check") else {
        return Ok((None, false));
    };
    let check = object(check, &format!("{what}: check"))?;
    let digest = |key: &str, length: usize| -> Result<String, String> {
        let given = check
            .get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{what}: {key} is not text"))?;
        hex(given, length)
            .ok_or_else(|| format!("{what}: {key} is not {length} hexadecimal digits"))
    };
    let kinds: Vec<&str> = check.keys().map(String::as_str).collect();
    let one = match kinds.as_slice() {
        ["sha256"] => FileCheck::Sha256(digest("sha256", 64)?),
        ["sha512"] => FileCheck::Sha512(digest("sha512", 128)?),
        ["md5"] => FileCheck::Md5(digest("md5", 32)?),
        ["git_blob"] => {
            if !names_commit(url) {
                return Err(format!(
                    "{what}: a git blob id checks a file only at a URL that names a full commit"
                ));
            }
            FileCheck::GitBlob(digest("git_blob", 40)?)
        }
        ["unverified"] => FileCheck::Unverified(
            check
                .get("unverified")
                .and_then(Value::as_str)
                .unwrap_or("the publisher gives no checksum")
                .to_owned(),
        ),
        other => {
            return Err(format!(
                "{what}: a check is one of sha256, sha512, md5, git_blob or unverified, not {}",
                other.join(", ")
            ));
        }
    };
    if derived && !names_commit(url) {
        return Err(format!(
            "{what}: a derived digest is believed only at a URL that names a full commit"
        ));
    }
    Ok((Some(one), derived))
}

fn file_of(
    map: &Map<String, Value>,
    default_name: Option<&str>,
    what: &str,
) -> Result<CatalogFile, String> {
    let url = text(map, "url", what)?;
    if !url.starts_with("https://") {
        return Err(format!("{what}: {url:?} is not an https address"));
    }
    let name = match optional(map, "file", what)? {
        Some(name) if !name.is_empty() => name,
        _ => default_name
            .map(str::to_owned)
            .or_else(|| url.rsplit('/').next().map(str::to_owned))
            .filter(|name| !name.is_empty())
            .ok_or_else(|| format!("{what} has no file name"))?,
    };
    if name.contains('\\')
        || name
            .split('/')
            .any(|part| part.is_empty() || part == ".." || part == ".")
    {
        return Err(format!(
            "{what}: {name:?} is not a relative path of plain names"
        ));
    }
    let bytes = match map.get("bytes") {
        None | Some(Value::Null) => None,
        Some(value) => Some(
            value
                .as_u64()
                .ok_or_else(|| format!("{what}: bytes is not a whole number"))?,
        ),
    };
    let (check, derived) = check_of(map, url, what)?;
    Ok(CatalogFile {
        url: url.to_owned(),
        file: name,
        bytes,
        check,
        derived,
    })
}

fn document(source: &str, format: &str) -> Result<Map<String, Value>, String> {
    let value: Value = serde_json::from_str(source)
        .map_err(|error| format!("the catalog is not JSON: {error}"))?;
    let map = object(&value, "the catalog")?.clone();
    match map.get("format").and_then(Value::as_str) {
        Some(found) if found == format => Ok(map),
        Some(other) => Err(format!("the catalog is {other:?}, not {format:?}")),
        None => Err(format!(
            "the catalog names no format; it should be {format:?}"
        )),
    }
}

/// Reads a `canoa.models/1` document.
///
/// # Errors
///
/// The reason the text is not one.
pub fn parse_models(source: &str) -> Result<ModelsCatalog, String> {
    let map = document(source, MODELS_FORMAT)?;
    let items = map
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| "the catalog has no items list".to_owned())?;
    if items.len() > MAX_ITEMS {
        return Err(format!("the catalog lists more than {MAX_ITEMS} items"));
    }
    let mut parsed: Vec<ModelItem> = Vec::new();
    for item in items {
        let item = object(item, "an item")?;
        let id = text(item, "id", "an item")?.to_owned();
        let what = format!("the item {id:?}");
        if parsed.iter().any(|each| each.id == id) {
            return Err(format!("{what} is listed twice"));
        }
        let mut facts = Vec::new();
        if let Some(listed) = item.get("facts") {
            for fact in listed
                .as_array()
                .ok_or_else(|| format!("{what}: facts is not a list"))?
            {
                let fact = object(fact, &format!("{what}: a fact"))?;
                facts.push((
                    text(fact, "label", &format!("{what}: a fact"))?.to_owned(),
                    text(fact, "value", &format!("{what}: a fact"))?.to_owned(),
                ));
            }
        }
        let mut files = Vec::new();
        for file in item
            .get("files")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("{what} has no files list"))?
        {
            files.push(file_of(
                object(file, &format!("{what}: a file"))?,
                None,
                &format!("{what}: a file"),
            )?);
        }
        if files.is_empty() {
            return Err(format!("{what} has no files"));
        }
        parsed.push(ModelItem {
            label: text(item, "label", &what).map_or_else(|_| id.clone(), str::to_owned),
            by: optional(item, "by", &what)?,
            tags: words(item, "tags", &what)?,
            license: optional(item, "license", &what)?,
            facts,
            files,
            id,
        });
    }
    Ok(ModelsCatalog {
        revision: optional(&map, "revision", "the catalog")?,
        items: parsed,
    })
}

/// Reads a `canoa.releases/1` document.
///
/// # Errors
///
/// The reason the text is not one.
pub fn parse_releases(source: &str) -> Result<ReleasesCatalog, String> {
    let map = document(source, RELEASES_FORMAT)?;
    let mut channels = BTreeMap::new();
    if let Some(listed) = map.get("channels") {
        for (name, version) in object(listed, "channels")? {
            channels.insert(
                name.clone(),
                version
                    .as_str()
                    .ok_or_else(|| format!("the channel {name:?} is not at a version"))?
                    .to_owned(),
            );
        }
    }
    let mut releases = Vec::new();
    for release in map
        .get("releases")
        .and_then(Value::as_array)
        .ok_or_else(|| "the catalog has no releases list".to_owned())?
    {
        let release = object(release, "a release")?;
        let version = text(release, "version", "a release")?.to_owned();
        let what = format!("the release {version:?}");
        let mut platforms = BTreeMap::new();
        if let Some(listed) = release.get("platforms") {
            for (name, platform) in object(listed, &format!("{what}: platforms"))? {
                let platform = object(platform, &format!("{what}: {name}"))?;
                let mut parts = Vec::new();
                for part in platform
                    .get("parts")
                    .and_then(Value::as_array)
                    .ok_or_else(|| format!("{what}: {name} has no parts"))?
                {
                    let part = object(part, &format!("{what}: {name}: a part"))?;
                    let place = text(part, "path", &format!("{what}: {name}: a part"))?.to_owned();
                    parts.push(ReleasePart {
                        file: file_of(part, Some(&place), &format!("{what}: {name}: {place}"))?,
                        executable: matches!(part.get("executable"), Some(Value::Bool(true))),
                        path: place,
                    });
                }
                platforms.insert(
                    name.clone(),
                    ReleasePlatform {
                        launch: optional(platform, "launch", &what)?,
                        parts,
                    },
                );
            }
        }
        releases.push(Release {
            channels: words(release, "channels", &what)?,
            notices: words(release, "notices", &what)?,
            platforms,
            version,
        });
    }
    let signed = match map.get("signed") {
        None | Some(Value::Null) => None,
        Some(signed) => {
            let signed = object(signed, "signed")?;
            Some((
                text(signed, "by", "signed")?.to_owned(),
                text(signed, "over", "signed")?.to_owned(),
            ))
        }
    };
    Ok(ReleasesCatalog {
        channels,
        releases,
        signed,
    })
}

/// Whether `text` is a document of `format`, and why not.
///
/// # Errors
///
/// The reason it is not one.
pub fn check_format(format: &str, source: &str) -> Result<(), String> {
    match format {
        MODELS_FORMAT => parse_models(source).map(|_| ()),
        RELEASES_FORMAT => parse_releases(source).map(|_| ()),
        other => Err(format!("{other:?} is not a format this host reads")),
    }
}

/// One line of what the catalog's items are, for a log: the counts that tell a refresh from an empty answer.
#[must_use]
pub fn summary(format: &str, source: &str) -> String {
    match format {
        MODELS_FORMAT => parse_models(source).map_or_else(
            |why| why,
            |catalog| format!("{} items", catalog.items.len()),
        ),
        RELEASES_FORMAT => parse_releases(source).map_or_else(
            |why| why,
            |catalog| catalog.releases.len().to_string() + " releases",
        ),
        other => other.to_owned(),
    }
}

/// Whether `name` matches `pattern`, where `*` stands for any run of characters (a name with no `*` matches only itself).
#[must_use]
pub fn wildcard(pattern: &str, name: &str) -> bool {
    let mut parts = pattern.split('*');
    let first = parts.next().unwrap_or_default();
    let Some(mut rest) = name.strip_prefix(first) else {
        return false;
    };
    let mut parts: Vec<&str> = parts.collect();
    let Some(last) = parts.pop() else {
        return rest.is_empty();
    };
    for part in parts {
        let Some(at) = rest.find(part) else {
            return false;
        };
        rest = &rest[at + part.len()..];
    }
    rest.ends_with(last)
}

impl CatalogFile {
    /// A file at `url` kept as `file`, with no digest yet.
    #[must_use]
    pub fn new(url: impl Into<String>, file: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            file: file.into(),
            bytes: None,
            check: None,
            derived: false,
        }
    }

    /// This file with its size.
    #[must_use]
    pub const fn sized(mut self, bytes: u64) -> Self {
        self.bytes = Some(bytes);
        self
    }

    /// This file checked by `check`.
    #[must_use]
    pub fn checked(mut self, check: FileCheck) -> Self {
        self.check = Some(check);
        self
    }

    /// This file's digest marked as one computed from content at a pinned address.
    #[must_use]
    pub const fn derived(mut self) -> Self {
        self.derived = true;
        self
    }

    fn to_value(&self, with_name: bool) -> Value {
        let mut map = Map::new();
        map.insert("url".into(), Value::String(self.url.clone()));
        if with_name {
            map.insert("file".into(), Value::String(self.file.clone()));
        }
        if let Some(bytes) = self.bytes {
            map.insert("bytes".into(), Value::from(bytes));
        }
        let (key, text) = match &self.check {
            Some(FileCheck::Sha256(value)) => ("sha256", value),
            Some(FileCheck::Sha512(value)) => ("sha512", value),
            Some(FileCheck::Md5(value)) => ("md5", value),
            Some(FileCheck::GitBlob(value)) => ("git_blob", value),
            Some(FileCheck::Unverified(why)) => ("unverified", why),
            None => return Value::Object(map),
        };
        let mut check = Map::new();
        check.insert(key.into(), Value::String(text.clone()));
        map.insert("check".into(), Value::Object(check));
        if self.derived {
            map.insert("derived".into(), Value::Bool(true));
        }
        Value::Object(map)
    }
}

impl ModelItem {
    /// An item with no tags, facts or files yet.
    #[must_use]
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            by: None,
            tags: Vec::new(),
            license: None,
            facts: Vec::new(),
            files: Vec::new(),
        }
    }
}

impl ModelsCatalog {
    /// A catalog read at `revision` listing `items`.
    #[must_use]
    pub fn new(revision: Option<String>, items: Vec<ModelItem>) -> Self {
        Self { revision, items }
    }

    /// This catalog as the text of a `canoa.models/1` document.
    #[must_use]
    pub fn to_json(&self) -> String {
        let items: Vec<Value> = self
            .items
            .iter()
            .map(|item| {
                let mut map = Map::new();
                map.insert("id".into(), Value::String(item.id.clone()));
                map.insert("label".into(), Value::String(item.label.clone()));
                if let Some(by) = &item.by {
                    map.insert("by".into(), Value::String(by.clone()));
                }
                if !item.tags.is_empty() {
                    map.insert("tags".into(), Value::from(item.tags.clone()));
                }
                if let Some(license) = &item.license {
                    map.insert("license".into(), Value::String(license.clone()));
                }
                if !item.facts.is_empty() {
                    let facts = item
                        .facts
                        .iter()
                        .map(|(label, value)| {
                            let mut fact = Map::new();
                            fact.insert("label".into(), Value::String(label.clone()));
                            fact.insert("value".into(), Value::String(value.clone()));
                            Value::Object(fact)
                        })
                        .collect();
                    map.insert("facts".into(), Value::Array(facts));
                }
                map.insert(
                    "files".into(),
                    Value::Array(item.files.iter().map(|file| file.to_value(true)).collect()),
                );
                Value::Object(map)
            })
            .collect();
        let mut document = Map::new();
        document.insert("format".into(), Value::String(MODELS_FORMAT.into()));
        if let Some(revision) = &self.revision {
            document.insert("revision".into(), Value::String(revision.clone()));
        }
        document.insert("items".into(), Value::Array(items));
        serde_json::to_string_pretty(&Value::Object(document)).unwrap_or_default()
    }
}

impl ReleasePart {
    /// A file placed at `path` in the copy.
    #[must_use]
    pub fn new(file: CatalogFile, path: impl Into<String>) -> Self {
        Self {
            file,
            path: path.into(),
            executable: false,
        }
    }

    /// This part started once placed.
    #[must_use]
    pub const fn executable(mut self) -> Self {
        self.executable = true;
        self
    }
}

impl ReleasePlatform {
    /// What one platform downloads, started from `launch`.
    #[must_use]
    pub const fn new(launch: Option<String>, parts: Vec<ReleasePart>) -> Self {
        Self { launch, parts }
    }
}

impl Release {
    /// A release of `version`, on no channel, with nothing to download yet.
    #[must_use]
    pub fn new(version: impl Into<String>) -> Self {
        Self {
            version: version.into(),
            channels: Vec::new(),
            notices: Vec::new(),
            platforms: BTreeMap::new(),
        }
    }
}

impl ReleasesCatalog {
    /// A catalog with the channels' versions and the releases it lists, signed by nobody.
    #[must_use]
    pub const fn new(channels: BTreeMap<String, String>, releases: Vec<Release>) -> Self {
        Self {
            channels,
            releases,
            signed: None,
        }
    }

    /// This catalog as the text of a `canoa.releases/1` document.
    #[must_use]
    pub fn to_json(&self) -> String {
        let releases: Vec<Value> = self
            .releases
            .iter()
            .map(|release| {
                let mut map = Map::new();
                map.insert("version".into(), Value::String(release.version.clone()));
                if !release.channels.is_empty() {
                    map.insert("channels".into(), Value::from(release.channels.clone()));
                }
                if !release.notices.is_empty() {
                    map.insert("notices".into(), Value::from(release.notices.clone()));
                }
                let mut platforms = Map::new();
                for (name, platform) in &release.platforms {
                    let mut each = Map::new();
                    if let Some(launch) = &platform.launch {
                        each.insert("launch".into(), Value::String(launch.clone()));
                    }
                    let parts = platform
                        .parts
                        .iter()
                        .map(|part| {
                            let Value::Object(mut one) =
                                part.file.to_value(part.file.file != part.path)
                            else {
                                return Value::Null;
                            };
                            one.insert("path".into(), Value::String(part.path.clone()));
                            if part.executable {
                                one.insert("executable".into(), Value::Bool(true));
                            }
                            Value::Object(one)
                        })
                        .collect();
                    each.insert("parts".into(), Value::Array(parts));
                    platforms.insert(name.clone(), Value::Object(each));
                }
                map.insert("platforms".into(), Value::Object(platforms));
                Value::Object(map)
            })
            .collect();
        let mut document = Map::new();
        document.insert("format".into(), Value::String(RELEASES_FORMAT.into()));
        let channels: Map<String, Value> = self
            .channels
            .iter()
            .map(|(name, version)| (name.clone(), Value::String(version.clone())))
            .collect();
        document.insert("channels".into(), Value::Object(channels));
        document.insert("releases".into(), Value::Array(releases));
        if let Some((by, over)) = &self.signed {
            let mut signed = Map::new();
            signed.insert("by".into(), Value::String(by.clone()));
            signed.insert("over".into(), Value::String(over.clone()));
            document.insert("signed".into(), Value::Object(signed));
        }
        serde_json::to_string_pretty(&Value::Object(document)).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_patterns_take_a_star_for_any_run_of_characters() {
        assert!(wildcard("ggml-*.bin", "ggml-base.en.bin"));
        assert!(!wildcard("ggml-*.bin", "ggml-base.zip"));
        assert!(wildcard("ggml-base.bin", "ggml-base.bin"));
        assert!(!wildcard("ggml-base.bin", "ggml-base.bin.zip"));
        assert!(wildcard("*large*", "ggml-large-v3.bin"));
        assert!(wildcard("a*b*c", "aXbYc"));
        assert!(!wildcard("a*a", "a"));
    }

    #[test]
    fn a_catalog_written_reads_back_the_same() {
        let mut item = ModelItem::new("base", "Base");
        item.tags = vec!["speech-to-text".into()];
        item.facts = vec![("Size".into(), "148 MB".into())];
        item.files = vec![
            CatalogFile::new(
                "https://huggingface.co/x/resolve/5359861c5359861c5359861c5359861c5359861c/ggml-base.bin",
                "ggml-base.bin",
            )
            .sized(7)
            .checked(FileCheck::Sha256(SHA256.into())),
            CatalogFile::new(
                "https://huggingface.co/x/resolve/5359861c5359861c5359861c5359861c5359861c/a.json",
                "a.json",
            )
            .checked(FileCheck::GitBlob("1799".repeat(10)))
            .derived(),
        ];
        let written = ModelsCatalog::new(Some("5359861c".into()), vec![item]);
        let read = parse_models(&written.to_json()).expect("a document");
        assert_eq!(read, written);

        let mut release = Release::new("1.2.3");
        release.channels = vec!["stable".into()];
        release.notices = vec!["a notice".into()];
        let file = CatalogFile::new("https://x.example/p.exe", "p.exe")
            .checked(FileCheck::Sha512("ab".repeat(64)));
        release.platforms.insert(
            "windows-x86_64".into(),
            ReleasePlatform::new(
                Some("p.exe".into()),
                vec![ReleasePart::new(file, "p.exe").executable()],
            ),
        );
        let mut channels = BTreeMap::new();
        channels.insert("stable".into(), "1.2.3".into());
        let mut written = ReleasesCatalog::new(channels, vec![release]);
        written.signed = Some(("5A3F".into(), "manifest.json".into()));
        assert_eq!(parse_releases(&written.to_json()), Ok(written));
    }

    const SHA256: &str = "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe";

    fn models(check: &str) -> String {
        format!(
            r#"{{"format":"canoa.models/1","revision":"5359861c","items":[{{"id":"base","label":"Base","by":"ggerganov",
            "tags":["multilingual"],"facts":[{{"label":"Language","value":"every"}}],
            "files":[{{"url":"https://huggingface.co/x/resolve/5359861c5359861c5359861c5359861c5359861c/ggml-base.bin",
            "file":"ggml-base.bin","bytes":147951465{check}}}]}}]}}"#
        )
    }

    #[test]
    fn a_models_document_reads_with_its_digest() {
        let catalog = parse_models(&models(&format!(r#","check":{{"sha256":"{SHA256}"}}"#)))
            .expect("a document");
        assert_eq!(catalog.revision.as_deref(), Some("5359861c"));
        let item = &catalog.items[0];
        assert_eq!(item.files[0].check, Some(FileCheck::Sha256(SHA256.into())));
        assert_eq!(item.verified_with().as_deref(), Some("SHA-256"));
        assert_eq!(item.facts, [("Language".to_owned(), "every".to_owned())]);
    }

    #[test]
    fn a_file_is_kept_at_a_relative_path_that_stays_inside_the_folder() {
        for (name, fine) in [
            ("base/model.bin", true),
            ("a/../b", false),
            ("/abs", false),
            ("a//b", false),
            ("a\\b", false),
            ("..", false),
        ] {
            let doc = models("").replace("\"ggml-base.bin\"", &format!("{name:?}"));
            assert_eq!(parse_models(&doc).is_ok(), fine, "{name}");
        }
    }

    #[test]
    fn a_file_with_no_digest_is_offered_as_not_verified() {
        let catalog = parse_models(&models("")).expect("a document");
        assert_eq!(catalog.items[0].verified_with(), None);
    }

    #[test]
    fn a_digest_is_named_by_its_algorithm_whichever_it_is() {
        let sha512 = format!(r#","check":{{"sha512":"{}"}}"#, "ab".repeat(64));
        let md5 = r#","check":{"md5":"d41d8cd98f00b204e9800998ecf8427e"}"#;
        for (check, name) in [(sha512.as_str(), "SHA-512"), (md5, "MD5")] {
            let catalog = parse_models(&models(check)).expect("a document");
            assert_eq!(catalog.items[0].verified_with().as_deref(), Some(name));
        }
        let short = parse_models(&models(r#","check":{"md5":"d41d8cd9"}"#));
        assert!(short.is_err_and(|why| why.contains("32 hexadecimal")));
    }

    #[test]
    fn an_item_with_one_unchecked_file_is_not_verified() {
        let mut item = ModelItem::new("x", "X");
        item.files = vec![
            CatalogFile::new("https://x.example/a", "a").checked(FileCheck::Sha256(SHA256.into())),
            CatalogFile::new("https://x.example/b", "b"),
        ];
        assert_eq!(item.verified_with(), None);
        item.files[1] = CatalogFile::new("https://x.example/b", "b")
            .checked(FileCheck::Md5("d41d8cd98f00b204e9800998ecf8427e".into()));
        assert_eq!(item.verified_with().as_deref(), Some("SHA-256, MD5"));
    }

    #[test]
    fn a_git_blob_id_needs_a_url_that_names_a_commit() {
        let blob = r#","check":{"git_blob":"1799aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#;
        assert_eq!(
            parse_models(&models(blob)).map(|catalog| catalog.items[0].verified_with()),
            Ok(Some("git blob id".to_owned()))
        );
        let moving = models(blob).replace("5359861c5359861c5359861c5359861c5359861c", "main");
        assert!(parse_models(&moving).is_err_and(|why| why.contains("commit")));
    }

    #[test]
    fn a_document_of_another_format_is_named() {
        assert!(
            parse_models(r#"{"format":"canoa.releases/1","items":[]}"#)
                .is_err_and(|why| why.contains("canoa.releases/1"))
        );
        assert!(parse_models("[]").is_err());
    }

    #[test]
    fn a_releases_document_reads_its_platforms_and_signature() {
        let text = format!(
            r#"{{"format":"canoa.releases/1","channels":{{"stable":"1.0.92"}},
            "releases":[{{"version":"1.0.92","channels":["stable"],"notices":["n"],
            "platforms":{{"windows-x86_64":{{"launch":"grok.exe","parts":[
            {{"url":"https://x.ai/cli/grok.exe","path":"grok.exe","executable":true,"check":{{"sha256":"{SHA256}"}}}}]}}}}}}],
            "signed":{{"by":"5A3F","over":"manifest.json"}}}}"#
        );
        let catalog = parse_releases(&text).expect("a document");
        assert_eq!(catalog.channels["stable"], "1.0.92");
        let part = &catalog.releases[0].platforms["windows-x86_64"].parts[0];
        assert!(part.executable);
        assert_eq!(part.file.file, "grok.exe");
        assert_eq!(
            catalog.signed,
            Some(("5A3F".into(), "manifest.json".into()))
        );
        assert_eq!(check_format(RELEASES_FORMAT, &text), Ok(()));
    }

    #[test]
    fn the_schemas_are_json_that_names_their_format() {
        for (schema, format) in [
            (MODELS_SCHEMA, MODELS_FORMAT),
            (RELEASES_SCHEMA, RELEASES_FORMAT),
        ] {
            let schema: Value = serde_json::from_str(schema).expect("JSON");
            assert_eq!(schema["properties"]["format"]["const"], format);
        }
    }
}
