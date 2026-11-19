//! Proxies, the routes a lookup answers with, and the text forms that name them.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;
use core::net::IpAddr;
use core::str::FromStr;

use http::uri::Authority;
use percent_encoding::{NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};

use crate::error::Error;

/// The protocol a proxy speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ProxyScheme {
    /// An HTTP proxy reached in the clear: `CONNECT` for `https:` origins.
    Http,
    /// An HTTP proxy reached over TLS.
    Https,
    /// SOCKS version 4 (and 4a).
    Socks4,
    /// SOCKS version 5.
    Socks5,
}

impl ProxyScheme {
    /// The port a proxy of this kind listens on when none is written.
    pub const fn default_port(self) -> u16 {
        match self {
            Self::Http => 80,
            Self::Https => 443,
            Self::Socks4 | Self::Socks5 => 1080,
        }
    }

    /// The scheme as it is written in a proxy URL.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Https => "https",
            Self::Socks4 => "socks4",
            Self::Socks5 => "socks5",
        }
    }

    fn from_text(text: &str) -> Option<Self> {
        match text.to_ascii_lowercase().as_str() {
            "http" | "proxy" => Some(Self::Http),
            "https" => Some(Self::Https),
            "socks" | "socks4" | "socks4a" => Some(Self::Socks4),
            "socks5" | "socks5h" => Some(Self::Socks5),
            _ => None,
        }
    }
}

impl fmt::Display for ProxyScheme {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A secret text: it prints as `***` and is read only through [`Secret::expose`].
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Secret(String);

impl Secret {
    /// Wraps `value`.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The secret itself.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("***")
    }
}

/// A user name and password for a proxy (or a server). The password never appears in `Debug`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Credentials {
    username: String,
    password: Secret,
}

impl Credentials {
    /// A user name and password. A Windows account is `DOMAIN\user` or `user@REALM`.
    pub fn new(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            username: username.into(),
            password: Secret::new(password),
        }
    }

    /// The user name.
    pub fn username(&self) -> &str {
        &self.username
    }

    /// The password.
    pub fn password(&self) -> &str {
        self.password.expose()
    }
}

/// One proxy: where it is, how to speak to it and what to log in with.
///
/// `Display` is `scheme://host:port`, never the credentials; `Debug` shows the names of the
/// extra headers and masks the password.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Proxy {
    scheme: ProxyScheme,
    host: String,
    port: u16,
    credentials: Option<Credentials>,
    headers: Vec<(String, Secret)>,
}

impl Proxy {
    /// Starts a proxy at `host:port`, to which credentials and headers may be added.
    pub fn builder(scheme: ProxyScheme, host: impl Into<String>, port: u16) -> ProxyBuilder {
        ProxyBuilder {
            proxy: Self {
                scheme,
                host: host.into(),
                port,
                credentials: None,
                headers: Vec::new(),
            },
        }
    }

    /// The protocol the proxy speaks.
    pub fn scheme(&self) -> ProxyScheme {
        self.scheme
    }

    /// The proxy's host name or address, without brackets.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// The proxy's port.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The user name and password, if the proxy has any.
    pub fn credentials(&self) -> Option<&Credentials> {
        self.credentials.as_ref()
    }

    /// The headers sent to this proxy only: in its `CONNECT`, never through the tunnel.
    pub fn headers(&self) -> impl Iterator<Item = (&str, &str)> {
        self.headers
            .iter()
            .map(|(name, value)| (name.as_str(), value.expose()))
    }

    /// This proxy with `credentials`, unless it already has some.
    #[must_use]
    pub(crate) fn or_credentials(mut self, credentials: Option<&Credentials>) -> Self {
        if self.credentials.is_none() {
            self.credentials = credentials.cloned();
        }
        self
    }

    /// `host:port`, with an IPv6 address bracketed.
    pub fn authority(&self) -> String {
        join_host_port(&self.host, self.port)
    }

    /// Reads a proxy URL: `[scheme://][user[:password]@]host[:port][/]`.
    ///
    /// A missing scheme is `http`; `socks` is SOCKS 4; the port defaults per scheme
    /// ([`ProxyScheme::default_port`]); credentials are percent-decoded. A path other than `/`
    /// is an error: that is a PAC location, not a proxy.
    ///
    /// # Errors
    ///
    /// [`Error::Parse`] for an unknown scheme, an empty host, a bad port or a path.
    pub fn parse(text: &str) -> Result<Self, Error> {
        let text = text.trim();
        let (scheme, rest) = match text.split_once("://") {
            Some((scheme, rest)) => (
                ProxyScheme::from_text(scheme).ok_or(Error::Parse("proxy scheme"))?,
                rest,
            ),
            None => (ProxyScheme::Http, text),
        };
        let rest = rest.strip_suffix('/').unwrap_or(rest);
        if rest.contains('/') || rest.contains('?') || rest.contains('#') {
            return Err(Error::Parse("proxy: it has a path"));
        }
        let (userinfo, hostport) = match rest.rsplit_once('@') {
            Some((userinfo, hostport)) => (Some(userinfo), hostport),
            None => (None, rest),
        };
        let authority = Authority::from_str(hostport).map_err(|_| Error::Parse("proxy host"))?;
        let host = authority.host();
        let host = host
            .strip_prefix('[')
            .and_then(|inner| inner.strip_suffix(']'))
            .unwrap_or(host);
        if host.is_empty() {
            return Err(Error::Parse("proxy host"));
        }
        let port = match authority.port() {
            Some(port) => {
                let port = port.as_u16();
                if port == 0 {
                    return Err(Error::Parse("proxy port"));
                }
                port
            }
            None if hostport.ends_with(':') => return Err(Error::Parse("proxy port")),
            None => scheme.default_port(),
        };
        let credentials = userinfo.map(|userinfo| {
            let (user, password) = userinfo.split_once(':').unwrap_or((userinfo, ""));
            Credentials::new(decode(user), decode(password))
        });
        Ok(Self {
            scheme,
            host: host.to_ascii_lowercase(),
            port,
            credentials,
            headers: Vec::new(),
        })
    }

    /// The proxy URL with its credentials percent-encoded: for a child process's environment.
    pub fn url_with_credentials(&self) -> String {
        match &self.credentials {
            Some(credentials) => alloc::format!(
                "{}://{}:{}@{}",
                self.scheme,
                utf8_percent_encode(credentials.username(), NON_ALPHANUMERIC),
                utf8_percent_encode(credentials.password(), NON_ALPHANUMERIC),
                self.authority()
            ),
            None => self.to_string(),
        }
    }
}

fn decode(text: &str) -> String {
    percent_decode_str(text).decode_utf8_lossy().into_owned()
}

impl fmt::Display for Proxy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}://{}", self.scheme, self.authority())
    }
}

impl fmt::Debug for Proxy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut shown = formatter.debug_struct("Proxy");
        shown.field("url", &format_args!("{self}"));
        if let Some(credentials) = &self.credentials {
            shown.field("credentials", credentials);
        }
        if !self.headers.is_empty() {
            let names: Vec<&str> = self.headers.iter().map(|(name, _)| name.as_str()).collect();
            shown.field("headers", &names);
        }
        shown.finish()
    }
}

impl FromStr for Proxy {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self, Error> {
        Self::parse(text)
    }
}

/// Builds a [`Proxy`] with credentials and headers.
#[derive(Debug, Clone)]
#[must_use]
pub struct ProxyBuilder {
    proxy: Proxy,
}

impl ProxyBuilder {
    /// The user name and password to log in with.
    pub fn credentials(mut self, credentials: Credentials) -> Self {
        self.proxy.credentials = Some(credentials);
        self
    }

    /// A header sent to the proxy only (`Proxy-Authorization: Bearer ...`, for one). Names must
    /// be HTTP tokens and values free of line breaks; the proxy builds without a bad one and
    /// [`ProxyBuilder::build`] says which kind it was.
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.proxy.headers.push((name.into(), Secret::new(value)));
        self
    }

    /// The proxy.
    ///
    /// # Errors
    ///
    /// [`Error::Parse`] for an empty host, a zero port, a header name that is not a token or a
    /// header value with a line break or NUL.
    pub fn build(self) -> Result<Proxy, Error> {
        let proxy = self.proxy;
        if proxy.host.is_empty() {
            return Err(Error::Parse("proxy host"));
        }
        if proxy.port == 0 {
            return Err(Error::Parse("proxy port"));
        }
        for (name, value) in &proxy.headers {
            if http::header::HeaderName::from_bytes(name.as_bytes()).is_err()
                || value
                    .expose()
                    .bytes()
                    .any(|byte| matches!(byte, b'\r' | b'\n' | 0))
            {
                return Err(Error::Parse("proxy header"));
            }
        }
        Ok(proxy)
    }
}

/// The way to reach a URL: straight to its server, or through a proxy.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Route {
    /// Connect to the server itself.
    Direct,
    /// Connect through this proxy.
    Proxy(Proxy),
}

impl Route {
    /// Reads a PAC `FindProxyForURL` result: `PROXY a:1; SOCKS5 b:2; DIRECT`.
    ///
    /// Keywords are case-insensitive: `DIRECT`, `PROXY`, `HTTP`, `HTTPS`, `SOCKS` (SOCKS 4),
    /// `SOCKS4`, `SOCKS5`. An entry with an unknown keyword or no usable host is dropped; when
    /// nothing usable is left the answer is `[Route::Direct]`, as in Chromium.
    pub fn parse_pac_result(text: &str) -> Vec<Self> {
        let mut routes = Vec::new();
        for entry in text.split(';') {
            let entry = entry.trim();
            if entry.is_empty() {
                continue;
            }
            let (keyword, target) = entry
                .split_once(char::is_whitespace)
                .map_or((entry, ""), |(keyword, target)| (keyword, target.trim()));
            if keyword.eq_ignore_ascii_case("direct") {
                routes.push(Self::Direct);
                continue;
            }
            let scheme = match keyword.to_ascii_uppercase().as_str() {
                "PROXY" | "HTTP" => Some(ProxyScheme::Http),
                "HTTPS" => Some(ProxyScheme::Https),
                "SOCKS" | "SOCKS4" => Some(ProxyScheme::Socks4),
                "SOCKS5" => Some(ProxyScheme::Socks5),
                _ => None,
            };
            let proxy =
                scheme.and_then(|scheme| Proxy::parse(&alloc::format!("{scheme}://{target}")).ok());
            match proxy {
                Some(proxy) => routes.push(Self::Proxy(proxy)),
                None => tracing::debug!("dropped an unusable entry of a pac result"),
            }
        }
        if routes.is_empty() {
            routes.push(Self::Direct);
        }
        routes
    }
}

/// `host:port`, an IPv6 host bracketed.
pub(crate) fn join_host_port(host: &str, port: u16) -> String {
    if host.contains(':') {
        alloc::format!("[{host}]:{port}")
    } else {
        alloc::format!("{host}:{port}")
    }
}

/// A URL a lookup is asked about: its scheme, host and port, as the rules read them.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct Target {
    /// Lower case.
    pub scheme: String,
    /// Lower case, an IPv6 address without brackets.
    pub host: String,
    /// The written port, else the scheme's default.
    pub port: u16,
}

impl Target {
    pub(crate) fn parse(url: &str) -> Result<Self, Error> {
        let uri = http::Uri::from_str(url.trim()).map_err(|_| Error::Parse("url"))?;
        let scheme = uri
            .scheme_str()
            .ok_or(Error::Parse("url: it has no scheme"))?
            .to_ascii_lowercase();
        let host = uri.host().ok_or(Error::Parse("url: it has no host"))?;
        let host = host
            .strip_prefix('[')
            .and_then(|inner| inner.strip_suffix(']'))
            .unwrap_or(host)
            .to_ascii_lowercase();
        let port = uri
            .port_u16()
            .or_else(|| default_port(&scheme))
            .unwrap_or(0);
        Ok(Self { scheme, host, port })
    }

    pub(crate) fn address(&self) -> Option<IpAddr> {
        self.host.split('%').next()?.parse().ok()
    }
}

/// The port a scheme uses when a URL writes none.
pub(crate) fn default_port(scheme: &str) -> Option<u16> {
    match scheme {
        "http" | "ws" => Some(80),
        "https" | "wss" => Some(443),
        "ftp" => Some(21),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_proxy_url_without_a_scheme_is_an_http_proxy_on_the_schemes_default_port() {
        let proxy = Proxy::parse("proxy.corp").unwrap();
        assert_eq!(proxy.to_string(), "http://proxy.corp:80");
        assert_eq!(Proxy::parse("https://p").unwrap().port(), 443);
        assert_eq!(Proxy::parse("socks5://p").unwrap().port(), 1080);
    }

    #[test]
    fn credentials_in_a_proxy_url_are_percent_decoded_and_never_displayed() {
        let proxy = Proxy::parse("http://dom%5Cuser:p%40ss@proxy:3128").unwrap();
        let credentials = proxy.credentials().unwrap();
        assert_eq!(credentials.username(), "dom\\user");
        assert_eq!(credentials.password(), "p@ss");
        assert!(!proxy.to_string().contains("ss"));
        assert!(!alloc::format!("{proxy:?}").contains("p@ss"));
        assert_eq!(
            proxy.url_with_credentials(),
            "http://dom%5Cuser:p%40ss@proxy:3128"
        );
    }

    #[test]
    fn an_ipv6_proxy_is_bracketed_and_a_path_is_refused() {
        let proxy = Proxy::parse("[::1]:8080").unwrap();
        assert_eq!(proxy.host(), "::1");
        assert_eq!(proxy.authority(), "[::1]:8080");
        assert!(Proxy::parse("http://proxy/pac.dat").is_err());
        assert!(Proxy::parse("http://proxy:0").is_err());
        assert!(Proxy::parse("ftp://proxy").is_err());
        assert!(Proxy::parse("http://").is_err());
    }

    #[test]
    fn a_pac_result_is_read_in_order_with_unknown_entries_dropped() {
        let routes = Route::parse_pac_result("PROXY a:1; bogus b:2; SOCKS5 c:3 ; direct");
        assert_eq!(routes.len(), 3);
        assert!(matches!(&routes[0], Route::Proxy(p) if p.host() == "a" && p.port() == 1));
        assert!(
            matches!(&routes[1], Route::Proxy(p) if p.scheme() == ProxyScheme::Socks5 && p.port() == 3)
        );
        assert_eq!(routes[2], Route::Direct);
    }

    #[test]
    fn a_pac_result_with_nothing_usable_is_direct() {
        assert_eq!(Route::parse_pac_result(""), [Route::Direct]);
        assert_eq!(Route::parse_pac_result("bogus x:1"), [Route::Direct]);
    }

    #[test]
    fn a_header_that_is_not_a_token_is_refused_when_the_proxy_is_built() {
        let built = Proxy::builder(ProxyScheme::Http, "p", 1)
            .header("bad name", "x")
            .build();
        assert!(built.is_err());
        let built = Proxy::builder(ProxyScheme::Http, "p", 1)
            .header("X-Token", "a\r\nb")
            .build();
        assert!(built.is_err());
        let ok = Proxy::builder(ProxyScheme::Http, "p", 1)
            .header("X-Token", "secret")
            .build()
            .unwrap();
        assert_eq!(ok.headers().next(), Some(("X-Token", "secret")));
        assert!(!alloc::format!("{ok:?}").contains("secret"));
    }

    #[test]
    fn a_target_takes_the_default_port_of_its_scheme() {
        let target = Target::parse("https://Example.COM/a?b").unwrap();
        assert_eq!((target.host.as_str(), target.port), ("example.com", 443));
        let target = Target::parse("http://[::1]:8080/").unwrap();
        assert_eq!((target.host.as_str(), target.port), ("::1", 8080));
        assert!(Target::parse("/relative").is_err());
    }
}
