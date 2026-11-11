//! The five stores built into this binary: one for each agent program Canoa knows how to find, check for updates and install.
//! Each is a [`Recipe`](super::engine::Recipe): where the publisher serves the program, how a download is checked, and how the
//! version is read. None runs a publisher's installer, and none changes anything outside the version's folder.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use canoa::store::{InstallTools, Platform, Serves};
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{Map, Status, Value};

pub mod agy;
pub mod anthropic;
pub mod codex;
pub mod grok;
pub mod pi;

/// The four platforms the stores are measured on.
#[must_use]
pub fn measured_platforms() -> Vec<Platform> {
    ["windows", "linux"]
        .into_iter()
        .flat_map(|os| ["x86_64", "aarch64"].map(|arch| Platform::new(os, arch)))
        .collect()
}

/// A provider that takes the program when its settings say `key` is `value`.
#[must_use]
pub fn serves_when(provider: &str, key: &str, value: &str) -> Serves {
    let mut when = Map::new();
    when.set(key, Value::from(value));
    let mut serves = Serves::any(provider);
    serves.when = Value::from(when);
    serves
}

pub(crate) fn unsupported(why: impl AsRef<str>) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_UNSUPPORTED, why.as_ref())
}

pub(crate) fn internal(why: impl AsRef<str>) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_INTERNAL, why.as_ref())
}

pub(crate) fn bad_value(why: impl AsRef<str>) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, why.as_ref())
}

/// A JSON document fetched through the host.
pub(crate) fn get_json(
    tools: &Object<dyn InstallTools>,
    url: &str,
) -> Result<serde_json::Value, ProviderError> {
    let text = tools.fetch_text(url, 4 << 20)?;
    serde_json::from_str(&text).map_err(|e| internal(format!("{url} is not JSON: {e}")))
}

/// Whether `version` is a plain version name a URL can carry.
pub(crate) fn valid_version(version: &str) -> bool {
    !version.is_empty()
        && version.len() <= 64
        && version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b'+'))
}

/// A version the caller named, or an error that says it is not one.
pub(crate) fn pinned(version: &str) -> Result<String, ProviderError> {
    if valid_version(version) {
        Ok(version.to_owned())
    } else {
        Err(bad_value(format!("{version:?} is not a version")))
    }
}

/// The first word of `line` that is `digits.digits.digits` and what may follow.
pub(crate) fn dotted(line: &str) -> Option<String> {
    line.split_whitespace().find_map(|word| {
        let word = word.trim_matches(|c: char| !c.is_ascii_alphanumeric());
        let mut parts = word.split('.');
        let numeric = |p: Option<&str>| {
            p.is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        };
        (numeric(parts.next())
            && numeric(parts.next())
            && parts.next().is_some()
            && valid_version(word))
        .then(|| word.to_string())
    })
}

/// Whether `path` contains `needle`, ignoring case and the direction of slashes.
pub(crate) fn path_has(path: &str, needle: &str) -> bool {
    path.replace('\\', "/")
        .to_ascii_lowercase()
        .contains(&needle.to_ascii_lowercase())
}

/// The publisher's spelling of `value`, from a table of `(std spelling, publisher spelling)`.
pub(crate) fn spell(value: &str, table: &[(&str, &str)]) -> Option<String> {
    table
        .iter()
        .find(|(ours, _)| *ours == value)
        .map(|(_, theirs)| (*theirs).to_owned())
}

/// The suffix of a program file on `os`.
pub(crate) const fn exe_suffix(os: &str) -> &'static str {
    // `str` equality is not const; the two names are compared by length and first byte.
    if os.len() == 7 && os.as_bytes()[0] == b'w' {
        ".exe"
    } else {
        ""
    }
}
