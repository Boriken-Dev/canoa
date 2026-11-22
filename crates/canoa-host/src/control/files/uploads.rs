//! Chunked uploads on the control channel (`upload/put` and `upload/remove`).
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files and locks are std's"
)]

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::PoisonError;

use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use super::super::{Caller, private_file};
use super::{
    Files, MAX_CHUNK, MAX_NAME, Outcome, PARTIAL, REFUSED, bad, plain, private_dir, refused,
    text_of, text_param,
};

impl Files {
    /// `upload/put`: keeps a file a page sends, in a folder of this caller's, and answers where. A file travels in chunks of at most
    /// [`MAX_CHUNK`] bytes, since the transport a page uses carries no larger message: the first call (`name`, `data`) names the file
    /// and, with `done: false`, answers an `id` to continue with (`id`, `offset` the size so far, `data`); the call with `done`
    /// true or absent ends it. A file sent in one chunk needs no `id`. A file is kept in the folder `.partial` until it is done.
    pub(crate) fn upload_put(&self, caller: &Caller, params: &Value) -> Outcome {
        let dir = self.owner_dir(caller)?;
        let _one = self
            .uploading
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let data = text_param(params, "data")?.ok_or_else(|| bad("data is needed"))?;
        // Before decoding, so a page cannot make the host hold more than it would keep.
        if data.len() > MAX_CHUNK / 3 * 4 + 8 {
            return Err(refused(&format!("a chunk is at most {MAX_CHUNK} bytes")));
        }
        let bytes = data_encoding::BASE64
            .decode(data.as_bytes())
            .map_err(|_| bad("data is base64"))?;
        if bytes.len() > MAX_CHUNK {
            return Err(refused(&format!("a chunk is at most {MAX_CHUNK} bytes")));
        }
        let done = params.get("done") != Some(&Value::Bool(false));
        let (count, held) = usage(&dir);
        let added = bytes.len() as u64;
        let (stored, part, so_far) = if let Some(id) = text_param(params, "id")? {
            if !stored_name(id) {
                return Err(bad("id is what upload/put answered"));
            }
            let part = dir.join(PARTIAL).join(id);
            let so_far = fs::metadata(&part)
                .map(|meta| meta.len())
                .map_err(|_| refused("that upload is not under way: start it again"))?;
            if params.get("offset").and_then(Value::as_u64) != Some(so_far) {
                return Err(refused(&format!(
                    "offset is {so_far}, where the upload stands"
                )));
            }
            (id.to_owned(), part, so_far)
        } else {
            let name = text_param(params, "name")?.ok_or_else(|| bad("name is needed"))?;
            if count >= self.limits.files {
                return Err(refused(
                    "this caller has no room for another upload: remove some",
                ));
            }
            private_dir(&dir.join(PARTIAL))
                .map_err(|error| refused(&format!("uploads could not be kept: {error}")))?;
            forget_stale(&dir);
            let mut tag = [0_u8; 6];
            getrandom::fill(&mut tag).map_err(|_| refused("no randomness to name the upload"))?;
            let stored = format!(
                "{}-{}",
                data_encoding::HEXLOWER.encode(&tag),
                file_name_of(name)
            );
            let part = dir.join(PARTIAL).join(&stored);
            (stored, part, 0)
        };
        if so_far + added > self.limits.file_bytes {
            let _ = fs::remove_file(&part);
            return Err(too_big(self.limits.file_bytes));
        }
        if held + added > self.limits.total_bytes {
            return Err(refused(
                "this caller has no room for another upload: remove some",
            ));
        }
        let mut file = if so_far == 0 {
            private_file(&part)
        } else {
            fs::OpenOptions::new().append(true).open(&part)
        }
        .map_err(|error| refused(&format!("the upload could not be kept: {error}")))?;
        file.write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|error| {
                let _ = fs::remove_file(&part);
                refused(&format!("the upload could not be kept: {error}"))
            })?;
        drop(file);
        let path = dir.join(&stored);
        if done {
            // An upload arrives in pieces over several requests: its file is kept apart until the last, then moved whole, so it
            // cannot be one `replace::write_file`.
            fs::rename(&part, &path).map_err(|error| {
                let _ = fs::remove_file(&part);
                refused(&format!("the upload could not be kept: {error}"))
            })?;
        }
        Ok(json!({
            "id": stored,
            "path": text_of(&path),
            "name": stored,
            "size": so_far + added,
            "done": done,
        }))
    }

    /// `upload/remove`: deletes a file this caller uploaded, by its `id` (the `name` it answered), and what is left of one not finished.
    pub(crate) fn upload_remove(&self, caller: &Caller, params: &Value) -> Outcome {
        let dir = self.owner_dir(caller)?;
        let _one = self
            .uploading
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let Ok(base) = fs::canonicalize(&dir).map(plain) else {
            return Ok(json!({"removed": false}));
        };
        // Only the name an upload was given, never a path: nothing here is opened by a caller's text.
        let name = text_param(params, "id")?
            .or(text_param(params, "name")?)
            .ok_or_else(|| bad("id is needed"))?
            .to_owned();
        if !stored_name(&name) {
            return Ok(json!({"removed": false}));
        }
        let mut removed = false;
        for target in [base.join(&name), base.join(PARTIAL).join(&name)] {
            if fs::symlink_metadata(&target).is_ok_and(|meta| meta.is_file()) {
                fs::remove_file(&target).map_err(|error| {
                    refused(&format!("the upload could not be removed: {error}"))
                })?;
                removed = true;
            }
        }
        Ok(json!({"removed": removed}))
    }

    /// The folder this caller's uploads are in: one for each key, named by a digest of its id.
    fn owner_dir(&self, caller: &Caller) -> Result<PathBuf, (i32, String)> {
        let root = self
            .uploads
            .as_deref()
            .ok_or_else(|| (REFUSED, "this host keeps no uploads".to_owned()))?;
        let digest = Sha256::digest(caller.owner().as_bytes());
        Ok(root.join(data_encoding::HEXLOWER.encode(&digest[..12])))
    }
}

/// Whether `name` is one `upload/put` made: 12 hexadecimal digits, a hyphen and a plain name.
fn stored_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    name.len() > 13
        && bytes[..12].iter().all(u8::is_ascii_hexdigit)
        && bytes[12] == b'-'
        && !name.contains(['/', '\\', ':'])
        && !name.contains("..")
        && !name.chars().any(char::is_control)
}

/// Takes away the unfinished uploads nobody has continued for an hour.
fn forget_stale(dir: &Path) {
    let Ok(reader) = fs::read_dir(dir.join(PARTIAL)) else {
        return;
    };
    for entry in reader.flatten() {
        let stale = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|at| at.elapsed().ok())
            .is_some_and(|age| age > std::time::Duration::from_secs(3600));
        if stale {
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn too_big(limit: u64) -> (i32, String) {
    (REFUSED, format!("an upload is at most {limit} bytes"))
}

/// The files and bytes `dir` holds.
fn usage(dir: &Path) -> (usize, u64) {
    let mut count = 0;
    let mut bytes = 0;
    for place in [dir.to_path_buf(), dir.join(PARTIAL)] {
        if let Ok(reader) = fs::read_dir(place) {
            for entry in reader.flatten() {
                if let Ok(meta) = entry.metadata()
                    && meta.is_file()
                {
                    count += 1;
                    bytes += meta.len();
                }
            }
        }
    }
    (count, bytes)
}

/// A name a file can be kept under: the last component, with anything but letters, digits and `.-_ ` made `_`, not led by a dot.
fn file_name_of(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or("");
    let mut kept: String = last
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect();
    while kept.len() > MAX_NAME {
        kept.pop();
    }
    let kept = kept.trim_matches([' ', '.']).to_owned();
    if kept.is_empty() {
        "upload".to_owned()
    } else {
        kept
    }
}
