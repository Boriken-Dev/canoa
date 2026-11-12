//! Packing: an app folder into the payload archive the launcher carries.

use crate::error::{Error, Result};
use crate::guard;
use crate::manifest::{Entry, MARKER, Manifest};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Cursor, Write};
use std::path::Path;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

/// Packs the folder `dir` as `version`: a zip archive whose first entry is the manifest and whose others are every file of
/// the folder, deflated. The same folder always gives the same bytes. A link or any entry that is not a plain file or
/// folder is refused, since the launcher would not unpack it.
pub fn pack(dir: &Path, version: &str) -> Result<Vec<u8>> {
    let mut files = Vec::new();
    walk(dir, "", &mut files)?;
    let mut entries = Vec::with_capacity(files.len());
    let mut bodies = Vec::with_capacity(files.len());
    for (rel, path) in files {
        let body = fs::read(&path).map_err(|e| Error::io(format!("read {}", path.display()), e))?;
        entries.push(Entry {
            path: rel,
            size: body.len() as u64,
            sha256: Sha256::digest(&body).into(),
        });
        bodies.push(body);
    }
    // Packed in manifest order, which sorts the entries.
    let mut order: Vec<usize> = (0..entries.len()).collect();
    order.sort_by(|&a, &b| entries[a].path.cmp(&entries[b].path));
    let manifest = Manifest::new(version, entries.clone())?;

    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let zip_err = |e: zip::result::ZipError| Error::Payload(format!("write the archive: {e}"));
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file(MARKER, options).map_err(zip_err)?;
    zip.write_all(manifest.text().as_bytes())
        .map_err(|e| Error::io("write the archive", e))?;
    for i in order {
        zip.start_file(entries[i].path.as_str(), options)
            .map_err(zip_err)?;
        zip.write_all(&bodies[i])
            .map_err(|e| Error::io("write the archive", e))?;
    }
    Ok(zip.finish().map_err(zip_err)?.into_inner())
}

fn walk(dir: &Path, prefix: &str, out: &mut Vec<(String, std::path::PathBuf)>) -> Result<()> {
    let read = fs::read_dir(dir).map_err(|e| Error::io(format!("list {}", dir.display()), e))?;
    for item in read {
        let item = item.map_err(|e| Error::io(format!("list {}", dir.display()), e))?;
        let name = item.file_name().into_string().map_err(|n| {
            Error::Payload(format!("{n:?} in {} is not valid Unicode", dir.display()))
        })?;
        let rel = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let path = item.path();
        let meta = fs::symlink_metadata(&path)
            .map_err(|e| Error::io(format!("inspect {}", path.display()), e))?;
        if guard::is_link(&meta) {
            return Err(Error::Payload(format!("{} is a link", path.display())));
        }
        if meta.is_dir() {
            walk(&path, &rel, out)?;
        } else if meta.is_file() {
            out.push((rel, path));
        } else {
            return Err(Error::Payload(format!(
                "{} is not a plain file or folder",
                path.display()
            )));
        }
    }
    Ok(())
}
