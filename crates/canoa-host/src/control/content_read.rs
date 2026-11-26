//! `content/stat`, `content/list`, `content/read`, `content/resolve` and `content/cache`: the content registry asked directly, which is
//! what the Content page's Try box does. The person's grant to host things is what reads a URI, since a URI may be a file of this
//! computer or a repository fetched with their git settings.

use canoa::content::{EntryKind, Refusal, RefusalKind, Stat};
use serde_json::{Value, json};

use super::{Control, NOT_FOUND, REFUSED, bad};

type Outcome = Result<Value, (i32, String)>;

/// The most a page of a listing holds unless the caller asks for less.
const PAGE: u32 = 200;
/// The most of a document `content/read` shows as text unless the caller asks for more.
const PREVIEW_BYTES: u64 = 64 * 1024;

fn kind_word(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::Dir => "dir",
        _ => "file",
    }
}

fn stat_json(stat: &Stat) -> Value {
    json!({
        "kind": kind_word(stat.kind),
        "size": stat.size,
        "modified_ms": stat.modified_ms,
        "etag": stat.etag,
        "content_type": stat.content_type,
        "revision": stat.revision,
        "immutable": stat.immutable,
        "digests": stat.digests.iter().map(|digest| json!({
            "algorithm": digest.algorithm,
            "value": digest.value,
            "provenance": match digest.provenance {
                canoa::content::Provenance::Published => "published",
                _ => "derived",
            },
        })).collect::<Vec<_>>(),
    })
}

/// What the Content page says a digest of a file is worth: the badge names the hash its publisher states, or says none is.
fn badge(stat: &Stat) -> Value {
    stat.digests
        .iter()
        .find(|digest| digest.provenance == canoa::content::Provenance::Published)
        .map_or(Value::Null, |digest| match digest.algorithm.as_str() {
            "sha256" => json!("SHA-256"),
            "sha512" => json!("SHA-512"),
            "sha1" => json!("SHA-1"),
            "md5" => json!("MD5"),
            "git_blob" => json!("Git blob"),
            other => json!(other.to_ascii_uppercase()),
        })
}

fn refused(refusal: &Refusal) -> (i32, String) {
    let code = if refusal.kind == RefusalKind::NotFound {
        NOT_FOUND
    } else {
        REFUSED
    };
    (code, refusal.why.clone())
}

fn uri_of(params: &Value) -> Result<&str, (i32, String)> {
    params
        .get("uri")
        .and_then(Value::as_str)
        .filter(|uri| !uri.trim().is_empty())
        .ok_or_else(|| bad("uri is the address or the path to ask about"))
}

fn number(params: &Value, key: &str) -> Result<Option<u64>, (i32, String)> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .map(Some)
            .ok_or_else(|| bad(&format!("{key} is a whole number"))),
    }
}

impl Control {
    /// `content/stat` `{"uri"}` -> `{"stat": {kind, size?, modified_ms?, etag?, content_type?, revision?, immutable, digests: [{algorithm,
    /// value, provenance}]}, "badge"}` where `badge` names the hash the publisher states (`SHA-256`, `Git blob`, ...) or is null.
    /// Needs `hosting`.
    pub(super) fn content_stat(&self, params: &Value) -> Outcome {
        let uri = uri_of(params)?;
        let stat = self
            .catalog_sources()
            .content()
            .stat(uri, true)
            .map_err(|why| refused(&why))?;
        Ok(json!({"stat": stat_json(&stat), "badge": badge(&stat)}))
    }

    /// `content/list` `{"uri", "cursor"?, "max_entries"?}` -> `{"entries": [{path, kind, size?}], "next"?}`. Needs `hosting`.
    pub(super) fn content_list(&self, params: &Value) -> Outcome {
        let uri = uri_of(params)?;
        let max = number(params, "max_entries")?
            .map_or(PAGE, |max| u32::try_from(max).unwrap_or(PAGE))
            .max(1);
        let cursor = params
            .get("cursor")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let listing = self
            .catalog_sources()
            .content()
            .list_page(uri, true, max, cursor)
            .map_err(|why| refused(&why))?;
        Ok(json!({
            "entries": listing.entries.iter().map(|entry| json!({
                "path": entry.path,
                "kind": kind_word(entry.kind),
                "size": entry.stat.as_ref().and_then(|stat| stat.size),
            })).collect::<Vec<_>>(),
            "next": listing.next,
        }))
    }

    /// `content/read` `{"uri", "max_bytes"?, "refresh"?}` -> `{"text", "bytes", "truncated", "from": "cache" | "fetched", "stat"?,
    /// "badge"?}`: the file as text (lossy), at most `max_bytes` of it (64 KiB unless asked), read through the content cache
    /// unless `refresh`. Needs `hosting`.
    pub(super) fn content_read(&self, params: &Value) -> Outcome {
        let uri = uri_of(params)?;
        let shown = number(params, "max_bytes")?.unwrap_or(PREVIEW_BYTES);
        let refresh = params
            .get("refresh")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let sources = self.catalog_sources();
        let content = sources.content();
        let read = content
            .read_cached(uri, true, content.most_bytes(), refresh)
            .map_err(|why| refused(&why))?;
        let total = read.bytes.len();
        let cut = usize::try_from(shown).unwrap_or(usize::MAX).min(total);
        let mut answer = json!({
            "text": String::from_utf8_lossy(&read.bytes[..cut]),
            "bytes": total,
            "truncated": cut < total,
            "from": read.from,
        });
        if let Some(stat) = &read.stat {
            answer["stat"] = stat_json(stat);
            answer["badge"] = badge(stat);
        }
        Ok(answer)
    }

    /// `content/resolve` `{"uri"}` -> `{"uri", "revision", "immutable"}`: a branch or tag made a commit. Needs `hosting`.
    pub(super) fn content_resolve(&self, params: &Value) -> Outcome {
        let uri = uri_of(params)?;
        let resolved = self
            .catalog_sources()
            .content()
            .resolve(uri, true)
            .map_err(|why| refused(&why))?;
        Ok(json!({
            "uri": resolved.uri,
            "revision": resolved.revision,
            "immutable": resolved.immutable,
        }))
    }

    /// `content/cache` `{"clear"?}` -> `{"entries", "bytes", "max_bytes", "cleared"?}`: what the content cache keeps; `clear: true`
    /// forgets it all first. Needs `hosting`.
    pub(super) fn content_cache(&self, params: &Value) -> Outcome {
        let sources = self.catalog_sources();
        let content = sources.content();
        let cleared = params
            .get("clear")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            .then(|| content.clear_cache());
        let (entries, bytes, max_bytes) = content.cache_stats().unwrap_or((0, 0, 0));
        let mut answer = json!({"entries": entries, "bytes": bytes, "max_bytes": max_bytes});
        if let Some(cleared) = cleared {
            answer["cleared"] = json!(cleared);
        }
        Ok(answer)
    }
}
