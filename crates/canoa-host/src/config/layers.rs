//! Where canoa.cfg is read from, and how its layers merge.
//!
//! Lowest first: the installed copy beside the program, the system's file, the person's own. A `--config` or
//! `CANOA_CONFIG` value is a list of files, `+` first to put them on top of the other layers and otherwise instead of them.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use std::path::{Path, PathBuf};

use anycfg::{Error, Merge, MergeValues};

/// The file's name below the user's and the system's configuration directories.
pub const NAME: &str = "Canoa/canoa.cfg";
/// The file's name beside the program.
pub const INSTALLED_NAME: &str = "canoa.cfg";
/// The variable that says which folder is "beside the program", when it is not the executable's own: the portable launcher
/// sets it to its own folder, since the app it starts runs from an unpacked copy.
pub const INSTALLED_DIR_VARIABLE: &str = "CANOA_INSTALLED_DIR";

/// The files canoa.cfg is read from, lowest layer first. A layer that does not exist is skipped.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Layers {
    /// The files, lowest first.
    pub files: Vec<PathBuf>,
    /// The file the app and `canoa accounts` write: the person's own, or the last one listed.
    pub own: PathBuf,
    /// How many of the last `files` were listed by `--config` or `CANOA_CONFIG`, and so must exist.
    listed: usize,
}

impl Layers {
    /// One file and no other layer.
    #[must_use]
    pub fn only(file: impl Into<PathBuf>) -> Self {
        let own = file.into();
        Self {
            files: alloc::vec![own.clone()],
            own,
            listed: 0,
        }
    }

    /// The layers for a `--config` or `CANOA_CONFIG` value, as the environment names the other layers.
    ///
    /// `listed` is the whole value; `None` or an empty one is the installed, system and user layers. With `must_exist`
    /// false the last listed file may be missing: it is what a command that creates canoa.cfg writes.
    ///
    /// # Errors
    ///
    /// A message for an empty entry in the list, a listed file that is missing, or no user directory to find.
    pub fn resolve(listed: Option<&str>, must_exist: bool) -> Result<Self, String> {
        let program = installed_file();
        let system = anycfg::system_config_path(Some(Path::new(NAME)));
        let user = || {
            anycfg::user_config_path(Some(Path::new(NAME)))
                .map_err(|_| "no canoa.cfg: give --config or CANOA_CONFIG".to_owned())
        };
        let listed = listed.filter(|value| !value.is_empty());
        let (merging, entries) = match listed {
            None => (true, None),
            Some(value) => match value.strip_prefix('+') {
                Some(rest) => (true, Some(entries_of(rest)?)),
                None => (false, Some(entries_of(value)?)),
            },
        };
        let mut files = Vec::new();
        if merging {
            files.extend(program);
            files.push(system);
            if entries.is_none() {
                files.push(user()?);
            } else if let Ok(user) = user() {
                files.push(user);
            }
        }
        let own = match &entries {
            Some(list) => list.last().cloned().unwrap_or_default(),
            None => files.last().cloned().unwrap_or_default(),
        };
        let mut listed_count = 0;
        if let Some(list) = entries {
            for (at, file) in list.iter().enumerate() {
                let last = at + 1 == list.len();
                if !file.is_file() && (must_exist || !last) {
                    return Err(format!(
                        "canoa.cfg entry {} ({}) is not a file",
                        at + 1,
                        file.display()
                    ));
                }
            }
            listed_count = list.len();
            files.extend(list);
        }
        // A file named twice is read once, at its highest place.
        let mut kept: Vec<PathBuf> = Vec::new();
        for file in files.into_iter().rev() {
            if !kept.contains(&file) {
                kept.push(file);
            }
        }
        kept.reverse();
        Ok(Self {
            files: kept,
            own,
            listed: listed_count,
        })
    }

    /// The files that exist, lowest first.
    #[must_use]
    pub fn existing(&self) -> Vec<&Path> {
        self.files
            .iter()
            .map(PathBuf::as_path)
            .filter(|file| file.is_file())
            .collect()
    }

    /// How many of the last files were listed.
    #[must_use]
    pub fn listed(&self) -> usize {
        self.listed
    }
}

/// The installed copy: beside the program, or in the folder the launcher names.
fn installed_file() -> Option<PathBuf> {
    let named = std::env::var_os(INSTALLED_DIR_VARIABLE)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let dir = named.or_else(|| {
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf))
    })?;
    Some(dir.join(INSTALLED_NAME))
}

/// The entries of a path list: split at `;` on Windows and `:` elsewhere, with no quoting; an empty entry is refused.
fn entries_of(list: &str) -> Result<Vec<PathBuf>, String> {
    let separator = if cfg!(windows) { ';' } else { ':' };
    let mut out = Vec::new();
    for (at, entry) in list.split(separator).enumerate() {
        if entry.is_empty() {
            return Err(format!("canoa.cfg entry {} is empty", at + 1));
        }
        out.push(PathBuf::from(entry));
    }
    Ok(out)
}

/// The paths below which a table is one entry, named by its key, that a higher layer replaces whole. `*` is any key.
pub const ENTRIES: &[&[&str]] = &[
    &["host", "agents", "*"],
    &["host", "providers", "*"],
    &["host", "tools", "*"],
    &["host", "profiles", "*"],
    &["host", "accounts", "*"],
    &["agents", "*"],
    &["profile", "mcp", "*"],
    &["profile", "skills", "*"],
    &["profile", "env", "*"],
    &["overlays", "*"],
    &["catalogs", "*"],
];

fn is_entry(path: &[&str]) -> bool {
    ENTRIES.iter().any(|pattern| {
        pattern.len() == path.len()
            && pattern
                .iter()
                .zip(path)
                .all(|(want, have)| *want == "*" || want == have)
    })
}

/// Merges `next`, a higher layer, into `previous`: tables key by key, a key above replacing the same key below, and the
/// tables of [`ENTRIES`] and every array whole.
pub fn merge_into(previous: &mut toml::Value, next: toml::Value) {
    fn tables(path: &mut Vec<String>, previous: &mut toml::Table, next: toml::Table) {
        for (key, value) in next {
            path.push(key.clone());
            let names: Vec<&str> = path.iter().map(String::as_str).collect();
            let into_table = !is_entry(&names);
            match (previous.get_mut(&key), value) {
                (Some(toml::Value::Table(below)), toml::Value::Table(above)) if into_table => {
                    tables(path, below, above);
                }
                (_, value) => {
                    previous.insert(key, value);
                }
            }
            path.pop();
        }
    }
    match (previous, next) {
        (toml::Value::Table(below), toml::Value::Table(above)) => {
            tables(&mut Vec::new(), below, above);
        }
        (previous, next) => *previous = next,
    }
}

/// The merge anycfg's loader runs over the layers.
#[derive(Clone, Copy, Debug)]
pub(super) struct Merging;

impl MergeValues<toml::Value> for Merging {
    fn merge(
        &self,
        previous: &mut toml::Value,
        next: toml::Value,
        _strategy: Merge,
        _max_depth: usize,
    ) -> Result<(), Error> {
        merge_into(previous, next);
        Ok(())
    }
}

impl core::fmt::Display for Layers {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let names: Vec<String> = self
            .files
            .iter()
            .map(|file| file.display().to_string())
            .collect();
        f.write_str(&names.join(", "))
    }
}

impl Layers {
    /// These files and no others, lowest first; the last is the person's own.
    #[must_use]
    pub fn from_files(files: Vec<PathBuf>) -> Self {
        let own = files.last().cloned().unwrap_or_default();
        Self {
            files,
            own,
            listed: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn examples() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/config")
    }

    fn fixture(name: &str) -> PathBuf {
        examples().join("layers").join(name)
    }

    fn list(files: &[&Path]) -> String {
        let separator = if cfg!(windows) { ";" } else { ":" };
        files
            .iter()
            .map(|file| file.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(separator)
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("canoa-layers-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("directory");
        dir
    }

    #[cfg(all(feature = "yaml", feature = "json"))]
    #[test]
    fn three_layers_in_three_formats_merge_key_by_key_and_entries_whole() {
        let mut merged = toml::Value::Table(toml::Table::new());
        for name in ["installed.cfg", "system.cfg", "user.cfg"] {
            let layer = crate::format::formats()
                .load(fixture(name), anycfg::Options::new())
                .expect(name);
            merge_into(&mut merged, layer);
        }
        let shown = serde_json::to_value(&merged).expect("json");
        let expected: serde_json::Value =
            serde_json::from_slice(&std::fs::read(fixture("merged.json")).expect("merged.json"))
                .expect("json");
        assert_eq!(shown, expected);
    }

    #[cfg(all(feature = "yaml", feature = "json"))]
    #[test]
    fn the_host_reads_the_layers_as_one_configuration() {
        let layers = Layers::from_files(
            ["installed.cfg", "system.cfg", "user.cfg"]
                .iter()
                .map(|name| fixture(name))
                .collect(),
        );
        let config = Config::load_layers(&layers).expect("loads");
        let names: Vec<&str> = config
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        assert_eq!(names, ["alpha", "beta"]);
        assert_eq!(config.limit_phrases, Some(vec!["rate limit".to_owned()]));
        // Entry `alpha` came whole from the system's file: the installed `listen` is not carried over.
        let alpha = config.entry("alpha").expect("alpha");
        assert_eq!(
            alpha.listen,
            crate::config::Listen::Address("127.0.0.1:0".parse().expect("address"))
        );
    }

    #[cfg(all(feature = "yaml", feature = "json"))]
    #[test]
    fn a_malformed_layer_is_named_with_its_key_and_no_value() {
        let dir = scratch("malformed");
        let good = dir.join("a.cfg");
        let bad = dir.join("b.cfg");
        std::fs::write(&good, "[host.agents.a]\nprogram = \"x\"\n").expect("write");
        std::fs::write(
            &bad,
            "#!yaml\nhost:\n  agents:\n    a:\n      max_peers: SECRETSECRET\n",
        )
        .expect("write");
        let shown = Config::load_layers(&Layers::from_files(vec![good, bad]))
            .expect_err("refused")
            .to_string();
        assert!(
            shown.contains("b.cfg") && shown.contains("max_peers"),
            "{shown}"
        );
        assert!(!shown.contains("SECRET"), "{shown}");
        std::fs::remove_dir_all(dir).expect("cleanup");
    }

    #[test]
    fn a_missing_layer_is_skipped() {
        let layers = Layers::from_files(vec![
            fixture("nothing-here.cfg"),
            fixture("installed.cfg"),
            fixture("nothing-either.cfg"),
        ]);
        assert_eq!(
            Config::load_layers(&layers).expect("loads").entries.len(),
            1
        );
    }

    #[test]
    fn a_value_without_plus_is_the_listed_files_only() {
        let dir = scratch("one");
        let (a, b) = (dir.join("a.cfg"), dir.join("b.cfg"));
        std::fs::write(&a, "").expect("write");
        std::fs::write(&b, "").expect("write");
        let one = Layers::resolve(Some(&list(&[&a])), true).expect("one");
        assert_eq!(one.files, core::slice::from_ref(&a));
        assert_eq!(one.own, a);
        let two = Layers::resolve(Some(&list(&[&a, &b])), true).expect("two");
        assert_eq!(two.files, [a, b.clone()]);
        assert_eq!(two.own, b);
        std::fs::remove_dir_all(dir).expect("cleanup");
    }

    #[test]
    fn a_leading_plus_puts_the_listed_files_over_the_other_layers() {
        let dir = scratch("plus");
        let (a, b) = (dir.join("a.cfg"), dir.join("b.cfg"));
        std::fs::write(&a, "").expect("write");
        std::fs::write(&b, "").expect("write");
        let value = format!("+{}", list(&[&a, &b]));
        let layers = Layers::resolve(Some(&value), true).expect("plus");
        let system = anycfg::system_config_path(Some(Path::new(NAME)));
        assert!(layers.files.contains(&system), "{layers}");
        assert_eq!(layers.files[layers.files.len() - 2..], [a, b.clone()]);
        assert_eq!(layers.own, b);
        std::fs::remove_dir_all(dir).expect("cleanup");
    }

    #[test]
    fn an_empty_entry_and_a_missing_file_are_refused_naming_the_entry() {
        let dir = scratch("refused");
        let a = dir.join("a.cfg");
        std::fs::write(&a, "").expect("write");
        let missing = dir.join("missing.cfg");
        let separator = if cfg!(windows) { ";" } else { ":" };
        let empty = format!("{}{separator}", a.display());
        let shown = Layers::resolve(Some(&empty), true).expect_err("empty");
        assert!(
            shown.contains("entry 2") && shown.contains("empty"),
            "{shown}"
        );
        assert!(Layers::resolve(Some("+"), true).is_err());
        let shown = Layers::resolve(Some(&list(&[&a, &missing])), true).expect_err("missing");
        assert!(
            shown.contains("entry 2") && shown.contains("missing.cfg"),
            "{shown}"
        );
        // A command that makes the file may name a last one that is not there yet, and no other.
        assert!(Layers::resolve(Some(&list(&[&a, &missing])), false).is_ok());
        assert!(Layers::resolve(Some(&list(&[&missing, &a])), false).is_err());
        std::fs::remove_dir_all(dir).expect("cleanup");
    }
}
