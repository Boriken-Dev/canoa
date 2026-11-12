//! The store: unpacking a payload once per version, checking an unpacked folder against its manifest, and removing the
//! versions the launcher left behind.
//!
//! Layout under the base folder: one folder per version holding the app and a copy of its manifest ([`MARKER`]), which marks
//! the folder as this launcher's. The launcher removes only a folder that carries a manifest naming its own folder.

use crate::error::{Error, Result};
use crate::guard;
use crate::manifest::{MARKER, Manifest, sha256_hex};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{self, Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use zip::ZipArchive;

/// Versions kept after a launch, the current one included.
pub const KEEP_VERSIONS: usize = 2;
/// Biggest manifest the launcher reads from a payload.
const MAX_MANIFEST: u64 = 4 << 20;
/// How old an unfinished unpack must be before another launch removes it.
const STALE_UNPACK: Duration = Duration::from_secs(3600);

const UNPACK: &str = ".unpack-";
const OLD: &str = ".old-";
const GONE: &str = ".gone-";

/// The payload a launcher carries: the archive and the SHA-256 recorded when it was built.
#[derive(Clone, Copy, Debug)]
pub struct Payload<'a> {
    /// The zip archive [`pack`](crate::pack) made.
    pub bytes: &'a [u8],
    /// Hex SHA-256 of `bytes`, fixed at build time.
    pub sha256: &'a str,
}

/// An unpacked, verified app folder.
#[derive(Clone, Debug)]
pub struct Installed {
    /// The folder, `<base>/<version>`.
    pub dir: PathBuf,
    /// What the folder holds.
    pub manifest: Manifest,
    /// Whether the folder was already there and intact, so nothing was unpacked.
    pub reused: bool,
}

impl Installed {
    /// The path of `name`, only when the manifest lists it: nothing else is ever run.
    pub fn program(&self, name: &str) -> Result<PathBuf> {
        if self.manifest.entry(name).is_none() {
            return Err(Error::Refused(format!("{name} is not in the manifest")));
        }
        Ok(self.dir.join(name))
    }
}

impl Payload<'_> {
    /// The manifest, after checking the archive against its recorded digest.
    pub fn manifest(&self) -> Result<Manifest> {
        if self.bytes.is_empty() {
            return Err(Error::Payload("this launcher carries no app".into()));
        }
        if sha256_hex(self.bytes) != self.sha256 {
            return Err(Error::Payload(
                "the archive does not match its recorded SHA-256".into(),
            ));
        }
        let mut zip = open(self.bytes)?;
        let mut file = zip
            .by_name(MARKER)
            .map_err(|e| Error::Payload(format!("no manifest: {e}")))?;
        let mut text = String::new();
        file.by_ref()
            .take(MAX_MANIFEST + 1)
            .read_to_string(&mut text)
            .map_err(|e| Error::Payload(format!("read the manifest: {e}")))?;
        if text.len() as u64 > MAX_MANIFEST {
            return Err(Error::Payload("the manifest is too large".into()));
        }
        Manifest::parse(&text)
    }
}

fn open(bytes: &[u8]) -> Result<ZipArchive<Cursor<&[u8]>>> {
    ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| Error::Payload(format!("not a zip archive: {e}")))
}

/// Makes `<base>/<version>` hold exactly the payload's files, unpacking only when it is absent or differs.
///
/// An existing folder is read first: a link or another owner is refused, and anything else that is not exactly the
/// manifest (a changed or missing file, one more file, a link inside) is replaced by a fresh unpack. The unpack goes to a
/// temporary sibling, every file is checked against the manifest as it is written, and a rename puts it in place.
pub fn install(base: &Path, payload: &Payload<'_>) -> Result<Installed> {
    let manifest = payload.manifest()?;
    guard::ensure_base(base)?;
    let dir = base.join(&manifest.version);
    if guard::check_dir(base, &dir)?.is_some() && differs(&dir, &manifest)?.is_none() {
        return Ok(Installed {
            dir,
            manifest,
            reused: true,
        });
    }
    let fresh = base.join(guard::unique(UNPACK));
    if let Err(e) = unpack(payload, &manifest, &fresh) {
        remove_tree(&fresh);
        return Err(e);
    }
    let aside = base.join(guard::unique(OLD));
    let had_old = fs::symlink_metadata(&dir).is_ok();
    if had_old {
        fs::rename(&dir, &aside).map_err(|e| {
            remove_tree(&fresh);
            Error::io(
                format!("replace {} (is Canoa running from it?)", dir.display()),
                e,
            )
        })?;
    }
    if let Err(e) = fs::rename(&fresh, &dir) {
        if had_old {
            let _ = fs::rename(&aside, &dir);
        }
        // Another launch may have put the same version in place first.
        let raced = guard::check_dir(base, &dir).ok().flatten().is_some()
            && differs(&dir, &manifest)?.is_none();
        remove_tree(&fresh);
        if raced {
            return Ok(Installed {
                dir,
                manifest,
                reused: true,
            });
        }
        return Err(Error::io(format!("put {} in place", dir.display()), e));
    }
    remove_tree(&aside);
    Ok(Installed {
        dir,
        manifest,
        reused: false,
    })
}

fn unpack(payload: &Payload<'_>, manifest: &Manifest, into: &Path) -> Result<()> {
    let mut zip = open(payload.bytes)?;
    if zip.len() != manifest.entries.len() + 1 {
        return Err(Error::Payload(
            "the archive holds files the manifest does not list".into(),
        ));
    }
    fs::create_dir(into).map_err(|e| Error::io(format!("create {}", into.display()), e))?;
    let mut buffer = vec![0u8; 64 << 10];
    for entry in &manifest.entries {
        let mut source = zip
            .by_name(&entry.path)
            .map_err(|e| Error::Payload(format!("{} is not in the archive: {e}", entry.path)))?;
        if source.size() != entry.size || !source.is_file() {
            return Err(Error::Payload(format!(
                "{} is not the size or kind the manifest says",
                entry.path
            )));
        }
        let target = into.join(&entry.path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| Error::io(format!("create {}", parent.display()), e))?;
        }
        let mut out = File::options()
            .write(true)
            .create_new(true)
            .open(&target)
            .map_err(|e| Error::io(format!("create {}", target.display()), e))?;
        let mut hasher = Sha256::new();
        let mut left = entry.size;
        while left > 0 {
            let want = buffer
                .len()
                .min(usize::try_from(left).unwrap_or(usize::MAX));
            let got = source
                .read(&mut buffer[..want])
                .map_err(|e| Error::io(format!("unpack {}", entry.path), e))?;
            if got == 0 {
                return Err(Error::Payload(format!("{} ends early", entry.path)));
            }
            hasher.update(&buffer[..got]);
            out.write_all(&buffer[..got])
                .map_err(|e| Error::io(format!("write {}", target.display()), e))?;
            left -= got as u64;
        }
        let mut extra = [0u8; 1];
        if source
            .read(&mut extra)
            .map_err(|e| Error::io(format!("unpack {}", entry.path), e))?
            != 0
        {
            return Err(Error::Payload(format!(
                "{} is longer than the manifest says",
                entry.path
            )));
        }
        if <[u8; 32]>::from(hasher.finalize()) != entry.sha256 {
            return Err(Error::Payload(format!(
                "{} does not match its SHA-256",
                entry.path
            )));
        }
    }
    fs::write(into.join(MARKER), manifest.text())
        .map_err(|e| Error::io("write the manifest copy", e))
}

/// Why the folder is not exactly what `manifest` lists, or `None` when it is.
///
/// Every listed file is read and hashed; the folder may hold no other file, no link and nothing that is neither a file nor
/// a folder, since a file the app's program would load without it being listed is how a planted library runs.
pub fn differs(dir: &Path, manifest: &Manifest) -> Result<Option<String>> {
    let mut found = BTreeSet::new();
    if let Some(why) = list(dir, "", &mut found)? {
        return Ok(Some(why));
    }
    let listed: BTreeSet<String> = manifest
        .entries
        .iter()
        .map(|e| e.path.to_lowercase())
        .collect();
    for path in &found {
        if path != &MARKER.to_lowercase() && !listed.contains(path) {
            return Ok(Some(format!("{path} is not in the manifest")));
        }
    }
    match fs::read_to_string(dir.join(MARKER)) {
        Ok(text) if text == manifest.text() => {}
        Ok(_) => return Ok(Some("its manifest copy differs".into())),
        Err(e) => return Ok(Some(format!("its manifest copy cannot be read: {e}"))),
    }
    let mut buffer = vec![0u8; 64 << 10];
    for entry in &manifest.entries {
        let path = dir.join(&entry.path);
        let mut file = match File::open(&path) {
            Ok(file) => file,
            Err(e) => return Ok(Some(format!("{} cannot be read: {e}", entry.path))),
        };
        let mut hasher = Sha256::new();
        let mut size = 0u64;
        loop {
            let got = file
                .read(&mut buffer)
                .map_err(|e| Error::io(format!("read {}", path.display()), e))?;
            if got == 0 {
                break;
            }
            size += got as u64;
            if size > entry.size {
                break;
            }
            hasher.update(&buffer[..got]);
        }
        if size != entry.size || <[u8; 32]>::from(hasher.finalize()) != entry.sha256 {
            return Ok(Some(format!("{} has changed", entry.path)));
        }
    }
    Ok(None)
}

/// Collects the lower-cased relative path of every file under `dir`; `Some(why)` for a link or an odd entry.
fn list(dir: &Path, prefix: &str, found: &mut BTreeSet<String>) -> Result<Option<String>> {
    let read = fs::read_dir(dir).map_err(|e| Error::io(format!("list {}", dir.display()), e))?;
    for item in read {
        let item = item.map_err(|e| Error::io(format!("list {}", dir.display()), e))?;
        let name = item.file_name().to_string_lossy().into_owned();
        let rel = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let meta = fs::symlink_metadata(item.path())
            .map_err(|e| Error::io(format!("inspect {rel}"), e))?;
        if guard::is_link(&meta) {
            return Ok(Some(format!("{rel} is a link")));
        }
        if meta.is_dir() {
            if let Some(why) = list(&item.path(), &rel, found)? {
                return Ok(Some(why));
            }
        } else if meta.is_file() {
            found.insert(rel.to_lowercase());
        } else {
            return Ok(Some(format!("{rel} is not a plain file or folder")));
        }
    }
    Ok(None)
}

/// Removes what earlier launches left: all but the newest version besides `current`, and unfinished or half-removed folders.
///
/// Only a folder that is a real directory, owned by this user, and holds a manifest copy naming that same folder is a
/// version; anything else in the base folder is left alone. A folder an app is still running from cannot be renamed on
/// Windows and stays until a later launch. Returns the names removed.
pub fn prune(base: &Path, current: &str) -> Vec<String> {
    let mut removed = Vec::new();
    let Ok(read) = fs::read_dir(base) else {
        return removed;
    };
    let mut versions: Vec<(SystemTime, String)> = Vec::new();
    for item in read.flatten() {
        let name = item.file_name().to_string_lossy().into_owned();
        let path = item.path();
        if !matches!(guard::check_dir(base, &path), Ok(Some(()))) {
            continue;
        }
        if name.starts_with(OLD) || name.starts_with(GONE) {
            remove_tree(&path);
        } else if name.starts_with(UNPACK) {
            let age = fs::metadata(&path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok());
            if age.is_some_and(|a| a > STALE_UNPACK) {
                remove_tree(&path);
            }
        } else if name != current && is_version(&path, &name) {
            let stamp = fs::metadata(path.join(MARKER))
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            versions.push((stamp, name));
        }
    }
    versions.sort_by(|a, b| b.cmp(a));
    for (_, name) in versions.into_iter().skip(KEEP_VERSIONS - 1) {
        let path = base.join(&name);
        let gone = base.join(guard::unique(GONE));
        if fs::rename(&path, &gone).is_ok() {
            remove_tree(&gone);
            removed.push(name);
        }
    }
    removed
}

/// Whether `path` holds a regular manifest copy whose version is `name`.
fn is_version(path: &Path, name: &str) -> bool {
    let marker = path.join(MARKER);
    let regular = fs::symlink_metadata(&marker).is_ok_and(|m| m.is_file() && !guard::is_link(&m));
    regular
        && fs::read_to_string(&marker)
            .ok()
            .and_then(|t| Manifest::parse(&t).ok())
            .is_some_and(|m| m.version == name)
}

/// Removes a tree this launcher made, best effort; a link is removed itself, never followed.
fn remove_tree(path: &Path) {
    match fs::symlink_metadata(path) {
        Ok(meta) if guard::is_link(&meta) => {
            let _ = fs::remove_dir(path).or_else(|_| fs::remove_file(path));
        }
        Ok(_) => {
            let _ = fs::remove_dir_all(path);
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(_) => {}
    }
}
