//! The models of a Store type: `store/models`, `store/model` and `store/model_setting`, over the `model_store` provider the type's
//! extension names, and what the person's speech-to-text asks of the service that runs those models.
//!
//! The host names no model and no program of a library's: it finds the type's program through the type's own `program_store`
//! providers, hands it to the provider with the person's home and the folder it keeps the type's models in, and keeps a row's
//! settings in the table of canoa.cfg the provider names (`[<table>.models."<id>"]`). It runs nothing itself, but for the two steps
//! of a row's action that move files: `fetch_files` downloads the row's files into that folder, checked against their digests, once
//! the type's program is there, and `delete_files` takes them out.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "paths and the registry are std's"
)]

use std::sync::Arc;

use canoa::store::models::{
    ModelAct, ModelKeep, ModelList, ModelQuery, ModelRow, ModelStore, ModelStoreInfo,
    STEP_DELETE_FILES, STEP_FETCH_FILES, ServiceQuery, ServiceReport, SpeechService,
};
use coqui_acp_api::ProcessSpawner;
use coqui_acp_api::guatiao::Status;
use coqui_acp_api::guatiao::library::{ProviderError, Registry};
use coqui_acp_harness::LocalSpawner;
use serde_json::{Map, Value, json};

use super::models_catalog::FromCatalogs;
use super::{BAD_PARAMS, Failed, REFUSED, Stores, json_of, text};
use crate::control::files::Files;

/// One `model_store` provider a Store type names.
#[derive(Clone)]
pub(super) struct Held {
    /// The Store type it lists the models of.
    pub(super) type_id: String,
    provider: String,
    registry: Arc<Registry>,
    pub(super) info: ModelStoreInfo,
}

/// The `model_store` providers of the Store types this host lists.
#[derive(Clone, Default)]
pub(super) struct ModelStores {
    held: Vec<Held>,
}

impl core::fmt::Debug for ModelStores {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list()
            .entries(self.held.iter().map(|each| &each.type_id))
            .finish()
    }
}

impl ModelStores {
    /// Takes the provider `provider` of `registry` as the models of the type `type_id`; why it is not offered, when it is not.
    pub(super) fn add(
        &mut self,
        type_id: &str,
        provider: &str,
        registry: &Arc<Registry>,
    ) -> Result<(), String> {
        let offer = registry
            .offer::<dyn ModelStore>(provider)
            .ok_or_else(|| format!("no provider {provider}"))?
            .map_err(|_| format!("{provider} is not a model_store"))?;
        let info = offer
            .default_instance()
            .ok_or_else(|| format!("{provider} needs settings"))?
            .describe()
            .map_err(|error| error.message().to_owned())?;
        if let Some(problem) = info.problem() {
            return Err(problem);
        }
        drop(offer);
        self.held.push(Held {
            type_id: type_id.to_owned(),
            provider: provider.to_owned(),
            registry: Arc::clone(registry),
            info,
        });
        Ok(())
    }

    pub(super) fn of(&self, type_id: &str) -> Option<&Held> {
        self.held.iter().find(|each| each.type_id == type_id)
    }

    pub(super) fn all(&self) -> impl Iterator<Item = &Held> {
        self.held.iter()
    }
}

impl Held {
    /// Runs `ask` on the provider, with the person's own reach as its spawner.
    fn ask<T>(
        &self,
        ask: impl FnOnce(
            &dyn ModelStore,
            coqui_acp_api::guatiao::library::Object<dyn ProcessSpawner>,
        ) -> Result<T, ProviderError>,
    ) -> Result<T, ProviderError> {
        let missing = |why: String| ProviderError::new(Status::GUATIAO_ERR_NOT_FOUND, &why);
        let offer = self
            .registry
            .offer::<dyn ModelStore>(&self.provider)
            .ok_or_else(|| missing(format!("no provider {}", self.provider)))?
            .map_err(|_| missing(format!("{} is not a model_store", self.provider)))?;
        let store = offer
            .default_instance()
            .ok_or_else(|| missing(format!("{} needs settings", self.provider)))?;
        ask(
            store,
            crate::store::sources::Reach(LocalSpawner::new()).into_object(),
        )
    }
}

/// What a provider's refusal is on the control channel: a value it would not take is a bad parameter, the rest is a refusal in
/// the provider's own words.
fn failure(error: &ProviderError) -> Failed {
    let words = if error.message().is_empty() {
        format!("the provider answered {:?}", error.status)
    } else {
        error.message().to_owned()
    };
    if error.status == Status::GUATIAO_ERR_BAD_VALUE {
        (BAD_PARAMS, words)
    } else {
        (REFUSED, words)
    }
}

impl Stores {
    /// The person's home, where a program keeps its own files: the one this host runs under unless a test names another.
    fn home_text(&self) -> Option<String> {
        self.home
            .as_deref()
            .map(|home| home.to_string_lossy().into_owned())
    }

    fn model_type(&self, params: &Value) -> Result<&Held, Failed> {
        let id = text(params, "type")?;
        self.models.of(id).ok_or_else(|| {
            (
                super::NOT_FOUND,
                format!("no Store type {id} lists models on this host"),
            )
        })
    }

    /// The program the type's stores found: the host's own verified copy of the first store that has one, else the first copy
    /// found on the computer. None when no store of the type found one.
    fn type_program(&self, type_id: &str) -> Option<String> {
        self.program_of(type_id, true)
    }

    /// As [`type_program`](Self::type_program) for a listing: the managed copy is the one `current.json` names, read without
    /// checking its files, which costs time in proportion to the copy. What is run with it still goes through `type_program`.
    fn type_program_found(&self, type_id: &str) -> Option<String> {
        self.program_of(type_id, false)
    }

    fn program_of(&self, type_id: &str, verified: bool) -> Option<String> {
        let stores = self.types.iter().find(|each| each.info.id == type_id)?;
        for provider in &stores.info.stores {
            let Ok((_, store)) = self.store(&json!({"provider": provider})) else {
                continue;
            };
            if self
                .programs
                .managed_version(&*store)
                .ok()
                .flatten()
                .is_some()
            {
                let launch = if verified {
                    self.programs.launch(&*store).ok().map(|launch| launch.path)
                } else {
                    self.programs.managed_launch(&*store).ok().flatten()
                };
                if launch.is_some() {
                    return launch;
                }
            }
            if let Ok(report) = self.programs.report(&*store, false, None)
                && let Some(copy) = report.copies.first()
            {
                return Some(copy.path.clone());
            }
        }
        None
    }

    /// The type's first program store: the one that installs what its models run on.
    fn type_store(&self, type_id: &str) -> Option<(String, Arc<dyn canoa::store::ProgramStore>)> {
        let provider = self.speech_store(type_id)?;
        let (_, store) = self.store(&json!({"provider": provider})).ok()?;
        Some((provider, store))
    }

    /// The folder the host keeps the models installed on their own in, for the type: beside its program's versions.
    ///
    /// A type with no program store keeps them under its own id on the shelf.
    pub(super) fn models_folder(&self, type_id: &str) -> Option<String> {
        let folder = if self.speech_store(type_id).is_some() {
            let (_, store) = self.type_store(type_id)?;
            self.programs.models_folder(&*store)
        } else {
            self.models.of(type_id)?;
            self.programs.models_folder_of(type_id)
        };
        folder
            .ok()
            .map(|folder| folder.to_string_lossy().into_owned())
    }

    /// The rows of the type as the provider lists them, from what the host kept of its last listing while that is within
    /// `[host.store] models_ttl`.
    fn model_rows(&self, held: &Held) -> Result<ModelList, Failed> {
        self.model_rows_asked(held, false)
    }

    /// As [`model_rows`](Self::model_rows); `refresh` lists them again whatever is kept.
    fn model_rows_asked(&self, held: &Held, refresh: bool) -> Result<ModelList, Failed> {
        self.caches.models.get(&held.type_id, refresh, || {
            held.ask(|store, spawner| {
                let mut query = ModelQuery::default();
                query.program = self.type_program_found(&held.type_id);
                query.home = self.home_text();
                query.folder = self.models_folder(&held.type_id);
                store.list(query, spawner)
            })
            .map_err(|error| failure(&error))
        })
    }

    /// The rows of the type: those its `model_store` lists, then the ones its catalogs list.
    fn rows_with_catalogs(
        &self,
        held: &Held,
        refresh: bool,
        files: &Files,
    ) -> Result<(ModelList, FromCatalogs), Failed> {
        let mut list = self.model_rows_asked(held, refresh)?;
        let taken = list.models.iter().map(|row| row.id.clone()).collect();
        let from = self.catalog_models(&held.type_id, files, &taken);
        list.models.extend(from.rows.iter().cloned());
        Ok((list, from))
    }

    /// What a note on the removal of the program of `provider` says of the models of its type: those installed on their own stay,
    /// and those installed with the program go with it. None when the program has no models.
    pub(crate) fn removal_note(&self, provider: &str) -> Option<String> {
        let added = self
            .types
            .iter()
            .find(|each| each.info.stores.iter().any(|store| store == provider))?;
        let held = self.models.of(&added.info.id)?;
        let rows = self.model_rows(held).ok()?;
        let installed = |own: bool| -> Vec<&str> {
            rows.models
                .iter()
                .filter(|row| row.on && can_remove(row) == own)
                .map(|row| row.id.as_str())
                .collect()
        };
        let (stay, go) = (installed(true), installed(false));
        let mut note = Vec::new();
        if !stay.is_empty() {
            note.push(format!(
                "The models installed on their own stay on this computer: {}.",
                stay.join(", ")
            ));
        }
        if !go.is_empty() {
            note.push(format!(
                "The models installed with the program go with it: {}.",
                go.join(", ")
            ));
        }
        (!note.is_empty()).then(|| note.join(" "))
    }

    /// `store/models` `{"type"}` -> `{"type", "provider", "title", "catalog", "publisher", "missing", "unavailable"?,
    /// "settings"?: {"help"?, "schema", "form"?}, "speech": bool, "sources": [{"label", "provider", "origin", "enabled", "trust",
    /// "pin", "fetched"?, "from"?, "stale"?, "items"?, "problem"?}], "models": [row, ...]}`, each row as `canoa::store::models::ModelRow`
    /// with `"values"`, what canoa.cfg keeps for it, and for a row a catalog listed `"catalog"` (`label`, `provider`, `origin`,
    /// `trust`, `pin`, `revision`, `fetched`, `from`, `stale`) and `"trust"` (`verified`, `pinned`, `unverified`, null when it has no
    /// digest and is not installable). Runs the type's program as this user, and nothing else.
    pub(crate) fn models_read(&self, params: &Value, files: &Files) -> Result<Value, Failed> {
        let held = self.model_type(params)?;
        let refresh = params
            .get("refresh")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let (list, from) = self.rows_with_catalogs(held, refresh, files)?;
        let table = held.info.settings.as_ref().map(|each| each.table.as_str());
        let kept = table.map(|table| files.model_settings(table));
        let mut rows = Vec::new();
        for row in &list.models {
            let mut shown = json_of(row)?;
            if let Some(meta) = from.meta.get(&row.id) {
                shown["catalog"] = meta["catalog"].clone();
                shown["trust"] = meta["trust"].clone();
            }
            shown["values"] = kept
                .as_ref()
                .and_then(|kept| kept.get(&row.id))
                .cloned()
                .unwrap_or_else(|| json!({}));
            rows.push(shown);
        }
        let settings = match &held.info.settings {
            Some(settings) => json!({
                "help": settings.help,
                "schema": crate::forms::json_of(&settings.schema).map_err(|why| (REFUSED, why))?,
                "form": settings.form.as_ref().and_then(|form| crate::forms::json_of(form).ok()),
            }),
            None => Value::Null,
        };
        Ok(json!({
            "type": held.type_id,
            "provider": held.provider,
            "title": held.info.title,
            "catalog": held.info.catalog,
            "publisher": held.info.publisher,
            "missing": held.info.missing,
            "unavailable": list.unavailable,
            "settings": settings,
            "speech": held.info.speech.is_some(),
            "program": self.type_store(&held.type_id).and_then(|(provider, store)| {
                let title = store.describe().ok()?.title;
                Some(json!({
                    "provider": provider,
                    "title": title,
                    "installed": self.type_program_found(&held.type_id).is_some(),
                }))
            }),
            "sources": from.sources,
            "models": rows,
        }))
    }

    /// `store/model` `{"type", "model", "action"}` -> `{"ok": true, "model", "action", "note"?}`: the provider runs one action of
    /// the row's, which may take long (a download), and answers in its program's words when it is refused. The steps `fetch_files` and
    /// `delete_files` are the host's: they move the row's files in the type's models folder.
    pub(crate) fn model_act(&self, params: &Value, files: &Files) -> Result<Value, Failed> {
        let held = self.model_type(params)?;
        let model = text(params, "model")?;
        let action = text(params, "action")?;
        // What the action changes is what a listing says, so none is kept past it, done or not.
        let _forget = Forget(&self.caches.models);
        if action == STEP_FETCH_FILES || action == STEP_DELETE_FILES {
            return self.move_files(held, model, action, files);
        }
        let mut request = ModelAct::new(model, action);
        request.program = self.type_program(&held.type_id);
        request.home = self.home_text();
        let acted = held
            .ask(|store, spawner| store.act(request, spawner))
            .map_err(|error| failure(&error))?;
        Ok(json!({"ok": true, "model": model, "action": action, "note": acted.note}))
    }

    /// The step `fetch_files` or `delete_files` of the row `model`: its files fetched into the models folder, or taken out of it.
    fn move_files(
        &self,
        held: &Held,
        model: &str,
        action: &str,
        files: &Files,
    ) -> Result<Value, Failed> {
        let (rows, from) = self.rows_with_catalogs(held, false, files)?;
        let row = rows
            .models
            .iter()
            .find(|row| row.id == model)
            .ok_or_else(|| {
                (
                    super::NOT_FOUND,
                    format!("{} lists no model {model}", held.info.title),
                )
            })?;
        if row.files.is_empty() {
            return Err((REFUSED, format!("{model} has no files to move")));
        }
        if action == STEP_FETCH_FILES
            && let Some(why) = from.refused.get(model)
        {
            return Err((REFUSED, format!("{model} cannot be installed: {why}")));
        }
        let extra = from.endpoints.get(model).cloned().unwrap_or_default();
        if action == STEP_FETCH_FILES && from.meta.contains_key(model) {
            let own = match self
                .type_store(&held.type_id)
                .filter(|_| self.speech_store(&held.type_id).is_some())
            {
                Some((_, store)) => store
                    .describe()
                    .map(|info| info.endpoints)
                    .unwrap_or_default(),
                None => held.info.endpoints.clone(),
            };
            if let Some(file) = row
                .files
                .iter()
                .find(|file| !is_reached(&own, &extra, &file.url))
            {
                return Err((
                    REFUSED,
                    format!(
                        "{model} cannot be installed: its file {} is at {}, which is none of the places {} downloads from; a catalog entry that is trusted or pinned names more with endpoints = [\"https://...\"]",
                        file.file, file.url, held.info.title
                    ),
                ));
            }
        }
        if self.speech_store(&held.type_id).is_none() {
            // A type with no program: its files are fetched from the origins its model store names, and nothing has to be installed.
            if action == STEP_DELETE_FILES {
                self.programs
                    .delete_models_in(&held.type_id, &row.files)
                    .map_err(|error| super::refusal(&error))?;
            } else {
                self.programs
                    .fetch_models_in(
                        &held.type_id,
                        &[held.info.endpoints.clone(), extra].concat(),
                        &row.files,
                        from.checks.get(model).map(Vec::as_slice),
                        Arc::new(std::sync::atomic::AtomicBool::new(false)),
                    )
                    .map_err(|error| super::refusal(&error))?;
            }
            return Ok(json!({"ok": true, "model": model, "action": action, "note": null}));
        }
        let (_, store) = self.type_store(&held.type_id).ok_or_else(|| {
            (
                REFUSED,
                format!("{} has no program to keep {model} beside", held.info.title),
            )
        })?;
        if action == STEP_DELETE_FILES {
            self.programs
                .delete_models(&*store, &row.files)
                .map_err(|error| super::refusal(&error))?;
            return Ok(json!({"ok": true, "model": model, "action": action, "note": null}));
        }
        if self.type_program(&held.type_id).is_none() {
            let program = store
                .describe()
                .map_or_else(|_| held.info.title.clone(), |info| info.title);
            return Err((
                REFUSED,
                format!(
                    "{model} runs on {program}, which is not installed: install {program} first"
                ),
            ));
        }
        let info = store
            .describe()
            .map_err(|error| (REFUSED, error.message().to_owned()))?;
        self.programs
            .fetch_models_in(
                &info.program,
                &[info.endpoints.clone(), extra].concat(),
                &row.files,
                from.checks.get(model).map(Vec::as_slice),
                Arc::new(std::sync::atomic::AtomicBool::new(false)),
            )
            .map_err(|error| super::refusal(&error))?;
        Ok(json!({"ok": true, "model": model, "action": action, "note": null}))
    }

    /// `store/model_setting` `{"type", "model", "values"}` -> `{"ok": true, "model", "values", "note"?}`: the values are checked
    /// against the store's schema, settled by the provider (a number held to what the model allows) and kept in canoa.cfg's
    /// `[<table>.models."<model>"]`; no values take the row's settings away.
    pub(crate) fn model_setting(&self, params: &Value, files: &Files) -> Result<Value, Failed> {
        let held = self.model_type(params)?;
        let model = text(params, "model")?;
        let settings = held.info.settings.as_ref().ok_or_else(|| {
            (
                REFUSED,
                format!("{} keeps no setting of a model", held.info.title),
            )
        })?;
        let given = match params.get("values") {
            None | Some(Value::Null) => Map::new(),
            Some(Value::Object(map)) => map
                .iter()
                .filter(|(_, value)| !value.is_null())
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
            Some(_) => return Err((BAD_PARAMS, "values is an object".to_owned())),
        };
        crate::forms::check(Some(&settings.schema), &given).map_err(|why| (BAD_PARAMS, why))?;
        let mut request = ModelKeep::new(model, Value::Object(given).to_string());
        request.home = self.home_text();
        let kept = held
            .ask(|store, spawner| store.keep(request, spawner))
            .map_err(|error| failure(&error))?;
        let values: Map<String, Value> = serde_json::from_str(&kept.values).map_err(|_| {
            (
                REFUSED,
                format!("{} answered values that are not an object", held.provider),
            )
        })?;
        crate::forms::check(Some(&settings.schema), &values).map_err(|why| (REFUSED, why))?;
        files.set_model_settings(
            &settings.table,
            model,
            (!values.is_empty()).then_some(values.clone()),
        )?;
        Ok(json!({"ok": true, "model": model, "values": values, "note": kept.note}))
    }

    /// The types whose models the person's speech-to-text may call, and how each says its service is called.
    pub(crate) fn speech_services(&self) -> Vec<(String, SpeechService)> {
        self.models
            .all()
            .filter_map(|held| Some((held.type_id.clone(), held.info.speech.clone()?)))
            .collect()
    }

    /// The types whose installed program reads text aloud, each with the id of the `speech_output` provider of the same library that
    /// speaks with it.
    pub(crate) fn voice_services(&self) -> Vec<(String, String)> {
        self.models
            .all()
            .filter_map(|held| Some((held.type_id.clone(), held.info.voice.clone()?)))
            .collect()
    }

    /// The settings the type `type_id`'s voice provider is built from, for the copy of its program the host installed or found: none
    /// when there is no copy, or the provider could not say.
    pub(crate) fn voice_settings(&self, type_id: &str) -> Option<Value> {
        let held = self
            .models
            .of(type_id)
            .filter(|held| held.info.voice.is_some())?;
        let mut query = ServiceQuery::default();
        query.home = self.home_text();
        query.folder = self.models_folder(type_id);
        query.program = self.type_program(type_id);
        query.program.as_ref()?;
        let report = held
            .ask(|store, spawner| store.service(query, spawner))
            .ok()?;
        crate::forms::json_of(&report.voice?).ok()
    }

    /// The store a type's program is installed as when its speech service is that program: the type's first.
    pub(crate) fn speech_store(&self, type_id: &str) -> Option<String> {
        self.types
            .iter()
            .find(|each| each.info.id == type_id)?
            .info
            .stores
            .first()
            .cloned()
    }

    /// Where the service of the type `type_id` is now, and with `models` which of its models can transcribe; for a program, how its copy
    /// is run. The reason when the provider could not say.
    pub(crate) fn speech_report(
        &self,
        type_id: &str,
        models: bool,
    ) -> Result<ServiceReport, String> {
        self.speech_report_of(type_id, models, None)
    }

    /// As [`speech_report`](Self::speech_report), for a program that runs the model `model` when it is installed.
    pub(crate) fn speech_report_of(
        &self,
        type_id: &str,
        models: bool,
        model: Option<&str>,
    ) -> Result<ServiceReport, String> {
        let held = self
            .models
            .of(type_id)
            .filter(|held| held.info.speech.is_some())
            .ok_or_else(|| format!("this host has no service {type_id}"))?;
        let mut query = ServiceQuery::default();
        query.home = self.home_text();
        query.folder = self.models_folder(type_id);
        query.model = model.map(str::to_owned);
        query.models = models;
        if held
            .info
            .speech
            .as_ref()
            .is_some_and(|speech| speech.command)
        {
            query.program = match self.speech_store(type_id) {
                Some(store) => self.managed_launch(&store).map_err(|failed| failed.1)?,
                None => None,
            };
        }
        held.ask(|store, spawner| store.service(query, spawner))
            .map_err(|error| failure(&error).1)
    }
}

/// Whether the host can take a row's files away: it offers the `delete_files` step.
fn can_remove(row: &ModelRow) -> bool {
    row.actions
        .iter()
        .any(|action| action.steps.iter().any(|step| step == STEP_DELETE_FILES))
}

/// Checks the `type` of an extension names a `model_store` the registry offers.
pub(super) fn offered(registry: &Registry, provider: &str) -> bool {
    registry.offer::<dyn ModelStore>(provider).is_some()
}

#[cfg(test)]
#[path = "models_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "models_files_tests.rs"]
mod files_tests;

/// Empties a cache when dropped.
pub(super) struct Forget<'a, V: Clone>(pub(super) &'a crate::store::caches::TtlCache<V>);

impl<V: Clone> Drop for Forget<'_, V> {
    fn drop(&mut self) {
        self.0.clear();
    }
}

/// Whether `url` is at one of `own` (the places the type downloads from) or `extra` (the ones a catalog's entry adds).
fn is_reached(own: &[String], extra: &[String], url: &str) -> bool {
    let origin = url
        .split_once("://")
        .map(|(scheme, rest)| format!("{scheme}://{}", rest.split('/').next().unwrap_or_default()))
        .unwrap_or_default();
    own.iter()
        .chain(extra)
        .any(|endpoint| crate::store::tools::endpoint_reaches(endpoint, url, &origin))
}
