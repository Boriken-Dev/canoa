//! Small helpers the resolver, the bypass rules and the PAC builtins share.

use alloc::string::String;
use alloc::vec::Vec;

/// Whether `pattern`, with `*` (any run) and `?` (any one character) as the only wildcards,
/// matches the whole of `text`.
pub(crate) fn wildcard_match(pattern: &str, text: &str, ignore_case: bool) -> bool {
    let fold = |c: char| {
        if ignore_case {
            c.to_ascii_lowercase()
        } else {
            c
        }
    };
    let pattern: Vec<char> = pattern.chars().map(fold).collect();
    let text: Vec<char> = text.chars().map(fold).collect();
    let (mut p, mut t) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while t < text.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == text[t]) && pattern[p] != '*' {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some(p);
            mark = t;
            p += 1;
        } else if let Some(star_at) = star {
            p = star_at + 1;
            mark += 1;
            t = mark;
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|&c| c == '*')
}

/// `host[:port]` or `[v6][:port]`. A bare IPv6 address (several colons, no brackets) is a host.
pub(crate) fn split_host_port(entry: &str) -> Option<(String, Option<u16>)> {
    if let Some(rest) = entry.strip_prefix('[') {
        let (host, after) = rest.split_once(']')?;
        return match after {
            "" => Some((host.into(), None)),
            _ => {
                let port = after.strip_prefix(':')?.parse().ok()?;
                Some((host.into(), Some(port)))
            }
        };
    }
    if entry.matches(':').count() > 1 {
        return Some((entry.into(), None));
    }
    match entry.split_once(':') {
        Some((host, port)) => Some((host.into(), Some(port.parse().ok()?))),
        None => Some((entry.into(), None)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wildcard_matches_the_whole_text_only() {
        assert!(wildcard_match("*.example.com", "a.example.com", false));
        assert!(!wildcard_match("*.example.com", "example.com", false));
        assert!(wildcard_match("a?c", "abc", false));
        assert!(!wildcard_match("a?c", "ac", false));
        assert!(wildcard_match("*", "", false));
        assert!(wildcard_match("a*b*c", "aXXbYYc", false));
        assert!(!wildcard_match("a*b*c", "aXXbYY", false));
        assert!(!wildcard_match("ABC", "abc", false));
        assert!(wildcard_match("ABC", "abc", true));
    }

    #[test]
    fn a_host_and_port_split_around_brackets_and_bare_ipv6() {
        assert_eq!(split_host_port("h:80"), Some(("h".into(), Some(80))));
        assert_eq!(split_host_port("[::1]:80"), Some(("::1".into(), Some(80))));
        assert_eq!(split_host_port("::1"), Some(("::1".into(), None)));
        assert_eq!(split_host_port("h:x"), None);
    }
}
