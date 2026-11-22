//! whisper.cpp's models as a Store page, and the speech offer that runs them. The host knows none of it: it asks this provider through
//! the `model_store` kind, draws the rows it answers, and writes the person's speech provider from the command it reports.
//!
//! A model is not moved on its own: the Store installs the program and one model together, the model being the install's channel, so
//! the page lists what can be chosen and which model the host's copy holds, and `act` has nothing to do.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "paths are std's"
)]

use std::path::Path;

use canoa::store::models::{
    Fact, ModelAct, ModelActed, ModelKeep, ModelKept, ModelList, ModelQuery, ModelRow, ModelStore,
    ModelStoreInfo, Section, ServedModel, ServiceQuery, ServiceReport, SpeechRun, SpeechService,
};
use coqui_acp_api::ProcessSpawner;
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{Provider, Status};

use crate::NAME;
use crate::store::{MODELS, Model, held_model, model_path};

/// What the speech offer says when the program is not installed.
const NOT_INSTALLED: &str = "whisper.cpp is not installed: install it in the Store first";

/// What the speech offer says when the copy holds no model.
const NO_MODEL: &str = "the installed copy holds no model: install whisper.cpp again";

/// The models of whisper.cpp, as a page of the Store and as the person's speech-to-text.
#[derive(Default, Debug, Clone, Copy, Provider)]
#[provider(ModelStore, id = "whisper_models", crate = ::coqui_acp_api::guatiao)]
pub struct WhisperModels;

fn unsupported(why: &str) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_UNSUPPORTED, why)
}

/// The arguments that make `whisper-cli` read a WAV stream on its standard input and print the text alone: no timestamps and none
/// of its log, the language the person said or its own detection, and their words as the prompt. `-of` names a file only so the
/// program prints to its standard output, which it does not for a stream otherwise.
#[must_use]
pub fn arguments(model: &str) -> Vec<String> {
    [
        "-m",
        model,
        "-f",
        "-",
        "-of",
        "canoa",
        "-nt",
        "-np",
        "-l",
        "{language_code|auto}",
        "--prompt",
        "{vocabulary}",
    ]
    .map(str::to_owned)
    .into()
}

/// How to run the copy whose program is `program`, with the model the copy holds. The program is in `bin` beside the version's
/// `models` folder.
///
/// # Errors
///
/// Why the copy cannot be run: its layout is not the Store's, or it holds no model.
pub fn run_of(program: &str) -> Result<SpeechRun, String> {
    let version = Path::new(program)
        .parent()
        .and_then(Path::parent)
        .ok_or("the installed copy has an unexpected layout")?;
    let held = held_model(version).ok_or(NO_MODEL)?;
    let model = version.join(model_path(held));
    Ok(SpeechRun::new(
        program,
        arguments(&model.to_string_lossy()),
        "wav",
        held.id,
    ))
}

fn megabytes(bytes: u64) -> String {
    format!("{} MB", bytes.div_ceil(1_000_000))
}

fn row_of(model: &Model, held: Option<&Model>) -> ModelRow {
    let mut row = ModelRow::new(model.id, model.label);
    let on = held.is_some_and(|held| held.id == model.id);
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
    let mut how = Section::new("Install it");
    how.paragraphs = vec![
        format!(
            "Install whisper.cpp from the Store's Whisper list and choose the channel {}: the Store downloads the program and this model together, each checked against its publisher's checksum.",
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
            "Install whisper.cpp from the Store to run a model.",
        );
        let mut speech = SpeechService::program(
            NAME,
            "Speech to text on this host's own processor, in every language the model knows. The program and the model are downloaded once, checked against their publishers' checksums.",
            "The audio goes to your host and is turned into text there by a program Canoa installed: nothing is sent to another service.",
        );
        speech.license = Some("MIT".to_owned());
        info.speech = Some(speech);
        Ok(info)
    }

    fn list(
        &self,
        query: ModelQuery,
        spawner: Object<dyn ProcessSpawner>,
    ) -> Result<ModelList, ProviderError> {
        drop(spawner);
        let held = query
            .program
            .as_deref()
            .and_then(|program| Path::new(program).parent()?.parent())
            .and_then(held_model);
        let mut list = ModelList::default();
        list.models = MODELS.iter().map(|model| row_of(model, held)).collect();
        Ok(list)
    }

    fn act(
        &self,
        request: ModelAct,
        spawner: Object<dyn ProcessSpawner>,
    ) -> Result<ModelActed, ProviderError> {
        drop((request, spawner));
        Err(unsupported(
            "a whisper.cpp model is installed with the program, as the Store's channel: it has no action of its own",
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
        Ok(report_of(query.program.as_deref(), query.models))
    }
}

/// What the speech offer reports for the copy whose program is `program`, with the models listed when `models`.
fn report_of(program: Option<&str>, models: bool) -> ServiceReport {
    let mut report = ServiceReport::default();
    if models {
        report.models = MODELS
            .iter()
            .map(|model| ServedModel::new(model.id, model.label))
            .collect();
    }
    match program.filter(|program| !program.is_empty()) {
        None => report.unavailable = Some(NOT_INSTALLED.to_owned()),
        Some(program) => match run_of(program) {
            Ok(run) => report.command = Some(run),
            Err(why) => report.unavailable = Some(why),
        },
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::program_file;

    fn copy(model: Option<&str>) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "canoa-whisper-copy-{}-{}",
            std::process::id(),
            model.unwrap_or("none")
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("bin")).expect("bin");
        std::fs::create_dir_all(root.join("models")).expect("models");
        std::fs::write(
            root.join("bin").join(program_file(std::env::consts::OS)),
            b"p",
        )
        .expect("program");
        if let Some(file) = model {
            std::fs::write(root.join("models").join(file), b"m").expect("model");
        }
        root
    }

    fn program_of(root: &Path) -> String {
        root.join("bin")
            .join(program_file(std::env::consts::OS))
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn the_run_of_a_copy_reads_a_wav_stream_and_names_the_model_the_copy_holds() {
        let root = copy(Some("ggml-small.en.bin"));
        let run = run_of(&program_of(&root)).expect("a run");
        assert_eq!(run.model, "small.en");
        assert_eq!(run.input, "wav");
        let held = root
            .join("models/ggml-small.en.bin")
            .to_string_lossy()
            .into_owned();
        let args: Vec<&str> = run.args.iter().map(String::as_str).collect();
        assert_eq!(args[..2], ["-m", held.as_str()]);
        assert!(args.windows(2).any(|pair| pair == ["-f", "-"]), "{args:?}");
        assert!(args.contains(&"{language_code|auto}"), "{args:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_copy_with_no_model_has_no_run() {
        let root = copy(None);
        let refused = run_of(&program_of(&root)).expect_err("no model");
        assert!(refused.contains("no model"), "{refused}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_service_lists_every_model_and_runs_the_copy_there_is() {
        let none = report_of(None, true);
        assert_eq!(none.models.len(), MODELS.len());
        assert_eq!(none.models[0].id, "base");
        assert!(none.command.is_none());
        assert!(
            none.unavailable
                .as_deref()
                .is_some_and(|why| why.contains("not installed")),
            "{none:?}"
        );
        let root = copy(Some("ggml-tiny.bin"));
        let up = report_of(Some(&program_of(&root)), false);
        assert!(up.models.is_empty(), "the models are listed when asked for");
        assert_eq!(up.command.expect("a command").model, "tiny");
        assert!(up.unavailable.is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_rows_mark_the_model_the_copy_holds() {
        let root = copy(Some("ggml-base.en.bin"));
        let held = held_model(&root);
        let rows: Vec<ModelRow> = MODELS.iter().map(|model| row_of(model, held)).collect();
        let on: Vec<&str> = rows
            .iter()
            .filter(|row| row.on)
            .map(|row| row.id.as_str())
            .collect();
        assert_eq!(on, ["base.en"]);
        assert!(rows.iter().all(|row| row.actions.is_empty()));
        assert!(
            rows.iter()
                .all(|row| row.tags.iter().any(|tag| tag == "speech-to-text"))
        );
        assert!(rows.iter().all(|row| {
            row.facts
                .iter()
                .any(|fact| fact.label == "Runtime" && fact.value == crate::NAME)
        }));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_description_is_a_program_and_passes_its_own_check() {
        let info = WhisperModels.describe().expect("described");
        assert!(info.problem().is_none(), "{:?}", info.problem());
        let speech = info.speech.expect("a speech service");
        assert!(speech.command && speech.route.is_empty());
        assert_eq!(speech.license.as_deref(), Some("MIT"));
    }
}
