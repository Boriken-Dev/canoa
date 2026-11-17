//! A git source fetched whole as a tree of files, which the extension install checks against the digest its catalog gave
//! before it writes any of it.

use std::collections::BTreeMap;
use std::fs;

use super::{Overlays, Trust, refused};
use crate::profile::full_commit_id;

/// What a source held at one commit under one path.
#[derive(Debug)]
pub(crate) struct FetchedTree {
    /// The tree digest Canoa computes over the files, as an overlay's pin is, as `sha256:` and 64 hexadecimal digits.
    pub(crate) digest: String,
    /// Each file by its path under the source's own path, with its bytes.
    pub(crate) files: BTreeMap<String, Vec<u8>>,
}

impl Overlays {
    /// The tree `path` names in the git source `repo` at `reference` (the source's head when none), one commit deep, as the
    /// source holds it: nothing under an excluded name, no link, and no more than this host keeps of an overlay. `trust` is how
    /// the fetch is made: the person's own git settings for a source in the catalog they added, or none of them for a source a
    /// catalog names, as a skill an overlay lists is fetched. Nothing is kept.
    pub(crate) fn fetch_tree(
        &self,
        repo: &str,
        reference: Option<&str>,
        path: &str,
        trust: Trust,
    ) -> Result<FetchedTree, (i32, String)> {
        let mut token = [0_u8; 8];
        getrandom::fill(&mut token).map_err(|_| refused("no randomness to name the fetch"))?;
        let cache = self.root()?.join(format!(
            ".fetch-{}.git",
            data_encoding::HEXLOWER.encode(&token)
        ));
        let answer = self.fetch_tree_into(&cache, repo, reference, path, trust);
        let _ = fs::remove_dir_all(&cache);
        answer
    }

    fn fetch_tree_into(
        &self,
        cache: &std::path::Path,
        repo: &str,
        reference: Option<&str>,
        path: &str,
        trust: Trust,
    ) -> Result<FetchedTree, (i32, String)> {
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
        let commit = match reference {
            Some(reference) => self.fetch(trust, cache, repo, reference)?,
            None => self.fetch_head(trust, cache, repo)?,
        };
        if !full_commit_id(&commit) {
            return Err(refused("the source gave no commit"));
        }
        let (listed, _skipped, digest, _bytes) = self.list(cache, &commit, path)?;
        if listed.is_empty() {
            return Err(refused("the source holds no file at that path"));
        }
        let ids: Vec<String> = listed.values().cloned().collect();
        let blobs = self.blobs(cache, &super::unique(ids.iter()))?;
        let mut files = BTreeMap::new();
        for (name, blob) in &listed {
            let bytes = blobs
                .get(blob)
                .ok_or_else(|| refused("git did not give a file it was asked for"))?;
            files.insert(name.clone(), bytes.clone());
        }
        Ok(FetchedTree { digest, files })
    }
}
