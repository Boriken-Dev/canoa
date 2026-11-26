//! The `https` scheme, and `http` for this computer's own loopback: a document at an address, fetched with the host's own client, so
//! the system's proxy, the operating system's certificate verifier and the person's network settings apply. Redirects are followed
//! only within the address's own origin.

use canoa::content::{
    ContentInfo, ContentSourceProvider, ListAnswer, ListRequest, OpenAnswer, OpenRequest, Opened,
    Refusal, RefusalKind, ResolveAnswer, SchemeInfo, Stat, StatAnswer, Target,
};
use coqui_acp_api::guatiao::Status;
use coqui_acp_api::guatiao::library::ProviderError;

use super::{failed, shown};
use crate::store::sources::Fetching;

/// The `https` provider.
pub(super) struct HttpsSource {
    fetching: Fetching,
}

impl HttpsSource {
    pub(super) const fn new(fetching: Fetching) -> Self {
        Self { fetching }
    }
}

/// `scheme://authority` of `url`.
fn origin_of(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    (!authority.is_empty()).then(|| format!("{}://{authority}", scheme.to_ascii_lowercase()))
}

/// What a failed fetch means of the place, from the words the fetch gives.
fn refusal_of(url: &str, error: &ProviderError) -> Refusal {
    let said = error.message();
    let kind = if said.contains("answered 404") || said.contains("answered 410") {
        RefusalKind::NotFound
    } else if said.contains("answered 401") || said.contains("answered 403") {
        RefusalKind::Denied
    } else if said.contains("answered 429") {
        RefusalKind::RateLimited
    } else if said.contains("larger than the limit") {
        RefusalKind::TooLarge
    } else if said.contains("not one of the") || said.contains("only https") {
        RefusalKind::RefusedPlace
    } else if said.contains("timed out") || said.contains("timeout") {
        RefusalKind::Timeout
    } else if said.contains("answered") || said.contains("too many redirects") {
        RefusalKind::BadReply
    } else {
        RefusalKind::Unreachable
    };
    Refusal::new(kind, format!("{}: {said}", shown(url)))
}

/// The bytes at `url`, reaching `endpoints` and no other place, at most `max_bytes`.
///
/// # Errors
///
/// Why the place gave none.
pub(super) fn get_bytes(
    fetching: &Fetching,
    url: &str,
    endpoints: Vec<String>,
    max_bytes: u64,
) -> Result<Vec<u8>, Refusal> {
    fetching
        .tools(endpoints)
        .loopback_http(true)
        .fetch_bytes(url, max_bytes)
        .map_err(|error| refusal_of(url, &error))
}

impl ContentSourceProvider for HttpsSource {
    fn describe(&self) -> Result<ContentInfo, ProviderError> {
        let mut info = ContentInfo::new("https", "Web addresses");
        info.help = Some(
            "A document at an https address (http for this computer's own loopback), over the host's own network settings."
                .to_owned(),
        );
        Ok(info
            .serving(SchemeInfo::new(
                "https",
                "A web address",
                "https://host/path?query",
            ))
            .serving(SchemeInfo::new(
                "http",
                "A web address on this computer",
                "http://localhost:port/path?query",
            )))
    }

    fn stat(&self, _target: Target) -> Result<StatAnswer, ProviderError> {
        Err(failed(
            Status::GUATIAO_ERR_UNSUPPORTED,
            "an address says what it holds only when it is read",
        ))
    }

    fn list(&self, _request: ListRequest) -> Result<ListAnswer, ProviderError> {
        Err(failed(
            Status::GUATIAO_ERR_UNSUPPORTED,
            "an address lists nothing: a page of links is not a listing",
        ))
    }

    fn open(&self, request: OpenRequest) -> Result<OpenAnswer, ProviderError> {
        let url = &request.target.uri;
        let Some(origin) = origin_of(url) else {
            return Ok(OpenAnswer::refused(Refusal::new(
                RefusalKind::RefusedPlace,
                format!("{} is not an address this host reads", shown(url)),
            )));
        };
        let limit = request.max_inline.min(request.max_bytes);
        Ok(match get_bytes(&self.fetching, url, vec![origin], limit) {
            Ok(bytes) => OpenAnswer::found(Opened::inline(Stat::file(), bytes)),
            Err(refusal) => OpenAnswer::refused(refusal),
        })
    }

    fn resolve(&self, _target: Target) -> Result<ResolveAnswer, ProviderError> {
        Err(failed(
            Status::GUATIAO_ERR_UNSUPPORTED,
            "an address has no revisions",
        ))
    }
}
