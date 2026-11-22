//! Connection limits and startup errors for the web app server.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "paths, addresses and IO errors are std's"
)]

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;
use core::net::SocketAddr;
use core::time::Duration;
use std::io;
use std::path::PathBuf;

/// What one connection may take. Every limit is a setting; one above the default is logged as a non-default setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Limits {
    /// Connections at once; past it a new one is closed unanswered. Default 64.
    pub max_connections: usize,
    /// Connections at once from one address, so one peer cannot hold them all. Default 16; behind a proxy every
    /// connection comes from the proxy's address.
    pub max_connections_per_address: usize,
    /// How long one connection may last, however busy; the answer being written then ends with it. Default 600 s.
    pub connection_time: Duration,
    /// The request line and headers, in bytes; past it the request is refused with 431. Default 16 KiB.
    pub max_request_bytes: usize,
    /// Requests one connection may make before it is closed. Default 1000.
    pub max_requests: usize,
    /// For a request's line and headers to arrive, counted from the connection (its TLS handshake included) for the
    /// first, from its first byte for each later one. Default 10 s.
    pub request_time: Duration,
    /// For a kept connection's next request to start. Default 30 s.
    pub idle_time: Duration,
    /// For one write to the connection to make progress. Default 30 s.
    pub send_time: Duration,
    /// How long a browser may keep a file whose name holds a hash (`main.3f9a1b2c.js`), or that is in a folder named for
    /// a revision, without asking again. Default 365 days. Every other file is asked about each time (`no-cache`, answered
    /// 304 while its tag holds).
    pub immutable_age: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_connections: 64,
            max_connections_per_address: 16,
            connection_time: Duration::from_secs(600),
            max_request_bytes: 16 * 1024,
            max_requests: 1000,
            request_time: Duration::from_secs(10),
            idle_time: Duration::from_secs(30),
            send_time: Duration::from_secs(30),
            immutable_age: Duration::from_secs(365 * 24 * 60 * 60),
        }
    }
}

impl Limits {
    /// Connections at once.
    #[must_use]
    pub const fn with_max_connections(mut self, count: usize) -> Self {
        self.max_connections = count;
        self
    }

    /// Connections at once from one address.
    #[must_use]
    pub const fn with_max_connections_per_address(mut self, count: usize) -> Self {
        self.max_connections_per_address = count;
        self
    }

    /// How long one connection may last.
    #[must_use]
    pub const fn with_connection_time(mut self, time: Duration) -> Self {
        self.connection_time = time;
        self
    }

    /// The request line and headers, in bytes.
    #[must_use]
    pub const fn with_max_request_bytes(mut self, bytes: usize) -> Self {
        self.max_request_bytes = bytes;
        self
    }

    /// Requests one connection may make.
    #[must_use]
    pub const fn with_max_requests(mut self, count: usize) -> Self {
        self.max_requests = count;
        self
    }

    /// For a request's line and headers to arrive.
    #[must_use]
    pub const fn with_request_time(mut self, time: Duration) -> Self {
        self.request_time = time;
        self
    }

    /// For a kept connection's next request to start.
    #[must_use]
    pub const fn with_idle_time(mut self, time: Duration) -> Self {
        self.idle_time = time;
        self
    }

    /// For one write to make progress.
    #[must_use]
    pub const fn with_send_time(mut self, time: Duration) -> Self {
        self.send_time = time;
        self
    }

    /// How long a hashed file may be kept unasked.
    #[must_use]
    pub const fn with_immutable_age(mut self, age: Duration) -> Self {
        self.immutable_age = age;
        self
    }

    /// Each limit above its default, as `(setting, value, default)` in canoa.cfg's words.
    pub(super) fn non_defaults(&self) -> Vec<(&'static str, String, String)> {
        let standard = Self::default();
        let mut found = Vec::new();
        let mut count = |setting, value: usize, default: usize| {
            if value > default {
                found.push((setting, value.to_string(), default.to_string()));
            }
        };
        count(
            "max_connections",
            self.max_connections,
            standard.max_connections,
        );
        count(
            "max_connections_per_address",
            self.max_connections_per_address,
            standard.max_connections_per_address,
        );
        count(
            "max_request_bytes",
            self.max_request_bytes,
            standard.max_request_bytes,
        );
        count("max_requests", self.max_requests, standard.max_requests);
        let mut time = |setting, value: Duration, default: Duration| {
            if value > default {
                found.push((
                    setting,
                    value.as_secs().to_string(),
                    default.as_secs().to_string(),
                ));
            }
        };
        time("request_seconds", self.request_time, standard.request_time);
        time("idle_seconds", self.idle_time, standard.idle_time);
        time("send_seconds", self.send_time, standard.send_time);
        time(
            "connection_seconds",
            self.connection_time,
            standard.connection_time,
        );
        found
    }

    /// The first limit that would serve nothing, in canoa.cfg's words.
    pub(super) fn zero(&self) -> Option<&'static str> {
        [
            ("max_connections", self.max_connections == 0),
            (
                "max_connections_per_address",
                self.max_connections_per_address == 0,
            ),
            ("connection_seconds", self.connection_time.is_zero()),
            ("max_request_bytes", self.max_request_bytes == 0),
            ("max_requests", self.max_requests == 0),
            ("request_seconds", self.request_time.is_zero()),
            ("idle_seconds", self.idle_time.is_zero()),
            ("send_seconds", self.send_time.is_zero()),
        ]
        .into_iter()
        .find_map(|(setting, zero)| zero.then_some(setting))
    }
}

/// Why the web app could not be served. No variant quotes a file's contents.
#[derive(Debug)]
#[non_exhaustive]
pub enum WebError {
    /// No directory was given, and this build holds no site (it was built without the `web` feature).
    NoSite,
    /// The site's directory cannot be served.
    Site {
        /// The directory.
        dir: PathBuf,
        /// Why: absent, not a directory, or without `index.html`.
        why: &'static str,
    },
    /// Plain HTTP was asked for beyond loopback, where it is refused: give a certificate and key.
    PlainBeyondLoopback(SocketAddr),
    /// A limit is zero, which would serve nothing.
    Limit(&'static str),
    /// A `connect` source is not one origin: `http`, `https`, `ws` or `wss` and a host, with an optional port and path.
    Connect,
    /// An `office_frames` source is not `https://` and a host (which may start with `*.`), with an optional port.
    OfficeFrame,
    /// The add-in's secret could not be read or made: there is no home to keep it in (`None`), or the file failed.
    Secret(Option<io::Error>),
    /// The certificate or key could not be used.
    Tls {
        /// The file, or the certificate's when the two do not go together.
        file: PathBuf,
        /// Why, in words of the TLS library's, which never quote the key.
        why: String,
    },
    /// The address could not be bound.
    Listen {
        /// The address.
        address: SocketAddr,
        /// What the operating system said.
        source: io::Error,
    },
}

impl fmt::Display for WebError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSite => f.write_str(
                "no site to serve: give [host.web] dir or --web DIR, or build canoa with the web feature",
            ),
            Self::Site { dir, why } => write!(f, "the site {} {why}", dir.display()),
            Self::PlainBeyondLoopback(address) => write!(
                f,
                "the web app listens on {address}, beyond loopback, where plain HTTP is refused: give [host.web] cert and key"
            ),
            Self::Limit(setting) => write!(f, "[host.web] {setting} must be above zero"),
            Self::Connect => f.write_str(
                "a [host.web] connect source is not http, https, ws or wss and a host, with an optional port and path",
            ),
            Self::OfficeFrame => f.write_str(
                "a [host.web] office_frames source is not https and a host (a leading *. is allowed), with an optional port",
            ),
            Self::Secret(None) => f.write_str(
                "[host.web] office_secret needs a home to keep the secret in, and this host has none",
            ),
            Self::Secret(Some(error)) => {
                write!(f, "the add-in's secret could not be read or made: {error}")
            }
            Self::Tls { file, why } => write!(f, "TLS from {}: {why}", file.display()),
            Self::Listen { address, source } => {
                write!(f, "the web app could not listen on {address}: {source}")
            }
        }
    }
}

impl core::error::Error for WebError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Listen { source, .. } => Some(source),
            Self::Secret(Some(error)) => Some(error),
            _ => None,
        }
    }
}
