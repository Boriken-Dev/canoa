//! The launcher's one error type.

use core::fmt;
use std::io;

/// Why a launch stopped.
#[derive(Debug)]
pub enum Error {
    /// A file system call failed; `context` says which.
    Io {
        /// What the launcher was doing.
        context: String,
        /// The operating system's error.
        source: io::Error,
    },
    /// A folder or file failed a safety check (a link, another owner, an unexpected file); nothing was run.
    Refused(String),
    /// The embedded payload is missing, damaged or malformed.
    Payload(String),
}

impl Error {
    pub(crate) fn io(context: impl Into<String>, source: io::Error) -> Self {
        Self::Io {
            context: context.into(),
            source,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { context, source } => write!(f, "{context}: {source}"),
            Self::Refused(why) => write!(f, "refused: {why}"),
            Self::Payload(why) => write!(f, "the payload is not usable: {why}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// `Result` with this crate's [`Error`].
pub type Result<T> = core::result::Result<T, Error>;
