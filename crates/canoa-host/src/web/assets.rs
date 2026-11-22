//! Site files, entity tags, cache validators, and URL/Host validation for the web app server.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files, paths, locks and time are std's"
)]

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use core::time::Duration;
use std::fs::File;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::SystemTime;

use super::{Site, WebError};

/// The site built in by the `web` feature: each file's path under the site, its bytes and its entity tag, by path.
#[cfg(feature = "web")]
pub(super) static BUILT: &[(&str, &[u8], &str)] =
    include!(concat!(env!("OUT_DIR"), "/web_site.rs"));

/// Tags already worked out: a file's length and modification time, and the tag of its contents. A file whose length or
/// time moved is read again.
static TAGS: Mutex<BTreeMap<PathBuf, (u64, u128, String)>> = Mutex::new(BTreeMap::new());

/// What an answer carries after its headers.
pub(crate) enum Body {
    None,
    #[cfg(feature = "web")]
    Bytes(&'static [u8]),
    Owned(Vec<u8>),
    File(File, u64),
}

impl Body {
    pub(super) fn len(&self) -> u64 {
        match self {
            Self::None => 0,
            #[cfg(feature = "web")]
            Self::Bytes(bytes) => bytes.len() as u64,
            Self::Owned(bytes) => bytes.len() as u64,
            Self::File(_, length) => *length,
        }
    }
}

/// A file found in the site.
pub(crate) struct Found {
    pub(crate) body: Body,
    /// A strong tag of the file's contents.
    pub(crate) tag: String,
    /// When the file last changed; a site built in has no such time.
    pub(crate) modified: Option<SystemTime>,
}

pub(crate) enum Lookup {
    Found(Found),
    /// Nothing is there, or it is not a file.
    Missing,
    /// It resolves outside the site.
    Refused,
}

/// `dir` resolved, holding `index.html` and none of `keep_out`.
pub(crate) fn site_root(dir: &Path, keep_out: &[PathBuf]) -> Result<PathBuf, WebError> {
    let refused = |why| WebError::Site {
        dir: dir.to_owned(),
        why,
    };
    let root = std::fs::canonicalize(dir).map_err(|_| refused("cannot be found"))?;
    if !root.is_dir() {
        return Err(refused("is not a directory"));
    }
    if !root.join("index.html").is_file() {
        return Err(refused("has no index.html"));
    }
    // A directory that does not exist yet is held by no site.
    if keep_out
        .iter()
        .filter_map(|private| std::fs::canonicalize(private).ok())
        .any(|private| private.starts_with(&root))
    {
        return Err(refused(
            "holds a directory the host keeps private, canoa.cfg's or its identity's: serve a directory of the site's own",
        ));
    }
    Ok(root)
}

/// A Content-Security-Policy source naming one origin: `http`, `https`, `ws` or `wss`, `://`, a host of letters, digits,
/// `-` and `.`, an optional port and an optional path, with nothing that could end the directive or widen it.
pub(crate) fn connect_source(source: &str) -> bool {
    let Some(rest) = ["https://", "http://", "wss://", "ws://"]
        .iter()
        .find_map(|scheme| source.strip_prefix(scheme))
    else {
        return false;
    };
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let (host, port) = authority.split_once(':').unwrap_or((authority, "0"));
    !host.is_empty()
        && host
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'.')
        && !port.is_empty()
        && port.len() <= 5
        && port.bytes().all(|byte| byte.is_ascii_digit())
        && path.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'.' | b'_' | b'~')
        })
}

/// Whether a request's `Host` names this computer's loopback: `localhost`, `127.x.x.x` or `[::1]`, with or without a port.
pub(super) fn loopback_host(value: &str) -> bool {
    let name = match value.strip_prefix('[') {
        Some(bracketed) => bracketed
            .split_once(']')
            .map_or(bracketed, |(address, _)| address),
        None => value.split(':').next().unwrap_or(value),
    };
    name.eq_ignore_ascii_case("localhost")
        || name
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

/// A request target's path, split and percent-decoded, or `None` for one that could name anything outside the site: a
/// target not starting with `/`, `.` or `..`, a segment starting or ending with a dot or ending with a space, an empty
/// segment but a last one, a separator, colon or control character, a character Windows refuses in a name, or a name
/// Windows keeps for a device. The query is left out.
pub(crate) fn segments(target: &str) -> Option<Vec<String>> {
    let path = target.split(['?', '#']).next()?;
    let rest = path.strip_prefix('/')?;
    let mut segments = Vec::new();
    if rest.is_empty() {
        return Some(segments);
    }
    let raw: Vec<&str> = rest.split('/').collect();
    for (index, part) in raw.iter().enumerate() {
        if part.is_empty() && index + 1 == raw.len() {
            break;
        }
        let decoded = percent_encoding::percent_decode_str(part)
            .decode_utf8()
            .ok()?;
        if !plain_segment(&decoded) {
            return None;
        }
        segments.push(decoded.into_owned());
    }
    Some(segments)
}

/// A name every platform reads as one name inside its directory.
fn plain_segment(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && !name.ends_with('.')
        && !name.ends_with(' ')
        && !name.chars().any(|c| {
            c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        })
        && !windows_device(name)
}

/// Whether Windows reads `name` as a device, with any extension: `CON`, `PRN`, `AUX`, `NUL`, `CONIN$`, `CONOUT$`,
/// `CLOCK$`, and `COM` or `LPT` with a digit or a superscript one, two or three.
fn windows_device(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).trim_end();
    let upper = stem.to_ascii_uppercase();
    if matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$" | "CLOCK$"
    ) {
        return true;
    }
    let Some(number) = upper
        .strip_prefix("COM")
        .or_else(|| upper.strip_prefix("LPT"))
    else {
        return false;
    };
    let mut chars = number.chars();
    matches!(
        (chars.next(), chars.next()),
        (Some('0'..='9' | '\u{b9}' | '\u{b2}' | '\u{b3}'), None)
    )
}

/// Whether `segments` name an app route, which is answered with `index.html`: a last segment with no extension.
pub(crate) fn is_app_route(segments: &[String]) -> bool {
    segments.last().is_none_or(|last| !last.contains('.'))
}

/// The media type of a file named `name`, by its extension; `application/octet-stream` for one not known.
pub(crate) fn content_type(name: &str) -> &'static str {
    let Some((_, extension)) = name.rsplit_once('.') else {
        return "application/octet-stream";
    };
    match extension.to_ascii_lowercase().as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "txt" => "text/plain; charset=utf-8",
        "md" => "text/markdown; charset=utf-8",
        "json" | "map" => "application/json",
        "webmanifest" => "application/manifest+json",
        "wasm" => "application/wasm",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "mp3" => "audio/mpeg",
        "ogg" => "audio/ogg",
        "wav" => "audio/wav",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}

/// Whether `name` holds a hash: a part of eight hexadecimal digits or more, after the first, split at `.`, `-` and `_`
/// before the extension (`main.3f9a1b2c.js`, `index-3f9a1b2c.css`).
pub(super) fn is_hashed(name: &str) -> bool {
    let Some((stem, _)) = name.rsplit_once('.') else {
        return false;
    };
    stem.split(['.', '-', '_'])
        .skip(1)
        .any(|part| part.len() >= 8 && part.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

/// Whether a request's `Host` names this host: `localhost`, an IP address, or one of `hosts`, with any port. A name a
/// page's own site resolves to this address (DNS rebinding) is none of these.
pub(crate) fn host_allowed(value: &str, hosts: &[String]) -> bool {
    let port = |text: &str| {
        !text.is_empty() && text.len() <= 5 && text.bytes().all(|byte| byte.is_ascii_digit())
    };
    if let Some(bracketed) = value.strip_prefix('[') {
        let Some((address, after)) = bracketed.split_once(']') else {
            return false;
        };
        return address.parse::<Ipv6Addr>().is_ok()
            && (after.is_empty() || after.strip_prefix(':').is_some_and(port));
    }
    let name = match value.split_once(':') {
        Some((name, after)) if port(after) => name,
        Some(_) => return false,
        None => value,
    };
    name.eq_ignore_ascii_case("localhost")
        || name.parse::<Ipv4Addr>().is_ok()
        || hosts.iter().any(|host| host.eq_ignore_ascii_case(name))
}

/// `time` as HTTP writes a date: `Sun, 06 Nov 1994 08:49:37 GMT`.
pub(crate) fn http_date(time: SystemTime) -> String {
    crate::utc::http_date(time)
}

/// The headers a file's answer carries: its type, its caching, its tag and, when the file has one, when it changed.
pub(crate) fn validators(file: &Found, cache: String, name: &str) -> Vec<(&'static str, String)> {
    let mut headers = vec![
        ("Content-Type", content_type(name).to_owned()),
        ("Cache-Control", cache),
        ("ETag", file.tag.clone()),
    ];
    if let Some(time) = file.modified {
        headers.push(("Last-Modified", http_date(time)));
    }
    headers
}

/// How long a browser keeps the file at `segments`: a year, never asking again, when the path carries a version (a part
/// of the file's name that is a hash, `main.3f9a1b2c.js`, or a folder named for a revision), else it asks each time and
/// is answered 304 while the tag holds. `private` marks a site behind a sign-in, which no shared cache may keep.
pub(crate) fn cache_control(
    segments: &[String],
    name: &str,
    immutable_age: Duration,
    private: bool,
) -> String {
    let scope = if private { "private" } else { "public" };
    if is_hashed(name)
        || segments
            .iter()
            .rev()
            .skip(1)
            .any(|folder| is_revision(folder))
    {
        format!("{scope}, max-age={}, immutable", immutable_age.as_secs())
    } else if private {
        "private, no-cache".to_owned()
    } else {
        "no-cache".to_owned()
    }
}

/// Whether a folder's name is a build's revision or digest: twelve hexadecimal digits or more, and nothing else.
fn is_revision(folder: &str) -> bool {
    folder.len() >= 12 && folder.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Whether the browser's copy is current: `If-None-Match` names the file's tag, or, when the request has none,
/// `If-Modified-Since` is no earlier than the file's change. A file with no change time is only asked by its tag.
pub(crate) fn not_modified(
    none_match: Option<&str>,
    modified_since: Option<&str>,
    file: &Found,
) -> bool {
    if let Some(tags) = none_match {
        return tag_matches(tags, &file.tag);
    }
    let (Some(since), Some(changed)) = (modified_since.and_then(parse_http_date), file.modified)
    else {
        return false;
    };
    changed
        .duration_since(SystemTime::UNIX_EPOCH)
        .is_ok_and(|changed| changed.as_secs() <= since)
}

/// Seconds since the epoch of a date in the form [`http_date`] writes (`Sun, 06 Nov 1994 08:49:37 GMT`); any other form is
/// not understood, and the file is sent.
pub(super) fn parse_http_date(text: &str) -> Option<u64> {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let mut parts = text.split_ascii_whitespace();
    parts.next()?.strip_suffix(',')?;
    let day: u64 = parts.next()?.parse().ok()?;
    let month_name = parts.next()?;
    let month = MONTHS.iter().position(|each| *each == month_name)? as u64 + 1;
    let year: u64 = parts.next()?.parse().ok()?;
    let mut clock = parts.next()?.split(':');
    let hour: u64 = clock.next()?.parse().ok()?;
    let minute: u64 = clock.next()?.parse().ok()?;
    let second: u64 = clock.next()?.parse().ok()?;
    if parts.next()? != "GMT" || parts.next().is_some() || clock.next().is_some() {
        return None;
    }
    if !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 60 || year < 1970 {
        return None;
    }
    crate::utc::unix_from_ymdhms(year, month, day, hour, minute, second)
}

/// Whether an `If-None-Match` value names `tag`, compared weakly as a `GET` is.
fn tag_matches(tags: &str, tag: &str) -> bool {
    let opaque = |tag: &str| tag.trim().trim_start_matches("W/").to_owned();
    tags.trim() == "*" || tags.split(',').any(|each| opaque(each) == opaque(tag))
}

pub(super) fn lookup(site: &Site, segments: &[String]) -> Lookup {
    match site {
        Site::Nothing => Lookup::Missing,
        Site::Dir(root) => lookup_dir(root, segments),
        #[cfg(feature = "web")]
        Site::BuiltIn => {
            let path = segments.join("/");
            match BUILT.binary_search_by(|(each, _, _)| (*each).cmp(path.as_str())) {
                Ok(at) => {
                    let (_, bytes, tag) = BUILT[at];
                    Lookup::Found(Found {
                        body: Body::Bytes(bytes),
                        tag: tag.to_owned(),
                        modified: None,
                    })
                }
                Err(_) => Lookup::Missing,
            }
        }
    }
}

/// The file `segments` name under `root`, once every link on the way is resolved; one that resolves outside `root` is
/// refused, whatever it is.
pub(crate) fn lookup_dir(root: &Path, segments: &[String]) -> Lookup {
    let mut path = root.to_path_buf();
    for segment in segments {
        path.push(segment);
    }
    let Ok(real) = std::fs::canonicalize(&path) else {
        return Lookup::Missing;
    };
    let Ok(inside) = real.strip_prefix(root) else {
        tracing::info!("web: a link out of the site was refused");
        return Lookup::Refused;
    };
    // What the name resolved to is held to the rules the request was: a link, or an NTFS short name such as GIT~1, can
    // lead to a dot-name the request could not have named.
    if !inside
        .components()
        .all(|part| part.as_os_str().to_str().is_some_and(plain_segment))
    {
        tracing::info!("web: a name that resolves to one never served was refused");
        return Lookup::Refused;
    }
    let Ok(file) = File::open(&real) else {
        return Lookup::Missing;
    };
    let Ok(metadata) = file.metadata() else {
        return Lookup::Missing;
    };
    if !metadata.is_file() {
        return Lookup::Missing;
    }
    let length = metadata.len();
    let modified = metadata.modified().ok();
    let nanos = modified
        .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map_or(0, |since| since.as_nanos());
    let Some(tag) = content_tag(&real, length, nanos) else {
        return Lookup::Missing;
    };
    Lookup::Found(Found {
        body: Body::File(file, length),
        tag,
        modified,
    })
}

/// The strong tag of the contents of the file at `real`: the first 128 bits of their SHA-256, read once while the file's
/// length and modification time stay as they were.
fn content_tag(real: &Path, length: u64, nanos: u128) -> Option<String> {
    use sha2::{Digest, Sha256};
    if let Some((was_length, was_nanos, tag)) = TAGS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(real)
        && (*was_length, *was_nanos) == (length, nanos)
    {
        return Some(tag.clone());
    }
    let mut file = File::open(real).ok()?;
    let mut hash = Sha256::new();
    let mut chunk = vec![0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut chunk).ok()?;
        if read == 0 {
            break;
        }
        hash.update(&chunk[..read]);
    }
    let digest = hash.finalize();
    let tag = format!("\"{}\"", data_encoding::HEXLOWER.encode(&digest[..16]));
    TAGS.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(real.to_path_buf(), (length, nanos, tag.clone()));
    Some(tag)
}
