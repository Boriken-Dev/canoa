//! Decoding a `gzip` or `deflate` body in bounded steps.
//!
//! `deflate` is read as zlib-wrapped data, or as raw data when the zlib header is absent, which is
//! what servers send under that name; concatenated gzip members are one body, and bytes after the
//! last complete member are not a body. Each read produces at most `chunk` decoded bytes, so a
//! body that expands a thousandfold is never held whole because its header said it was small.

use alloc::boxed::Box;
use std::io::{self, BufRead, BufReader, Read};

use flate2::bufread::{DeflateDecoder, GzDecoder, ZlibDecoder};

use super::headers::Headers;

type Source = Box<dyn Read + Send>;

/// `source` decoded as `headers` say, or as it is when they name no coding this knows.
pub(crate) fn decoder_for(headers: &Headers, source: Source, chunk: usize) -> Source {
    let names: alloc::vec::Vec<String> = headers
        .get("Content-Encoding")
        .unwrap_or_default()
        .split(',')
        .map(|name| name.trim().to_ascii_lowercase())
        .filter(|name| !name.is_empty() && name != "identity")
        .collect();
    let [name] = names.as_slice() else {
        return source;
    };
    match name.as_str() {
        "gzip" | "x-gzip" => Box::new(Gzip::new(source, chunk)),
        "deflate" => Box::new(Deflate::new(source, chunk)),
        _ => source,
    }
}

fn invalid(kind: &str, error: io::Error) -> io::Error {
    match error.kind() {
        io::ErrorKind::UnexpectedEof => io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "the compressed response body ended early",
        ),
        io::ErrorKind::InvalidInput | io::ErrorKind::InvalidData => io::Error::new(
            io::ErrorKind::InvalidData,
            alloc::format!("the response body is not valid {kind} data"),
        ),
        _ => error,
    }
}

struct Gzip {
    member: Option<GzDecoder<BufReader<Source>>>,
    between: Option<BufReader<Source>>,
    members: usize,
    chunk: usize,
}

impl Gzip {
    fn new(source: Source, chunk: usize) -> Self {
        Self {
            member: None,
            between: Some(BufReader::new(source)),
            members: 0,
            chunk: chunk.max(1),
        }
    }
}

impl Read for Gzip {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        let limit = buffer.len().min(self.chunk);
        loop {
            if let Some(member) = self.member.as_mut() {
                match member.read(&mut buffer[..limit]) {
                    Ok(0) => {
                        self.members += 1;
                        self.between = self.member.take().map(GzDecoder::into_inner);
                    }
                    Ok(amount) => return Ok(amount),
                    Err(error) if self.members > 0 => {
                        // Bytes after the last complete member are not a body.
                        let _ = error;
                        self.member = None;
                        return Ok(0);
                    }
                    Err(error) => return Err(invalid("gzip", error)),
                }
            } else if let Some(mut reader) = self.between.take() {
                if reader.fill_buf()?.is_empty() {
                    return Ok(0);
                }
                self.member = Some(GzDecoder::new(reader));
            } else {
                return Ok(0);
            }
        }
    }
}

enum Deflated {
    Waiting(BufReader<Source>),
    Zlib(ZlibDecoder<BufReader<Source>>),
    Raw(DeflateDecoder<BufReader<Source>>),
    Done,
}

struct Deflate {
    state: Deflated,
    chunk: usize,
}

impl Deflate {
    fn new(source: Source, chunk: usize) -> Self {
        Self {
            state: Deflated::Waiting(BufReader::new(source)),
            chunk: chunk.max(1),
        }
    }
}

impl Read for Deflate {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        let limit = buffer.len().min(self.chunk);
        if let Deflated::Waiting(_) = self.state {
            let Deflated::Waiting(mut reader) = core::mem::replace(&mut self.state, Deflated::Done)
            else {
                return Ok(0);
            };
            let head = reader.fill_buf()?;
            let wrapped = head.len() < 2
                || (head[0] & 0x0F == 8
                    && ((u16::from(head[0]) << 8) | u16::from(head[1])) % 31 == 0);
            self.state = if wrapped {
                Deflated::Zlib(ZlibDecoder::new(reader))
            } else {
                Deflated::Raw(DeflateDecoder::new(reader))
            };
        }
        let result = match &mut self.state {
            Deflated::Zlib(decoder) => decoder.read(&mut buffer[..limit]),
            Deflated::Raw(decoder) => decoder.read(&mut buffer[..limit]),
            _ => return Ok(0),
        };
        result.map_err(|error| invalid("deflate", error))
    }
}
