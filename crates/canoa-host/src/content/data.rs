//! The `data` scheme (RFC 2397): content written in the URI itself.

use canoa::content::{
    ContentInfo, ContentSourceProvider, Digest, ListAnswer, ListRequest, OpenAnswer, OpenRequest,
    Opened, Refusal, RefusalKind, ResolveAnswer, SchemeInfo, Stat, StatAnswer, Target,
};
use coqui_acp_api::guatiao::Status;
use coqui_acp_api::guatiao::library::ProviderError;
use sha2::{Digest as _, Sha256};

use super::failed;

/// The `data` provider.
pub(super) struct DataSource;

/// The media type and bytes a `data:` URI holds.
fn decode(uri: &str) -> Result<(String, Vec<u8>), Refusal> {
    let bad = |why: &str| Refusal::new(RefusalKind::BadReply, format!("a data: URI {why}"));
    let rest = uri
        .get(..5)
        .filter(|head| head.eq_ignore_ascii_case("data:"))
        .map(|_| &uri[5..])
        .ok_or_else(|| bad("starts with data:"))?;
    let (head, body) = rest
        .split_once(',')
        .ok_or_else(|| bad("has a comma before its data"))?;
    let mut parts = head.split(';');
    let media = parts.next().unwrap_or_default();
    let base64 = parts.any(|part| part.eq_ignore_ascii_case("base64"));
    let bytes = if base64 {
        let clean: String = percent_encoding::percent_decode_str(body)
            .decode_utf8_lossy()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        data_encoding::BASE64
            .decode(clean.as_bytes())
            .or_else(|_| data_encoding::BASE64_NOPAD.decode(clean.as_bytes()))
            .map_err(|_| bad("holds base64 that does not decode"))?
    } else {
        percent_encoding::percent_decode_str(body).collect()
    };
    let media = if media.is_empty() {
        "text/plain"
    } else {
        media
    };
    Ok((media.to_owned(), bytes))
}

fn stat_of(media: String, bytes: &[u8]) -> Stat {
    let mut stat = Stat::file().sized(bytes.len() as u64);
    stat.content_type = Some(media);
    stat.immutable = true;
    stat.digests.push(Digest::derived(
        "sha256",
        data_encoding::HEXLOWER.encode(&Sha256::digest(bytes)),
    ));
    stat
}

impl ContentSourceProvider for DataSource {
    fn describe(&self) -> Result<ContentInfo, ProviderError> {
        let mut info = ContentInfo::new("data", "Data in the address");
        info.help = Some("Content written in the URI itself (RFC 2397).".to_owned());
        Ok(info.serving(SchemeInfo::new(
            "data",
            "The URI itself",
            "data:[<media type>][;base64],<data>",
        )))
    }

    fn stat(&self, target: Target) -> Result<StatAnswer, ProviderError> {
        Ok(match decode(&target.uri) {
            Ok((media, bytes)) => StatAnswer::found(stat_of(media, &bytes)),
            Err(refusal) => StatAnswer::refused(refusal),
        })
    }

    fn list(&self, _request: ListRequest) -> Result<ListAnswer, ProviderError> {
        Err(failed(
            Status::GUATIAO_ERR_UNSUPPORTED,
            "a data: URI holds one file",
        ))
    }

    fn open(&self, request: OpenRequest) -> Result<OpenAnswer, ProviderError> {
        let (media, bytes) = match decode(&request.target.uri) {
            Ok(decoded) => decoded,
            Err(refusal) => return Ok(OpenAnswer::refused(refusal)),
        };
        let limit = request.max_inline.min(request.max_bytes);
        if bytes.len() as u64 > limit {
            return Ok(OpenAnswer::refused(Refusal::new(
                RefusalKind::TooLarge,
                format!(
                    "a data: URI holds {} bytes, more than the {limit} this read takes",
                    bytes.len()
                ),
            )));
        }
        let stat = stat_of(media, &bytes);
        Ok(OpenAnswer::found(Opened::inline(stat, bytes)))
    }

    fn resolve(&self, _target: Target) -> Result<ResolveAnswer, ProviderError> {
        Err(failed(
            Status::GUATIAO_ERR_UNSUPPORTED,
            "a data: URI is already fixed",
        ))
    }
}
