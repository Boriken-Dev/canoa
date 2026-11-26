//! What a request sends for itself to the server, and what makes a client authenticated.

use core::fmt;

use coqui_proxy::{Credentials, Secret};
use url::Url;

use super::client::Client;
use super::error::BoxError;

/// The `Authorization` of one request, sent to the origin of its URL unless a header names an
/// `Authorization`; it beats a token of the client.
#[derive(Clone)]
#[non_exhaustive]
pub enum Auth {
    /// `Authorization: Basic`, from a user and a password.
    Basic(Credentials),
    /// The `Authorization` value as it is (`Bearer <token>`).
    Header(Secret),
}

impl fmt::Debug for Auth {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Basic(_) => "Auth::Basic(..)",
            Self::Header(_) => "Auth::Header(..)",
        })
    }
}

/// Something that makes a client authenticated for a URL (a login, a token refresh): the client
/// calls it before each request, and [`fetch`](super::fetch)'s `reauth` again after a `401` or
/// `403`.
pub trait AuthProvider: Send + Sync {
    /// Makes `client` authenticated for `url`.
    ///
    /// # Errors
    ///
    /// Whatever went wrong; the client swallows it, `fetch(reauth)` reports it.
    fn ensure(&self, client: &Client, url: &Url) -> Result<(), BoxError>;
}

impl<F> AuthProvider for F
where
    F: Fn(&Client, &Url) -> Result<(), BoxError> + Send + Sync,
{
    fn ensure(&self, client: &Client, url: &Url) -> Result<(), BoxError> {
        self(client, url)
    }
}
