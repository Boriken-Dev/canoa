//! The host's own [`ConfigStore`]: canoa.cfg's layers, as files in TOML, YAML or JSON, the person's own the one written.
//!
//! Every read and write of the configuration the host makes goes through a store; this is the one it is given unless a test or an
//! embedding gives another. A file in TOML is edited in place, so its comments, its order and the way the person spelled their keys
//! stay; YAML and JSON have no layout of their own and are rewritten whole.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files are std's"
)]

use std::path::{Path, PathBuf};

use canoa::config_store::{
    Changed, Changes, ConfigStore, Held, Key, Layer, LayerInfo, Snapshot, StoreInfo, json_of,
    path_problem, value_of,
};
use canoa::status::{bad_value, changed, internal};
use coqui_acp_api::guatiao::library::ProviderError;
use sha2::{Digest as _, Sha256};

use crate::accounts::commands::{Change, check_cfg, config_writes, edit_cfg, read_cfg};
use crate::cli::Failed;
use crate::config::Layers;

struct FileLayer {
    id: String,
    label: String,
    path: PathBuf,
}

/// canoa.cfg's files as a [`ConfigStore`].
pub(crate) struct FileStore {
    layers: Vec<FileLayer>,
    own: usize,
}

impl core::fmt::Debug for FileStore {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FileStore")
            .field("own", &self.layers[self.own].path)
            .finish_non_exhaustive()
    }
}

fn why(failed: &Failed) -> ProviderError {
    if failed.status == 2 {
        bad_value(&failed.why)
    } else {
        internal(&failed.why)
    }
}

impl FileStore {
    /// The one file `path`, which is written.
    pub(crate) fn at(path: &Path) -> Self {
        Self {
            layers: vec![FileLayer {
                id: "own".to_owned(),
                label: format!("canoa.cfg ({})", path.display()),
                path: path.to_owned(),
            }],
            own: 0,
        }
    }

    /// The files of `layers`, lowest first, of which `layers.own` is written.
    pub(crate) fn of(layers: &Layers) -> Self {
        let own = layers
            .files
            .iter()
            .position(|file| *file == layers.own)
            .unwrap_or(layers.files.len().saturating_sub(1));
        let made: Vec<FileLayer> = layers
            .files
            .iter()
            .enumerate()
            .map(|(at, path)| FileLayer {
                id: if at == own {
                    "own".to_owned()
                } else {
                    format!("layer-{at}")
                },
                label: format!("canoa.cfg ({})", path.display()),
                path: path.clone(),
            })
            .collect();
        if made.is_empty() {
            return Self::at(&layers.own);
        }
        Self { layers: made, own }
    }

    fn own(&self) -> &FileLayer {
        &self.layers[self.own]
    }

    fn table(path: &Path) -> Result<toml::Table, ProviderError> {
        let text = read_cfg(path).map_err(|failed| why(&failed))?;
        Self::table_of(path, &text)
    }

    /// The tables `text` holds, read as the file at `path` is.
    fn table_of(path: &Path, text: &str) -> Result<toml::Table, ProviderError> {
        if text.trim().is_empty() {
            return Ok(toml::Table::new());
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("canoa.cfg");
        match crate::format::formats()
            .from_slice(text.as_bytes(), anycfg::Options::new().name(name))
        {
            Ok(toml::Value::Table(table)) => Ok(table),
            Ok(_) => Err(bad_value("canoa.cfg is not a table of settings")),
            Err(error) => Err(bad_value(format!("{}: {error}", path.display()))),
        }
    }

    fn revision_now(&self) -> Result<String, ProviderError> {
        let mut hash = Sha256::new();
        for layer in &self.layers {
            hash.update(layer.path.as_os_str().as_encoded_bytes());
            hash.update(
                read_cfg(&layer.path)
                    .map_err(|failed| why(&failed))?
                    .as_bytes(),
            );
            hash.update([0]);
        }
        Ok(data_encoding::HEXLOWER.encode(&hash.finalize()))
    }
}

fn json_table(table: &toml::Table) -> serde_json::Value {
    serde_json::to_value(table).unwrap_or(serde_json::Value::Null)
}

fn find<'a>(table: &'a toml::Table, path: &[String]) -> Option<&'a toml::Value> {
    let (first, rest) = path.split_first()?;
    let mut here = table.get(first)?;
    for key in rest {
        here = here.as_table()?.get(key)?;
    }
    Some(here)
}

impl ConfigStore for FileStore {
    fn describe(&self) -> Result<StoreInfo, ProviderError> {
        let own = self.own();
        let mut info = StoreInfo::new(
            "canoa.cfg",
            self.layers
                .iter()
                .map(|layer| {
                    let mut info = LayerInfo::new(&layer.id, &layer.label);
                    info.origin = Some(layer.path.display().to_string());
                    info.writable = layer.id == own.id;
                    info
                })
                .collect(),
        );
        info.base_dir = own.path.parent().map(|dir| dir.display().to_string());
        info.name = own
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned());
        Ok(info)
    }

    fn read(&self) -> Result<Snapshot, ProviderError> {
        let mut layers = Vec::new();
        for layer in &self.layers {
            layers.push(Layer::new(
                &layer.id,
                value_of(&json_table(&Self::table(&layer.path)?)),
            ));
        }
        Ok(Snapshot::new(self.revision_now()?, layers))
    }

    fn get(&self, key: Key) -> Result<Held, ProviderError> {
        if let Some(problem) = path_problem(&key.path) {
            return Err(bad_value(problem));
        }
        for layer in self.layers.iter().rev() {
            let table = Self::table(&layer.path)?;
            if let Some(found) = find(&table, &key.path) {
                return Ok(Held::of(
                    value_of(&serde_json::to_value(found).unwrap_or(serde_json::Value::Null)),
                    &layer.id,
                ));
            }
        }
        Ok(Held::none())
    }

    fn change(&self, changes: Changes) -> Result<Changed, ProviderError> {
        let _one = config_writes();
        let path = &self.own().path;
        if let Some(expected) = &changes.expect_revision
            && *expected != self.revision_now()?
        {
            return Err(changed("canoa.cfg changed since it was read"));
        }
        let mut text = read_cfg(path).map_err(|failed| why(&failed))?;
        let was = text.clone();
        for op in &changes.ops {
            if let Some(problem) = path_problem(&op.path) {
                return Err(bad_value(problem));
            }
            let keys: Vec<&str> = op.path.iter().map(String::as_str).collect();
            let before = Self::table_of(path, &text)?;
            // A name on the way that holds a value is no table to put anything in.
            let mut at = &before;
            for (depth, key) in op.path.iter().enumerate().take(op.path.len() - 1) {
                match at.get(key) {
                    None => break,
                    Some(toml::Value::Table(next)) => at = next,
                    Some(_) => {
                        return Err(bad_value(format!(
                            "{} is a value, not a table",
                            op.path[..=depth].join(".")
                        )));
                    }
                }
            }
            match &op.value {
                Some(value) => {
                    let json =
                        json_of(value).ok_or_else(|| bad_value("a value JSON cannot hold"))?;
                    let value = crate::accounts::commands::toml_value(&json)
                        .ok_or_else(|| bad_value("a null is no setting"))?;
                    let (last, parents) = keys.split_last().ok_or_else(|| bad_value("no path"))?;
                    text = match value {
                        toml::Value::Table(table) => {
                            edit_cfg(path, &text, &[Change::Replace(&keys, &table)])
                        }
                        other if parents.is_empty() => {
                            let _ = other;
                            return Err(bad_value("a top-level key of canoa.cfg is a table"));
                        }
                        other => {
                            let mut table = toml::Table::new();
                            table.insert((*last).to_owned(), other);
                            edit_cfg(path, &text, &[Change::Merge(parents, &table)])
                        }
                    }
                    .map_err(|failed| why(&failed))?
                    .0;
                }
                None => {
                    let held = find(&before, &op.path);
                    if held.is_none() {
                        if let Some(layer) = self.layers[..self.own].iter().rev().find(|layer| {
                            Self::table(&layer.path)
                                .ok()
                                .is_some_and(|table| find(&table, &op.path).is_some())
                        }) {
                            return Err(bad_value(format!(
                                "{} is in {}: remove it there",
                                op.path.join("."),
                                layer.label
                            )));
                        }
                        continue;
                    }
                    let change = if held.is_some_and(toml::Value::is_table) {
                        Change::RemoveTable(&keys)
                    } else {
                        let (last, parents) =
                            keys.split_last().ok_or_else(|| bad_value("no path"))?;
                        Change::RemoveKey(parents, last)
                    };
                    text = edit_cfg(path, &text, &[change])
                        .map_err(|failed| why(&failed))?
                        .0;
                }
            }
        }
        check_cfg(path, &text).map_err(|failed| why(&failed))?;
        let document = value_of(&json_table(&Self::table_of(path, &text)?));
        if !changes.check_only && text != was {
            crate::replace::write_file(path, text.as_bytes(), 0o600)
                .map_err(|error| internal(format!("{}: {error}", path.display())))?;
        }
        Ok(Changed::new(self.revision_now()?, document))
    }

    fn revision(&self) -> Result<String, ProviderError> {
        self.revision_now()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use canoa::config_store::{Op, contract};

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("canoa-store-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("directory");
        dir
    }

    #[test]
    fn the_files_meet_the_contract_every_store_meets() {
        let dir = dir("contract");
        let file = dir.join("canoa.cfg");
        std::fs::write(&file, "# mine\n[desktop]\nstart_hidden = true\n").expect("write");
        contract::check(&FileStore::at(&file));
        let text = std::fs::read_to_string(&file).expect("read");
        assert!(text.starts_with("# mine\n"), "its comment stays: {text:?}");
        assert!(text.contains("start_hidden = true"));
        assert!(
            !text.contains("contract"),
            "the contract cleaned up: {text:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(all(feature = "yaml", feature = "json"))]
    #[test]
    fn a_file_in_yaml_meets_it_too_and_stays_yaml() {
        let dir = dir("yaml");
        let file = dir.join("canoa.cfg");
        std::fs::write(&file, "#!yaml\ndesktop:\n  start_hidden: true\n").expect("write");
        contract::check(&FileStore::at(&file));
        let text = std::fs::read_to_string(&file).expect("read");
        assert!(text.starts_with("#!yaml"), "{text:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn what_a_lower_layer_holds_is_shown_with_its_file_and_a_removal_names_it() {
        let dir = dir("layers");
        let (low, own) = (dir.join("installed.cfg"), dir.join("canoa.cfg"));
        std::fs::write(&low, "[host.agents.low]\nprogram = \"x\"\n").expect("write");
        std::fs::write(&own, "[desktop]\nstart_hidden = true\n").expect("write");
        let store = FileStore::of(&Layers::from_files(vec![low.clone(), own.clone()]));
        let info = store.describe().expect("describe");
        assert_eq!(info.writable(), Some("own"));
        let held = store.get(Key::new(["host", "agents", "low"])).expect("get");
        assert_eq!(held.layer.as_deref(), Some("layer-0"));
        let refused = store
            .change(Changes::of(vec![Op::remove(["host", "agents", "low"])]))
            .expect_err("it is the installed copy's");
        assert!(
            refused.to_string().contains("installed.cfg"),
            "the file is named: {refused}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_change_the_host_would_refuse_is_not_written() {
        let dir = dir("refused");
        let file = dir.join("canoa.cfg");
        std::fs::write(&file, "[desktop]\nstart_hidden = true\n").expect("write");
        let store = FileStore::at(&file);
        let bad = Op::set(
            ["host", "web"],
            canoa::config_store::value_of(&serde_json::json!({"no_such_key": 1})),
        );
        assert!(store.change(Changes::of(vec![bad])).is_err());
        assert_eq!(
            std::fs::read_to_string(&file).expect("read"),
            "[desktop]\nstart_hidden = true\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
