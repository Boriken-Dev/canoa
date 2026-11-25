//! Reading a catalog of a type whose catalogs answer in one of the host's formats (`canoa.models/1`, `canoa.releases/1`): through the
//! entry's source provider, kept on disk beside `latest.json` so that opening the Store asks nobody and a first run with no network
//! still shows what a pinned default ships.
//!
//! An answer comes from, in this order: the file kept for the entry while it is fresh (a pinned entry's never goes stale); the
//! snapshot a pinned default ships; the provider, whose text is then checked as a document of the format (and against a
//! `pin = "sha256:..."`) before it is kept. A provider that fails leaves the last kept text in place, with the reason beside it.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files and clocks are std's"
)]

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use canoa::store::catalog::check_format;
use canoa::store::source::ReadRequest;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use super::catalogs::{Entry, READ, TypeCatalogs};
use super::sources::Sources;

/// How long a catalog answers without asking its source, unless `[host.store] source_ttl` says otherwise.
pub const DEFAULT_SOURCE_TTL: Duration = Duration::from_secs(60 * 60);

/// The most an address a bare `read` entry names may answer with.
const MOST_BYTES: u64 = 8 << 20;

/// The form of the file kept for a catalog.
const SCHEMA: u32 = 1;

/// What reading one catalog gave.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Reading {
    /// The catalog's text, a document of the type's format.
    pub(crate) text: String,
    /// When it was read from its source, in milliseconds since the epoch; zero for a snapshot.
    pub(crate) fetched: u64,
    /// Where it came from now: `fetched`, `cache` or `snapshot`.
    pub(crate) from: &'static str,
    /// Whether it is older than it should be and the source could not be asked, the reason being [`problem`](Self::problem).
    pub(crate) stale: bool,
    /// Why the source was not read, when it was not and an older answer stands in.
    pub(crate) problem: Option<String>,
}

/// The files the host keeps catalogs in, and what decides when they are asked again.
#[derive(Clone, Debug)]
pub(crate) struct Run {
    /// `<home>/store`, where `latest.json` is: none keeps nothing on disk.
    pub(crate) dir: Option<PathBuf>,
    /// `[host.store] source_ttl`.
    pub(crate) ttl: Duration,
    pub(crate) sources: Sources,
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
        })
}

/// What is kept of one catalog.
struct Kept {
    fetched: u64,
    text: String,
}

impl Run {
    /// The file the catalog `entry` of the scope `kind` is kept in: `<dir>/catalogs/<scope>/<label>.json`.
    fn file(&self, kind: &TypeCatalogs, entry: &Entry) -> Option<PathBuf> {
        let scope: String = kind
            .id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        Some(
            self.dir
                .as_ref()?
                .join("catalogs")
                .join(scope)
                .join(format!("{}.json", coqui_aos::normalize_name(&entry.label))),
        )
    }

    /// What is kept for `entry`, when what it was read with is what `entry` says now.
    fn kept(&self, kind: &TypeCatalogs, entry: &Entry) -> Option<Kept> {
        let bytes = std::fs::read(self.file(kind, entry)?).ok()?;
        let held: Value = serde_json::from_slice(&bytes).ok()?;
        (held["schema"] == json!(SCHEMA) && held["spec"] == json!(entry.spec())).then_some(())?;
        Some(Kept {
            fetched: held["fetched"].as_u64()?,
            text: held["text"].as_str()?.to_owned(),
        })
    }

    fn keep(&self, kind: &TypeCatalogs, entry: &Entry, text: &str, fetched: u64) {
        let Some(file) = self.file(kind, entry) else {
            return;
        };
        let held = json!({
            "schema": SCHEMA,
            "spec": entry.spec(),
            "fetched": fetched,
            "pin": entry.pin,
            "digest": digest(text),
            "text": text,
        });
        if let Some(folder) = file.parent() {
            let _ = std::fs::create_dir_all(folder);
        }
        let _ = crate::replace::write_file_in_folder(
            &file,
            held.to_string().as_bytes(),
            crate::replace::SHARED,
        );
    }

    /// When `entry` was last read from its source and where from, with nothing asked: what the Sources page shows beside it.
    pub(crate) fn peek(&self, kind: &TypeCatalogs, entry: &Entry) -> Value {
        match self.kept(kind, entry) {
            Some(kept) => json!({"fetched": kept.fetched, "from": "cache"}),
            None if entry.snapshot.is_some() => json!({"fetched": 0, "from": "snapshot"}),
            None => json!({"fetched": null, "from": null}),
        }
    }

    /// Asks the source of `entry` again when a catalog kept for it is past its time: what the background renewal does. One never
    /// read, a pinned one and one that is off are left alone. A source that fails leaves what is kept, to be asked at the next
    /// renewal.
    pub(crate) fn renew(&self, kind: &TypeCatalogs, entry: &Entry) {
        if !entry.enabled || entry.pin.is_some() {
            return;
        }
        let ttl = entry.ttl.map_or(self.ttl, Duration::from_secs);
        if self.kept(kind, entry).is_some_and(|kept| {
            u128::from(now_millis().saturating_sub(kept.fetched)) >= ttl.as_millis()
        }) {
            let _ = self.read(kind, entry, true);
        }
    }

    /// The catalog `entry` of `kind`, asking its source only when what is kept is stale, or `refresh` says to.
    ///
    /// # Errors
    ///
    /// Why there is no answer: the source failed and nothing older stands in, or what it gave is no document of the format.
    pub(crate) fn read(
        &self,
        kind: &TypeCatalogs,
        entry: &Entry,
        refresh: bool,
    ) -> Result<Reading, String> {
        let format = kind
            .format
            .as_deref()
            .ok_or_else(|| format!("{} keeps no catalog in a format of the host's", kind.title))?;
        if !entry.enabled {
            return Err(format!("{} is turned off", entry.label));
        }
        let ttl = entry.ttl.map_or(self.ttl, Duration::from_secs);
        let kept = self.kept(kind, entry);
        let now = now_millis();
        let fresh = |kept: &Kept| {
            entry.pin.is_some() || u128::from(now.saturating_sub(kept.fetched)) < ttl.as_millis()
        };
        if !refresh {
            if let Some(kept) = kept.as_ref().filter(|kept| fresh(kept)) {
                return Ok(Reading {
                    text: kept.text.clone(),
                    fetched: kept.fetched,
                    from: "cache",
                    stale: false,
                    problem: None,
                });
            }
            if entry.pin.is_some()
                && let Some(snapshot) = &entry.snapshot
            {
                check(entry, format, snapshot)
                    .map_err(|why| format!("the shipped snapshot: {why}"))?;
                return Ok(Reading {
                    text: snapshot.clone(),
                    fetched: 0,
                    from: "snapshot",
                    stale: false,
                    problem: None,
                });
            }
        }
        match self.fetch(kind, entry, format) {
            Ok(text) => {
                self.keep(kind, entry, &text, now);
                Ok(Reading {
                    text,
                    fetched: now,
                    from: "fetched",
                    stale: false,
                    problem: None,
                })
            }
            Err(why) => {
                if let Some(kept) = kept {
                    Ok(Reading {
                        text: kept.text,
                        fetched: kept.fetched,
                        from: "cache",
                        stale: true,
                        problem: Some(why),
                    })
                } else if let Some(snapshot) = &entry.snapshot
                    && check(entry, format, snapshot).is_ok()
                {
                    Ok(Reading {
                        text: snapshot.clone(),
                        fetched: 0,
                        from: "snapshot",
                        stale: true,
                        problem: Some(why),
                    })
                } else {
                    Err(why)
                }
            }
        }
    }

    /// The source's answer, checked as a document of the format and against the entry's digest pin.
    fn fetch(&self, kind: &TypeCatalogs, entry: &Entry, format: &str) -> Result<String, String> {
        let text = match entry
            .source()
            .filter(|source| entry.provider == READ && source.starts_with("https://"))
        {
            // An address a bare entry names is read by the host: the person wrote it, so it is the one place it may reach.
            Some(url) => self.sources.get_text(url, MOST_BYTES)?,
            None => {
                let mut request = ReadRequest::new(
                    kind.id.as_str(),
                    entry.label.as_str(),
                    Value::Object(entry.fields.clone()).to_string(),
                )
                .signed_by(entry.keys.iter().cloned());
                if let Some(pin) = &entry.pin {
                    request = request.pinned(pin.as_str());
                }
                self.sources.read(&entry.provider, request)?
            }
        };
        check(entry, format, &text)?;
        Ok(text)
    }
}

fn digest(text: &str) -> String {
    format!(
        "sha256:{}",
        data_encoding::HEXLOWER.encode(&Sha256::digest(text.as_bytes()))
    )
}

/// Whether `text` is a document of `format` and, when the entry pins a digest of it, that one.
fn check(entry: &Entry, format: &str, text: &str) -> Result<(), String> {
    if let Some(pin) = entry
        .pin
        .as_deref()
        .and_then(|pin| pin.strip_prefix("sha256:"))
        && !digest(text).ends_with(&pin.to_ascii_lowercase())
    {
        return Err(format!(
            "what {} answered is not the catalog its pin names (sha256:{pin})",
            entry.provider
        ));
    }
    check_format(format, text)
}

/// A path that names where a kept catalog of `scope` and `label` is, for a test.
#[cfg(test)]
pub(crate) fn kept_file(dir: &std::path::Path, scope: &str, label: &str) -> PathBuf {
    dir.join("catalogs")
        .join(scope)
        .join(format!("{}.json", coqui_aos::normalize_name(label)))
}

#[cfg(test)]
#[path = "catalog_run_tests.rs"]
mod tests;
