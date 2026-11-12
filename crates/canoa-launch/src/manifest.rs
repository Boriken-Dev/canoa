//! The manifest: every file the app folder holds, with its size and SHA-256, and the version it is.
//!
//! The text is canonical (sorted, one form), so the same folder always packs to the same manifest and a parsed manifest
//! prints back byte for byte; a text that is not that form is refused rather than interpreted.

use crate::error::{Error, Result};
use crate::names;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

/// The first line of a manifest.
const HEADER: &str = "canoa-launch-manifest 1";
/// The name of the manifest inside the payload archive, and of its copy in an unpacked folder: the mark of a folder this
/// launcher made. No file of the app may take it.
pub const MARKER: &str = "canoa-launch.manifest";
/// Most files a payload may hold.
pub const MAX_FILES: usize = 20_000;
/// Most bytes a payload may unpack to.
pub const MAX_BYTES: u64 = 4 << 30;

/// One file of the app folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Relative path, `/`-separated.
    pub path: String,
    /// Size in bytes.
    pub size: u64,
    /// SHA-256 of the content.
    pub sha256: [u8; 32],
}

/// A parsed, canonical manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    /// The version, which names the folder it unpacks to.
    pub version: String,
    /// Files, sorted by path.
    pub entries: Vec<Entry>,
}

/// Lower-case hex of `bytes`.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// SHA-256 of `bytes`, as hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn unhex(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64
        || !text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

impl Manifest {
    /// Builds a manifest, sorting the entries and refusing a path, version or count the launcher would not unpack.
    pub fn new(version: &str, mut entries: Vec<Entry>) -> Result<Self> {
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        let manifest = Self {
            version: version.to_owned(),
            entries,
        };
        manifest.validate()?;
        Ok(manifest)
    }

    fn validate(&self) -> Result<()> {
        let bad = |why: String| Error::Payload(format!("manifest: {why}"));
        names::check_version(&self.version).map_err(bad)?;
        if self.entries.is_empty() || self.entries.len() > MAX_FILES {
            return Err(bad(format!(
                "{} files is not 1..={MAX_FILES}",
                self.entries.len()
            )));
        }
        let mut seen = BTreeSet::new();
        let mut total = 0u64;
        for entry in &self.entries {
            names::check_path(&entry.path).map_err(bad)?;
            // Windows reads names without regard to case, so two that differ only in it are one file.
            if !seen.insert(entry.path.to_lowercase()) {
                return Err(bad(format!("{:?} appears twice", entry.path)));
            }
            if entry.path.eq_ignore_ascii_case(MARKER) {
                return Err(bad(format!("{:?} is the launcher's own file", entry.path)));
            }
            total = total.saturating_add(entry.size);
        }
        if total > MAX_BYTES {
            return Err(bad(format!("{total} bytes is more than {MAX_BYTES}")));
        }
        if self.entries.windows(2).any(|w| w[0].path >= w[1].path) {
            return Err(bad("entries are not sorted".into()));
        }
        Ok(())
    }

    /// The canonical text.
    pub fn text(&self) -> String {
        let mut text = format!("{HEADER}\nversion {}\n", self.version);
        for e in &self.entries {
            text.push_str(&format!("{} {} {}\n", hex(&e.sha256), e.size, e.path));
        }
        text
    }

    /// SHA-256 of [`text`](Self::text), as hex: the digest that says which build of the app a folder holds.
    pub fn digest(&self) -> String {
        sha256_hex(self.text().as_bytes())
    }

    /// Parses `text`, refusing anything but the canonical form.
    pub fn parse(text: &str) -> Result<Self> {
        let bad = |why: &str| Error::Payload(format!("manifest: {why}"));
        let mut lines = text.split('\n');
        if lines.next() != Some(HEADER) {
            return Err(bad("not a canoa-launch manifest"));
        }
        let version = lines
            .next()
            .and_then(|l| l.strip_prefix("version "))
            .ok_or_else(|| bad("no version line"))?;
        let mut entries = Vec::new();
        for line in lines {
            if line.is_empty() {
                continue;
            }
            let mut parts = line.splitn(3, ' ');
            let (Some(digest), Some(size), Some(path)) = (parts.next(), parts.next(), parts.next())
            else {
                return Err(bad("a line is not `<sha256> <size> <path>`"));
            };
            let sha256 =
                unhex(digest).ok_or_else(|| bad("a digest is not 64 lower-case hex digits"))?;
            let size = size
                .parse::<u64>()
                .map_err(|_| bad("a size is not a number"))?;
            entries.push(Entry {
                path: path.to_owned(),
                size,
                sha256,
            });
        }
        let manifest = Self::new(version, entries)?;
        if manifest.text() != text {
            return Err(bad("the text is not in canonical form"));
        }
        Ok(manifest)
    }

    /// The entry named `path`, if any.
    pub fn entry(&self, path: &str) -> Option<&Entry> {
        self.entries
            .binary_search_by(|e| e.path.as_str().cmp(path))
            .ok()
            .map(|i| &self.entries[i])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, body: &[u8]) -> Entry {
        Entry {
            path: path.into(),
            size: body.len() as u64,
            sha256: Sha256::digest(body).into(),
        }
    }

    #[test]
    fn a_manifest_prints_back_exactly() {
        let m = Manifest::new(
            "1.0.0",
            vec![entry("b.dll", b"b"), entry("CanoaUI.exe", b"a")],
        )
        .unwrap();
        assert_eq!(m.entries[0].path, "CanoaUI.exe");
        assert_eq!(Manifest::parse(&m.text()).unwrap(), m);
        assert_eq!(m.digest(), sha256_hex(m.text().as_bytes()));
        assert!(m.entry("b.dll").is_some() && m.entry("nope").is_none());
    }

    #[test]
    fn a_manifest_that_is_not_canonical_or_not_safe_is_refused() {
        let good = Manifest::new("1.0.0", vec![entry("a", b"a")])
            .unwrap()
            .text();
        let digest = hex(&Sha256::digest(b"a"));
        assert!(Manifest::parse(&good).is_ok());
        for bad in [
            good.replace("1.0.0", "../x"),
            good.replace(" a\n", " ../a\n"),
            good.replace(&digest, &digest.to_uppercase()),
            format!("{good}\n\n"),
            format!("{good}{digest} 1 A\n"),
            format!("{good}{digest} 1 canoa-launch.manifest\n"),
            good.replace("manifest 1", "manifest 2"),
            good.replace(" 1 a", " one a"),
            format!("{good}{}", good.lines().nth(2).unwrap()),
        ] {
            assert!(Manifest::parse(&bad).is_err(), "{bad:?}");
        }
        let unsorted = format!("{HEADER}\nversion 1\n{digest} 1 b\n{digest} 1 a\n");
        assert!(Manifest::parse(&unsorted).is_err());
    }
}
