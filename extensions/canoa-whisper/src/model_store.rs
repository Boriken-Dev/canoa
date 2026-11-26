//! The Whisper type's side of the host's calls: what the install does once the folder is downloaded (writes whisper.cpp's GGML file
//! from it) and the speech offer that runs the installed models with either engine.
//!
//! The page's rows are the catalogs' ([`crate::source`]): the host downloads each model's three files into `<id>/` of the models
//! folder, checked against the digests the listing gives. This provider says what the host cannot: which installed models each engine
//! can run, with the settings of the engine's own speech provider.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "paths are std's"
)]

use std::path::Path;

use crate::catalog::{ENDPOINTS, FILE_NAMES, describe_id, runs_model};
use canoa::store::models::{
    ModelAct, ModelActed, ModelKeep, ModelKept, ModelList, ModelQuery, ModelStore, ModelStoreInfo,
    ServedModel, ServiceQuery, ServiceReport, SpeechService,
};
use coqui_acp_api::ProcessSpawner;
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{Provider, Status, ToValue};

#[cfg(feature = "whispercpp")]
use crate::WHISPERCPP;
use crate::{CANDLE, NAME, WRITE_GGML};

/// What the speech offer says when the host keeps no models folder.
const NO_FOLDER: &str = "this host has no folder to keep models in";

/// What the speech offer says when no model is installed.
const NO_MODEL: &str = "no model is installed: install one from the Store";

/// The models of Whisper, as the person's speech-to-text. The page's rows are the catalogs'.
#[derive(Default, Debug, Clone, Copy, Provider)]
#[provider(ModelStore, id = "whisper_models", crate = ::coqui_acp_api::guatiao)]
pub struct WhisperModels;

fn unsupported(why: &str) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_UNSUPPORTED, why)
}

/// One way an installed model runs.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Runs {
    /// The engine's id: [`CANDLE`] or [`WHISPERCPP`].
    engine: &'static str,
    /// The model's id: the name of its folder.
    model: String,
    /// What the person reads of the engine: its name and, for a file written at install, the form it was written in.
    how: String,
}

impl Runs {
    /// The id a recording's request, and the offer's model list, name it by.
    fn id(&self) -> String {
        format!("{}:{}", self.engine, self.model)
    }

    fn label(&self) -> String {
        format!("{} ({})", describe_id(&self.model), self.how)
    }
}

/// The form a GGML file was written in, from the record the install left beside it.
#[cfg(feature = "whispercpp")]
fn precision_of(folder: &Path) -> Option<String> {
    let record = std::fs::read_to_string(folder.join(canoa_whispercpp::GGML_RECORD)).ok()?;
    let record: serde_json::Value = serde_json::from_str(&record).ok()?;
    record.get("precision")?.as_str().map(str::to_owned)
}

/// Every way the models installed in `folder` run, by model and then engine, Candle first.
fn installed(folder: &Path) -> Vec<Runs> {
    let mut models: Vec<String> = std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    models.sort();
    let mut found = Vec::new();
    for model in models {
        let own = folder.join(&model);
        if runs_model(&model) && FILE_NAMES.iter().all(|name| own.join(name).is_file()) {
            found.push(Runs {
                engine: CANDLE,
                model: model.clone(),
                how: "Candle".to_owned(),
            });
        }
        #[cfg(feature = "whispercpp")]
        if own.join(canoa_whispercpp::GGML_FILE).is_file() {
            let how = precision_of(&own).map_or_else(
                || "Whisper.cpp".to_owned(),
                |precision| format!("Whisper.cpp, {precision}"),
            );
            found.push(Runs {
                engine: WHISPERCPP,
                model,
                how,
            });
        }
    }
    found
}

/// The id of the speech provider that runs `engine`.
fn provider_of(engine: &str) -> &'static str {
    #[cfg(feature = "whispercpp")]
    if engine == WHISPERCPP {
        return canoa_whispercpp::PROVIDER;
    }
    let _ = engine;
    canoa_candle::PROVIDER
}

impl ModelStore for WhisperModels {
    fn describe(&self) -> Result<ModelStoreInfo, ProviderError> {
        let mut info = ModelStoreInfo::new(
            NAME,
            NAME,
            "OpenAI",
            "Nothing is listed yet: check the connection to Hugging Face.",
        );
        let mut speech = SpeechService::in_process(
            NAME,
            "Speech to text on this host's own processor, in the languages the model knows, with OpenAI's Whisper. A model is downloaded once, checked against its publisher's checksums, and runs inside the host with Candle or whisper.cpp.",
            "The audio goes to your host and is turned into text there, inside the host itself: nothing is sent to another service.",
            canoa_candle::PROVIDER,
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
        drop(spawner);
        if request.action != WRITE_GGML {
            return Err(unsupported(
                "a Whisper model is downloaded and removed by the host: the one action it runs is writing the whisper.cpp file",
            ));
        }
        write_ggml(&request)
    }

    fn keep(
        &self,
        request: ModelKeep,
        spawner: Object<dyn ProcessSpawner>,
    ) -> Result<ModelKept, ProviderError> {
        drop((request, spawner));
        Err(unsupported("a Whisper model keeps no setting"))
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

/// Writes whisper.cpp's file for the model `request.model` into its folder, in the form `request.options` names for the engine
/// (f16 when it names none), and records the form and the file's SHA-256 beside it.
#[cfg(feature = "whispercpp")]
fn write_ggml(request: &ModelAct) -> Result<ModelActed, ProviderError> {
    use canoa_whispercpp::{GGML_FILE, GGML_RECORD, Precision, convert};
    use sha2::{Digest, Sha256};
    use std::io::Read as _;

    let bad = |why: String| ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, &why);
    let folder = request
        .folder
        .as_deref()
        .ok_or_else(|| bad("this host has no folder to keep models in".to_owned()))?;
    let model = request.model.as_str();
    if model.is_empty() || model.contains(['/', '\\']) || model == ".." || model == "." {
        return Err(bad(format!(
            "{model:?} is not the name of a model's folder"
        )));
    }
    let precision = match request
        .options
        .as_deref()
        .and_then(|options| serde_json::from_str::<serde_json::Value>(options).ok())
        .and_then(|options| {
            options
                .get(WHISPERCPP)
                .and_then(|form| form.as_str().map(str::to_owned))
        }) {
        Some(name) => Precision::parse(&name).ok_or_else(|| {
            bad(format!(
                "{name:?} is not a form whisper.cpp's file can be written in"
            ))
        })?,
        None => Precision::F16,
    };
    let own = Path::new(folder).join(model);
    let out = own.join(GGML_FILE);
    convert(&own, &out, precision).map_err(|error| {
        ProviderError::new(
            Status::GUATIAO_ERR_INTERNAL,
            &format!("whisper.cpp's file for {model} could not be written: {error}"),
        )
    })?;
    let mut file = std::fs::File::open(&out)
        .map_err(|error| ProviderError::new(Status::GUATIAO_ERR_INTERNAL, &error.to_string()))?;
    let (mut digest, mut bytes, mut chunk) = (Sha256::new(), 0_u64, vec![0_u8; 1 << 20]);
    loop {
        let read = file.read(&mut chunk).map_err(|error| {
            ProviderError::new(Status::GUATIAO_ERR_INTERNAL, &error.to_string())
        })?;
        if read == 0 {
            break;
        }
        digest.update(&chunk[..read]);
        bytes += read as u64;
    }
    let sha256: String = digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let name = precision.suffix().trim_start_matches('-');
    let name = if name.is_empty() { "f16" } else { name };
    let record = serde_json::json!({"precision": name, "bytes": bytes, "sha256": sha256});
    std::fs::write(own.join(GGML_RECORD), record.to_string())
        .map_err(|error| ProviderError::new(Status::GUATIAO_ERR_INTERNAL, &error.to_string()))?;
    let mut acted = ModelActed::default();
    acted.note = Some(format!(
        "whisper.cpp's file for {model} is written as {name} ({bytes} bytes, SHA-256 {sha256})."
    ));
    Ok(acted)
}

#[cfg(not(feature = "whispercpp"))]
fn write_ggml(request: &ModelAct) -> Result<ModelActed, ProviderError> {
    let _ = request;
    Err(unsupported(
        "this build has no whisper.cpp to write a file for",
    ))
}

/// What the speech offer reports for the models folder `folder`: with `models`, every way an installed model runs; and the settings
/// of the engine's speech provider for `wanted` (`<engine>:<model>`, or a model's name for the first engine that runs it) when it
/// is installed, else for the first that is.
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
            .map(|runs| ServedModel::new(runs.id(), runs.label()))
            .collect();
    }
    let pick = wanted
        .and_then(|asked| {
            ready
                .iter()
                .find(|runs| runs.id() == asked)
                .or_else(|| ready.iter().find(|runs| runs.model == asked))
        })
        .or_else(|| ready.first());
    let Some(runs) = pick else {
        report.unavailable = Some(NO_MODEL.to_owned());
        return report;
    };
    report.provider = Some(provider_of(runs.engine).to_owned());
    report.settings = settings_of(runs, folder);
    report
}

/// The settings of the speech provider of `runs`' engine, for its model in `folder`.
fn settings_of(runs: &Runs, folder: &Path) -> Option<coqui_acp_api::guatiao::Value> {
    let dir = folder.to_string_lossy();
    #[cfg(feature = "whispercpp")]
    if runs.engine == WHISPERCPP {
        return canoa_whispercpp::WhisperSettings::new(runs.model.as_str(), dir)
            .to_value()
            .ok();
    }
    canoa_candle::WhisperCandleSettings::new(runs.model.as_str(), dir)
        .to_value()
        .ok()
}

#[cfg(all(test, feature = "whispercpp"))]
#[path = "model_store_tests.rs"]
mod tests;
