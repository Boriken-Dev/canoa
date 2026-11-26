//! What a paired page may do with this computer's files over the control channel: browse folders, read and write canoa.cfg
//! by named section, and upload files. Each is behind a grant of its own, and every limit is a setting.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files and locks are std's"
)]

use alloc::sync::Arc;
use canoa::brand::CONFIG_FILE;
use std::fs;
use std::io::Read as _;
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, RwLock};

use canoa::store::protect::Protected;
use serde_json::{Value, json};

use super::{Caller, EntryRoots, REFUSED, bad, private_dir, text_param, whole_machine};

type Outcome = Result<Value, (i32, String)>;

/// The entries one `fs/list` answers unless the request names a `limit`; the rest are asked for with `offset`.
pub const DEFAULT_LIST: usize = 1000;
/// The bytes `fs/read` answers with when the request names no limit. A page's transport takes no message over a mebibyte, and the
/// bytes go in base64.
pub const DEFAULT_READ_BYTES: u64 = 256 * 1024;
/// The most bytes one `fs/read` answer carries, whatever the request asks: the transport's message, not a limit on the file. The
/// rest is read with `offset`, `next` saying where.
pub const READ_PAGE_BYTES: u64 = 512 * 1024;
/// How much of a file's start tells what it is, when a page far into it is read.
const HEAD_BYTES: u64 = 4096;
/// The largest upload, in bytes, unless `[host.control] max_upload_bytes` says otherwise. A request carries it in base64.
pub const DEFAULT_MAX_UPLOAD_BYTES: u64 = 8 * 1024 * 1024;
/// What one owner (a key, or the app) may keep uploaded, in bytes, unless `max_upload_total_bytes` says otherwise.
pub const DEFAULT_MAX_UPLOAD_TOTAL: u64 = 256 * 1024 * 1024;
/// The files one owner keep uploaded, unless `max_uploads` says otherwise.
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
    "attachments",
    "connections",
    "defaults",
    "loadouts",
    "network",
    "notifications",
    "overlays",
    "permissions",
    "profiles",
    "sessions",
    "store",
    "usage",
    "voice",
];

/// The tables that change what an agent may do or is given (the permission mode and protected paths, the overlays and skills
/// every session that names no profile starts with): writing them takes the grant `config.write.sessions`, which is as much as access to
/// the agent.
pub const GUARDED_SECTIONS: &[&str] = &[
    "defaults",
    "loadouts",
    "overlays",
    "permissions",
    "profiles",
];

/// The table of the host's own settings, which `config/get`, `config/set` and `config/check` serve only a key that holds `hosting`.
pub(crate) const HOST: &str = "host";

/// The tables only the app this host runs in reads and writes, since they name the programs it runs and what may run: another
/// key never edits them through `config/set`.
pub(crate) const OWNER_SECTIONS: &[&str] = &["agents", "desktop", "profile", "skill_pins"];

/// The code of a refusal because the table a write was made from is no longer the file's.
pub(crate) const CHANGED: i32 = -32004;

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

/// A configuration store, held where a page's reads and writes of canoa.cfg go through it.
#[derive(Clone)]
struct Kept(Arc<dyn canoa::config_store::ConfigStore>);

impl core::fmt::Debug for Kept {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("a configuration store")
    }
}

/// Where a page's files are, and what it may do there.
#[derive(Debug, Default)]
pub(crate) struct Files {
    /// This user's home, which every key may browse.
    home: Option<PathBuf>,
    /// The working roots of each entry, and the keys that may use it.
    roots: Vec<EntryRoots>,
    /// The folder beside the person's own canoa.cfg, where overlays install and trust is kept.
    config: Option<PathBuf>,
    /// Where the configuration lives: the one place a page's reads and writes of it go through.
    store: Option<Kept>,
    /// Where uploads are kept, a directory for each owner.
    uploads: Option<PathBuf>,
    limits: UploadLimits,
    /// One write of canoa.cfg at a time.
    writing: Mutex<()>,
    /// One upload step at a time, so the limits are checked and spent together.
    uploading: Mutex<()>,
    /// The Documents folder a chat's scratch folder is made under, when it is not the system's.
    documents: Option<PathBuf>,
    /// What can be started for a folder on this computer.
    opener: workspace::Opener,
    /// Configured `[host] openers`, or `None` to offer today's seeded three.
    openers: RwLock<Option<Vec<crate::config::OpenerConfig>>>,
    /// The person's protected paths, in gitignore syntax (`[permissions] protected`); the recommended list when none is written.
    protected: Option<Vec<String>>,
}

pub(crate) mod advanced;
mod config;
mod folders;
mod instructions;
mod links;
mod profiles;
mod rules;
mod uploads;
pub(crate) mod workspace;
pub(crate) use config::table_in;
pub(crate) use folders::Scratch;

impl Files {
    pub(crate) fn new(
        home: Option<PathBuf>,
        roots: Vec<EntryRoots>,
        config: Option<PathBuf>,
        uploads: Option<PathBuf>,
        limits: UploadLimits,
    ) -> Self {
        let store = config
            .as_deref()
            .map(|path| Kept(Arc::new(crate::config_store::FileStore::at(path))));
        Self {
            home,
            roots,
            config,
            store,
            uploads,
            limits,
            writing: Mutex::new(()),
            uploading: Mutex::new(()),
            documents: None,
            opener: workspace::Opener::default(),
            openers: RwLock::new(None),
            protected: None,
        }
    }

    /// The store the configuration lives in, when this host has one.
    pub(crate) fn config_store(&self) -> Option<&dyn canoa::config_store::ConfigStore> {
        self.store.as_ref().map(|kept| &*kept.0)
    }

    /// Keeps the configuration in `store` instead of the file `Files::new` was given.
    #[must_use]
    pub(crate) fn with_store(mut self, store: Arc<dyn canoa::config_store::ConfigStore>) -> Self {
        self.store = Some(Kept(store));
        self
    }

    /// Takes the person's list of protected paths, which `fs/write` never writes; `None` is the recommended list.
    #[must_use]
    pub(crate) fn with_protected(mut self, lines: Option<Vec<String>>) -> Self {
        self.protected = lines;
        self
    }

    /// Starts programs through this opener instead of the computer's own.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn with_opener(mut self, opener: workspace::Opener) -> Self {
        self.opener = opener;
        self
    }

    /// Uses these configured `[host] openers` (`None` seeds VS Code, the system's file manager and Reveal).
    #[must_use]
    pub(crate) fn with_openers(
        mut self,
        openers: Option<Vec<crate::config::OpenerConfig>>,
    ) -> Self {
        self.openers = RwLock::new(openers);
        self
    }

    /// Makes chats' scratch folders under `documents` rather than the system's Documents folder.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn with_documents(mut self, documents: PathBuf) -> Self {
        self.documents = Some(documents);
        self
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

    /// The places `fs/list` starts from and keeps to: for a caller holding `files`, and for the owner, the home and the whole
    /// computer; for any other key what [`Self::roots_of`] gives.
    fn browse_roots(&self, caller: &Caller) -> Vec<PathBuf> {
        if !caller.may(super::Grant::Files) {
            return self.roots_of(caller, true);
        }
        let mut roots: Vec<PathBuf> = Vec::new();
        for root in self.home.iter().cloned().chain(whole_machine()) {
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
        // The person running the host names any place their account reaches, a network share included; a key only a working root.
        let resolved = match caller {
            Caller::Owner => fs::canonicalize(&path).map(plain).ok(),
            Caller::Peer { .. } => {
                confined(&path.to_string_lossy(), &self.roots_of(caller, false)).ok()
            }
        };
        let Some(resolved) = resolved else {
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
        let free = caller.may(super::Grant::Files);
        let roots = self.browse_roots(caller);
        let hidden = params.get("hidden") == Some(&Value::Bool(true));
        let files = params.get("files") == Some(&Value::Bool(true));
        let Some(asked) = text_param(params, "path")? else {
            let places: Vec<Value> = roots
                .iter()
                .map(|root| json!({"name": visible_name(&shown_name(root)), "path": text_of(root), "dir": true}))
                .collect();
            return Ok(json!({"path": null, "entries": places, "more": false}));
        };
        let path = if free {
            // The grant is the only gate: what this account can list, anywhere, a share included.
            if !Path::new(asked).is_absolute() {
                return Err(bad("a path must be absolute: it starts at a drive or at /"));
            }
            fs::canonicalize(asked).map(plain).map_err(|_| {
                (
                    super::NOT_FOUND,
                    "there is no folder at that path, or this account cannot see it".to_owned(),
                )
            })?
        } else {
            confined(asked, &roots)?
        };
        let metadata = fs::metadata(&path).map_err(|_| not_found())?;
        if !metadata.is_dir() {
            return Err(bad("that is a file, not a folder: read it instead"));
        }
        let reader = fs::read_dir(&path)
            .map_err(|error| refused(&format!("this account cannot list that folder: {error}")))?;
        let offset = match params.get("offset") {
            None | Some(Value::Null) => 0,
            Some(value) => value
                .as_u64()
                .ok_or_else(|| bad("offset is a whole number of entries"))?,
        };
        let limit = match params.get("limit") {
            None | Some(Value::Null) => DEFAULT_LIST as u64,
            Some(value) => value
                .as_u64()
                .filter(|limit| *limit > 0)
                .ok_or_else(|| bad("limit is a whole number of entries above zero"))?,
        };
        let mut entries: Vec<(String, bool, PathBuf)> = Vec::new();
        for entry in reader.flatten() {
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if name.is_empty() || (!hidden && name.starts_with('.')) {
                continue;
            }
            let target = entry.path();
            // What a link points to counts: a key held to some places is not shown a link out of them.
            let Ok(resolved) = fs::canonicalize(&target).map(plain) else {
                continue;
            };
            if !free && !roots.iter().any(|root| resolved.starts_with(root)) {
                continue;
            }
            let Ok(kind) = fs::metadata(&resolved) else {
                continue;
            };
            if kind.is_dir() || (files && kind.is_file()) {
                entries.push((name, kind.is_dir(), resolved));
            }
        }
        entries.sort_by(|a, b| {
            b.1.cmp(&a.1)
                .then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase()))
                .then_with(|| a.0.cmp(&b.0))
        });
        // The page is cut from the sorted whole, so every entry is reachable by offset and none is lost to a cut made before sorting.
        let total = entries.len() as u64;
        let page: Vec<(String, bool, PathBuf)> = entries
            .into_iter()
            .skip(usize::try_from(offset).unwrap_or(usize::MAX))
            .take(usize::try_from(limit).unwrap_or(usize::MAX))
            .collect();
        let end = offset.saturating_add(page.len() as u64);
        let next = (end < total).then_some(end);
        let entries = page;
        let parent = path
            .parent()
            .map(Path::to_path_buf)
            .filter(|parent| free || roots.iter().any(|root| parent.starts_with(root)))
            .map(|parent| Value::String(text_of(&parent)))
            .unwrap_or(Value::Null);
        Ok(json!({
            "path": text_of(&path),
            "parent": parent,
            "entries": entries
                .iter()
                .map(|(name, dir, resolved)| json!({"name": visible_name(name), "path": text_of(resolved), "dir": dir}))
                .collect::<Vec<_>>(),
            "more": next.is_some(),
            "next": next,
        }))
    }

    /// `fs/read`: one page of a file, from `offset` (the start unless the request says), for a preview or to read it all. See
    /// [`DEFAULT_READ_BYTES`] and [`READ_PAGE_BYTES`] for the size of a page; `next` is where the following one starts.
    ///
    /// A key holding `files` reads what this computer's account can read, anywhere: the grant is the only gate. What is refused is
    /// what would hurt the person who asked, and the answer says why: a path or a link into a network share (Windows would send
    /// this computer's login to its server), and anything that is not a regular file (opening a pipe waits for ever).
    pub(crate) fn read(&self, params: &Value) -> Outcome {
        let asked =
            text_param(params, "path")?.ok_or_else(|| bad("a path is needed: the file to read"))?;
        let limit = match params.get("max_bytes") {
            None | Some(Value::Null) => DEFAULT_READ_BYTES,
            Some(value) => value
                .as_u64()
                .filter(|bytes| *bytes > 0)
                .ok_or_else(|| bad("max_bytes is a whole number of bytes above zero"))?
                .min(READ_PAGE_BYTES),
        };
        let offset = match params.get("offset") {
            None | Some(Value::Null) => 0,
            Some(value) => value
                .as_u64()
                .ok_or_else(|| bad("offset is a whole number of bytes"))?,
        };
        let path = Path::new(asked);
        if !path.is_absolute() {
            return Err(bad("a path must be absolute: it starts at a drive or at /"));
        }
        let missing = || {
            (
                super::NOT_FOUND,
                "there is no file at that path, or this account cannot see it".to_owned(),
            )
        };
        let path = fs::canonicalize(path).map(plain).map_err(|_| missing())?;
        let metadata = fs::metadata(&path).map_err(|_| missing())?;
        if metadata.is_dir() {
            return Err(bad("that is a folder, not a file: list it instead"));
        }
        if !metadata.is_file() {
            return Err(bad(
                "that is not a regular file (a pipe, a socket or a device), and reading one can wait for ever, so it is not opened",
            ));
        }
        let mut options = fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            // Without waiting, in case the name became a pipe since it was looked at.
            let flags = rustix::fs::OFlags::NONBLOCK.bits();
            options.custom_flags(
                i32::try_from(flags).map_err(|_| refused("that file could not be opened"))?,
            );
        }
        let mut file = options
            .open(&path)
            .map_err(|error| refused(&format!("this account cannot open that file: {error}")))?;
        let opened = file
            .metadata()
            .map_err(|error| refused(&format!("that file could not be inspected: {error}")))?;
        if !opened.is_file() {
            return Err(bad(
                "that is not a regular file any more, so it is not read",
            ));
        }
        let size = opened.len();
        // What a file is is told by its start, whichever page is read.
        let mut head = Vec::new();
        if offset > 0 {
            std::io::Read::take(&mut file, HEAD_BYTES)
                .read_to_end(&mut head)
                .map_err(|error| refused(&format!("that file could not be read: {error}")))?;
            std::io::Seek::seek(&mut file, std::io::SeekFrom::Start(offset))
                .map_err(|error| refused(&format!("that file could not be read: {error}")))?;
        }
        let mut bytes = Vec::new();
        std::io::Read::take(&mut file, limit)
            .read_to_end(&mut bytes)
            .map_err(|error| refused(&format!("that file could not be read: {error}")))?;
        let end = offset.saturating_add(bytes.len() as u64);
        let next = (end < size).then_some(end);
        let truncated = next.is_some();
        let (kind, mime) = if offset == 0 {
            classify(&bytes, truncated)
        } else {
            classify(&head, size > head.len() as u64)
        };
        let data = match kind {
            "text" | "image" => Some(bytes.as_slice()),
            _ => None,
        };
        Ok(json!({
            "path": text_of(&path),
            "name": visible_name(&shown_name(&path)),
            "size": size,
            "kind": kind,
            "mime": mime,
            "truncated": truncated,
            "offset": offset,
            "next": next,
            "data": data.map(|data| data_encoding::BASE64.encode(data)),
        }))
    }

    /// Whether `path` is on, in or under a place of the person's protected list, as `path` is and as the links in it lead.
    fn protects(&self, path: &Path) -> bool {
        let list = self.protected.as_ref().map_or_else(
            || Protected::new(crate::config::RECOMMENDED_PROTECTED.lines()),
            Protected::new,
        );
        list.protects(path, self.home.as_deref())
    }

    /// The files and folders this host keeps for itself, which `fs/write` leaves alone unless the key holds `files.write.host`:
    /// canoa.cfg and the host's state folder (the keys it trusts, the ledger, the accounts, the uploads).
    fn keeps(&self, path: &Path) -> bool {
        let resolved = |place: &Path| fs::canonicalize(place).map(plain).ok();
        let file = self.config.as_deref().and_then(resolved);
        let state = self
            .uploads
            .as_deref()
            .and_then(|dir| dir.parent().and_then(Path::parent))
            .and_then(resolved);
        file.is_some_and(|file| path == file) || state.is_some_and(|state| path.starts_with(state))
    }

    /// `fs/write` `{"path", "text"}`: writes `text` as the whole of the regular file at `path`, whole or not at all, with the mode
    /// the file has (a new file the mode any new file has here). The key holds `files` and `files.write`, and the place is one it
    /// browses. What is refused is a place on the person's protected list, and, without `files.write.host`, canoa.cfg and the host's
    /// own state folder; each refusal says which. The text arrives in one request, so `[host] max_message_bytes` bounds it.
    pub(crate) fn write(&self, caller: &Caller, params: &Value) -> Outcome {
        let asked = text_param(params, "path")?
            .ok_or_else(|| bad("a path is needed: the file to write"))?;
        let text = match params.get("text") {
            Some(Value::String(text)) => text.as_str(),
            _ => return Err(bad("text is needed: the UTF-8 contents to write")),
        };
        let path = Path::new(asked);
        if !path.is_absolute()
            || asked.chars().any(char::is_control)
            || asked
                .split(['/', '\\'])
                .any(|segment| segment == "." || segment == "..")
        {
            return Err(bad(
                "a path must be absolute, with no `.` or `..` and no control character",
            ));
        }
        let missing = || {
            (
                super::NOT_FOUND,
                "the parent folder does not exist, or this account cannot see it".to_owned(),
            )
        };
        let resolved = match fs::canonicalize(path).map(plain) {
            Ok(real) => {
                let meta = fs::metadata(&real).map_err(|error| {
                    refused(&format!("that file could not be inspected: {error}"))
                })?;
                if meta.is_dir() {
                    return Err(bad("that is a folder, not a file"));
                }
                if !meta.is_file() {
                    return Err(bad("that is not a regular file, so it is not written"));
                }
                real
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let name = path
                    .file_name()
                    .ok_or_else(|| bad("the path names no file"))?;
                // Windows reads a name ending in a dot or a space, or holding a colon, as another one.
                if cfg!(windows) {
                    let shown = name.to_string_lossy();
                    if shown.ends_with(['.', ' ']) || shown.contains(':') {
                        return Err(bad(
                            "on Windows a name ending in a dot or a space, or holding a colon, is read as another name",
                        ));
                    }
                }
                let parent = path.parent().ok_or_else(missing)?;
                let real_parent = fs::canonicalize(parent).map(plain).map_err(|_| missing())?;
                if !fs::metadata(&real_parent).is_ok_and(|meta| meta.is_dir()) {
                    return Err(bad("its parent is not a folder"));
                }
                real_parent.join(name)
            }
            Err(_) => {
                return Err((
                    super::NOT_FOUND,
                    "there is no file at that path, or this account cannot see it".to_owned(),
                ));
            }
        };
        if !self
            .browse_roots(caller)
            .iter()
            .any(|root| resolved.starts_with(root))
        {
            return Err(refused("that place is not one this key may write in"));
        }
        if self.protects(&resolved) {
            return Err(refused(
                "that place is on your protected list ([permissions] protected), so it is not written from here",
            ));
        }
        if self.keeps(&resolved) && !caller.may(super::Grant::FilesWriteHost) {
            return Err(refused(&format!(
                "that is {CONFIG_FILE} or this host's own state folder, which a key writes only with the files.write.host grant"
            )));
        }
        crate::replace::write_file(&resolved, text.as_bytes(), crate::replace::SHARED)
            .map_err(|error| refused(&format!("that file could not be written: {error}")))?;
        Ok(json!({
            "path": text_of(&resolved),
            "name": visible_name(&shown_name(&resolved)),
            "size": text.len(),
        }))
    }
}

fn refused(what: &str) -> (i32, String) {
    (REFUSED, what.to_owned())
}

fn not_found() -> (i32, String) {
    (
        super::NOT_FOUND,
        "there is no folder at that path that this key may list".to_owned(),
    )
}

/// What the first bytes of a file are: `text` (valid UTF-8, with no zero byte, allowing a character cut off by the end of what
/// was read), an `image` a page can draw (PNG, JPEG, GIF, WebP; never SVG, which can carry script), or `binary`; and the
/// image's media type.
fn classify(bytes: &[u8], truncated: bool) -> (&'static str, Option<&'static str>) {
    let image = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    };
    if let Some(mime) = image {
        return ("image", Some(mime));
    }
    if bytes.contains(&0) {
        return ("binary", None);
    }
    match core::str::from_utf8(bytes) {
        Ok(_) => ("text", None),
        Err(error) if truncated && error.error_len().is_none() => ("text", None),
        Err(_) => ("binary", None),
    }
}

/// `name` with each control, bidirectional or invisible character shown as `<U+XXXX>`: every name is listed, and none can pass for
/// another.
fn visible_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_control() || canoa::store::text::hides_text(c) {
            out.push_str(&format!("<U+{:04X}>", u32::from(c)));
        } else {
            out.push(c);
        }
    }
    out
}

/// `asked` resolved, when it is plain and under one of `roots`.
fn confined(asked: &str, roots: &[PathBuf]) -> Result<PathBuf, (i32, String)> {
    let path = Path::new(asked);
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
        return Err(bad(
            "a path must be absolute, with no `.` or `..` and no control character",
        ));
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

/// `path` without the `\\?\` prefix Windows' resolution adds: a drive's path as `C:\work`, a share's as `\\server\share`.
fn plain(path: PathBuf) -> PathBuf {
    crate::share::plain(&path).into_owned()
}
