//! What a client does above a transport with credentials and redirects.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use coqui_proxy::Secret;
use percent_encoding::percent_decode_str;
use ureq::http::Method;
use url::Url;

use super::headers::{check_url, peer_text, shown};
use super::response::Response;

/// The headers a caller keeps a secret in, which a redirect to another scheme, host or port
/// leaves off, for that hop and every later one. The cookies of the client's jar are chosen again
/// for each hop.
pub(crate) const CREDENTIAL_HEADERS: [&str; 3] = ["Authorization", "Cookie", "Proxy-Authorization"];

/// Whether a method's redirects are followed. Every other method goes through a single resend:
/// following would turn a state-changing one into a different request, and a `PROPFIND` into a
/// `GET` or one without its body.
pub(crate) fn is_followed(method: &Method) -> bool {
    matches!(method.as_str(), "GET" | "HEAD" | "OPTIONS")
}

/// The redirect statuses after which a method is sent again, same method and body, to a
/// `Location` on the same scheme, host and port. `PROPFIND` only reads, so it is repeated for
/// every redirect; any other method only for 307 and 308, which keep the method and the body.
pub(crate) fn resent_statuses(method: &Method) -> &'static [u16] {
    if method.as_str() == "PROPFIND" {
        &[301, 302, 303, 307, 308]
    } else {
        &[307, 308]
    }
}

/// A reply's `Location` as an absolute `http:` or `https:` URL without userinfo, or `None` when
/// there is none or it cannot be used.
pub(crate) fn redirect_location(url: &Url, response: &Response) -> Option<Url> {
    let location = response.headers().get("Location")?;
    if location.is_empty() {
        return None;
    }
    // Header bytes are read as Latin-1; servers send UTF-8.
    let bytes: Option<Vec<u8>> = location
        .chars()
        .map(|character| u8::try_from(u32::from(character)).ok())
        .collect();
    let location = bytes
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or(location);
    if location.chars().any(|character| character.is_control()) {
        return None;
    }
    let mut absolute = url.join(&location).ok()?;
    let _ = absolute.set_username("");
    let _ = absolute.set_password(None);
    // What a transport would refuse is not a place to go.
    check_url(absolute.as_str()).ok()
}

/// The error for a redirect that is not followed.
pub(crate) fn refused(
    method: &Method,
    url: &Url,
    response: &Response,
    location: Option<&Url>,
    reason: &str,
    limit: usize,
) -> super::error::Error {
    let status = alloc::format!(
        "HTTP {} {}",
        response.status(),
        peer_text(response.reason(), limit)
    )
    .trim_end()
    .to_string();
    let target = location.map_or_else(|| "(none)".to_string(), |url| peer_text(&shown(url), limit));
    super::error::Error::Redirect(alloc::format!(
        "{status} for {method} {}: redirect to {target} not followed ({reason})",
        shown(url)
    ))
}

fn credential(text: &str) -> String {
    percent_decode_str(text).decode_utf8_lossy().into_owned()
}

/// Splits the userinfo out of an absolute URL: the URL without it, and the `(user, password)` it
/// held (percent-decoded), `None` when both are empty. Credentials inside a request URL would end
/// up in the reply's URL, in error text and in redirect handling; they are sent in an
/// `Authorization` header instead.
#[must_use]
pub fn split_userinfo(url: &str) -> (String, Option<(String, Secret)>) {
    let Some(scheme_end) = url.find("://").filter(|at| {
        let scheme = &url[..*at];
        scheme
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic())
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
    }) else {
        return (url.to_string(), None);
    };
    let start = scheme_end + 3;
    let end = url[start..]
        .find(['/', '?', '#'])
        .map_or(url.len(), |offset| start + offset);
    let authority = &url[start..end];
    let Some((userinfo, host)) = authority.rsplit_once('@') else {
        return (url.to_string(), None);
    };
    let (user, password) = userinfo.split_once(':').unwrap_or((userinfo, ""));
    let (user, password) = (credential(user), credential(password));
    let stripped = alloc::format!("{}{host}{}", &url[..start], &url[end..]);
    if user.is_empty() && password.is_empty() {
        (stripped, None)
    } else {
        (stripped, Some((user, Secret::new(password))))
    }
}
