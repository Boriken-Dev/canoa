//! Canoa's extensions catalog: a TOML file in a git repository, named as `<repository>@<ref>#extensions.toml`, that lists
//! extensions the host can install. Each `[extensions.NAME]` is pinned, so nothing of it is kept before what was fetched has
//! the digest the entry gives. The format is Canoa's own and is written down in `docs/configuration.md`.
//!
//! Reading is static and total: a file that cannot be read is refused whole, an entry that is not valid is refused alone, and an
//! informational field that is not valid is left out.

#![allow(
    clippy::std_instead_of_alloc,
    reason = "the map is the std one the rest of the index uses"
)]

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

/// The largest catalog file read.
pub(crate) const MAX_BYTES: usize = 4 * 1024 * 1024;
/// The most entries one catalog lists.
pub(crate) const MAX_ENTRIES: usize = 2000;

/// Why an entry is not listed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// The name is not one canoa.cfg keeps an entry under.
    InvalidName,
    /// Two names are equal once case is ignored: both are refused.
    DuplicateName,
    /// Not a table, an unknown key, or a key of the wrong type or form.
    InvalidEntry,
    /// A source git and the download tool cannot be given: empty, with a control character, a scheme neither speaks, or a path
    /// that is no path.
    InvalidSource,
    /// A `pin` or `sha256` that is not a digest.
    InvalidPin,
    /// An entry with no digest, or none its kind of source takes.
    Unpinned,
    /// A `ref` where the source has a ref of its own or takes none, so two refs or one that means nothing.
    RefNotAllowed,
    /// A relative source that leaves the repository, so names nothing.
    LeavesRepository,
}

impl Refusal {
    /// The word the host answers with.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidName => "invalid-name",
            Self::DuplicateName => "duplicate-name",
            Self::InvalidEntry => "invalid-entry",
            Self::InvalidSource => "invalid-source",
            Self::InvalidPin => "invalid-pin",
            Self::Unpinned => "unpinned",
            Self::RefNotAllowed => "ref-not-allowed",
            Self::LeavesRepository => "leaves-repository",
        }
    }
}

/// Where an extension's bytes are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    /// A folder of a git repository, wherever git reads one (a local folder, `file:`, `ssh`, `https`, with the credentials the
    /// location holds), at a ref (the repository's head when none).
    Git {
        repo: String,
        reference: Option<String>,
        path: Option<String>,
    },
    /// One file at an `http` or `https` URL, which may hold the credentials or the signature it needs.
    Url(String),
}

/// The digest what is fetched must have.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Digest {
    /// The tree digest of the folder, as `sha256:` and 64 hexadecimal digits: what an overlay's `pin` is.
    Tree(String),
    /// The SHA-256 of the file, as 64 hexadecimal digits.
    File(String),
}

/// What an extension needs of the host.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Needs {
    pub(crate) kind: Option<String>,
    pub(crate) version: Option<String>,
    pub(crate) platforms: Option<Vec<String>>,
}

/// Why a host cannot use an extension, in the order it is checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unfit {
    Kind,
    Version,
    Platform,
}

impl Needs {
    /// Whether a host that implements `kinds` (each with its versions) and runs on `os` and `arch` can use the extension. A
    /// kind and a version are compared as written; a platform is an OS name or an OS name, `-` and an architecture.
    pub(crate) fn fits(
        &self,
        kinds: &[(&str, &[&str])],
        os: &str,
        arch: &str,
    ) -> Result<(), Unfit> {
        if let Some(kind) = &self.kind {
            let Some((_, versions)) = kinds.iter().find(|(name, _)| name == kind) else {
                return Err(Unfit::Kind);
            };
            if self
                .version
                .as_ref()
                .is_some_and(|version| !versions.contains(&version.as_str()))
            {
                return Err(Unfit::Version);
            }
        }
        let named = format!("{os}-{arch}");
        match &self.platforms {
            Some(platforms) if !platforms.iter().any(|each| each == os || *each == named) => {
                Err(Unfit::Platform)
            }
            _ => Ok(()),
        }
    }

    fn json(&self) -> Value {
        let mut out = Map::new();
        if let Some(kind) = &self.kind {
            out.insert("kind".into(), json!(kind));
        }
        if let Some(version) = &self.version {
            out.insert("version".into(), json!(version));
        }
        if let Some(platforms) = &self.platforms {
            out.insert("platforms".into(), json!(platforms));
        }
        Value::Object(out)
    }
}

/// One extension a catalog lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Entry {
    pub(crate) name: String,
    pub(crate) source: Source,
    pub(crate) digest: Digest,
    /// For a tree, the library file in it the host loads.
    pub(crate) load: Option<String>,
    pub(crate) provider: String,
    pub(crate) needs: Needs,
    pub(crate) description: Option<String>,
    /// The valid informational fields: `summary`, `icon`, `homepage`, `repository`, `license`, `authors`, `tags`.
    pub(crate) about: Map<String, Value>,
}

/// What a read of one catalog file gives.
#[derive(Debug, Default)]
pub(crate) struct Catalog {
    pub(crate) name: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) entries: BTreeMap<String, Entry>,
    pub(crate) refused: Vec<(String, Refusal)>,
}

/// Where the catalog file is, which a relative source is read from.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Base<'a> {
    pub(crate) repo: &'a str,
    pub(crate) reference: &'a str,
    /// The catalog file's path in the repository.
    pub(crate) path: &'a str,
}

const KEYS: [&str; 9] = [
    "source",
    "ref",
    "pin",
    "sha256",
    "load",
    "provider",
    "needs",
    "description",
    "about",
];

/// A line of display text: not empty, with no control character, line or paragraph separator, or bidirectional override.
fn is_line(text: &str, most: usize) -> bool {
    !text.is_empty()
        && text.chars().count() <= most
        && !text.chars().any(|c| {
            c.is_control()
                || matches!(c, '\u{2028}' | '\u{2029}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
}

fn is_hex64(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_tool_name(text: &str) -> bool {
    let bytes = text.as_bytes();
    (1..=128).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
}

fn is_version(text: &str) -> bool {
    let bytes = text.as_bytes();
    (1..=64).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'+' | b'_' | b'-'))
}

/// `os` or `os-arch`: lower-case letters and digits, the architecture also `_`.
fn is_platform(text: &str) -> bool {
    let (os, arch) = match text.split_once('-') {
        Some((os, arch)) => (os, Some(arch)),
        None => (text, None),
    };
    let word = |part: &str| {
        !part.is_empty()
            && part
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
    };
    os.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && word(os)
        && arch.is_none_or(|arch| arch.split('_').all(word))
}

/// A `/`-separated path inside a folder: no empty, `.` or `..` name (which name nothing or climb out), no control character and no
/// backslash (a separator on Windows, so a second way to climb out).
fn is_plain_path(path: &str) -> bool {
    !path.is_empty()
        && path.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && !part.chars().any(|c| c.is_control() || c == '\\')
        })
}

/// A ref git can be asked for: not empty, not starting with `-` (git would read an option), none of the characters a ref name or
/// the refspec the host builds from it cannot hold.
fn is_ref(text: &str) -> bool {
    !text.is_empty()
        && !text.starts_with('-')
        && !text.contains("..")
        && !text.chars().any(|c| {
            c.is_control()
                || c.is_whitespace()
                || matches!(c, ':' | '\\' | '@' | '#' | '?' | '*' | '[' | '^' | '~')
        })
}

fn is_web_url(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    (lower.starts_with("https://") || lower.starts_with("http://"))
        && !text.chars().any(|c| c.is_control() || c.is_whitespace())
}

/// The informational fields that are valid.
fn about_of(value: &Value) -> Map<String, Value> {
    let mut out = Map::new();
    let Some(table) = value.as_object() else {
        return out;
    };
    for (key, value) in table {
        let valid = match (key.as_str(), value) {
            ("summary", Value::String(text)) => is_line(text, 200),
            ("license", Value::String(text)) => is_line(text, 128),
            ("homepage" | "repository", Value::String(text)) => is_web_url(text),
            ("icon", Value::String(text)) => {
                let lower = text.to_ascii_lowercase();
                is_plain_path(text) && (lower.ends_with(".png") || lower.ends_with(".webp"))
            }
            ("authors", Value::Array(list)) => {
                list.len() <= 16
                    && list
                        .iter()
                        .all(|each| each.as_str().is_some_and(|text| is_line(text, 200)))
            }
            ("tags", Value::Array(list)) => {
                list.len() <= 16
                    && list.iter().enumerate().all(|(at, each)| {
                        each.as_str().is_some_and(|tag| {
                            tag.len() <= 32
                                && !tag.is_empty()
                                && tag.split('-').all(|part| {
                                    !part.is_empty()
                                        && part.bytes().all(|byte| {
                                            byte.is_ascii_lowercase() || byte.is_ascii_digit()
                                        })
                                })
                                && !list[..at].contains(each)
                        })
                    })
            }
            _ => false,
        };
        if valid {
            out.insert(key.clone(), value.clone());
        }
    }
    out
}

fn needs_of(value: &Value) -> Option<Needs> {
    let table = value.as_object()?;
    let mut needs = Needs::default();
    for (key, value) in table {
        match key.as_str() {
            "kind" => {
                needs.kind = Some(value.as_str().filter(|text| is_tool_name(text))?.to_owned())
            }
            "version" => {
                needs.version = Some(value.as_str().filter(|text| is_version(text))?.to_owned());
            }
            "platforms" => {
                let list = value.as_array().filter(|list| !list.is_empty())?;
                let mut platforms: Vec<String> = Vec::new();
                for each in list {
                    let platform = each.as_str().filter(|text| is_platform(text))?;
                    if platforms.iter().any(|seen| seen == platform) {
                        return None;
                    }
                    platforms.push(platform.to_owned());
                }
                needs.platforms = Some(platforms);
            }
            _ => return None,
        }
    }
    (needs.version.is_none() || needs.kind.is_some()).then_some(needs)
}

/// `a/b/../c` as `a/c`, or `None` when it climbs out of the root.
fn join_path(folder: &str, relative: &str) -> Option<String> {
    let mut parts: Vec<&str> = folder.split('/').filter(|part| !part.is_empty()).collect();
    for part in relative.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    Some(parts.join("/"))
}

/// The end of the authority of a location: after `://`, the first `/`; for `host:path` and a drive path, the `:`; else 0.
fn authority_end(body: &str) -> usize {
    match body.find("://") {
        Some(at) => body[at + 3..]
            .find('/')
            .map_or(body.len(), |slash| at + 3 + slash),
        None => body.find(':').unwrap_or(0),
    }
}

/// The source `text` names, with `reference` (the entry's `ref`), made absolute against `base` when it is relative.
fn source_of(
    text: &str,
    reference: Option<&str>,
    base: Option<&Base<'_>>,
) -> Result<Source, Refusal> {
    let text = text.trim();
    if text.is_empty() || text.chars().any(char::is_control) {
        return Err(Refusal::InvalidSource);
    }
    if text.starts_with("./") || text.starts_with("../") {
        let base = base.ok_or(Refusal::InvalidSource)?;
        if text.contains(['\\', '#']) {
            return Err(Refusal::InvalidSource);
        }
        if reference.is_some() {
            return Err(Refusal::RefNotAllowed);
        }
        let folder = base.path.rsplit_once('/').map_or("", |(folder, _)| folder);
        let path = join_path(folder, text).ok_or(Refusal::LeavesRepository)?;
        if !path.is_empty() && !is_plain_path(&path) {
            return Err(Refusal::InvalidSource);
        }
        return Ok(Source::Git {
            repo: base.repo.to_owned(),
            reference: Some(base.reference.to_owned()),
            path: (!path.is_empty()).then_some(path),
        });
    }
    let (forced, body) = match text.get(..4) {
        Some(prefix) if prefix.eq_ignore_ascii_case("git+") => (true, &text[4..]),
        _ => (false, text),
    };
    let lower = body.to_ascii_lowercase();
    let web = lower.starts_with("https://") || lower.starts_with("http://");
    let remote_git = lower.starts_with("ssh://")
        || lower.starts_with("git://")
        || lower.starts_with("file:")
        || body.starts_with("git@");
    // Where git reads a repository from this computer: a folder by its path, absolute or with a drive.
    let local = body.starts_with(['/', '\\']) || body.as_bytes().get(1) == Some(&b':');
    if !(web || remote_git || local) {
        // A scheme neither git nor the download tool speaks, or text that names nothing.
        return Err(Refusal::InvalidSource);
    }
    let (before, path) = match body.split_once('#') {
        Some((before, path)) => (before, Some(path)),
        None => (body, None),
    };
    let mut location = before;
    let mut own_ref = None;
    if let Some(at) = before.rfind('@')
        && at > authority_end(before)
        && !before[at + 1..].contains(['/', ':', '\\'])
    {
        location = &before[..at];
        own_ref = Some(&before[at + 1..]);
    }
    if web && !forced && !location.ends_with(".git") {
        // One file at a URL: it has no ref and no folder, and `@` is part of its name.
        if path.is_some() {
            return Err(Refusal::InvalidSource);
        }
        if reference.is_some() {
            return Err(Refusal::RefNotAllowed);
        }
        return Ok(Source::Url(body.to_owned()));
    }
    if own_ref.is_some() && reference.is_some() {
        return Err(Refusal::RefNotAllowed);
    }
    let chosen = own_ref.or(reference);
    if chosen.is_some_and(|text| !is_ref(text)) {
        return Err(Refusal::InvalidEntry);
    }
    let path = match path {
        None => None,
        Some(path) => {
            let path = path.trim_matches('/');
            if path.is_empty() {
                None
            } else if is_plain_path(path) {
                Some(path.to_owned())
            } else {
                return Err(Refusal::InvalidSource);
            }
        }
    };
    Ok(Source::Git {
        repo: location.to_owned(),
        reference: chosen.map(str::to_owned),
        path,
    })
}

/// The parts of an entry's table that do not depend on where its source is: the provider, the file to load and what it needs.
fn parts_of(fields: &Map<String, Value>) -> Result<(String, Option<String>, Needs), Refusal> {
    let provider = fields
        .get("provider")
        .and_then(Value::as_str)
        .filter(|text| is_tool_name(text))
        .ok_or(Refusal::InvalidEntry)?;
    let load = match fields.get("load") {
        None => None,
        Some(Value::String(load)) if is_plain_path(load) => Some(load.clone()),
        Some(_) => return Err(Refusal::InvalidEntry),
    };
    let needs = match fields.get("needs") {
        None => Needs::default(),
        Some(value) => needs_of(value).ok_or(Refusal::InvalidEntry)?,
    };
    Ok((provider.to_owned(), load, needs))
}

/// The entry once its source is known: the digest its kind of source takes, and the informational fields.
fn finish(
    name: &str,
    fields: &Map<String, Value>,
    source: Source,
    parts: (String, Option<String>, Needs),
) -> Result<Entry, Refusal> {
    let (provider, load, needs) = parts;
    let digest = match &source {
        Source::Url(_) => {
            if load.is_some() || fields.contains_key("pin") {
                return Err(Refusal::InvalidEntry);
            }
            match fields.get("sha256") {
                None => return Err(Refusal::Unpinned),
                Some(Value::String(hex)) if is_hex64(hex) => Digest::File(hex.to_ascii_lowercase()),
                Some(_) => return Err(Refusal::InvalidPin),
            }
        }
        Source::Git { .. } => {
            if fields.contains_key("sha256") {
                return Err(Refusal::InvalidEntry);
            }
            match fields.get("pin") {
                None => return Err(Refusal::Unpinned),
                Some(Value::String(pin)) if pin.strip_prefix("sha256:").is_some_and(is_hex64) => {
                    Digest::Tree(pin.to_ascii_lowercase())
                }
                Some(_) => return Err(Refusal::InvalidPin),
            }
        }
    };
    Ok(Entry {
        name: name.to_owned(),
        source,
        digest,
        load,
        provider,
        needs,
        description: fields
            .get("description")
            .and_then(Value::as_str)
            .filter(|text| text.chars().count() <= 2000 && !text.contains('\0'))
            .map(str::to_owned),
        about: fields.get("about").map(about_of).unwrap_or_default(),
    })
}

/// One entry from its table in a catalog file, whose place is `base`: a relative source resolves there.
pub(crate) fn entry_of(name: &str, table: &Value, base: &Base<'_>) -> Result<Entry, Refusal> {
    let Some(fields) = table.as_object() else {
        return Err(Refusal::InvalidEntry);
    };
    if fields.keys().any(|key| !KEYS.contains(&key.as_str())) {
        return Err(Refusal::InvalidEntry);
    }
    let source_text = fields
        .get("source")
        .and_then(Value::as_str)
        .ok_or(Refusal::InvalidEntry)?;
    let parts = parts_of(fields)?;
    let reference = match fields.get("ref") {
        None => None,
        Some(Value::String(text)) => Some(text.as_str()),
        Some(_) => return Err(Refusal::InvalidEntry),
    };
    let source = source_of(source_text, reference, Some(base))?;
    finish(name, fields, source, parts)
}

/// Reads a catalog file. `base` says where it is, so a relative source resolves in the same repository at the same ref.
///
/// # Errors
///
/// Why the whole file is refused: too large, not TOML, or no `[extensions]` table.
pub(crate) fn read(text: &str, base: &Base<'_>) -> Result<Catalog, String> {
    if text.len() > MAX_BYTES {
        return Err(format!(
            "It is larger than the {MAX_BYTES} bytes this host reads"
        ));
    }
    let document: Value = toml::from_str(text).map_err(|_| "It is not a TOML file".to_owned())?;
    let Some(extensions) = document.get("extensions").and_then(Value::as_object) else {
        return Err("It has no [extensions] table".to_owned());
    };
    let line = |key: &str| {
        document
            .get(key)
            .and_then(Value::as_str)
            .filter(|text| is_line(text, 200))
            .map(str::to_owned)
    };
    let mut catalog = Catalog {
        name: line("name"),
        description: line("description"),
        ..Catalog::default()
    };
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for name in extensions.keys() {
        *seen.entry(name.to_ascii_lowercase()).or_default() += 1;
    }
    for (name, table) in extensions.iter().take(MAX_ENTRIES) {
        if crate::config::check_name(name).is_err() {
            catalog.refused.push((name.clone(), Refusal::InvalidName));
        } else if seen[&name.to_ascii_lowercase()] > 1 {
            catalog.refused.push((name.clone(), Refusal::DuplicateName));
        } else {
            match entry_of(name, table, base) {
                Ok(entry) => {
                    catalog.entries.insert(name.clone(), entry);
                }
                Err(why) => catalog.refused.push((name.clone(), why)),
            }
        }
    }
    Ok(catalog)
}

impl Entry {
    /// The source in parts, as the index gives every entry's `origin`.
    pub(crate) fn origin_json(&self) -> Value {
        match &self.source {
            Source::Git {
                repo,
                reference,
                path,
            } => json!({"kind": "git", "location": repo, "ref": reference, "path": path}),
            Source::Url(url) => json!({"kind": "url", "location": url, "ref": null, "path": null}),
        }
    }

    /// What the index gives beside the origin: the provider, the digest, the file to load and what the entry needs.
    pub(crate) fn extension_json(&self) -> Value {
        let mut out = Map::new();
        out.insert("provider".into(), json!(self.provider));
        match &self.digest {
            Digest::Tree(pin) => out.insert("pin".into(), json!(pin)),
            Digest::File(sha256) => out.insert("sha256".into(), json!(sha256)),
        };
        if let Some(load) = &self.load {
            out.insert("load".into(), json!(load));
        }
        out.insert("needs".into(), self.needs.json());
        Value::Object(out)
    }

    /// The entry an index entry stands for, read again from the parts the index kept: what the host installs is checked once more
    /// with every rule of the digest, the file and `needs`, and the source is the one the index read from the catalog.
    pub(crate) fn from_index(name: &str, entry: &Value) -> Result<Self, Refusal> {
        let origin = entry.get("origin").ok_or(Refusal::InvalidEntry)?;
        let listed = entry.get("extension").ok_or(Refusal::InvalidEntry)?;
        let text = |key: &str| origin.get(key).and_then(Value::as_str).map(str::to_owned);
        let location = text("location").ok_or(Refusal::InvalidEntry)?;
        if location.is_empty() || location.chars().any(char::is_control) {
            return Err(Refusal::InvalidSource);
        }
        let source = match text("kind").as_deref() {
            Some("git") => {
                let reference = text("ref");
                if reference.as_deref().is_some_and(|each| !is_ref(each)) {
                    return Err(Refusal::InvalidSource);
                }
                let path = text("path");
                if path.as_deref().is_some_and(|each| !is_plain_path(each)) {
                    return Err(Refusal::InvalidSource);
                }
                Source::Git {
                    repo: location,
                    reference,
                    path,
                }
            }
            Some("url") if is_web_url(&location) => Source::Url(location),
            _ => return Err(Refusal::InvalidSource),
        };
        let mut fields = Map::new();
        for key in ["pin", "sha256", "load", "provider", "needs"] {
            if let Some(value) = listed.get(key) {
                fields.insert(key.into(), value.clone());
            }
        }
        let parts = parts_of(&fields)?;
        finish(name, &fields, source, parts)
    }
}
