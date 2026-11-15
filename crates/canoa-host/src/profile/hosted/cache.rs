//! What a connection would otherwise work out again and get the same: where the interpreters are, the overlays' files, and
//! the sweep of what a killed host left.

use alloc::collections::BTreeMap;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, Once, PoisonError};
use std::time::SystemTime;

use coqui_aos::OverlayTree;

/// The interpreters found, by AOS name, for one `PATH` and `PATHEXT`.
type Found = BTreeMap<&'static str, PathBuf>;

type Searched = HashMap<(String, Option<String>), Found>;

static INTERPRETERS: Mutex<Option<Searched>> = Mutex::new(None);

/// The interpreters for `path` and `extensions`: those an earlier walk of the same values found, while each is still a
/// runnable file, else `walk`'s, which are then kept.
pub(super) fn interpreters(
    path: Option<String>,
    extensions: Option<String>,
    walk: impl FnOnce() -> Found,
) -> Found {
    let Some(path) = path else {
        return walk();
    };
    let key = (path, extensions);
    let kept = INTERPRETERS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .and_then(|map| map.get(&key).cloned());
    if let Some(found) = kept
        && found.values().all(|each| super::runnable(each))
    {
        return found;
    }
    #[cfg(test)]
    counted::walked(&key.0);
    let found = walk();
    INTERPRETERS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get_or_insert_with(HashMap::new)
        .insert(key, found.clone());
    found
}

/// One file of an overlay as the filesystem lists it: its path below the root, size and modification time.
type Stamp = (String, u64, Option<SystemTime>);

struct Kept {
    /// The digest the overlay's record named.
    expected: String,
    files: Vec<Stamp>,
    tree: OverlayTree,
}

static OVERLAYS: Mutex<Option<HashMap<PathBuf, Kept>>> = Mutex::new(None);

/// Every file below `root` with its size and modification time, in path order; nothing when a link, an entry that cannot
/// be read or anything else a plain walk would not settle is met, so that the overlay is read whole.
fn stamps(root: &Path) -> Option<Vec<Stamp>> {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<Stamp>) -> Option<()> {
        for entry in std::fs::read_dir(dir).ok()? {
            let entry = entry.ok()?;
            let name = entry.file_name().into_string().ok()?;
            let meta = entry.path().symlink_metadata().ok()?;
            let path = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            if meta.is_dir() {
                walk(&entry.path(), &path, out)?;
            } else if meta.is_file() {
                out.push((path, meta.len(), meta.modified().ok()));
            } else {
                return None;
            }
        }
        Some(())
    }
    let mut out = Vec::new();
    walk(root, "", &mut out)?;
    out.sort();
    Some(out)
}

/// The tree at `root`, where `expected` is the digest the overlay's record names: the one kept from an earlier read when
/// that digest and every file's path, size and modification time are the same, else what `read` returns (it checks the
/// digest itself), which is then kept.
pub(super) fn overlay<E>(
    root: &Path,
    expected: &str,
    read: impl FnOnce() -> Result<OverlayTree, E>,
) -> Result<OverlayTree, E> {
    // Taken before the read, so a file changed during it differs the next time.
    let files = stamps(root);
    if let Some(files) = &files
        && let Some(kept) = OVERLAYS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .and_then(|map| map.get(root))
        && kept.expected == expected
        && kept.files == *files
    {
        return Ok(kept.tree.clone());
    }
    #[cfg(test)]
    counted::read(root);
    let tree = read()?;
    let mut map = OVERLAYS.lock().unwrap_or_else(PoisonError::into_inner);
    match files {
        Some(files) => {
            map.get_or_insert_with(HashMap::new).insert(
                root.to_path_buf(),
                Kept {
                    expected: expected.to_owned(),
                    files,
                    tree: tree.clone(),
                },
            );
        }
        None => {
            if let Some(map) = map.as_mut() {
                map.remove(root);
            }
        }
    }
    Ok(tree)
}

/// Removes what a host that was killed left in the places private directories are made, once per process and off the
/// connection's path.
pub(super) fn sweep_once() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        #[cfg(test)]
        counted::SWEEPS.fetch_add(1, core::sync::atomic::Ordering::SeqCst);
        let _ = std::thread::Builder::new()
            .name("canoa-sweep".into())
            .spawn(|| {
                super::hooks::remove_stale(&std::env::temp_dir());
                if super::private::base() != std::env::temp_dir() {
                    super::hooks::remove_stale(&super::private::base());
                }
            });
    });
}

/// Counts of the work done, so a test can see that a second connection did none.
#[cfg(test)]
pub(super) mod counted {
    use core::sync::atomic::{AtomicUsize, Ordering};
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::{Mutex, PoisonError};

    type Counts = Mutex<Option<HashMap<String, usize>>>;

    pub(super) static SWEEPS: AtomicUsize = AtomicUsize::new(0);
    static WALKS: Counts = Mutex::new(None);
    static READS: Counts = Mutex::new(None);

    fn bump(counts: &Counts, key: String) {
        *counts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_or_insert_with(HashMap::new)
            .entry(key)
            .or_default() += 1;
    }

    fn get(counts: &Counts, key: &str) -> usize {
        counts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .and_then(|map| map.get(key).copied())
            .unwrap_or(0)
    }

    pub(super) fn walked(path: &str) {
        bump(&WALKS, path.to_owned());
    }

    pub(super) fn read(root: &Path) {
        bump(&READS, root.to_string_lossy().into_owned());
    }

    /// How many times the `PATH` value `path` was walked for interpreters.
    pub(in crate::profile::hosted) fn walks(path: &str) -> usize {
        get(&WALKS, path)
    }

    /// How many times the overlay at `root` was read whole.
    pub(in crate::profile::hosted) fn reads(root: &Path) -> usize {
        get(&READS, &root.to_string_lossy())
    }

    /// How many times the sweep was started.
    pub(in crate::profile::hosted) fn sweeps() -> usize {
        SWEEPS.load(Ordering::SeqCst)
    }
}
