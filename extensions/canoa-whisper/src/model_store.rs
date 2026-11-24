//! whisper.cpp's models as a Store page, and the speech offer that runs them. The host knows none of it: it asks this provider through
//! the `model_store` kind, draws the rows it answers, and writes the person's speech provider from the settings it reports.
//!
//! A model is its own install: the page's `+` has the host download the model's file into the models folder it keeps for the type,
//! checked against the publisher's SHA-256, and the row's remove takes it away. There is no program: [`crate::WhisperSpeech`] runs
//! the model inside the host's process.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "paths are std's"
)]

use std::path::Path;

use canoa::store::models::{
    Fact, ModelAct, ModelActed, ModelFile, ModelKeep, ModelKept, ModelList, ModelQuery, ModelRow,
    ModelStore, ModelStoreInfo, RowAction, STEP_DELETE_FILES, STEP_FETCH_FILES, Section,
    ServedModel, ServiceQuery, ServiceReport, SpeechService,
};
use coqui_acp_api::ProcessSpawner;
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{Provider, Status, ToValue};

use crate::NAME;
use crate::catalog::{ENDPOINTS, MODELS, MODELS_ORIGIN, Model};
use crate::speech::{PROVIDER, WhisperSettings};

/// What the speech offer says when the host keeps no models folder.
const NO_FOLDER: &str = "this host has no folder to keep models in";

/// What the speech offer says when no model is installed.
const NO_MODEL: &str = "no model is installed: install one from the Models page";

/// The models of whisper.cpp, as a page of the Store and as the person's speech-to-text.
#[derive(Debug, Clone, Copy, Provider)]
#[provider(ModelStore, id = "whisper_models", crate = ::coqui_acp_api::guatiao)]
pub struct WhisperModels {
    models: &'static [Model],
}

impl Default for WhisperModels {
    fn default() -> Self {
        Self { models: &MODELS }
    }
}

impl WhisperModels {
    /// The page over another list of models, which a test serves from a world of its own.
    #[must_use]
    pub const fn over(models: &'static [Model]) -> Self {
        Self { models }
    }
}

fn unsupported(why: &str) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_UNSUPPORTED, why)
}

/// Whether `model`'s file is in the models folder `folder`.
fn installed(model: &Model, folder: Option<&Path>) -> bool {
    folder.is_some_and(|folder| folder.join(model.file).is_file())
}

fn megabytes(bytes: u64) -> String {
    format!("{} MB", bytes.div_ceil(1_000_000))
}

fn row_of(model: &Model, on: bool, folder_known: bool) -> ModelRow {
    let mut row = ModelRow::new(model.id, model.label);
    row.on = on;
    row.state = on.then(|| "installed".to_owned());
    row.tags = vec![
        "speech-to-text".to_owned(),
        if model.id.ends_with(".en") {
            "English only"
        } else {
            "Every language"
        }
        .to_owned(),
    ];
    row.facts = vec![
        Fact::new("Runtime", NAME),
        Fact::new("Size", megabytes(model.bytes)),
        Fact::new("File", model.file),
        Fact::new("SHA-256", model.sha256),
    ];
    row.files = vec![ModelFile::new(
        format!("{MODELS_ORIGIN}{}", model.file),
        model.file,
        Some(model.bytes),
        model.sha256,
    )];
    if folder_known {
        row.actions.push(if on {
            RowAction::new(
                format!("Remove {}", model.id),
                format!(
                    "Takes the file of {} ({}) off this computer. Installing it again downloads it again.",
                    model.id,
                    megabytes(model.bytes)
                ),
                "Remove",
                [STEP_DELETE_FILES],
            )
        } else {
            RowAction::new(
                format!("Install {}", model.id),
                format!(
                    "Downloads {} ({}) from its publisher and checks it against the publisher's SHA-256. whisper.cpp runs it inside this host.",
                    model.id,
                    megabytes(model.bytes)
                ),
                "Install",
                [STEP_FETCH_FILES],
            )
        });
    }
    let mut how = Section::new("Install it");
    how.paragraphs = vec![
        format!(
            "The Models page downloads {} on its own, checked against its publisher's checksum, and whisper.cpp runs it inside the host: there is no program to install.",
            model.id
        ),
        "Dictation then runs on this computer's processor and the audio goes nowhere else. A larger model is slower and uses more memory.".to_owned(),
    ];
    row.sections = vec![how];
    row
}

impl ModelStore for WhisperModels {
    fn describe(&self) -> Result<ModelStoreInfo, ProviderError> {
        let mut info = ModelStoreInfo::new(
            "Whisper models",
            "whisper.cpp's models",
            "ggml-org",
            "The models whisper.cpp runs.",
        );
        let mut speech = SpeechService::in_process(
            NAME,
            "Speech to text on this host's own processor, in every language the model knows. A model is downloaded once, checked against its publisher's checksum, and runs inside the host.",
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
        drop(spawner);
        let folder = query.folder.as_deref().map(Path::new);
        let mut list = ModelList::default();
        list.models = self
            .models
            .iter()
            .map(|model| row_of(model, installed(model, folder), folder.is_some()))
            .collect();
        Ok(list)
    }

    fn act(
        &self,
        request: ModelAct,
        spawner: Object<dyn ProcessSpawner>,
    ) -> Result<ModelActed, ProviderError> {
        drop((request, spawner));
        Err(unsupported(
            "a whisper.cpp model is downloaded and removed by the host: it has no other action",
        ))
    }

    fn keep(
        &self,
        request: ModelKeep,
        spawner: Object<dyn ProcessSpawner>,
    ) -> Result<ModelKept, ProviderError> {
        drop((request, spawner));
        Err(unsupported("a whisper.cpp model keeps no setting"))
    }

    fn service(
        &self,
        query: ServiceQuery,
        spawner: Object<dyn ProcessSpawner>,
    ) -> Result<ServiceReport, ProviderError> {
        drop(spawner);
        Ok(report_over(
            self.models,
            query.folder.as_deref().map(Path::new),
            query.model.as_deref(),
            query.models,
        ))
    }
}

/// What the speech offer reports for the models folder `folder`: with `models`, the models installed there (the default first); and
/// the settings that run `wanted` when it is installed, else the first that is.
fn report_over(
    all: &[Model],
    folder: Option<&Path>,
    wanted: Option<&str>,
    models: bool,
) -> ServiceReport {
    let mut report = ServiceReport::default();
    let Some(folder) = folder else {
        report.unavailable = Some(NO_FOLDER.to_owned());
        return report;
    };
    let ready: Vec<&Model> = all
        .iter()
        .filter(|model| installed(model, Some(folder)))
        .collect();
    if models {
        report.models = ready
            .iter()
            .map(|model| ServedModel::new(model.id, model.label))
            .collect();
    }
    let pick = wanted
        .and_then(|id| ready.iter().find(|model| model.id == id))
        .or_else(|| ready.first());
    match pick {
        None => report.unavailable = Some(NO_MODEL.to_owned()),
        Some(model) => {
            let settings = WhisperSettings::new(model.id, folder.to_string_lossy());
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
            "canoa-whisper-folder-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("a folder");
        for file in files {
            std::fs::write(root.join(file), b"m").expect("model");
        }
        root
    }

    fn rows(folder: Option<&Path>) -> Vec<ModelRow> {
        MODELS
            .iter()
            .map(|model| row_of(model, installed(model, folder), folder.is_some()))
            .collect()
    }

    #[test]
    fn a_model_that_is_not_installed_offers_to_be_downloaded_with_its_file_and_digest() {
        let models = folder("offer", &[]);
        let all = rows(Some(&models));
        assert_eq!(all.len(), MODELS.len());
        for (row, model) in all.iter().zip(MODELS.iter()) {
            assert!(!row.on, "{}", row.id);
            let [install] = row.actions.as_slice() else {
                panic!("one action: {:?}", row.actions);
            };
            assert_eq!(install.steps, [STEP_FETCH_FILES]);
            let [file] = row.files.as_slice() else {
                panic!("one file");
            };
            assert_eq!(file.url, format!("{MODELS_ORIGIN}{}", model.file));
            assert_eq!(file.file, model.file);
            assert_eq!(file.sha256, model.sha256);
            assert_eq!(file.bytes, Some(model.bytes));
        }
        let _ = std::fs::remove_dir_all(&models);
    }

    #[test]
    fn a_model_in_the_hosts_folder_is_installed_and_offers_its_removal() {
        let models = folder("removal", &["ggml-base.en.bin"]);
        let all = rows(Some(&models));
        let on: Vec<&str> = all
            .iter()
            .filter(|row| row.on)
            .map(|row| row.id.as_str())
            .collect();
        assert_eq!(on, ["base.en"]);
        let row = all.iter().find(|row| row.on).expect("a row");
        assert_eq!(row.state.as_deref(), Some("installed"));
        let [remove] = row.actions.as_slice() else {
            panic!("one action: {:?}", row.actions);
        };
        assert_eq!(remove.steps, [STEP_DELETE_FILES]);
        assert!(
            all.iter()
                .all(|row| row.tags.iter().any(|tag| tag == "speech-to-text"))
        );
        assert!(all.iter().all(|row| {
            row.facts
                .iter()
                .any(|fact| fact.label == "Runtime" && fact.value == crate::NAME)
        }));
        let _ = std::fs::remove_dir_all(&models);
    }

    #[test]
    fn a_host_with_no_folder_lists_models_it_cannot_move() {
        let all = rows(None);
        assert!(all.iter().all(|row| row.actions.is_empty() && !row.on));
    }

    #[test]
    fn the_service_lists_the_models_installed_and_settles_on_the_one_asked_for_or_the_first() {
        let none = report_over(&MODELS, None, None, true);
        assert!(none.settings.is_none() && none.models.is_empty());
        assert!(
            none.unavailable
                .as_deref()
                .is_some_and(|why| why.contains("no folder")),
            "{none:?}"
        );
        let empty = folder("service-empty", &[]);
        let nothing = report_over(&MODELS, Some(&empty), None, true);
        assert!(nothing.models.is_empty() && nothing.settings.is_none());
        assert!(
            nothing
                .unavailable
                .as_deref()
                .is_some_and(|why| why.contains("no model")),
            "{nothing:?}"
        );
        let some = folder("service", &["ggml-tiny.bin", "ggml-base.en.bin"]);
        let all = report_over(&MODELS, Some(&some), None, true);
        let ids: Vec<&str> = all.models.iter().map(|model| model.id.as_str()).collect();
        assert_eq!(
            ids,
            ["base.en", "tiny"],
            "installed ones, in the list's order"
        );
        assert!(all.unavailable.is_none());
        let settings = WhisperSettings::from_value(&all.settings.expect("settings")).expect("read");
        assert_eq!(settings.model, "base.en", "the first installed");
        assert_eq!(Path::new(&settings.models_dir), some);
        let asked = report_over(&MODELS, Some(&some), Some("tiny"), false);
        assert!(
            asked.models.is_empty(),
            "the models are listed when asked for"
        );
        let tiny = WhisperSettings::from_value(&asked.settings.expect("settings")).expect("read");
        assert_eq!(tiny.model, "tiny");
        let missing = report_over(&MODELS, Some(&some), Some("small"), false);
        let first =
            WhisperSettings::from_value(&missing.settings.expect("settings")).expect("read");
        assert_eq!(
            first.model, "base.en",
            "a model that is not installed is not run"
        );
        let _ = std::fs::remove_dir_all(&some);
        let _ = std::fs::remove_dir_all(&empty);
    }

    #[test]
    fn the_description_is_an_in_process_provider_with_the_origins_its_files_come_from() {
        let info = WhisperModels::default().describe().expect("described");
        assert!(info.problem().is_none(), "{:?}", info.problem());
        let speech = info.speech.expect("a speech service");
        assert!(!speech.command && speech.route.is_empty());
        assert_eq!(speech.provider.as_deref(), Some(PROVIDER));
        assert_eq!(speech.license.as_deref(), Some("MIT"));
        assert!(
            info.endpoints
                .iter()
                .any(|origin| MODELS_ORIGIN.starts_with(origin.as_str()))
        );
        assert!(
            info.endpoints
                .iter()
                .all(|origin| origin.starts_with("https://"))
        );
    }
}
