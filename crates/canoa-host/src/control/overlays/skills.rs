//! `skill/*`: skills fetched from elsewhere, which the app reviews and the host fetches, keeps and removes.
//!
//! The host holds the git and the files; the app holds the review (what the skill is, its digest and pin, which overlay
//! names it). Each skill is `.skills/<lower-case name>/` (the files its digest covers) and `.skills/<lower-case name>.json`
//! (what the app accepted), the layout the profile reader checks again before it delivers one.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use coqui_aos::{
    Digest, Limit, Settings, read_fetched_skill, valid_overlay_name, valid_relative_path,
};
use serde_json::{Map, Value, json};

use super::{
    Outcome, Overlays, PoisonError, Trust, bad, git_source, is_local, refused, text_param,
    valid_skill_name, write_beside,
};
use crate::profile::full_commit_id;

/// What one skill may hold, as the profile reader reads it: 1024 files, 16 MiB.
const MAX_SKILL_FILES: usize = 1024;
const MAX_SKILL_BYTES: u64 = 16 * 1024 * 1024;
/// The most bytes a record may hold; the app's holds a few hundred.
const MAX_RECORD: u64 = 64 * 1024;
/// The most overlays a record may name.
const MAX_NAMING: usize = 256;

/// A git source a skill request names, and whose settings its fetch uses.
pub(super) struct NamedSource {
    pub(super) repo: String,
    pub(super) reference: String,
    pub(super) path: String,
    /// The request names no ref, so the repository's head is meant.
    pub(super) headless: bool,
    pub(super) trust: Trust,
}

fn limits() -> Settings {
    Settings::default()
        .tree_files(Limit::at(MAX_SKILL_FILES as u64))
        .tree_bytes(Limit::at(MAX_SKILL_BYTES))
}

/// A skill's name, which is also a path component on this host: the shape the app and the profile reader both hold it to.
fn skill_name(params: &Value) -> Result<&str, (i32, String)> {
    let name = text_param(params, "name")?.ok_or_else(|| bad("name is needed"))?;
    if valid_skill_name(name) && valid_relative_path(name) {
        Ok(name)
    } else {
        Err(bad("that is not a skill name"))
    }
}

fn key_of(name: &str) -> String {
    name.to_ascii_lowercase()
}

/// A regular file or folder that is not a link.
fn plain(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| !meta.file_type().is_symlink())
}

fn is_dot_json(name: &str) -> Option<&str> {
    name.strip_suffix(".json")
}

fn piece<'a>(object: &'a Map<String, Value>, key: &str) -> Result<Option<&'a str>, (i32, String)> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) if text.len() <= 4096 && !text.chars().any(char::is_control) => {
            Ok(Some(text))
        }
        Some(_) => Err(bad(&format!("{key} is a line of text"))),
    }
}

/// The record the app keeps of an accepted skill, with only the members it reads.
fn clean_record(object: &Map<String, Value>) -> Result<Map<String, Value>, (i32, String)> {
    let mut out = Map::new();
    let name = piece(object, "name")?.ok_or_else(|| bad("name is needed"))?;
    out.insert("name".into(), name.into());
    let source = piece(object, "source")?.ok_or_else(|| bad("source is needed"))?;
    out.insert("source".into(), source.into());
    let digest = piece(object, "digest")?.ok_or_else(|| bad("digest is needed"))?;
    if Digest::parse(digest).is_err() {
        return Err(bad("digest is sha256: and 64 hexadecimal digits"));
    }
    out.insert("digest".into(), digest.into());
    for key in ["ref", "pin", "catalog"] {
        if let Some(text) = piece(object, key)? {
            out.insert(key.into(), text.into());
        }
    }
    if let Some(pin) = out.get("pin").and_then(Value::as_str)
        && Digest::parse(pin).is_err()
    {
        return Err(bad("pin is sha256: and 64 hexadecimal digits"));
    }
    let overlays = match object.get("overlays") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(listed)) => listed.clone(),
        Some(_) => return Err(bad("overlays is a list")),
    };
    if overlays.len() > MAX_NAMING
        || overlays
            .iter()
            .any(|each| !each.as_str().is_some_and(valid_overlay_name))
    {
        return Err(bad("overlays are overlay names"));
    }
    out.insert("overlays".into(), Value::Array(overlays));
    let chosen = match object.get("chosen") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(chosen)) => *chosen,
        Some(_) => return Err(bad("chosen is true or false")),
    };
    out.insert("chosen".into(), chosen.into());
    Ok(out)
}

impl Overlays {
    fn skills_dir(&self) -> Result<PathBuf, (i32, String)> {
        Ok(self.root()?.join(".skills"))
    }

    /// `skill/list` -> `{"skills": {lower-case name: record}}`: what the app accepted, as its records say and without
    /// files. A record that names another skill than its own file does, or that cannot be read, is no skill.
    pub(crate) fn skill_list(&self) -> Outcome {
        let mut found = Map::new();
        let Ok(reader) = fs::read_dir(self.skills_dir()?) else {
            return Ok(json!({"skills": found}));
        };
        for entry in reader.flatten() {
            let file = entry.file_name();
            let Some(stem) = file.to_str().and_then(is_dot_json) else {
                continue;
            };
            let path = entry.path();
            if !plain(&path)
                || fs::metadata(&path)
                    .map_or(true, |meta| !meta.is_file() || meta.len() > MAX_RECORD)
            {
                continue;
            }
            let Ok(bytes) = fs::read(&path) else {
                continue;
            };
            let Ok(Value::Object(object)) = serde_json::from_slice::<Value>(&bytes) else {
                continue;
            };
            let Ok(record) = clean_record(&object) else {
                continue;
            };
            let named = record.get("name").and_then(Value::as_str).unwrap_or("");
            if !(valid_skill_name(named) && valid_relative_path(named))
                || key_of(named) != stem
                || !(valid_skill_name(stem) && valid_relative_path(stem))
            {
                continue;
            }
            found.insert(stem.to_owned(), Value::Object(record));
        }
        Ok(json!({"skills": found}))
    }

    /// `skill/read` `{"name"}` -> `{"files": {path: base64}}`: the files kept for a skill, none when it is not a plain folder
    /// of files within what a skill may hold, which the app's digest check then refuses.
    pub(crate) fn skill_read(&self, params: &Value) -> Outcome {
        let name = skill_name(params)?;
        let folder = self.skills_dir()?.join(key_of(name));
        let mut files = Map::new();
        if plain(&folder)
            && folder.is_dir()
            && collect(&folder, &folder, &mut files, &mut 0).is_none()
        {
            files.clear();
        }
        Ok(json!({"files": files}))
    }

    /// The source a request names, held to what the host fetches. Whose git settings the fetch may use is the request's `own`:
    /// a skill source is someone else's repository, so it is fetched [`Trust::Isolated`] unless the app says the person typed it
    /// themselves, which a shell would fetch with their credentials and keys.
    pub(super) fn skill_source(
        params: &Value,
        local: &dyn Fn(&str) -> bool,
    ) -> Result<NamedSource, (i32, String)> {
        let trust = match params.get("own") {
            None | Some(Value::Null | Value::Bool(false)) => Trust::Isolated,
            Some(Value::Bool(true)) => Trust::Own,
            Some(_) => return Err(bad("own is true or false")),
        };
        let headless = matches!(params.get("ref"), None | Some(Value::Null));
        let named = if headless {
            let mut with = params.clone();
            with["ref"] = Value::String("HEAD".into());
            git_source(&with)?
        } else {
            git_source(params)?
        };
        let (repo, reference, path, _spec) = named;
        if is_local(&repo) && !local(&repo) {
            return Err(refused(
                "a source on this computer must be in a folder this key may browse",
            ));
        }
        Ok(NamedSource {
            repo,
            reference,
            path,
            headless,
            trust,
        })
    }

    /// Runs `work` with a scratch repository in this host's skills folder, and removes it after.
    pub(super) fn with_scratch<T>(
        &self,
        work: impl FnOnce(&Path) -> Result<T, (i32, String)>,
    ) -> Result<T, (i32, String)> {
        let mut token = [0_u8; 8];
        getrandom::fill(&mut token).map_err(|_| refused("no randomness to name the fetch"))?;
        let cache = self.skills_dir()?.join(format!(
            ".fetch-{}.git",
            data_encoding::HEXLOWER.encode(&token)
        ));
        let answer = work(&cache);
        let _ = fs::remove_dir_all(&cache);
        answer
    }

    /// `skill/fetch` `{"repo", "ref"?, "path"?, "own"?}` -> `{"commit", "files": {path: base64}}`: the files of the skill a git
    /// source names, one commit deep, as the source holds them: nothing under an excluded name, no link, and no more than a skill
    /// may hold. Nothing is installed or kept. `local` says whether a source on this computer may be read.
    pub(crate) fn skill_fetch(&self, params: &Value, local: &dyn Fn(&str) -> bool) -> Outcome {
        let named = Self::skill_source(params, local)?;
        self.with_scratch(|cache| self.fetch_skill_files(cache, &named))
    }

    /// Makes `cache` an empty bare repository and fetches the commit `named` names into it, which it returns.
    pub(super) fn fetch_commit(
        &self,
        cache: &Path,
        named: &NamedSource,
    ) -> Result<String, (i32, String)> {
        fs::create_dir_all(cache)
            .map_err(|error| refused(format!("the fetch could not be made: {error}")))?;
        self.run(
            &[
                "init".into(),
                "--quiet".into(),
                "--bare".into(),
                "--template=".into(),
                cache.into(),
            ],
            None,
            None,
            1024 * 1024,
        )?;
        if named.headless {
            self.fetch_head(named.trust, cache, &named.repo)
        } else {
            self.fetch(named.trust, cache, &named.repo, &named.reference)
        }
    }

    fn fetch_skill_files(&self, cache: &Path, named: &NamedSource) -> Outcome {
        let commit = self.fetch_commit(cache, named)?;
        let (files, _skipped, _digest, bytes) = self.list(cache, &commit, &named.path)?;
        if files.len() > MAX_SKILL_FILES || bytes > MAX_SKILL_BYTES {
            return Err(refused(format!(
                "a skill holds at most {MAX_SKILL_FILES} files and {MAX_SKILL_BYTES} bytes"
            )));
        }
        let ids: Vec<String> = files.values().cloned().collect();
        let blobs = self.blobs(cache, &super::unique(ids.iter()))?;
        let mut out = Map::new();
        for (path, blob) in &files {
            let bytes = blobs
                .get(blob)
                .ok_or_else(|| refused("git did not give a file it was asked for"))?;
            out.insert(
                path.clone(),
                Value::String(data_encoding::BASE64.encode(bytes)),
            );
        }
        Ok(json!({"commit": commit, "files": out}))
    }

    /// The commit the repository's head names, fetched one commit deep.
    fn fetch_head(&self, trust: Trust, cache: &Path, repo: &str) -> Result<String, (i32, String)> {
        let dir = format!("--git-dir={}", cache.display());
        self.run_as(
            trust,
            &[
                dir.clone().into(),
                "fetch".into(),
                "--quiet".into(),
                "--no-tags".into(),
                "--no-recurse-submodules".into(),
                "--depth=1".into(),
                "--".into(),
                repo.into(),
                "HEAD".into(),
            ],
            None,
            Some(cache),
            1024 * 1024,
        )?;
        let found = self.run(
            &[
                dir.into(),
                "rev-parse".into(),
                "--verify".into(),
                "--end-of-options".into(),
                "FETCH_HEAD^{commit}".into(),
            ],
            None,
            None,
            4096,
        )?;
        let commit = String::from_utf8_lossy(&found).trim().to_owned();
        if full_commit_id(&commit) {
            Ok(commit)
        } else {
            Err(refused("the source has no head to fetch"))
        }
    }

    /// `skill/save` `{record, "files": {path: base64}}` -> `{"saved": true}`: keeps what the app accepted. The files are
    /// checked here as the profile reader checks them: paths inside the skill's folder, within what a skill may hold, and
    /// the digest the record names. The skill's folder is replaced whole.
    pub(crate) fn skill_save(&self, params: &Value) -> Outcome {
        let object = params
            .as_object()
            .ok_or_else(|| bad("params are an object"))?;
        let record = clean_record(object)?;
        let name = record["name"].as_str().unwrap_or_default().to_owned();
        if !(valid_skill_name(&name) && valid_relative_path(&name)) {
            return Err(bad("that is not a skill name"));
        }
        let digest = record["digest"].as_str().unwrap_or_default().to_owned();
        let listed = object
            .get("files")
            .and_then(Value::as_object)
            .ok_or_else(|| bad("files are an object of base64 text"))?;
        if listed.is_empty() || listed.len() > MAX_SKILL_FILES {
            return Err(refused(format!(
                "a skill holds 1 to {MAX_SKILL_FILES} files"
            )));
        }
        let mut files: Vec<(String, Vec<u8>)> = Vec::new();
        let mut bytes = 0_u64;
        for (path, text) in listed {
            if !valid_relative_path(path) {
                return Err(refused("a skill file has no valid relative path"));
            }
            let data = data_encoding::BASE64
                .decode(
                    text.as_str()
                        .ok_or_else(|| bad("a file is base64 text"))?
                        .as_bytes(),
                )
                .map_err(|_| bad("a file is base64 text"))?;
            bytes += data.len() as u64;
            if bytes > MAX_SKILL_BYTES {
                return Err(refused(format!(
                    "a skill holds at most {MAX_SKILL_BYTES} bytes"
                )));
            }
            files.push((path.clone(), data));
        }
        read_fetched_skill(&name, &files, Some(&digest), &limits()).map_err(|error| {
            refused(format!(
                "those files are not a skill of that name with that digest ({})",
                error.class
            ))
        })?;
        let key = key_of(&name);
        let base = self.skills_dir()?;
        let gate = self.gate(&format!("skill:{key}"));
        let _one = gate.lock().unwrap_or_else(PoisonError::into_inner);
        let staging = base.join(format!(".staging-{key}"));
        let _ = fs::remove_dir_all(&staging);
        fs::create_dir_all(&staging)
            .map_err(|error| refused(format!("the skill could not be written: {error}")))?;
        if let Err(error) = write_skill_files(&staging, &files) {
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }
        let target = base.join(&key);
        let old = base.join(format!(".old-{key}"));
        let _ = fs::remove_dir_all(&old);
        if fs::symlink_metadata(&target).is_ok() {
            fs::rename(&target, &old)
                .map_err(|error| refused(format!("the skill could not be replaced: {error}")))?;
        }
        fs::rename(&staging, &target)
            .map_err(|error| refused(format!("the skill could not be placed: {error}")))?;
        let _ = fs::remove_dir_all(&old);
        let mut text = serde_json::to_string_pretty(&Value::Object(record))
            .map_err(|error| refused(error.to_string()))?;
        text.push('\n');
        write_beside(&base.join(format!("{key}.json")), text.as_bytes()).map_err(|error| {
            refused(format!("the skill's record could not be written: {error}"))
        })?;
        Ok(json!({"saved": true}))
    }

    /// `skill/remove` `{"name"}`: the skill's record and files.
    pub(crate) fn skill_remove(&self, params: &Value) -> Outcome {
        let name = skill_name(params)?;
        let key = key_of(name);
        let base = self.skills_dir()?;
        let gate = self.gate(&format!("skill:{key}"));
        let _one = gate.lock().unwrap_or_else(PoisonError::into_inner);
        let record = base.join(format!("{key}.json"));
        if fs::symlink_metadata(&record).is_ok() {
            fs::remove_file(&record)
                .map_err(|error| refused(format!("the skill could not be removed: {error}")))?;
        }
        let folder = base.join(&key);
        if let Ok(meta) = fs::symlink_metadata(&folder) {
            // A link is removed as a link, never followed.
            let done = if meta.file_type().is_symlink() || meta.is_file() {
                fs::remove_file(&folder).or_else(|_| fs::remove_dir(&folder))
            } else {
                fs::remove_dir_all(&folder)
            };
            done.map_err(|error| refused(format!("the skill could not be removed: {error}")))?;
        }
        Ok(json!({"removed": true}))
    }
}

/// Writes `files` below `staging`, each exclusively: a name the file system takes for one already written fails the install.
fn write_skill_files(staging: &Path, files: &[(String, Vec<u8>)]) -> Result<(), (i32, String)> {
    for (path, bytes) in files {
        let file = staging.join(path);
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| refused(format!("the skill could not be written: {error}")))?;
        }
        let mut out = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&file)
            .map_err(|error| refused(format!("the skill could not be written: {error}")))?;
        out.write_all(bytes)
            .and_then(|()| out.sync_all())
            .map_err(|error| refused(format!("the skill could not be written: {error}")))?;
    }
    Ok(())
}

/// Every file below `folder` by relative path, base64; none (and so an empty answer) when the tree holds a link or passes
/// what a skill may hold.
fn collect(
    root: &Path,
    folder: &Path,
    out: &mut Map<String, Value>,
    bytes: &mut u64,
) -> Option<()> {
    for entry in fs::read_dir(folder).ok()?.flatten() {
        let path = entry.path();
        let meta = fs::symlink_metadata(&path).ok()?;
        if meta.file_type().is_symlink() {
            return None;
        }
        if meta.is_dir() {
            collect(root, &path, out, bytes)?;
        } else if meta.is_file() {
            *bytes += meta.len();
            if out.len() >= MAX_SKILL_FILES || *bytes > MAX_SKILL_BYTES {
                return None;
            }
            let relative = path.strip_prefix(root).ok()?.to_str()?.replace('\\', "/");
            out.insert(
                relative,
                Value::String(data_encoding::BASE64.encode(&fs::read(&path).ok()?)),
            );
        }
    }
    Some(())
}
