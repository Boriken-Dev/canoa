//! A [`ConfigStore`] that keeps its layers in memory: the store of a host with nowhere to keep one, and the second provider that proves
//! nothing above a store knows where the configuration lives.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use std::sync::{Mutex, PoisonError};

use coqui_acp_api::guatiao::library::ProviderError;
use serde_json::{Map, Value as Json};

use super::{
    Changed, Changes, ConfigStore, Held, Key, Layer, LayerInfo, Snapshot, StoreInfo, json_of,
    path_problem, value_of,
};
use crate::status::{bad_value, changed};

struct State {
    layers: Vec<(LayerInfo, Map<String, Json>)>,
    revision: u64,
}

/// Layers of tables held in memory, the last one writable.
pub struct MemoryStore {
    state: Mutex<State>,
}

impl core::fmt::Debug for MemoryStore {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MemoryStore").finish_non_exhaustive()
    }
}

impl Default for MemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryStore {
    /// An empty store: one writable layer, `own`.
    #[must_use]
    pub fn new() -> Self {
        let mut own = LayerInfo::new("own", "the writable layer");
        own.writable = true;
        Self {
            state: Mutex::new(State {
                layers: alloc::vec![(own, Map::new())],
                revision: 0,
            }),
        }
    }

    /// A layer below the ones it has, which [`ConfigStore::change`] never writes. `document` is a JSON object.
    ///
    /// # Panics
    ///
    /// `document` is not an object.
    #[must_use]
    pub fn with_lower(self, id: &str, label: &str, document: Json) -> Self {
        let Json::Object(map) = document else {
            panic!("a layer's document is an object");
        };
        let mut info = LayerInfo::new(id, label);
        info.origin = Some(alloc::format!("memory:{id}"));
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .layers
            .insert(0, (info, map));
        self
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn at<'a>(root: &'a Map<String, Json>, path: &[String]) -> Option<&'a Json> {
    let (first, rest) = path.split_first()?;
    let mut here = root.get(first)?;
    for key in rest {
        here = here.as_object()?.get(key)?;
    }
    Some(here)
}

fn set(root: &mut Map<String, Json>, path: &[String], value: Json) -> Result<(), ProviderError> {
    let (last, parents) = path.split_last().ok_or_else(|| bad_value("no path"))?;
    let mut here = root;
    for key in parents {
        let next = here
            .entry(key.clone())
            .or_insert_with(|| Json::Object(Map::new()));
        here = next
            .as_object_mut()
            .ok_or_else(|| bad_value(alloc::format!("{key} is a value, not a table")))?;
    }
    here.insert(last.clone(), value);
    Ok(())
}

fn remove(root: &mut Map<String, Json>, path: &[String]) -> Result<bool, ProviderError> {
    let (last, parents) = path.split_last().ok_or_else(|| bad_value("no path"))?;
    let mut here = root;
    for key in parents {
        match here.get_mut(key) {
            None => return Ok(false),
            Some(next) => {
                here = next
                    .as_object_mut()
                    .ok_or_else(|| bad_value(alloc::format!("{key} is a value, not a table")))?;
            }
        }
    }
    Ok(here.remove(last).is_some())
}

impl ConfigStore for MemoryStore {
    fn describe(&self) -> Result<StoreInfo, ProviderError> {
        let state = self.lock();
        Ok(StoreInfo::new(
            "memory",
            state.layers.iter().map(|(info, _)| info.clone()).collect(),
        ))
    }

    fn read(&self) -> Result<Snapshot, ProviderError> {
        let state = self.lock();
        Ok(Snapshot {
            revision: state.revision.to_string(),
            layers: state
                .layers
                .iter()
                .map(|(info, document)| Layer {
                    id: info.id.clone(),
                    document: value_of(&Json::Object(document.clone())),
                })
                .collect(),
        })
    }

    fn get(&self, key: Key) -> Result<Held, ProviderError> {
        if let Some(why) = path_problem(&key.path) {
            return Err(bad_value(why));
        }
        let state = self.lock();
        for (info, document) in state.layers.iter().rev() {
            if let Some(found) = at(document, &key.path) {
                return Ok(Held {
                    value: Some(value_of(found)),
                    layer: Some(info.id.clone()),
                });
            }
        }
        Ok(Held::none())
    }

    fn change(&self, changes: Changes) -> Result<Changed, ProviderError> {
        let mut state = self.lock();
        if let Some(expected) = &changes.expect_revision
            && *expected != state.revision.to_string()
        {
            return Err(changed("the store changed since it was read"));
        }
        let own = state.layers.len() - 1;
        let mut document = state.layers[own].1.clone();
        for op in &changes.ops {
            if let Some(why) = path_problem(&op.path) {
                return Err(bad_value(why));
            }
            match &op.value {
                Some(value) => {
                    let json =
                        json_of(value).ok_or_else(|| bad_value("a value JSON cannot hold"))?;
                    set(&mut document, &op.path, json)?;
                }
                None => {
                    if !remove(&mut document, &op.path)? {
                        let held_below = state.layers[..own]
                            .iter()
                            .rev()
                            .find(|(_, lower)| at(lower, &op.path).is_some());
                        if let Some((info, _)) = held_below {
                            return Err(bad_value(alloc::format!(
                                "{} is held by {}: remove it there",
                                op.path.join("."),
                                info.label
                            )));
                        }
                    }
                }
            }
        }
        let now = value_of(&Json::Object(document.clone()));
        if !changes.check_only && document != state.layers[own].1 {
            state.layers[own].1 = document;
            state.revision += 1;
        }
        Ok(Changed {
            revision: state.revision.to_string(),
            document: now,
        })
    }

    fn revision(&self) -> Result<String, ProviderError> {
        Ok(self.lock().revision.to_string())
    }
}
