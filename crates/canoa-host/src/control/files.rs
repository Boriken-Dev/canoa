//! What a paired page may do with this computer's files over the control channel: browse folders, read and write canoa.cfg
//! by named section, and upload files. Each is behind a grant of its own, and every limit is a setting.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files and locks are std's"
)]

use std::fs;
use std::io::Write as _;
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use super::{Caller, EntryRoots, REFUSED, bad, private_dir, text_param, whole_machine};

type Outcome = Result<Value, (i32, String)>;

/// The most entries one `fs/list` answers.
pub const MAX_LIST: usize = 1000;
/// The largest upload, in bytes, unless `[host.control] max_upload_bytes` says otherwise. A request carries it in base64.
pub const DEFAULT_MAX_UPLOAD_BYTES: u64 = 8 * 1024 * 1024;
/// What one owner (a key, or the app) may keep uploaded, in bytes, unless `max_upload_total_bytes` says otherwise.
pub const DEFAULT_MAX_UPLOAD_TOTAL: u64 = 256 * 1024 * 1024;
/// The files one owner may keep uploaded, unless `max_uploads` says otherwise.
pub const DEFAULT_MAX_UPLOADS: usize = 256;
/// The most raw bytes one `upload/put` carries: a page's transport takes no message over a mebibyte, base64 and sealing included.
pub const MAX_CHUNK: usize = 512 * 1024;
/// Where an upload not yet done is kept, apart from the files a name could collide with.
const PARTIAL: &str = ".partial";
/// The most of a file's name that is kept.
const MAX_NAME: usize = 100;
/// The largest section `config/set` writes, in bytes of JSON.
pub const MAX_SECTION_BYTES: usize = 256 * 1024;

/// The tables of canoa.cfg a page may read and write: the app's own settings. `host`, `agents`, `profile`, `desktop` and
/// `skill_pins` name programs to run or what may run, so they stay the person's to edit in the file.
pub const SECTIONS: &[&str] = &[
    "appearance",
    "assets",
    "attachments",
    "catalogs",
    "loadout",
    "network",
    "notifications",
    "overlays",
    "permissions",
    "sessions",
    "store",
    "usage",
];

/// The tables that change what an agent may do or is given (the permission mode and protected paths, the overlays and skills
/// every `own_profile` session starts with): writing them takes the grant `config.write.sessions`, which is as much as access to
/// the agent.
pub const GUARDED_SECTIONS: &[&str] = &["loadout", "overlays", "permissions"];

/// The sections the host itself reads, which it takes up when it next starts.
const READ_AT_START: &[&str] = &["permissions", "usage"];

/// The limits of uploads.
#[derive(Clone, Copy, Debug)]
pub(crate) struct UploadLimits {
    pub(crate) file_bytes: u64,
    pub(crate) total_bytes: u64,
    pub(crate) files: usize,
}

impl Default for UploadLimits {
    fn default() -> Self {
        Self {
            file_bytes: DEFAULT_MAX_UPLOAD_BYTES,
            total_bytes: DEFAULT_MAX_UPLOAD_TOTAL,
            files: DEFAULT_MAX_UPLOADS,
        }
    }
}

/// Where a page's files are, and what it may do there.
#[derive(Debug, Default)]
pub(crate) struct Files {
    /// This user's home, which every key may browse.
    home: Option<PathBuf>,
    /// The working roots of each entry, and the keys that may use it.
    roots: Vec<EntryRoots>,
    /// The person's own canoa.cfg, the one file a page writes.
    config: Option<PathBuf>,
    /// Where uploads are kept, a directory for each owner.
    uploads: Option<PathBuf>,
    limits: UploadLimits,
    /// One write of canoa.cfg at a time.
    writing: Mutex<()>,
    /// One upload step at a time, so the limits are checked and spent together.
    uploading: Mutex<()>,
}

impl Files {
    pub(crate) fn new(
        home: Option<PathBuf>,
        roots: Vec<EntryRoots>,
        config: Option<PathBuf>,
        uploads: Option<PathBuf>,
        limits: UploadLimits,
    ) -> Self {
        Self {
            home,
            roots,
            config,
            uploads,
            limits,
            writing: Mutex::new(()),
            uploading: Mutex::new(()),
        }
    }

    /// The places `caller` may browse: the whole machine for the owner, else the home and the working roots of the entries
    /// the key may use, each resolved and no longer than they are.
    fn roots_of(&self, caller: &Caller, with_home: bool) -> Vec<PathBuf> {
        let wanted: Vec<PathBuf> = match caller {
            Caller::Owner => whole_machine(),
            Caller::Peer { id, .. } => self
                .home
                .iter()
                .filter(|_| with_home)
                .cloned()
                .chain(
                    self.roots
                        .iter()
                        .filter(|entry| {
                            entry
                                .peers
                                .as_ref()
                                .is_none_or(|peers| peers.iter().any(|peer| peer == id))
                        })
                        .flat_map(|entry| entry.roots.iter().cloned()),
                )
                .collect(),
        };
        let mut roots: Vec<PathBuf> = Vec::new();
        for root in wanted {
            if let Ok(resolved) = fs::canonicalize(&root) {
                let resolved = plain(resolved);
                if !roots.contains(&resolved) {
                    roots.push(resolved);
                }
            }
        }
        roots
    }

    /// Whether `caller` may name the place on this computer `repo` stands for as a source: it needs the `folders` grant, and the
    /// place must be under a working root of an entry it may use (not the whole home), not a folder this host keeps for itself,
    /// and a repository's own top level: a `.git` folder (not a file that points elsewhere), or a bare repository.
    pub(crate) fn may_read_source(&self, caller: &Caller, repo: &str) -> bool {
        if !caller.may(super::Grant::Folders) {
            return false;
        }
        let path = super::overlays::local_path(repo);
        let Ok(resolved) = confined(&path.to_string_lossy(), &self.roots_of(caller, false)) else {
            return false;
        };
        let kept = self
            .config
            .iter()
            .filter_map(|file| file.parent())
            .chain(
                self.uploads
                    .iter()
                    .filter_map(|dir| dir.parent().and_then(Path::parent)),
            )
            .filter_map(|dir| fs::canonicalize(dir).ok().map(plain));
        if kept.into_iter().any(|dir| resolved.starts_with(dir)) {
            return false;
        }
        // The place must be a repository's own top level, or git could fall back to a sibling `<path>.git`.
        fs::symlink_metadata(resolved.join(".git")).is_ok_and(|meta| meta.is_dir())
            || (resolved.join("HEAD").is_file() && resolved.join("objects").is_dir())
    }

    /// `fs/list`: the folders in `path`, or the places to start from when it names none.
    pub(crate) fn list(&self, caller: &Caller, params: &Value) -> Outcome {
        let roots = self.roots_of(caller, true);
        let hidden = params.get("hidden") == Some(&Value::Bool(true));
        let files = params.get("files") == Some(&Value::Bool(true));
        let Some(asked) = text_param(params, "path")? else {
            let places: Vec<Value> = roots
                .iter()
                .map(|root| json!({"name": shown_name(root), "path": text_of(root), "dir": true}))
                .collect();
            return Ok(json!({"path": null, "entries": places, "more": false}));
        };
        let path = confined(asked, &roots)?;
        let metadata = fs::metadata(&path).map_err(|_| not_found())?;
        if !metadata.is_dir() {
            return Err(bad("that is not a folder"));
        }
        let reader = fs::read_dir(&path).map_err(|_| not_found())?;
        let mut entries: Vec<(String, bool, PathBuf)> = Vec::new();
        let mut more = false;
        for entry in reader.flatten() {
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if !listable(&name) || (!hidden && name.starts_with('.')) {
                continue;
            }
            // What a link points to counts, so a link out of the roots is not listed.
            let target = entry.path();
            let Ok(resolved) = fs::canonicalize(&target).map(plain) else {
                continue;
            };
            if !roots.iter().any(|root| resolved.starts_with(root)) {
                continue;
            }
            let Ok(kind) = fs::metadata(&resolved) else {
                continue;
            };
            if kind.is_dir() || (files && kind.is_file()) {
                if entries.len() >= MAX_LIST {
                    more = true;
                    break;
                }
                entries.push((name, kind.is_dir(), resolved));
            }
        }
        entries.sort_by(|a, b| {
            b.1.cmp(&a.1)
                .then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase()))
                .then_with(|| a.0.cmp(&b.0))
        });
        let parent = path
            .parent()
            .map(Path::to_path_buf)
            .filter(|parent| roots.iter().any(|root| parent.starts_with(root)))
            .map(|parent| Value::String(text_of(&parent)))
            .unwrap_or(Value::Null);
        Ok(json!({
            "path": text_of(&path),
            "parent": parent,
            "entries": entries
                .iter()
                .map(|(name, dir, resolved)| json!({"name": name, "path": text_of(resolved), "dir": dir}))
                .collect::<Vec<_>>(),
            "more": more,
        }))
    }

    fn config_path(&self) -> Result<&Path, (i32, String)> {
        self.config.as_deref().ok_or_else(|| {
            (
                REFUSED,
                "this host was not started with a canoa.cfg a page may edit".to_owned(),
            )
        })
    }

    /// `config/get`: one named section of the person's own canoa.cfg, as JSON, or null when it has none.
    pub(crate) fn config_get(&self, params: &Value) -> Outcome {
        let section = section_of(params)?;
        let path = self.config_path()?;
        let (text, _) = read_cfg(path)?;
        let table = parse(path, &text)?;
        Ok(json!({
            "section": section,
            "value": table.get(section).and_then(|value| serde_json::to_value(value).ok()),
            "restart": false,
        }))
    }

    /// `config/set`: replaces one named section of the person's own canoa.cfg with an object, or removes it with null. The file
    /// is written in the format it is in, after the host has read what it would hold.
    pub(crate) fn config_set(&self, params: &Value, guarded: bool) -> Outcome {
        let section = section_of(params)?;
        if GUARDED_SECTIONS.contains(&section) && !guarded {
            return Err((
                super::DENIED,
                format!("{section} takes the grant config.write.sessions"),
            ));
        }
        let path = self.config_path()?;
        let value = params.get("value").unwrap_or(&Value::Null);
        if !(value.is_null() || value.is_object()) {
            return Err(bad("value is an object, or null to remove the section"));
        }
        if serde_json::to_vec(value).map_or(usize::MAX, |bytes| bytes.len()) > MAX_SECTION_BYTES {
            return Err((
                REFUSED,
                format!("a section is at most {MAX_SECTION_BYTES} bytes"),
            ));
        }
        let _one = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        let (text, existed) = read_cfg(path)?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("canoa.cfg");
        let formats = crate::format::formats();
        let mut document = parse(path, &text)?;
        let before = document.clone();
        match toml_value(value) {
            Some(toml::Value::Table(table)) => {
                document.insert(section.to_owned(), toml::Value::Table(table));
            }
            _ => {
                document.remove(section);
            }
        }
        let document = toml::Value::Table(document);
        let format = if text.trim().is_empty() && !existed {
            "toml".to_owned()
        } else {
            formats
                .detect(text.as_bytes(), anycfg::Options::new().name(name))
                .map(|detected| detected.format.to_owned())
                .map_err(|_| refused("canoa.cfg is in no format this host reads"))?
        };
        let extension = path.extension().and_then(|ext| ext.to_str());
        let bytes = if extension.is_some_and(|ext| ext.eq_ignore_ascii_case("cfg")) {
            formats.to_auto_vec(&document, &format, "cfg")
        } else {
            formats.to_vec(&document, &format)
        }
        .map_err(|_| refused("that section has no form in canoa.cfg's format"))?;
        let written = String::from_utf8(bytes).map_err(|_| refused("canoa.cfg is not text"))?;
        // What the host itself reads must still be well formed, or the next start would refuse the file.
        crate::config::Config::from_str_named(
            &written,
            name,
            path.parent().unwrap_or_else(|| Path::new(".")),
        )
        .map_err(|error| refused(&format!("that would leave canoa.cfg refused: {error}")))?;
        // Nothing but the named table may differ once the text is read back.
        let mut reread = parse(path, &written)?;
        let mut kept = before;
        reread.remove(section);
        kept.remove(section);
        if reread != kept {
            return Err(refused(
                "that would change a table other than the one named",
            ));
        }
        // A canoa.cfg that is a link is written through to its target.
        let target = fs::canonicalize(path).map_or_else(|_| path.to_path_buf(), plain);
        write_beside(&target, written.as_bytes())
            .map_err(|error| refused(&format!("canoa.cfg could not be written: {error}")))?;
        Ok(json!({"section": section, "restart": READ_AT_START.contains(&section)}))
    }

    /// `upload/put`: keeps a file a page sends, in a folder of this caller's, and answers where. A file travels in chunks of at most
    /// [`MAX_CHUNK`] bytes, since the transport a page uses carries no larger message: the first call (`name`, `data`) names the file
    /// and, with `done: false`, answers an `id` to continue with (`id`, `offset` the size so far, `data`); the call with `done`
    /// true or absent ends it. A file sent in one chunk needs no `id`. A file is kept in the folder `.partial` until it is done.
    pub(crate) fn upload_put(&self, caller: &Caller, params: &Value) -> Outcome {
        let dir = self.owner_dir(caller)?;
        let _one = self
            .uploading
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let data = text_param(params, "data")?.ok_or_else(|| bad("data is needed"))?;
        // Before decoding, so a page cannot make the host hold more than it would keep.
        if data.len() > MAX_CHUNK / 3 * 4 + 8 {
            return Err(refused(&format!("a chunk is at most {MAX_CHUNK} bytes")));
        }
        let bytes = data_encoding::BASE64
            .decode(data.as_bytes())
            .map_err(|_| bad("data is base64"))?;
        if bytes.len() > MAX_CHUNK {
            return Err(refused(&format!("a chunk is at most {MAX_CHUNK} bytes")));
        }
        let done = params.get("done") != Some(&Value::Bool(false));
        let (count, held) = usage(&dir);
        let added = bytes.len() as u64;
        let (stored, part, so_far) = if let Some(id) = text_param(params, "id")? {
            if !stored_name(id) {
                return Err(bad("id is what upload/put answered"));
            }
            let part = dir.join(PARTIAL).join(id);
            let so_far = fs::metadata(&part)
                .map(|meta| meta.len())
                .map_err(|_| refused("that upload is not under way: start it again"))?;
            if params.get("offset").and_then(Value::as_u64) != Some(so_far) {
                return Err(refused(&format!(
                    "offset is {so_far}, where the upload stands"
                )));
            }
            (id.to_owned(), part, so_far)
        } else {
            let name = text_param(params, "name")?.ok_or_else(|| bad("name is needed"))?;
            if count >= self.limits.files {
                return Err(refused(
                    "this caller has no room for another upload: remove some",
                ));
            }
            private_dir(&dir.join(PARTIAL))
                .map_err(|error| refused(&format!("uploads could not be kept: {error}")))?;
            forget_stale(&dir);
            let mut tag = [0_u8; 6];
            getrandom::fill(&mut tag).map_err(|_| refused("no randomness to name the upload"))?;
            let stored = format!(
                "{}-{}",
                data_encoding::HEXLOWER.encode(&tag),
                file_name_of(name)
            );
            let part = dir.join(PARTIAL).join(&stored);
            (stored, part, 0)
        };
        if so_far + added > self.limits.file_bytes {
            let _ = fs::remove_file(&part);
            return Err(too_big(self.limits.file_bytes));
        }
        if held + added > self.limits.total_bytes {
            return Err(refused(
                "this caller has no room for another upload: remove some",
            ));
        }
        let mut file = if so_far == 0 {
            private_new(&part)
        } else {
            fs::OpenOptions::new().append(true).open(&part)
        }
        .map_err(|error| refused(&format!("the upload could not be kept: {error}")))?;
        file.write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|error| {
                let _ = fs::remove_file(&part);
                refused(&format!("the upload could not be kept: {error}"))
            })?;
        drop(file);
        let path = dir.join(&stored);
        if done {
            fs::rename(&part, &path).map_err(|error| {
                let _ = fs::remove_file(&part);
                refused(&format!("the upload could not be kept: {error}"))
            })?;
        }
        Ok(json!({
            "id": stored,
            "path": text_of(&path),
            "name": stored,
            "size": so_far + added,
            "done": done,
        }))
    }

    /// `upload/remove`: deletes a file this caller uploaded, by its `id` (the `name` it answered), and what is left of one not finished.
    pub(crate) fn upload_remove(&self, caller: &Caller, params: &Value) -> Outcome {
        let dir = self.owner_dir(caller)?;
        let _one = self
            .uploading
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let Ok(base) = fs::canonicalize(&dir).map(plain) else {
            return Ok(json!({"removed": false}));
        };
        // Only the name an upload was given, never a path: nothing here is opened by a caller's text.
        let name = text_param(params, "id")?
            .or(text_param(params, "name")?)
            .ok_or_else(|| bad("id is needed"))?
            .to_owned();
        if !stored_name(&name) {
            return Ok(json!({"removed": false}));
        }
        let mut removed = false;
        for target in [base.join(&name), base.join(PARTIAL).join(&name)] {
            if fs::symlink_metadata(&target).is_ok_and(|meta| meta.is_file()) {
                fs::remove_file(&target).map_err(|error| {
                    refused(&format!("the upload could not be removed: {error}"))
                })?;
                removed = true;
            }
        }
        Ok(json!({"removed": removed}))
    }

    /// The folder this caller's uploads are in: one for each key, named by a digest of its id.
    fn owner_dir(&self, caller: &Caller) -> Result<PathBuf, (i32, String)> {
        let root = self
            .uploads
            .as_deref()
            .ok_or_else(|| (REFUSED, "this host keeps no uploads".to_owned()))?;
        let digest = Sha256::digest(caller.owner().as_bytes());
        Ok(root.join(data_encoding::HEXLOWER.encode(&digest[..12])))
    }
}

/// Whether `name` is one `upload/put` made: 12 hexadecimal digits, a hyphen and a plain name.
fn stored_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    name.len() > 13
        && bytes[..12].iter().all(u8::is_ascii_hexdigit)
        && bytes[12] == b'-'
        && !name.contains(['/', '\\', ':'])
        && !name.contains("..")
        && !name.chars().any(char::is_control)
}

/// Takes away the unfinished uploads nobody has continued for an hour.
fn forget_stale(dir: &Path) {
    let Ok(reader) = fs::read_dir(dir.join(PARTIAL)) else {
        return;
    };
    for entry in reader.flatten() {
        let stale = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|at| at.elapsed().ok())
            .is_some_and(|age| age > std::time::Duration::from_secs(3600));
        if stale {
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn too_big(limit: u64) -> (i32, String) {
    (REFUSED, format!("an upload is at most {limit} bytes"))
}

fn refused(what: &str) -> (i32, String) {
    (REFUSED, what.to_owned())
}

fn not_found() -> (i32, String) {
    (super::NOT_FOUND, "no such folder".to_owned())
}

fn section_of(params: &Value) -> Result<&str, (i32, String)> {
    let section = text_param(params, "section")?.ok_or_else(|| bad("section is needed"))?;
    if SECTIONS.contains(&section) {
        Ok(section)
    } else {
        Err((
            super::DENIED,
            format!("{section} is not a section a page may edit"),
        ))
    }
}

/// The files and bytes `dir` holds.
fn usage(dir: &Path) -> (usize, u64) {
    let mut count = 0;
    let mut bytes = 0;
    for place in [dir.to_path_buf(), dir.join(PARTIAL)] {
        if let Ok(reader) = fs::read_dir(place) {
            for entry in reader.flatten() {
                if let Ok(meta) = entry.metadata()
                    && meta.is_file()
                {
                    count += 1;
                    bytes += meta.len();
                }
            }
        }
    }
    (count, bytes)
}

/// A name a file can be kept under: the last component, with anything but letters, digits and `.-_ ` made `_`, not led by a dot.
fn file_name_of(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or("");
    let mut kept: String = last
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect();
    while kept.len() > MAX_NAME {
        kept.pop();
    }
    let kept = kept.trim_matches([' ', '.']).to_owned();
    if kept.is_empty() {
        "upload".to_owned()
    } else {
        kept
    }
}

/// Whether a directory entry's name may be shown: no control or bidirectional characters.
fn listable(name: &str) -> bool {
    !name.is_empty()
        && name.chars().all(|c| {
            !c.is_control() && !matches!(c, '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
}

/// Whether `text` opens with `\` or `//`: a network share, a verbatim path or a device path, which a filesystem call would open.
pub(crate) fn network_path(text: &str) -> bool {
    let text = text.replace('/', "\\");
    text.starts_with("\\\\") || text.starts_with("\\??\\")
}

/// `asked` resolved, when it is plain and under one of `roots`.
fn confined(asked: &str, roots: &[PathBuf]) -> Result<PathBuf, (i32, String)> {
    let path = Path::new(asked);
    // A network share, a verbatim or a device path is refused by its text, before anything opens it.
    if network_path(asked) {
        return Err(bad("that is a network, verbatim or device path"));
    }
    if !roots.iter().any(|root| text_under(asked, root)) {
        return Err(not_found());
    }
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::CurDir | Component::ParentDir))
        || asked.split(['/', '\\']).any(|segment| segment == "..")
        || asked.chars().any(char::is_control)
    {
        return Err(bad("path is absolute and plain"));
    }
    let resolved = fs::canonicalize(path).map(plain).map_err(|_| not_found())?;
    if roots.iter().any(|root| resolved.starts_with(root)) {
        Ok(resolved)
    } else {
        // Not saying whether it exists.
        Err(not_found())
    }
}

/// Whether the text `path` names `root` or something below it, compared as text with no filesystem call.
fn text_under(path: &str, root: &Path) -> bool {
    let fold = |text: &str| {
        let text = text.replace('\\', "/");
        let text = text.trim_end_matches('/').to_owned();
        if cfg!(windows) {
            text.to_lowercase()
        } else {
            text
        }
    };
    let (path, root) = (fold(path), fold(&root.to_string_lossy()));
    path == root || path.starts_with(&format!("{root}/"))
}

fn shown_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map_or_else(|| text_of(path), str::to_owned)
}

fn text_of(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// `path` without the `\\?\` prefix Windows' resolution adds to a drive's path.
fn plain(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC\\") => PathBuf::from(rest),
        _ => path,
    }
}

/// canoa.cfg's text, and whether the file is there; empty when it is not.
fn read_cfg(path: &Path) -> Result<(String, bool), (i32, String)> {
    match fs::read(path) {
        Ok(bytes) => String::from_utf8(bytes)
            .map(|text| (text, true))
            .map_err(|_| refused("canoa.cfg is not text")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok((String::new(), false)),
        Err(error) => Err(refused(&format!("canoa.cfg could not be read: {error}"))),
    }
}

fn parse(path: &Path, text: &str) -> Result<toml::Table, (i32, String)> {
    if text.trim().is_empty() {
        return Ok(toml::Table::new());
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("canoa.cfg");
    match crate::format::formats().from_slice(text.as_bytes(), anycfg::Options::new().name(name)) {
        Ok(toml::Value::Table(table)) => Ok(table),
        Ok(_) => Err(refused("canoa.cfg is not a table of settings")),
        Err(_) => Err(refused("canoa.cfg is not valid in its format")),
    }
}

/// JSON as TOML's model, which canoa.cfg is read as; `None` for what it cannot hold (a null at the top).
fn toml_value(value: &Value) -> Option<toml::Value> {
    Some(match value {
        Value::Null => return None,
        Value::Bool(b) => toml::Value::Boolean(*b),
        Value::Number(n) => n
            .as_i64()
            .map(toml::Value::Integer)
            .or_else(|| n.as_f64().map(toml::Value::Float))?,
        Value::String(s) => toml::Value::String(s.clone()),
        Value::Array(items) => toml::Value::Array(items.iter().filter_map(toml_value).collect()),
        Value::Object(map) => toml::Value::Table(
            map.iter()
                .filter_map(|(key, value)| Some((key.clone(), toml_value(value)?)))
                .collect(),
        ),
    })
}

/// Writes `bytes` to `path` whole, beside it first so a stop in the middle leaves the old file, readable by this user alone.
fn write_beside(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    let mut beside = path.as_os_str().to_owned();
    beside.push(".new");
    let beside = PathBuf::from(beside);
    {
        let mut file = private_new(&beside)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(&beside, path)
}

fn private_new(path: &Path) -> std::io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(path)
}
