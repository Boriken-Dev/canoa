//! How a request reaches Exchange: one POST of XML to the server, signed in as the account says, through [`Network`] like every
//! other fetch the host makes, so the system's proxy and certificate verifier apply.
//!
//! There are two sign-ins. Basic sends the account's user name and password on every request. Windows is the person's own
//! logon (Negotiate: Kerberos, then NTLM), answered to the server's `401` by coqui-syshttp's origin login.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "the clock and shared ownership are std's"
)]

use core::time::Duration;
use std::io::ErrorKind;
use std::sync::Arc;

use canoa::network::{Network, OriginAuth, Secret};
use data_encoding::BASE64;
use ureq::http::Request;
use url::Url;

use canoa_mail::mail_backend::MailError;

/// The most bytes read from one answer: a page of a message list or one message with its body.
const MAX_ANSWER: u64 = 64 * 1024 * 1024;

const CONTENT_TYPE: &str = "text/xml; charset=utf-8";
const ACCEPT: &str = "text/xml, application/xml";

/// How the account signs in.
#[derive(Clone, Debug)]
pub enum Auth {
    /// The user name and password the account names, sent on every request.
    Basic {
        /// Account user name (`DOMAIN\user` or UPN).
        user: String,
        /// Password or secret token.
        password: Secret,
    },
    /// The person's Windows logon.
    Windows,
}

/// What the server answered.
#[derive(Debug)]
pub(super) struct Reply {
    pub(super) status: u16,
    /// The sign-in schemes a refusal offered (`Negotiate`, `Basic`).
    schemes: Vec<String>,
    pub(super) body: String,
}

/// Posts XML to a server as an account.
#[derive(Clone, Debug)]
pub(super) struct Transport {
    network: Arc<Network>,
    auth: Auth,
    timeout: Duration,
}

impl Transport {
    pub(super) fn new(network: Arc<Network>, auth: Auth, timeout: Duration) -> Self {
        Self {
            network,
            auth,
            timeout,
        }
    }

    /// Posts `body` to `url` and answers with what the server said when it accepted the sign-in.
    ///
    /// # Errors
    ///
    /// The server could not be reached, or it refused the sign-in (the error says which sign-ins it offered).
    pub(super) fn post(&self, url: &str, body: &str) -> Result<Reply, MailError> {
        let reply = match &self.auth {
            Auth::Basic { user, password } => {
                let token = BASE64.encode(format!("{user}:{}", password.expose()).as_bytes());
                self.send(url, body, Some(&format!("Basic {token}")))?
            }
            Auth::Windows => self.post_negotiate(url, body)?,
        };
        if reply.status == 401 || reply.status == 403 {
            return Err(MailError::Auth(format!(
                "the server refused the sign-in ({}){}",
                reply.status,
                Self::offered(&reply)
            )));
        }
        Ok(reply)
    }

    fn offered(reply: &Reply) -> String {
        if reply.schemes.is_empty() {
            String::new()
        } else {
            format!(": it accepts {}", reply.schemes.join(", "))
        }
    }

    /// Signs in with the person's Windows logon (Negotiate, Kerberos then NTLM) when the server answers `401`, through the one
    /// client that answers a server's challenge: every leg of an NTLM exchange rides one connection, and the body goes with the
    /// last. A server that offers no scheme the logon can answer is left at its `401`, which [`Transport::post`] reports with
    /// the schemes it offered.
    fn post_negotiate(&self, url: &str, body: &str) -> Result<Reply, MailError> {
        let host = Url::parse(url)
            .ok()
            .and_then(|parsed| parsed.host_str().map(str::to_owned))
            .ok_or_else(|| MailError::Unreachable(format!("{url} is not a server address")))?;
        let policy = OriginAuth::builder()
            .negotiate([host])
            .build()
            .map_err(|error| login_failed(&error))?;
        let client = self
            .network
            .login_agent(self.config(), policy)
            .map_err(|error| {
                MailError::Unreachable(format!("the network is not usable: {error}"))
            })?;
        let request = Request::post(url)
            .header("content-type", CONTENT_TYPE)
            .header("accept", ACCEPT)
            .body(body.as_bytes().to_vec())
            .map_err(|error| MailError::Unreachable(format!("{url}: {error}")))?;
        let response = client.run(request).map_err(|error| match error {
            ureq::Error::Io(ref io) if io.kind() == ErrorKind::PermissionDenied => login_failed(io),
            other => MailError::Unreachable(format!("{url}: {other}")),
        })?;
        Self::reply(url, response)
    }

    fn config(&self) -> ureq::config::Config {
        ureq::Agent::config_builder()
            .max_redirects(0)
            .http_status_as_error(false)
            .timeout_global(Some(self.timeout))
            .build()
    }

    /// One POST, with `authorization` as the header when there is one.
    fn send(&self, url: &str, body: &str, authorization: Option<&str>) -> Result<Reply, MailError> {
        let agent = self.network.agent(self.config()).map_err(|error| {
            MailError::Unreachable(format!("the network is not usable: {error}"))
        })?;
        let mut request = agent
            .post(url)
            .header("content-type", CONTENT_TYPE)
            .header("accept", ACCEPT);
        if let Some(value) = authorization {
            request = request.header("authorization", value);
        }
        let response = request
            .send(body)
            .map_err(|error| MailError::Unreachable(format!("{url}: {error}")))?;
        Self::reply(url, response)
    }

    /// What the server answered: the status, the schemes a refusal offered and the body.
    fn reply(url: &str, response: ureq::http::Response<ureq::Body>) -> Result<Reply, MailError> {
        let status = response.status().as_u16();
        let schemes: Vec<String> = response
            .headers()
            .get_all("www-authenticate")
            .iter()
            .filter_map(|value| value.to_str().ok())
            .map(|value| value.split_whitespace().next().unwrap_or("").to_owned())
            .filter(|scheme| !scheme.is_empty())
            .collect();
        let text = response
            .into_body()
            .with_config()
            .limit(MAX_ANSWER)
            .read_to_string()
            .map_err(|error| MailError::Unreachable(format!("{url}: {error}")))?;
        Ok(Reply {
            status,
            schemes,
            body: text,
        })
    }
}

fn login_failed(error: &dyn core::fmt::Display) -> MailError {
    MailError::Auth(format!(
        "signing in to the server with the Windows logon failed: {error}"
    ))
}
