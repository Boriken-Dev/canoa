//! `state/*`: named documents the person's app keeps in the host's home, so what a page would lose with its browser (drafts, the
//! window layout, the permission rules and their log) follows the person to any browser that reaches this host.
//!
//! A name is up to four `/`-separated parts of letters, digits, `.`, `_` and `-`, none starting with a dot; each is a file under
//! `<home>/state/`. Contents move as base64 in chunks, as uploads do, since a page's transport takes no message over a mebibyte.
//! Documents are shared by every key that holds the grant, not owned by one: they are the person's, not a browser's.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files and locks are std's"
)]

use std::fs;
use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use serde_json::{Value, json};

use super::{NOT_FOUND, REFUSED, bad, private_dir, text_param};

type Outcome = Result<Value, (i32, String)>;

/// The most raw bytes one `state/get` answers or one `state/put` takes.
pub(crate) const MAX_CHUNK: usize = 512 * 1024;
/// The most the documents may hold together, in bytes: a paired page cannot fill the disk.
pub(crate) const MAX_TOTAL: u64 = 512 * 1024 * 1024;
/// The most names `state/list` answers.
pub(crate) const MAX_LIST: usize = 2000;
const MAX_PARTS: usize = 4;
const MAX_PART: usize = 100;
/// Where a document being written is kept until it is whole.
const PART: &str = ".part";

/// The documents kept under one folder; none when the host has no home.
#[derive(Debug, Default)]
pub(crate) struct State {
    dir: Option<PathBuf>,
    one: Mutex<()>,
}

impl State {
    pub(crate) fn new(dir: Option<PathBuf>) -> Self {
        Self {
            dir,
            one: Mutex::new(()),
        }
    }

    fn root(&self) -> Result<&Path, (i32, String)> {
        self.dir
            .as_deref()
            .ok_or_else(|| (REFUSED, "this host has no home to keep state in".to_owned()))
    }

    /// The path of the document `name` names.
    fn path(&self, params: &Value, key: &str) -> Result<PathBuf, (i32, String)> {
        let name = text_param(params, key)?.ok_or_else(|| bad(&format!("{key} is needed")))?;
        let parts = parts_of(name)?;
        let mut path = self.root()?.to_path_buf();
        path.extend(parts);
        Ok(path)
    }

    /// `state/get {name, offset?}` -> `{data, size, done}`: up to [`MAX_CHUNK`] bytes of the document from `offset`.
    pub(crate) fn get(&self, params: &Value) -> Outcome {
        let path = self.path(params, "name")?;
        let _one = self.one.lock().unwrap_or_else(PoisonError::into_inner);
        let mut file = fs::File::open(&path)
            .ok()
            .filter(|_| path.is_file())
            .ok_or_else(|| (NOT_FOUND, "no such document".to_owned()))?;
        let size = file.metadata().map_err(refuse)?.len();
        let offset = params.get("offset").and_then(Value::as_u64).unwrap_or(0);
        file.seek(SeekFrom::Start(offset)).map_err(refuse)?;
        let mut bytes = Vec::new();
        file.take(MAX_CHUNK as u64)
            .read_to_end(&mut bytes)
            .map_err(refuse)?;
        let done = offset.saturating_add(bytes.len() as u64) >= size;
        Ok(json!({"data": data_encoding::BASE64.encode(&bytes), "size": size, "done": done}))
    }

    /// `state/put {name, data, offset?, done?}`: a document written whole, or in chunks (`done: false` until the last; a chunk
    /// after the first names the `offset` the document stands at). The old document stays until the new one is whole.
    pub(crate) fn put(&self, params: &Value) -> Outcome {
        let path = self.path(params, "name")?;
        let data = text_param(params, "data")?.ok_or_else(|| bad("data is needed"))?;
        if data.len() > MAX_CHUNK / 3 * 4 + 8 {
            return Err(refused_chunk());
        }
        let bytes = data_encoding::BASE64
            .decode(data.as_bytes())
            .map_err(|_| bad("data is base64"))?;
        if bytes.len() > MAX_CHUNK {
            return Err(refused_chunk());
        }
        let done = params.get("done") != Some(&Value::Bool(false));
        let offset = params.get("offset").and_then(Value::as_u64).unwrap_or(0);
        let _one = self.one.lock().unwrap_or_else(PoisonError::into_inner);
        let part = beside(&path, PART);
        if let Some(parent) = path.parent() {
            private_dir(parent).map_err(refuse)?;
        }
        let held = held_by(self.root()?);
        if held.saturating_add(bytes.len() as u64) > MAX_TOTAL {
            return Err((
                REFUSED,
                format!("the host keeps at most {MAX_TOTAL} bytes of state: remove some"),
            ));
        }
        let mut file = if offset == 0 {
            super::private_file(&part).map_err(refuse)?
        } else {
            let so_far = fs::metadata(&part).map(|meta| meta.len()).map_err(|_| {
                (
                    REFUSED,
                    "that document is not being written: start it again".to_owned(),
                )
            })?;
            if so_far != offset {
                return Err((
                    REFUSED,
                    format!("offset is {so_far}, where the document stands"),
                ));
            }
            fs::OpenOptions::new()
                .append(true)
                .open(&part)
                .map_err(refuse)?
        };
        file.write_all(&bytes).map_err(refuse)?;
        if !done {
            return Ok(json!({}));
        }
        file.sync_all().map_err(refuse)?;
        drop(file);
        // The pieces arrive over several requests, so the file beside is kept between them and is moved whole at the last: it
        // cannot be one `replace::write_file`.
        fs::rename(&part, &path).map_err(refuse)?;
        Ok(json!({}))
    }

    /// `state/append {name, text}`: a line added to a document that only grows (the permission rules' log).
    pub(crate) fn append(&self, params: &Value) -> Outcome {
        let path = self.path(params, "name")?;
        let text = text_param(params, "text")?.ok_or_else(|| bad("text is needed"))?;
        if text.len() > MAX_CHUNK {
            return Err(refused_chunk());
        }
        let _one = self.one.lock().unwrap_or_else(PoisonError::into_inner);
        if held_by(self.root()?).saturating_add(text.len() as u64) > MAX_TOTAL {
            return Err((
                REFUSED,
                format!("the host keeps at most {MAX_TOTAL} bytes of state: remove some"),
            ));
        }
        if let Some(parent) = path.parent() {
            private_dir(parent).map_err(refuse)?;
        }
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(refuse)?;
        file.write_all(text.as_bytes()).map_err(refuse)?;
        file.sync_all().map_err(refuse)?;
        Ok(json!({}))
    }

    /// `state/list {prefix?}` -> `{names}`: the documents under `prefix` (a name of folders), their names whole, sorted.
    pub(crate) fn list(&self, params: &Value) -> Outcome {
        let root = self.root()?;
        let base = if text_param(params, "prefix")?.is_some_and(|prefix| !prefix.is_empty()) {
            self.path(params, "prefix")?
        } else {
            root.to_path_buf()
        };
        let _one = self.one.lock().unwrap_or_else(PoisonError::into_inner);
        let mut names = Vec::new();
        walk(root, &base, &mut names);
        names.sort();
        names.truncate(MAX_LIST);
        Ok(json!({"names": names}))
    }

    /// `state/remove {name}`: a document, or a folder of them. Nothing there is not a failure.
    pub(crate) fn remove(&self, params: &Value) -> Outcome {
        let path = self.path(params, "name")?;
        let _one = self.one.lock().unwrap_or_else(PoisonError::into_inner);
        let outcome = if path.is_dir() {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        };
        match outcome {
            Ok(()) => Ok(json!({})),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
            Err(error) => Err(refuse(error)),
        }
    }
}

fn refused_chunk() -> (i32, String) {
    (REFUSED, format!("a chunk is at most {MAX_CHUNK} bytes"))
}

fn refuse(error: std::io::Error) -> (i32, String) {
    (
        REFUSED,
        format!("the host's state could not be kept: {error}"),
    )
}

/// `name` as the parts of a path, each a plain name.
fn parts_of(name: &str) -> Result<Vec<&str>, (i32, String)> {
    let parts: Vec<&str> = name.split('/').collect();
    let plain = |part: &&str| {
        !part.is_empty()
            && part.len() <= MAX_PART
            && !part.starts_with('.')
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    };
    if parts.len() > MAX_PARTS || !parts.iter().all(plain) {
        return Err(bad(
            "a state name is up to four parts of letters, digits, `.`, `_` and `-`, none starting with a dot, joined by `/`",
        ));
    }
    Ok(parts)
}

/// `path` with `.suffix` after its file name, in the same folder.
fn beside(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

/// The bytes the documents (and any half-written one) hold.
fn held_by(dir: &Path) -> u64 {
    let Ok(read) = fs::read_dir(dir) else {
        return 0;
    };
    read.flatten()
        .map(|entry| match entry.metadata() {
            Ok(meta) if meta.is_dir() => held_by(&entry.path()),
            Ok(meta) => meta.len(),
            Err(_) => 0,
        })
        .sum()
}

/// The names of the files under `dir`, relative to `root`, skipping a half-written one.
fn walk(root: &Path, dir: &Path, names: &mut Vec<String>) {
    if dir.is_file() {
        if let Some(name) = relative(root, dir) {
            names.push(name);
        }
        return;
    }
    let Ok(read) = fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            walk(root, &path, names);
        } else if kind.is_file()
            && !path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with(PART))
            && let Some(name) = relative(root, &path)
        {
            names.push(name);
        }
    }
}

fn relative(root: &Path, path: &Path) -> Option<String> {
    let rest = path.strip_prefix(root).ok()?;
    let parts: Vec<String> = rest
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect();
    Some(parts.join("/"))
}
