//! What an agent's crate needs to list the session files its program keeps: reading folders without following a link, naming a
//! file, reading the start of one for a title, and putting a listing in the order the host wants.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "folders and files are std's"
)]

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use super::{SessionFile, SessionsRequest};

/// How much of a session's start is read to find its title.
pub const TITLE_WINDOW: u64 = 256 * 1024;

/// The longest title or folder shown, in characters.
pub const SHOWN: usize = 120;

/// Text a person will be shown that a file chose: control and bidirectional characters out, runs of space as one, cut to [`SHOWN`].
#[must_use]
pub fn tidy(text: &str) -> String {
    let mut out = String::new();
    let mut gap = false;
    for ch in text.chars() {
        let hidden = ch.is_control()
            || matches!(ch, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}');
        if hidden || ch.is_whitespace() {
            gap = !out.is_empty();
            continue;
        }
        if gap {
            out.push(' ');
            gap = false;
        }
        out.push(ch);
        if out.chars().count() >= SHOWN {
            break;
        }
    }
    out
}

/// The folders directly under `path`; a link is not followed.
#[must_use]
pub fn folders(path: &Path) -> Vec<PathBuf> {
    children(path, |kind| kind.is_dir())
        .map(|entry| entry.path())
        .collect()
}

/// The files directly under `path`, as entries; a link is not followed.
#[must_use]
pub fn files(path: &Path) -> Vec<fs::DirEntry> {
    children(path, |kind| kind.is_file()).collect()
}

fn children(path: &Path, wanted: fn(&fs::FileType) -> bool) -> impl Iterator<Item = fs::DirEntry> {
    fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .filter(move |entry| entry.file_type().is_ok_and(|kind| wanted(&kind)))
}

/// A file's name as text.
#[must_use]
pub fn name_of(entry: &fs::DirEntry) -> String {
    entry.file_name().to_string_lossy().into_owned()
}

/// The session `id` kept in `entry`, with its size and when it changed.
#[must_use]
pub fn found(entry: &fs::DirEntry, id: &str) -> Option<SessionFile> {
    let meta = entry.metadata().ok()?;
    let updated = meta
        .modified()
        .ok()
        .and_then(|at| at.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |since| u64::try_from(since.as_millis()).unwrap_or(0));
    Some(SessionFile::new(
        id,
        entry.path().to_string_lossy(),
        meta.len(),
        updated,
    ))
}

/// The start of the file at `path`, up to [`TITLE_WINDOW`] bytes, as text; `None` when it cannot be read.
#[must_use]
pub fn head(path: &Path) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(TITLE_WINDOW).read_to_end(&mut bytes).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// `found` put newest first, and the newest `request.peek` of them given the title and folder `peek` reads from the start of
/// the file (`path` in, `(title, folder)` out).
#[must_use]
pub fn listing(
    mut found: Vec<SessionFile>,
    request: &SessionsRequest,
    peek: impl Fn(&str) -> (Option<String>, Option<String>),
) -> Vec<SessionFile> {
    found.sort_by(|a, b| {
        b.updated_ms
            .cmp(&a.updated_ms)
            .then_with(|| a.id.cmp(&b.id))
    });
    let take = usize::try_from(request.peek).unwrap_or(usize::MAX);
    for each in found.iter_mut().take(take) {
        let (title, cwd) = peek(&each.path);
        each.title = title;
        each.cwd = cwd;
    }
    found
}

/// The lines of `text`'s first [`TITLE_WINDOW`] bytes that are JSON objects, for a peek.
#[must_use]
pub fn window_lines(text: &str) -> Vec<String> {
    let mut end = text
        .len()
        .min(usize::try_from(TITLE_WINDOW).unwrap_or(usize::MAX));
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end]
        .lines()
        .filter(|line| line.trim_start().starts_with('{'))
        .map(ToString::to_string)
        .collect()
}

/// [`window_lines`] of the file at `path`; none when it cannot be read.
#[must_use]
pub fn json_lines(path: &str) -> Vec<String> {
    head(Path::new(path))
        .map(|text| window_lines(&text))
        .unwrap_or_default()
}
