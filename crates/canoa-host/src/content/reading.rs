//! What a catalog generator reads with: the registry, limited to the places the entry's `source` and the generator's own
//! `endpoints` name, so a generator never opens a connection or a file by itself.

use canoa::content::{ContentHost, Listing, Refusal, RefusalKind, Resolved, Stat, without_query};
use coqui_acp_api::guatiao::library::ProviderError;
use coqui_acp_api::guatiao::{Bytes, Status};

use super::{Content, shown};

/// How many entries one `list` of a generator returns at most.
const MOST_ENTRIES: u32 = 10_000;

/// One place a generator may read: a prefix of URIs.
#[derive(Clone, Debug)]
pub(super) struct Place {
    prefix: String,
    /// Whether the person wrote it (the entry's `source`), as opposed to a place the generator declares.
    written: bool,
}

impl Place {
    /// A place the generator declares.
    pub(super) fn declared(endpoint: &str) -> Self {
        Self {
            prefix: endpoint.to_owned(),
            written: false,
        }
    }

    /// The entry's `source`, which the person wrote: it and everything below it.
    pub(super) fn written(source: &str) -> Self {
        Self {
            prefix: without_query(source).to_owned(),
            written: true,
        }
    }

    fn reaches(&self, uri: &str) -> bool {
        let bare = without_query(uri);
        bare.starts_with(&self.prefix)
            && (self.prefix.ends_with('/')
                || bare.len() == self.prefix.len()
                || bare[self.prefix.len()..].starts_with('/'))
    }
}

/// The registry as a generator sees it.
pub(super) struct HostReach {
    content: Content,
    places: Vec<Place>,
}

impl HostReach {
    pub(super) const fn new(content: Content, places: Vec<Place>) -> Self {
        Self { content, places }
    }

    /// Whether the person wrote the place `uri` is in, or an error saying the generator may not read it.
    fn allowed(&self, uri: &str) -> Result<bool, ProviderError> {
        let bare = without_query(uri);
        if bare.split(['/', '\\']).any(|part| part == "..") {
            return Err(error(&Refusal::new(
                RefusalKind::RefusedPlace,
                format!("{} has a .. in its path", shown(uri)),
            )));
        }
        self.places
            .iter()
            .find(|place| place.reaches(uri))
            .map(|place| place.written)
            .ok_or_else(|| {
                error(&Refusal::new(
                    RefusalKind::RefusedPlace,
                    format!("{} is not a place this generator reads", shown(uri)),
                ))
            })
    }
}

/// A refusal as the error a generator's call answers with.
fn error(refusal: &Refusal) -> ProviderError {
    let status = match refusal.kind {
        RefusalKind::NotFound => Status::GUATIAO_ERR_NOT_FOUND,
        RefusalKind::TooLarge | RefusalKind::BadReply | RefusalKind::RefusedPlace => {
            Status::GUATIAO_ERR_BAD_VALUE
        }
        _ => Status::GUATIAO_ERR_INTERNAL,
    };
    ProviderError::new(status, &refusal.why)
}

impl ContentHost for HostReach {
    fn stat(&self, uri: &str) -> Result<Stat, ProviderError> {
        let person = self.allowed(uri)?;
        self.content.stat(uri, person).map_err(|why| error(&why))
    }

    fn list(&self, uri: &str, max_entries: u32) -> Result<Listing, ProviderError> {
        let person = self.allowed(uri)?;
        self.content
            .list(uri, person, max_entries.min(MOST_ENTRIES))
            .map_err(|why| error(&why))
    }

    fn open_text(&self, uri: &str, max_bytes: u64) -> Result<String, ProviderError> {
        let person = self.allowed(uri)?;
        self.content
            .open_text(uri, person, max_bytes)
            .map_err(|why| error(&why))
    }

    fn open_json(&self, uri: &str, max_bytes: u64) -> Result<String, ProviderError> {
        let text = self.open_text(uri, max_bytes)?;
        serde_json::from_str::<serde_json::Value>(&text).map_err(|_| {
            error(&Refusal::new(
                RefusalKind::BadReply,
                format!("{} is not JSON", shown(uri)),
            ))
        })?;
        Ok(text)
    }

    fn open_bytes(&self, uri: &str, max_bytes: u64) -> Result<Bytes, ProviderError> {
        let person = self.allowed(uri)?;
        self.content
            .open_bytes(uri, person, max_bytes)
            .map(Bytes)
            .map_err(|why| error(&why))
    }

    fn resolve(&self, uri: &str) -> Result<Resolved, ProviderError> {
        let person = self.allowed(uri)?;
        self.content.resolve(uri, person).map_err(|why| error(&why))
    }

    fn child(&self, base: &str, name: &str) -> Result<String, ProviderError> {
        canoa::content::child(base, name)
            .map_err(|why| ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, &why))
    }
}
