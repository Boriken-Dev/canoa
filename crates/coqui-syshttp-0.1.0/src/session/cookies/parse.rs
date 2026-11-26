//! `Set-Cookie` as RFC 6265 section 5.2 reads it.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use coqui_proxy::Secret;

use super::{Cookie, HostAliases, is_ip, normal_host};
use crate::session::httpdate;

/// Second-level labels under a two-letter country code that are registries, not owners (`co.uk`):
/// the list `http.cookiejar` uses.
const REGISTRY_LABELS: [&str; 21] = [
    "co", "ac", "com", "edu", "org", "net", "gov", "mil", "int", "aero", "biz", "cat", "coop",
    "info", "jobs", "mobi", "museum", "name", "pro", "travel", "eu",
];

/// What parsing a `Set-Cookie` header needs besides its text.
#[derive(Debug, Clone)]
#[must_use]
pub struct SetCookieContext {
    now: Option<SystemTime>,
    request_path: String,
    aliases: HostAliases,
    max_age: Duration,
}

impl Default for SetCookieContext {
    fn default() -> Self {
        Self {
            now: None,
            request_path: "/".to_string(),
            aliases: HostAliases::default(),
            max_age: Duration::from_secs(400 * 24 * 3600),
        }
    }
}

impl SetCookieContext {
    /// The defaults: now, the path `/`, no aliases, a 400-day lifetime cap.
    pub fn new() -> Self {
        Self::default()
    }

    /// The moment a lifetime counts from (default: the time of the call).
    pub fn now(mut self, now: SystemTime) -> Self {
        self.now = Some(now);
        self
    }

    /// The path of the request the header answers, from which a cookie with no `Path` takes the
    /// directory (default `/`).
    pub fn request_path(mut self, path: impl Into<String>) -> Self {
        self.request_path = path.into();
        self
    }

    /// Groups of hosts that are one for cookies: a cookie a member sets with no `Domain`, or one
    /// naming a member, belongs to the group's first host.
    pub fn aliases(mut self, aliases: HostAliases) -> Self {
        self.aliases = aliases;
        self
    }

    /// The longest lifetime a cookie keeps, a later expiry being cut to it (400 days).
    pub fn max_age(mut self, max_age: Duration) -> Self {
        self.max_age = max_age;
        self
    }
}

fn has_control(text: &str) -> bool {
    text.chars()
        .any(|character| matches!(character, '\0'..='\u{8}' | '\n'..='\u{1f}' | '\u{7f}'))
}

fn ascii_int(text: &str) -> Option<i64> {
    let text = text.trim();
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

fn unix(time: SystemTime) -> i64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_secs()).unwrap_or(i64::MAX),
        Err(before) => -i64::try_from(before.duration().as_secs()).unwrap_or(i64::MAX),
    }
}

pub(crate) fn from_unix(seconds: i64) -> SystemTime {
    if seconds >= 0 {
        UNIX_EPOCH + Duration::from_secs(seconds.unsigned_abs())
    } else {
        UNIX_EPOCH - Duration::from_secs(seconds.unsigned_abs())
    }
}

fn is_public_suffix(domain: &str) -> bool {
    let labels: Vec<&str> = domain.split('.').collect();
    labels.len() == 1
        || (labels.len() == 2 && REGISTRY_LABELS.contains(&labels[0]) && labels[1].len() == 2)
}

/// RFC 6265 section 5.3 steps 4-6: the domain and whether it covers subdomains, for a cookie set
/// by `host` with the Domain `attribute`, or `None` to drop it.
fn cookie_domain(host: &str, attribute: Option<&str>) -> Option<(String, bool)> {
    let host = host.to_ascii_lowercase();
    let domain = attribute.unwrap_or("").trim();
    let domain = domain
        .strip_prefix('.')
        .unwrap_or(domain)
        .to_ascii_lowercase();
    if domain.is_empty() {
        return Some((host, false));
    }
    if is_ip(&host) {
        return (domain == host).then_some((host, false));
    }
    if domain.split('.').any(str::is_empty) || is_ip(&domain) {
        return None;
    }
    if host != domain && !host.ends_with(&alloc::format!(".{domain}")) {
        return None;
    }
    if is_public_suffix(&domain) {
        return (domain == host).then_some((host, false));
    }
    Some((domain, true))
}

/// [`cookie_domain`] with `host`'s alias group as one host: a cookie with no Domain, or a Domain
/// naming a host of the group, belongs to the group's first host. A Domain outside the group is
/// judged on `host`.
fn placed_for(
    host: &str,
    attribute: Option<&str>,
    aliases: &HostAliases,
) -> Option<(String, bool)> {
    let Some(target) = aliases.first(host) else {
        return cookie_domain(host, attribute);
    };
    let named = normal_host(attribute.unwrap_or("").trim().trim_start_matches('.'));
    if named.is_empty() {
        return Some((target.to_string(), false));
    }
    if aliases.first(&named) == Some(target) {
        return cookie_domain(target, Some(target));
    }
    cookie_domain(host, attribute)
}

/// RFC 6265 section 5.1.4: the directory of the request path.
fn default_path(request_path: &str) -> String {
    let path = request_path.split('?').next().unwrap_or("");
    let path = path.split('#').next().unwrap_or("");
    if !path.starts_with('/') || path.matches('/').count() == 1 {
        return "/".to_string();
    }
    path[..path.rfind('/').unwrap_or(0)].to_string()
}

struct Parsed {
    name: String,
    value: String,
    domain: Option<String>,
    path: Option<String>,
    expires: Option<i64>,
    secure: bool,
    http_only: bool,
}

/// RFC 6265 section 5.2: one header's name, value and attributes; `None` when it holds no cookie
/// or a control character (the tab apart), for which RFC 6265bis 5.6 and curl ignore the whole
/// cookie.
fn parse_header(header: &str, now: i64, max_age: i64) -> Option<Parsed> {
    if has_control(header) {
        return None;
    }
    let (pair, rest) = header.split_once(';').unwrap_or((header, ""));
    let (name, value) = pair.split_once('=')?;
    let (name, value) = (name.trim(), value.trim());
    if name.is_empty() {
        return None;
    }
    let mut parsed = Parsed {
        name: name.to_string(),
        value: value.to_string(),
        domain: None,
        path: None,
        expires: None,
        secure: false,
        http_only: false,
    };
    let mut expires: Option<i64> = None;
    let mut lifetime: Option<i64> = None;
    for item in rest.split(';') {
        let (key, attribute) = item.split_once('=').unwrap_or((item, ""));
        let (key, attribute) = (key.trim().to_ascii_lowercase(), attribute.trim());
        match key.as_str() {
            "expires" => {
                if let Some(when) = httpdate::parse(attribute) {
                    expires = Some(when);
                }
            }
            "max-age" => {
                let negative = attribute.starts_with('-');
                if let Some(seconds) = ascii_int(attribute.strip_prefix('-').unwrap_or(attribute)) {
                    lifetime = Some(if negative { -seconds } else { seconds });
                }
            }
            "domain" if !attribute.is_empty() => parsed.domain = Some(attribute.to_string()),
            "path" if !attribute.is_empty() => parsed.path = Some(attribute.to_string()),
            "secure" => parsed.secure = true,
            "httponly" => parsed.http_only = true,
            _ => {}
        }
    }
    if let Some(seconds) = lifetime {
        // Max-Age beats Expires, whatever the order.
        expires = Some(if seconds > 0 {
            now.saturating_add(seconds)
        } else {
            now
        });
    }
    parsed.expires = expires.map(|when| when.min(now.saturating_add(max_age)));
    Some(parsed)
}

/// `(cookie, gone)` for every cookie the `Set-Cookie` header `values` carry, read as RFC 6265
/// section 5.2 does.
///
/// `default_domain` is the host asked. A `Domain` attribute is kept only when that host
/// domain-matches it and it is not a public suffix (judged without the Public Suffix List: a top
/// level domain or a `co.uk`-style registry); it then covers subdomains. Without one the cookie is
/// host-only. A cookie with no `Path` gets the directory of the request path. `gone` marks a cookie
/// the server expires: an empty value, `Max-Age` of zero or less, a past date. A header that is not
/// a cookie, or names a foreign domain, is skipped. An expiry beyond the context's lifetime cap is
/// cut to it.
#[must_use]
pub fn parse_set_cookie<S: AsRef<str>>(
    values: impl IntoIterator<Item = S>,
    default_domain: &str,
    context: &SetCookieContext,
) -> Vec<(Cookie, bool)> {
    let now = unix(context.now.unwrap_or_else(SystemTime::now));
    let max_age = i64::try_from(context.max_age.as_secs()).unwrap_or(i64::MAX);
    let mut found = Vec::new();
    for header in values {
        let Some(parsed) = parse_header(header.as_ref(), now, max_age) else {
            continue;
        };
        let Some((domain, subdomains)) =
            placed_for(default_domain, parsed.domain.as_deref(), &context.aliases)
        else {
            continue;
        };
        let path = match parsed.path {
            Some(path) if path.starts_with('/') => path,
            _ => default_path(&context.request_path),
        };
        let gone = parsed.value.is_empty() || parsed.expires.is_some_and(|when| when <= now);
        found.push((
            Cookie {
                domain,
                path,
                secure: parsed.secure,
                expires: parsed.expires.map(from_unix),
                name: parsed.name,
                value: Secret::new(parsed.value),
                http_only: parsed.http_only,
                subdomains,
            },
            gone,
        ));
    }
    found
}
