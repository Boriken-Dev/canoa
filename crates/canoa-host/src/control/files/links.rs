//! What the app needs of this computer's files to judge a remembered permission rule for a path: where a path really goes once its
//! links are followed, and where the repository of a folder starts.
//!
//! Both are behind the grant `fs/list` is behind (`folders`) and keep to the same places: a key holding `files` is told of any
//! place this account can reach, any other key only of the home and the working roots of its entries, and never where a link
//! inside them leads outside.

#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files are std's"
)]

use std::ffi::OsStr;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde_json::{Value, json};

use super::{Files, Outcome, plain, text_of, text_under};
use crate::control::{Caller, Grant, bad};

/// The most paths one `fs/realpath` or `fs/repo_root` answers for.
pub const MAX_PATHS: usize = 64;

/// What a refused place says, whether it is not there or this key may not see it.
const NOT_SEEN: &str = "there is no such place, or this key may not see it";

impl Files {
    /// `fs/realpath`: for each of `paths`, `{path, real}`, where `real` is the path with the links followed in the deepest part of
    /// it that exists and what does not exist yet after it as written, or null (with `why`) when a link cannot be followed, no part
    /// of it is there, it is not absolute, or the place is not one this key may see.
    pub(crate) fn realpath(&self, caller: &Caller, params: &Value) -> Outcome {
        let found: Vec<Value> = paths_of(params)?
            .into_iter()
            .map(|path| match self.followed(caller, path) {
                Ok(real) => json!({"path": path, "real": text_of(&real)}),
                Err(why) => json!({"path": path, "real": null, "why": why}),
            })
            .collect();
        Ok(json!({"paths": found}))
    }

    /// `fs/repo_root`: for each of `paths` (folders, which need not exist), `{path, root}`: the nearest folder at or above it that
    /// holds a `.git`, or null. A linked worktree or a submodule, whose `.git` is a file naming a folder inside its main
    /// repository's `.git`, answers that main repository, so the worktrees of one repository have one root. The root is null too
    /// when it lies outside the places this key may see.
    pub(crate) fn repo_root(&self, caller: &Caller, params: &Value) -> Outcome {
        let found: Vec<Value> = paths_of(params)?
            .into_iter()
            .map(|path| {
                let root = self
                    .seen(caller, path)
                    .ok()
                    .and_then(|folder| repository_root(&folder))
                    .filter(|root| self.within(caller, root));
                json!({"path": path, "root": root.map(|root| text_of(&root))})
            })
            .collect();
        Ok(json!({"folders": found}))
    }

    /// `fs/guards`: `{home, config}`, the user's home folder and the folder holding canoa.cfg (each null when this host has none),
    /// which no remembered rule for a path may reach.
    pub(crate) fn guards(&self) -> Outcome {
        Ok(json!({
            "home": self.home.as_deref().map(text_of),
            "config": self.config.as_deref().and_then(Path::parent).map(text_of),
        }))
    }

    /// `asked` as a plain absolute path this key may name, as written.
    fn seen(&self, caller: &Caller, asked: &str) -> Result<PathBuf, String> {
        let path = Path::new(asked);
        if !path.is_absolute()
            || asked.chars().any(char::is_control)
            || path
                .components()
                .any(|part| matches!(part, Component::CurDir | Component::ParentDir))
            || asked.split(['/', '\\']).any(|part| part == "..")
        {
            return Err(
                "a path must be absolute, with no `.` or `..` and no control character".to_owned(),
            );
        }
        if !caller.may(Grant::Files)
            && !self
                .browse_roots(caller)
                .iter()
                .any(|root| text_under(asked, root))
        {
            return Err(NOT_SEEN.to_owned());
        }
        Ok(path.to_path_buf())
    }

    /// `asked` with its links followed, when this key may see where it leads.
    fn followed(&self, caller: &Caller, asked: &str) -> Result<PathBuf, String> {
        let path = self.seen(caller, asked)?;
        let real = follow(&path).ok_or_else(|| {
            "a link in it could not be followed, or no part of it is there".to_owned()
        })?;
        if self.within(caller, &real) {
            Ok(real)
        } else {
            Err(NOT_SEEN.to_owned())
        }
    }
}

/// The `path` or `paths` a request names, as text.
fn paths_of(params: &Value) -> Result<Vec<&str>, (i32, String)> {
    let mut asked: Vec<&str> = Vec::new();
    match params.get("paths") {
        None | Some(Value::Null) => {}
        Some(Value::Array(list)) => {
            for each in list {
                asked.push(
                    each.as_str()
                        .ok_or_else(|| bad("each of paths is a path as text"))?,
                );
            }
        }
        Some(_) => return Err(bad("paths is a list of paths")),
    }
    match params.get("path") {
        None | Some(Value::Null) => {}
        Some(Value::String(one)) => asked.push(one),
        Some(_) => return Err(bad("path is a path as text")),
    }
    if asked.is_empty() {
        return Err(bad("path or paths is needed"));
    }
    if asked.len() > MAX_PATHS {
        return Err(bad(&format!("at most {MAX_PATHS} paths in one request")));
    }
    Ok(asked)
}

/// `path` with the links resolved in the deepest part of it that exists and what is not there yet after it as written; none when
/// a link cannot be followed or nothing of it can be found.
fn follow(path: &Path) -> Option<PathBuf> {
    let mut rest: Vec<&OsStr> = Vec::new();
    let mut current = path;
    loop {
        match fs::symlink_metadata(current) {
            Ok(_) => {
                let mut real = plain(fs::canonicalize(current).ok()?);
                for part in rest.iter().rev() {
                    real.push(part);
                }
                return Some(real);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                rest.push(current.file_name()?);
                current = current.parent()?;
            }
            Err(_) => return None,
        }
    }
}

/// The folder at or above `folder` that holds `.git`, with a linked worktree's or a submodule's answered as its main repository.
fn repository_root(folder: &Path) -> Option<PathBuf> {
    for dir in folder.ancestors() {
        match fs::symlink_metadata(dir.join(".git")) {
            Ok(meta) if meta.is_file() => {
                return Some(main_repository(dir).unwrap_or_else(|| dir.to_path_buf()));
            }
            Ok(_) => return Some(dir.to_path_buf()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return None,
        }
    }
    None
}

/// The main repository a `.git` file in `folder` points into (`gitdir: <main>/.git/worktrees/<name>`), or none when it names none.
fn main_repository(folder: &Path) -> Option<PathBuf> {
    let body = fs::read_to_string(folder.join(".git")).ok()?;
    let line = body.lines().next()?.trim();
    let target = line.strip_prefix("gitdir:")?.trim().replace('\\', "/");
    let target = if target.starts_with('/') || has_drive(&target) {
        target
    } else {
        format!("{}/{target}", text_of(folder).replace('\\', "/"))
    };
    let at = target.find("/.git/").filter(|at| *at > 0)?;
    let main = &target[..at];
    Some(PathBuf::from(if cfg!(windows) {
        main.replace('/', "\\")
    } else {
        main.to_owned()
    }))
}

fn has_drive(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/'
}
