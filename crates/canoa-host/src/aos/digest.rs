//! The tree digest: one SHA-256 over an overlay's files, the same on every platform, and reading a directory into the
//! entries it is computed from without following any link.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt::Write as _;
use std::path::Path;

use sha2::{Digest as _, Sha256};

use super::{AosError, EXCLUDED_NAMES, is_relative_path};

/// What an entry of a tree is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EntryKind {
    /// A regular file.
    File,
    /// A directory, which is never a member.
    Dir,
    /// A symbolic link, a junction or another reparse point.
    Symlink,
    /// Anything else: a device, a pipe, a socket.
    Special,
}

/// One entry of a tree: its names from the root, as stored, its kind, and a file's bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeEntry {
    /// Each name from the root, as its bytes.
    pub components: Vec<Vec<u8>>,
    /// Its kind.
    pub kind: EntryKind,
    /// A file's bytes; empty for the other kinds.
    pub data: Vec<u8>,
}

/// A tree digest and the lines it is the digest of.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct TreeDigest {
    /// `sha256:` and 64 lowercase hexadecimal digits.
    pub digest: String,
    /// Each member's line, `<path>\0sha256:<hex>\n`, sorted by path bytes.
    pub lines: Vec<Vec<u8>>,
}

impl TreeEntry {
    /// Whether the entry is under an excluded name, or a `.pyc` file: not part of an overlay.
    #[must_use]
    pub fn is_excluded(&self) -> bool {
        excluded(&self.components, self.kind)
    }
}

impl TreeDigest {
    /// The members' paths, in the digest's order.
    #[must_use]
    pub fn paths(&self) -> Vec<String> {
        self.lines
            .iter()
            .filter_map(|line| {
                let end = line.iter().position(|byte| *byte == 0)?;
                String::from_utf8(line[..end].to_vec()).ok()
            })
            .collect()
    }
}

/// `sha256:` and the lowercase hexadecimal SHA-256 of `data`.
#[must_use]
pub fn sha256_digest(data: &[u8]) -> String {
    let mut out = String::from("sha256:");
    for byte in Sha256::digest(data) {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Whether two digests are the same, comparing the hexadecimal digits without regard to case; false when either is not
/// `sha256:` and 64 hexadecimal digits.
#[must_use]
pub fn same_digest(a: &str, b: &str) -> bool {
    let form = |text: &str| {
        text.strip_prefix("sha256:")
            .filter(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .map(str::to_ascii_lowercase)
    };
    matches!((form(a), form(b)), (Some(a), Some(b)) if a == b)
}

fn shown(components: &[Vec<u8>]) -> String {
    components
        .iter()
        .map(|name| String::from_utf8_lossy(name).into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

fn excluded(components: &[Vec<u8>], kind: EntryKind) -> bool {
    components.iter().any(|name| {
        EXCLUDED_NAMES
            .iter()
            .any(|excluded| excluded.as_bytes() == name.as_slice())
    }) || kind != EntryKind::Dir
        && components
            .last()
            .is_some_and(|name| name.ends_with(b".pyc"))
}

/// The tree digest of `entries`.
///
/// # Errors
///
/// Outside an excluded entry: `symlink` for a link of any kind, `special-file` for what is neither a file nor a
/// directory, `non-utf8-name`, `invalid-name` for a member whose path is not a relative path, and `case-collision` for
/// two members equal after ASCII lowercasing. The subject is the entry's path.
pub fn digest_entries(entries: &[TreeEntry]) -> Result<TreeDigest, AosError> {
    let mut lines: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    for entry in entries {
        if entry.kind == EntryKind::Dir || excluded(&entry.components, entry.kind) {
            continue;
        }
        match entry.kind {
            EntryKind::Symlink => return Err(AosError::new("symlink", shown(&entry.components))),
            EntryKind::Special => {
                return Err(AosError::new("special-file", shown(&entry.components)));
            }
            _ => {}
        }
        if entry
            .components
            .iter()
            .any(|name| core::str::from_utf8(name).is_err())
        {
            return Err(AosError::new("non-utf8-name", shown(&entry.components)));
        }
        let path = entry.components.join(&b'/');
        if !core::str::from_utf8(&path).is_ok_and(is_relative_path) {
            return Err(AosError::new("invalid-name", shown(&entry.components)));
        }
        let mut line = path.clone();
        line.push(0);
        line.extend_from_slice(sha256_digest(&entry.data).as_bytes());
        line.push(b'\n');
        lines.push((path, line));
    }
    let mut folded = alloc::collections::BTreeMap::new();
    for (path, _) in &lines {
        let other = folded
            .entry(path.to_ascii_lowercase())
            .or_insert_with(|| path.clone());
        if other != path {
            return Err(AosError::new(
                "case-collision",
                String::from_utf8_lossy(path).into_owned(),
            ));
        }
    }
    lines.sort_by(|a, b| a.0.cmp(&b.0));
    let lines: Vec<Vec<u8>> = lines.into_iter().map(|(_, line)| line).collect();
    Ok(TreeDigest {
        digest: sha256_digest(&lines.concat()),
        lines,
    })
}

/// Reads the tree under `root` into entries, never following a link: a link, a junction or another reparse point is an
/// entry of kind [`EntryKind::Symlink`], never entered; an excluded directory is not entered either. Reading stops with
/// `too-large` once the files pass `max_files` or `max_bytes`, before the rest is read.
///
/// # Errors
///
/// `too-large`, or `unreadable` with the path when the directory or a file cannot be read.
pub fn read_tree(
    root: &Path,
    max_files: usize,
    max_bytes: u64,
) -> Result<Vec<TreeEntry>, AosError> {
    let mut out = Vec::new();
    let mut budget = (0usize, 0u64);
    walk(
        root,
        &mut Vec::new(),
        &mut out,
        &mut budget,
        (max_files, max_bytes),
    )?;
    Ok(out)
}

fn name_bytes(name: &std::ffi::OsStr) -> Vec<u8> {
    #[cfg(unix)]
    {
        std::os::unix::ffi::OsStrExt::as_bytes(name).to_vec()
    }
    #[cfg(not(unix))]
    {
        // A name that is not valid Unicode (a lone surrogate on Windows) keeps an invalid byte, so the digest refuses it.
        name.to_str()
            .map_or_else(|| alloc::vec![0xff], |name| name.as_bytes().to_vec())
    }
}

fn is_link(meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        const REPARSE_POINT: u32 = 0x400;
        if std::os::windows::fs::MetadataExt::file_attributes(meta) & REPARSE_POINT != 0 {
            return true;
        }
    }
    meta.file_type().is_symlink()
}

fn walk(
    directory: &Path,
    prefix: &mut Vec<Vec<u8>>,
    out: &mut Vec<TreeEntry>,
    budget: &mut (usize, u64),
    limits: (usize, u64),
) -> Result<(), AosError> {
    let unreadable = |path: &Path| AosError::new("unreadable", path.display().to_string());
    let mut entries: Vec<std::fs::DirEntry> = std::fs::read_dir(directory)
        .map_err(|_| unreadable(directory))?
        .collect::<Result<_, _>>()
        .map_err(|_| unreadable(directory))?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let meta = std::fs::symlink_metadata(&path).map_err(|_| unreadable(&path))?;
        prefix.push(name_bytes(&entry.file_name()));
        let components = prefix.clone();
        let is_excluded = excluded(
            &components,
            if meta.is_dir() {
                EntryKind::Dir
            } else {
                EntryKind::File
            },
        );
        let kind = if is_link(&meta) {
            EntryKind::Symlink
        } else if meta.is_dir() {
            EntryKind::Dir
        } else if meta.is_file() {
            EntryKind::File
        } else {
            EntryKind::Special
        };
        let mut data = Vec::new();
        if kind == EntryKind::File && !is_excluded {
            budget.0 += 1;
            budget.1 += meta.len();
            if budget.0 > limits.0 || budget.1 > limits.1 {
                return Err(AosError::new("too-large", shown(&components)));
            }
            data = std::fs::read(&path).map_err(|_| unreadable(&path))?;
        }
        out.push(TreeEntry {
            components,
            kind,
            data,
        });
        if kind == EntryKind::Dir && !is_excluded {
            walk(&path, prefix, out, budget, limits)?;
        }
        prefix.pop();
    }
    Ok(())
}
