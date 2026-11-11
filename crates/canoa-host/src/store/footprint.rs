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
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use canoa::store::{Change, PATH, PROFILE_FILE, REGISTRY, STATE_DIR, USER_PATH};
use sha2::{Digest, Sha256};

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
    /// Watches the profile files under `home`, `~/.local/bin`, and the top level of `home`.
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

    /// Also writes down every file under `root` (a throwaway home, in the suite: too slow for a real one).
    pub fn tree(mut self, root: impl Into<PathBuf>) -> Self {
        self.tree = Some(root.into());
        self
    }

    /// Leaves out everything under `dir`: the folder the install was given.
    pub fn skip(mut self, dir: impl Into<PathBuf>) -> Self {
        self.skip.push(dir.into());
        self
    }

    /// On Windows, also the user's registry keys an installer writes (`HKCU` `Environment`, `Run`, `RunOnce`, the
    /// `Uninstall` names). Real, not redirected by a throwaway profile; nothing elsewhere.
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
}

/// A registry value, raw: its type and bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawValue {
    /// The registry type number.
    pub ty: u32,
    /// The bytes.
    pub bytes: Vec<u8>,
}

/// What was written down.
#[derive(Debug, Clone, Default)]
pub struct Footprint {
    /// Key to a fingerprint.
    items: BTreeMap<String, String>,
    /// The values of `HKCU\Environment`, raw, to put back.
    environment: BTreeMap<String, RawValue>,
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
                let path = home.join(relative);
                footprint.file(&path);
            }
            for relative in WATCHED_DIRS {
                footprint.listing(&home.join(relative), &watch.skip, false);
            }
            // The home's top level: what appears or goes (an installer making `~/.tool`), not what an agent running there rewrites.
            footprint.listing(home, &watch.skip, true);
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
        if let Ok(bytes) = fs::read(path) {
            let digest = Sha256::digest(&bytes);
            self.items
                .insert(format!("profile:{}", path.display()), format!("{digest:x}"));
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
            if meta.is_dir() {
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
        if let Ok(key) = CURRENT_USER.open("Environment")
            && let Ok(values) = key.values()
        {
            for (name, value) in values {
                self.environment.insert(
                    name,
                    RawValue {
                        ty: code(value.ty()),
                        bytes: value.as_ref().to_vec(),
                    },
                );
            }
        }
        for path in [
            r"Software\Microsoft\Windows\CurrentVersion\Run",
            r"Software\Microsoft\Windows\CurrentVersion\RunOnce",
        ] {
            if let Ok(key) = CURRENT_USER.open(path)
                && let Ok(values) = key.values()
            {
                for (name, value) in values {
                    let digest = Sha256::digest(value.as_ref());
                    self.items
                        .insert(format!("reg:HKCU\\{path}\\{name}"), format!("{digest:x}"));
                }
            }
        }
        if let Ok(key) = CURRENT_USER.open(r"Software\Microsoft\Windows\CurrentVersion\Uninstall")
            && let Ok(names) = key.keys()
        {
            for name in names {
                self.items.insert(
                    format!(
                        "reg:HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{name}"
                    ),
                    String::from("present"),
                );
            }
        }
        for (name, value) in &self.environment {
            let digest = Sha256::digest(&value.bytes);
            self.items.insert(
                format!("reg:HKCU\\Environment\\{name}"),
                format!("{}:{digest:x}", value.ty),
            );
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
            out.push(describe(key));
        }
        out
    }

    /// Puts back the `HKCU\Environment` values this snapshot holds where `after` differs, and removes the ones `after`
    /// added; answers how many it restored. Nothing on other platforms.
    #[must_use]
    pub fn restore_environment(&self, after: &Self) -> usize {
        restore_environment(&self.environment, &after.environment)
    }

    /// The home directory this was taken for.
    #[must_use]
    pub fn home(&self) -> Option<&Path> {
        self.home.as_deref()
    }
}

#[cfg(windows)]
fn restore_environment(
    before: &BTreeMap<String, RawValue>,
    after: &BTreeMap<String, RawValue>,
) -> usize {
    use windows_registry::{CURRENT_USER, Type};
    let Ok(key) = CURRENT_USER.options().read().write().open("Environment") else {
        return 0;
    };
    let mut restored = 0;
    for (name, old) in before {
        if after.get(name) != Some(old) {
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
    for name in after.keys() {
        if !before.contains_key(name) && key.remove_value(name).is_ok() {
            restored += 1;
        }
    }
    restored
}

#[cfg(not(windows))]
fn restore_environment(
    _before: &BTreeMap<String, RawValue>,
    _after: &BTreeMap<String, RawValue>,
) -> usize {
    0
}

/// Shell and PowerShell profile files, relative to the home.
const PROFILES: &[&str] = &[
    ".bashrc",
    ".bash_profile",
    ".profile",
    ".zshrc",
    ".zprofile",
    ".config/fish/config.fish",
    "Documents/PowerShell/Microsoft.PowerShell_profile.ps1",
    "Documents/PowerShell/profile.ps1",
    "Documents/WindowsPowerShell/Microsoft.PowerShell_profile.ps1",
    "Documents/WindowsPowerShell/profile.ps1",
];

/// Directories under the home whose listing is written down.
const WATCHED_DIRS: &[&str] = &[
    ".local/bin",
    ".config/systemd/user",
    "Library/LaunchAgents",
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
        };
    }
    if let Some(rest) = key.strip_prefix("profile:") {
        return Difference {
            what: rest.to_owned(),
            kind: PROFILE_FILE,
        };
    }
    Difference {
        what: key.strip_prefix("file:").unwrap_or(key).to_owned(),
        kind: PATH,
    }
}

/// The differences that no declared change covers.
///
/// - `USER_PATH` covers `HKCU\Environment\Path` and the profile files (their lines that change `PATH`).
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

fn under(path: &str, prefix: &str) -> bool {
    let normal = |s: &str| s.replace('\\', "/");
    let (path, prefix) = (normal(path), normal(prefix));
    let prefix = prefix.trim_end_matches('/');
    same(&path, prefix)
        || (path.len() > prefix.len()
            && same(&path[..prefix.len()], prefix)
            && path.as_bytes()[prefix.len()] == b'/')
}

fn covers(change: &Change, difference: &Difference) -> bool {
    match change.kind.as_str() {
        USER_PATH => difference.kind == USER_PATH || difference.kind == PROFILE_FILE,
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
