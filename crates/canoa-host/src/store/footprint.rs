//! [`Footprint`]: what an install may change outside the folder it was given, written down before and after, and the
//! difference between the two named item by item. A change is covered when the plan declared one of the matching kind.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files and the registry are std's and the system's"
)]

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec::Vec;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use canoa::store::{Change, PATH, PROFILE_FILE, REGISTRY, STATE_DIR, USER_PATH};
use sha2::{Digest, Sha256};

/// The most of a profile file kept as text, to tell a `PATH` line from another; a longer one is only hashed.
const PROFILE_TEXT_LIMIT: u64 = 256 * 1024;

/// What to write down.
#[derive(Debug, Clone, Default)]
#[must_use]
pub struct Watch {
    home: Option<PathBuf>,
    path_dirs: Vec<PathBuf>,
    tree: Option<PathBuf>,
    skip: Vec<PathBuf>,
    registry: bool,
}

impl Watch {
    /// Watches the profile files and start-up places under `home`, `~/.local/bin`, and the top level of `home`.
    pub fn new(home: impl Into<PathBuf>) -> Self {
        Self {
            home: Some(home.into()),
            ..Self::default()
        }
    }

    /// Also lists each of these directories (the `PATH`'s).
    pub fn path_dirs(mut self, dirs: impl IntoIterator<Item = PathBuf>) -> Self {
        self.path_dirs = dirs.into_iter().filter(|d| d.is_absolute()).collect();
        self
    }

    /// Also writes down every file under `root`: a throwaway home in the suite, and the programs folder in the host, where
    /// another program's copy changing while one installs is not the install's doing.
    pub fn tree(mut self, root: impl Into<PathBuf>) -> Self {
        self.tree = Some(root.into());
        self
    }

    /// Leaves out everything under `dir`: the folder the install was given.
    pub fn skip(mut self, dir: impl Into<PathBuf>) -> Self {
        self.skip.push(dir.into());
        self
    }

    /// On Windows, also the user's registry keys an installer writes (`HKCU` `Environment`, `Run`, `RunOnce`, the policy
    /// `Run`, the command processor's `AutoRun`, the `Uninstall` and `Classes` names). Real, not redirected by a throwaway
    /// profile; nothing elsewhere.
    pub const fn registry(mut self, watch: bool) -> Self {
        self.registry = watch;
        self
    }
}

/// The absolute directories of a `PATH`, less the operating system's own, whose listing changes by itself while an install runs
/// and so says nothing about it: `%SystemRoot%` and below on Windows, and the Windows directories a Linux under WSL carries on
/// its `PATH` (`/mnt/c/Windows/...`).
#[must_use]
pub fn user_path_dirs(path: &std::ffi::OsStr) -> Vec<PathBuf> {
    let system =
        std::env::var_os("SystemRoot").map(|root| root.to_string_lossy().to_ascii_lowercase());
    std::env::split_paths(path)
        .filter(|dir| dir.is_absolute())
        .filter(|dir| {
            let text = dir.to_string_lossy().to_ascii_lowercase();
            let wsl_windows = text.starts_with("/mnt/") && text.contains("/windows");
            !wsl_windows
                && system
                    .as_ref()
                    .is_none_or(|root| !text.starts_with(root.as_str()))
        })
        .collect()
}

/// One thing that is different between two snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Difference {
    /// What changed, for people: a path or a registry value.
    pub what: String,
    /// The [`Change`] kind that would have declared it.
    pub kind: &'static str,
    /// For a profile file: whether every line that changed mentions `PATH`, so that a `USER_PATH` change covers it.
    pub path_lines_only: bool,
}

/// A registry value, raw: its type and bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawValue {
    /// The registry type number.
    pub ty: u32,
    /// The bytes.
    pub bytes: Vec<u8>,
}

/// The registry keys whose values are written down raw and put back: key path under `HKCU`.
#[cfg(windows)]
const RAW_KEYS: &[&str] = &[
    "Environment",
    r"Software\Microsoft\Windows\CurrentVersion\Run",
    r"Software\Microsoft\Windows\CurrentVersion\RunOnce",
    r"Software\Microsoft\Windows\CurrentVersion\Policies\Explorer\Run",
    r"Software\Microsoft\Command Processor",
];

/// What was written down.
#[derive(Debug, Clone, Default)]
pub struct Footprint {
    /// Key to a fingerprint.
    items: BTreeMap<String, String>,
    /// The lines of each profile file small enough to keep, by path.
    texts: BTreeMap<String, Vec<String>>,
    /// The values of the registry keys put back, raw, by key path.
    registry: BTreeMap<String, BTreeMap<String, RawValue>>,
    home: Option<PathBuf>,
}

impl Footprint {
    /// Writes down what `watch` names, now.
    pub fn take(watch: &Watch) -> Self {
        let mut footprint = Self {
            home: watch.home.clone(),
            ..Self::default()
        };
        if let Some(home) = &watch.home {
            for relative in PROFILES {
                footprint.file(&home.join(relative));
            }
            for relative in WATCHED_DIRS {
                footprint.listing(&home.join(relative), &watch.skip, false);
            }
            // The home's top level, and the places a login reads, are watched for what appears or goes (an installer making
            // `~/.tool`), not for what an agent running there rewrites.
            footprint.listing(home, &watch.skip, true);
            for relative in PRESENCE_DIRS {
                footprint.listing(&home.join(relative), &watch.skip, true);
            }
            #[cfg(windows)]
            if watch.registry {
                // Documents may be redirected (to OneDrive, say): the real folder is the registry's word.
                if let Some(documents) = real_documents(home) {
                    for relative in DOCUMENT_PROFILES {
                        footprint.file(&documents.join(relative));
                    }
                }
            }
        }
        for dir in &watch.path_dirs {
            footprint.listing(dir, &watch.skip, false);
        }
        if let Some(root) = &watch.tree {
            footprint.walk(root, &watch.skip);
        }
        if watch.registry {
            footprint.registry();
        }
        footprint
    }

    fn file(&mut self, path: &Path) {
        let Ok(meta) = fs::metadata(path) else { return };
        let Ok(bytes) = fs::read(path) else { return };
        let digest = Sha256::digest(&bytes);
        let key = format!("profile:{}", path.display());
        self.items.insert(key.clone(), format!("{digest:x}"));
        if meta.len() <= PROFILE_TEXT_LIMIT {
            let text = String::from_utf8_lossy(&bytes);
            self.texts
                .insert(key, text.lines().map(str::to_owned).collect());
        }
    }

    fn listing(&mut self, dir: &Path, skip: &[PathBuf], presence_only: bool) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if skip.iter().any(|s| path.starts_with(s)) {
                continue;
            }
            if let Ok(meta) = entry.metadata() {
                let seen = if presence_only {
                    "present".to_owned()
                } else {
                    fingerprint(&meta)
                };
                self.items.insert(format!("file:{}", path.display()), seen);
            }
        }
    }

    fn walk(&mut self, root: &Path, skip: &[PathBuf]) {
        let Ok(entries) = fs::read_dir(root) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if skip.iter().any(|s| path.starts_with(s)) {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            self.items
                .insert(format!("file:{}", path.display()), fingerprint(&meta));
            if meta.is_dir() && !meta.file_type().is_symlink() {
                self.walk(&path, skip);
            }
        }
    }

    #[cfg(windows)]
    fn registry(&mut self) {
        use windows_registry::{CURRENT_USER, Type};
        let code = |ty: Type| -> u32 {
            match ty {
                Type::U32 => 4,
                Type::U64 => 11,
                Type::String => 1,
                Type::ExpandString => 2,
                Type::MultiString => 7,
                Type::Bytes => 3,
                Type::Other(n) => n,
            }
        };
        for path in RAW_KEYS {
            let mut values = BTreeMap::new();
            if let Ok(key) = CURRENT_USER.open(path)
                && let Ok(found) = key.values()
            {
                for (name, value) in found {
                    let raw = RawValue {
                        ty: code(value.ty()),
                        bytes: value.as_ref().to_vec(),
                    };
                    let digest = Sha256::digest(&raw.bytes);
                    self.items.insert(
                        format!("reg:HKCU\\{path}\\{name}"),
                        format!("{}:{digest:x}", raw.ty),
                    );
                    values.insert(name, raw);
                }
            }
            self.registry.insert((*path).to_owned(), values);
        }
        for path in [
            r"Software\Microsoft\Windows\CurrentVersion\Uninstall",
            r"Software\Classes",
        ] {
            if let Ok(key) = CURRENT_USER.open(path)
                && let Ok(names) = key.keys()
            {
                for name in names {
                    self.items
                        .insert(format!("reg:HKCU\\{path}\\{name}"), String::from("present"));
                }
            }
        }
    }

    #[cfg(not(windows))]
    #[allow(
        clippy::unused_self,
        reason = "the Windows build reads the registry into self"
    )]
    fn registry(&mut self) {}

    /// Every item that differs in `after`, named.
    #[must_use]
    pub fn diff(&self, after: &Self) -> Vec<Difference> {
        let mut names: BTreeSet<&String> = self.items.keys().collect();
        names.extend(after.items.keys());
        let mut out = Vec::new();
        for key in names {
            if self.items.get(key) == after.items.get(key) {
                continue;
            }
            let mut difference = describe(key);
            if difference.kind == PROFILE_FILE {
                difference.path_lines_only =
                    path_lines_only(self.texts.get(key), after.texts.get(key));
            }
            out.push(difference);
        }
        out
    }

    /// Puts back the values of the registry keys this snapshot holds (the user's environment, the places a login runs from)
    /// where `after` differs, and removes the ones `after` added; answers how many it restored. Nothing on other platforms.
    #[must_use]
    pub fn restore_registry(&self, after: &Self) -> usize {
        restore_registry(&self.registry, &after.registry)
    }

    /// The home directory this was taken for.
    #[must_use]
    pub fn home(&self) -> Option<&Path> {
        self.home.as_deref()
    }
}

/// Whether every line that is in one text and not the other mentions `PATH`, and some is. A file that was not kept as text, or
/// that appeared or went whole, counts only when each of its lines does.
fn path_lines_only(before: Option<&Vec<String>>, after: Option<&Vec<String>>) -> bool {
    let nothing = Vec::new();
    let (before, after) = (before.unwrap_or(&nothing), after.unwrap_or(&nothing));
    let mut counts: BTreeMap<&str, i64> = BTreeMap::new();
    for line in before {
        *counts.entry(line.as_str()).or_default() -= 1;
    }
    for line in after {
        *counts.entry(line.as_str()).or_default() += 1;
    }
    let changed: Vec<&str> = counts
        .into_iter()
        .filter(|(_, n)| *n != 0)
        .map(|(line, _)| line)
        .collect();
    !changed.is_empty()
        && changed
            .iter()
            .all(|line| line.to_ascii_lowercase().contains("path"))
}

#[cfg(windows)]
fn real_documents(home: &Path) -> Option<PathBuf> {
    use windows_registry::CURRENT_USER;
    let key = CURRENT_USER
        .open(r"Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders")
        .ok()?;
    let personal = key.get_string("Personal").ok()?;
    let expanded = personal.replace("%USERPROFILE%", &home.to_string_lossy());
    let path = PathBuf::from(expanded);
    path.is_absolute().then_some(path)
}

#[cfg(windows)]
fn restore_registry(
    before: &BTreeMap<String, BTreeMap<String, RawValue>>,
    after: &BTreeMap<String, BTreeMap<String, RawValue>>,
) -> usize {
    use windows_registry::{CURRENT_USER, Type};
    let mut restored = 0;
    for (path, old_values) in before {
        let Ok(key) = CURRENT_USER.options().read().write().open(path) else {
            continue;
        };
        let empty = BTreeMap::new();
        let new_values = after.get(path).unwrap_or(&empty);
        for (name, old) in old_values {
            if new_values.get(name) != Some(old) {
                let ty = match old.ty {
                    4 => Type::U32,
                    11 => Type::U64,
                    1 => Type::String,
                    2 => Type::ExpandString,
                    7 => Type::MultiString,
                    3 => Type::Bytes,
                    n => Type::Other(n),
                };
                if key.set_bytes(name, ty, &old.bytes).is_ok() {
                    restored += 1;
                }
            }
        }
        for name in new_values.keys() {
            if !old_values.contains_key(name) && key.remove_value(name).is_ok() {
                restored += 1;
            }
        }
    }
    restored
}

#[cfg(not(windows))]
fn restore_registry(
    _before: &BTreeMap<String, BTreeMap<String, RawValue>>,
    _after: &BTreeMap<String, BTreeMap<String, RawValue>>,
) -> usize {
    0
}

/// Shell and PowerShell profile files, relative to the home, and the files a login or a tool reads that can run or redirect
/// code: hashed, and kept as lines.
const PROFILES: &[&str] = &[
    ".bashrc",
    ".bash_profile",
    ".bash_login",
    ".bash_logout",
    ".profile",
    ".zshrc",
    ".zshenv",
    ".zprofile",
    ".zlogin",
    ".zlogout",
    ".config/fish/config.fish",
    ".gitconfig",
    ".npmrc",
    ".ssh/authorized_keys",
    ".ssh/config",
    "Documents/PowerShell/Microsoft.PowerShell_profile.ps1",
    "Documents/PowerShell/profile.ps1",
    "Documents/WindowsPowerShell/Microsoft.PowerShell_profile.ps1",
    "Documents/WindowsPowerShell/profile.ps1",
];

/// PowerShell profiles under the real Documents folder (Windows).
#[cfg(windows)]
const DOCUMENT_PROFILES: &[&str] = &[
    "PowerShell/Microsoft.PowerShell_profile.ps1",
    "PowerShell/profile.ps1",
    "WindowsPowerShell/Microsoft.PowerShell_profile.ps1",
    "WindowsPowerShell/profile.ps1",
];

/// Directories under the home whose listing, with each entry's length and time, is written down.
const WATCHED_DIRS: &[&str] = &[
    ".local/bin",
    ".config/systemd/user",
    ".config/autostart",
    ".config/fish/conf.d",
    "Library/LaunchAgents",
    "AppData/Roaming/Microsoft/Windows/Start Menu/Programs/Startup",
];

/// Directories under the home whose entries are written down only as present.
const PRESENCE_DIRS: &[&str] = &[
    ".ssh",
    "AppData/Roaming/Microsoft/Windows/Start Menu/Programs",
];

/// A file's length and time; a directory is only that it is one, since its own time follows its entries' on some file systems
/// only when its handle closes, and what it holds is listed entry by entry.
fn fingerprint(meta: &fs::Metadata) -> String {
    if meta.is_dir() {
        return "dir".to_owned();
    }
    let modified = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    format!("{}:{modified}", meta.len())
}

fn describe(key: &str) -> Difference {
    if let Some(rest) = key.strip_prefix("reg:") {
        let kind = if rest.eq_ignore_ascii_case("HKCU\\Environment\\Path") {
            USER_PATH
        } else {
            REGISTRY
        };
        return Difference {
            what: rest.to_owned(),
            kind,
            path_lines_only: false,
        };
    }
    if let Some(rest) = key.strip_prefix("profile:") {
        return Difference {
            what: rest.to_owned(),
            kind: PROFILE_FILE,
            path_lines_only: false,
        };
    }
    Difference {
        what: key.strip_prefix("file:").unwrap_or(key).to_owned(),
        kind: PATH,
        path_lines_only: false,
    }
}

/// The differences that no declared change covers.
///
/// - `USER_PATH` covers `HKCU\Environment\Path` and a profile file whose changed lines all mention `PATH`.
/// - `PROFILE_FILE` covers its own path.
/// - `PATH` and `STATE_DIR` cover their path and what is under it.
/// - `REGISTRY` covers its key and what is under it.
#[must_use]
pub fn uncovered(differences: &[Difference], declared: &[Change]) -> Vec<Difference> {
    differences
        .iter()
        .filter(|difference| !declared.iter().any(|change| covers(change, difference)))
        .cloned()
        .collect()
}

fn same(a: &str, b: &str) -> bool {
    if cfg!(windows) {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

/// The parts of a path or a key, whatever the direction of its separators.
fn parts(text: &str) -> Vec<&str> {
    text.split(['/', '\\'])
        .filter(|p| !p.is_empty() && *p != ".")
        .collect()
}

/// Whether `path` is `prefix` or lies below it, part by part.
fn under(path: &str, prefix: &str) -> bool {
    let (path, prefix) = (parts(path), parts(prefix));
    prefix.len() <= path.len() && prefix.iter().zip(&path).all(|(p, q)| same(p, q))
}

/// Whether `path` lies inside `root`, part by part, without a part that climbs out.
#[must_use]
pub fn is_within(path: &Path, root: &Path) -> bool {
    let climbs = |p: &Path| p.components().any(|c| c == Component::ParentDir);
    !climbs(path) && !climbs(root) && path.starts_with(root)
}

fn covers(change: &Change, difference: &Difference) -> bool {
    match change.kind.as_str() {
        USER_PATH => {
            difference.kind == USER_PATH
                || (difference.kind == PROFILE_FILE && difference.path_lines_only)
        }
        PROFILE_FILE => difference.kind == PROFILE_FILE && same(&difference.what, &change.target),
        PATH | STATE_DIR => {
            matches!(difference.kind, PATH | PROFILE_FILE)
                && under(&difference.what, &change.target)
        }
        REGISTRY => {
            matches!(difference.kind, REGISTRY | USER_PATH)
                && under(&difference.what, &change.target)
        }
        _ => false,
    }
}

/// Whether `path` is `prefix` or lies below it, part by part (not byte by byte), by the file system's idea of case.
#[must_use]
pub fn path_under(path: &str, prefix: &str) -> bool {
    under(path, prefix)
}
