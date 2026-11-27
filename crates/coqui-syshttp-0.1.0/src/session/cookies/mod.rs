//! Cookies as RFC 6265 and curl read them: the rows a client keeps, how they match a request and
//! how a `Set-Cookie` header makes them, and the Netscape cookie file they are stored in.
//!
//! The rules are those of Python syshttp: `Secure` cookies also go to loopback over plain http,
//! a `Secure` cookie is neither set, replaced nor deleted from plain http (RFC 6265bis 5.7),
//! `Max-Age` beats `Expires`, a lifetime is cut to a limit (400 days), and a public suffix is
//! a top level domain, a registry under a country code such as `co.uk`, a suffix the operator
//! lists, or (feature `psl`, on with `session`) an entry of the Public Suffix List.

mod aliases;
mod netscape;
mod parse;

use alloc::string::{String, ToString};
use core::net::IpAddr;
use std::time::SystemTime;

use coqui_proxy::Secret;
use url::Url;

pub use aliases::HostAliases;
pub use netscape::{load_netscape, load_netscape_with, save_netscape, save_netscape_with};
pub use parse::{SetCookieContext, parse_set_cookie};

/// A Netscape cookie file marks an HttpOnly cookie by prefixing its domain with this.
pub const HTTP_ONLY_PREFIX: &str = "#HttpOnly_";

/// Hosts that count as a secure context: `Secure` cookies go to them over plain http too.
const SECURE_HOSTS: [&str; 3] = ["localhost", "127.0.0.1", "::1"];

/// What [`save_netscape`] reads off a cookie.
pub trait CookieLike {
    /// The domain the cookie is for; a leading dot says it covers subdomains.
    fn domain(&self) -> &str;
    /// The path the cookie is for (`/` when empty).
    fn path(&self) -> &str;
    /// Whether the cookie goes over https only.
    fn secure(&self) -> bool;
    /// When the cookie expires, `None` for a session cookie.
    fn expires(&self) -> Option<SystemTime>;
    /// The cookie's name.
    fn name(&self) -> &str;
    /// The cookie's value.
    fn value(&self) -> &str;
    /// Whether the cookie is HttpOnly (written as curl writes it).
    fn http_only(&self) -> bool {
        false
    }
    /// The include-subdomains flag, for a domain written without the leading dot.
    fn subdomains(&self) -> bool {
        false
    }
}

/// One cookie. The value is a [`Secret`]: it never appears in `Debug` output, errors or logs.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Cookie {
    /// The domain, without a leading dot once loaded or parsed.
    pub domain: String,
    /// The path.
    pub path: String,
    /// Whether the cookie goes over https only.
    pub secure: bool,
    /// When it expires, `None` for a session cookie; whole seconds.
    pub expires: Option<SystemTime>,
    /// The name.
    pub name: String,
    /// The value.
    pub value: Secret,
    /// Written as curl writes it: the domain field prefixed `#HttpOnly_`.
    pub http_only: bool,
    /// The Netscape include-subdomains flag (`TRUE`), for a domain written without the leading
    /// dot that says the same.
    pub subdomains: bool,
}

impl core::fmt::Debug for Cookie {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("Cookie")
            .field("domain", &self.domain)
            .field("path", &self.path)
            .field("name", &self.name)
            .field("value", &"***")
            .finish_non_exhaustive()
    }
}

impl CookieLike for Cookie {
    fn domain(&self) -> &str {
        &self.domain
    }
    fn path(&self) -> &str {
        &self.path
    }
    fn secure(&self) -> bool {
        self.secure
    }
    fn expires(&self) -> Option<SystemTime> {
        self.expires
    }
    fn name(&self) -> &str {
        &self.name
    }
    fn value(&self) -> &str {
        self.value.expose()
    }
    fn http_only(&self) -> bool {
        self.http_only
    }
    fn subdomains(&self) -> bool {
        self.subdomains
    }
}

impl<T: CookieLike + ?Sized> CookieLike for &T {
    fn domain(&self) -> &str {
        (**self).domain()
    }
    fn path(&self) -> &str {
        (**self).path()
    }
    fn secure(&self) -> bool {
        (**self).secure()
    }
    fn expires(&self) -> Option<SystemTime> {
        (**self).expires()
    }
    fn name(&self) -> &str {
        (**self).name()
    }
    fn value(&self) -> &str {
        (**self).value()
    }
    fn http_only(&self) -> bool {
        (**self).http_only()
    }
    fn subdomains(&self) -> bool {
        (**self).subdomains()
    }
}

/// `host` as the cookie rules key it: lower case, without IPv6 brackets or a trailing dot.
pub(crate) fn normal_host(host: &str) -> String {
    let host = host.trim().to_ascii_lowercase();
    let host = host
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or(&host);
    host.strip_suffix('.').unwrap_or(host).to_string()
}

pub(crate) fn is_ip(host: &str) -> bool {
    host.parse::<IpAddr>().is_ok()
}

/// Whether a cookie `domain` applies to `host`: the host itself, or a subdomain when the domain
/// has a leading dot (the Netscape flag).
#[must_use]
pub fn domain_covers(domain: &str, host: &str) -> bool {
    let domain = domain.to_ascii_lowercase();
    let host = host.to_ascii_lowercase();
    if domain.is_empty() || host.is_empty() {
        return false;
    }
    let bare = domain.trim_start_matches('.');
    host == bare || (domain.starts_with('.') && host.ends_with(&alloc::format!(".{bare}")))
}

/// RFC 6265 path-match: `/admin` covers `/admin` and `/admin/x`, not `/adminx`.
pub(crate) fn path_matches(cookie_path: &str, path: &str) -> bool {
    if path == cookie_path || cookie_path == "/" {
        return true;
    }
    path.starts_with(cookie_path)
        && (cookie_path.ends_with('/') || path[cookie_path.len()..].starts_with('/'))
}

/// The host of `url` as the cookie rules read it (lower case, no brackets).
pub(crate) fn url_host(url: &Url) -> String {
    normal_host(url.host_str().unwrap_or(""))
}

impl Cookie {
    /// Whether this cookie goes with a request to `url`, as curl decides: the domain (the host
    /// itself, or a subdomain for a leading dot or the include-subdomains flag, never for an IP
    /// address, which only matches itself), the path (RFC 6265 path-match), `Secure` only over
    /// https or to localhost and loopback addresses, and not expired (`None` is a session cookie).
    #[must_use]
    pub fn applies_to(&self, url: &Url, now: SystemTime) -> bool {
        let host = url_host(url);
        let mut domain = self.domain.clone();
        if is_ip(&host) {
            if domain.trim_start_matches('.').to_ascii_lowercase() != host {
                return false;
            }
        } else {
            if self.subdomains && !domain.starts_with('.') {
                domain.insert(0, '.');
            }
            if !domain_covers(&domain, &host) {
                return false;
            }
        }
        let path = if url.path().is_empty() {
            "/"
        } else {
            url.path()
        };
        let own = if self.path.is_empty() {
            "/"
        } else {
            &self.path
        };
        if !path_matches(own, path) {
            return false;
        }
        if self.secure && url.scheme() != "https" && !SECURE_HOSTS.contains(&host.as_str()) {
            return false;
        }
        self.expires.is_none_or(|expires| expires > now)
    }
}
