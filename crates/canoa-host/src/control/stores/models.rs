//! The models of a Store type: `store/models`, `store/model` and `store/model_setting`, over the `model_store` provider the type's
//! extension names, and what the person's speech-to-text asks of the service that runs those models.
//!
//! The host names no model and no program of a library's: it finds the type's program through the type's own `program_store`
//! providers, hands it to the provider with the person's home, runs nothing itself, and keeps a row's settings in the table of
//! canoa.cfg the provider names (`[<table>.models."<id>"]`).
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "paths and the registry are std's"
)]

use std::sync::Arc;

use canoa::store::models::{
    ModelAct, ModelKeep, ModelList, ModelQuery, ModelStore, ModelStoreInfo, ServiceQuery,
    ServiceReport, SpeechService,
};
use coqui_acp_api::ProcessSpawner;
use coqui_acp_api::guatiao::Status;
use coqui_acp_api::guatiao::library::{ProviderError, Registry};
use coqui_acp_harness::LocalSpawner;
use serde_json::{Map, Value, json};

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
                && let Ok(launch) = self.programs.launch(&*store)
            {
                return Some(launch.path);
            }
            if let Ok(report) = self.programs.report(&*store, false, None)
                && let Some(copy) = report.copies.first()
            {
                return Some(copy.path.clone());
            }
        }
        None
    }

    /// `store/models` `{"type"}` -> `{"type", "provider", "title", "catalog", "publisher", "missing", "unavailable"?,
    /// "settings"?: {"help"?, "schema", "form"?}, "speech": bool, "models": [row, ...]}`, each row as `canoa::store::models::ModelRow`
    /// with `"values"`, what canoa.cfg keeps for it. Runs the type's program as this user, and nothing else.
    pub(crate) fn models_read(&self, params: &Value, files: &Files) -> Result<Value, Failed> {
        let held = self.model_type(params)?;
        let list: ModelList = held
            .ask(|store, spawner| {
                let mut query = ModelQuery::default();
                query.program = self.type_program(&held.type_id);
                query.home = self.home_text();
                store.list(query, spawner)
            })
            .map_err(|error| failure(&error))?;
        let table = held.info.settings.as_ref().map(|each| each.table.as_str());
        let kept = table.map(|table| files.model_settings(table));
        let mut rows = Vec::new();
        for row in &list.models {
            let mut shown = json_of(row)?;
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
            "models": rows,
        }))
    }

    /// `store/model` `{"type", "model", "action"}` -> `{"ok": true, "model", "action", "note"?}`: the provider runs one action of
    /// the row's, which may take long (a download), and answers in its program's words when it is refused.
    pub(crate) fn model_act(&self, params: &Value) -> Result<Value, Failed> {
        let held = self.model_type(params)?;
        let model = text(params, "model")?;
        let action = text(params, "action")?;
        let mut request = ModelAct::new(model, action);
        request.program = self.type_program(&held.type_id);
        request.home = self.home_text();
        let acted = held
            .ask(|store, spawner| store.act(request, spawner))
            .map_err(|error| failure(&error))?;
        Ok(json!({"ok": true, "model": model, "action": action, "note": acted.note}))
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

    /// Where the service of the type `type_id` is now, and with `models` which of its models can transcribe; the reason when the
    /// provider could not say.
    pub(crate) fn speech_report(
        &self,
        type_id: &str,
        models: bool,
    ) -> Result<ServiceReport, String> {
        let held = self
            .models
            .of(type_id)
            .filter(|held| held.info.speech.is_some())
            .ok_or_else(|| format!("this host has no service {type_id}"))?;
        let mut query = ServiceQuery::default();
        query.home = self.home_text();
        query.models = models;
        held.ask(|store, spawner| store.service(query, spawner))
            .map_err(|error| failure(&error).1)
    }
}

/// Checks the `type` of an extension names a `model_store` the registry offers.
pub(super) fn offered(registry: &Registry, provider: &str) -> bool {
    registry.offer::<dyn ModelStore>(provider).is_some()
}

#[cfg(test)]
#[path = "models_tests.rs"]
mod tests;
