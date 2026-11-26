//! What a transport starts from: a request that passed the checks every transport holds itself to.

use alloc::boxed::Box;
use alloc::string::ToString;
use core::time::Duration;

use ureq::http::Method;
use url::Url;

use super::body::{Body, Exact};
use super::error::Error;
use super::headers::{Headers, check_headers, check_url, wants_decoding};

/// How long a request may wait: to connect, and at each step of the reply. `None` waits without
/// limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timeouts {
    /// The time to connect.
    pub connect: Option<Duration>,
    /// The time to wait for the server at each step of the reply.
    pub read: Option<Duration>,
}

impl Timeouts {
    /// One limit for both waits.
    #[must_use]
    pub fn both(limit: Option<Duration>) -> Self {
        Self {
            connect: limit,
            read: limit,
        }
    }

    /// No limit on either wait.
    #[must_use]
    pub fn unlimited() -> Self {
        Self::both(None)
    }

    pub(crate) fn check(self) -> Result<Self, Error> {
        if self.connect.is_some_and(|limit| limit.is_zero())
            || self.read.is_some_and(|limit| limit.is_zero())
        {
            return Err(Error::InvalidRequest(
                "a timeout is a positive duration, or none".to_string(),
            ));
        }
        Ok(self)
    }
}

/// A request that passed the checks of the transport contract.
///
/// `headers` hold text values, with the `Content-Length` of a body whose length is known (`0` for
/// a `PATCH`, `POST` or `PUT` without a body) and an `Accept-Encoding` of `gzip, deflate` unless
/// the caller named one. `decode` says whether the reply is decoded when it is `gzip` or `deflate`
/// encoded.
#[derive(Debug)]
#[non_exhaustive]
pub struct PreparedRequest {
    /// The method.
    pub method: Method,
    /// The URL, without a fragment.
    pub url: Url,
    /// The headers to send.
    pub headers: Headers,
    /// The body to send.
    pub body: Body,
    /// The waits.
    pub timeout: Timeouts,
    /// Whether a `gzip` or `deflate` reply is decoded.
    pub decode: bool,
}

/// The `Content-Length` the caller set.
fn declared_length(headers: &Headers) -> Result<Option<u64>, Error> {
    let found = headers.get_all("Content-Length");
    let not_a_count =
        || Error::InvalidRequest("the Content-Length header is not one count of bytes".to_string());
    match found.as_slice() {
        [] => Ok(None),
        [value] => {
            let value = value.trim();
            if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(not_a_count());
            }
            value.parse().map(Some).map_err(|_| not_a_count())
        }
        _ => Err(not_a_count()),
    }
}

impl PreparedRequest {
    /// The arguments of a send as the contract reads them.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidRequest`], before anything is sent, for a header name that is not an HTTP
    /// token, a header value or URL with a control character, a URL with credentials or another
    /// scheme, a `Content-Length` that is not a count of bytes or exceeds the body, and a timeout
    /// of zero. No message quotes the URL or a header value.
    pub fn prepare(
        method: Method,
        url: &str,
        headers: Headers,
        body: Body,
        timeout: Timeouts,
    ) -> Result<Self, Error> {
        let url = check_url(url)?;
        check_headers(&headers)?;
        let timeout = timeout.check()?;
        let declared = declared_length(&headers)?;
        let mut headers = headers;
        let too_long = || {
            Error::InvalidRequest("the Content-Length header is larger than the body".to_string())
        };
        let body = match body {
            Body::Empty => Body::Empty,
            Body::Bytes(mut bytes) => {
                match declared {
                    Some(length) if length > bytes.len() as u64 => return Err(too_long()),
                    Some(length) => bytes.truncate(usize::try_from(length).unwrap_or(usize::MAX)),
                    None => {}
                }
                Body::Bytes(bytes)
            }
            Body::Text(text) => {
                let mut bytes = text.into_bytes();
                match declared {
                    Some(length) if length > bytes.len() as u64 => return Err(too_long()),
                    Some(length) => bytes.truncate(usize::try_from(length).unwrap_or(usize::MAX)),
                    None => {}
                }
                Body::Bytes(bytes)
            }
            streamed @ (Body::Reader(_) | Body::Chunks(_)) => match declared {
                // A length the caller set is trusted: nothing is chunked and exactly that many
                // bytes are sent.
                Some(length) => Body::Reader(Box::new(Exact::new(streamed.into_reader(), length))),
                None => streamed,
            },
        };
        if declared.is_none() {
            match body.len() {
                Some(0) if matches!(method, Method::POST | Method::PUT | Method::PATCH) => {
                    headers.insert("Content-Length", "0");
                }
                Some(length) if length > 0 => headers.insert("Content-Length", length.to_string()),
                _ => {}
            }
        }
        let decode = wants_decoding(&headers);
        if !headers.contains("Accept-Encoding") {
            headers.insert("Accept-Encoding", "gzip, deflate");
        }
        Ok(Self {
            method,
            url,
            headers,
            body,
            timeout,
            decode,
        })
    }
}
