//! Headers, and the checks a request passes before anything is sent.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

use url::Url;

use super::error::Error;

/// The headers of a request or a reply: ordered, and names compare without regard to case. A name
/// sent more than once reads as its values joined with `", "` (RFC 9110 5.3); [`Headers::get_all`]
/// returns them separately.
///
/// `Debug` lists names only: a header can carry a credential.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Headers {
    pairs: Vec<(String, String)>,
}

impl fmt::Debug for Headers {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_list().entries(self.keys()).finish()
    }
}

impl Headers {
    /// No headers.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Headers from `(name, value)` pairs, in order.
    pub fn from_pairs<N: Into<String>, V: Into<String>>(
        pairs: impl IntoIterator<Item = (N, V)>,
    ) -> Self {
        Self {
            pairs: pairs
                .into_iter()
                .map(|(name, value)| (name.into(), value.into()))
                .collect(),
        }
    }

    /// The values of `name` joined with `", "`, `None` when it is not there.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<String> {
        let values = self.get_all(name);
        (!values.is_empty()).then(|| values.join(", "))
    }

    /// Every value sent for `name`, in order.
    #[must_use]
    pub fn get_all(&self, name: &str) -> Vec<&str> {
        self.pairs
            .iter()
            .filter(|(field, _)| field.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
            .collect()
    }

    /// Whether `name` is there.
    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.pairs
            .iter()
            .any(|(field, _)| field.eq_ignore_ascii_case(name))
    }

    /// Every `(name, value)` pair, in order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.pairs
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }

    /// The distinct names, each as first spelled.
    #[must_use]
    pub fn keys(&self) -> Vec<&str> {
        let mut seen: Vec<&str> = Vec::new();
        for (name, _) in &self.pairs {
            if !seen.iter().any(|known| known.eq_ignore_ascii_case(name)) {
                seen.push(name);
            }
        }
        seen
    }

    /// The number of distinct names.
    #[must_use]
    pub fn len(&self) -> usize {
        self.keys().len()
    }

    /// Whether there are no headers.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }

    /// Adds a value for `name`, keeping those already there.
    pub fn append(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.pairs.push((name.into(), value.into()));
    }

    /// Sets `name` to one value, replacing every value it had.
    pub fn insert(&mut self, name: impl Into<String>, value: impl Into<String>) {
        let name = name.into();
        self.remove(&name);
        self.pairs.push((name, value.into()));
    }

    /// Removes every value of `name`.
    pub fn remove(&mut self, name: &str) {
        self.pairs
            .retain(|(field, _)| !field.eq_ignore_ascii_case(name));
    }

    /// `layers` laid over one another, a later one winning on a name whatever its case; a value of
    /// `None` removes the name.
    pub(crate) fn merged<'a>(
        layers: impl IntoIterator<Item = &'a [(String, Option<String>)]>,
    ) -> Self {
        let mut merged = Self::new();
        for layer in layers {
            for (name, value) in layer {
                match value {
                    Some(value) => merged.insert(name.clone(), value.clone()),
                    None => merged.remove(name),
                }
            }
        }
        merged
    }
}

impl<N: Into<String>, V: Into<String>> FromIterator<(N, V)> for Headers {
    fn from_iter<T: IntoIterator<Item = (N, V)>>(iter: T) -> Self {
        Self::from_pairs(iter)
    }
}

/// Text from the other end (a reason phrase, a `Location`, a header value) as an error message
/// may show it: characters a terminal or a log would act on (controls, line and bidirectional
/// separators) escaped, and cut to `limit` characters.
#[must_use]
pub fn peer_text(text: &str, limit: usize) -> String {
    let cut = text.chars().count() > limit;
    let mut shown = String::new();
    for character in text.chars().take(limit) {
        let unprintable = character.is_control()
            || matches!(
                character,
                '\u{200B}'..='\u{200F}' | '\u{2028}'..='\u{202E}' | '\u{2060}'..='\u{206F}' | '\u{FEFF}'
            )
            || (character.is_whitespace() && character != ' ');
        if !unprintable {
            shown.push(character);
            continue;
        }
        match character {
            '\t' => shown.push_str("\\t"),
            '\n' => shown.push_str("\\n"),
            '\r' => shown.push_str("\\r"),
            other => {
                let code = u32::from(other);
                let escaped = if code < 0x100 {
                    alloc::format!("\\x{code:02x}")
                } else if code < 0x1_0000 {
                    alloc::format!("\\u{code:04x}")
                } else {
                    alloc::format!("\\U{code:08x}")
                };
                shown.push_str(&escaped);
            }
        }
    }
    if cut {
        shown.push_str("...");
    }
    shown
}

/// `url` as an error message may print it: scheme, host, port and path; no userinfo, query or
/// fragment, which can carry a credential.
#[must_use]
pub fn shown(url: &Url) -> String {
    let mut text = alloc::format!("{}://", url.scheme());
    if let Some(host) = url.host_str() {
        text.push_str(host);
    }
    if let Some(port) = url.port() {
        text.push(':');
        text.push_str(&port.to_string());
    }
    text.push_str(url.path());
    text
}

fn is_token(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
}

/// Refuses a header name that is not an HTTP token and a value with a control character (a line
/// break included) or text that is not Latin-1. The refused text is not quoted: it may be a
/// credential.
pub(crate) fn check_headers(headers: &Headers) -> Result<(), Error> {
    for (name, value) in headers.iter() {
        if !is_token(name) {
            return Err(Error::InvalidRequest(
                "a request header name is not an HTTP token".to_string(),
            ));
        }
        if value
            .chars()
            .any(|character| matches!(character, '\0'..='\u{8}' | '\n'..='\u{1f}' | '\u{7f}'))
        {
            return Err(Error::InvalidRequest(alloc::format!(
                "the value of the {name} header has a control character"
            )));
        }
        if value.chars().any(|character| u32::from(character) > 0xFF) {
            return Err(Error::InvalidRequest(alloc::format!(
                "the value of the {name} header is not Latin-1"
            )));
        }
    }
    Ok(())
}

/// The URL of a request: `http` or `https`, with a host, no credentials and no control character.
/// The fragment is dropped.
pub(crate) fn check_url(text: &str) -> Result<Url, Error> {
    if text.chars().any(|character| character.is_control()) {
        return Err(Error::InvalidRequest(
            "the request URL has a control character".to_string(),
        ));
    }
    let mut url = Url::parse(text)
        .map_err(|_| Error::InvalidRequest("the request URL cannot be parsed".to_string()))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(Error::InvalidRequest(alloc::format!(
            "the request URL has scheme {:?}, not http or https",
            url.scheme()
        )));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(Error::InvalidRequest(
            "the request URL carries credentials; send them in an Authorization header".to_string(),
        ));
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err(Error::InvalidRequest(
            "the request URL has no host".to_string(),
        ));
    }
    url.set_fragment(None);
    Ok(url)
}

/// Whether a reply is decoded for a request with these headers: unless the request names a content
/// coding of its own in `Accept-Encoding` (`identity` is not one, so a server that encodes anyway
/// is still decoded).
pub(crate) fn wants_decoding(headers: &Headers) -> bool {
    match headers.get("Accept-Encoding") {
        None => true,
        Some(value) => value.split(',').all(|part| {
            let token = part
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            token.is_empty() || token == "identity"
        }),
    }
}
