//! Where the portal is mounted: the path a proxy serves it under, such as `/canoa/`.
//!
//! The routes and pages are written for `/`. A mount under a prefix is applied at the two edges alone: [`Mount::enter`] takes
//! the prefix off a request, and [`Mount::leave`] puts it on every path an answer carries. With `/` neither does anything, so
//! the answers are the ones a portal at the root gives.

use super::{Payload, Request, Response};

/// The attributes of the portal's pages that hold a path of its own, up to the slash that starts the path.
const PATH_ATTRIBUTES: [&str; 5] = [
    "href=\"",
    "action=\"",
    "src=\"",
    "data-options-url=\"",
    "<option value=\"",
];

/// What a request's path is, against the mount.
pub(super) enum Entry {
    /// Under the mount: the request now carries the path below it.
    Inside,
    /// The mount's own address with no closing slash: the browser goes to the address with it.
    Bare,
    /// Not under the mount.
    Outside,
}

pub(super) struct Mount {
    /// A leading and a trailing slash: `/`, or `/canoa/`.
    base: String,
    /// The mount's words joined with `_`, which names the cookies of a mount below the root; none at `/`.
    slug: Option<String>,
}

impl Mount {
    /// A mount at `base`, which [`normalise`] has made.
    pub(super) fn new(base: &str) -> Self {
        let slug = (base != "/").then(|| base.trim_matches('/').replace('/', "_"));
        Self {
            base: base.to_owned(),
            slug,
        }
    }

    /// The mount, with its slashes.
    pub(super) fn base(&self) -> &str {
        &self.base
    }

    /// `own`, a path that starts with `/`, below the mount.
    pub(super) fn path(&self, own: &str) -> String {
        format!("{}{}", self.base, own.trim_start_matches('/'))
    }

    /// Takes the mount off `request`'s path, so that the routes see the path they were written with.
    pub(super) fn enter(&self, request: &mut Request) -> Entry {
        let Some(slug) = &self.slug else {
            return Entry::Inside;
        };
        let target = request.target.as_str();
        let (path, rest) = target
            .find(['?', '#'])
            .map_or((target, ""), |at| target.split_at(at));
        if path == self.base.trim_end_matches('/') {
            return Entry::Bare;
        }
        let Some(below) = path.strip_prefix(&self.base) else {
            return Entry::Outside;
        };
        request.target = format!("/{below}{rest}");
        request.cookie_slug = Some(slug.clone());
        Entry::Inside
    }

    /// Puts the mount on what `response` says of the portal's own paths: its redirect, its cookies' names and paths, and the
    /// links and forms of a page.
    pub(super) fn leave(&self, response: &mut Response) {
        let Some(slug) = &self.slug else { return };
        for (name, value) in &mut response.headers {
            match *name {
                "Location" if value.starts_with('/') && !value.starts_with("//") => {
                    *value = self.path(value);
                }
                "Set-Cookie" => *value = self.cookie(value, slug),
                _ => {}
            }
        }
        let html = response
            .headers
            .iter()
            .any(|(name, value)| *name == "Content-Type" && value.starts_with("text/html"));
        if let (true, Payload::Bytes(bytes)) = (html, &mut response.payload)
            && let Ok(text) = core::str::from_utf8(bytes)
        {
            let mut text = text.to_owned();
            for head in PATH_ATTRIBUTES {
                text = text.replace(&format!("{head}/"), &format!("{head}{}", self.base));
            }
            *bytes = text.into_bytes();
        }
    }

    /// A `Set-Cookie` of a cookie named for the root, named and scoped for the mount.
    fn cookie(&self, value: &str, slug: &str) -> String {
        let Some((name, attributes)) = value.split_once('=') else {
            return value.to_owned();
        };
        if !name.starts_with("__Host-") {
            return value.to_owned();
        }
        format!("{}={attributes}", scoped_name(name, slug))
            .replace("; Path=/;", &format!("; Path={};", self.base))
    }
}

/// The name a cookie called `__Host-NAME` has under a mount: `__Secure-NAME.SLUG`. A `__Host-` cookie must have `Path=/`, which
/// would send it to every portal on the host, so a mount uses `__Secure-` with the mount as its path and the mount in its name.
pub(super) fn scoped_name(host_name: &str, slug: &str) -> String {
    let name = host_name.strip_prefix("__Host-").unwrap_or(host_name);
    format!("__Secure-{name}.{slug}")
}

/// The mount a setting names: a leading and a trailing slash around words of letters, digits and `-`.
///
/// # Errors
///
/// A message when a word is empty, is `.` or `..`, or has any other character.
pub(super) fn normalise(text: &str) -> Result<String, String> {
    let words: Vec<&str> = text.split('/').filter(|word| !word.is_empty()).collect();
    if text.contains("//")
        || words.iter().any(|word| {
            matches!(*word, "." | "..")
                || !word
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
    {
        return Err("base_path: words of letters, digits and - between slashes".to_owned());
    }
    Ok(if words.is_empty() {
        "/".to_owned()
    } else {
        format!("/{}/", words.join("/"))
    })
}
