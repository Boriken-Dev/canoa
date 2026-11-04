//! Overlays in the Agent Overlay Specification (AOS) v0 format, edition `0.1.0-draft`
//! (<https://github.com/Boriken-Dev/agent-overlay-spec>): reading `overlay.cfg` and env files, validating a manifest,
//! overlay names, source strings, tree digests, the defaults conventional files contribute, composition order, and the
//! environment an overlay builds with its substitution.
//!
//! Canoa claims the manifest reader (TOML and JSON; YAML is refused as an unsupported format), validator, integrity,
//! resolver and environment classes. It runs no env script, no setup and no package manager: those declarations are
//! read and validated, and [`crate::profile`] reports them as not applied. `[about]`, store metadata, is validated and
//! never read otherwise: a problem in it never stops an overlay ([`applies`]). Every function here is static: it reads no
//! file, runs nothing and consults no environment, except [`read_tree`], which lists and reads a directory, and the
//! callbacks a caller passes.

mod apply;
mod digest;
mod read;
mod subst;
mod validate;

use alloc::string::String;

pub(crate) use apply::default_paths;
pub use apply::{
    Base, EnvFile, Environment, Facts, Missing, Overlay, PathAddition, Resolved, Weakening,
    apply_overlay, composition_order, context_of, env_files_of, grants_report, os_matches,
    paths_of, resolve, rules_of, skills_of,
};
pub use digest::{
    EntryKind, TreeDigest, TreeEntry, digest_entries, read_tree, same_digest, sha256_digest,
};
pub use read::{DotenvLine, Limits, parse_dotenv, read_document, read_dotenv};
pub use subst::substitute;
pub use validate::{Issue, applies, validate};

/// A refusal, by the class the specification's conformance vectors name it (`parse-error`, `undefined-variable`,
/// `symlink`, ...), and what it is about: a variable, a path, a format or an overlay, never a value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AosError {
    /// The class.
    pub code: &'static str,
    /// The variable, path, format name or overlay it is about; empty when there is none to name.
    pub subject: String,
}

impl AosError {
    pub(crate) fn new(code: &'static str, subject: impl Into<String>) -> Self {
        Self {
            code,
            subject: subject.into(),
        }
    }
}

impl core::fmt::Display for AosError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.subject.is_empty() {
            f.write_str(self.code)
        } else {
            write!(f, "{}: {}", self.code, self.subject)
        }
    }
}

impl core::error::Error for AosError {}

/// The names a consumer's scopes use, which no overlay may take.
const RESERVED_NAMES: [&str; 6] = [
    "default",
    "overlay",
    "system",
    "user",
    "project",
    "project-root",
];

/// Path components that are never part of an overlay, compared exactly by the digest and without ASCII case by
/// [`is_relative_path`].
pub const EXCLUDED_NAMES: [&str; 10] = [
    ".git",
    ".hg",
    ".svn",
    "__pycache__",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
    ".tox",
    ".venv",
    "node_modules",
];

fn is_device_stem(stem: &str, aliases: bool) -> bool {
    let lower = stem.to_ascii_lowercase();
    if ["con", "prn", "aux", "nul"].contains(&lower.as_str()) {
        return true;
    }
    if aliases && ["conin$", "conout$"].contains(&lower.as_str()) {
        return true;
    }
    let mut chars = lower.chars();
    let head: String = chars.by_ref().take(3).collect();
    let (Some(digit), None) = (chars.next(), chars.next()) else {
        return false;
    };
    (head == "com" || head == "lpt")
        && (digit.is_ascii_digit() && (aliases || digit != '0')
            || aliases && matches!(digit, '¹' | '²' | '³'))
}

/// Whether `name` is a valid overlay identity: an ASCII letter, then letters, digits, `_`, `.` or `-`, ending in a
/// letter or digit; not a scope label (`user`, `project`, ...) and not a Windows device name before its first `.`.
#[must_use]
pub fn valid_overlay_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    let (Some(first), Some(last)) = (bytes.first(), bytes.last()) else {
        return false;
    };
    first.is_ascii_alphabetic()
        && last.is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
        && !RESERVED_NAMES.contains(&name.to_ascii_lowercase().as_str())
        && !is_device_stem(name.split('.').next().unwrap_or(name), false)
}

/// An identity as it is compared: ASCII lowercase, with `_` and `.` made `-`.
#[must_use]
pub fn normalize_name(name: &str) -> String {
    name.to_ascii_lowercase().replace(['_', '.'], "-")
}

fn variable_stem(identity: &str) -> String {
    normalize_name(identity)
        .to_ascii_uppercase()
        .chars()
        .map(|c| {
            if c.is_ascii_uppercase() || c.is_ascii_digit() {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// `<NAME>_OVERLAY_ROOT` for an overlay identity.
#[must_use]
pub fn root_variable(identity: &str) -> String {
    variable_stem(identity) + "_OVERLAY_ROOT"
}

/// `<NAME>_OVERLAY_STATE` for an overlay identity.
#[must_use]
pub fn state_variable(identity: &str) -> String {
    variable_stem(identity) + "_OVERLAY_STATE"
}

/// Whether a variable belongs to the consumer: `AOS_*`, `*_OVERLAY_ROOT` or `*_OVERLAY_STATE`, in any ASCII case.
#[must_use]
pub fn is_reserved_variable(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    upper.starts_with("AOS_")
        || upper.ends_with("_OVERLAY_ROOT")
        || upper.ends_with("_OVERLAY_STATE")
}

/// `[A-Za-z_][A-Za-z0-9_]*`.
#[must_use]
pub fn is_env_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

/// The specification's sensitive variables: those that change how programs find and load code, which files run at
/// startup, which certificates are trusted, where packages come from, or where traffic goes. Compared without ASCII case.
#[must_use]
pub fn is_sensitive(name: &str) -> bool {
    const NAMES: [&str; 43] = [
        "PATH",
        "PATHEXT",
        "HOME",
        "USERPROFILE",
        "SHELL",
        "COMSPEC",
        "SYSTEMROOT",
        "BASH_ENV",
        "ENV",
        "ZDOTDIR",
        "PROMPT_COMMAND",
        "IFS",
        "EDITOR",
        "VISUAL",
        "PAGER",
        "LESSOPEN",
        "LESSCLOSE",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "APPDATA",
        "LOCALAPPDATA",
        "TMPDIR",
        "TEMP",
        "TMP",
        "PSMODULEPATH",
        "CLASSPATH",
        "JAVA_TOOL_OPTIONS",
        "_JAVA_OPTIONS",
        "JDK_JAVA_OPTIONS",
        "PERL5OPT",
        "PERL5LIB",
        "RUBYOPT",
        "RUBYLIB",
        "RUSTFLAGS",
        "GOPROXY",
        "GONOSUMDB",
        "GONOSUMCHECK",
        "GOPRIVATE",
        "GOFLAGS",
        "SSH_ASKPASS",
        "SSLKEYLOGFILE",
        "REQUESTS_CA_BUNDLE",
        "CURL_CA_BUNDLE",
    ];
    const PREFIXES: [&str; 14] = [
        "LD_",
        "DYLD_",
        "GIT_",
        "PYTHON",
        "NODE_",
        "NPM_CONFIG_",
        "PIP_",
        "UV_",
        "CARGO_",
        "RUSTC_",
        "DOTNET_",
        "COR_",
        "CORECLR_",
        "SSL_",
    ];
    let upper = name.to_ascii_uppercase();
    NAMES.contains(&upper.as_str())
        || PREFIXES.iter().any(|prefix| upper.starts_with(prefix))
        || upper.ends_with("_PROXY")
        || upper.ends_with("_BASE_URL")
}

fn forbidden_in_path(c: char) -> bool {
    matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        || matches!(c, '\u{0}'..='\u{1f}' | '\u{7f}'..='\u{9f}' | '\u{200c}'..='\u{200f}')
        || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{206f}' | '\u{feff}')
}

/// A relative path as the specification defines it: `/`-separated components, none empty or ending in `.` or a space,
/// none holding a character Windows refuses or Unicode ignores, none an excluded or device name in any ASCII case or
/// ending in `.pyc`, and no `~` followed by a digit anywhere.
#[must_use]
pub fn is_relative_path(path: &str) -> bool {
    let chars: alloc::vec::Vec<char> = path.chars().collect();
    if chars
        .windows(2)
        .any(|pair| pair[0] == '~' && pair[1].is_ascii_digit())
    {
        return false;
    }
    !path.is_empty()
        && path.split('/').all(|component| {
            let lower = component.to_ascii_lowercase();
            !component.is_empty()
                && !component.ends_with(['.', ' '])
                && !component.chars().any(forbidden_in_path)
                && !EXCLUDED_NAMES.contains(&lower.as_str())
                && !is_device_stem(lower.split('.').next().unwrap_or(&lower), true)
                && !lower.ends_with(".pyc")
        })
}

/// Where an overlay comes from, as a source string names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SourceKind {
    /// A Git repository.
    Git,
    /// Another URL.
    Url,
    /// A directory on this computer.
    Directory,
}

/// A parsed source string, `[git+]<location>[@<ref>][#<path>]`.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Source {
    /// Git, a URL or a directory.
    pub kind: SourceKind,
    /// The repository, URL or directory, without `git+`, the ref or the path.
    pub location: String,
    /// A Git branch, tag or commit; none for the default branch, and always none for the other kinds.
    pub reference: Option<String>,
    /// The overlay's root inside the location, a relative path; none for the location's root.
    pub path: Option<String>,
}

fn git_location(location: &str) -> bool {
    location.ends_with(".git")
        || ["git@", "ssh://", "git://"]
            .iter()
            .any(|prefix| location.starts_with(prefix))
}

/// Parses a source string as the specification's distribution section says.
///
/// # Errors
///
/// `invalid-source` for an empty source, a path that is not a relative path, and a Git source whose location or ref
/// opens with `-`, whose location holds `::`, names a scheme other than `https`, `http`, `ssh`, `git` and `file`, or a
/// host that opens with `-`.
pub fn parse_source(text: &str) -> Result<Source, AosError> {
    let refused = |why: &str| AosError::new("invalid-source", why);
    let mut raw = text.trim();
    if raw.is_empty() {
        return Err(refused("empty"));
    }
    let explicit = raw.starts_with("git+");
    if explicit {
        raw = &raw["git+".len()..];
    }
    let (body, path) = match raw.split_once('#') {
        Some((body, path)) => {
            let path = path.trim().trim_matches('/');
            (body, (!path.is_empty()).then(|| path.to_owned()))
        }
        None => (raw, None),
    };
    let mut location = body.trim();
    let mut reference = None;
    if let Some((head, tail)) = location.rsplit_once('@')
        && !head.is_empty()
        && !tail.is_empty()
        && !tail.contains(['/', ':', '\\'])
        && !head.ends_with("://")
        && !head.ends_with(':')
        && (explicit || git_location(head))
    {
        location = head;
        reference = Some(tail.to_owned());
    }
    if path.as_deref().is_some_and(|path| !is_relative_path(path)) {
        return Err(refused("path"));
    }
    if explicit || git_location(location) {
        if location.starts_with('-') || reference.as_deref().is_some_and(|r| r.starts_with('-')) {
            return Err(refused("option-like"));
        }
        if location.contains("::") {
            return Err(refused("git remote helper"));
        }
        let host = if let Some((scheme, rest)) = location.split_once("://") {
            if !["https", "http", "ssh", "git", "file"]
                .contains(&scheme.to_ascii_lowercase().as_str())
            {
                return Err(refused("scheme"));
            }
            let authority = rest.split('/').next().unwrap_or("");
            authority.rsplit('@').next().unwrap_or("")
        } else if location.contains('@') {
            let authority = location.split(':').next().unwrap_or("");
            authority.rsplit('@').next().unwrap_or("")
        } else {
            ""
        };
        if host.starts_with('-') {
            return Err(refused("option-like host"));
        }
        return Ok(Source {
            kind: SourceKind::Git,
            location: location.to_owned(),
            reference,
            path,
        });
    }
    Ok(Source {
        kind: if location.contains("://") {
            SourceKind::Url
        } else {
            SourceKind::Directory
        },
        location: location.to_owned(),
        reference: None,
        path,
    })
}
