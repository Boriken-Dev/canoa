//! Piper's voices as a Store page, and the provider that speaks with the installed copy. The host knows none of it: it asks this
//! provider through the `model_store` kind, draws the rows it answers and builds the voice provider it names from the settings
//! `service` reports.
//!
//! The voices the page lists come from the type's catalogs ([`crate::source`]), which the host reads: a voice is its own install, the
//! page's `+` has the host download the voice's two files into the models folder it keeps beside the program's versions, each
//! checked against the digest its catalog gives, and the row's remove takes them away. This provider says what the host cannot:
//! which voices the installed copy speaks with.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "paths are std's"
)]

use std::path::{Path, PathBuf};

use canoa::store::models::{
    ModelAct, ModelActed, ModelKeep, ModelKept, ModelList, ModelQuery, ModelStore, ModelStoreInfo,
    ServiceQuery, ServiceReport,
};
use coqui_acp_api::ProcessSpawner;
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{Provider, Status, ToValue};

use crate::speech::{PROVIDER, PiperSettings};

/// The folder of a copy of the program that holds voices put beside it.
const VOICES_DIR: &str = "voices";

/// What the voice offer says when the program is not installed.
const NOT_INSTALLED: &str = "Piper is not installed: install it in the Store first";

/// The voices of Piper, as the voice the host reads replies aloud with. The page's rows are the catalogs'.
#[derive(Default, Debug, Clone, Copy, Provider)]
#[provider(ModelStore, id = "piper_voices", crate = ::coqui_acp_api::guatiao)]
pub struct PiperModels;

fn unsupported(why: &str) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_UNSUPPORTED, why)
}

/// The copy's folder, from the launch path of its program: the program is in its own folder beside the copy's voices.
fn copy_of(program: &str) -> Option<&Path> {
    Path::new(program).parent()?.parent()
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

    #[test]
    fn the_description_names_the_provider_that_speaks_and_passes_its_own_check() {
        let info = PiperModels.describe().expect("described");
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
}
