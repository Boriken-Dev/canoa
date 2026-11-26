//! The content cache: documents read through the registry, kept on disk by URI so a reader that asks for caching answers from what
//! it kept while it is fresh. The oldest go first when the folder holds more than `[host.content] cache_bytes`.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest as _, Sha256};

/// What the cache keeps of one document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Cached {
    pub(crate) body: Vec<u8>,
    /// When it was read, in milliseconds since the epoch.
    pub(crate) fetched_ms: u64,
    /// Whether it can never change (a commit's file, a `data:` URI): it never goes stale.
    pub(crate) immutable: bool,
}

/// The folder documents are kept in and how much it may hold.
#[derive(Clone, Debug)]
pub(crate) struct ContentCache {
    dir: PathBuf,
    max_bytes: u64,
}

/// The most the cache holds unless `[host.content] cache_bytes` says otherwise.
pub(crate) const CACHE_BYTES: u64 = 512 << 20;

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
        })
}

impl ContentCache {
    pub(crate) const fn new(dir: PathBuf, max_bytes: u64) -> Self {
        Self { dir, max_bytes }
    }

    pub(crate) const fn max_bytes(&self) -> u64 {
        self.max_bytes
    }

    fn file(&self, uri: &str) -> PathBuf {
        let name = data_encoding::HEXLOWER.encode(&Sha256::digest(uri.as_bytes()));
        self.dir.join(format!("{}.doc", &name[..40]))
    }

    /// What is kept for `uri`.
    pub(crate) fn get(&self, uri: &str) -> Option<Cached> {
        let bytes = std::fs::read(self.file(uri)).ok()?;
        let end = bytes.iter().position(|byte| *byte == b'\n')?;
        let header: serde_json::Value = serde_json::from_slice(&bytes[..end]).ok()?;
        (header["uri"] == uri).then_some(())?;
        Some(Cached {
            body: bytes[end + 1..].to_vec(),
            fetched_ms: header["fetched"].as_u64()?,
            immutable: header["immutable"].as_bool().unwrap_or(false),
        })
    }

    /// Keeps `body` for `uri`, then drops the oldest documents while the folder holds more than it may. A document larger than the
    /// whole cache is not kept.
    pub(crate) fn put(&self, uri: &str, body: &[u8], immutable: bool) {
        if body.len() as u64 > self.max_bytes {
            return;
        }
        let header = serde_json::json!({"uri": uri, "fetched": now_ms(), "immutable": immutable});
        let mut bytes = header.to_string().into_bytes();
        bytes.push(b'\n');
        bytes.extend_from_slice(body);
        let _ = std::fs::create_dir_all(&self.dir);
        if std::fs::write(self.file(uri), bytes).is_ok() {
            self.prune();
        }
    }

    fn files(&self) -> Vec<(PathBuf, u64, SystemTime)> {
        std::fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "doc"))
            .filter_map(|entry| {
                let meta = entry.metadata().ok()?;
                Some((
                    entry.path(),
                    meta.len(),
                    meta.modified().unwrap_or(UNIX_EPOCH),
                ))
            })
            .collect()
    }

    fn prune(&self) {
        let mut files = self.files();
        let mut total: u64 = files.iter().map(|(_, size, _)| size).sum();
        files.sort_by_key(|(_, _, modified)| *modified);
        for (path, size, _) in files {
            if total <= self.max_bytes {
                break;
            }
            if std::fs::remove_file(path).is_ok() {
                total = total.saturating_sub(size);
            }
        }
    }

    /// How many documents are kept and how many bytes they take.
    pub(crate) fn stats(&self) -> (usize, u64) {
        let files = self.files();
        (files.len(), files.iter().map(|(_, size, _)| size).sum())
    }

    /// Forgets every document; how many there were.
    pub(crate) fn clear(&self) -> usize {
        self.files()
            .into_iter()
            .filter(|(path, _, _)| std::fs::remove_file(path).is_ok())
            .count()
    }
}
