//! What a repository's work tree holds now against a commit, read only: the files that differ with their added and removed
//! lines and a patch each, so a client can show the disk's state beside what an agent reported. Git runs with the same
//! limits as for a worktree, and never with an external diff program or a text conversion; nothing is written.

use alloc::borrow::ToOwned;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use std::io::Read as _;
use std::path::Path;

use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use super::{Refusal, Worktrees, refused, resolve};

/// Files listed for one request; the totals count every one.
const MAX_FILES: usize = 300;
/// The most of one file's patch sent, in bytes.
const MAX_PATCH: usize = 96 * 1024;
/// A file changed in more lines than this is listed with its counts and no patch.
const MAX_PATCH_LINES: u64 = 20_000;
/// The largest untracked file whose lines are shown as added.
const MAX_UNTRACKED: u64 = 256 * 1024;
/// The tree git hashes an empty input to: the base of a repository with no commit yet.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// One file that differs.
struct Changed {
    path: String,
    status: char,
    added: u64,
    removed: u64,
    binary: bool,
    patch: Option<String>,
    cut: bool,
}

impl Worktrees {
    /// `{"path", "base"?}` to `{"exists": false}` or `{"exists": true, "head", "branch", "base", "added", "removed", "files",
    /// "more"}`: the work tree under `path` against `base` (a commit's hex id; HEAD when none), tracked changes and untracked
    /// files alike, limited to the folder `path` names.
    pub(super) fn diff(&self, path: &str, base: Option<&str>) -> Result<Value, Refusal> {
        let Some(path) = self.checked(path)? else {
            return Ok(json!({"exists": false}));
        };
        self.safe_repository(&path)?;
        let head = self
            .read(&path, &["rev-parse", "--verify", "--quiet", "HEAD"])
            .ok()
            .map(|head| head.trim().to_owned());
        let base = match base {
            Some(base) => {
                if !(base.len() == 40 || base.len() == 64)
                    || !base.bytes().all(|b| b.is_ascii_hexdigit())
                {
                    return Err(refused("a base is a commit's hexadecimal id"));
                }
                let found = format!("{base}^{{commit}}");
                self.read(&path, &["rev-parse", "--verify", "--quiet", &found])
                    .map_err(|_| refused("that commit is not in this repository"))?;
                base.to_ascii_lowercase()
            }
            None => head.clone().unwrap_or_else(|| EMPTY_TREE.to_owned()),
        };
        let branch = self
            .read(&path, &["symbolic-ref", "--quiet", "--short", "HEAD"])
            .ok()
            .map(|branch| branch.trim_end_matches(['\n', '\r']).to_owned());

        let mut files = self.tracked(&path, &base)?;
        files.extend(self.untracked(&path)?);
        files.sort_by(|a, b| a.path.cmp(&b.path));
        let (added, removed) = files.iter().fold((0, 0), |(added, removed), file| {
            (added + file.added, removed + file.removed)
        });
        let more = files.len() > MAX_FILES;
        files.truncate(MAX_FILES);
        let listed: Vec<Value> = files
            .iter()
            .map(|file| {
                let mut digest = Sha256::new();
                digest.update(file.path.as_bytes());
                digest.update([0]);
                digest.update(file.patch.as_deref().unwrap_or("").as_bytes());
                digest.update(format!("{}{}{}", file.added, file.removed, file.binary).as_bytes());
                let print: String = digest
                    .finalize()
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect();
                json!({
                    "path": file.path,
                    "status": file.status.to_string(),
                    "added": file.added,
                    "removed": file.removed,
                    "binary": file.binary,
                    "patch": file.patch,
                    "cut": file.cut,
                    "print": print,
                })
            })
            .collect();
        Ok(json!({
            "exists": true,
            "head": head,
            "branch": branch,
            "base": base,
            "added": added,
            "removed": removed,
            "files": listed,
            "more": more,
        }))
    }

    /// `git args` in `dir` as text, with every argument taken literally.
    fn read(&self, dir: &Path, args: &[&str]) -> Result<String, Refusal> {
        let mut all: Vec<&std::ffi::OsStr> = vec![
            "--no-optional-locks".as_ref(),
            "--literal-pathspecs".as_ref(),
        ];
        all.extend(args.iter().map(std::ffi::OsStr::new));
        self.git(dir, &all)
    }

    /// Files git tracks that differ from `base` in the work tree.
    fn tracked(&self, dir: &Path, base: &str) -> Result<Vec<Changed>, Refusal> {
        let plain = [
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "--no-renames",
            "--relative",
        ];
        let mut numbers = vec!["diff", "--numstat", "-z"];
        numbers.extend(plain);
        numbers.extend([base, "--", "."]);
        let counted = self.read(dir, &numbers)?;
        let mut names = vec!["diff", "--name-status", "-z"];
        names.extend(plain);
        names.extend([base, "--", "."]);
        let named = self.read(dir, &names)?;

        let mut files: Vec<Changed> = Vec::new();
        let counts = counted.split('\0').filter(|field| !field.is_empty());
        let mut states = named.split('\0').filter(|field| !field.is_empty());
        for count in counts {
            let mut parts = count.splitn(3, '\t');
            let (Some(added), Some(removed), Some(path)) =
                (parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            let status = states
                .next()
                .and_then(|state| state.chars().next())
                .unwrap_or('M');
            // `--name-status -z` puts the path in the next field.
            let _ = states.next();
            let binary = added == "-";
            files.push(Changed {
                path: path.to_owned(),
                status,
                added: added.parse().unwrap_or(0),
                removed: removed.parse().unwrap_or(0),
                binary,
                patch: None,
                cut: false,
            });
        }
        let wanted: Vec<&String> = files
            .iter()
            .take(MAX_FILES)
            .filter(|file| !file.binary && file.added + file.removed <= MAX_PATCH_LINES)
            .map(|file| &file.path)
            .collect();
        if wanted.is_empty() {
            return Ok(files);
        }
        let mut patch = vec!["diff", "-U3"];
        patch.extend(plain);
        patch.push(base);
        patch.push("--");
        patch.extend(wanted.iter().map(|path| path.as_str()));
        let text = self.read(dir, &patch)?;
        let parts: Vec<&str> = text.split("\ndiff --git ").collect();
        if parts.len() != wanted.len() {
            // The patches do not line up with the files: the counts stand alone.
            return Ok(files);
        }
        let wanted: Vec<String> = wanted.into_iter().cloned().collect();
        for (part, path) in parts.into_iter().zip(wanted) {
            let Some(file) = files.iter_mut().find(|file| file.path == path) else {
                continue;
            };
            let body = part.find("\n@@").map_or("", |at| &part[at + 1..]);
            let (kept, cut) = cut_patch(body);
            file.patch = Some(kept);
            file.cut = cut;
        }
        Ok(files)
    }

    /// Files git does not track and does not ignore, whose lines are shown as added when they are small text.
    fn untracked(&self, dir: &Path) -> Result<Vec<Changed>, Refusal> {
        let listed = self.read(
            dir,
            &[
                "ls-files",
                "--others",
                "--exclude-standard",
                "-z",
                "--",
                ".",
            ],
        )?;
        let mut files = Vec::new();
        for name in listed.split('\0').filter(|name| !name.is_empty()) {
            let full = dir.join(name);
            let plain = std::fs::symlink_metadata(&full)
                .ok()
                .filter(|meta| meta.is_file());
            let Some(meta) = plain else { continue };
            let mut file = Changed {
                path: name.to_owned(),
                status: 'A',
                added: 0,
                removed: 0,
                binary: true,
                patch: None,
                cut: false,
            };
            let inside = resolve(&full).is_ok_and(|resolved| self.inside(&resolved));
            if inside && meta.len() <= MAX_UNTRACKED {
                let mut bytes = Vec::new();
                if std::fs::File::open(&full)
                    .and_then(|opened| opened.take(MAX_UNTRACKED).read_to_end(&mut bytes))
                    .is_ok()
                    && !bytes.contains(&0)
                {
                    let text = String::from_utf8_lossy(&bytes);
                    let lines: Vec<&str> = text.lines().collect();
                    let mut patch = format!("@@ -0,0 +1,{} @@\n", lines.len());
                    for line in &lines {
                        patch.push('+');
                        patch.push_str(line);
                        patch.push('\n');
                    }
                    let (kept, cut) = cut_patch(&patch);
                    file.binary = false;
                    file.added = lines.len() as u64;
                    file.patch = Some(kept);
                    file.cut = cut;
                }
            }
            files.push(file);
        }
        Ok(files)
    }
}

/// `patch` cut to [`MAX_PATCH`] bytes at a line end, and whether it was cut.
fn cut_patch(patch: &str) -> (String, bool) {
    if patch.len() <= MAX_PATCH {
        return (patch.to_owned(), false);
    }
    let mut end = MAX_PATCH;
    while !patch.is_char_boundary(end) {
        end -= 1;
    }
    let end = patch[..end].rfind('\n').map_or(end, |at| at + 1);
    (patch[..end].to_owned(), true)
}
