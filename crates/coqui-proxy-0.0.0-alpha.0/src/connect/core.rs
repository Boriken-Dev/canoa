//! The sans-I/O half of an HTTP proxy connection: the `CONNECT` request, its reply, and the login
//! a `407` starts. Nothing here reads or writes a socket; a driver does, and feeds the bytes in.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::auth::{self, Client, Negotiate, Scheme};
use crate::error::{Error, Reason};
use crate::proxy::Proxy;
use crate::settings::ConnectorSettings;

/// What a decoder made of the bytes it was given.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Progress<T> {
    /// More bytes are needed; at least this many.
    Incomplete(usize),
    /// A value, made of the first `consumed` bytes. Later bytes belong to whatever follows.
    Complete {
        /// What was decoded.
        value: T,
        /// How many bytes it took.
        consumed: usize,
    },
}

/// A `CONNECT` request to send to an HTTP proxy.
#[derive(Debug, Clone)]
pub struct ConnectRequest {
    authority: String,
    user_agent: String,
    authorization: Option<String>,
    headers: Vec<(String, String)>,
}

impl ConnectRequest {
    /// A request for the tunnel to `authority` (`host:port`), as `user_agent`.
    pub fn new(authority: impl Into<String>, user_agent: impl Into<String>) -> Self {
        Self {
            authority: authority.into(),
            user_agent: user_agent.into(),
            authorization: None,
            headers: Vec::new(),
        }
    }

    /// The `Proxy-Authorization` value to send, unless a header added later is one.
    #[must_use]
    pub fn authorization(mut self, value: Option<String>) -> Self {
        self.authorization = value;
        self
    }

    /// A header sent as it is, after the standard ones.
    #[must_use]
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// The request as bytes.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let authority = &self.authority;
        let mut request = format!(
            "CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\nProxy-Connection: Keep-Alive\r\nUser-Agent: {}\r\n",
            self.user_agent
        );
        let own_authorization = self
            .headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("proxy-authorization"));
        if let Some(value) = self.authorization.as_deref().filter(|_| !own_authorization) {
            request.push_str("Proxy-Authorization: ");
            request.push_str(value);
            request.push_str("\r\n");
        }
        for (name, value) in &self.headers {
            request.push_str(name);
            request.push_str(": ");
            request.push_str(value);
            request.push_str("\r\n");
        }
        request.push_str("\r\n");
        request.into_bytes()
    }
}

/// A proxy's reply to a `CONNECT`: its status and headers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectResponse {
    status: u16,
    headers: Vec<(String, String)>,
}

impl ConnectResponse {
    /// Decodes a reply head from the start of `bytes`, within the limits of `settings`
    /// (`max_response_head_bytes`, `max_response_headers`).
    ///
    /// Any bytes at all give a value or an error, never a panic.
    ///
    /// # Errors
    ///
    /// [`Error::Refused`] when the head is over a limit or is not HTTP.
    pub fn decode(bytes: &[u8], settings: &ConnectorSettings) -> Result<Progress<Self>, Error> {
        let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") else {
            if bytes.len() >= settings.max_response_head_bytes {
                return Err(Error::Refused(Reason::new("the reply head is too long")));
            }
            return Ok(Progress::Incomplete(1));
        };
        let consumed = end + 4;
        if consumed > settings.max_response_head_bytes {
            return Err(Error::Refused(Reason::new("the reply head is too long")));
        }
        let text = String::from_utf8_lossy(&bytes[..end]);
        let mut lines = text.split("\r\n");
        let status = lines
            .next()
            .filter(|line| line.starts_with("HTTP/"))
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|code| code.parse().ok())
            .ok_or_else(|| Error::Refused(Reason::new("the reply is not HTTP")))?;
        let headers: Vec<(String, String)> = lines
            .filter_map(|line| line.split_once(':'))
            .map(|(name, value)| (name.trim().to_string(), value.trim().to_string()))
            .collect();
        if headers.len() > settings.max_response_headers {
            return Err(Error::Refused(Reason::new(
                "the reply has too many headers",
            )));
        }
        Ok(Progress::Complete {
            value: Self { status, headers },
            consumed,
        })
    }

    /// The status code.
    #[must_use]
    pub fn status(&self) -> u16 {
        self.status
    }

    /// The first header called `name`, ignoring case.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.all(name).first().copied()
    }

    /// Every header called `name`, ignoring case, in order.
    #[must_use]
    pub fn all(&self, name: &str) -> Vec<&str> {
        self.headers
            .iter()
            .filter(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
            .collect()
    }
}

/// What a login decided after a reply.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Outcome {
    /// The tunnel is open.
    Established,
    /// Send the `CONNECT` again, on the same connection, with this `Proxy-Authorization`. The
    /// reply's body is drained first.
    Retry {
        /// The value to send.
        authorization: String,
    },
}

/// The login of one `CONNECT`: Basic from the proxy's credentials, or a Negotiate or NTLM
/// handshake that goes on over several replies.
pub struct ProxyLogin {
    host: String,
    has_credentials: bool,
    basic: Option<String>,
    negotiate: Option<Negotiate>,
    secure: bool,
    handshake: Option<(Scheme, Box<dyn Client>)>,
    answered: usize,
}

impl core::fmt::Debug for ProxyLogin {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ProxyLogin")
            .field("host", &self.host)
            .field("answered", &self.answered)
            .finish_non_exhaustive()
    }
}

impl ProxyLogin {
    /// A login to `proxy`; `secure` is whether the connection to it is TLS.
    #[must_use]
    pub fn new(proxy: &Proxy, negotiate: Option<&Negotiate>, secure: bool) -> Self {
        Self {
            host: proxy.host().to_string(),
            has_credentials: proxy.credentials().is_some(),
            basic: proxy.credentials().map(auth::basic_header),
            negotiate: negotiate.cloned(),
            secure,
            handshake: None,
            answered: 0,
        }
    }

    /// The `Proxy-Authorization` for the first request: Basic, when the proxy has credentials.
    #[must_use]
    pub fn first_authorization(&self) -> Option<String> {
        self.basic.clone()
    }

    /// Reads the proxy's reply to a request.
    ///
    /// # Errors
    ///
    /// [`Error::Refused`] for a status other than `200` and `407`; [`Error::Login`] when the
    /// proxy refuses the credentials, asks for a login that is not configured, closes the
    /// connection mid-login, or the handshake fails or takes too many legs.
    pub fn answer(&mut self, head: &ConnectResponse) -> Result<Outcome, Error> {
        match head.status() {
            200 => return self.finish(head).map(|()| Outcome::Established),
            407 => {}
            status => return Err(Error::Refused(Reason::new(format!("status {status}")))),
        }
        let max_legs = self
            .negotiate
            .as_ref()
            .map_or(auth::DEFAULT_MAX_LEGS, Negotiate::max_legs);
        if self.answered > max_legs {
            return Err(Error::Login(Reason::new("the login took too many legs")));
        }
        self.answered += 1;
        let challenges = auth::parse_challenges(head.all("proxy-authenticate"));
        if self.has_credentials {
            return Err(Error::Login(Reason::new(
                "the proxy refused the credentials",
            )));
        }
        let Some(policy) = self
            .negotiate
            .clone()
            .filter(|policy| policy.wants_proxy(&self.host))
        else {
            return Err(Error::Login(Reason::new(
                "the proxy asks for a login that is not configured",
            )));
        };
        let server_token = match &self.handshake {
            None => None,
            Some((scheme, _)) => challenges
                .iter()
                .find(|challenge| challenge.scheme == scheme.header_name().to_ascii_lowercase())
                .and_then(|challenge| challenge.token.as_deref())
                .map(|text| auth::decode_token(text, policy.max_token()))
                .transpose()?,
        };
        if self.handshake.is_none() {
            let scheme = Scheme::choose(&challenges).ok_or_else(|| {
                Error::Login(Reason::new("the proxy offers no scheme this client speaks"))
            })?;
            policy.check_transport(&self.host, self.secure)?;
            self.handshake = Some((
                scheme,
                auth::start(scheme, &self.host, policy.credentials())?,
            ));
        } else if server_token.is_none() {
            return Err(Error::Login(Reason::new(
                "the proxy refused the credentials",
            )));
        }
        if head
            .header("connection")
            .is_some_and(|value| value.eq_ignore_ascii_case("close"))
        {
            return Err(Error::Login(Reason::new(
                "the proxy closed the connection during the login",
            )));
        }
        let Some((scheme, client)) = self.handshake.as_mut() else {
            return Err(Error::Login(Reason::new("no handshake")));
        };
        let step = client.step(server_token.as_deref())?;
        let Some(token) = step.token else {
            return Err(Error::Login(Reason::new("the handshake produced no token")));
        };
        if token.len() > policy.max_token() {
            return Err(Error::Login(Reason::new("a token is over the limit")));
        }
        Ok(Outcome::Retry {
            authorization: auth::header_value(*scheme, &token),
        })
    }

    fn finish(&mut self, head: &ConnectResponse) -> Result<(), Error> {
        let Some((scheme, mut client)) = self.handshake.take() else {
            return Ok(());
        };
        // A mechanism that proves the server (Kerberos) sends its closing token with the 200.
        let max = self
            .negotiate
            .as_ref()
            .map_or(auth::DEFAULT_MAX_TOKEN, Negotiate::max_token);
        let closing = auth::parse_challenges(head.all("proxy-authenticate"))
            .into_iter()
            .find(|challenge| challenge.scheme == scheme.header_name().to_ascii_lowercase())
            .and_then(|challenge| challenge.token);
        if let Some(text) = closing {
            let token = auth::decode_token(&text, max)?;
            client.step(Some(&token))?;
        }
        Ok(())
    }
}

/// How the body of a reply is framed, so a driver can read past it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BodyFraming {
    /// This many bytes.
    Length(usize),
    /// `Transfer-Encoding: chunked`.
    Chunked,
}

impl ConnectResponse {
    /// How this reply's body is framed (no body is a length of zero).
    #[must_use]
    pub fn body_framing(&self) -> BodyFraming {
        if self
            .header("transfer-encoding")
            .is_some_and(|value| value.to_ascii_lowercase().contains("chunked"))
        {
            return BodyFraming::Chunked;
        }
        BodyFraming::Length(
            self.header("content-length")
                .and_then(|value| value.parse().ok())
                .unwrap_or(0),
        )
    }
}

/// Reads a chunk-size line (`1a3f;ext\r\n`): the size and the bytes the line took.
///
/// # Errors
///
/// [`Error::Login`] when the line is malformed or longer than 32 bytes.
pub fn decode_chunk_size(bytes: &[u8]) -> Result<Progress<usize>, Error> {
    let Some(end) = bytes.windows(2).position(|window| window == b"\r\n") else {
        if bytes.len() > 32 {
            return Err(Error::Login(Reason::new("the proxy's reply is malformed")));
        }
        return Ok(Progress::Incomplete(1));
    };
    let size = core::str::from_utf8(&bytes[..end])
        .ok()
        .and_then(|line| {
            usize::from_str_radix(line.trim().split(';').next().unwrap_or(""), 16).ok()
        })
        .ok_or_else(|| Error::Login(Reason::new("the proxy's reply is malformed")))?;
    Ok(Progress::Complete {
        value: size,
        consumed: end + 2,
    })
}
