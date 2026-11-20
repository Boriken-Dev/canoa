//! Piper's voices as a Store page, and the provider that speaks with the installed copy. The host knows none of it: it asks this
//! provider through the `model_store` kind, draws the rows it answers and builds the voice provider it names from the settings
//! `service` reports.
//!
//! A voice is not moved on its own: the Store installs the program and the voices of a language together, the language being the
//! install's channel, so the page lists what can be chosen and which voices the host's copy holds, and `act` has nothing to do.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "paths are std's"
)]

use std::path::Path;

use canoa::store::models::{
    Fact, ModelAct, ModelActed, ModelKeep, ModelKept, ModelList, ModelQuery, ModelRow, ModelStore,
    ModelStoreInfo, Section, ServiceQuery, ServiceReport,
};
use coqui_acp_api::ProcessSpawner;
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{Provider, Status, ToValue};

use crate::speech::{PROVIDER, PiperSettings};
use crate::store::{Catalog, VOICES_DIR, VoiceFile};

/// What the voice offer says when the program is not installed.
const NOT_INSTALLED: &str = "Piper is not installed: install it in the Store first";

/// The voices of Piper, as a page of the Store and as the voice the host reads replies aloud with.
#[derive(Default, Debug, Clone, Copy, Provider)]
#[provider(ModelStore, id = "piper_voices", crate = ::coqui_acp_api::guatiao)]
pub struct PiperModels;

fn unsupported(why: &str) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_UNSUPPORTED, why)
}

/// The copy's folder, from the launch path of its program: the program is in its own folder beside the voices.
fn copy_of(program: &str) -> Option<&Path> {
    Path::new(program).parent()?.parent()
}

fn megabytes(bytes: u64) -> String {
    format!("{} MB", bytes.div_ceil(1_000_000))
}

fn channel_of(language: &str) -> &'static str {
    match language {
        "en" => "english",
        "es" => "spanish",
        _ => "standard",
    }
}

fn row_of(voice: &VoiceFile, copy: Option<&Path>) -> ModelRow {
    let held = copy.is_some_and(|copy| {
        copy.join(VOICES_DIR)
            .join(format!("{}.onnx", voice.name))
            .is_file()
    });
    let mut row = ModelRow::new(voice.name, format!("A {} voice", voice.language));
    row.on = held;
    row.state = held.then(|| "installed".to_owned());
    row.tags = vec![voice.language.to_owned()];
    row.facts = vec![
        Fact::new("Size", megabytes(voice.onnx.1)),
        Fact::new("Licence", voice.licence),
        Fact::new("SHA-256", voice.onnx.0),
    ];
    let mut how = Section::new("Install it");
    how.paragraphs = vec![
        format!(
            "Install Piper from the Store and choose the channel {}: the Store downloads the program and the voices of that language together, each checked against a digest Canoa measured.",
            channel_of(voice.language)
        ),
        "Replies are then read aloud on this computer, and the text goes nowhere else.".to_owned(),
    ];
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
        let mut list = ModelList::default();
        list.models = Catalog::default()
            .voices
            .iter()
            .map(|voice| row_of(voice, copy))
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
            "a Piper voice is installed with the program, as the Store's channel: it has no action of its own",
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
        Ok(report_of(query.program.as_deref()))
    }
}

/// What the voice offer reports for the copy whose program is `program`: the settings its provider is built from.
fn report_of(program: Option<&str>) -> ServiceReport {
    let mut report = ServiceReport::default();
    let Some(program) = program.filter(|program| !program.is_empty()) else {
        report.unavailable = Some(NOT_INSTALLED.to_owned());
        return report;
    };
    let Some(copy) = copy_of(program) else {
        report.unavailable = Some("the installed copy has an unexpected layout".to_owned());
        return report;
    };
    let settings = PiperSettings::new(program, copy.join(VOICES_DIR).to_string_lossy());
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
    fn the_service_has_no_voice_without_a_copy_and_the_settings_of_the_copy_there_is() {
        let none = report_of(None);
        assert!(none.voice.is_none());
        assert!(
            none.unavailable
                .as_deref()
                .is_some_and(|why| why.contains("not installed")),
            "{none:?}"
        );
        let copy = Path::new("copy").join("piper").join("piper");
        let up = report_of(Some(&copy.to_string_lossy()));
        let value = up.voice.expect("settings");
        let settings = PiperSettings::from_value(&value).expect("the provider's settings");
        assert_eq!(settings.program, copy.to_string_lossy());
        assert_eq!(
            Path::new(&settings.voices_dir),
            Path::new("copy").join("voices")
        );
    }

    #[test]
    fn the_rows_mark_the_voices_the_copy_holds() {
        let root = std::env::temp_dir().join(format!("canoa-piper-copy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(VOICES_DIR)).expect("voices");
        let catalog = Catalog::default();
        let first = catalog.voices.first().expect("a voice").name;
        std::fs::write(root.join(VOICES_DIR).join(format!("{first}.onnx")), b"m").expect("voice");
        let rows: Vec<ModelRow> = catalog
            .voices
            .iter()
            .map(|voice| row_of(voice, Some(&root)))
            .collect();
        let on: Vec<&str> = rows
            .iter()
            .filter(|row| row.on)
            .map(|row| row.id.as_str())
            .collect();
        assert_eq!(on, [first]);
        assert!(rows.iter().all(|row| row.actions.is_empty()));
        let _ = std::fs::remove_dir_all(&root);
    }
}
