//! How a request reaches Exchange: one POST of XML to the server, signed in as the account says, through [`Network`] like every
//! other fetch the host makes, so the system's proxy and certificate verifier apply.
//!
//! There are two sign-ins. Basic sends the account's user name and password on every request. Windows is the person's own
//! logon (Negotiate: Kerberos, then NTLM); the one place it is made is [`Transport::post_negotiate`].
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "the clock and shared ownership are std's"
)]

use core::time::Duration;
use std::sync::Arc;

use canoa::network::{Network, Secret};
use data_encoding::BASE64;

use crate::mail::model::MailError;

/// The most bytes read from one answer: a page of a message list or one message with its body.
const MAX_ANSWER: u64 = 64 * 1024 * 1024;

/// How the account signs in.
#[derive(Clone, Debug)]
pub(crate) enum Auth {
    /// The user name and password the account names, sent on every request.
    Basic { user: String, password: Secret },
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

    /// The seam for signing in with the person's Windows logon (Negotiate, Kerberos then NTLM) to the server itself. The HTTP
    /// stack signs in to a proxy that way; to a server it does not yet, so this says so rather than sending a request that would
    /// fail with a 401. Whatever signs in here answers as [`Transport::send`] does, once the exchange of challenges is done on
    /// one connection.
    #[allow(
        clippy::unused_self,
        reason = "the signed-in exchange will use the transport"
    )]
    fn post_negotiate(&self, _url: &str, _body: &str) -> Result<Reply, MailError> {
        Err(MailError::Auth(
            "signing in to the server with the Windows logon is not available in this build: give the account a user and \
             password_env (Basic), or use the Outlook backend, which signs in as Outlook does"
                .to_owned(),
        ))
    }

    /// One POST, with `authorization` as the header when there is one.
    fn send(&self, url: &str, body: &str, authorization: Option<&str>) -> Result<Reply, MailError> {
        let config = ureq::Agent::config_builder()
            .max_redirects(0)
            .http_status_as_error(false)
            .timeout_global(Some(self.timeout))
            .build();
        let agent = self.network.agent(config).map_err(|error| {
            MailError::Unreachable(format!("the network is not usable: {error}"))
        })?;
        let mut request = agent
            .post(url)
            .header("content-type", "text/xml; charset=utf-8")
            .header("accept", "text/xml, application/xml");
        if let Some(value) = authorization {
            request = request.header("authorization", value);
        }
        let response = request
            .send(body)
            .map_err(|error| MailError::Unreachable(format!("{url}: {error}")))?;
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
