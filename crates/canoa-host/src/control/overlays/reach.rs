//! Reading one file, or one folder's entries, of a git repository at a ref: what the `git+` content schemes answer with. It runs the
//! same isolated git the overlays use, into a bare cache of its own under `.content/`, so a fetch of a ref follows the ref's tip
//! and never leaves another repository's objects in an overlay's cache.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files and processes are std's"
)]

use std::ffi::OsString;
use std::path::PathBuf;

use sha2::{Digest as _, Sha256};

use super::{Overlays, Trust, refused};
use crate::profile::full_commit_id;

/// One entry of a git folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GitEntry {
    /// Its path relative to the folder listed.
    pub(crate) path: String,
    /// Whether it is a folder.
    pub(crate) dir: bool,
    /// The size of a file.
    pub(crate) size: Option<u64>,
}

/// What a path of a commit is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum GitNode {
    /// A file, with its object id and size.
    File { id: String, size: u64 },
    /// A folder and what it holds.
    Dir(Vec<GitEntry>),
}

/// A repository fetched to a bare cache at one commit.
#[derive(Clone, Debug)]
pub(crate) struct Fetched {
    cache: PathBuf,
    /// The commit the ref named when it was fetched.
    pub(crate) commit: String,
}

type Failure = (i32, String);

/// One row of `git ls-tree -l`: mode, type, size and `id<TAB>name`.
type Row = (String, String, u64, String);

impl Overlays {
    /// The commit `reference` names at `repo` now, from one `ls-remote` and no fetch; none names the default branch.
    pub(crate) fn content_commit(
        &self,
        trust: Trust,
        repo: &str,
        reference: Option<&str>,
    ) -> Result<String, Failure> {
        match reference {
            Some(reference) => self.remote_ref(trust, repo, reference).map(|(_, id)| id),
            None => {
                let listed = self.run_as(
                    trust,
                    &["ls-remote".into(), "--".into(), repo.into(), "HEAD".into()],
                    None,
                    None,
                    1024 * 1024,
                )?;
                String::from_utf8_lossy(&listed)
                    .lines()
                    .find_map(|line| line.split_once('\t'))
                    .map(|(id, _)| id.to_owned())
                    .ok_or_else(|| refused("The source has no default branch"))
            }
        }
    }

    /// Fetches `reference` (the default branch when none) of `repo`, one commit deep, into this repository's own cache.
    pub(crate) fn content_fetch(
        &self,
        trust: Trust,
        repo: &str,
        reference: Option<&str>,
    ) -> Result<Fetched, Failure> {
        let name = data_encoding::HEXLOWER.encode(&Sha256::digest(repo.as_bytes()));
        let cache = self
            .root()?
            .join(".content")
            .join(format!("{}.git", &name[..32]));
        let gate = self.gate(&format!("content-{}", &name[..32]));
        let _held = gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(parent) = cache.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| refused(format!("the cache could not be made: {error}")))?;
        }
        self.init_cache(&cache)?;
        let commit = match reference {
            Some(reference) => self.fetch(trust, &cache, repo, reference)?,
            None => {
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
                    Some(&cache),
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
                String::from_utf8_lossy(&found).trim().to_owned()
            }
        };
        if !full_commit_id(&commit) {
            return Err(refused("git did not give a commit id"));
        }
        Ok(Fetched { cache, commit })
    }

    /// The rows `git ls-tree` gives for `target` at the fetched commit; `deep` lists every file below a folder.
    fn content_rows(
        &self,
        fetched: &Fetched,
        target: &str,
        deep: bool,
    ) -> Result<Vec<Row>, Failure> {
        let mut args: Vec<OsString> = vec![
            format!("--git-dir={}", fetched.cache.display()).into(),
            "ls-tree".into(),
            "-l".into(),
            "-z".into(),
        ];
        if deep {
            args.push("-r".into());
        }
        args.extend([fetched.commit.clone().into(), "--".into()]);
        if !target.is_empty() {
            args.push(target.into());
        }
        let out = self.run(&args, None, None, 64 * 1024 * 1024)?;
        let mut rows = Vec::new();
        for record in out.split(|byte| *byte == 0).filter(|each| !each.is_empty()) {
            let text = String::from_utf8_lossy(record);
            let (meta, name) = text
                .split_once('\t')
                .ok_or_else(|| refused("git ls-tree answered badly"))?;
            let mut words = meta.split_whitespace();
            let (Some(mode), Some(kind), Some(id), size) =
                (words.next(), words.next(), words.next(), words.next())
            else {
                return Err(refused("git ls-tree answered badly"));
            };
            rows.push((
                mode.to_owned(),
                kind.to_owned(),
                size.and_then(|size| size.parse().ok()).unwrap_or(0),
                format!("{id}\t{name}"),
            ));
        }
        Ok(rows)
    }

    /// The entries of the folder `path` (the repository's root when empty) at the fetched commit, a link or a submodule left out.
    fn content_entries(
        &self,
        fetched: &Fetched,
        path: &str,
        deep: bool,
    ) -> Result<Vec<GitEntry>, Failure> {
        let prefix = if path.is_empty() {
            String::new()
        } else {
            format!("{path}/")
        };
        Ok(self
            .content_rows(fetched, &prefix, deep)?
            .into_iter()
            .filter(|(mode, kind, _, _)| mode != "160000" && mode != "120000" && kind != "commit")
            .filter_map(|(_, kind, size, tail)| {
                let name = tail.split_once('\t')?.1;
                let relative = name.strip_prefix(&prefix).unwrap_or(name);
                Some(GitEntry {
                    path: relative.to_owned(),
                    dir: kind == "tree",
                    size: (kind != "tree").then_some(size),
                })
            })
            .collect())
    }

    /// What `path` is at the fetched commit; `recursive` lists every file below a folder.
    pub(crate) fn content_node(
        &self,
        fetched: &Fetched,
        path: &str,
        recursive: bool,
    ) -> Result<GitNode, Failure> {
        let path = path.trim_matches('/');
        if path.is_empty() {
            return self
                .content_entries(fetched, "", recursive)
                .map(GitNode::Dir);
        }
        let mut found = self.content_rows(fetched, path, false)?;
        let Some((mode, kind, size, tail)) = found.pop().filter(|_| found.is_empty()) else {
            return Err((
                crate::control::NOT_FOUND,
                format!("the commit has no {path}"),
            ));
        };
        if kind == "tree" {
            return self
                .content_entries(fetched, path, recursive)
                .map(GitNode::Dir);
        }
        if kind != "blob" || mode == "120000" {
            return Err(refused(format!("{path} is not a file this host reads")));
        }
        let id = tail
            .split_once('\t')
            .map_or_else(String::new, |(id, _)| id.to_owned());
        Ok(GitNode::File { id, size })
    }

    /// The bytes of the blob `id` of the fetched repository, a file of at most `max_bytes`.
    pub(crate) fn content_blob(
        &self,
        fetched: &Fetched,
        id: &str,
        max_bytes: u64,
    ) -> Result<Vec<u8>, Failure> {
        let limits = super::OverlayLimits {
            bytes: max_bytes,
            ..super::OverlayLimits::default()
        };
        let mut blobs = self.blobs_within(&fetched.cache, &[id.to_owned()], &limits)?;
        blobs
            .remove(id)
            .ok_or_else(|| refused("git did not give the file"))
    }
}
