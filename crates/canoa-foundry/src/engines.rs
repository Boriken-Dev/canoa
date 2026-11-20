//! What runs the model of an account: the Foundry Local service the person runs, or the SDK in a process of the host's own.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files and processes are std's"
)]

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::account::{EngineKind, Settings};
use crate::child::{ChildEngine, Launch, Spec};
use crate::{Engine, EngineError};

/// What opens the engine of an account: the real one reads the settings, a test's answers what it is told.
pub(crate) type Engines =
    Arc<dyn Fn(&Settings) -> Result<Arc<dyn Engine>, String> + Send + Sync + 'static>;

/// The engine an account's settings ask for: the service the person runs, or the SDK in a process of its own, started through
/// `launch`. `copies` is the folder the SDK engine makes a model's adjusted copies in (a window the person set, or the tool-call
/// markers a model's package lacks), and `models` the models folder of the host's own that the model is linked into.
pub(crate) fn default_engine(
    settings: &Settings,
    (copies, models): (Option<&Path>, Option<&Path>),
    launch: Option<&Launch>,
) -> Result<Arc<dyn Engine>, String> {
    match settings.engine {
        EngineKind::Service => {
            if let Some(tokens) = settings.window {
                tracing::info!(
                    "{} has a window of {tokens} tokens, which only the sdk engine uses: the service runs the model as Foundry Local cached it",
                    settings.model
                );
            }
            service_engine(settings)
        }
        EngineKind::Sdk => sdk_engine(settings, (copies, models), launch),
    }
}

#[cfg(feature = "daemon")]
fn service_engine(settings: &Settings) -> Result<Arc<dyn Engine>, String> {
    use crate::daemon::{DaemonConfig, DaemonEngine};
    let mut config = DaemonConfig::new(settings.model.clone());
    config.endpoint.clone_from(&settings.endpoint);
    if let Some(seconds) = settings.answer_seconds {
        config.answer = std::time::Duration::from_secs(seconds);
    }
    let home = std::env::home_dir().unwrap_or_default();
    DaemonEngine::open(&config, &home)
        .map(|engine| Arc::new(engine) as Arc<dyn Engine>)
        .map_err(|error| error.to_string())
}

#[cfg(not(feature = "daemon"))]
fn service_engine(_settings: &Settings) -> Result<Arc<dyn Engine>, String> {
    Err("this build runs models through the sdk engine only: the service engine needs canoa-foundry's daemon feature".to_owned())
}

/// The SDK engine, in a process of the host's own. The native library it loads crashes on some models and prompts, and a crash
/// there is the process's and not the host's.
fn sdk_engine(
    settings: &Settings,
    (copies, models): (Option<&Path>, Option<&Path>),
    launch: Option<&Launch>,
) -> Result<Arc<dyn Engine>, String> {
    let launch = launch
        .cloned()
        .filter(|launch| !launch.helper.is_empty())
        .ok_or_else(|| {
            "this host has no program of its own to run the model's process with".to_owned()
        })?;
    if let Some(tokens) = settings.window
        && copies.is_none()
    {
        return Err(format!(
            "{} has a window of {tokens} tokens, and this host has no folder of its own to keep the adjusted model in",
            settings.model
        ));
    }
    let spec = Spec {
        model: settings.model.clone(),
        library: settings.library.as_ref().map(PathBuf::from),
        cache: None,
        window: settings.window,
        copies: copies.map(Path::to_path_buf),
        own_cache: models.map(Path::to_path_buf),
    };
    // The model lives in that process, so naming this engine is the person's choice to load it there; it is unloaded when the
    // engine goes.
    ChildEngine::open(launch, spec)
        .map(|engine| Arc::new(engine) as Arc<dyn Engine>)
        .map_err(|error| format!("{} could not be loaded: {error}", settings.model))
}

/// What the helper command is started with: the engine's own settings.
#[derive(Debug, Clone, PartialEq, Eq)]
struct HelperSpec {
    model: String,
    library: Option<PathBuf>,
    cache: Option<PathBuf>,
    window: Option<u64>,
    copies: Option<PathBuf>,
    own_cache: Option<PathBuf>,
}

/// Reads the arguments a child engine's command line carries ([`Spec::args`]).
fn parse(args: &[String]) -> Result<HelperSpec, String> {
    let mut model = None;
    let mut spec = HelperSpec {
        model: String::new(),
        library: None,
        cache: None,
        window: None,
        copies: None,
        own_cache: None,
    };
    let mut args = args.iter();
    while let Some(flag) = args.next() {
        let mut value = || {
            args.next()
                .cloned()
                .ok_or_else(|| format!("{flag} needs a value"))
        };
        match flag.as_str() {
            "--model" => model = Some(value()?),
            "--library" => spec.library = Some(PathBuf::from(value()?)),
            "--cache" => spec.cache = Some(PathBuf::from(value()?)),
            "--copies" => spec.copies = Some(PathBuf::from(value()?)),
            "--own-cache" => spec.own_cache = Some(PathBuf::from(value()?)),
            "--window" => {
                spec.window = Some(
                    value()?
                        .parse()
                        .map_err(|_| "--window is a whole number of tokens".to_owned())?,
                );
            }
            other => return Err(format!("{other} is not an argument of the model's process")),
        }
    }
    spec.model = model.ok_or_else(|| "--model names the model to run".to_owned())?;
    if spec.window.is_some() && spec.copies.is_none() {
        return Err("--window needs --copies".to_owned());
    }
    Ok(spec)
}

/// Runs the SDK engine for a host that started this process, until the host closes its input: what the helper command of this
/// library does. `args` are what follows the library's name on its command line. The engine is opened, its model loaded, and the
/// lines of [`crate::worker`] served on standard input and output.
///
/// # Errors
///
/// An argument that is not the process's, or the host's lines could not be written because it is gone.
pub fn run_helper(args: &[String]) -> io::Result<()> {
    let spec = parse(args).map_err(|why| io::Error::new(io::ErrorKind::InvalidInput, why))?;
    serve_engine(spec)
}

#[cfg(feature = "sdk")]
fn serve_engine(spec: HelperSpec) -> io::Result<()> {
    use crate::sdk::{SdkConfig, SdkEngine, SdkWindow};
    let HelperSpec {
        model,
        library,
        cache,
        window,
        copies,
        own_cache,
    } = spec;
    let open = || {
        let mut config = SdkConfig::new(model.clone());
        config.library = library;
        config.cache = cache;
        config.own_cache = own_cache;
        config.copies.clone_from(&copies);
        config.window = window
            .zip(copies)
            .map(|(tokens, folder)| SdkWindow::new(tokens, folder));
        let engine = SdkEngine::open(&config)?;
        for note in engine.notes() {
            tracing::warn!("{note}");
        }
        if let Some(made) = engine.adjusted() {
            tracing::info!(
                "{model} runs with a window of {} tokens from {}",
                made.window,
                made.folder.display()
            );
            if let Some(note) = &made.note {
                tracing::warn!("{model}: {note}");
            }
        }
        engine.load().map_err(|error| {
            EngineError::Failed(format!("{model} could not be loaded: {error}"))
        })?;
        Ok(engine)
    };
    crate::worker::serve(open, io::stdin().lock(), io::stdout())
}

#[cfg(not(feature = "sdk"))]
fn serve_engine(spec: HelperSpec) -> io::Result<()> {
    use crate::{Cancel, ModelInfo};
    /// What stands in for the engine in a build that has none: it is never opened.
    struct Unavailable;
    impl Engine for Unavailable {
        fn model(&self) -> Result<ModelInfo, EngineError> {
            Err(EngineError::Failed(String::new()))
        }
        fn models(&self) -> Vec<ModelInfo> {
            Vec::new()
        }
        fn chat(
            &self,
            _request: &serde_json::Value,
            _cancel: &Cancel,
        ) -> Result<serde_json::Value, EngineError> {
            Err(EngineError::Failed(String::new()))
        }
    }
    let _ = spec;
    crate::worker::serve(
        || {
            Err::<Unavailable, _>(EngineError::Failed(
                "this build runs models through the service only: the sdk engine needs canoa-foundry's sdk feature".to_owned(),
            ))
        },
        io::stdin().lock(),
        io::stdout(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|each| (*each).to_owned()).collect()
    }

    #[test]
    fn the_helper_reads_what_a_child_engines_command_line_carries() {
        let spec = Spec {
            model: "stand-in".to_owned(),
            library: Some(PathBuf::from("lib")),
            cache: Some(PathBuf::from("cache")),
            window: Some(16384),
            copies: Some(PathBuf::from("copies")),
            own_cache: Some(PathBuf::from("own")),
        };
        let read = parse(&spec.args()).expect("reads");
        assert_eq!(read.model, "stand-in");
        assert_eq!(read.library, Some(PathBuf::from("lib")));
        assert_eq!(read.cache, Some(PathBuf::from("cache")));
        assert_eq!(read.window, Some(16384));
        assert_eq!(read.copies, Some(PathBuf::from("copies")));
        assert_eq!(read.own_cache, Some(PathBuf::from("own")));
    }

    #[test]
    fn the_helper_wants_a_model_and_a_folder_for_a_window_and_refuses_what_it_does_not_know() {
        assert!(parse(&args(&[])).unwrap_err().contains("--model"));
        assert!(
            parse(&args(&["--model", "m", "--window", "9"]))
                .unwrap_err()
                .contains("--copies")
        );
        assert!(
            parse(&args(&[
                "--model", "m", "--window", "many", "--copies", "c"
            ]))
            .is_err()
        );
        assert!(parse(&args(&["--model", "m", "--other", "x"])).is_err());
        assert!(parse(&args(&["--model"])).is_err());
        assert!(parse(&args(&["--model", "m"])).is_ok());
    }
}
