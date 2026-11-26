//! Whisper's models for Candle as a Store page, and the speech offer that runs them. The host knows none of it: it asks this provider through
//! the `model_store` kind and writes the person's speech provider from the settings it reports.
//!
//! The models the page lists come from the type's catalogs ([`crate::source`]), which the host reads: a model is its own install, the
//! page's `+` has the host download the model's three files into its folder of the models folder the host keeps for the type, each
//! checked against the digest its catalog gives, and the row's remove takes them away. This provider says what the host cannot:
//! which folders of that folder hold a model the speech offer can run. There is no program: [`crate::WhisperCandleSpeech`] runs the
//! model inside the host's process.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "paths are std's"
)]

use std::path::Path;

use canoa::store::models::{
    ModelAct, ModelActed, ModelKeep, ModelKept, ModelList, ModelQuery, ModelStore, ModelStoreInfo,
    ServedModel, ServiceQuery, ServiceReport, SpeechService,
};
use coqui_acp_api::ProcessSpawner;
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{Provider, Status, ToValue};

use crate::NAME;
use crate::catalog::{ENDPOINTS, FILE_NAMES, describe_id};
use crate::speech::{PROVIDER, WhisperCandleSettings};

/// What the speech offer says when the host keeps no models folder.
const NO_FOLDER: &str = "this host has no folder to keep models in";

/// What the speech offer says when no model is installed.
const NO_MODEL: &str = "no model is installed: install one from the Models page";

/// The models of Whisper for Candle, as the person's speech-to-text. The page's rows are the catalogs'.
#[derive(Default, Debug, Clone, Copy, Provider)]
#[provider(ModelStore, id = "whispercandle_models", crate = ::coqui_acp_api::guatiao)]
pub struct WhisperCandleModels;

fn unsupported(why: &str) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_UNSUPPORTED, why)
}

/// The models installed in `folder`: each folder of it that holds all three files, by its name, as `(id, label)`.
fn installed(folder: &Path) -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter(|entry| {
            FILE_NAMES
                .iter()
                .all(|name| entry.path().join(name).is_file())
        })
        .map(|entry| {
            let id = entry.file_name().to_string_lossy().into_owned();
            let label = describe_id(&id);
            (id, label)
        })
        .collect();
    found.sort();
    found
}

impl ModelStore for WhisperCandleModels {
    fn describe(&self) -> Result<ModelStoreInfo, ProviderError> {
        let mut info = ModelStoreInfo::new(
            "Whisper models for Candle",
            "Whisper's models for Candle",
            "OpenAI",
            "The models Candle runs.",
        );
        let mut speech = SpeechService::in_process(
            NAME,
            "Speech to text on this host's own processor, in every language the model knows, with Whisper written in Rust. A model is downloaded once, checked against its publisher's checksums, and runs inside the host.",
            "The audio goes to your host and is turned into text there, inside the host itself: nothing is sent to another service.",
            PROVIDER,
        );
        speech.license = Some("MIT".to_owned());
        info.speech = Some(speech);
        info.endpoints = ENDPOINTS.map(str::to_owned).to_vec();
        Ok(info)
    }

    fn list(
        &self,
        query: ModelQuery,
        spawner: Object<dyn ProcessSpawner>,
    ) -> Result<ModelList, ProviderError> {
        drop((query, spawner));
        Ok(ModelList::default())
    }

    fn act(
        &self,
        request: ModelAct,
        spawner: Object<dyn ProcessSpawner>,
    ) -> Result<ModelActed, ProviderError> {
        drop((request, spawner));
        Err(unsupported(
            "a Candle model is downloaded and removed by the host: it has no other action",
        ))
    }

    fn keep(
        &self,
        request: ModelKeep,
        spawner: Object<dyn ProcessSpawner>,
    ) -> Result<ModelKept, ProviderError> {
        drop((request, spawner));
        Err(unsupported("a Candle model keeps no setting"))
    }

    fn service(
        &self,
        query: ServiceQuery,
        spawner: Object<dyn ProcessSpawner>,
    ) -> Result<ServiceReport, ProviderError> {
        drop(spawner);
        Ok(report_over(
            query.folder.as_deref().map(Path::new),
            query.model.as_deref(),
            query.models,
        ))
    }
}

/// What the speech offer reports for the models folder `folder`: with `models`, the models installed there (by name); and the
/// settings that run `wanted` when it is installed, else the first that is.
fn report_over(folder: Option<&Path>, wanted: Option<&str>, models: bool) -> ServiceReport {
    let mut report = ServiceReport::default();
    let Some(folder) = folder else {
        report.unavailable = Some(NO_FOLDER.to_owned());
        return report;
    };
    let ready = installed(folder);
    if models {
        report.models = ready
            .iter()
            .map(|(id, label)| ServedModel::new(id.as_str(), label.as_str()))
            .collect();
    }
    let pick = wanted
        .and_then(|id| ready.iter().find(|(each, _)| each == id))
        .or_else(|| ready.first());
    match pick {
        None => report.unavailable = Some(NO_MODEL.to_owned()),
        Some((id, _)) => {
            let settings = WhisperCandleSettings::new(id.as_str(), folder.to_string_lossy());
            report.settings = settings.to_value().ok();
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use coqui_acp_api::guatiao::FromValue;

    use super::*;

    /// The host's models folder, with `files` in it.
    fn folder(name: &str, files: &[&str]) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "canoa-whisper-candle-folder-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("a folder");
        for file in files {
            let path = root.join(file);
            std::fs::create_dir_all(path.parent().expect("a parent")).expect("a model folder");
            std::fs::write(path, b"m").expect("model");
        }
        root
    }

    /// The three files of `id`, as the folder holds them.
    fn files_of(id: &str) -> Vec<String> {
        FILE_NAMES
            .iter()
            .map(|name| format!("{id}/{name}"))
            .collect()
    }

    #[test]
    fn the_service_lists_the_models_installed_and_settles_on_the_one_asked_for_or_the_first() {
        let none = report_over(None, None, true);
        assert!(none.settings.is_none() && none.models.is_empty());
        assert!(
            none.unavailable
                .as_deref()
                .is_some_and(|why| why.contains("no folder")),
            "{none:?}"
        );
        let empty = folder("service-empty", &[]);
        let nothing = report_over(Some(&empty), None, true);
        assert!(nothing.models.is_empty() && nothing.settings.is_none());
        assert!(
            nothing
                .unavailable
                .as_deref()
                .is_some_and(|why| why.contains("no model")),
            "{nothing:?}"
        );
        let mut present = files_of("tiny");
        present.extend(files_of("base"));
        // Half a model is not a model.
        present.extend(files_of("small").into_iter().take(2));
        let present: Vec<&str> = present.iter().map(String::as_str).collect();
        let some = folder("service", &present);
        let all = report_over(Some(&some), None, true);
        let ids: Vec<&str> = all.models.iter().map(|model| model.id.as_str()).collect();
        assert_eq!(ids, ["base", "tiny"], "installed ones, by name");
        assert!(all.unavailable.is_none());
        let settings =
            WhisperCandleSettings::from_value(&all.settings.expect("settings")).expect("read");
        assert_eq!(settings.model, "base", "the first installed");
        assert_eq!(Path::new(&settings.models_dir), some);
        let asked = report_over(Some(&some), Some("tiny"), false);
        assert!(
            asked.models.is_empty(),
            "the models are listed when asked for"
        );
        let tiny =
            WhisperCandleSettings::from_value(&asked.settings.expect("settings")).expect("read");
        assert_eq!(tiny.model, "tiny");
        let missing = report_over(Some(&some), Some("small"), false);
        let first =
            WhisperCandleSettings::from_value(&missing.settings.expect("settings")).expect("read");
        assert_eq!(
            first.model, "base",
            "a model that is not installed is not run"
        );
        let _ = std::fs::remove_dir_all(&some);
        let _ = std::fs::remove_dir_all(&empty);
    }

    #[test]
    fn the_description_is_an_in_process_provider_with_the_origins_its_files_come_from() {
        let info = WhisperCandleModels.describe().expect("described");
        assert!(info.problem().is_none(), "{:?}", info.problem());
        let speech = info.speech.expect("a speech service");
        assert!(!speech.command && speech.route.is_empty());
        assert_eq!(speech.provider.as_deref(), Some(PROVIDER));
        assert_eq!(speech.license.as_deref(), Some("MIT"));
        assert!(
            info.endpoints
                .iter()
                .all(|origin| origin.starts_with("https://"))
        );
        assert!(info.endpoints.iter().any(|origin| {
            "https://huggingface.co/openai/whisper-tiny/resolve/x/config.json"
                .starts_with(origin.as_str())
        }));
    }
}
