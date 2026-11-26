//! The reply every transport returns, with the decoding they share.

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;
use std::io::{self, Read};

use url::Url;

use super::body::Body;
use super::decode::decoder_for;
use super::error::Error;
use super::headers::{Headers, shown};

/// Most bytes read from a transport's body in one call.
const CHUNK: usize = 64 * 1024;

/// What a text decode does with bytes that are not text in its charset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum TextErrors {
    /// Replace each with U+FFFD (the default).
    #[default]
    Replace,
    /// Fail the decode.
    Strict,
    /// Drop them.
    Ignore,
}

enum Source {
    Memory { data: Vec<u8>, at: usize },
    Stream(Box<dyn Read + Send>),
}

/// One reply: [`status`](Response::status), [`reason`](Response::reason),
/// [`headers`](Response::headers), [`url`](Response::url) (the URL the request went to) and the
/// body, read with [`Read`], [`Response::chunks`] or [`Response::content`]. Dropping it releases the
/// connection.
///
/// The body is decoded when the reply says it is `gzip` or `deflate` encoded, unless the request
/// set its own `Accept-Encoding`; `headers` are as received, so `Content-Length` counts the
/// encoded bytes. A body that ends before the length the server stated is an error, never a
/// shorter body that reads as complete.
pub struct Response {
    status: u16,
    reason: String,
    headers: Headers,
    url: Url,
    source: Source,
    content: Option<Vec<u8>>,
    max_reply_bytes: usize,
}

impl fmt::Debug for Response {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Response")
            .field("status", &self.status)
            .field("reason", &self.reason)
            .field("url", &shown(&self.url))
            .field("headers", &self.headers)
            .finish_non_exhaustive()
    }
}

/// Makes a [`Response`]: what a transport does with what it read.
#[derive(Debug)]
#[must_use]
pub struct ResponseBuilder {
    status: u16,
    reason: String,
    headers: Headers,
    url: Option<Url>,
    body: Body,
    decode: bool,
    max_reply_bytes: usize,
    decode_chunk: usize,
}

impl ResponseBuilder {
    /// A reply with this status, no headers and no body.
    pub fn new(status: u16) -> Self {
        Self {
            status,
            reason: String::new(),
            headers: Headers::new(),
            url: None,
            body: Body::Empty,
            decode: true,
            max_reply_bytes: 8 * 1024 * 1024,
            decode_chunk: 256 * 1024,
        }
    }

    /// The reason phrase.
    pub fn reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = reason.into();
        self
    }

    /// The headers as received.
    pub fn headers(mut self, headers: Headers) -> Self {
        self.headers = headers;
        self
    }

    /// The URL the request went to.
    pub fn url(mut self, url: Url) -> Self {
        self.url = Some(url);
        self
    }

    /// The body: bytes, or a reader whose end is the body's end.
    pub fn body(mut self, body: impl Into<Body>) -> Self {
        self.body = body.into();
        self
    }

    /// Whether a `gzip` or `deflate` body is decoded (true); false for a body the transport
    /// already decoded or did not ask a coding for.
    pub fn decode(mut self, decode: bool) -> Self {
        self.decode = decode;
        self
    }

    /// Most bytes of the decoded body [`Response::content`] reads whole (8 MiB).
    pub fn max_reply_bytes(mut self, limit: usize) -> Self {
        self.max_reply_bytes = limit;
        self
    }

    /// Most decoded bytes one decompression step produces (256 KiB).
    pub fn decode_chunk(mut self, chunk: usize) -> Self {
        self.decode_chunk = chunk;
        self
    }

    /// The reply.
    pub fn build(self) -> Response {
        let url = self
            .url
            .unwrap_or_else(|| Url::parse("http://invalid.invalid/").expect("a constant URL"));
        let source = match self.body {
            Body::Empty => Source::Memory {
                data: Vec::new(),
                at: 0,
            },
            Body::Bytes(data) => Source::Memory { data, at: 0 },
            Body::Text(text) => Source::Memory {
                data: text.into_bytes(),
                at: 0,
            },
            streamed => Source::Stream(streamed.into_reader()),
        };
        let source = match source {
            Source::Stream(reader) if self.decode => {
                Source::Stream(decoder_for(&self.headers, reader, self.decode_chunk))
            }
            // A body held in memory is decoded when read, as a streamed one is.
            Source::Memory { data, at: 0 }
                if self.decode && self.headers.contains("Content-Encoding") =>
            {
                Source::Stream(decoder_for(
                    &self.headers,
                    Box::new(io::Cursor::new(data)),
                    self.decode_chunk,
                ))
            }
            other => other,
        };
        Response {
            status: self.status,
            reason: self.reason,
            headers: self.headers,
            url,
            source,
            content: None,
            max_reply_bytes: self.max_reply_bytes,
        }
    }
}

impl Read for Response {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match &mut self.source {
            Source::Memory { data, at } => {
                let amount = buffer.len().min(data.len() - *at);
                buffer[..amount].copy_from_slice(&data[*at..*at + amount]);
                *at += amount;
                Ok(amount)
            }
            Source::Stream(reader) => reader.read(buffer),
        }
    }
}

fn too_large(what: &str, limit: usize) -> Error {
    Error::TooLarge {
        what: what.to_string(),
        limit,
    }
}

fn charset(content_type: &str) -> Option<String> {
    content_type.split(';').skip(1).find_map(|parameter| {
        let (key, value) = parameter.split_once('=')?;
        key.trim()
            .eq_ignore_ascii_case("charset")
            .then(|| value.trim().trim_matches('"').to_string())
    })
}

impl Response {
    /// The status code.
    #[must_use]
    pub fn status(&self) -> u16 {
        self.status
    }

    /// The reason phrase (empty when the server sent none).
    #[must_use]
    pub fn reason(&self) -> &str {
        &self.reason
    }

    /// The headers as received.
    #[must_use]
    pub fn headers(&self) -> &Headers {
        &self.headers
    }

    /// The URL the request went to.
    #[must_use]
    pub fn url(&self) -> &Url {
        &self.url
    }

    /// Whether this is a redirect that names where to go: a 301, 302, 303, 307 or 308 with a
    /// `Location`.
    #[must_use]
    pub fn is_redirect(&self) -> bool {
        matches!(self.status, 301 | 302 | 303 | 307 | 308) && self.headers.contains("Location")
    }

    /// Releases the connection; a body not read to its end is abandoned, and its connection is
    /// closed.
    pub fn close(self) {}

    /// The rest of the body in pieces of at most `size` bytes, each as soon as it has arrived.
    pub fn chunks(&mut self, size: usize) -> impl Iterator<Item = io::Result<Vec<u8>>> + '_ {
        let size = size.max(1);
        core::iter::from_fn(move || {
            let mut chunk = alloc::vec![0_u8; size];
            match self.read(&mut chunk) {
                Ok(0) => None,
                Ok(amount) => {
                    chunk.truncate(amount);
                    Some(Ok(chunk))
                }
                Err(error) => Some(Err(error)),
            }
        })
    }

    /// Reads the rest of the body, at most `limit` bytes.
    fn take_rest(&mut self, limit: usize, what: &str) -> Result<Vec<u8>, Error> {
        let mut data = Vec::new();
        let mut buffer = alloc::vec![0_u8; CHUNK.min(limit.saturating_add(1))];
        loop {
            let room = limit.saturating_add(1) - data.len();
            if room == 0 {
                return Err(too_large(what, limit));
            }
            let want = buffer.len().min(room);
            let amount = self.read(&mut buffer[..want])?;
            if amount == 0 {
                return Ok(data);
            }
            data.extend_from_slice(&buffer[..amount]);
            if data.len() > limit {
                return Err(too_large(what, limit));
            }
        }
    }

    /// The rest of the decoded body, read on first use and kept, so that `content`, `text` and
    /// `json` agree however often they are used. Bytes already taken with `read` or `chunks` are
    /// not in it.
    ///
    /// # Errors
    ///
    /// [`Error::TooLarge`] for a body longer than the reply limit (8 MiB by default), and what the
    /// connection reports.
    pub fn content(&mut self) -> Result<&[u8], Error> {
        if self.content.is_none() {
            let what = shown(&self.url);
            let data = self.take_rest(self.max_reply_bytes, &what)?;
            self.content = Some(data);
        }
        Ok(self.content.as_deref().unwrap_or_default())
    }

    /// The body decoded with the `charset` of the `Content-Type` header, else UTF-8; bytes that
    /// are not text are replaced.
    ///
    /// # Errors
    ///
    /// As [`Response::content`].
    pub fn text(&mut self) -> Result<String, Error> {
        self.text_with(None, TextErrors::Replace)
    }

    /// The body decoded with `encoding`, else the `charset` of the `Content-Type` header, else
    /// UTF-8 (an unknown charset is UTF-8 too); `errors` says what happens to bytes that do not
    /// decode.
    ///
    /// # Errors
    ///
    /// As [`Response::content`], and an I/O error of kind `InvalidData` for `TextErrors::Strict`.
    pub fn text_with(
        &mut self,
        encoding: Option<&str>,
        errors: TextErrors,
    ) -> Result<String, Error> {
        let label = encoding
            .map(str::to_string)
            .or_else(|| {
                self.headers
                    .get("Content-Type")
                    .and_then(|value| charset(&value))
            })
            .unwrap_or_else(|| "utf-8".to_string());
        let bytes = self.content()?;
        let encoding =
            encoding_rs::Encoding::for_label(label.trim().as_bytes()).unwrap_or(encoding_rs::UTF_8);
        Ok(match errors {
            TextErrors::Replace => encoding.decode_without_bom_handling(bytes).0.into_owned(),
            TextErrors::Strict => encoding
                .decode_without_bom_handling_and_without_replacement(bytes)
                .ok_or_else(|| {
                    Error::Io(io::Error::new(
                        io::ErrorKind::InvalidData,
                        alloc::format!("the body is not valid {}", encoding.name()),
                    ))
                })?
                .into_owned(),
            TextErrors::Ignore if encoding == encoding_rs::UTF_8 => bytes
                .utf8_chunks()
                .map(|chunk| chunk.valid())
                .collect::<String>(),
            TextErrors::Ignore => encoding
                .decode_without_bom_handling(bytes)
                .0
                .chars()
                .filter(|character| *character != '\u{FFFD}')
                .collect(),
        })
    }

    /// The body parsed as JSON; an empty body reads as `null`.
    ///
    /// # Errors
    ///
    /// As [`Response::content`], and [`Error::Response`] naming the status for a body that is not
    /// the JSON asked for.
    #[cfg(feature = "json")]
    #[cfg_attr(docsrs, doc(cfg(feature = "json")))]
    pub fn json<T: serde::de::DeserializeOwned>(&mut self) -> Result<T, Error> {
        let status = self.status;
        let data = self.content()?;
        let data = if data.iter().all(u8::is_ascii_whitespace) {
            b"null".as_slice()
        } else {
            data
        };
        serde_json::from_slice(data).map_err(|_| Error::Response { status })
    }

    /// Reads the rest of the body into memory, releases the connection and returns this reply,
    /// which can then be read. `limit` defaults to the reply limit (8 MiB).
    ///
    /// # Errors
    ///
    /// [`Error::TooLarge`] for a body that decodes to more than `limit` bytes (its connection is
    /// closed), and what the connection reports.
    pub fn buffered(mut self, limit: Option<usize>) -> Result<Self, Error> {
        let limit = limit.unwrap_or(self.max_reply_bytes);
        let what = shown(&self.url);
        let data = self.take_rest(limit, &what)?;
        self.source = Source::Memory { data, at: 0 };
        Ok(self)
    }

    /// The decoded body, which must be at most `limit` bytes (default the reply limit).
    ///
    /// # Errors
    ///
    /// [`Error::TooLarge`] naming `what` otherwise, with the connection closed.
    pub fn read_limited(mut self, what: &str, limit: Option<usize>) -> Result<Vec<u8>, Error> {
        let limit = limit.unwrap_or(self.max_reply_bytes);
        self.take_rest(limit, what)
    }

    /// The JSON document in the body (at most `limit` bytes, default the reply limit).
    ///
    /// # Errors
    ///
    /// [`Error::TooLarge`] naming `what`, and [`Error::Response`] for a body that is not JSON.
    #[cfg(feature = "json")]
    #[cfg_attr(docsrs, doc(cfg(feature = "json")))]
    pub fn json_limited<T: serde::de::DeserializeOwned>(
        self,
        what: &str,
        limit: Option<usize>,
    ) -> Result<T, Error> {
        let status = self.status;
        let data = self.read_limited(what, limit)?;
        serde_json::from_slice(&data).map_err(|_| Error::Response { status })
    }

    /// Gives up the body of a reply nobody looks at: it is read, up to `limit` bytes (default the
    /// reply limit), so the connection can serve the next request, and the reply is dropped
    /// (which closes the connection when the body was longer). A failure while reading it is not
    /// the caller's concern: the status was already used.
    pub fn discard(mut self, limit: Option<usize>) {
        let mut remaining = limit.unwrap_or(self.max_reply_bytes);
        let mut buffer = alloc::vec![0_u8; CHUNK];
        while remaining > 0 {
            let want = buffer.len().min(remaining);
            match self.read(&mut buffer[..want]) {
                Ok(0) | Err(_) => break,
                Ok(amount) => remaining -= amount,
            }
        }
    }

    /// This reply, unless its status is 400 or above.
    ///
    /// # Errors
    ///
    /// A [`StatusError`] holding the reply.
    pub fn error_for_status(self) -> Result<Self, StatusError> {
        if self.status >= 400 {
            Err(StatusError {
                response: Box::new(self),
            })
        } else {
            Ok(self)
        }
    }

    /// Makes the body check `deadline` before each read.
    pub(crate) fn bound(&mut self, deadline: super::deadline::Deadline) {
        if let Source::Stream(reader) = &mut self.source {
            let inner = core::mem::replace(reader, Box::new(io::empty()));
            *reader = Box::new(Bounded { inner, deadline });
        }
    }
}

struct Bounded {
    inner: Box<dyn Read + Send>,
    deadline: super::deadline::Deadline,
}

impl Read for Bounded {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.deadline
            .check()
            .map_err(|error| io::Error::new(io::ErrorKind::TimedOut, error.to_string()))?;
        self.inner.read(buffer)
    }
}

/// A reply with an error status (400 or above), from [`Response::error_for_status`].
pub struct StatusError {
    response: Box<Response>,
}

impl StatusError {
    /// The reply.
    #[must_use]
    pub fn response(&self) -> &Response {
        &self.response
    }

    /// The reply, to read its body.
    #[must_use]
    pub fn into_response(self) -> Response {
        *self.response
    }
}

impl fmt::Debug for StatusError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StatusError")
            .field("status", &self.response.status)
            .finish()
    }
}

impl fmt::Display for StatusError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "HTTP status {}", self.response.status)
    }
}

impl core::error::Error for StatusError {}
