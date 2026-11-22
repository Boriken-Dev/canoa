//! `catalog/fetch`, `catalog/read`, and `catalog/remove` over fetched catalog repositories.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files and git invocations are std's"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::sync::PoisonError;

use coqui_aos::valid_relative_path;
use serde_json::{Value, json};

use crate::profile::full_commit_id;

use super::super::{bad, text_param};
use super::{
    MAX_READ_BYTES, Outcome, Overlays, Trust, checked_name, git_source, is_local, refused, unique,
};

/// The files one `catalog/read` may ask for.
const MAX_CATALOG_PATHS: usize = 4096;
/// The most characters of paths one `ls-tree` is given, since Windows ends a command line at 32767.
const PATH_BUDGET: usize = 8000;

/// Whether the request is for an extensions catalog (`"extensions": true`) and not an overlay catalog.
fn flag(params: &Value, key: &str) -> bool {
    params.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn gate_kind(extensions: bool) -> &'static str {
    if extensions {
        "extension-catalog"
    } else {
        "catalog"
    }
}

impl Overlays {
    /// `catalog/fetch` `{"label", "source"}` -> `{"commit"}`: the catalog repository at its ref, in the cache for `label`.
    pub(crate) fn catalog_fetch(&self, params: &Value, local: &dyn Fn(&str) -> bool) -> Outcome {
        let label =
            checked_name(text_param(params, "label")?.ok_or_else(|| bad("label is needed"))?)?;
        let (repo, reference, _path, _spec) = git_source(params)?;
        if is_local(&repo) && !local(&repo) {
            return Err(refused(
                "a source on this computer must be in a folder this key may browse",
            ));
        }
        let extensions = flag(params, "extensions");
        let gate = self.gate(&format!("{}:{label}", gate_kind(extensions)));
        let _one = gate.lock().unwrap_or_else(PoisonError::into_inner);
        let cache = self.catalog_cache(label, extensions)?;
        self.init_cache(&cache)?;
        let commit = self.fetch(Trust::Own, &cache, &repo, &reference)?;
        Ok(json!({"commit": commit}))
    }

    /// `catalog/read` `{"label", "commit", "paths", "maxBytes", "list"?}` -> `{"files": {path: {"size", "data"?}|null},
    /// "directories"?}`: regular files of a fetched catalog at `commit`, looked up literally; `data` (base64) only for one of
    /// at most `maxBytes`. `list` is a directory whose subdirectories' names come back as `directories`, null for no directory.
    pub(crate) fn catalog_read(&self, params: &Value) -> Outcome {
        let label =
            checked_name(text_param(params, "label")?.ok_or_else(|| bad("label is needed"))?)?;
        let commit = text_param(params, "commit")?.ok_or_else(|| bad("commit is needed"))?;
        if !full_commit_id(commit) {
            return Err(bad("commit is a whole commit id"));
        }
        let extensions = flag(params, "extensions");
        let paths: Vec<&str> = params
            .get("paths")
            .and_then(Value::as_array)
            .ok_or_else(|| bad("paths is a list"))?
            .iter()
            .map(|path| path.as_str().ok_or_else(|| bad("paths is a list of text")))
            .collect::<Result<_, _>>()?;
        if paths.len() > MAX_CATALOG_PATHS {
            return Err(bad("too many paths"));
        }
        let max = params
            .get("maxBytes")
            .and_then(Value::as_u64)
            .unwrap_or(MAX_READ_BYTES)
            .min(MAX_READ_BYTES);
        let cache = self.catalog_cache(label, extensions)?;
        let mut found: BTreeMap<String, (String, u64)> = BTreeMap::new();
        let valid: Vec<&str> = paths
            .iter()
            .copied()
            .filter(|path| valid_relative_path(path))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let mut at = 0;
        while at < valid.len() {
            let mut length = 0;
            let mut chunk: Vec<&str> = Vec::new();
            while at < valid.len() && (chunk.is_empty() || length + valid[at].len() < PATH_BUDGET) {
                length += valid[at].len() + 1;
                chunk.push(valid[at]);
                at += 1;
            }
            let mut args: Vec<OsString> = vec![
                format!("--git-dir={}", cache.display()).into(),
                "ls-tree".into(),
                "-l".into(),
                "-z".into(),
                "--full-tree".into(),
                commit.into(),
                "--".into(),
            ];
            args.extend(chunk.iter().map(OsString::from));
            let out = self.run(&args, None, None, 4 * 1024 * 1024)?;
            let asked: BTreeSet<&str> = chunk.iter().copied().collect();
            for record in String::from_utf8_lossy(&out).split('\0') {
                let Some((head, path)) = record.split_once('\t') else {
                    continue;
                };
                if !asked.contains(path) {
                    continue;
                }
                let fields: Vec<&str> = head.split_whitespace().collect();
                if let [mode, "blob", id, size] = fields[..]
                    && matches!(mode, "100644" | "100755")
                    && let Ok(size) = size.parse::<u64>()
                {
                    found.insert(path.to_owned(), (id.to_owned(), size));
                }
            }
        }
        let wanted: Vec<String> = unique(
            found
                .values()
                .filter(|(_, size)| *size <= max)
                .map(|(id, _)| id),
        );
        let total: u64 = found
            .values()
            .filter(|(_, size)| *size <= max)
            .map(|(_, size)| size)
            .sum();
        if total > MAX_READ_BYTES.saturating_mul(2) {
            return Err(refused("those files hold more than one answer carries"));
        }
        let blobs = self.blobs(&cache, &wanted)?;
        let mut files = serde_json::Map::new();
        for path in &paths {
            let entry = match found.get(*path) {
                None => Value::Null,
                Some((_, size)) if *size > max => json!({"size": size}),
                Some((id, size)) => {
                    let bytes = blobs
                        .get(id)
                        .ok_or_else(|| refused("git did not give a file it was asked for"))?;
                    json!({"size": size, "data": data_encoding::BASE64.encode(bytes)})
                }
            };
            files.insert((*path).to_owned(), entry);
        }
        let mut answer = json!({"files": files});
        if let Some(list) = text_param(params, "list")? {
            answer["directories"] = self.catalog_directories(&cache, commit, list)?;
        }
        Ok(answer)
    }

    /// The names of the subdirectories of the directory `list` at `commit` (the root for none), or null when there is no
    /// directory there. A link or a submodule is none.
    fn catalog_directories(
        &self,
        cache: &Path,
        commit: &str,
        list: &str,
    ) -> Result<Value, (i32, String)> {
        let folder = list.trim_end_matches('/');
        if !folder.is_empty() && !valid_relative_path(folder) {
            return Ok(Value::Null);
        }
        let prefix = if folder.is_empty() {
            String::new()
        } else {
            format!("{folder}/")
        };
        let mut args: Vec<OsString> = vec![
            format!("--git-dir={}", cache.display()).into(),
            "ls-tree".into(),
            "-z".into(),
            "--full-tree".into(),
            commit.into(),
        ];
        if !folder.is_empty() {
            args.push("--".into());
            args.push(prefix.clone().into());
        }
        let out = self.run(&args, None, None, 4 * 1024 * 1024)?;
        let mut any = false;
        let mut names = Vec::new();
        for record in String::from_utf8_lossy(&out).split('\0') {
            let Some((head, path)) = record.split_once('\t') else {
                continue;
            };
            let Some(name) = path.strip_prefix(prefix.as_str()) else {
                continue;
            };
            any = true;
            if head.split(' ').next() == Some("040000") && !name.contains('/') {
                names.push(Value::String(name.to_owned()));
            }
        }
        Ok(if any {
            Value::Array(names)
        } else {
            Value::Null
        })
    }

    /// `catalog/remove` `{"label"}`.
    pub(crate) fn catalog_remove(&self, params: &Value) -> Outcome {
        let label =
            checked_name(text_param(params, "label")?.ok_or_else(|| bad("label is needed"))?)?;
        let extensions = flag(params, "extensions");
        let cache = self.catalog_cache(label, extensions)?;
        let gate = self.gate(&format!("{}:{label}", gate_kind(extensions)));
        let _one = gate.lock().unwrap_or_else(PoisonError::into_inner);
        if cache.exists() {
            fs::remove_dir_all(cache)
                .map_err(|error| refused(format!("the catalog could not be removed: {error}")))?;
        }
        Ok(json!({"removed": true}))
    }
}
