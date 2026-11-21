//! What a page cannot do for itself about registries and the Store's cache: `registry/fetch` (one page of an MCP registry's
//! list, read by the host), `host/which` (is a program on this host's `PATH`) and `store/contents` / `store/contents_put` (the
//! document the app keeps per catalog of what every entry provides).
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files, environment and processes are std's"
)]

use std::ffi::OsStr;
use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use super::{Caller, REFUSED, bad, text_param};
use crate::store::tools::{HttpTransport, ToolsLimits, Transport};

type Outcome = Result<Value, (i32, String)>;

/// The official registry, which every key may read.
pub(crate) const OFFICIAL: &str = "https://registry.modelcontextprotocol.io";
/// The most bytes of a registry's answer that are read.
pub(crate) const MAX_REGISTRY_BYTES: u64 = 4 * 1024 * 1024;
/// How long a registry has to answer, and again to send its body.
const REGISTRY_TIME: Duration = Duration::from_secs(20);
/// The most bytes of one catalog's document the cache keeps.
pub(crate) const MAX_DOCUMENT_BYTES: usize = 8 * 1024 * 1024;
/// The documents one caller's cache holds.
const MAX_DOCUMENTS: usize = 256;
/// The most names one `host/which` looks up.
const MAX_NAMES: usize = 32;

/// The registry reads and the cache of what catalogs provide, which the host keeps for the app.
#[derive(Clone)]
pub(crate) struct Registry {
    transport: Arc<dyn Transport>,
    /// Where each caller's documents are kept; none keeps nothing.
    cache: Option<PathBuf>,
}

impl core::fmt::Debug for Registry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Registry")
            .field("cache", &self.cache)
            .finish_non_exhaustive()
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new(None)
    }
}

impl Registry {
    /// Reads registries over HTTPS and keeps documents under `cache`.
    pub(crate) fn new(cache: Option<PathBuf>) -> Self {
        Self {
            transport: Arc::new(HttpTransport::default()),
            cache,
        }
    }

    /// Reads registries through `transport` instead.
    #[cfg(test)]
    pub(crate) fn with_transport(mut self, transport: Arc<dyn Transport>) -> Self {
        self.transport = transport;
        self
    }

    /// `registry/fetch` `{"url"}` -> `{"body", "warning"?}`: one page of a registry's list. The address must be a registry's list endpoint,
    /// `<base>/v0.1/servers` with at most the `limit`, `cursor` and `search` parameters, over https or http,
    /// with no user information; no redirect is followed. `bases` is where a key may read, or `None` for the person running the host.
    pub(crate) fn fetch(&self, params: &Value, bases: Option<&[String]>) -> Outcome {
        let url = text_param(params, "url")?.ok_or_else(|| bad("url is needed"))?;
        let base = list_endpoint_base(url)?;
        if let Some(bases) = bases
            && !bases.iter().any(|allowed| same_base(allowed, &base))
        {
            return Err((
                REFUSED,
                "this key may read only the registries canoa.cfg lists under [store.mcps.catalogs], and the official one".to_owned(),
            ));
        }
        let body = self.read(url, "registry", REGISTRY_TIME, MAX_REGISTRY_BYTES)?;
        let mut answer = json!({"body": String::from_utf8_lossy(&body)});
        if let Some(note) = canoa::address::plain_http_note("the registry", &base) {
            answer["warning"] = Value::String(note);
        }
        Ok(answer)
    }

    /// One answer of `url` as bytes, with no redirect followed: `what` names the service in the words of a refusal, `time` is how
    /// long it has to answer and again to send its body, and an answer of more than `most` bytes is refused.
    pub(super) fn read(
        &self,
        url: &str,
        what: &str,
        time: Duration,
        most: u64,
    ) -> Result<Vec<u8>, (i32, String)> {
        let limits = ToolsLimits {
            response_timeout: time,
            body_timeout: time,
            ..ToolsLimits::default()
        };
        let reply = self
            .transport
            .get(url, &limits)
            .map_err(|why| (REFUSED, format!("The {what} cannot be reached: {why}")))?;
        if reply.status != 200 {
            let redirect = if reply.status / 100 == 3 {
                " (a redirect, which Canoa does not follow)"
            } else {
                ""
            };
            return Err((
                REFUSED,
                format!("The {what} answered {}{redirect}", reply.status),
            ));
        }
        let mut body = Vec::new();
        reply
            .body
            .take(most + 1)
            .read_to_end(&mut body)
            .map_err(|error| {
                (
                    REFUSED,
                    format!("The {what} did not finish its answer: {error}"),
                )
            })?;
        if body.len() as u64 > most {
            return Err((
                REFUSED,
                format!("The {what}'s answer is larger than Canoa reads"),
            ));
        }
        Ok(body)
    }

    /// The folder of `caller`'s documents: the person's own, or one per key, so a key never reads what another wrote.
    fn folder(&self, caller: &Caller) -> Result<PathBuf, (i32, String)> {
        let root = self
            .cache
            .as_ref()
            .ok_or_else(|| (REFUSED, "this host keeps no cache".to_owned()))?;
        Ok(match caller {
            Caller::Owner => root.join("owner"),
            Caller::Peer { id, .. } => root.join(format!("key-{}", file_safe(id, 80))),
        })
    }

    /// `store/contents` `{"label"}` -> `{"document"}`: what the app kept for a catalog, or null.
    pub(crate) fn contents(&self, caller: &Caller, params: &Value) -> Outcome {
        let file = self.folder(caller)?.join(document_name(params)?);
        match fs::read_to_string(&file) {
            Ok(text) => Ok(json!({"document": text})),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(json!({"document": Value::Null}))
            }
            Err(error) => Err((REFUSED, format!("the cache cannot be read: {error}"))),
        }
    }

    /// `store/contents_put` `{"label", "document"}`: keeps the app's JSON document for a catalog. It is the caller's own cache and
    /// nothing else reads it; one document holds at most [`MAX_DOCUMENT_BYTES`].
    pub(crate) fn put_contents(&self, caller: &Caller, params: &Value) -> Outcome {
        let name = document_name(params)?;
        let document = text_param(params, "document")?.ok_or_else(|| bad("document is needed"))?;
        if document.len() > MAX_DOCUMENT_BYTES {
            return Err((
                REFUSED,
                format!("a document holds at most {MAX_DOCUMENT_BYTES} bytes"),
            ));
        }
        if !serde_json::from_str::<Value>(document).is_ok_and(|value| value.is_object()) {
            return Err(bad("document is a JSON object"));
        }
        let folder = self.folder(caller)?;
        fs::create_dir_all(&folder)
            .map_err(|error| (REFUSED, format!("the cache cannot be made: {error}")))?;
        let file = folder.join(&name);
        if !file.exists() && fs::read_dir(&folder).map_or(0, Iterator::count) >= MAX_DOCUMENTS {
            return Err((
                REFUSED,
                format!("the cache holds {MAX_DOCUMENTS} documents already"),
            ));
        }
        crate::replace::write_file_in_folder(&file, document.as_bytes(), crate::replace::SHARED)
            .map_err(|error| (REFUSED, format!("the cache cannot be written: {error}")))?;
        Ok(json!({}))
    }
}

/// `host/which` `{"names"}` -> `{"found": {name: path|null}}`: where each program is on this host's `PATH`, or in the folder of
/// a runtime the Store installed (`runtimes`, searched after `PATH`, so the person's own copy comes first). It only reads
/// directories and starts nothing.
pub(crate) fn which(params: &Value, runtimes: &[PathBuf]) -> Outcome {
    let names = params
        .get("names")
        .and_then(Value::as_array)
        .ok_or_else(|| bad("names is a list of program names"))?;
    if names.len() > MAX_NAMES {
        return Err(bad(&format!("at most {MAX_NAMES} names at once")));
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    let path = std::env::join_paths(std::env::split_paths(&path).chain(runtimes.iter().cloned()))
        .unwrap_or(path);
    let pathext = std::env::var("PATHEXT").unwrap_or_default();
    let mut found = serde_json::Map::new();
    for name in names {
        let name = name
            .as_str()
            .filter(|name| program_name(name))
            .ok_or_else(|| {
                bad("a program name is 1 to 64 letters, digits, `.`, `_`, `+` or `-`")
            })?;
        found.insert(
            name.to_owned(),
            which_in(&path, &pathext, name).map_or(Value::Null, |file| {
                Value::String(file.display().to_string())
            }),
        );
    }
    Ok(json!({"found": found}))
}

fn program_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._+-".contains(&byte))
}

/// The first runnable file called `name` in an absolute directory of `path`. On Windows a name takes each of `pathext`'s
/// extensions (the usual four when it is empty) unless it has one; elsewhere the file must be executable.
pub(crate) fn which_in(path: &OsStr, pathext: &str, name: &str) -> Option<PathBuf> {
    let suffixes: Vec<String> = if cfg!(windows) {
        let listed: Vec<String> = pathext
            .split(';')
            .filter(|ext| ext.starts_with('.') && ext.len() > 1)
            .map(str::to_ascii_lowercase)
            .collect();
        let listed = if listed.is_empty() {
            vec![".com".into(), ".exe".into(), ".bat".into(), ".cmd".into()]
        } else {
            listed
        };
        if listed
            .iter()
            .any(|ext| name.to_ascii_lowercase().ends_with(ext))
        {
            vec![String::new()]
        } else {
            listed
        }
    } else {
        vec![String::new()]
    };
    std::env::split_paths(path)
        .filter(|directory| directory.is_absolute())
        .flat_map(|directory| {
            suffixes
                .iter()
                .map(move |suffix| directory.join(format!("{name}{suffix}")))
        })
        .find(|candidate| runnable(candidate))
}

fn runnable(file: &Path) -> bool {
    let Ok(meta) = fs::metadata(file) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// A catalog label as a file name: the app's own rule, so what it kept before is found again.
fn document_name(params: &Value) -> Result<String, (i32, String)> {
    let label = text_param(params, "label")?.ok_or_else(|| bad("label is needed"))?;
    if label.is_empty() || label.len() > 128 {
        return Err(bad("label is 1 to 128 characters"));
    }
    Ok(format!("{}.json", file_safe(label, 128)))
}

fn file_safe(text: &str, most: usize) -> String {
    text.chars()
        .take(most)
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// The registry base a list endpoint address belongs to, when the address is one a registry serves: `https` or `http` (plain
/// http is the person's choice wherever it goes), no user information or fragment, a path ending `/v0.1/servers`, and only `limit`, `cursor` and `search` asked.
fn list_endpoint_base(url: &str) -> Result<String, (i32, String)> {
    let refused = |why: &str| (REFUSED, format!("not a registry's list address: {why}"));
    if url.len() > 2048
        || url
            .chars()
            .any(|c| c.is_control() || c == ' ' || c == '\\' || c == '#')
    {
        return Err(refused(
            "it holds a space, control character, backslash or fragment",
        ));
    }
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| refused("it has no scheme"))?;
    let end = rest.find(['/', '?']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    if authority.is_empty() || authority.contains('@') {
        return Err(refused("it has no host, or holds user information"));
    }
    let host = if let Some(inner) = authority.strip_prefix('[') {
        inner.split(']').next().map(|inside| format!("[{inside}]"))
    } else {
        authority.split(':').next().map(str::to_owned)
    }
    .unwrap_or_default()
    .to_ascii_lowercase();
    if host.is_empty()
        || !(scheme.eq_ignore_ascii_case("https") || scheme.eq_ignore_ascii_case("http"))
    {
        return Err(refused("it is an http or https address"));
    }
    let (path, query) = tail
        .split_once('?')
        .map_or((tail, ""), |(path, query)| (path, query));
    let prefix = path
        .strip_suffix("/v0.1/servers")
        .ok_or_else(|| refused("its path does not end /v0.1/servers"))?;
    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let key = pair.split('=').next().unwrap_or_default();
        if !matches!(key, "limit" | "cursor" | "search") {
            return Err(refused("it asks for more than limit, cursor and search"));
        }
    }
    Ok(format!("{scheme}://{authority}{prefix}").to_ascii_lowercase())
}

/// Whether `listed`, an address canoa.cfg lists, is the same registry as `base`.
fn same_base(listed: &str, base: &str) -> bool {
    listed.trim_end_matches('/').eq_ignore_ascii_case(base)
}

/// The registries a key may read: the official one and what `[store.mcps.catalogs]` lists, git sources left out.
pub(crate) fn listed_bases(store: &Value) -> Vec<String> {
    std::iter::once(OFFICIAL.to_owned())
        .chain(
            crate::store::catalogs::read(crate::store::catalogs::table_of(store, "mcps"))
                .into_iter()
                .filter_map(Result::ok)
                .filter(|entry| entry.provider == crate::store::catalogs::READ)
                .filter_map(|entry| entry.source().map(str::to_owned))
                .filter(|address| !address.contains(['@', '#'])),
        )
        .collect()
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
