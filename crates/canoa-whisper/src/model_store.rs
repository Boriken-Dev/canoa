//! whisper.cpp's models as a Store page, and the speech offer that runs them. The host knows none of it: it asks this provider through
//! the `model_store` kind, draws the rows it answers, and writes the person's speech provider from the command it reports.
//!
//! A model is its own install: the page's `+` has the host download the model's file into the models folder it keeps beside the
//! program's versions, checked against the publisher's SHA-256, and the row's remove takes it away. A copy of the program installed
//! with a model (the release channel names it) keeps that model in its own folder, which the page lists as installed.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "paths are std's"
)]

use std::path::{Path, PathBuf};

use canoa::store::models::{
    Fact, ModelAct, ModelActed, ModelFile, ModelKeep, ModelKept, ModelList, ModelQuery, ModelRow,
    ModelStore, ModelStoreInfo, RowAction, STEP_DELETE_FILES, STEP_FETCH_FILES, Section,
    ServedModel, ServiceQuery, ServiceReport, SpeechRun, SpeechService,
};
use coqui_acp_api::ProcessSpawner;
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{Provider, Status};

use crate::NAME;
use crate::store::{MODELS, MODELS_ORIGIN, Model, model_path};

/// What the speech offer says when the program is not installed.
const NOT_INSTALLED: &str = "whisper.cpp is not installed: install it in the Store first";

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

/// Where a model is installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    /// In the models folder of the host: installed on its own, and the person's to remove.
    Folder,
    /// In the folder of the copy of the program, put there when the program was installed with it.
    Program,
}

/// The folder of one version of the program, from the launch path of its `bin/whisper-cli`.
fn version_of(program: &str) -> Option<&Path> {
    Path::new(program).parent()?.parent()
}

/// Where `model` is installed for the copy whose program is `program` and the models folder `folder`, and its file.
fn installed(
    model: &Model,
    program: Option<&str>,
    folder: Option<&Path>,
) -> Option<(Place, PathBuf)> {
    let kept = program
        .and_then(version_of)
        .map(|version| version.join(model_path(model)))
        .filter(|path| path.is_file());
    if let Some(path) = kept {
        return Some((Place::Program, path));
    }
    let own = folder?.join(model.file);
    own.is_file().then_some((Place::Folder, own))
}

/// How to run the copy whose program is `program`, with the model it is asked for or, when that one is not installed or none is
/// asked for, the first of the models that is: one the copy holds, else one in the models folder `folder`. The program is in `bin`
/// beside the version's `models` folder.
///
/// # Errors
///
/// Why the copy cannot be run: its layout is not the Store's, or no model is installed.
pub fn run_of(
    program: &str,
    folder: Option<&Path>,
    wanted: Option<&str>,
) -> Result<SpeechRun, String> {
    run_over(&MODELS, program, folder, wanted)
}

fn run_over(
    models: &[Model],
    program: &str,
    folder: Option<&Path>,
    wanted: Option<&str>,
) -> Result<SpeechRun, String> {
    version_of(program).ok_or("the installed copy has an unexpected layout")?;
    let pick = |model: &Model| Some((model.id, installed(model, Some(program), folder)?.1));
    let (id, path) = wanted
        .and_then(|id| models.iter().filter(|model| model.id == id).find_map(pick))
        .or_else(|| models.iter().find_map(pick))
        .ok_or(NO_MODEL)?;
    Ok(SpeechRun::new(
        program,
        arguments(&path.to_string_lossy()),
        "wav",
        id,
    ))
}

fn megabytes(bytes: u64) -> String {
    format!("{} MB", bytes.div_ceil(1_000_000))
}

fn row_of(model: &Model, place: Option<Place>, folder_known: bool) -> ModelRow {
    let mut row = ModelRow::new(model.id, model.label);
    row.on = place.is_some();
    row.state = place.map(|place| match place {
        Place::Folder => "installed".to_owned(),
        Place::Program => "installed with the program".to_owned(),
    });
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
        match place {
            None => row.actions.push(RowAction::new(
                format!("Install {}", model.id),
                format!(
                    "Downloads {} ({}) from its publisher and checks it against the publisher's SHA-256. It runs on whisper.cpp, which is installed first when it is not.",
                    model.id,
                    megabytes(model.bytes)
                ),
                "Install",
                [STEP_FETCH_FILES],
            )),
            Some(Place::Folder) => row.actions.push(RowAction::new(
                format!("Remove {}", model.id),
                format!(
                    "Takes the file of {} ({}) off this computer. Installing it again downloads it again.",
                    model.id,
                    megabytes(model.bytes)
                ),
                "Remove",
                [STEP_DELETE_FILES],
            )),
            Some(Place::Program) => {}
        }
    }
    let mut how = Section::new("Install it");
    how.paragraphs = vec![
        format!(
            "The Models page downloads {} on its own, checked against its publisher's checksum, and whisper.cpp runs it: install the program from the Store's Programs list first, or let the page install it with the model.",
            model.id
        ),
        "Dictation then runs on this computer's processor and the audio goes nowhere else. A larger model is slower and uses more memory.".to_owned(),
    ];
    if place == Some(Place::Program) {
        how.paragraphs.push(
            "This copy was installed with the program, in the program's own folder: it goes with the program when that is removed."
                .to_owned(),
        );
    }
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
        let folder = query.folder.as_deref().map(Path::new);
        let mut list = ModelList::default();
        list.models = self
            .models
            .iter()
            .map(|model| {
                let place = installed(model, query.program.as_deref(), folder).map(|found| found.0);
                row_of(model, place, folder.is_some())
            })
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
            query.program.as_deref(),
            query.folder.as_deref().map(Path::new),
            query.model.as_deref(),
            query.models,
        ))
    }
}

/// What the speech offer reports for the copy whose program is `program`, with the models listed when `models`.
fn report_over(
    all: &[Model],
    program: Option<&str>,
    folder: Option<&Path>,
    wanted: Option<&str>,
    models: bool,
) -> ServiceReport {
    let mut report = ServiceReport::default();
    if models {
        report.models = all
            .iter()
            .map(|model| ServedModel::new(model.id, model.label))
            .collect();
    }
    match program.filter(|program| !program.is_empty()) {
        None => report.unavailable = Some(NOT_INSTALLED.to_owned()),
        Some(program) => match run_over(all, program, folder, wanted) {
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

    /// A version's folder with the program, and the model file in its own `models` folder when `model` names one.
    fn copy(name: &str, model: Option<&str>) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("canoa-whisper-copy-{}-{name}", std::process::id()));
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

    fn program_of(root: &Path) -> String {
        root.join("bin")
            .join(program_file(std::env::consts::OS))
            .to_string_lossy()
            .into_owned()
    }

    fn rows(program: Option<&str>, folder: Option<&Path>) -> Vec<ModelRow> {
        MODELS
            .iter()
            .map(|model| {
                let place = installed(model, program, folder).map(|found| found.0);
                row_of(model, place, folder.is_some())
            })
            .collect()
    }

    #[test]
    fn the_run_of_a_copy_reads_a_wav_stream_and_names_the_model_the_copy_holds() {
        let root = copy("held", Some("ggml-small.en.bin"));
        let run = run_of(&program_of(&root), None, None).expect("a run");
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
    fn a_model_installed_on_its_own_is_run_and_the_one_asked_for_wins() {
        let root = copy("own", None);
        let models = folder("own", &["ggml-base.bin", "ggml-tiny.en.bin"]);
        let program = program_of(&root);
        let first = run_of(&program, Some(&models), None).expect("a run");
        assert_eq!(first.model, "base", "the default comes first");
        assert_eq!(
            Path::new(&first.args[1]),
            models.join("ggml-base.bin"),
            "the file in the host's folder"
        );
        let asked = run_of(&program, Some(&models), Some("tiny.en")).expect("a run");
        assert_eq!(asked.model, "tiny.en");
        let missing = run_of(&program, Some(&models), Some("small")).expect("a run");
        assert_eq!(
            missing.model, "base",
            "a model that is not installed is not run"
        );
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&models);
    }

    #[test]
    fn a_copy_with_no_model_anywhere_has_no_run() {
        let root = copy("none", None);
        let models = folder("none", &[]);
        let refused = run_of(&program_of(&root), Some(&models), None).expect_err("no model");
        assert!(refused.contains("no model"), "{refused}");
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&models);
    }

    #[test]
    fn the_service_lists_every_model_and_runs_the_copy_there_is() {
        let none = report_over(&MODELS, None, None, None, true);
        assert_eq!(none.models.len(), MODELS.len());
        assert_eq!(none.models[0].id, "base");
        assert!(none.command.is_none());
        assert!(
            none.unavailable
                .as_deref()
                .is_some_and(|why| why.contains("not installed")),
            "{none:?}"
        );
        let root = copy("service", Some("ggml-tiny.bin"));
        let up = report_over(&MODELS, Some(&program_of(&root)), None, None, false);
        assert!(up.models.is_empty(), "the models are listed when asked for");
        assert_eq!(up.command.expect("a command").model, "tiny");
        assert!(up.unavailable.is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_model_that_is_not_installed_offers_to_be_downloaded_with_its_file_and_digest() {
        let models = folder("offer", &[]);
        let all = rows(None, Some(&models));
        assert_eq!(all.len(), MODELS.len());
        for (row, model) in all.iter().zip(MODELS.iter()) {
            assert!(!row.on, "{}", row.id);
            let [install] = row.actions.as_slice() else {
                panic!("one action: {:?}", row.actions);
            };
            assert_eq!(install.steps, [STEP_FETCH_FILES]);
            assert!(
                install.confirm.contains("whisper.cpp"),
                "{}",
                install.confirm
            );
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
        let all = rows(None, Some(&models));
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
        let _ = std::fs::remove_dir_all(&models);
    }

    #[test]
    fn a_model_installed_with_the_program_is_listed_installed_and_is_not_the_hosts_to_remove() {
        let root = copy("combined", Some("ggml-base.en.bin"));
        let models = folder("combined", &[]);
        let all = rows(Some(&program_of(&root)), Some(&models));
        let row = all.iter().find(|row| row.id == "base.en").expect("a row");
        assert!(row.on);
        assert_eq!(row.state.as_deref(), Some("installed with the program"));
        assert!(row.actions.is_empty(), "{:?}", row.actions);
        assert!(
            all.iter()
                .filter(|each| each.id != "base.en")
                .all(|each| !each.on)
        );
        assert!(
            all.iter()
                .all(|row| row.tags.iter().any(|tag| tag == "speech-to-text"))
        );
        assert!(all.iter().all(|row| {
            row.facts
                .iter()
                .any(|fact| fact.label == "Runtime" && fact.value == crate::NAME)
        }));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&models);
    }

    #[test]
    fn a_host_with_no_folder_lists_models_it_cannot_move() {
        let all = rows(None, None);
        assert!(all.iter().all(|row| row.actions.is_empty()));
    }

    #[test]
    fn the_description_is_a_program_and_passes_its_own_check() {
        let info = WhisperModels::default().describe().expect("described");
        assert!(info.problem().is_none(), "{:?}", info.problem());
        let speech = info.speech.expect("a speech service");
        assert!(speech.command && speech.route.is_empty());
        assert_eq!(speech.license.as_deref(), Some("MIT"));
    }
}
