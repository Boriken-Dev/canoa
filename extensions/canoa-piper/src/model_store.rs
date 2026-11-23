//! Piper's voices as a Store page, and the provider that speaks with the installed copy. The host knows none of it: it asks this
//! provider through the `model_store` kind, draws the rows it answers and builds the voice provider it names from the settings
//! `service` reports.
//!
//! A voice is its own install: the page's `+` has the host download the voice's two files into the models folder it keeps beside the
//! program's versions, each checked against the digest Canoa measured, and the row's remove takes them away. A copy of the program
//! installed with the voices of a language (the release channel names it) keeps them in its own `voices` folder, which the page
//! lists as installed.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "paths are std's"
)]

use std::path::{Path, PathBuf};

use canoa::store::models::{
    Fact, ModelAct, ModelActed, ModelFile, ModelKeep, ModelKept, ModelList, ModelQuery, ModelRow,
    ModelStore, ModelStoreInfo, RowAction, STEP_DELETE_FILES, STEP_FETCH_FILES, Section,
    ServiceQuery, ServiceReport,
};
use coqui_acp_api::ProcessSpawner;
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{Provider, Status, ToValue};

use crate::speech::{PROVIDER, PiperSettings};
use crate::store::{Catalog, VOICES_DIR, VoiceFile};

/// What the voice offer says when the program is not installed.
const NOT_INSTALLED: &str = "Piper is not installed: install it in the Store first";

/// The voices of Piper, as a page of the Store and as the voice the host reads replies aloud with.
#[derive(Default, Debug, Clone, Provider)]
#[provider(ModelStore, id = "piper_voices", crate = ::coqui_acp_api::guatiao)]
pub struct PiperModels {
    catalog: Catalog,
}

impl PiperModels {
    /// The page over another catalog, which a test serves from a world of its own.
    #[must_use]
    pub const fn over(catalog: Catalog) -> Self {
        Self { catalog }
    }
}

fn unsupported(why: &str) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_UNSUPPORTED, why)
}

/// The copy's folder, from the launch path of its program: the program is in its own folder beside the copy's voices.
fn copy_of(program: &str) -> Option<&Path> {
    Path::new(program).parent()?.parent()
}

fn megabytes(bytes: u64) -> String {
    format!("{} MB", bytes.div_ceil(1_000_000))
}

/// Where a voice is installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    /// In the models folder of the host: installed on its own, and the person's to remove.
    Folder,
    /// In the voices folder of the copy of the program, put there when the program was installed with them.
    Program,
}

fn place_of(voice: &VoiceFile, copy: Option<&Path>, folder: Option<&Path>) -> Option<Place> {
    let model = format!("{}.onnx", voice.name);
    if copy.is_some_and(|copy| copy.join(VOICES_DIR).join(&model).is_file()) {
        return Some(Place::Program);
    }
    folder
        .is_some_and(|folder| folder.join(model).is_file())
        .then_some(Place::Folder)
}

fn row_of(
    catalog: &Catalog,
    voice: &VoiceFile,
    place: Option<Place>,
    folder_known: bool,
) -> ModelRow {
    let mut row = ModelRow::new(voice.name, format!("A {} voice", voice.language));
    row.on = place.is_some();
    row.state = place.map(|place| match place {
        Place::Folder => "installed".to_owned(),
        Place::Program => "installed with the program".to_owned(),
    });
    row.tags = vec![voice.language.to_owned()];
    row.facts = vec![
        Fact::new("Size", megabytes(voice.onnx.1)),
        Fact::new("Licence", voice.licence),
        Fact::new("SHA-256", voice.onnx.0),
    ];
    row.files = catalog
        .files_of(voice)
        .into_iter()
        .map(|file| ModelFile::new(file.url, file.file, Some(file.bytes), file.sha256))
        .collect();
    if folder_known {
        match place {
            None => row.actions.push(RowAction::new(
                format!("Install {}", voice.name),
                format!(
                    "Downloads the voice {} ({}) from its publisher and checks it against the digest Canoa measured; its data is licensed as {}. It is spoken by Piper, which is installed first when it is not.",
                    voice.name,
                    megabytes(voice.onnx.1),
                    voice.licence
                ),
                "Install",
                [STEP_FETCH_FILES],
            )),
            Some(Place::Folder) => row.actions.push(RowAction::new(
                format!("Remove {}", voice.name),
                format!(
                    "Takes the voice {} ({}) off this computer. Installing it again downloads it again.",
                    voice.name,
                    megabytes(voice.onnx.1)
                ),
                "Remove",
                [STEP_DELETE_FILES],
            )),
            Some(Place::Program) => {}
        }
    }
    let mut how = Section::new("Install it");
    how.paragraphs = vec![
        "The Models page downloads this voice on its own, checked against the digest Canoa measured, and Piper speaks with it: install the program from the Store's Programs list first, or let the page install it with the voice.".to_owned(),
        "Replies are then read aloud on this computer, and the text goes nowhere else.".to_owned(),
    ];
    if place == Some(Place::Program) {
        how.paragraphs.push(
            "This voice was installed with the program, in the program's own folder: it goes with the program when that is removed."
                .to_owned(),
        );
    }
    row.sections = vec![how];
    row
}

impl ModelStore for PiperModels {
    fn describe(&self) -> Result<ModelStoreInfo, ProviderError> {
        let mut info = ModelStoreInfo::new(
            "Piper voices",
            "Piper's voices",
            "Rhasspy",
            "Install Piper from the Store to read replies aloud.",
        );
        info.voice = Some(PROVIDER.to_owned());
        Ok(info)
    }

    fn list(
        &self,
        query: ModelQuery,
        spawner: Object<dyn ProcessSpawner>,
    ) -> Result<ModelList, ProviderError> {
        drop(spawner);
        let copy = query.program.as_deref().and_then(copy_of);
        let folder = query.folder.as_deref().map(Path::new);
        let mut list = ModelList::default();
        list.models = self
            .catalog
            .voices
            .iter()
            .map(|voice| {
                row_of(
                    &self.catalog,
                    voice,
                    place_of(voice, copy, folder),
                    folder.is_some(),
                )
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
            "a Piper voice is downloaded and removed by the host: it has no other action",
        ))
    }

    fn keep(
        &self,
        request: ModelKeep,
        spawner: Object<dyn ProcessSpawner>,
    ) -> Result<ModelKept, ProviderError> {
        drop((request, spawner));
        Err(unsupported("a Piper voice keeps no setting"))
    }

    fn service(
        &self,
        query: ServiceQuery,
        spawner: Object<dyn ProcessSpawner>,
    ) -> Result<ServiceReport, ProviderError> {
        drop(spawner);
        Ok(report_of(
            query.program.as_deref(),
            query.folder.as_deref().map(Path::new),
        ))
    }
}

/// The voices folders the provider reads, as one search path: the host's own, then the copy's.
fn voices_dirs(copy: &Path, folder: Option<&Path>) -> Result<String, String> {
    let dirs: Vec<PathBuf> = folder
        .into_iter()
        .map(Path::to_path_buf)
        .chain([copy.join(VOICES_DIR)])
        .collect();
    std::env::join_paths(dirs)
        .map(|joined| joined.to_string_lossy().into_owned())
        .map_err(|error| error.to_string())
}

/// What the voice offer reports for the copy whose program is `program`: the settings its provider is built from, which read the
/// voices installed on their own and the ones the copy was installed with.
fn report_of(program: Option<&str>, folder: Option<&Path>) -> ServiceReport {
    let mut report = ServiceReport::default();
    let Some(program) = program.filter(|program| !program.is_empty()) else {
        report.unavailable = Some(NOT_INSTALLED.to_owned());
        return report;
    };
    let Some(copy) = copy_of(program) else {
        report.unavailable = Some("the installed copy has an unexpected layout".to_owned());
        return report;
    };
    let voices = match voices_dirs(copy, folder) {
        Ok(voices) => voices,
        Err(why) => {
            report.unavailable = Some(why);
            return report;
        }
    };
    let settings = PiperSettings::new(program, voices);
    match settings.to_value() {
        Ok(value) => report.voice = Some(value),
        Err(error) => report.unavailable = Some(error.to_string()),
    }
    report
}

#[cfg(test)]
mod tests {
    use coqui_acp_api::guatiao::FromValue;

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("canoa-piper-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("a folder");
        root
    }

    fn rows(copy: Option<&Path>, folder: Option<&Path>) -> Vec<ModelRow> {
        let catalog = Catalog::default();
        catalog
            .voices
            .iter()
            .map(|voice| {
                row_of(
                    &catalog,
                    voice,
                    place_of(voice, copy, folder),
                    folder.is_some(),
                )
            })
            .collect()
    }

    #[test]
    fn the_description_names_the_provider_that_speaks_and_passes_its_own_check() {
        let info = PiperModels::default().describe().expect("described");
        assert_eq!(info.problem(), None);
        assert_eq!(info.voice.as_deref(), Some(PROVIDER));
        assert!(info.speech.is_none(), "it is no speech-to-text");
    }

    #[test]
    fn the_service_has_no_voice_without_a_copy_and_reads_both_voices_folders_of_the_copy_there_is()
    {
        let none = report_of(None, None);
        assert!(none.voice.is_none());
        assert!(
            none.unavailable
                .as_deref()
                .is_some_and(|why| why.contains("not installed")),
            "{none:?}"
        );
        let copy = Path::new("copy").join("piper").join("piper");
        let own = Path::new("models");
        let up = report_of(Some(&copy.to_string_lossy()), Some(own));
        let value = up.voice.expect("settings");
        let settings = PiperSettings::from_value(&value).expect("the provider's settings");
        assert_eq!(settings.program, copy.to_string_lossy());
        let dirs: Vec<PathBuf> = std::env::split_paths(&settings.voices_dir).collect();
        assert_eq!(
            dirs,
            [own.to_path_buf(), Path::new("copy").join("voices")],
            "the host's folder, then the copy's"
        );
        let alone = report_of(Some(&copy.to_string_lossy()), None);
        let settings =
            PiperSettings::from_value(&alone.voice.expect("settings")).expect("settings");
        assert_eq!(
            Path::new(&settings.voices_dir),
            Path::new("copy").join("voices")
        );
    }

    #[test]
    fn a_voice_not_installed_offers_its_two_files_and_a_voice_in_the_hosts_folder_offers_its_removal()
     {
        let folder = scratch("own");
        let catalog = Catalog::default();
        let first = catalog.voices.first().expect("a voice");
        let all = rows(None, Some(&folder));
        for (row, voice) in all.iter().zip(&catalog.voices) {
            let [install] = row.actions.as_slice() else {
                panic!("one action: {:?}", row.actions);
            };
            assert_eq!(install.steps, [STEP_FETCH_FILES]);
            let names: Vec<&str> = row.files.iter().map(|file| file.file.as_str()).collect();
            assert_eq!(
                names,
                [
                    format!("{}.onnx", voice.name),
                    format!("{}.onnx.json", voice.name)
                ]
            );
            assert!(row.files.iter().all(|file| file.sha256.len() == 64));
        }
        std::fs::write(folder.join(format!("{}.onnx", first.name)), b"m").expect("voice");
        let after = rows(None, Some(&folder));
        let on: Vec<&str> = after
            .iter()
            .filter(|row| row.on)
            .map(|row| row.id.as_str())
            .collect();
        assert_eq!(on, [first.name]);
        let held = after.iter().find(|row| row.on).expect("a row");
        assert_eq!(held.state.as_deref(), Some("installed"));
        assert_eq!(held.actions[0].steps, [STEP_DELETE_FILES]);
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_voice_installed_with_the_program_is_listed_installed_and_is_not_the_hosts_to_remove() {
        let root = scratch("combined");
        std::fs::create_dir_all(root.join(VOICES_DIR)).expect("voices");
        let folder = scratch("combined-own");
        let catalog = Catalog::default();
        let first = catalog.voices.first().expect("a voice").name;
        std::fs::write(root.join(VOICES_DIR).join(format!("{first}.onnx")), b"m").expect("voice");
        let all = rows(Some(&root), Some(&folder));
        let on: Vec<&str> = all
            .iter()
            .filter(|row| row.on)
            .map(|row| row.id.as_str())
            .collect();
        assert_eq!(on, [first]);
        let held = all.iter().find(|row| row.on).expect("a row");
        assert_eq!(held.state.as_deref(), Some("installed with the program"));
        assert!(held.actions.is_empty(), "{:?}", held.actions);
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_host_with_no_folder_lists_voices_it_cannot_move() {
        assert!(rows(None, None).iter().all(|row| row.actions.is_empty()));
    }
}
