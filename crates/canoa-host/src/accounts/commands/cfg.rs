//! Format-preserving `canoa.cfg` editing helpers for accounts, pools and offers.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files and locks are std's"
)]

use std::fs;
use std::path::Path;
use std::sync::{Mutex, PoisonError};

use canoa::config_store::{Changes, ConfigStore, Key, Op};

use crate::cli::Failed;
use crate::config::{AccountConfig, Config};

pub(crate) fn read_cfg(path: &Path) -> Result<String, Failed> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(Failed::run(format!("{}: {error}", path.display()))),
    }
}

/// Whether `text` is a canoa.cfg this host reads, as the file at `path` would be.
pub(crate) fn check_cfg(path: &Path, text: &str) -> Result<(), Failed> {
    Config::from_str_named(
        text,
        file_name(path),
        path.parent().unwrap_or_else(|| Path::new("")),
    )
    .map(|_| ())
    .map_err(|error| Failed::run(format!("the edited canoa.cfg would not be read: {error}")))
}

/// Writes `text` as the whole of the file at `path`, and only if it is a canoa.cfg this host reads.
#[cfg(test)]
pub(crate) fn write_cfg(path: &Path, text: &str) -> Result<(), Failed> {
    check_cfg(path, text)?;
    crate::replace::write_file(path, text.as_bytes(), 0o600)
        .map_err(|error| Failed::run(format!("{}: {error}", path.display())))
}

pub(crate) fn file_name(path: &Path) -> &str {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("canoa.cfg")
}

/// The registered name of the format `text` is in, by its file's name or first line.
pub(crate) fn format_of(path: &Path, text: &str) -> Result<String, Failed> {
    let options = anycfg::Options::new().name(file_name(path));
    crate::format::formats()
        .detect(text.as_bytes(), options)
        .map(|detected| detected.format.to_owned())
        .map_err(|error| Failed::run(format!("{}: {error}", path.display())))
}

/// A JSON value as TOML's model, which is what canoa.cfg is read as; `None` for one it cannot hold.
pub(crate) fn toml_value(value: &serde_json::Value) -> Option<toml::Value> {
    Some(match value {
        serde_json::Value::Null => return None,
        serde_json::Value::Bool(b) => toml::Value::Boolean(*b),
        serde_json::Value::Number(n) => n
            .as_i64()
            .map(toml::Value::Integer)
            .or_else(|| n.as_f64().map(toml::Value::Float))?,
        serde_json::Value::String(s) => toml::Value::String(s.clone()),
        serde_json::Value::Array(items) => {
            toml::Value::Array(items.iter().filter_map(toml_value).collect())
        }
        serde_json::Value::Object(map) => toml::Value::Table(
            map.iter()
                .filter_map(|(key, value)| Some((key.clone(), toml_value(value)?)))
                .collect(),
        ),
    })
}

/// A file in YAML or JSON, edited as a document and written back in its own format: its comments and layout are not kept.
pub(crate) fn edit_document(
    path: &Path,
    text: &str,
    format: &str,
    edit: impl FnOnce(&mut toml::Table) -> Result<(), Failed>,
) -> Result<String, Failed> {
    let formats = crate::format::formats();
    let name = file_name(path);
    let refused = |error: anycfg::Error| Failed::run(format!("{}: {error}", path.display()));
    let mut value = formats
        .from_slice(text.as_bytes(), anycfg::Options::new().name(name))
        .map_err(refused)?;
    let toml::Value::Table(table) = &mut value else {
        return Err(Failed::run(format!("{}: not a table", path.display())));
    };
    edit(table)?;
    let extension = Path::new(name).extension().and_then(|ext| ext.to_str());
    let bytes = if extension.is_some_and(|ext| ext.eq_ignore_ascii_case("cfg")) {
        formats.to_auto_vec(&value, format, "cfg")
    } else {
        formats.to_vec(&value, format)
    }
    .map_err(refused)?;
    String::from_utf8(bytes).map_err(|error| Failed::run(error.to_string()))
}

/// Adds the account `name` as `[host.accounts.NAME]`.
pub(super) fn append_account(
    store: &dyn ConfigStore,
    name: &str,
    account: &AccountConfig,
) -> Result<(), Failed> {
    let mut block = toml::Table::new();
    block.insert("kind".into(), toml::Value::String(account.kind.clone()));
    block.insert("id".into(), toml::Value::String(account.id.clone()));
    if let Some(settings) = &account.settings
        && let Some(value) = toml_value(&serde_json::Value::Object(settings.as_map().clone()))
    {
        block.insert("settings".into(), value);
    }
    append_table(store, &["host", "accounts"], name, &block)
}

/// One change to canoa.cfg, made in whatever format the file is in.
pub(crate) enum Change<'a> {
    /// The table at the path holds exactly this one.
    Replace(&'a [&'a str], &'a toml::Table),
    /// These keys are set in the table at the path, which is made when there is none; its other keys stay.
    Merge(&'a [&'a str], &'a toml::Table),
    /// The table at the path is gone.
    RemoveTable(&'a [&'a str]),
    /// The key is gone from the table at the path.
    RemoveKey(&'a [&'a str], &'a str),
}

/// `text` with `changes` made, in its own format, and whether every removal found what it removes. A file in TOML is edited in
/// place: its comments, its order and the way its person spelled each name stay. A file in YAML or JSON, which has no layout of its
/// own to keep, is read as a document and written whole.
pub(crate) fn edit_cfg(
    path: &Path,
    text: &str,
    changes: &[Change<'_>],
) -> Result<(String, bool), Failed> {
    let format = format_of(path, text)?;
    let mut found = true;
    if format != "toml" {
        let edited = edit_document(path, text, &format, |table| {
            for change in changes {
                found &= change_tree(table, change)?;
            }
            Ok(())
        })?;
        return Ok((edited, found));
    }
    let refused = |error: String| Failed::run(format!("{}: {error}", path.display()));
    let mut document = crate::cfg_edit::parse(text).map_err(refused)?;
    for change in changes {
        found &= match change {
            Change::Replace(at, table) => {
                crate::cfg_edit::replace(&mut document, at, table).map(|()| true)
            }
            Change::Merge(at, table) => {
                crate::cfg_edit::merge(&mut document, at, table).map(|()| true)
            }
            Change::RemoveTable(at) => crate::cfg_edit::remove(&mut document, at),
            Change::RemoveKey(at, key) => crate::cfg_edit::remove_key(&mut document, at, key),
        }
        .map_err(refused)?;
    }
    Ok((document.to_string(), found))
}

/// The table at `path` in a document, made when `make` is set.
fn tree_at<'a>(
    root: &'a mut toml::Table,
    path: &[&str],
    make: bool,
) -> Result<Option<&'a mut toml::Table>, Failed> {
    let mut at = root;
    for key in path {
        if !at.contains_key(*key) {
            if !make {
                return Ok(None);
            }
            at.insert((*key).to_owned(), toml::Value::Table(toml::Table::new()));
        }
        at = at
            .get_mut(*key)
            .and_then(toml::Value::as_table_mut)
            .ok_or_else(|| Failed::run(format!("{key} is not a table")))?;
    }
    Ok(Some(at))
}

/// `change` made in a document; whether a removal found what it removes.
fn change_tree(root: &mut toml::Table, change: &Change<'_>) -> Result<bool, Failed> {
    Ok(match change {
        Change::Replace(path, table) => {
            let (last, parents) = path
                .split_last()
                .ok_or_else(|| Failed::run("no table is named".to_owned()))?;
            if let Some(at) = tree_at(root, parents, true)? {
                at.insert((*last).to_owned(), toml::Value::Table((*table).clone()));
            }
            true
        }
        Change::Merge(path, table) => {
            if let Some(at) = tree_at(root, path, true)? {
                for (key, value) in *table {
                    at.insert(key.clone(), value.clone());
                }
            }
            true
        }
        Change::RemoveTable(path) => {
            let (last, parents) = path
                .split_last()
                .ok_or_else(|| Failed::run("no table is named".to_owned()))?;
            tree_at(root, parents, false)?.is_some_and(|at| {
                at.get(*last).is_some_and(toml::Value::is_table) && at.remove(*last).is_some()
            })
        }
        Change::RemoveKey(path, key) => {
            tree_at(root, path, false)?.is_some_and(|at| at.remove(*key).is_some())
        }
    })
}

/// Adds the table `[section.NAME]` to the configuration.
///
/// # Errors
///
/// The configuration already has a table of that name, or the store refuses it.
pub(crate) fn append_table(
    store: &dyn ConfigStore,
    section: &[&str],
    name: &str,
    table: &toml::Table,
) -> Result<(), Failed> {
    let mut at = section.to_vec();
    at.push(name);
    if holds(store, &at)? {
        return Err(Failed::usage(format!(
            "canoa.cfg has a table [{}] already",
            at.join(".")
        )));
    }
    apply(store, vec![set_op(&at, table)])
}

/// Whether the store holds a table at `at`.
fn holds(store: &dyn ConfigStore, at: &[&str]) -> Result<bool, Failed> {
    let held = store
        .get(Key::new(at.iter().copied()))
        .map_err(|error| Failed::run(error.to_string()))?;
    Ok(held
        .value
        .as_ref()
        .and_then(canoa::config_store::json_of)
        .is_some_and(|value| value.is_object()))
}

/// An op that makes the table at `at` hold `table`.
fn set_op(at: &[&str], table: &toml::Table) -> Op {
    let json = serde_json::to_value(table).unwrap_or(serde_json::Value::Null);
    Op::set(at.iter().copied(), canoa::config_store::value_of(&json))
}

/// Makes `ops` in the store, all or none.
fn apply(store: &dyn ConfigStore, ops: Vec<Op>) -> Result<(), Failed> {
    store
        .change(Changes::of(ops))
        .map(|_| ())
        .map_err(|error| Failed::usage(error.to_string()))
}

/// Held while canoa.cfg is read, changed and written back, so that an offer, an account added or removed and a setting saved never
/// undo one another: each reads the file the one before it wrote.
static CONFIG_WRITES: Mutex<()> = Mutex::new(());

/// Takes the right to change canoa.cfg, waiting for whoever has it.
pub(crate) fn config_writes() -> std::sync::MutexGuard<'static, ()> {
    CONFIG_WRITES.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) fn set_offered(
    store: &dyn ConfigStore,
    entry: &str,
    harness: &str,
    accounts: &[String],
    made: &[(&str, toml::Value)],
) -> Result<(), Failed> {
    let at = ["host", "agents", entry];
    let fresh = !holds(store, &at)?;
    if fresh && accounts.is_empty() {
        return Ok(());
    }
    let mut table = toml::Table::new();
    if fresh {
        table.insert("harness".into(), toml::Value::String(harness.to_owned()));
        for (key, value) in made {
            table.insert((*key).to_owned(), value.clone());
        }
    }
    let list = toml::Value::Array(
        accounts
            .iter()
            .map(|name| toml::Value::String(name.clone()))
            .collect(),
    );
    table.insert("accounts".into(), list.clone());
    let key = ["host", "agents", entry, "accounts"];
    let ops = if accounts.is_empty() {
        vec![Op::remove(key)]
    } else if fresh {
        vec![set_op(&at, &table)]
    } else {
        let json = serde_json::to_value(&list).unwrap_or(serde_json::Value::Null);
        vec![Op::set(key, canoa::config_store::value_of(&json))]
    };
    apply(store, ops)
}

pub(crate) fn set_pool(
    store: &dyn ConfigStore,
    name: &str,
    pool: &crate::config::PoolConfig,
) -> Result<(), Failed> {
    let mut table = toml::Table::new();
    table.insert(
        "standard".into(),
        toml::Value::String(pool.standard.clone()),
    );
    table.insert(
        "accounts".into(),
        toml::Value::Array(
            pool.accounts
                .iter()
                .map(|member| toml::Value::String(member.clone()))
                .collect(),
        ),
    );
    let at = ["host", "pools", name];
    apply(store, vec![set_op(&at, &table)])
}

pub(crate) fn set_table(
    store: &dyn ConfigStore,
    at: &[&str],
    table: Option<&toml::Table>,
) -> Result<(), Failed> {
    match table {
        Some(table) => apply(store, vec![set_op(at, table)]),
        None => {
            if !holds(store, at)? {
                return Err(Failed::usage(format!(
                    "canoa.cfg does not hold [{}] as a table of its own: change it by hand",
                    at.join(".")
                )));
            }
            apply(store, vec![Op::remove(at.iter().copied())])
        }
    }
}

pub(super) fn remove_account(store: &dyn ConfigStore, name: &str) -> Result<(), Failed> {
    remove_table(store, &["host", "accounts"], name, "canoa accounts add")
}

pub(crate) fn remove_table(
    store: &dyn ConfigStore,
    section: &[&str],
    name: &str,
    writer: &str,
) -> Result<(), Failed> {
    let mut at = section.to_vec();
    at.push(name);
    if !holds(store, &at)? {
        return Err(Failed::usage(format!(
            "canoa.cfg does not hold [{}] as `{writer}` writes it: remove it from the file by hand",
            at.join(".")
        )));
    }
    apply(store, vec![Op::remove(at.iter().copied())])
}

#[cfg(test)]
pub(crate) fn remove_entry(store: &dyn ConfigStore, name: &str) -> Result<(), Failed> {
    let at = ["host", "agents", name];
    let held = store
        .get(Key::new(at))
        .map_err(|error| Failed::run(error.to_string()))?;
    let json = held.value.as_ref().and_then(canoa::config_store::json_of);
    match json {
        Some(serde_json::Value::Object(entry)) => {
            if entry.values().any(serde_json::Value::is_object) {
                return Err(Failed::usage(format!(
                    "[host.agents.{name}] has tables of its own: change canoa.cfg by hand"
                )));
            }
        }
        _ => {
            return Err(Failed::usage(format!(
                "canoa.cfg has no [host.agents.{name}] as a table of its own: change it by hand"
            )));
        }
    }
    apply(store, vec![Op::remove(at)])
}

#[cfg(all(feature = "yaml", feature = "json"))]
#[cfg(test)]
mod format_tests {
    use super::*;
    use crate::config::EntryKind;

    /// A rewrite of canoa.cfg leaves its mode as the person set it, a file only this user reads included.
    #[cfg(unix)]
    #[test]
    fn a_rewrite_of_canoa_cfg_keeps_its_mode() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = std::env::temp_dir().join(format!("canoa-accounts-mode-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("directory");
        for mode in [0o600, 0o644] {
            let path = dir.join(format!("{mode:o}.cfg"));
            fs::write(&path, "[desktop]\nstart_hidden = true\n").expect("write");
            fs::set_permissions(&path, fs::Permissions::from_mode(mode)).expect("mode");
            write_cfg(&path, "[desktop]\nstart_hidden = false\n").expect("rewritten");
            let now = fs::metadata(&path).expect("metadata").permissions().mode() & 0o777;
            assert_eq!(now, mode, "{mode:o} became {now:o}");
            assert!(fs::read_to_string(&path).expect("read").contains("false"));
        }
        let _ = fs::remove_dir_all(&dir);
    }

    fn account() -> AccountConfig {
        AccountConfig {
            owner: None,
            kind: "api".to_owned(),
            id: "id-1".to_owned(),
            settings: None,
        }
    }

    /// `accounts add` and `remove` keep a YAML or JSON file in its format, with its marker or its extension.
    #[test]
    fn an_account_is_added_and_removed_in_the_files_own_format() {
        let dir =
            std::env::temp_dir().join(format!("canoa-accounts-format-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("directory");
        for (name, text) in [
            ("a.cfg", "#!yaml\ndesktop:\n  start_hidden: true\n"),
            ("b.cfg", "#!json\n{\"desktop\": {\"start_hidden\": true}}\n"),
            ("c.yaml", "desktop:\n  start_hidden: true\n"),
            ("d.json", "{\"desktop\": {\"start_hidden\": true}}\n"),
            ("e.cfg", "[desktop]\nstart_hidden = true\n"),
        ] {
            let path = dir.join(name);
            fs::write(&path, text).expect("write");
            append_account(
                &crate::config_store::FileStore::at(&path),
                "work",
                &account(),
            )
            .expect(name);
            let written = fs::read_to_string(&path).expect("read");
            let marker = text.lines().next().filter(|line| line.starts_with("#!"));
            if let Some(marker) = marker {
                assert!(written.starts_with(marker), "{name}: {written}");
            } else {
                assert!(!written.starts_with("#!"), "{name}: {written}");
            }
            let config = Config::from_str_named(&written, name, Path::new(".")).expect(name);
            assert_eq!(config.accounts["work"].id, "id-1", "{name}");
            assert!(
                written.contains("start_hidden"),
                "{name}: the other tables stay"
            );
            remove_account(&crate::config_store::FileStore::at(&path), "work").expect(name);
            let after = fs::read_to_string(&path).expect("read");
            assert!(!after.contains("work"), "{name}: {after}");
            assert!(after.contains("start_hidden"), "{name}: {after}");
        }
        fs::remove_dir_all(dir).expect("cleanup");
    }

    /// An entry's offered accounts are set in a TOML, YAML or JSON file in that file's own format, whether the entry is there or not.
    #[test]
    fn the_accounts_an_entry_offers_are_set_in_the_files_own_format() {
        let dir = std::env::temp_dir().join(format!("canoa-offer-format-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("directory");
        let offered = |text: &str, name: &str| {
            let config = Config::from_str_named(text, name, Path::new(".")).expect(name);
            let entry = config.entries.iter().find(|each| each.name == "Claude");
            match entry.map(|each| &each.kind) {
                Some(EntryKind::Harness {
                    harness, accounts, ..
                }) => Some((harness.clone(), accounts.clone())),
                _ => None,
            }
        };
        for (name, text) in [
            ("a.cfg", "#!yaml\ndesktop:\n  start_hidden: true\n"),
            ("b.cfg", "#!json\n{\"desktop\": {\"start_hidden\": true}}\n"),
            ("c.yaml", "desktop:\n  start_hidden: true\n"),
            ("e.cfg", "[desktop]\nstart_hidden = true\n"),
            (
                "f.cfg",
                "[host.agents.Claude]\n# the person's own\nharness = \"claude\"\ntitle = \"Mine\"\n\n[desktop]\nstart_hidden = true\n",
            ),
            (
                "g.cfg",
                "[host.agents.Claude]\nharness = \"claude\"\naccounts = [\"work\"]\n\n[desktop]\nstart_hidden = true\n",
            ),
        ] {
            let path = dir.join(name);
            fs::write(&path, text).expect("write");
            for who in ["work", "other"] {
                let mut made = account();
                made.id = format!("id-{who}");
                append_account(&crate::config_store::FileStore::at(&path), who, &made).expect(name);
            }
            let both = vec!["other".to_owned(), "work".to_owned()];
            set_offered(
                &crate::config_store::FileStore::at(&path),
                "Claude",
                "claude",
                &both,
                &[],
            )
            .expect(name);
            let written = fs::read_to_string(&path).expect("read");
            assert_eq!(
                offered(&written, name),
                Some(("claude".to_owned(), both.clone())),
                "{name}: {written}"
            );
            assert!(
                written.contains("start_hidden"),
                "{name}: other tables stay"
            );
            if name == "f.cfg" {
                assert!(written.contains("# the person's own") && written.contains("Mine"));
            }
            set_offered(
                &crate::config_store::FileStore::at(&path),
                "Claude",
                "claude",
                &["work".to_owned()],
                &[],
            )
            .expect(name);
            let written = fs::read_to_string(&path).expect("read");
            assert_eq!(
                offered(&written, name).map(|each| each.1),
                Some(vec!["work".to_owned()]),
                "{name}: {written}"
            );
            set_offered(
                &crate::config_store::FileStore::at(&path),
                "Claude",
                "claude",
                &[],
                &[],
            )
            .expect(name);
            let written = fs::read_to_string(&path).expect("read");
            assert_eq!(
                offered(&written, name).map(|each| each.1),
                Some(Vec::new()),
                "{name}: {written}"
            );
        }
        fs::remove_dir_all(dir).expect("cleanup");
    }

    /// Accounts added and offers made at the same moment all reach canoa.cfg: each reads the file the one before it wrote.
    #[test]
    fn changes_made_at_the_same_moment_do_not_undo_one_another() {
        let dir = std::env::temp_dir().join(format!("canoa-writes-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("directory");
        let path = dir.join("canoa.cfg");
        fs::write(&path, "[desktop]\nstart_hidden = true\n").expect("write");
        std::thread::scope(|scope| {
            for index in 0..12 {
                let store = crate::config_store::FileStore::at(&path);
                scope.spawn(move || {
                    let mut made = account();
                    made.id = format!("id-{index}");
                    append_account(&store, &format!("a{index}"), &made).expect("added");
                    set_offered(
                        &store,
                        &format!("E{index}"),
                        "claude",
                        &[format!("a{index}")],
                        &[],
                    )
                    .expect("offered");
                });
            }
        });
        let written = fs::read_to_string(&path).expect("read");
        let config = Config::from_str_named(&written, "canoa.cfg", Path::new(".")).expect("reads");
        assert_eq!(config.accounts.len(), 12, "{written}");
        assert_eq!(config.entries.len(), 12, "{written}");
        assert!(written.contains("start_hidden"));
        fs::remove_dir_all(dir).expect("cleanup");
    }
}
