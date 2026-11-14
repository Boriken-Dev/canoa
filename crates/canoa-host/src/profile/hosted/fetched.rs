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
pub(super) const SKILLS_DIR: &str = ".skills";

/// What one skill came to: delivered with its files, or reported.
#[derive(Debug, Default)]
pub(super) struct Fetched {
    /// The files of each delivered skill, by the name its overlay gives it, then by path below the skill's directory.
    pub(super) files: BTreeMap<String, BTreeMap<String, Vec<u8>>>,
    /// What to report: the skill's name, the state and why.
    pub(super) notes: Vec<(String, State, String)>,
}

/// What one skill's directory may hold: 1024 files, 16 MiB.
fn limits() -> Settings {
    Settings::default()
        .tree_files(Limit::at(1024))
        .tree_bytes(Limit::at(16 * 1024 * 1024))
}

/// A skill name, which is also one component of a path the host reads: letters, digits, `_`, `.` and `-`, starting
/// with a letter or digit, and no name Windows keeps for a device.
pub(crate) fn confined(name: &str) -> bool {
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

/// The most bytes a record may hold; the app's holds a few hundred.
const MAX_RECORD: u64 = 64 * 1024;

/// Reads the skills `entries` (those of overlay `identity` that are not bundled) from `store`, the directory of the
/// installed overlays. `pins` are the digests canoa.cfg's `[skill_pins]` holds, by lower-case skill name.
pub(super) fn read(
    store: &Path,
    identity: &str,
    entries: &[SkillEntry],
    pins: &BTreeMap<String, String>,
) -> Fetched {
    let mut out = Fetched::default();
    for entry in entries {
        let name = entry.name.as_str();
        let pinned = pins.contains_key(&name.to_ascii_lowercase());
        match one(store, &Whose::Overlay(identity, entry), pins) {
            Ok((files, keys)) => {
                if entry.origin == SkillOrigin::Source && entry.pin.is_none() && !pinned {
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

/// Whom a record is read for: an overlay that names the skill, or the person who chose it themselves.
enum Whose<'a> {
    Overlay(&'a str, &'a SkillEntry),
    Chosen(&'a str),
}

/// What the person's own chosen skills `names` came to, read from `store` with the checks of an overlay's fetched skill: the
/// record says the person chose it, and its files have the record's digest and pass the checks. `pins` are canoa.cfg's
/// `[skill_pins]`. A skill that fails is reported and not delivered.
pub(super) fn read_chosen(
    store: &Path,
    names: &[String],
    pins: &BTreeMap<String, String>,
) -> Fetched {
    let mut out = Fetched::default();
    for name in names {
        match one(store, &Whose::Chosen(name), pins) {
            Ok((files, keys)) => {
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
                out.files.insert(name.clone(), files);
            }
            Err((state, why)) => out.notes.push((name.clone(), state, why)),
        }
    }
    out
}

/// Whether `path` is a link of any kind, which the app never makes in the store.
fn is_link(path: &Path) -> bool {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return false;
    };
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // A reparse point: a symbolic link or a junction.
        if meta.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    meta.file_type().is_symlink()
}

fn one(
    store: &Path,
    whose: &Whose<'_>,
    pins: &BTreeMap<String, String>,
) -> Result<Read, (State, String)> {
    let name = match whose {
        Whose::Overlay(_, entry) => entry.name.as_str(),
        Whose::Chosen(name) => name,
    };
    if !confined(name) {
        return Err((State::Refused, "its name is not a skill name".to_owned()));
    }
    let lower = name.to_ascii_lowercase();
    let dir = store.join(SKILLS_DIR);
    let not_fetched = || match whose {
        Whose::Overlay(_, entry) if entry.origin == SkillOrigin::Source => format!(
            "from {}, not fetched: install or sync the overlay in Canoa",
            entry.source.as_deref().unwrap_or("its source")
        ),
        Whose::Overlay(..) => {
            "a named skill; no catalog resolved it here: install or sync the overlay in Canoa"
                .to_owned()
        }
        Whose::Chosen(_) => "chosen, but not fetched: install it again in Loadout".to_owned(),
    };
    let refused = |why: &str| (State::Refused, format!("not delivered: {why}"));
    if is_link(&dir) {
        return Err(refused("the store of fetched skills is a link"));
    }
    let record_path = dir.join(format!("{lower}.json"));
    let Ok(meta) = std::fs::symlink_metadata(&record_path) else {
        return Err((State::Unsupported, not_fetched()));
    };
    if !meta.is_file() || is_link(&record_path) || meta.len() > MAX_RECORD {
        return Err(refused(
            "its record is not a plain file of a size the app writes",
        ));
    }
    let Ok(bytes) = std::fs::read(&record_path) else {
        return Err((State::Unsupported, not_fetched()));
    };
    let record: Value = serde_json::from_slice(&bytes)
        .map_err(|_| (State::Refused, "its record is not JSON".to_owned()))?;
    if !record.is_object() {
        return Err(refused("its record is not an object"));
    }
    let text = |key: &str| record.get(key).and_then(Value::as_str);
    if !text("name").is_some_and(|recorded| recorded.eq_ignore_ascii_case(name)) {
        return Err(refused("its record is for another skill"));
    }
    let listed = match whose {
        Whose::Overlay(identity, _) => record
            .get("overlays")
            .and_then(Value::as_array)
            .is_some_and(|names| {
                names
                    .iter()
                    .filter_map(Value::as_str)
                    .any(|named| aos::normalize_name(named) == aos::normalize_name(identity))
            }),
        Whose::Chosen(_) => record.get("chosen").and_then(Value::as_bool) == Some(true),
    };
    if !listed {
        return Err((State::Unsupported, not_fetched()));
    }
    let Some(digest) = text("digest").filter(|digest| aos::Digest::parse(digest).is_ok()) else {
        return Err(refused("its record holds no digest"));
    };
    if let Whose::Overlay(_, entry) = whose
        && entry.origin == SkillOrigin::Source
    {
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
    if pins
        .get(&lower)
        .is_some_and(|pin| !same_digest(pin, digest))
    {
        return Err(refused(
            "its record is not at the pin canoa.cfg's skill_pins names",
        ));
    }
    let skill_dir = dir.join(&lower);
    if is_link(&skill_dir) {
        return Err(refused("its directory is a link"));
    }
    let (tree, read) = aos::read_tree(&StdFs, &skill_dir, &limits())
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
