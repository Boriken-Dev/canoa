//! A loopback HTTP proxy started inside the process, that sends what it is given through a
//! [`ProxyConnector`](crate::ProxyConnector).
//!
//! An HTTP client that cannot take a connector, or retry a route (`reqwest`), is given the relay
//! as its only proxy and gets everything the connector does: the routes of a
//! [`Resolver`](crate::Resolver) with failover, PAC, bypass rules and proxy login, SOCKS and a
//! direct connection.
//!
//! There are two relays with the same security model, settings and behaviour:
//!
//! | | Feature | Runs on | Dials with |
//! | --- | --- | --- | --- |
//! | [`Relay`] | `relay` | a Tokio runtime ([`Relay::start`] the current one, [`Relay::start_on`] a handle) | the asynchronous connector |
//! | [`blocking::Relay`] | `relay-blocking` | its own threads, no Tokio | the blocking connector |
//!
//! # What they serve
//!
//! - `CONNECT host:port`: the destination is dialled through the connector, then `200` and a
//!   splice in both directions. TLS to the destination is the client's.
//! - An absolute-form `http://` request: the destination is dialled through the connector and
//!   the request forwarded in origin form, without its hop-by-hop headers, the reply streamed
//!   back. A connection to the destination serves one request; the client's connection to the
//!   relay is kept alive.
//! - A dial that fails on every route is `502`; a request without the credential is `407`; any
//!   other request is `405`.
//!
//! # Security
//!
//! The relay has no access control but its credential, so it listens on a loopback address only
//! (a [`RelaySettings::address`] that is not one is an error) and every
//! request must carry `Proxy-Authorization: Basic` with a random credential made for each relay
//! (256 bits from the operating system's generator), or the same secret in the [`TOKEN_HEADER`]
//! header, compared in constant time. Another local user or process cannot use it without having
//! read it. The credential never reaches a log, an error or a `Debug`, and is never forwarded.
//! Each bound of [`RelaySettings`] is a setting.
//!
//! The header exists for clients that drop `Proxy-Authorization` when a redirect leads to another
//! host (`reqwest` does for plain `http:` requests, which would then get `407`): a header of
//! another name is kept.
//!
//! # Example
//!
//! ```no_run
//! # #[cfg(feature = "relay")]
//! # async fn run() -> Result<(), coqui_proxy::Error> {
//! use coqui_proxy::relay::Relay;
//! use coqui_proxy::{ProxyConnector, Resolver};
//!
//! let relay = Relay::builder(ProxyConnector::default().asynch(), Resolver::system()).start()?;
//! // A client sends `Proxy-Authorization: Basic` with these, to this URL.
//! let (url, user, password) = (
//!     relay.proxy_url(),
//!     relay.credentials().username(),
//!     relay.credentials().password(),
//! );
//! # let _ = (url, user, password);
//! relay.shutdown().await;
//! # Ok(())
//! # }
//! ```
//!
//! Without Tokio:
//!
//! ```no_run
//! # #[cfg(feature = "relay-blocking")]
//! # fn run() -> Result<(), coqui_proxy::Error> {
//! use coqui_proxy::relay::blocking::Relay;
//! use coqui_proxy::{ProxyConnector, Resolver};
//!
//! let relay = Relay::builder(ProxyConnector::default(), Resolver::system()).start()?;
//! let url = relay.proxy_url();
//! # let _ = url;
//! drop(relay); // closes the listener, ends the connections and joins the threads
//! # Ok(())
//! # }
//! ```

use alloc::format;
use alloc::string::ToString;
use alloc::vec::Vec;
use std::io;

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use http::header::{
    CONNECTION, HeaderMap, HeaderName, PROXY_AUTHENTICATE, PROXY_AUTHORIZATION, TE, TRAILER,
    TRANSFER_ENCODING, UPGRADE,
};
use subtle::ConstantTimeEq;

use crate::error::{Error, Reason};
use crate::proxy::Credentials;
use crate::settings::RelaySettings;

#[cfg(feature = "relay")]
mod asynch;
#[cfg(feature = "relay-blocking")]
#[cfg_attr(docsrs, doc(cfg(feature = "relay-blocking")))]
pub mod blocking;

#[cfg(feature = "relay")]
#[cfg_attr(docsrs, doc(cfg(feature = "relay")))]
pub use asynch::{Relay, RelayBuilder, RelayHandle};

/// The user name of the relay's credential.
const USER: &str = "coqui-relay";

/// The header that carries the relay's password as it is, instead of `Proxy-Authorization`, for
/// a client that keeps only headers it does not know to be credentials across a redirect.
pub const TOKEN_HEADER: &str = "x-coqui-relay-token";

/// Bytes of randomness in a relay's password (256 bits).
const PASSWORD_BYTES: usize = 32;

/// The `Proxy-Authenticate` challenge of a `407`.
const CHALLENGE: &str = "Basic realm=\"coqui-proxy relay\"";

/// Headers a proxy does not forward, besides those the `Connection` header names.
const HOP_BY_HOP: [HeaderName; 7] = [
    CONNECTION,
    PROXY_AUTHENTICATE,
    PROXY_AUTHORIZATION,
    TE,
    TRAILER,
    TRANSFER_ENCODING,
    UPGRADE,
];

/// Checks the settings both relays share.
fn check_settings(settings: &RelaySettings) -> Result<(), Error> {
    let refused = |reason: &'static str| Err(Error::Settings(Reason::new(reason)));
    if !settings.address.is_loopback() {
        return refused("the relay listens on a loopback address only");
    }
    if settings.max_connections == 0 {
        return refused("max_connections must be at least 1");
    }
    if settings.header_timeout.is_zero()
        || settings.idle_timeout.is_zero()
        || settings.accept_retry.is_zero()
        || settings.poll_interval.is_zero()
    {
        return refused("a relay time limit must not be zero");
    }
    if settings.max_head_bytes < RelaySettings::MIN_MAX_HEAD_BYTES {
        return refused("max_head_bytes is under the 8 KiB the HTTP/1 server works with");
    }
    Ok(())
}

/// A relay's credential, as the login a client is given and as the two forms it is checked in.
struct Guard {
    credentials: Credentials,
    expected: Vec<u8>,
    token: Vec<u8>,
}

impl Guard {
    /// A new random credential from the operating system's generator.
    fn generate() -> Result<Self, Error> {
        let mut secret = [0u8; PASSWORD_BYTES];
        getrandom::fill(&mut secret)
            .map_err(|error| Error::Io(io::Error::other(error.to_string())))?;
        let credentials = Credentials::new(USER, URL_SAFE_NO_PAD.encode(secret));
        let expected = format!(
            "Basic {}",
            STANDARD.encode(format!("{USER}:{}", credentials.password()))
        );
        Ok(Self {
            token: credentials.password().as_bytes().to_vec(),
            expected: expected.into_bytes(),
            credentials,
        })
    }

    /// Whether `Proxy-Authorization` or the token header is the relay's credential, compared in
    /// constant time (both are compared, so that which one matched is not a timing).
    fn allows(&self, headers: &HeaderMap) -> bool {
        let matches = |name: &str, wanted: &[u8]| {
            headers
                .get(name)
                .is_some_and(|given| bool::from(given.as_bytes().ct_eq(wanted)))
        };
        let by_login = matches(PROXY_AUTHORIZATION.as_str(), &self.expected);
        let by_token = matches(TOKEN_HEADER, &self.token);
        by_login | by_token
    }
}

/// `Host` for a destination: its name and port, without any login in the URL.
fn host_header(authority: &http::uri::Authority) -> alloc::string::String {
    match authority.port() {
        Some(port) => format!("{}:{port}", authority.host()),
        None => authority.host().to_string(),
    }
}

/// The scheme that tells the resolver which proxy variable and PAC branch apply to a tunnel to
/// `port`.
fn tunnel_scheme(port: u16) -> &'static str {
    if port == 80 { "http" } else { "https" }
}

/// Removes what a proxy does not forward: the fixed list and every header `Connection` names.
fn strip_hop_by_hop(headers: &mut HeaderMap) {
    let named: Vec<HeaderName> = headers
        .get_all(CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .filter_map(|token| HeaderName::from_bytes(token.trim().as_bytes()).ok())
        .collect();
    for name in named.iter().chain(&HOP_BY_HOP) {
        headers.remove(name);
    }
    headers.remove("keep-alive");
    headers.remove("proxy-connection");
}
