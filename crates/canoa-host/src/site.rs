//! A webpage's session works in a scratch folder the host makes for it. The page has no file system and names no path: its
//! `session/new` carries the site in `_meta`, and the host resolves `<Documents>/Canoa/scratch/<site>/<session>` itself,
//! makes it and starts the agent there. Nothing a page or an agent sends becomes a path except through [`site_name`]: a host
//! name that is already one safe folder name, or nothing. The host makes no other folder.

use alloc::borrow::Cow;
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use serde_json::{Map, Value, json};

use crate::scope::{Scope, Verdict};
use crate::share;

/// The `_meta` key of a webpage's session request, whose value is `{"site": "<host name>"}`; the host's answer to
/// `session/new` carries the same key with `{"site", "id", "folder"}`.
pub const SITE_META: &str = "dev.boriken.canoa/site";

/// JSON-RPC's code for parameters the receiver will not take.
const INVALID_PARAMS: i64 = -32602;
/// How many site sessions one connection may have waiting for the agent's answer.
const MAX_PENDING: usize = 64;
/// The longest a whole host name may be, and its longest label (RFC 1035).
const MAX_NAME: usize = 253;
const MAX_LABEL: usize = 63;
/// The session request methods a site may name.
const NEW: &str = "session/new";
const REOPEN: [&str; 2] = ["session/load", "session/resume"];

/// One connection's webpage sessions: the folders made for `session/new` requests the agent has not answered yet.
#[derive(Debug, Default)]
pub(crate) struct Sites {
    documents: Option<PathBuf>,
    pending: Mutex<BTreeMap<String, Value>>,
}

/// A client message that names a site.
struct Request {
    id: Option<Value>,
    method: String,
    /// What the client called the site, untrusted.
    site: Option<String>,
    value: Value,
}

impl Request {
    fn read(message: &[u8]) -> Option<Self> {
        // A cheap look first: most messages name no site.
        if !message
            .windows(SITE_META.len())
            .any(|window| window == SITE_META.as_bytes())
        {
            return None;
        }
        let value: Value = serde_json::from_slice(message).ok()?;
        let method = value.get("method")?.as_str()?;
        if method != NEW && !REOPEN.contains(&method) {
            return None;
        }
        let named = value.pointer("/params/_meta")?.get(SITE_META)?;
        Some(Self {
            id: value.get("id").cloned(),
            method: method.to_owned(),
            site: named.get("site").and_then(Value::as_str).map(str::to_owned),
            value,
        })
    }
}

/// A folder chosen for a session.
struct Placed {
    folder: PathBuf,
    id: String,
    site: String,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Sites {
    pub(crate) fn new(documents: Option<PathBuf>) -> Self {
        Self {
            documents,
            pending: Mutex::new(BTreeMap::new()),
        }
    }

    /// What becomes of a client's message: [`Scope::check`], except that a session request naming a site gets the folder the
    /// host chose as its `cwd` (a `cwd` the client sent is replaced) and the folder is made once the scope has passed it. A
    /// site that is not a plain host name, or a folder to reopen outside the site's own, fails the request, and nothing is made.
    pub(crate) fn check(&self, scope: &Scope, message: &[u8]) -> Verdict {
        let Some(request) = Request::read(message) else {
            return scope.check(message);
        };
        let refuse = |why: &str| {
            Verdict::Refuse(request.id.as_ref().map(|id| {
                let reply = json!({"jsonrpc": "2.0", "id": id, "error": {"code": INVALID_PARAMS, "message": why}});
                serde_json::to_vec(&reply).unwrap_or_default()
            }))
        };
        let placed = match self.place(&request) {
            Ok(placed) => placed,
            Err(why) => return refuse(why),
        };
        let mut value = request.value.clone();
        if let Some(params) = value.get_mut("params").and_then(Value::as_object_mut) {
            params.insert(
                "cwd".to_owned(),
                Value::String(placed.folder.to_string_lossy().into_owned()),
            );
        }
        let Ok(rewritten) = serde_json::to_vec(&value) else {
            return refuse("the request could not be written");
        };
        let Verdict::Forward(bytes) = scope.check(&rewritten) else {
            return scope.check(&rewritten);
        };
        if request.method == NEW {
            let mut pending = lock(&self.pending);
            if pending.len() >= MAX_PENDING {
                return refuse("too many sessions wait for the agent's answer");
            }
            let key = request.id.as_ref().map(ToString::to_string);
            if let Some(key) = key {
                pending.insert(
                    key,
                    json!({"site": placed.site, "id": placed.id, "folder": placed.folder.to_string_lossy()}),
                );
            }
        }
        if make_private(&placed.folder).is_err() {
            lock(&self.pending).retain(|_, held| held["id"] != json!(placed.id));
            return refuse("the folder for this session could not be made");
        }
        Verdict::Forward(bytes)
    }

    fn place(&self, request: &Request) -> Result<Placed, &'static str> {
        let site = request
            .site
            .as_deref()
            .and_then(site_name)
            .ok_or("the site is not a host name that is safe as a folder name")?;
        let documents = self
            .documents
            .clone()
            .or_else(system_documents)
            .ok_or("this host has no Documents folder")?;
        let beside = documents.join("Canoa").join("scratch").join(site);
        if request.method == NEW {
            let id = new_id().ok_or("this host could not make a session id")?;
            return Ok(Placed {
                folder: beside.join(&id),
                id,
                site: site.to_owned(),
            });
        }
        // Reopening: the folder the host recorded, which must be one session's own under this site's.
        let named = request
            .value
            .pointer("/params/cwd")
            .and_then(Value::as_str)
            .ok_or("a session of a site is reopened in the folder it was made in")?;
        let folder = share::plain(Path::new(named)).into_owned();
        let mut rest = folder
            .strip_prefix(&beside)
            .map_err(|_| "that folder is not one this host made for this site")?
            .components();
        let id = match (rest.next(), rest.next()) {
            (Some(Component::Normal(name)), None) => name.to_str().filter(|name| session_id(name)),
            _ => None,
        }
        .ok_or("that folder is not one this host made for this site")?;
        Ok(Placed {
            id: id.to_owned(),
            folder,
            site: site.to_owned(),
        })
    }

    /// The agent's message to the client, with the folder the host made added to its answer to a site's `session/new`. An
    /// answer to any other request carries none, whatever the agent wrote there.
    pub(crate) fn annotate<'a>(&self, message: &'a [u8]) -> Cow<'a, [u8]> {
        let named = message
            .windows(SITE_META.len())
            .any(|window| window == SITE_META.as_bytes());
        if !named && lock(&self.pending).is_empty() {
            return Cow::Borrowed(message);
        }
        let Ok(mut value) = serde_json::from_slice::<Value>(message) else {
            return Cow::Borrowed(message);
        };
        if value.get("method").is_some() {
            return Cow::Borrowed(message);
        }
        let told = value
            .get("id")
            .and_then(|id| lock(&self.pending).remove(&id.to_string()));
        let Some(result) = value.get_mut("result").and_then(Value::as_object_mut) else {
            return Cow::Borrowed(message);
        };
        let mut changed = false;
        if let Some(meta) = result.get_mut("_meta").and_then(Value::as_object_mut) {
            changed = meta.remove(SITE_META).is_some();
        }
        if let Some(told) = told {
            let meta = result
                .entry("_meta")
                .or_insert_with(|| Value::Object(Map::new()));
            if let Some(meta) = meta.as_object_mut() {
                meta.insert(SITE_META.to_owned(), told);
                changed = true;
            }
        }
        if !changed {
            return Cow::Borrowed(message);
        }
        serde_json::to_vec(&value).map_or(Cow::Borrowed(message), Cow::Owned)
    }
}

/// A new session id, as a version 4 UUID.
fn new_id() -> Option<String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).ok()?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: Vec<String> = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    Some(format!(
        "{}-{}-{}-{}-{}",
        hex[..4].concat(),
        hex[4..6].concat(),
        hex[6..8].concat(),
        hex[8..10].concat(),
        hex[10..].concat()
    ))
}

/// Whether `name` is a session folder's name as the host makes it: letters, digits and hyphens, no longer than a UUID.
fn session_id(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 36
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        && !is_device(name)
}

/// Makes `folder` and the folders above it, private to the user on Unix.
fn make_private(folder: &Path) -> std::io::Result<()> {
    let mut made = std::fs::DirBuilder::new();
    made.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut made, 0o700);
    made.create(folder)
}

/// Whether a label is a name Windows reserves for a device, which it reserves with an extension too.
fn is_device(label: &str) -> bool {
    let lower = label.to_ascii_lowercase();
    matches!(lower.as_str(), "con" | "prn" | "aux" | "nul")
        || ["com", "lpt"].iter().any(|prefix| {
            lower
                .strip_prefix(prefix)
                .is_some_and(|digit| matches!(digit.as_bytes(), [b'1'..=b'9']))
        })
}

/// `text` when it is already one safe folder name for a site, else nothing: lower case, an international name as punycode,
/// no port, path, credentials or trailing dot. Refused, never repaired: an empty name, one over 253 characters, an empty
/// label or one over 63, a label with anything but letters, digits and inner hyphens (so no `..`, separator, `:`, percent
/// sign, space or control character, and no IPv6 address), a first label Windows reserves as a device name, and a name of
/// numbers that is not an IPv4 address.
pub(crate) fn site_name(text: &str) -> Option<&str> {
    if text.is_empty() || text.len() > MAX_NAME {
        return None;
    }
    let mut numbers = true;
    for (at, label) in text.split('.').enumerate() {
        let bytes = label.as_bytes();
        let inner = |byte: u8| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-';
        let edge = |byte: u8| byte.is_ascii_lowercase() || byte.is_ascii_digit();
        if bytes.is_empty()
            || bytes.len() > MAX_LABEL
            || !bytes.iter().copied().all(inner)
            || !bytes.first().copied().is_some_and(edge)
            || !bytes.last().copied().is_some_and(edge)
        {
            return None;
        }
        if at == 0 && is_device(label) {
            return None;
        }
        numbers &= bytes.iter().all(u8::is_ascii_digit);
    }
    (!numbers || ipv4(text)).then_some(text)
}

/// Four numbers of 0 to 255, none with a leading zero.
fn ipv4(text: &str) -> bool {
    let parts: Vec<&str> = text.split('.').collect();
    parts.len() == 4
        && parts
            .iter()
            .all(|part| part.parse::<u8>().is_ok() && (part.len() == 1 || !part.starts_with('0')))
}

/// This machine's Documents folder as the system names it.
fn system_documents() -> Option<PathBuf> {
    let home = std::env::home_dir();
    let env = |name: &str| std::env::var(name).ok();
    let from_env = documents_in(env, home.clone(), None);
    if env("XDG_DOCUMENTS_DIR").is_some_and(|named| !named.is_empty()) {
        return from_env;
    }
    #[cfg(windows)]
    if let Some(known) = home
        .as_deref()
        .and_then(crate::store::footprint::real_documents)
    {
        return Some(known);
    }
    #[cfg(target_os = "linux")]
    {
        let config = env("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|home| home.join(".config")))?;
        let listed = std::fs::read_to_string(config.join("user-dirs.dirs")).ok();
        documents_in(env, home, listed.as_deref())
    }
    #[cfg(not(target_os = "linux"))]
    from_env
}

/// The Documents folder from `XDG_DOCUMENTS_DIR` in `env` (absolute, or from `$HOME`), else from the text of
/// `user-dirs.dirs`, else `Documents` in `home`.
pub(crate) fn documents_in(
    env: impl Fn(&str) -> Option<String>,
    home: Option<PathBuf>,
    user_dirs: Option<&str>,
) -> Option<PathBuf> {
    let listed = || {
        user_dirs?.lines().find_map(|line| {
            let value = line.trim().strip_prefix("XDG_DOCUMENTS_DIR")?;
            let value = value.trim_start().strip_prefix('=')?.trim();
            Some(value.trim_matches('"').to_owned())
        })
    };
    let named = env("XDG_DOCUMENTS_DIR")
        .filter(|named| !named.is_empty())
        .or_else(listed);
    if let Some(named) = named {
        let expanded = match (named.strip_prefix("$HOME"), &home) {
            (Some(rest), Some(home)) if rest.is_empty() || rest.starts_with('/') => {
                home.join(rest.trim_start_matches('/'))
            }
            _ => PathBuf::from(&named),
        };
        if expanded.is_absolute() {
            return Some(expanded);
        }
    }
    home.map(|home| home.join("Documents"))
}

#[cfg(test)]
mod tests;
