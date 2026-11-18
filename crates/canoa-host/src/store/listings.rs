//! What a catalog source's text means for the Store types whose catalogs the host does not index: Skills and MCP servers.
//!
//! A source the type reads itself (`read`) is an address or a place git fetches. Any other source answers with text, in the form
//! its type documents: for Skills a listing of skills, each naming the source that holds it; for MCP servers a page of the
//! official registry's list. Both are read as data: an entry that is not what it says is left out or marked, never trusted.

use serde_json::{Map, Value, json};

use coqui_aos::valid_relative_path;

use crate::control::valid_skill_name;

/// The most skills one listing carries.
pub(crate) const MAX_SKILLS: usize = 1024;
/// The most servers one page carries.
pub(crate) const MAX_SERVERS: usize = 2000;
/// The most characters of a description a listing carries.
const MAX_DESCRIPTION: usize = 1024;

/// A place that holds skills, as a person writes it for `npx skills add`: `owner/repo`, optionally with a folder, a `github:` or
/// `gitlab:` prefix, a GitHub or GitLab tree address, a git address, or a folder on this computer; `#ref` or `@v1.2.3` pins a
/// branch, tag or commit, and `@name` asks for the one skill of that name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SkillSpec {
    /// What was written, trimmed.
    pub(crate) spec: String,
    /// What git fetches: an address, or a folder on this computer.
    pub(crate) repo: String,
    /// The branch, tag or commit to read; none reads the repository's head.
    pub(crate) reference: Option<String>,
    /// The folder of the repository to look in; empty looks in all of it.
    pub(crate) path: String,
    /// The one skill asked for by name, or none for every skill the source holds.
    pub(crate) only: Option<String>,
}

fn word(text: &str) -> bool {
    text.chars()
        .next()
        .is_some_and(|first| first.is_ascii_alphanumeric())
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

fn is_version(text: &str) -> bool {
    let body = text.strip_prefix('v').unwrap_or(text);
    let (numbers, tail) = body
        .find(['-', '+'])
        .map_or((body, ""), |at| body.split_at(at));
    !numbers.is_empty()
        && numbers
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
        && tail
            .chars()
            .skip(1)
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '-'))
}

fn is_commit(text: &str) -> bool {
    matches!(text.len(), 40 | 64) && text.bytes().all(|b| b.is_ascii_hexdigit())
}

fn is_folder(text: &str) -> bool {
    let bytes = text.as_bytes();
    text.starts_with(['/', '\\'])
        || text.starts_with("./")
        || text.starts_with("../")
        || (bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'\\' | b'/'))
        || text.to_ascii_lowercase().starts_with("file://")
}

/// Whether `text` starts like `user@host:`, the address form scp takes.
fn scp_like(text: &str) -> bool {
    let name = |part: &str| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
    };
    text.split_once('@').is_some_and(|(user, rest)| {
        name(user) && rest.split_once(':').is_some_and(|(host, _)| name(host))
    })
}

/// Where an `@` that starts a pin or a skill name may be: after the last `/`, or the last `:` of an scp-like address.
fn suffix_at(text: &str) -> Option<usize> {
    let from = text
        .rfind('/')
        .into_iter()
        .chain(text.rfind(':'))
        .max()
        .map_or(0, |at| at + 1);
    text[from..].find('@').map(|at| at + from)
}

/// Whether a location carries a password: `scheme://user:password@host`.
pub(crate) fn holds_password(location: &str) -> bool {
    let Some((_, rest)) = location.split_once("://") else {
        return false;
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    rest[..end]
        .rsplit_once('@')
        .is_some_and(|(user, _)| user.contains(':'))
}

fn good_path(path: &str) -> bool {
    !path.contains('\\')
        && path.split('/').all(|part| {
            !part.is_empty() && part != "." && part != ".." && !part.chars().any(char::is_control)
        })
}

impl SkillSpec {
    /// Reads `text`. The error says what is wrong, never quoting more than was written.
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let spec = text.trim();
        if spec.is_empty() {
            return Err("Name a source: owner/repo, a git URL or a folder".to_owned());
        }
        if spec.chars().any(char::is_control) {
            return Err("A source is one line of text".to_owned());
        }
        if holds_password(spec) {
            return Err(
                "A source with a password in it is not kept: use a credential helper".to_owned(),
            );
        }
        let lower = spec.to_ascii_lowercase();
        if lower.contains("/.well-known/") {
            return Err("A site that lists its skills at /.well-known is not supported yet: name its git repository".to_owned());
        }
        let archive = [".zip", ".tar", ".tgz", ".tar.gz", ".tar.bz2", ".tar.xz"]
            .iter()
            .any(|ext| lower.ends_with(ext));
        if (lower.starts_with("http://") || lower.starts_with("https://"))
            && (lower.ends_with("/skill.md") || archive)
        {
            return Err("A link to one SKILL.md or an archive is not supported: name the git repository that holds it".to_owned());
        }
        if is_folder(spec) {
            return Ok(Self {
                spec: spec.to_owned(),
                repo: spec.to_owned(),
                reference: None,
                path: String::new(),
                only: None,
            });
        }
        let mut rest = spec;
        let mut host = "github.com";
        for (known, site) in [("github:", "github.com"), ("gitlab:", "gitlab.com")] {
            if rest.to_ascii_lowercase().starts_with(known) {
                host = site;
                rest = &rest[known.len()..];
            }
        }
        let prefixed = host != "github.com" || spec.len() != rest.len();
        let is_url =
            !prefixed && (rest.contains("://") || rest.starts_with("git+") || scp_like(rest));
        let mut reference: Option<String> = None;
        let mut only: Option<String> = None;
        let mut rest = rest.to_owned();
        if let Some(hash) = rest.rfind('#') {
            let mut after = rest[hash + 1..].to_owned();
            rest.truncate(hash);
            if after.is_empty() {
                return Err("Nothing follows the #: name a branch, a tag or a commit".to_owned());
            }
            if let Some(at) = suffix_at(&after) {
                only = Some(after[at + 1..].to_owned());
                after.truncate(at);
            }
            reference = Some(after);
        } else if let Some(at) = suffix_at(&rest) {
            let tail = rest[at + 1..].to_owned();
            rest.truncate(at);
            if is_version(&tail) || is_commit(&tail) {
                reference = Some(tail);
            } else {
                only = Some(tail);
            }
        }
        if only.as_deref().is_some_and(|name| !word(name)) {
            return Err(
                "A skill named after @ is a skill name: letters, digits, - _ and .".to_owned(),
            );
        }
        if reference.as_deref().is_some_and(|name| {
            name.is_empty() || name.starts_with('-') || name.contains("..") || name.contains(' ')
        }) {
            return Err("That is not a branch, tag or commit".to_owned());
        }
        let mut path = String::new();
        let repo = if is_url {
            let mut repo = rest.strip_prefix("git+").unwrap_or(&rest).to_owned();
            if let Some((bare, tree_ref, tree_path)) = tree_address(&repo) {
                repo = bare;
                reference.get_or_insert(tree_ref);
                path = tree_path;
            }
            let schemes = ["https://", "http://", "ssh://", "git://", "file://"];
            if !(schemes.iter().any(|scheme| repo.starts_with(scheme)) || scp_like(&repo)) {
                return Err("A source git may fetch starts with https://, ssh://, git://, file:// or user@host:".to_owned());
            }
            repo
        } else {
            let parts: Vec<&str> = rest.split('/').collect();
            let bare = |part: &str| part.strip_suffix(".git").unwrap_or(part).to_owned();
            if parts.len() < 2 || !word(parts[0]) || !word(&bare(parts[1])) {
                return Err(
                    "Write a source as owner/repo, a git URL, or a folder on this computer"
                        .to_owned(),
                );
            }
            path = parts[2..].join("/");
            format!("https://{host}/{}/{}.git", parts[0], bare(parts[1]))
        };
        let path = path.trim_end_matches('/').to_owned();
        if !path.is_empty() && !good_path(&path) {
            return Err("A folder in a source is a relative path with no .. in it".to_owned());
        }
        if repo.len() > 2048 {
            return Err("That source is too long".to_owned());
        }
        Ok(Self {
            spec: spec.to_owned(),
            repo,
            reference,
            path,
            only,
        })
    }
}

/// `https://github.com/owner/repo/tree/ref/folder`, or the same on GitLab (with `/-/tree/`): the repository, the ref and the
/// folder, when `repo` is one.
fn tree_address(repo: &str) -> Option<(String, String, String)> {
    let rest = ["https://github.com/", "https://gitlab.com/"]
        .iter()
        .find_map(|site| repo.strip_prefix(site).map(|rest| (*site, rest)))?;
    let (site, rest) = rest;
    let parts: Vec<&str> = rest.split('/').collect();
    let owner = *parts.first()?;
    let name = *parts.get(1)?;
    if owner.is_empty() || name.is_empty() {
        return None;
    }
    let mut at = 2;
    if parts.get(at) == Some(&"-") {
        at += 1;
    }
    if parts.get(at) != Some(&"tree") {
        return None;
    }
    let reference = *parts.get(at + 1)?;
    if reference.is_empty() {
        return None;
    }
    let path = parts[at + 2..].join("/");
    if parts.len() > at + 2 && path.is_empty() {
        return None;
    }
    Some((
        format!(
            "{site}{owner}/{}",
            name.strip_suffix(".git").unwrap_or(name)
        ),
        reference.to_owned(),
        path,
    ))
}

/// One skill a listing names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ListedSkill {
    pub(crate) name: String,
    pub(crate) description: String,
    /// The source that holds it, as `read` takes one.
    pub(crate) source: String,
    pub(crate) reference: Option<String>,
    /// The skill's folder from the repository's root.
    pub(crate) path: String,
    /// Why it cannot be installed, if it cannot.
    pub(crate) problem: Option<String>,
}

impl ListedSkill {
    /// The entry as `store/catalog` answers it.
    pub(crate) fn to_value(&self) -> Value {
        let mut each = json!({
            "name": self.name,
            "description": self.description,
            "source": self.source,
            "path": self.path,
        });
        if let Some(reference) = &self.reference {
            each["ref"] = json!(reference);
        }
        if let Some(problem) = &self.problem {
            each["problem"] = json!(problem);
        }
        each
    }
}

/// A skills listing read.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct SkillListing {
    pub(crate) skills: Vec<ListedSkill>,
    /// Entries left out: no name, no source, or a name already listed.
    pub(crate) skipped: usize,
    /// Whether more were named than are listed.
    pub(crate) truncated: bool,
}

/// Reads a skills listing: a JSON object with a `skills` list, each entry `{"name", "description"?, "source", "path"?, "ref"?}`.
/// `source` is where the skill is, written as a skills source is (`owner/repo`, a git address, a folder); `path` is the skill's
/// folder from that repository's root, when the source does not already name it; `ref` pins a branch, tag or commit. An entry
/// with no name or source is left out and counted, as is a second of a name; one whose name or source is not valid is listed
/// with the reason it cannot be installed.
pub(crate) fn skills_listing(text: &str) -> Result<SkillListing, String> {
    let value: Value =
        serde_json::from_str(text).map_err(|error| format!("The listing is not JSON: {error}"))?;
    let entries = value
        .get("skills")
        .and_then(Value::as_array)
        .ok_or("The listing is not {\"skills\": [...]}")?;
    let mut listing = SkillListing::default();
    let mut seen = alloc::collections::BTreeSet::new();
    for entry in entries {
        let Some(table) = entry.as_object() else {
            listing.skipped += 1;
            continue;
        };
        let text_of = |key: &str| table.get(key).and_then(Value::as_str);
        let (Some(name), Some(source)) = (
            text_of("name").filter(|name| !name.is_empty()),
            text_of("source").filter(|source| !source.trim().is_empty()),
        ) else {
            listing.skipped += 1;
            continue;
        };
        if !seen.insert(name.to_ascii_lowercase()) {
            listing.skipped += 1;
            continue;
        }
        if listing.skills.len() >= MAX_SKILLS {
            listing.truncated = true;
            break;
        }
        listing.skills.push(listed(name, source, table));
    }
    Ok(listing)
}

fn listed(name: &str, source: &str, table: &Map<String, Value>) -> ListedSkill {
    let description: String = table
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .chars()
        .take(MAX_DESCRIPTION)
        .collect();
    let mut out = ListedSkill {
        name: name.to_owned(),
        description,
        source: source.trim().to_owned(),
        reference: None,
        path: String::new(),
        problem: None,
    };
    let problem = (|| {
        if !valid_skill_name(name) || !valid_relative_path(name) {
            return Some(format!("{name:?} is not a skill name"));
        }
        let spec = match SkillSpec::parse(source) {
            Ok(spec) => spec,
            Err(why) => return Some(why),
        };
        out.reference.clone_from(&spec.reference);
        out.path.clone_from(&spec.path);
        if let Some(path) = table.get("path") {
            match path.as_str() {
                Some(path) if path.is_empty() || (good_path(path) && valid_relative_path(path)) => {
                    path.clone_into(&mut out.path);
                }
                _ => {
                    return Some("path is a folder of the repository, with no .. in it".to_owned());
                }
            }
        }
        if let Some(reference) = table.get("ref") {
            match reference.as_str() {
                Some(text)
                    if !text.is_empty()
                        && !text.starts_with('-')
                        && !text.contains("..")
                        && !text.contains(' ')
                        && !text.chars().any(char::is_control) =>
                {
                    out.reference = Some(text.to_owned());
                }
                _ => return Some("ref is a branch, a tag or a commit".to_owned()),
            }
        }
        None
    })();
    out.problem = problem;
    out
}

/// One page of MCP servers.
#[derive(Debug, PartialEq)]
pub(crate) struct McpPage {
    /// Each item as the registry lists it: `{"server": {...}, "_meta"?: {...}}`.
    pub(crate) servers: Vec<Value>,
    /// What to ask for the next page, when the source has one.
    pub(crate) next: Option<String>,
}

impl McpPage {
    /// The page in the shape of the registry's own list answer, which a client reads as it reads a registry's.
    pub(crate) fn to_value(&self) -> Value {
        json!({
            "servers": self.servers,
            "metadata": {"nextCursor": self.next},
        })
    }
}

const OFFICIAL_META: &str = "io.modelcontextprotocol.registry/official";

/// Reads a page of the registry's list shape: `{"servers": [{"server": {...}, "_meta": {...}}], "metadata": {"nextCursor"}}`.
/// An item whose `server` names nothing, or which the registry marks as not its latest version, is left out; so is one that does
/// not hold every word of `search` in its name, title or description (a source that ignores a search is narrowed here).
pub(crate) fn mcp_page(text: &str, search: &str) -> Result<McpPage, String> {
    let value: Value =
        serde_json::from_str(text).map_err(|_| "The answer is not JSON".to_owned())?;
    let items = value
        .get("servers")
        .and_then(Value::as_array)
        .ok_or("The answer is not a list of servers")?;
    let words: Vec<String> = search.split_whitespace().map(str::to_lowercase).collect();
    let mut servers = Vec::new();
    for item in items {
        let Some(server) = item.get("server").and_then(Value::as_object) else {
            continue;
        };
        let field = |key: &str| server.get(key).and_then(Value::as_str).unwrap_or_default();
        if field("name").is_empty() {
            continue;
        }
        let latest = item
            .get("_meta")
            .and_then(|meta| meta.get(OFFICIAL_META))
            .and_then(|official| official.get("isLatest"))
            != Some(&Value::Bool(false));
        if !latest {
            continue;
        }
        let haystack = format!(
            "{}\n{}\n{}",
            field("name"),
            field("title"),
            field("description")
        )
        .to_lowercase();
        if words.iter().all(|word| haystack.contains(word.as_str())) {
            servers.push(item.clone());
        }
        if servers.len() >= MAX_SERVERS {
            break;
        }
    }
    let next = value
        .get("metadata")
        .and_then(|meta| meta.get("nextCursor"))
        .and_then(Value::as_str)
        .filter(|cursor| !cursor.is_empty())
        .map(str::to_owned);
    Ok(McpPage { servers, next })
}

#[cfg(test)]
#[path = "listings_tests.rs"]
mod tests;
