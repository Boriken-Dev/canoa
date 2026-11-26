//! The Netscape cookie file, as curl reads and writes it.

use alloc::string::String;
use alloc::vec::Vec;
use std::io;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use coqui_proxy::Secret;

use super::parse::from_unix;
use super::{Cookie, CookieLike, HTTP_ONLY_PREFIX};
use crate::session::fileio::{read_text, write_private};

/// A name or value that holds a control character but the tab is neither read nor written, so a
/// value cannot end a line of a jar file.
fn has_control(text: &str) -> bool {
    text.chars()
        .any(|character| matches!(character, '\0'..='\u{8}' | '\n'..='\u{1f}' | '\u{7f}'))
}

/// A field of a row cannot carry any control character, the tab (the separator) included.
fn unsafe_field(text: &str) -> bool {
    text.chars()
        .any(|character| matches!(character, '\0'..='\u{1f}' | '\u{7f}'))
}

fn ascii_seconds(text: &str) -> Option<u64> {
    let text = text.trim();
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// The rows of the Netscape cookie file `path` (none when it is missing or unreadable).
///
/// Lines end at `\n` only, one trailing `\r` taken off, and a row whose name or value holds a
/// control character is skipped: other characters a text splitter might honour (`\x0b`, `\x0c`,
/// `\x85`) are part of a value, not the end of a row.
#[must_use]
pub fn load_netscape(path: &Path) -> Vec<Cookie> {
    let Ok(text) = read_text(path) else {
        return Vec::new();
    };
    let mut cookies = Vec::new();
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        // Only the leading blanks go: a trailing tab separates an empty value.
        let line = line.trim_start_matches(' ');
        // curl writes an HttpOnly cookie's domain as `#HttpOnly_<domain>`; any other `#` line is a
        // comment.
        if line.trim().is_empty() || (line.starts_with('#') && !line.starts_with(HTTP_ONLY_PREFIX))
        {
            continue;
        }
        let mut parts: Vec<&str> = line.split('\t').collect();
        if parts.len() == 6 {
            // An editor trimmed the empty value's tab.
            parts.push("");
        }
        if parts.len() < 7 {
            continue;
        }
        let (domain, flag, cookie_path, secure, expires, name, value) = (
            parts[0], parts[1], parts[2], parts[3], parts[4], parts[5], parts[6],
        );
        if has_control(name) || has_control(value) {
            continue;
        }
        let (http_only, domain) = match domain.strip_prefix(HTTP_ONLY_PREFIX) {
            Some(rest) => (true, rest),
            None => (false, domain),
        };
        cookies.push(Cookie {
            domain: String::from(domain),
            path: String::from(if cookie_path.is_empty() {
                "/"
            } else {
                cookie_path
            }),
            secure: secure.trim().eq_ignore_ascii_case("TRUE"),
            // `0` is how the format writes a session cookie: no expiry.
            expires: ascii_seconds(expires)
                .filter(|seconds| *seconds != 0)
                .and_then(|seconds| i64::try_from(seconds).ok())
                .map(from_unix),
            name: String::from(name),
            value: Secret::new(value),
            http_only,
            subdomains: flag.trim().eq_ignore_ascii_case("TRUE"),
        });
    }
    cookies
}

fn seconds(time: SystemTime) -> i64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_secs()).unwrap_or(i64::MAX),
        Err(before) => -i64::try_from(before.duration().as_secs()).unwrap_or(i64::MAX),
    }
}

/// Writes `cookies` to `path` as a Netscape cookie file, owner only (`0600`) and replaced whole,
/// with LF line ends. A cookie with no name, or with a control character or a tab in a field, is
/// not written: it would split its row.
///
/// # Errors
///
/// What creating the directory, writing or replacing the file reports.
pub fn save_netscape<C: CookieLike>(
    cookies: impl IntoIterator<Item = C>,
    path: &Path,
) -> io::Result<()> {
    let mut text = String::from("# Netscape HTTP Cookie File\n");
    for cookie in cookies {
        let mut domain = String::from(cookie.domain());
        let flag = if domain.starts_with('.') || cookie.subdomains() {
            "TRUE"
        } else {
            "FALSE"
        };
        if cookie.http_only() {
            domain.insert_str(0, HTTP_ONLY_PREFIX);
        }
        let cookie_path = if cookie.path().is_empty() {
            "/"
        } else {
            cookie.path()
        };
        let secure = if cookie.secure() { "TRUE" } else { "FALSE" };
        let expires = cookie.expires().map_or(0, seconds);
        if cookie.name().is_empty()
            || [domain.as_str(), cookie_path, cookie.name(), cookie.value()]
                .into_iter()
                .any(unsafe_field)
        {
            continue;
        }
        text.push_str(&alloc::format!(
            "{domain}\t{flag}\t{cookie_path}\t{secure}\t{expires}\t{}\t{}\n",
            cookie.name(),
            cookie.value()
        ));
    }
    write_private(path, text.as_bytes())
}
