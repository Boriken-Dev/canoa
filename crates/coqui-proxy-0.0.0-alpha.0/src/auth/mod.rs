//! Logging in to a proxy: Basic from given credentials, and Negotiate or NTLM (SSPI on Windows,
//! GSSAPI elsewhere) for the proxies a [`Negotiate`] policy names.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::net::IpAddr;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

use crate::error::{Error, Reason};
use crate::proxy::Credentials;

#[cfg(all(unix, feature = "negotiate"))]
mod gssapi;
mod origin;
pub use origin::{Answer, DEFAULT_MAX_BODY, OriginAuth, OriginAuthBuilder, OriginLogin};
#[cfg(all(windows, feature = "negotiate"))]
#[expect(
    unsafe_code,
    reason = "the SSPI calls have no safe binding; each block states its contract"
)]
mod sspi;

/// The most bytes of a Negotiate token sent or received unless the policy says otherwise.
pub const DEFAULT_MAX_TOKEN: usize = 65_536;
/// The most authenticated requests of one handshake unless the policy says otherwise.
pub const DEFAULT_MAX_LEGS: usize = 5;

/// `Authorization: Basic ...` for `credentials` (also the value of `Proxy-Authorization`).
pub fn basic_header(credentials: &Credentials) -> String {
    let pair = alloc::format!("{}:{}", credentials.username(), credentials.password());
    alloc::format!("Basic {}", STANDARD.encode(pair))
}

/// A header value naming a challenge scheme and an optional token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Challenge {
    pub scheme: String,
    pub token: Option<String>,
}

/// The challenges of `Proxy-Authenticate` (or `WWW-Authenticate`) header values: each value is a
/// comma-separated list that may hold several schemes; a bare token after `Negotiate` or `NTLM`
/// is the server's token.
pub(crate) fn parse_challenges<'a>(values: impl IntoIterator<Item = &'a str>) -> Vec<Challenge> {
    let mut challenges = Vec::new();
    for value in values {
        for part in value.split(',') {
            let part = part.trim();
            let mut words = part.splitn(2, char::is_whitespace);
            let first = words.next().unwrap_or("");
            let rest = words.next().map(str::trim).filter(|rest| !rest.is_empty());
            let is_scheme = ["negotiate", "ntlm", "basic", "digest", "bearer"]
                .iter()
                .any(|scheme| first.eq_ignore_ascii_case(scheme));
            if is_scheme {
                // Only these two schemes carry a token; `Basic realm="x"` carries parameters.
                let carries_token =
                    first.eq_ignore_ascii_case("negotiate") || first.eq_ignore_ascii_case("ntlm");
                let token = rest.filter(|_| carries_token);
                challenges.push(Challenge {
                    scheme: first.to_ascii_lowercase(),
                    token: token.map(str::to_string),
                });
            }
        }
    }
    challenges
}

/// Which proxies are logged in to with Negotiate or NTLM, and with what identity.
///
/// A policy names nobody by default. A proxy's host name is compared without case, brackets or a
/// trailing dot; `*.suffix` covers every host below a domain (not the domain itself) and `*`
/// covers all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Negotiate {
    proxies: Vec<String>,
    credentials: Option<Credentials>,
    plain_http: bool,
    max_legs: usize,
    max_token: usize,
}

/// Builds a [`Negotiate`] policy.
#[derive(Debug, Clone)]
#[must_use]
pub struct NegotiateBuilder {
    policy: Negotiate,
}

impl Negotiate {
    /// Starts a policy that logs in to nobody.
    pub fn builder() -> NegotiateBuilder {
        NegotiateBuilder {
            policy: Self {
                proxies: Vec::new(),
                credentials: None,
                plain_http: false,
                max_legs: DEFAULT_MAX_LEGS,
                max_token: DEFAULT_MAX_TOKEN,
            },
        }
    }

    /// Whether the proxy whose host name is `host` is logged in to.
    pub fn wants_proxy(&self, host: &str) -> bool {
        let host = normalise(host);
        self.proxies
            .iter()
            .any(|pattern| pattern_matches(pattern, &host))
    }

    /// The identity to log in with; `None` means the logon session (SSPI) or the Kerberos
    /// credential cache (GSSAPI).
    pub fn credentials(&self) -> Option<&Credentials> {
        self.credentials.as_ref()
    }

    /// Whether a handshake over plain `http:` to a host that is not loopback is allowed.
    pub fn plain_http(&self) -> bool {
        self.plain_http
    }

    /// The most authenticated requests of one handshake.
    pub fn max_legs(&self) -> usize {
        self.max_legs
    }

    /// The most bytes of a token, sent or received.
    pub fn max_token(&self) -> usize {
        self.max_token
    }

    /// Refuses a plain-http handshake with a host that is not loopback unless allowed.
    pub(crate) fn check_transport(&self, host: &str, secure: bool) -> Result<(), Error> {
        check_plain(self.plain_http, host, secure)
    }
}

/// Refuses a login sent over plain `http:` to a host that is not loopback, unless `allowed`.
pub(crate) fn check_plain(allowed: bool, host: &str, secure: bool) -> Result<(), Error> {
    if secure || allowed {
        return Ok(());
    }
    let host = normalise(host);
    let loopback = host == "localhost"
        || host.strip_suffix(".localhost").is_some()
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback());
    if loopback {
        Ok(())
    } else {
        Err(Error::Login(Reason::new(
            "a login over plain http is refused; allow it with plain_http(true)",
        )))
    }
}

/// Whether `pattern` (a normalised name, `*.suffix` or `*`) covers the normalised `host`.
pub(crate) fn pattern_matches(pattern: &str, host: &str) -> bool {
    pattern == "*"
        || pattern
            .strip_prefix("*.")
            .map_or(pattern == host, |suffix| {
                host.strip_suffix(suffix)
                    .is_some_and(|front| front.ends_with('.'))
            })
}

/// Whether `pattern` (already normalised) is a name, `*.suffix` or `*` a policy can hold.
pub(crate) fn pattern_is_valid(pattern: &str) -> bool {
    let name = pattern.strip_prefix("*.").unwrap_or(pattern);
    pattern == "*"
        || (!name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | ':')))
}

pub(crate) fn normalise(host: &str) -> String {
    host.trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.')
        .to_ascii_lowercase()
}

impl NegotiateBuilder {
    /// The proxies to log in to, by the host name of the proxy.
    pub fn proxies<I, S>(mut self, proxies: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.policy.proxies = proxies
            .into_iter()
            .map(|host| normalise(host.as_ref()))
            .collect();
        self
    }

    /// One identity for every host the policy names, as `DOMAIN\user` or `user@REALM`.
    pub fn credentials(mut self, credentials: Credentials) -> Self {
        self.policy.credentials = Some(credentials);
        self
    }

    /// Allows a handshake over plain `http:` to a host that is not loopback.
    pub fn plain_http(mut self, allow: bool) -> Self {
        self.policy.plain_http = allow;
        self
    }

    /// The most authenticated requests of one handshake (default 5).
    pub fn max_legs(mut self, legs: usize) -> Self {
        self.policy.max_legs = legs;
        self
    }

    /// The most bytes of a token, sent or received (default 65536).
    pub fn max_token(mut self, bytes: usize) -> Self {
        self.policy.max_token = bytes;
        self
    }

    /// The policy.
    ///
    /// # Errors
    ///
    /// [`Error::Parse`] for a host that is empty or holds a character no host name has, a
    /// limit of zero, or an identity with a line break.
    pub fn build(self) -> Result<Negotiate, Error> {
        let policy = self.policy;
        if !policy
            .proxies
            .iter()
            .all(|pattern| pattern_is_valid(pattern))
        {
            return Err(Error::Parse("negotiate host"));
        }
        if policy.max_legs == 0 || policy.max_token == 0 {
            return Err(Error::Parse("negotiate limit"));
        }
        if let Some(credentials) = &policy.credentials
            && (credentials.username().contains(['\r', '\n'])
                || credentials.password().contains(['\r', '\n']))
        {
            return Err(Error::Parse("negotiate identity"));
        }
        Ok(policy)
    }
}

/// Which mechanism a handshake runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scheme {
    Negotiate,
    Ntlm,
}

impl Scheme {
    pub(crate) fn header_name(self) -> &'static str {
        match self {
            Self::Negotiate => "Negotiate",
            Self::Ntlm => "NTLM",
        }
    }

    /// `Negotiate` when a reply offers it, else `NTLM`.
    pub(crate) fn choose(challenges: &[Challenge]) -> Option<Self> {
        let offers = |name: &str| challenges.iter().any(|challenge| challenge.scheme == name);
        if offers("negotiate") {
            Some(Self::Negotiate)
        } else if offers("ntlm") {
            Some(Self::Ntlm)
        } else {
            None
        }
    }
}

/// One step of a handshake.
#[derive(Debug)]
pub(crate) struct Step {
    /// The token to send, if any.
    pub token: Option<Vec<u8>>,
}

/// A client side of a handshake.
pub(crate) trait Client: Send {
    /// Takes the server's token (none for the first step) and returns what to send.
    fn step(&mut self, input: Option<&[u8]>) -> Result<Step, Error>;
}

/// Starts a handshake with the host `target`.
///
/// # Errors
///
/// [`Error::Login`] when this build or machine has no mechanism, or it refuses to start.
#[cfg(all(windows, feature = "negotiate"))]
pub(crate) fn start(
    scheme: Scheme,
    target: &str,
    credentials: Option<&Credentials>,
) -> Result<alloc::boxed::Box<dyn Client>, Error> {
    Ok(alloc::boxed::Box::new(sspi::SspiClient::new(
        scheme,
        target,
        credentials,
    )?))
}

/// Starts a handshake with the host `target`.
///
/// # Errors
///
/// [`Error::Login`] when the mechanism refuses to start.
#[cfg(all(unix, feature = "negotiate"))]
pub(crate) fn start(
    scheme: Scheme,
    target: &str,
    credentials: Option<&Credentials>,
) -> Result<alloc::boxed::Box<dyn Client>, Error> {
    Ok(alloc::boxed::Box::new(gssapi::GssapiClient::new(
        scheme,
        target,
        credentials,
    )?))
}

/// Starts a handshake with the host `target`.
///
/// # Errors
///
/// Always [`Error::Login`]: this build has no mechanism.
#[cfg(not(all(feature = "negotiate", any(windows, unix))))]
pub(crate) fn start(
    _scheme: Scheme,
    _target: &str,
    _credentials: Option<&Credentials>,
) -> Result<alloc::boxed::Box<dyn Client>, Error> {
    Err(Error::Login(Reason::new(
        "Negotiate needs the `negotiate` feature",
    )))
}

/// Decodes a base64 token a server sent, within `limit` bytes.
pub(crate) fn decode_token(text: &str, limit: usize) -> Result<Vec<u8>, Error> {
    let bytes = STANDARD
        .decode(text.trim())
        .map_err(|_| Error::Login(Reason::new("a server token is not base64")))?;
    if bytes.len() > limit {
        return Err(Error::Login(Reason::new(
            "a server token is over the limit",
        )));
    }
    Ok(bytes)
}

/// `Negotiate <base64>` or `NTLM <base64>`.
pub(crate) fn header_value(scheme: Scheme, token: &[u8]) -> String {
    alloc::format!("{} {}", scheme.header_name(), STANDARD.encode(token))
}

impl core::fmt::Display for Scheme {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(&self.header_name().to_ascii_lowercase())
    }
}

/// Whether the build can log in with Negotiate at all.
pub const fn negotiate_available() -> bool {
    cfg!(all(feature = "negotiate", any(windows, unix)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_is_the_base64_of_user_colon_password() {
        assert_eq!(
            basic_header(&Credentials::new("Aladdin", "open sesame")),
            "Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ=="
        );
    }

    #[test]
    fn a_policy_names_nobody_by_default_and_matches_proxies_by_name_and_suffix() {
        let policy = Negotiate::builder()
            .proxies(["proxy.corp.example", "*.edge.example"])
            .build()
            .unwrap();
        assert!(policy.wants_proxy("PROXY.corp.example."));
        assert!(policy.wants_proxy("a.edge.example"));
        assert!(!policy.wants_proxy("edge.example"));
        assert!(!policy.wants_proxy("other.example"));
        let everyone = Negotiate::builder().proxies(["*"]).build().unwrap();
        assert!(everyone.wants_proxy("anything"));
        let nobody = Negotiate::builder().build().unwrap();
        assert!(!nobody.wants_proxy("proxy.corp.example"));
    }

    #[test]
    fn a_bad_host_or_limit_is_refused_when_the_policy_is_built() {
        assert!(Negotiate::builder().proxies(["bad host"]).build().is_err());
        assert!(Negotiate::builder().proxies([""]).build().is_err());
        assert!(Negotiate::builder().max_legs(0).build().is_err());
        assert!(
            Negotiate::builder()
                .credentials(Credentials::new("u\n", "p"))
                .build()
                .is_err()
        );
    }

    #[test]
    fn a_handshake_over_plain_http_is_refused_unless_allowed_or_loopback() {
        let policy = Negotiate::builder().build().unwrap();
        assert!(policy.check_transport("proxy.corp", false).is_err());
        assert!(policy.check_transport("proxy.corp", true).is_ok());
        assert!(policy.check_transport("127.0.0.1", false).is_ok());
        assert!(policy.check_transport("localhost", false).is_ok());
        let allowed = Negotiate::builder().plain_http(true).build().unwrap();
        assert!(allowed.check_transport("proxy.corp", false).is_ok());
    }

    #[test]
    fn challenges_are_read_from_comma_separated_header_values() {
        let found = parse_challenges(["Negotiate abc123, NTLM", "Basic realm=\"x\""]);
        assert_eq!(
            found[0],
            Challenge {
                scheme: "negotiate".into(),
                token: Some("abc123".into())
            }
        );
        assert_eq!(
            found[1],
            Challenge {
                scheme: "ntlm".into(),
                token: None
            }
        );
        assert_eq!(
            found[2],
            Challenge {
                scheme: "basic".into(),
                token: None
            }
        );
        assert_eq!(Scheme::choose(&found), Some(Scheme::Negotiate));
        assert_eq!(
            Scheme::choose(&parse_challenges(["NTLM"])),
            Some(Scheme::Ntlm)
        );
        assert_eq!(
            Scheme::choose(&parse_challenges(["Basic realm=\"x\""])),
            None
        );
    }

    #[test]
    fn a_token_keeps_its_base64_padding() {
        let found = parse_challenges(["NTLM TlRMTVNTUAACAAAA=="]);
        assert_eq!(found[0].token.as_deref(), Some("TlRMTVNTUAACAAAA=="));
    }

    #[test]
    fn a_server_token_that_is_not_base64_or_too_long_is_refused() {
        assert!(decode_token("***", 100).is_err());
        assert!(decode_token(&STANDARD.encode([0u8; 10]), 5).is_err());
        assert_eq!(decode_token(&STANDARD.encode(b"hi"), 5).unwrap(), b"hi");
    }
}
