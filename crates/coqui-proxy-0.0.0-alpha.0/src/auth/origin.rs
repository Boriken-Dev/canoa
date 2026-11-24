//! Logging in to an origin server: Basic, Negotiate and NTLM, answered to a `401`.
//!
//! This is the sans-IO half: [`OriginLogin`] is told the `WWW-Authenticate` values of a `401` and
//! says what `Authorization` to retry with. An HTTP client drives it.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use super::{
    Challenge, Client, DEFAULT_MAX_LEGS, DEFAULT_MAX_TOKEN, Scheme, basic_header, check_plain,
    decode_token, header_value, normalise, parse_challenges, pattern_is_valid, pattern_matches,
};
use crate::error::{Error, Reason};
use crate::proxy::{Credentials, Target};

/// How a rule answers a challenge.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Method {
    /// `Basic` with these credentials.
    Basic(Credentials),
    /// `Negotiate` or `NTLM`, as the logon session (`None`) or as these credentials.
    Negotiate(Option<Credentials>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Rule {
    pattern: String,
    method: Method,
}

/// Which origin servers are logged in to, and how.
///
/// A policy names nobody by default. A server's host name is compared without case, brackets or a
/// trailing dot; `*.suffix` covers every host below a domain (not the domain itself) and `*` covers
/// all. A host may match several rules: a `401` offering `Negotiate` (preferred) or `NTLM` is
/// answered by a Negotiate rule, one offering `Basic` by a Basic rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OriginAuth {
    rules: Vec<Rule>,
    plain_http: bool,
    max_legs: usize,
    max_token: usize,
    max_body: usize,
}

/// Builds an [`OriginAuth`].
#[derive(Debug, Clone)]
#[must_use]
pub struct OriginAuthBuilder {
    auth: OriginAuth,
    hosts: Vec<(Vec<String>, Method)>,
}

/// The most bytes of a `401` reply's body read and dropped between two legs, unless the policy
/// says otherwise.
pub const DEFAULT_MAX_BODY: usize = 1 << 20;

impl OriginAuth {
    /// Starts a policy that logs in to nobody.
    pub fn builder() -> OriginAuthBuilder {
        OriginAuthBuilder {
            auth: Self {
                rules: Vec::new(),
                plain_http: false,
                max_legs: DEFAULT_MAX_LEGS,
                max_token: DEFAULT_MAX_TOKEN,
                max_body: DEFAULT_MAX_BODY,
            },
            hosts: Vec::new(),
        }
    }

    /// Whether the server whose host name is `host` has a rule.
    pub fn wants_host(&self, host: &str) -> bool {
        let host = normalise(host);
        self.rules
            .iter()
            .any(|rule| pattern_matches(&rule.pattern, &host))
    }

    /// What this policy allows that is looser than the defaults; empty when strict.
    #[must_use]
    pub fn non_defaults(&self) -> Vec<String> {
        let mut loose = Vec::new();
        if self.plain_http {
            loose.push("OriginAuth::plain_http(true): a login may cross plain http".to_string());
        }
        loose
    }

    /// Whether a login over plain `http:` to a host that is not loopback is allowed.
    pub fn plain_http(&self) -> bool {
        self.plain_http
    }

    /// The most authenticated requests of one login.
    pub fn max_legs(&self) -> usize {
        self.max_legs
    }

    /// The most bytes of a token, sent or received.
    pub fn max_token(&self) -> usize {
        self.max_token
    }

    /// The most bytes of a `401` reply's body read and dropped between two legs.
    pub fn max_body(&self) -> usize {
        self.max_body
    }

    /// A login for the server of `url`, or `None` when no rule covers its host.
    ///
    /// # Errors
    ///
    /// [`Error::Parse`] for a URL without a scheme and host.
    pub fn login(&self, url: &str) -> Result<Option<OriginLogin>, Error> {
        let target = Target::parse(url)?;
        let methods: Vec<Method> = self
            .rules
            .iter()
            .filter(|rule| pattern_matches(&rule.pattern, &normalise(&target.host)))
            .map(|rule| rule.method.clone())
            .collect();
        if methods.is_empty() {
            return Ok(None);
        }
        Ok(Some(OriginLogin {
            host: target.host,
            secure: matches!(target.scheme.as_str(), "https" | "wss"),
            methods,
            plain_http: self.plain_http,
            max_legs: self.max_legs,
            max_token: self.max_token,
            state: State::Idle,
            legs: 0,
        }))
    }
}

impl OriginAuthBuilder {
    /// Answers a `Basic` challenge from the servers `hosts` with `credentials`.
    pub fn basic<I, S>(mut self, hosts: I, credentials: Credentials) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.add(hosts, Method::Basic(credentials));
        self
    }

    /// Answers a `Negotiate` or `NTLM` challenge from the servers `hosts` as the logon session
    /// (SSPI) or the Kerberos credential cache (GSSAPI).
    pub fn negotiate<I, S>(mut self, hosts: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.add(hosts, Method::Negotiate(None));
        self
    }

    /// Answers a `Negotiate` or `NTLM` challenge from the servers `hosts` as `credentials`
    /// (`DOMAIN\user` or `user@REALM`).
    pub fn negotiate_as<I, S>(mut self, hosts: I, credentials: Credentials) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.add(hosts, Method::Negotiate(Some(credentials)));
        self
    }

    fn add<I, S>(&mut self, hosts: I, method: Method)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.hosts.push((
            hosts
                .into_iter()
                .map(|host| normalise(host.as_ref()))
                .collect(),
            method,
        ));
    }

    /// Allows a login over plain `http:` to a host that is not loopback: Basic then sends the
    /// password in the clear.
    pub fn plain_http(mut self, allow: bool) -> Self {
        self.auth.plain_http = allow;
        self
    }

    /// The most authenticated requests of one login (default 5).
    pub fn max_legs(mut self, legs: usize) -> Self {
        self.auth.max_legs = legs;
        self
    }

    /// The most bytes of a token, sent or received (default 65536).
    pub fn max_token(mut self, bytes: usize) -> Self {
        self.auth.max_token = bytes;
        self
    }

    /// The most bytes of a `401` reply's body read and dropped between two legs, so the
    /// connection can carry the next one (default 1 MiB).
    pub fn max_body(mut self, bytes: usize) -> Self {
        self.auth.max_body = bytes;
        self
    }

    /// The policy.
    ///
    /// # Errors
    ///
    /// [`Error::Parse`] for a host that is empty or holds a character no host name has, a limit
    /// of zero, or an identity with a line break.
    pub fn build(self) -> Result<OriginAuth, Error> {
        let mut auth = self.auth;
        for (hosts, method) in self.hosts {
            if hosts.iter().any(|pattern| !pattern_is_valid(pattern)) {
                return Err(Error::Parse("origin host"));
            }
            let credentials = match &method {
                Method::Basic(credentials) => Some(credentials),
                Method::Negotiate(credentials) => credentials.as_ref(),
            };
            if let Some(credentials) = credentials
                && (credentials.username().contains(['\r', '\n'])
                    || credentials.password().contains(['\r', '\n']))
            {
                return Err(Error::Parse("origin identity"));
            }
            for pattern in hosts {
                auth.rules.push(Rule {
                    pattern,
                    method: method.clone(),
                });
            }
        }
        if auth.max_legs == 0 || auth.max_token == 0 {
            return Err(Error::Parse("origin limit"));
        }
        Ok(auth)
    }
}

enum State {
    /// Nothing sent yet.
    Idle,
    /// Basic was sent; a second `401` means it was refused.
    Basic,
    /// A Negotiate or NTLM exchange is under way.
    Handshake(Scheme, Box<dyn Client>),
    /// The server refused, or the exchange is over.
    Done,
}

impl core::fmt::Debug for State {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Idle => "Idle",
            Self::Basic => "Basic",
            Self::Handshake(..) => "Handshake",
            Self::Done => "Done",
        })
    }
}

/// What to retry a request with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    authorization: String,
    probe: bool,
}

impl Answer {
    /// The value of the `Authorization` header.
    pub fn authorization(&self) -> &str {
        &self.authorization
    }

    /// Whether this leg should carry no request body: the first message of an NTLM exchange only
    /// opens the login, and the body goes with the last one. The method and headers stay.
    pub fn probe(&self) -> bool {
        self.probe
    }
}

/// One request's login to one server. It must answer every leg on the same connection: NTLM
/// authenticates the connection, not the request.
#[derive(Debug)]
pub struct OriginLogin {
    host: String,
    secure: bool,
    methods: Vec<Method>,
    plain_http: bool,
    max_legs: usize,
    max_token: usize,
    state: State,
    legs: usize,
}

impl OriginLogin {
    /// Answers a `401`: the `WWW-Authenticate` header values it carried.
    ///
    /// `Ok(None)` means there is nothing to retry with: the server offers no scheme a rule
    /// covers, or it refused what was sent. The `401` is then the answer.
    ///
    /// # Errors
    ///
    /// [`Error::Login`] when a mechanism cannot start or fails, a token is over the limit, the
    /// login takes more legs than allowed, or it would send a password over plain `http:` to a
    /// host that is not loopback.
    pub fn answer(&mut self, www_authenticate: &[&str]) -> Result<Option<Answer>, Error> {
        let challenges = parse_challenges(www_authenticate.iter().copied());
        match core::mem::replace(&mut self.state, State::Done) {
            State::Idle => self.begin(&challenges),
            State::Basic | State::Done => Ok(None),
            State::Handshake(scheme, mut client) => {
                self.legs += 1;
                if self.legs > self.max_legs {
                    return Err(Error::Login(Reason::new("the login took too many legs")));
                }
                let name = scheme.header_name().to_ascii_lowercase();
                let Some(text) = challenges
                    .iter()
                    .find(|challenge| challenge.scheme == name)
                    .and_then(|challenge| challenge.token.as_deref())
                else {
                    // A bare challenge after our token: the server did not accept it.
                    return Ok(None);
                };
                let token = decode_token(text, self.max_token)?;
                let Some(next) = client.step(Some(&token))?.token else {
                    return Ok(None);
                };
                self.check_size(&next)?;
                let answer = Answer {
                    authorization: header_value(scheme, &next),
                    probe: false,
                };
                self.state = State::Handshake(scheme, client);
                Ok(Some(answer))
            }
        }
    }

    /// Reads the `WWW-Authenticate` values of a reply that was not a `401`: a mechanism that
    /// proves the server too (Kerberos) sends its closing token there.
    ///
    /// # Errors
    ///
    /// [`Error::Login`] when the closing token is malformed or the server fails to prove itself.
    pub fn finish(&mut self, www_authenticate: &[&str]) -> Result<(), Error> {
        let State::Handshake(scheme, client) = &mut self.state else {
            return Ok(());
        };
        let name = scheme.header_name().to_ascii_lowercase();
        let closing = parse_challenges(www_authenticate.iter().copied())
            .into_iter()
            .find(|challenge| challenge.scheme == name)
            .and_then(|challenge| challenge.token);
        if let Some(text) = closing {
            let token = decode_token(&text, self.max_token)?;
            client.step(Some(&token))?;
        }
        Ok(())
    }

    fn begin(&mut self, challenges: &[Challenge]) -> Result<Option<Answer>, Error> {
        let mut failure = None;
        let negotiate = self.methods.iter().find_map(|method| match method {
            Method::Negotiate(credentials) => Some(credentials.clone()),
            Method::Basic(_) => None,
        });
        if let (Some(credentials), Some(scheme)) = (negotiate, Scheme::choose(challenges)) {
            match self.begin_handshake(scheme, credentials.as_ref()) {
                Ok(answer) => return Ok(Some(answer)),
                Err(error) => {
                    tracing::debug!(%error, "a Negotiate login did not start");
                    failure = Some(error);
                }
            }
        }
        let basic = self.methods.iter().find_map(|method| match method {
            Method::Basic(credentials) => Some(credentials.clone()),
            Method::Negotiate(_) => None,
        });
        if let Some(credentials) = basic
            && challenges
                .iter()
                .any(|challenge| challenge.scheme == "basic")
        {
            check_plain(self.plain_http, &self.host, self.secure)?;
            self.state = State::Basic;
            return Ok(Some(Answer {
                authorization: basic_header(&credentials),
                probe: false,
            }));
        }
        failure.map_or(Ok(None), Err)
    }

    fn begin_handshake(
        &mut self,
        scheme: Scheme,
        credentials: Option<&Credentials>,
    ) -> Result<Answer, Error> {
        check_plain(self.plain_http, &self.host, self.secure)?;
        let mut client = super::start(scheme, &self.host, credentials)?;
        let token = client
            .step(None)?
            .token
            .ok_or_else(|| Error::Login(Reason::new("the handshake produced no token")))?;
        self.check_size(&token)?;
        let answer = Answer {
            authorization: header_value(scheme, &token),
            // NTLM opens with a message that needs no body; Negotiate may be answered in one leg.
            probe: scheme == Scheme::Ntlm,
        };
        self.state = State::Handshake(scheme, client);
        Ok(answer)
    }

    fn check_size(&self, token: &[u8]) -> Result<(), Error> {
        if token.len() > self.max_token {
            Err(Error::Login(Reason::new("a token is over the limit")))
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn basic_only() -> OriginAuth {
        OriginAuth::builder()
            .basic(
                ["mail.corp.example"],
                Credentials::new("Aladdin", "open sesame"),
            )
            .plain_http(true)
            .build()
            .unwrap()
    }

    #[test]
    fn a_policy_names_nobody_by_default_and_matches_hosts_by_name_and_suffix() {
        let none = OriginAuth::builder().build().unwrap();
        assert!(none.login("https://mail.corp.example/").unwrap().is_none());
        let auth = OriginAuth::builder()
            .negotiate(["mail.corp.example", "*.edge.example"])
            .build()
            .unwrap();
        assert!(auth.wants_host("MAIL.corp.example."));
        assert!(auth.wants_host("a.edge.example"));
        assert!(!auth.wants_host("edge.example"));
        assert!(auth.login("https://a.edge.example/ews").unwrap().is_some());
        assert!(auth.login("https://other.example/").unwrap().is_none());
    }

    #[test]
    fn a_bad_host_limit_or_identity_is_refused_when_the_policy_is_built() {
        assert!(
            OriginAuth::builder()
                .negotiate(["bad host"])
                .build()
                .is_err()
        );
        assert!(OriginAuth::builder().negotiate([""]).build().is_err());
        assert!(OriginAuth::builder().max_legs(0).build().is_err());
        assert!(
            OriginAuth::builder()
                .basic(["a"], Credentials::new("u\n", "p"))
                .build()
                .is_err()
        );
    }

    #[test]
    fn a_basic_challenge_is_answered_once_and_a_second_one_means_refused() {
        let mut login = basic_only()
            .login("https://mail.corp.example/ews")
            .unwrap()
            .unwrap();
        let answer = login
            .answer(&["Basic realm=\"mail\""])
            .unwrap()
            .expect("an answer");
        assert_eq!(answer.authorization(), "Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ==");
        assert!(!answer.probe());
        assert!(login.answer(&["Basic realm=\"mail\""]).unwrap().is_none());
    }

    #[test]
    fn a_challenge_no_rule_answers_gets_no_answer() {
        let mut login = basic_only()
            .login("https://mail.corp.example/")
            .unwrap()
            .unwrap();
        assert!(login.answer(&["Bearer realm=\"x\""]).unwrap().is_none());
        let mut login = basic_only()
            .login("https://mail.corp.example/")
            .unwrap()
            .unwrap();
        assert!(login.answer(&["Negotiate", "NTLM"]).unwrap().is_none());
    }

    #[test]
    fn basic_over_plain_http_is_refused_unless_allowed_or_loopback() {
        let auth = OriginAuth::builder()
            .basic(["*"], Credentials::new("u", "p"))
            .build()
            .unwrap();
        let mut login = auth.login("http://mail.corp.example/").unwrap().unwrap();
        assert!(matches!(
            login.answer(&["Basic realm=\"x\""]),
            Err(Error::Login(_))
        ));
        let mut login = auth.login("http://127.0.0.1:8080/").unwrap().unwrap();
        assert!(login.answer(&["Basic realm=\"x\""]).unwrap().is_some());
        let mut login = auth.login("https://mail.corp.example/").unwrap().unwrap();
        assert!(login.answer(&["Basic realm=\"x\""]).unwrap().is_some());
    }

    #[cfg(not(all(feature = "negotiate", any(windows, unix))))]
    #[test]
    fn without_a_mechanism_negotiate_falls_back_to_basic_or_fails() {
        let both = OriginAuth::builder()
            .negotiate(["*"])
            .basic(["*"], Credentials::new("u", "p"))
            .build()
            .unwrap();
        let mut login = both.login("https://h.example/").unwrap().unwrap();
        let answer = login
            .answer(&["Negotiate", "Basic realm=\"x\""])
            .unwrap()
            .unwrap();
        assert!(answer.authorization().starts_with("Basic "));
        let only = OriginAuth::builder().negotiate(["*"]).build().unwrap();
        let mut login = only.login("https://h.example/").unwrap().unwrap();
        assert!(matches!(login.answer(&["Negotiate"]), Err(Error::Login(_))));
    }
}
