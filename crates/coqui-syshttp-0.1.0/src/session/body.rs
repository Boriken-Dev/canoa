//! A request or reply body, and how a request body is framed.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;
use std::io::{self, Read};

/// A body. Only [`Body::Empty`], [`Body::Bytes`] and [`Body::Text`] can be sent twice, so only
/// they are sent again after a redirect, a dropped connection or a retry.
#[derive(Default)]
#[non_exhaustive]
pub enum Body {
    /// No body.
    #[default]
    Empty,
    /// Bytes in memory.
    Bytes(Vec<u8>),
    /// Text in memory, sent as UTF-8.
    Text(String),
    /// A reader, sent under the `Content-Length` header if there is one, else chunked.
    Reader(Box<dyn Read + Send>),
    /// Pieces of bytes, sent under the `Content-Length` header if there is one, else chunked.
    Chunks(Box<dyn Iterator<Item = Vec<u8>> + Send>),
}

impl fmt::Debug for Body {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("Body::Empty"),
            Self::Bytes(bytes) => write!(formatter, "Body::Bytes({} bytes)", bytes.len()),
            Self::Text(text) => write!(formatter, "Body::Text({} bytes)", text.len()),
            Self::Reader(_) => formatter.write_str("Body::Reader(..)"),
            Self::Chunks(_) => formatter.write_str("Body::Chunks(..)"),
        }
    }
}

impl Body {
    /// A body read from `reader`.
    pub fn reader(reader: impl Read + Send + 'static) -> Self {
        Self::Reader(Box::new(reader))
    }

    /// A body made of `chunks`.
    pub fn chunks(chunks: impl Iterator<Item = Vec<u8>> + Send + 'static) -> Self {
        Self::Chunks(Box::new(chunks))
    }

    /// Whether the body can be sent twice.
    #[must_use]
    pub fn is_replayable(&self) -> bool {
        matches!(self, Self::Empty | Self::Bytes(_) | Self::Text(_))
    }

    /// The bytes it will send, when that is known without reading it.
    #[must_use]
    pub fn len(&self) -> Option<u64> {
        match self {
            Self::Empty => Some(0),
            Self::Bytes(bytes) => Some(bytes.len() as u64),
            Self::Text(text) => Some(text.len() as u64),
            _ => None,
        }
    }

    /// Whether it is [`Body::Empty`] or holds no bytes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == Some(0)
    }

    /// The same body again, for a body that can be sent twice.
    pub(crate) fn replay(&self) -> Option<Self> {
        match self {
            Self::Empty => Some(Self::Empty),
            Self::Bytes(bytes) => Some(Self::Bytes(bytes.clone())),
            Self::Text(text) => Some(Self::Text(text.clone())),
            _ => None,
        }
    }

    /// The body as one reader.
    pub(crate) fn into_reader(self) -> Box<dyn Read + Send> {
        match self {
            Self::Empty => Box::new(io::empty()),
            Self::Bytes(bytes) => Box::new(io::Cursor::new(bytes)),
            Self::Text(text) => Box::new(io::Cursor::new(text.into_bytes())),
            Self::Reader(reader) => reader,
            Self::Chunks(chunks) => Box::new(ChunkReader {
                chunks,
                pending: Vec::new(),
                at: 0,
            }),
        }
    }
}

impl From<Vec<u8>> for Body {
    fn from(bytes: Vec<u8>) -> Self {
        Self::Bytes(bytes)
    }
}

impl From<&[u8]> for Body {
    fn from(bytes: &[u8]) -> Self {
        Self::Bytes(bytes.to_vec())
    }
}

impl From<String> for Body {
    fn from(text: String) -> Self {
        Self::Text(text)
    }
}

impl From<&str> for Body {
    fn from(text: &str) -> Self {
        Self::Text(text.into())
    }
}

struct ChunkReader {
    chunks: Box<dyn Iterator<Item = Vec<u8>> + Send>,
    pending: Vec<u8>,
    at: usize,
}

impl Read for ChunkReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        while self.at >= self.pending.len() {
            match self.chunks.next() {
                Some(chunk) => {
                    self.pending = chunk;
                    self.at = 0;
                }
                None => return Ok(0),
            }
        }
        let amount = buffer.len().min(self.pending.len() - self.at);
        buffer[..amount].copy_from_slice(&self.pending[self.at..self.at + amount]);
        self.at += amount;
        Ok(amount)
    }
}

/// A body sent under a `Content-Length`: exactly `length` bytes of `source`. It never reads or
/// sends more, so a file that grew since it was measured cannot overrun the length, and a source
/// that ends first is an error, not a shorter upload.
pub(crate) struct Exact {
    source: Box<dyn Read + Send>,
    left: u64,
}

impl Exact {
    pub(crate) fn new(source: Box<dyn Read + Send>, length: u64) -> Self {
        Self {
            source,
            left: length,
        }
    }
}

impl Read for Exact {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.left == 0 || buffer.is_empty() {
            return Ok(0);
        }
        let want = usize::try_from(self.left).map_or(buffer.len(), |left| left.min(buffer.len()));
        let amount = self.source.read(&mut buffer[..want])?;
        if amount == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the request body ended before its Content-Length",
            ));
        }
        self.left -= amount as u64;
        Ok(amount)
    }
}
