//! The http and https addresses a person types: any is accepted, and one that leaves this computer over plain http is told of, never
//! refused.

use alloc::format;
use alloc::string::String;
use core::fmt;

use url::{Host, Url};

/// Why [`parse`] could not read text as an `http` or `https` address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The address holds whitespace or a control character.
    ControlOrWhitespace,
    /// The scheme is not `http` or `https`, or the text is not a URL.
    NotHttpOrHttps,
    /// The URL names no host.
    MissingHost,
    /// The URL holds user credentials.
    HasCredentials,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ControlOrWhitespace => {
                f.write_str("the address holds a space or a control character")
            }
            Self::NotHttpOrHttps => f.write_str("the address starts with https:// or http://"),
            Self::MissingHost => f.write_str("the address names no host"),
            Self::HasCredentials => f.write_str(
                "the address holds a login: a key goes in the key field, a password in the password field",
            ),
        }
    }
}

impl core::error::Error for Error {}

/// `address` as an `http` or `https` address with a host, no user information and no space or control character.
///
/// Plain `http` is accepted wherever the host is: it is the person's network, and [`plain_http_note`] says what it costs.
///
/// # Errors
///
/// Why it is not one, in words that never repeat the address.
pub fn parse(address: &str) -> Result<Url, Error> {
    if address.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(Error::ControlOrWhitespace);
    }
    let url = Url::parse(address).map_err(|error| match error {
        url::ParseError::EmptyHost | url::ParseError::InvalidDomainCharacter => Error::MissingHost,
        _ => Error::NotHttpOrHttps,
    })?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(Error::NotHttpOrHttps);
    }
    if url.host().is_none() {
        return Err(Error::MissingHost);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(Error::HasCredentials);
    }
    Ok(url)
}

/// Whether `url` names this computer: `localhost`, or an address of its loopback (`127.0.0.0/8`, `::1`).
#[must_use]
pub fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
        Some(Host::Ipv4(ip)) => ip.is_loopback(),
        Some(Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

/// Whether `url` is plain `http` to somewhere other than this computer: what is sent crosses the network in the clear.
#[must_use]
pub fn plain_beyond_loopback(url: &Url) -> bool {
    url.scheme() == "http" && !is_loopback(url)
}

/// What the person is told when `address` is plain `http` beyond this computer, and `None` for any other address, valid or not.
/// `what` names the setting, such as `account work: base_url`.
#[must_use]
pub fn plain_http_note(what: &str, address: &str) -> Option<String> {
    let url = parse(address).ok()?;
    plain_beyond_loopback(&url).then(|| {
        format!(
            "{what} is plain http to {}: what is sent there, a key or a password included, crosses the network unprotected",
            url.host_str().unwrap_or("another computer")
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn any_http_or_https_address_is_accepted() {
        for good in [
            "https://llm.example",
            "https://llm.example:8443/v1",
            "http://127.0.0.1:8080",
            "http://127.0.0.2/",
            "http://localhost/x",
            "http://[::1]:9",
            "http://192.0.2.11:4000",
            "http://llm.example",
            "HTTP://LLM.example/v1",
        ] {
            assert!(parse(good).is_ok(), "{good}");
        }
    }

    #[test]
    fn what_is_no_address_is_still_refused() {
        for bad in [
            "ftp://llm.example",
            "llm.example",
            "https://",
            "https://user@llm.example",
            "https://user:pw@llm.example",
            "http://llm .example",
            "http://llm.example/\n",
            "file:///etc/passwd",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn plain_http_is_told_of_off_this_computer_only() {
        for quiet in [
            "https://llm.example",
            "http://127.0.0.1:8080",
            "http://127.1.2.3",
            "http://localhost",
            "http://LOCALHOST:1",
            "http://[::1]:9",
            "ftp://llm.example",
        ] {
            assert_eq!(plain_http_note("x", quiet), None, "{quiet}");
        }
        for loud in [
            "http://192.0.2.11:4000",
            "http://llm.example",
            "http://127.0.0.1.example.com",
            "http://localhost.evil.test",
        ] {
            let note = plain_http_note("base_url", loud).expect(loud);
            assert!(note.starts_with("base_url is plain http to "), "{note}");
            assert!(!note.contains("://"), "{note}");
        }
    }
}
