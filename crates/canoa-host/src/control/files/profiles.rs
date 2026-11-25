//! What the app needs of this computer's files to show a profile's layers and to trust a project's profile: an overlay's folder read
//! whole, a folder with its links resolved, a project's profile file, and the record of the projects the person trusts.
//!
//! The app does the reading of what a layer says; the host does the file work, so a page and a desktop show the same layers.

#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files are std's"
)]

use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use coqui_aos::{self as aos, Limit, Settings, StdFs};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use super::{Files, Outcome, plain, refused, text_of};
use crate::control::{Caller, bad, text_param};

/// The project profile file in a working directory.
pub const PROJECT_PROFILE: &str = ".canoa/profile.toml";
/// The most bytes a project profile may hold.
const MAX_PROFILE_BYTES: u64 = 1024 * 1024;
/// What an overlay may hold, as `overlay/check` allows.
const MAX_TREE_FILES: u64 = 4096;
const MAX_TREE_BYTES: u64 = 64 * 1024 * 1024;
/// The most bytes of file contents one `profile/tree` answer carries.
pub const MAX_TREE_ANSWER: u64 = 24 * 1024 * 1024;

impl Files {
    /// The folder beside canoa.cfg, where overlays install.
    fn overlays_folder(&self) -> Option<PathBuf> {
        self.config
            .as_deref()?
            .parent()
            .map(|dir| dir.join("overlays"))
    }

    /// `path` as a folder of the person's own canoa.cfg names it: absolute, or from the folder canoa.cfg is in.
    fn beside_config(&self, path: &str) -> Option<PathBuf> {
        let given = Path::new(path);
        if given.is_absolute() {
            return Some(given.to_path_buf());
        }
        if path.chars().any(char::is_control)
            || path.replace('\\', "/").split('/').any(|part| part == "..")
        {
            return None;
        }
        Some(self.config.as_deref()?.parent()?.join(given))
    }

    /// The folders `[overlays]` names with `dir`, resolved.
    fn configured_dirs(&self) -> Vec<PathBuf> {
        let Ok(table) = self.own_tables() else {
            return Vec::new();
        };
        let Some(overlays) = table.get("overlays").and_then(toml::Value::as_table) else {
            return Vec::new();
        };
        overlays
            .values()
            .filter_map(|entry| entry.get("dir")?.as_str())
            .filter_map(|dir| self.beside_config(dir))
            .filter_map(|dir| fs::canonicalize(dir).ok().map(plain))
            .collect()
    }

    /// Whether `real` (resolved) is a folder the profile reads of an overlay: under the overlays folder, or one `[overlays]` names.
    fn overlay_folder(&self, real: &Path) -> bool {
        if let Some(root) = self
            .overlays_folder()
            .and_then(|root| fs::canonicalize(root).ok())
            .map(plain)
            && real.starts_with(&root)
        {
            return true;
        }
        self.configured_dirs().iter().any(|dir| dir == real)
    }

    /// `profile/canonical` `{"path"}` -> `{"directory": string|null}`: the folder with its links resolved, when it is one this key may
    /// name: an overlay's (under the overlays folder, or one `[overlays]` names with `dir`, relative paths from canoa.cfg's folder)
    /// or a place `fs/list` shows it.
    pub(crate) fn profile_canonical(&self, caller: &Caller, params: &Value) -> Outcome {
        let asked = text_param(params, "path")?.ok_or_else(|| bad("path is needed"))?;
        let found = self.overlay_or_place(caller, asked);
        Ok(json!({"directory": found.map(|real| text_of(&real))}))
    }

    fn overlay_or_place(&self, caller: &Caller, asked: &str) -> Option<PathBuf> {
        if let Some(path) = self.beside_config(asked)
            && let Ok(real) = fs::canonicalize(&path).map(plain)
            && real.is_dir()
            && self.overlay_folder(&real)
        {
            return Some(real);
        }
        let real = self.place(caller, asked).ok()?;
        real.is_dir().then_some(real)
    }

    /// `profile/tree` `{"directory"}` -> `{"directory", "files": {path: base64}}`: every file of an overlay's folder as the tree digest
    /// counts them (no link followed, `.git` and the like left out), for the app to digest and read. The folder is one of an
    /// overlay: under the overlays folder, or one `[overlays]` names with `dir`.
    pub(crate) fn profile_tree(&self, params: &Value) -> Outcome {
        let asked = text_param(params, "directory")?.ok_or_else(|| bad("directory is needed"))?;
        let real = self
            .beside_config(asked)
            .and_then(|path| fs::canonicalize(path).ok())
            .map(plain)
            .filter(|real| real.is_dir() && self.overlay_folder(real))
            .ok_or_else(|| refused("that is not the folder of an overlay on this host"))?;
        let limits = Settings::default()
            .tree_files(Limit::at(MAX_TREE_FILES))
            .tree_bytes(Limit::at(MAX_TREE_BYTES));
        let (tree, _) = aos::read_tree(&StdFs, &real, &limits)
            .map_err(|error| refused(&format!("its files cannot be read ({})", error.class)))?;
        let mut files = serde_json::Map::new();
        let mut total = 0_u64;
        for path in tree.paths() {
            let bytes = tree.bytes(path).unwrap_or_default();
            total += bytes.len() as u64;
            if total > MAX_TREE_ANSWER {
                return Err(refused(&format!(
                    "the overlay holds more than {MAX_TREE_ANSWER} bytes, more than one answer carries"
                )));
            }
            files.insert(
                path.to_owned(),
                Value::String(data_encoding::BASE64.encode(bytes)),
            );
        }
        Ok(json!({"directory": text_of(&real), "files": files}))
    }

    /// `profile/project` `{"directory"}` -> `{"directory", "profile": base64|null}`: the folder's resolved name, which this key may
    /// name as `fs/list` does, and the bytes of its `.canoa/profile.toml`, null when it has none.
    pub(crate) fn profile_project(&self, caller: &Caller, params: &Value) -> Outcome {
        let asked = text_param(params, "directory")?.ok_or_else(|| bad("directory is needed"))?;
        let real = self.place(caller, asked)?;
        Ok(json!({
            "directory": text_of(&real),
            "profile": project_profile(&real)?.map(|bytes| data_encoding::BASE64.encode(&bytes)),
        }))
    }

    fn trust_path(&self) -> Result<PathBuf, (i32, String)> {
        self.config
            .as_deref()
            .and_then(Path::parent)
            .map(|dir| dir.join("trust.json"))
            .ok_or_else(|| {
                refused("this host was not started with a canoa.cfg to keep trust beside")
            })
    }

    /// `trust/list` -> `{"projects": [{directory, sha256, trusted?}]}`: the directories whose project profiles the person trusts.
    pub(crate) fn trust_list(&self) -> Outcome {
        Ok(json!({"projects": self.trusted()?}))
    }

    fn trusted(&self) -> Result<Vec<Value>, (i32, String)> {
        let path = self.trust_path()?;
        let Ok(bytes) = fs::read(&path) else {
            return Ok(Vec::new());
        };
        // A record that cannot be read trusts nothing.
        let Ok(Value::Object(record)) = serde_json::from_slice::<Value>(&bytes) else {
            return Ok(Vec::new());
        };
        Ok(record
            .get("projects")
            .and_then(Value::as_array)
            .map(|projects| {
                projects
                    .iter()
                    .filter(|project| {
                        project.get("directory").is_some_and(Value::is_string)
                            && project.get("sha256").is_some_and(Value::is_string)
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default())
    }

    fn save_trusted(&self, projects: &[Value]) -> Result<(), (i32, String)> {
        let path = self.trust_path()?;
        let text = serde_json::to_vec_pretty(&json!({"version": 1, "projects": projects}))
            .map_err(|error| refused(&error.to_string()))?;
        crate::replace::write_file(&path, &text, crate::replace::SHARED)
            .map_err(|error| refused(&format!("trust.json: {error}")))
    }

    /// `trust/set` `{"directory", "sha256"}` -> the list: trusts the folder's project profile as the person was shown it. The
    /// profile is read now, and one that is not the one the digest names is refused, so what is trusted is what was shown.
    pub(crate) fn trust_set(&self, caller: &Caller, params: &Value) -> Outcome {
        let asked = text_param(params, "directory")?.ok_or_else(|| bad("directory is needed"))?;
        let shown = text_param(params, "sha256")?.ok_or_else(|| bad("sha256 is needed"))?;
        let real = self.place(caller, asked)?;
        let _one = self
            .writing
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let bytes = project_profile(&real)?
            .ok_or_else(|| refused("that folder has no project profile to trust"))?;
        let digest = data_encoding::HEXLOWER.encode(&Sha256::digest(&bytes));
        if !digest.eq_ignore_ascii_case(shown) {
            return Err(refused(
                "the project profile is not the one that was shown: it changed, so look at it again",
            ));
        }
        let name = text_of(&real);
        let mut projects = self.trusted()?;
        projects.retain(|project| !same_place(project["directory"].as_str().unwrap_or(""), &name));
        projects.push(json!({
            "directory": name,
            "sha256": digest,
            "trusted": crate::control::overlays::iso_now(),
        }));
        self.save_trusted(&projects)?;
        Ok(json!({"projects": projects}))
    }

    /// `trust/revoke` `{"directory"}` -> the list: stops trusting the folder, which need no longer be there.
    pub(crate) fn trust_revoke(&self, caller: &Caller, params: &Value) -> Outcome {
        let asked = text_param(params, "directory")?.ok_or_else(|| bad("directory is needed"))?;
        let resolved = self.place(caller, asked).ok().map(|real| text_of(&real));
        let _one = self
            .writing
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut projects = self.trusted()?;
        projects.retain(|project| {
            let held = project["directory"].as_str().unwrap_or("");
            !same_place(held, asked)
                && !resolved
                    .as_deref()
                    .is_some_and(|real| same_place(held, real))
        });
        self.save_trusted(&projects)?;
        Ok(json!({"projects": projects}))
    }
}

/// The bytes of `folder`'s project profile; none when it has none, an error for one that is not a plain file or is too large.
fn project_profile(folder: &Path) -> Result<Option<Vec<u8>>, (i32, String)> {
    let path = folder.join(PROJECT_PROFILE);
    let Ok(meta) = fs::symlink_metadata(&path) else {
        return Ok(None);
    };
    if !meta.is_file() {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    fs::File::open(&path)
        .and_then(|file| file.take(MAX_PROFILE_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|error| refused(&format!("the project profile cannot be read: {error}")))?;
    if bytes.len() as u64 > MAX_PROFILE_BYTES {
        return Err(refused(&format!(
            "the project profile holds more than {MAX_PROFILE_BYTES} bytes"
        )));
    }
    Ok(Some(bytes))
}

/// Whether two folder names are one place, as the platform compares them.
fn same_place(a: &str, b: &str) -> bool {
    if cfg!(any(windows, target_os = "macos")) {
        a.to_lowercase() == b.to_lowercase()
    } else {
        a == b
    }
}
