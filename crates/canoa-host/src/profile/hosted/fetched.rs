//! The skills an overlay names from elsewhere, as the app fetched them: read from `overlays/.skills/` and checked
//! again here, since the host fetches nothing and runs nothing of them.
//!
//! The app writes `.skills/<lowercase name>/` (the files the skill's tree digest covers) and
//! `.skills/<lowercase name>.json` (what it accepted: the source, the pin or digest, and the overlays naming it).
//! A copy is used for an overlay only when that record says so, the files have the digest the record names, and
//! they pass the checks of a fetched skill; otherwise the skill is reported as not delivered.

use alloc::borrow::ToOwned;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use std::path::Path;

use coqui_aos::{self as aos, Limit, Settings, SkillEntry, SkillOrigin, StdFs};
use serde_json::Value;

use super::State;

/// The directory under an overlay store the fetched skills are kept in.
pub(crate) const SKILLS_DIR: &str = ".skills";

/// What one skill came to: delivered with its files, or reported.
#[derive(Debug, Default)]
pub(crate) struct Fetched {
    /// The files of each delivered skill, by the name its overlay gives it, then by path below the skill's directory.
    pub(crate) files: BTreeMap<String, BTreeMap<String, Vec<u8>>>,
    /// What to report: the skill's name, the state and why.
    pub(crate) notes: Vec<(String, State, String)>,
}

/// What one skill's directory may hold: 1024 files, 16 MiB.
fn limits() -> Settings {
    Settings::default()
        .tree_files(Limit::at(1024))
        .tree_bytes(Limit::at(16 * 1024 * 1024))
}

/// A skill name, which is also one component of a path the host reads: letters, digits, `_`, `.` and `-`, starting
/// with a letter or digit, and no name Windows keeps for a device.
pub(super) fn confined(name: &str) -> bool {
    let shaped = name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'));
    shaped && aos::valid_relative_path(name)
}

fn same_digest(a: &str, b: &str) -> bool {
    matches!((aos::Digest::parse(a), aos::Digest::parse(b)), (Ok(a), Ok(b)) if a == b)
}

/// Reads the skills `entries` (those of overlay `identity` that are not bundled) from `store`, the directory of the
/// installed overlays.
pub(crate) fn read(store: &Path, identity: &str, entries: &[SkillEntry]) -> Fetched {
    let mut out = Fetched::default();
    for entry in entries {
        let name = entry.name.as_str();
        match one(store, identity, entry) {
            Ok((files, keys)) => {
                if entry.origin == SkillOrigin::Source && entry.pin.is_none() {
                    out.notes.push((
                        format!("{name}.unpinned"),
                        State::Applied,
                        "its source names no pin: what was fetched is what the app recorded, not what a pin vouches for"
                            .to_owned(),
                    ));
                }
                if !keys.is_empty() {
                    out.notes.push((
                        format!("{name}.skill-keys"),
                        State::Applied,
                        format!(
                            "its front matter has keys beyond the inert ones ({}), which a harness may take as a grant",
                            keys.join(", ")
                        ),
                    ));
                }
                out.files.insert(name.to_owned(), files);
            }
            Err((state, why)) => out.notes.push((name.to_owned(), state, why)),
        }
    }
    out
}

type Read = (BTreeMap<String, Vec<u8>>, Vec<String>);

fn one(store: &Path, identity: &str, entry: &SkillEntry) -> Result<Read, (State, String)> {
    let name = entry.name.as_str();
    if !confined(name) {
        return Err((State::Refused, "its name is not a skill name".to_owned()));
    }
    let lower = name.to_ascii_lowercase();
    let dir = store.join(SKILLS_DIR);
    let not_fetched = || match entry.origin {
        SkillOrigin::Source => format!(
            "from {}, not fetched: install or sync the overlay in Canoa",
            entry.source.as_deref().unwrap_or("its source")
        ),
        _ => "a named skill; no catalog resolved it here: install or sync the overlay in Canoa"
            .to_owned(),
    };
    let Ok(bytes) = std::fs::read(dir.join(format!("{lower}.json"))) else {
        return Err((State::Unsupported, not_fetched()));
    };
    let record: Value = serde_json::from_slice(&bytes)
        .map_err(|_| (State::Refused, "its record is not JSON".to_owned()))?;
    let text = |key: &str| record.get(key).and_then(Value::as_str);
    let refused = |why: &str| (State::Refused, format!("not delivered: {why}"));
    if !text("name").is_some_and(|recorded| recorded.eq_ignore_ascii_case(name)) {
        return Err(refused("its record is for another skill"));
    }
    let listed = record
        .get("overlays")
        .and_then(Value::as_array)
        .is_some_and(|names| {
            names
                .iter()
                .filter_map(Value::as_str)
                .any(|named| aos::normalize_name(named) == aos::normalize_name(identity))
        });
    if !listed {
        return Err((State::Unsupported, not_fetched()));
    }
    let Some(digest) = text("digest").filter(|digest| aos::Digest::parse(digest).is_ok()) else {
        return Err(refused("its record holds no digest"));
    };
    if entry.origin == SkillOrigin::Source {
        if text("source") != entry.key.as_deref() {
            return Err(refused(
                "its record is of another source than the overlay names",
            ));
        }
        if entry
            .pin
            .as_deref()
            .is_some_and(|pin| !same_digest(pin, digest))
        {
            return Err(refused("its record is not at the pin the overlay names"));
        }
    }
    let (tree, read) = aos::read_tree(&StdFs, &dir.join(&lower), &limits())
        .map_err(|error| refused(&format!("its files cannot be read ({})", error.class)))?;
    if !same_digest(&read.digest.to_string(), digest) {
        return Err(refused(
            "its files are not those its record's digest names: sync the overlay",
        ));
    }
    let files: Vec<(String, Vec<u8>)> = tree
        .paths()
        .filter_map(|path| Some((path.to_owned(), tree.bytes(path)?.to_vec())))
        .collect();
    let checked = aos::read_fetched_skill(name, &files, Some(digest), &limits())
        .map_err(|error| refused(&format!("it is not a skill of that name ({})", error.class)))?;
    let files = files
        .into_iter()
        .filter(|(path, _)| checked.files.contains(path))
        .collect();
    Ok((files, checked.reported))
}
