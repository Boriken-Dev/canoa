//! The `git+https`, `git+ssh` and `git+file` schemes: a file or a folder of a git repository at a branch, a tag or a commit,
//! `git+https://host/owner/repo.git!/path/in/the/repo?ref=REV`. With no `ref` it is the default branch, so what is read is what the
//! repository serves now; a `ref` that is a full commit id never changes.
//!
//! It runs the isolated git the overlays use (no credential helper, none of the person's git configuration) for an address a
//! catalog document named, and as the person for one the person wrote.

use std::path::Path;
use std::sync::Arc;

use canoa::content::{
    ContentInfo, ContentSourceProvider, Digest, EntryKind, ListAnswer, ListEntry, ListRequest,
    Listing, OpenAnswer, OpenRequest, Opened, Refusal, RefusalKind, ResolveAnswer, Resolved,
    SchemeInfo, Stat, StatAnswer, Target, query_param, without_query,
};
use coqui_acp_api::guatiao::library::ProviderError;

use super::shown;
use crate::control::{Fetched, GitNode, OverlayLimits, Overlays, Trust};

/// The three git schemes.
const SCHEMES: [(&str, &str); 3] = [
    ("git+https", "A git repository over https"),
    ("git+ssh", "A git repository over ssh"),
    ("git+file", "A git repository on this computer"),
];

/// The git provider.
pub(super) struct GitSource {
    overlays: Option<Arc<Overlays>>,
}

impl GitSource {
    pub(super) fn new(root: Option<&Path>) -> Self {
        Self {
            overlays: root.map(|root| {
                Arc::new(Overlays::new(
                    Some(root.to_path_buf()),
                    OverlayLimits::default(),
                ))
            }),
        }
    }
}

/// What a `git+` URI names.
struct Place {
    /// The repository's address as git takes it (`https://host/owner/repo.git`).
    repo: String,
    /// The path inside the repository, without slashes at its ends.
    path: String,
    /// The branch, tag or commit; none is the default branch.
    reference: Option<String>,
}

fn place(uri: &str) -> Result<Place, Refusal> {
    let bad = |why: &str| Refusal::new(RefusalKind::RefusedPlace, format!("{}: {why}", shown(uri)));
    let rest = uri
        .strip_prefix("git+")
        .or_else(|| {
            uri.get(4..)
                .filter(|_| uri[..4].eq_ignore_ascii_case("git+"))
        })
        .ok_or_else(|| bad("a git address starts with git+"))?;
    let reference = query_param(rest, "ref")
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let location = without_query(rest);
    let (repo, path) = location
        .split_once("!/")
        .map_or((location, ""), |(repo, path)| (repo, path));
    if repo.is_empty() || !repo.contains("://") {
        return Err(bad("the repository is an address with a scheme"));
    }
    let path = path.trim_matches('/');
    if path.split('/').any(|part| part == "..") {
        return Err(bad("the path goes above the repository"));
    }
    let path = percent_encoding::percent_decode_str(path)
        .decode_utf8()
        .map_err(|_| bad("the path is not UTF-8"))?
        .into_owned();
    Ok(Place {
        repo: repo.to_owned(),
        path,
        reference,
    })
}

fn refusal_of(uri: &str, (code, why): &(i32, String)) -> Refusal {
    let kind = if *code == crate::control::NOT_FOUND
        || why.contains("no branch or tag")
        || why.contains("no commit")
        || why.contains("no default branch")
    {
        RefusalKind::NotFound
    } else if why.contains("Permission denied") || why.contains("Authentication failed") {
        RefusalKind::Denied
    } else {
        RefusalKind::Unreachable
    };
    Refusal::new(kind, format!("{}: {why}", shown(uri)))
}

impl GitSource {
    fn overlays(&self, uri: &str) -> Result<&Overlays, Refusal> {
        self.overlays.as_deref().ok_or_else(|| {
            Refusal::new(
                RefusalKind::Unreachable,
                format!(
                    "{}: this host keeps no folder to fetch repositories into",
                    shown(uri)
                ),
            )
        })
    }

    fn fetched(&self, target: &Target, place: &Place) -> Result<Fetched, Refusal> {
        let trust = if target.person {
            Trust::Own
        } else {
            Trust::Isolated
        };
        self.overlays(&target.uri)?
            .content_fetch(trust, &place.repo, place.reference.as_deref())
            .map_err(|failure| refusal_of(&target.uri, &failure))
    }

    fn immutable(place: &Place) -> bool {
        place
            .reference
            .as_deref()
            .is_some_and(crate::profile::full_commit_id)
    }

    fn file_stat(place: &Place, fetched: &Fetched, id: &str, size: u64) -> Stat {
        let mut stat = Stat::file().sized(size);
        stat.etag = Some(id.to_owned());
        stat.revision = Some(fetched.commit.clone());
        stat.immutable = Self::immutable(place);
        stat.digests.push(Digest::published("git_blob", id));
        stat
    }
}

impl ContentSourceProvider for GitSource {
    fn describe(&self) -> Result<ContentInfo, ProviderError> {
        let mut info = ContentInfo::new("git", "Git repositories");
        info.help = Some(
            "A file or folder of a git repository at a branch, a tag or a commit; with no ref, the default branch."
                .to_owned(),
        );
        for (name, title) in SCHEMES {
            info = info.serving(
                SchemeInfo::new(
                    name,
                    title,
                    format!("{name}://host/owner/repo.git[!/path][?ref=REV]"),
                )
                .listing()
                .resolving(),
            );
        }
        Ok(info)
    }

    fn stat(&self, target: Target) -> Result<StatAnswer, ProviderError> {
        let found = (|| {
            let place = place(&target.uri)?;
            let fetched = self.fetched(&target, &place)?;
            let node = self
                .overlays(&target.uri)?
                .content_node(&fetched, &place.path, false)
                .map_err(|failure| refusal_of(&target.uri, &failure))?;
            Ok::<_, Refusal>(match node {
                GitNode::File { id, size } => Self::file_stat(&place, &fetched, &id, size),
                GitNode::Dir(_) => {
                    let mut stat = Stat::dir();
                    stat.revision = Some(fetched.commit.clone());
                    stat.immutable = Self::immutable(&place);
                    stat
                }
            })
        })();
        Ok(match found {
            Ok(stat) => StatAnswer::found(stat),
            Err(refusal) => StatAnswer::refused(refusal),
        })
    }

    fn list(&self, request: ListRequest) -> Result<ListAnswer, ProviderError> {
        let target = &request.target;
        let found = (|| {
            let place = place(&target.uri)?;
            let fetched = self.fetched(target, &place)?;
            let node = self
                .overlays(&target.uri)?
                .content_node(&fetched, &place.path, request.recursive)
                .map_err(|failure| refusal_of(&target.uri, &failure))?;
            let GitNode::Dir(mut entries) = node else {
                return Err(Refusal::new(
                    RefusalKind::NotFound,
                    format!("{} is a file, not a folder", shown(&target.uri)),
                ));
            };
            entries.sort_by(|a, b| a.path.cmp(&b.path));
            let start = request
                .cursor
                .as_deref()
                .and_then(|cursor| cursor.parse::<usize>().ok())
                .unwrap_or(0)
                .min(entries.len());
            let page = usize::try_from(request.max_entries)
                .unwrap_or(usize::MAX)
                .max(1);
            let end = start.saturating_add(page).min(entries.len());
            let mut listing = Listing::of(
                entries[start..end]
                    .iter()
                    .map(|entry| {
                        let mut each = ListEntry::new(
                            entry.path.clone(),
                            if entry.dir {
                                EntryKind::Dir
                            } else {
                                EntryKind::File
                            },
                        );
                        let mut stat = if entry.dir { Stat::dir() } else { Stat::file() };
                        stat.size = entry.size;
                        stat.revision = Some(fetched.commit.clone());
                        each.stat = Some(stat);
                        each
                    })
                    .collect(),
            );
            listing.next = (end < entries.len()).then(|| end.to_string());
            Ok(listing)
        })();
        Ok(match found {
            Ok(listing) => ListAnswer::found(listing),
            Err(refusal) => ListAnswer::refused(refusal),
        })
    }

    fn open(&self, request: OpenRequest) -> Result<OpenAnswer, ProviderError> {
        let target = &request.target;
        let found = (|| {
            let place = place(&target.uri)?;
            let fetched = self.fetched(target, &place)?;
            let overlays = self.overlays(&target.uri)?;
            let node = overlays
                .content_node(&fetched, &place.path, false)
                .map_err(|failure| refusal_of(&target.uri, &failure))?;
            let GitNode::File { id, size } = node else {
                return Err(Refusal::new(
                    RefusalKind::NotFound,
                    format!("{} is a folder, not a file", shown(&target.uri)),
                ));
            };
            let stat = Self::file_stat(&place, &fetched, &id, size);
            if request.if_none_match.as_deref() == Some(id.as_str()) {
                return Ok(Opened::new(stat, canoa::content::Delivery::not_modified()));
            }
            let limit = request.max_inline.min(request.max_bytes);
            if size > limit {
                return Err(Refusal::new(
                    RefusalKind::TooLarge,
                    format!(
                        "{} holds {size} bytes, more than the {limit} this read takes",
                        shown(&target.uri)
                    ),
                ));
            }
            let bytes = overlays
                .content_blob(&fetched, &id, limit)
                .map_err(|failure| refusal_of(&target.uri, &failure))?;
            Ok(Opened::inline(stat, bytes))
        })();
        Ok(match found {
            Ok(opened) => OpenAnswer::found(opened),
            Err(refusal) => OpenAnswer::refused(refusal),
        })
    }

    fn resolve(&self, target: Target) -> Result<ResolveAnswer, ProviderError> {
        let found = (|| {
            let place = place(&target.uri)?;
            let trust = if target.person {
                Trust::Own
            } else {
                Trust::Isolated
            };
            let commit = self
                .overlays(&target.uri)?
                .content_commit(trust, &place.repo, place.reference.as_deref())
                .map_err(|failure| refusal_of(&target.uri, &failure))?;
            // The same place with the revision fixed: the `ref` becomes the commit.
            let (head, query) = target
                .uri
                .split_once('?')
                .map_or((target.uri.as_str(), ""), |(head, query)| (head, query));
            let mut query: Vec<String> = query
                .split('&')
                .filter(|pair| !pair.is_empty() && !pair.starts_with("ref="))
                .map(str::to_owned)
                .collect();
            query.push(format!("ref={commit}"));
            Ok::<_, Refusal>(Resolved::new(
                format!("{head}?{}", query.join("&")),
                commit,
                true,
            ))
        })();
        Ok(match found {
            Ok(resolved) => ResolveAnswer::found(resolved),
            Err(refusal) => ResolveAnswer::refused(refusal),
        })
    }
}
