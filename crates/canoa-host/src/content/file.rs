//! The `file` scheme: files and folders on this computer, by the person's own reach. A string with no scheme is one of them.
//!
//! Only the person names a file: a URI a catalog document names (`person` false) is refused, since a third party's document has
//! no business pointing the host at this computer's files.

use std::path::PathBuf;
use std::time::UNIX_EPOCH;

use canoa::content::{
    ContentInfo, ContentSourceProvider, EntryKind, ListAnswer, ListEntry, ListRequest, Listing,
    OpenAnswer, OpenRequest, Opened, Refusal, RefusalKind, ResolveAnswer, SchemeInfo, Stat,
    StatAnswer, Target,
};
use coqui_acp_api::guatiao::Status;
use coqui_acp_api::guatiao::library::ProviderError;

use super::failed;

/// The `file` provider.
pub(super) struct FileSource;

/// The path a `file` URI or a plain path names.
///
/// # Errors
///
/// Why the text names no path on this computer.
pub(super) fn path_of(uri: &str) -> Result<PathBuf, Refusal> {
    let bad = |why: &str| Refusal::new(RefusalKind::RefusedPlace, format!("{uri}: {why}"));
    let Some(rest) = uri
        .get(..5)
        .filter(|head| head.eq_ignore_ascii_case("file:"))
        .map(|_| &uri[5..])
    else {
        return Ok(PathBuf::from(uri));
    };
    let without_query = rest.split(['?', '#']).next().unwrap_or(rest);
    let decode = |text: &str| {
        percent_encoding::percent_decode_str(text)
            .decode_utf8()
            .map(std::borrow::Cow::into_owned)
            .map_err(|_| bad("the path is not UTF-8"))
    };
    let Some(after) = without_query.strip_prefix("//") else {
        return Ok(PathBuf::from(decode(without_query)?));
    };
    let (host, path) = after
        .find('/')
        .map_or((after, ""), |at| (&after[..at], &after[at..]));
    let path = decode(path)?;
    if host.is_empty() || host.eq_ignore_ascii_case("localhost") {
        // `/C:/x` is the drive letter's path on Windows.
        let bytes = path.as_bytes();
        let drive = bytes.len() >= 3
            && bytes[0] == b'/'
            && bytes[1].is_ascii_alphabetic()
            && bytes[2] == b':';
        return Ok(PathBuf::from(if drive { &path[1..] } else { &path }));
    }
    if cfg!(windows) {
        Ok(PathBuf::from(format!(
            "\\\\{host}{}",
            path.replace('/', "\\")
        )))
    } else {
        Err(bad("a file on another computer is not read"))
    }
}

fn io_refusal(uri: &str, error: &std::io::Error) -> Refusal {
    let kind = match error.kind() {
        std::io::ErrorKind::NotFound => RefusalKind::NotFound,
        std::io::ErrorKind::PermissionDenied => RefusalKind::Denied,
        _ => RefusalKind::Unreachable,
    };
    Refusal::new(kind, format!("{uri}: {error}"))
}

fn stat_of(meta: &std::fs::Metadata) -> Stat {
    let mut stat = if meta.is_dir() {
        Stat::dir()
    } else {
        Stat::file()
    };
    if meta.is_file() {
        stat.size = Some(meta.len());
    }
    stat.modified_ms = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .and_then(|since| i64::try_from(since.as_millis()).ok());
    if meta.is_file() {
        stat.etag = Some(format!("{}-{}", meta.len(), stat.modified_ms.unwrap_or(0)));
    }
    stat
}

fn named_by_the_person(target: &Target) -> Result<PathBuf, Refusal> {
    if !target.person {
        return Err(Refusal::new(
            RefusalKind::RefusedPlace,
            "a file on this computer is named by the person, not by a catalog".to_owned(),
        ));
    }
    path_of(&target.uri)
}

impl ContentSourceProvider for FileSource {
    fn describe(&self) -> Result<ContentInfo, ProviderError> {
        let mut info = ContentInfo::new("file", "Files on this computer");
        info.help = Some(
            "A file or folder on this computer, as the person's own account reads it.".to_owned(),
        );
        Ok(info.serving(
            SchemeInfo::new(
                "file",
                "This computer",
                "file:///abs/path, D:/folder/file or /home/me/file",
            )
            .listing(),
        ))
    }

    fn stat(&self, target: Target) -> Result<StatAnswer, ProviderError> {
        let path = match named_by_the_person(&target) {
            Ok(path) => path,
            Err(refusal) => return Ok(StatAnswer::refused(refusal)),
        };
        Ok(match std::fs::metadata(&path) {
            Ok(meta) => StatAnswer::found(stat_of(&meta)),
            Err(error) => StatAnswer::refused(io_refusal(&target.uri, &error)),
        })
    }

    fn list(&self, request: ListRequest) -> Result<ListAnswer, ProviderError> {
        let path = match named_by_the_person(&request.target) {
            Ok(path) => path,
            Err(refusal) => return Ok(ListAnswer::refused(refusal)),
        };
        let uri = &request.target.uri;
        let mut found = Vec::new();
        let mut stack = vec![(path.clone(), String::new())];
        while let Some((folder, prefix)) = stack.pop() {
            let reader = match std::fs::read_dir(&folder) {
                Ok(reader) => reader,
                Err(error) => return Ok(ListAnswer::refused(io_refusal(uri, &error))),
            };
            for entry in reader.flatten() {
                let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                    continue;
                };
                let Ok(meta) = entry.metadata() else {
                    continue;
                };
                let relative = format!("{prefix}{name}");
                let kind = if meta.is_dir() {
                    EntryKind::Dir
                } else {
                    EntryKind::File
                };
                if meta.is_dir() && request.recursive {
                    stack.push((entry.path(), format!("{relative}/")));
                }
                let mut each = ListEntry::new(relative, kind);
                each.stat = Some(stat_of(&meta));
                found.push(each);
            }
        }
        found.sort_by(|a, b| a.path.cmp(&b.path));
        let start = request
            .cursor
            .as_deref()
            .and_then(|cursor| cursor.parse::<usize>().ok())
            .unwrap_or(0)
            .min(found.len());
        let page = usize::try_from(request.max_entries)
            .unwrap_or(usize::MAX)
            .max(1);
        let end = start.saturating_add(page).min(found.len());
        let next = (end < found.len()).then(|| end.to_string());
        let mut listing = Listing::of(found[start..end].to_vec());
        listing.next = next;
        Ok(ListAnswer::found(listing))
    }

    fn open(&self, request: OpenRequest) -> Result<OpenAnswer, ProviderError> {
        let path = match named_by_the_person(&request.target) {
            Ok(path) => path,
            Err(refusal) => return Ok(OpenAnswer::refused(refusal)),
        };
        let uri = &request.target.uri;
        let meta = match std::fs::metadata(&path) {
            Ok(meta) => meta,
            Err(error) => return Ok(OpenAnswer::refused(io_refusal(uri, &error))),
        };
        if !meta.is_file() {
            return Ok(OpenAnswer::refused(Refusal::new(
                RefusalKind::NotFound,
                format!("{uri} is not a file"),
            )));
        }
        let stat = stat_of(&meta);
        if request.if_none_match.is_some() && request.if_none_match == stat.etag {
            return Ok(OpenAnswer::found(Opened::new(
                stat,
                canoa::content::Delivery::not_modified(),
            )));
        }
        if meta.len() > request.max_inline.min(request.max_bytes) {
            return Ok(OpenAnswer::refused(Refusal::new(
                RefusalKind::TooLarge,
                format!(
                    "{uri} holds {} bytes, more than the {} this read takes",
                    meta.len(),
                    request.max_inline.min(request.max_bytes)
                ),
            )));
        }
        match std::fs::read(&path) {
            Ok(bytes) => Ok(OpenAnswer::found(Opened::inline(stat, bytes))),
            Err(error) => Ok(OpenAnswer::refused(io_refusal(uri, &error))),
        }
    }

    fn resolve(&self, _target: Target) -> Result<ResolveAnswer, ProviderError> {
        Err(failed(
            Status::GUATIAO_ERR_UNSUPPORTED,
            "a file on this computer has no revisions",
        ))
    }
}
