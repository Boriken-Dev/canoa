//! What a client remembers between requests: the cookies of every host it talked to, kept in
//! memory and, with a jar, on disk by host; and the tokens that become `Authorization` headers.
//!
//! Both are chosen by the host of the URL being requested, the origin, never by a proxy's or a
//! gateway's: they sit above the transport.

use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::SystemTime;

use coqui_proxy::Secret;
use url::Url;

use super::cookies::{Cookie, HostAliases, SetCookieContext, domain_covers, parse_set_cookie};
use super::cookies::{is_ip, path_matches, url_host};
use super::error::Error;
use super::jar::{CookieJar, TokenJar};
use super::settings::CookieSettings;

const SECURE_HOSTS: [&str; 3] = ["localhost", "127.0.0.1", "::1"];

/// The key of a jar entry: the origin's host, or one name for every host.
#[derive(Debug, Clone)]
pub(crate) enum JarKey {
    ByHost,
    Named(String),
}

#[derive(Default)]
struct Rows {
    /// Rows in the order they were first kept; the key is `(domain, path, name)`.
    list: Vec<Cookie>,
    loaded: HashSet<String>,
    synced: HashMap<String, HashSet<Cookie>>,
}

impl Rows {
    fn position(&self, domain: &str, path: &str, name: &str) -> Option<usize> {
        self.list
            .iter()
            .position(|row| row.domain == domain && row.path == path && row.name == name)
    }

    fn put(&mut self, cookie: Cookie) {
        match self.position(&cookie.domain, &cookie.path, &cookie.name) {
            Some(index) => self.list[index] = cookie,
            None => self.list.push(cookie),
        }
    }
}

/// The cookies of one client.
///
/// Rows are [`Cookie`]s, set from `Set-Cookie` as RFC 6265 reads it and sent as
/// [`Cookie::applies_to`] decides. With a jar, the rows of a host are loaded from it the first
/// time that host is asked for, and written back only when they differ from what was loaded or
/// written last.
pub(crate) struct Cookies {
    jar: Option<Arc<dyn CookieJar>>,
    key: JarKey,
    aliases: HostAliases,
    settings: CookieSettings,
    rows: Mutex<Rows>,
}

fn covers(row: &Cookie, host: &str) -> bool {
    // A row's domain is bare; its include-subdomains flag is the dot.
    if row.subdomains {
        domain_covers(&alloc::format!(".{}", row.domain), host)
    } else {
        domain_covers(&row.domain, host)
    }
}

impl Cookies {
    pub(crate) fn new(
        jar: Option<Arc<dyn CookieJar>>,
        key: JarKey,
        aliases: HostAliases,
        settings: CookieSettings,
    ) -> Self {
        Self {
            jar,
            key,
            aliases,
            settings,
            rows: Mutex::new(Rows::default()),
        }
    }

    fn host(&self, url: &Url) -> String {
        let host = url_host(url);
        self.aliases.first(&host).map_or(host, str::to_string)
    }

    fn keys(&self, url: &Url) -> Vec<String> {
        match &self.key {
            JarKey::Named(name) => alloc::vec![name.clone()],
            JarKey::ByHost => {
                let host = self.host(url);
                if host.is_empty() {
                    Vec::new()
                } else {
                    alloc::vec![host]
                }
            }
        }
    }

    /// `url`, and the same URL for every other host of its alias group.
    fn urls(&self, url: &Url) -> Vec<Url> {
        let host = url_host(url);
        let mut urls = alloc::vec![url.clone()];
        if let Some(first) = self.aliases.first(&host) {
            for member in self.aliases.members(first) {
                if member != host {
                    let mut other = url.clone();
                    let name = if member.contains(':') {
                        alloc::format!("[{member}]")
                    } else {
                        member.to_string()
                    };
                    if other.set_host(Some(&name)).is_ok() {
                        urls.push(other);
                    }
                }
            }
        }
        urls
    }

    fn load(rows: &mut Rows, loaded: Vec<Cookie>, hosts: Option<&[&str]>) {
        for row in loaded {
            let bare = row.domain.trim_start_matches('.').to_ascii_lowercase();
            if bare.is_empty() {
                continue;
            }
            // An IP address matches only itself, whatever the flag says.
            let subdomains = (row.subdomains || row.domain.starts_with('.')) && !is_ip(&bare);
            let spec = Cookie {
                domain: bare,
                subdomains,
                path: if row.path.is_empty() {
                    "/".to_string()
                } else {
                    row.path.clone()
                },
                ..row
            };
            if hosts.is_some_and(|hosts| !hosts.iter().any(|host| covers(&spec, host))) {
                continue;
            }
            rows.put(spec);
        }
    }

    /// The rows written under `key`: a by-host key keeps only the rows whose domain covers its
    /// host (or group), so a client that talked to two hosts never copies one's cookies into the
    /// other's file; a named key keeps everything.
    fn rows_for(&self, rows: &Rows, key: &str, host: &str) -> Vec<Cookie> {
        if key != host {
            return rows.list.clone();
        }
        let members = self.aliases.members(key);
        let hosts: Vec<&str> = if members.is_empty() {
            alloc::vec![key]
        } else {
            members
        };
        rows.list
            .iter()
            .filter(|row| hosts.iter().any(|h| covers(row, h)))
            .cloned()
            .collect()
    }

    /// Loads the jar's rows for the keys of `url` that were not yet loaded.
    pub(crate) fn before(&self, url: &Url) {
        let Some(jar) = &self.jar else { return };
        let Ok(mut rows) = self.rows.lock() else {
            return;
        };
        for key in self.keys(url) {
            if rows.loaded.contains(&key) {
                continue;
            }
            let host = self.host(url);
            let members = self.aliases.members(&key);
            let covered: Option<Vec<&str>> = match self.key {
                JarKey::ByHost => Some(if members.is_empty() {
                    alloc::vec![key.as_str()]
                } else {
                    members
                }),
                JarKey::Named(_) => None,
            };
            // An unreadable jar is an empty one.
            let found = jar.get(&key).unwrap_or_default();
            Self::load(&mut rows, found, covered.as_deref());
            let state: HashSet<Cookie> = self.rows_for(&rows, &key, &host).into_iter().collect();
            rows.synced.insert(key.clone(), state);
            rows.loaded.insert(key);
        }
    }

    /// The `Cookie` header for a request to `url`, or `None`. Longer paths come first (RFC 6265
    /// 5.4).
    pub(crate) fn header(&self, url: &Url, now: SystemTime) -> Option<Secret> {
        let urls = self.urls(url);
        let mut chosen: Vec<Cookie> = {
            let rows = self.rows.lock().ok()?;
            rows.list
                .iter()
                .filter(|row| urls.iter().any(|u| row.applies_to(u, now)))
                .cloned()
                .collect()
        };
        chosen.sort_by_key(|row| core::cmp::Reverse(row.path.len()));
        if chosen.is_empty() {
            return None;
        }
        let pairs: Vec<String> = chosen
            .iter()
            .map(|row| alloc::format!("{}={}", row.name, row.value.expose()))
            .collect();
        Some(Secret::new(pairs.join("; ")))
    }

    /// Keeps the cookies of the `Set-Cookie` `values` a reply from `url` sent, deletes the ones it
    /// expires, and writes the jar when rows changed.
    pub(crate) fn store(&self, url: &Url, values: &[String], now: SystemTime) {
        let context = SetCookieContext::new()
            .now(now)
            .request_path(url.path())
            .aliases(self.aliases.clone())
            .max_age(self.settings.max_age);
        let found = parse_set_cookie(values, &url_host(url), &context);
        let secure = url.scheme() == "https" || SECURE_HOSTS.contains(&url_host(url).as_str());
        let Ok(mut rows) = self.rows.lock() else {
            return;
        };
        for (spec, gone) in found {
            if !secure && (spec.secure || Self::shadows_secure(&rows, &spec)) {
                // A plain-http reply cannot set, replace or delete a Secure cookie.
                continue;
            }
            if gone {
                if let Some(index) = rows.position(&spec.domain, &spec.path, &spec.name) {
                    rows.list.remove(index);
                }
                continue;
            }
            if spec.name.len() + spec.value.expose().len() > self.settings.max_bytes {
                continue;
            }
            let domain = spec.domain.clone();
            rows.put(spec);
            let mut count = rows.list.iter().filter(|row| row.domain == domain).count();
            while count > self.settings.max_per_host {
                if let Some(oldest) = rows.list.iter().position(|row| row.domain == domain) {
                    rows.list.remove(oldest);
                }
                count -= 1;
            }
        }
        self.write(&mut rows, url);
    }

    /// Whether a stored Secure cookie has `spec`'s name, a domain that matches its domain or the
    /// other way round, and a path `spec`'s path matches (RFC 6265bis 5.7).
    fn shadows_secure(rows: &Rows, spec: &Cookie) -> bool {
        rows.list.iter().any(|row| {
            row.secure
                && row.name == spec.name
                && (row.domain == spec.domain
                    || row.domain.ends_with(&alloc::format!(".{}", spec.domain))
                    || spec.domain.ends_with(&alloc::format!(".{}", row.domain)))
                && path_matches(&row.path, &spec.path)
        })
    }

    fn write(&self, rows: &mut Rows, url: &Url) {
        let Some(jar) = &self.jar else { return };
        let host = self.host(url);
        for key in self.keys(url) {
            let list = self.rows_for(rows, &key, &host);
            let state: HashSet<Cookie> = list.iter().cloned().collect();
            if rows.synced.get(&key) == Some(&state) {
                continue;
            }
            if jar.set(&key, &list).is_ok() {
                rows.synced.insert(key, state);
            }
        }
    }

    /// Every cookie kept, as a snapshot.
    pub(crate) fn all(&self) -> Vec<Cookie> {
        self.rows
            .lock()
            .map(|rows| rows.list.clone())
            .unwrap_or_default()
    }
}

/// The tokens of one client: `Authorization` values chosen by the origin's host, or one key for
/// every host. A secret: never logged or put in a message.
pub(crate) struct Tokens {
    jar: Arc<dyn TokenJar>,
    key: JarKey,
}

impl Tokens {
    pub(crate) fn new(jar: Arc<dyn TokenJar>, key: JarKey) -> Self {
        Self { jar, key }
    }

    /// The `Authorization` value for `url`'s host, or `None`. A token that already names its
    /// scheme (`Basic ...`, `token ...`) is sent verbatim; a bare one is a bearer token.
    ///
    /// # Errors
    ///
    /// [`Error::Configuration`] naming the key, never the value, for a token with a CR, LF or
    /// NUL.
    pub(crate) fn authorization(&self, url: &Url) -> Result<Option<Secret>, Error> {
        let key = match &self.key {
            JarKey::Named(name) => name.clone(),
            JarKey::ByHost => url.host_str().unwrap_or("").to_ascii_lowercase(),
        };
        if key.is_empty() {
            return Ok(None);
        }
        let Some(token) = self.jar.get(&key) else {
            return Ok(None);
        };
        let token = token.expose().trim();
        if token.is_empty() {
            return Ok(None);
        }
        if token.contains(['\r', '\n', '\0']) {
            return Err(Error::config(
                alloc::format!("the token for {key:?}"),
                "contains a carriage return, line feed or NUL character",
            ));
        }
        let names_scheme = {
            let mut parts = token.splitn(2, char::is_whitespace);
            let scheme = parts.next().unwrap_or("");
            let rest = parts.next().unwrap_or("").trim_start();
            scheme
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic())
                && scheme
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                && !rest.is_empty()
        };
        Ok(Some(Secret::new(if names_scheme {
            token.to_string()
        } else {
            alloc::format!("Bearer {token}")
        })))
    }
}

/// The groups of hosts that share cookies: the setting, else `SYSHTTP_COOKIE_HOSTS`.
pub(crate) fn cookie_aliases(
    hosts: Option<&Vec<Vec<String>>>,
    env: &super::env::HttpEnv,
) -> Result<HostAliases, Error> {
    if let Some(groups) = hosts {
        return HostAliases::new(groups.iter().map(|group| group.iter().map(String::as_str)));
    }
    match env.find(&[super::env::COOKIE_HOSTS])? {
        None => Ok(HostAliases::default()),
        Some((variable, text)) => HostAliases::parse(&text)
            .map_err(|error| Error::config(variable, alloc::format!("{error}"))),
    }
}
