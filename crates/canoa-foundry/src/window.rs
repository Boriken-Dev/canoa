//! A model's window: how many tokens a prompt and its answer may take together, and a folder of Canoa's own that sets it.
//!
//! An ONNX GenAI model folder says its window in `genai_config.json`: `model.context_length` is what the model was built for, and
//! `search.max_length` is the longest prompt plus answer a session over it allows (it sizes the memory the session keeps for the
//! text so far). Foundry Local's cache is not Canoa's to edit, and a re-download would undo an edit, so a different
//! `max_length` is a folder of Canoa's own: a hard link to each file of the cached model, and its own `genai_config.json`. The
//! weights are not copied, since they are gigabytes: on a filesystem that cannot link them, [`adjust`] refuses and says so.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files are std's"
)]

use alloc::boxed::Box;
use alloc::string::{String, ToString as _};
use alloc::vec::Vec;
use core::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// The file that holds a model's window.
pub const CONFIG: &str = "genai_config.json";

/// The file in an adjusted folder that says what it was made from, so a folder already right is not made again.
pub const MARKER: &str = "canoa-window.json";

/// The largest file that is copied when it cannot be linked; a larger one is a model's weights and is never copied.
const COPY_LIMIT: u64 = 16 * 1024 * 1024;

/// What a model folder's `genai_config.json` says of its window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Window {
    /// `model.context_length`: the most the model was built for, and the most a window can be set to.
    pub context_length: Option<u64>,
    /// `search.max_length`: the most a prompt and its answer take together in a session.
    pub max_length: Option<u64>,
}

impl Window {
    /// The most the window can be set to: the model's own context length, else the window it has now.
    #[must_use]
    pub fn limit(&self) -> Option<u64> {
        self.context_length.or(self.max_length)
    }
}

/// Why a window could not be read or made.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WindowError {
    /// The model folder, or what is in it, could not be read.
    Read(String),
    /// A window of no tokens, or a configuration that names none.
    Invalid(String),
    /// The weights cannot be linked and are too large to copy; the text says which file and what to do.
    NotLinked(String),
    /// The folder could not be written.
    Write(String),
}

impl fmt::Display for WindowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(text) | Self::Invalid(text) | Self::NotLinked(text) | Self::Write(text) => {
                f.write_str(text)
            }
        }
    }
}

impl core::error::Error for WindowError {}

fn read_error(path: &Path, error: impl fmt::Display) -> WindowError {
    WindowError::Read(format!("{}: {error}", path.display()))
}

fn write_error(path: &Path, error: impl fmt::Display) -> WindowError {
    WindowError::Write(format!("{}: {error}", path.display()))
}

fn config_of(folder: &Path) -> Result<Value, WindowError> {
    let path = folder.join(CONFIG);
    let text = fs::read_to_string(&path).map_err(|error| read_error(&path, error))?;
    serde_json::from_str(&text).map_err(|error| read_error(&path, error))
}

fn window_of(config: &Value) -> Window {
    Window {
        context_length: config
            .pointer("/model/context_length")
            .and_then(Value::as_u64),
        max_length: config.pointer("/search/max_length").and_then(Value::as_u64),
    }
}

/// The window a model folder's `genai_config.json` says.
///
/// # Errors
///
/// The file is missing or is not JSON.
pub fn read(folder: &Path) -> Result<Window, WindowError> {
    config_of(folder).map(|config| window_of(&config))
}

/// The folder in Foundry Local's cache that holds the variant `id` (`qwen3-1.7b-generic-gpu:2`), which is
/// `<cache>/<publisher>/<name>-<version>/v<version>`, when the model is downloaded there. `cache` is the cache's `models` folder,
/// `~/.foundry/cache/models` unless the person moved it; the SDK's own `Model::path` is the answer when it is in use.
#[must_use]
pub fn cached_folder(cache: &Path, id: &str) -> Option<PathBuf> {
    let (name, version) = id.rsplit_once(':')?;
    if name.is_empty() || version.is_empty() || !version.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let folder = format!("{name}-{version}");
    let inner = format!("v{version}");
    for publisher in fs::read_dir(cache).ok()?.flatten() {
        let Ok(models) = fs::read_dir(publisher.path()) else {
            continue;
        };
        for model in models.flatten() {
            if model
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case(&folder)
            {
                let path = model.path().join(&inner);
                if path.join(CONFIG).is_file() {
                    return Some(path);
                }
            }
        }
    }
    None
}

/// What [`adjust`] made.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Adjusted {
    /// Canoa's folder: the cached model's files, linked, with its own `genai_config.json`.
    pub folder: PathBuf,
    /// The window that folder sets: what the person asked for, or the model's limit when that was less.
    pub window: u64,
    /// The most the window can be: the model's own context length.
    pub limit: u64,
    /// Said when the window is not what was asked for: the request was above `limit`.
    pub note: Option<String>,
    /// Whether the folder was already made for this model and window, and nothing was written.
    pub reused: bool,
}

/// The name of the folder an adjusted model gets under Canoa's folder: its id with what a path cannot hold made `_`.
#[must_use]
pub fn folder_name(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// The id an adjusted model is registered under, so that the SDK's catalog names it apart from the cached one:
/// `qwen3-1.7b-generic-gpu:2` with a window of 16384 is `canoa-qwen3-1.7b-generic-gpu-w16384:2`.
#[must_use]
pub fn registered_id(id: &str, window: u64) -> String {
    match id.rsplit_once(':') {
        Some((name, version)) => format!("canoa-{name}-w{window}:{version}"),
        None => format!("canoa-{id}-w{window}:1"),
    }
}

/// The window a request for `wanted` tokens gets over a model that allows `limit`, and the note when it is less.
fn capped(wanted: u64, limit: u64) -> (u64, Option<String>) {
    if wanted > limit {
        (
            limit,
            Some(format!(
                "{wanted} tokens is above the model's own context length: the window is {limit} tokens"
            )),
        )
    } else {
        (wanted, None)
    }
}

/// What the person's request comes to for a model that allows `limit` tokens, without making anything: the window and the note
/// that says it was lowered.
///
/// # Errors
///
/// A window of no tokens.
pub fn clamp(wanted: u64, limit: u64) -> Result<(u64, Option<String>), WindowError> {
    if wanted == 0 {
        return Err(WindowError::Invalid(
            "a window is at least 1 token".to_string(),
        ));
    }
    Ok(capped(wanted, limit))
}

/// The `genai_config.json` text of `original` with `search.max_length` set to `window`. Nothing else is changed: in particular
/// `model.context_length` stays the model's own, which is what a window can never go above.
fn with_window(original: &str, window: u64) -> Result<String, WindowError> {
    let mut config: Value = serde_json::from_str(original)
        .map_err(|error| WindowError::Read(format!("{CONFIG}: {error}")))?;
    let search = config
        .as_object_mut()
        .map(|top| top.entry("search").or_insert_with(|| json!({})))
        .and_then(Value::as_object_mut)
        .ok_or_else(|| WindowError::Read(format!("{CONFIG} is not a JSON object")))?;
    search.insert("max_length".to_string(), json!(window));
    serde_json::to_string_pretty(&config).map_err(|error| WindowError::Write(error.to_string()))
}

fn marker(source: &Path, window: u64) -> Value {
    json!({"source": source.to_string_lossy(), "window": window})
}

fn is_made(folder: &Path, wanted: &Value) -> bool {
    fs::read_to_string(folder.join(MARKER))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .is_some_and(|found| &found == wanted)
        && folder.join(CONFIG).is_file()
}

/// Links every file of `from` into `to`, copying only what is small and cannot be linked. `to` must not exist.
fn link_tree(from: &Path, to: &Path, top: bool) -> Result<(), WindowError> {
    fs::create_dir_all(to).map_err(|error| write_error(to, error))?;
    let mut files: Vec<(PathBuf, u64)> = Vec::new();
    for entry in fs::read_dir(from).map_err(|error| read_error(from, error))? {
        let entry = entry.map_err(|error| read_error(from, error))?;
        let path = entry.path();
        let name = entry.file_name();
        if path.is_dir() {
            link_tree(&path, &to.join(&name), false)?;
        } else if !(top
            && (name == CONFIG || name.to_string_lossy() == format!("{CONFIG}.dlverified")))
        {
            let size = fs::metadata(&path).map_or(0, |meta| meta.len());
            files.push((path, size));
        }
    }
    // The large files first: a filesystem that cannot link them is found before anything is copied.
    files.sort_by_key(|(_, size)| core::cmp::Reverse(*size));
    for (path, size) in files {
        let target = to.join(path.file_name().unwrap_or_default());
        if fs::hard_link(&path, &target).is_ok() {
            continue;
        }
        if size > COPY_LIMIT {
            return Err(WindowError::NotLinked(format!(
                "{} ({} MB) cannot be linked into {}, and a model's weights are not copied: keep Canoa's folder on the same drive as Foundry Local's cache, or leave the window as the model has it",
                path.display(),
                size / (1024 * 1024),
                to.display()
            )));
        }
        fs::copy(&path, &target).map_err(|error| write_error(&target, error))?;
    }
    Ok(())
}

/// The file in a mirrored folder that says what it was linked from, so that a folder already right is not made again.
pub const MIRROR_MARKER: &str = "canoa-mirror.json";

struct MarkerRule {
    prefix: String,
    start: &'static str,
    end: &'static str,
}

fn loaded_tool_markers() -> &'static [MarkerRule] {
    static RULES: std::sync::OnceLock<Vec<MarkerRule>> = std::sync::OnceLock::new();
    RULES.get_or_init(|| {
        let Ok(Value::Array(items)) =
            serde_json::from_str::<Value>(include_str!("tool_markers.json"))
        else {
            return Vec::new();
        };
        items
            .into_iter()
            .filter_map(|item| {
                let prefix = item["prefix"].as_str()?.to_ascii_lowercase();
                let start = Box::leak(item["start"].as_str()?.to_owned().into_boxed_str());
                let end = Box::leak(item["end"].as_str()?.to_owned().into_boxed_str());
                Some(MarkerRule { prefix, start, end })
            })
            .collect()
    })
}

/// The tool-call markers to register a model with when its own package publishes none, as `(opening, closing)`: Foundry Local
/// refuses a request that carries tools to a model with no markers ("structured tool calling requires model-published opening and
/// closing tool markers"), though the model writes calls in a fixed form. Loaded from `tool_markers.json`; `None` for
/// every other, which is registered as it is.
#[must_use]
pub fn tool_markers(id: &str) -> Option<(&'static str, &'static str)> {
    let name = id
        .rsplit_once(':')
        .map_or(id, |(name, _)| name)
        .to_ascii_lowercase();
    loaded_tool_markers()
        .iter()
        .find(|rule| name.starts_with(&rule.prefix))
        .map(|rule| (rule.start, rule.end))
}

fn mirror_marker(source: &Path) -> Value {
    let config = fs::metadata(source.join(CONFIG)).ok();
    json!({
        "source": source.to_string_lossy(),
        "config_len": config.as_ref().map(fs::Metadata::len),
        "config_modified": config
            .and_then(|meta| meta.modified().ok())
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|since| since.as_secs()),
    })
}

/// Links the model `id` that Foundry Local downloaded into `from` into `to`, at the same place under it
/// (`<publisher>/<name>-<n>/v<n>`), and returns where. `to` is a models folder of Canoa's own: the SDK keeps its catalog of
/// registered models in the models folder it is given, so a folder of its own keeps Foundry Local's cache as the person left it.
/// The weights are linked and never copied. A folder already linked from the same files is reused.
///
/// # Errors
///
/// The model is not downloaded in `from`, a folder at the place that is not one Canoa linked, the weights cannot be linked
/// (`to` is on another drive than `from`), or the disk refuses.
pub fn mirror(from: &Path, to: &Path, id: &str) -> Result<PathBuf, WindowError> {
    let source = cached_folder(from, id).ok_or_else(|| {
        WindowError::Invalid(format!(
            "{id} is not downloaded in {}: download it with Foundry Local first",
            from.display()
        ))
    })?;
    let relative = source
        .strip_prefix(from)
        .map_err(|error| read_error(&source, error))?;
    let folder = to.join(relative);
    let made = mirror_marker(&source);
    let found = fs::read_to_string(folder.join(MIRROR_MARKER))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok());
    if found.as_ref() == Some(&made) && folder.join(CONFIG).is_file() {
        return Ok(folder);
    }
    if folder.exists() && found.is_none() {
        return Err(WindowError::Write(format!(
            "{} exists and is not a folder Canoa linked: it is left as it is",
            folder.display()
        )));
    }
    let partial = folder.with_extension("partial");
    if partial.exists() {
        fs::remove_dir_all(&partial).map_err(|error| write_error(&partial, error))?;
    }
    let built = link_tree(&source, &partial, false).and_then(|()| {
        let file = partial.join(MIRROR_MARKER);
        fs::write(&file, made.to_string()).map_err(|error| write_error(&file, error))
    });
    if let Err(error) = built {
        let _ = fs::remove_dir_all(&partial);
        return Err(error);
    }
    if folder.exists() {
        fs::remove_dir_all(&folder).map_err(|error| write_error(&folder, error))?;
    }
    fs::rename(&partial, &folder).map_err(|error| write_error(&folder, error))?;
    Ok(folder)
}

/// Makes Canoa's own folder for the model cached at `source`, with a window of `wanted` tokens (capped at the model's own
/// context length), under `base` as [`folder_name`]`(id)`. The cached folder is only read. A folder already made for this
/// source and window is reused.
///
/// # Errors
///
/// A window of no tokens, a `genai_config.json` that cannot be read or names no length, a folder that is not one Canoa made, the
/// weights cannot be linked, or the disk refuses.
pub fn adjust(source: &Path, base: &Path, id: &str, wanted: u64) -> Result<Adjusted, WindowError> {
    if wanted == 0 {
        return Err(WindowError::Invalid(
            "a window is at least 1 token".to_string(),
        ));
    }
    let path = source.join(CONFIG);
    let original = fs::read_to_string(&path).map_err(|error| read_error(&path, error))?;
    let config: Value =
        serde_json::from_str(&original).map_err(|error| read_error(&path, error))?;
    let limit = window_of(&config).limit().ok_or_else(|| {
        WindowError::Invalid(format!(
            "{} names neither model.context_length nor search.max_length",
            path.display()
        ))
    })?;
    let (window, note) = capped(wanted, limit);
    let folder = base.join(folder_name(id));
    let made = marker(source, window);
    if is_made(&folder, &made) {
        return Ok(Adjusted {
            folder,
            window,
            limit,
            note,
            reused: true,
        });
    }
    if folder.exists() && !folder.join(MARKER).is_file() {
        return Err(WindowError::Write(format!(
            "{} exists and is not a folder Canoa made: it is left as it is",
            folder.display()
        )));
    }
    let text = with_window(&original, window)?;
    let partial = base.join(format!("{}.partial", folder_name(id)));
    if partial.exists() {
        fs::remove_dir_all(&partial).map_err(|error| write_error(&partial, error))?;
    }
    fs::create_dir_all(base).map_err(|error| write_error(base, error))?;
    let built = link_tree(source, &partial, true)
        .and_then(|()| {
            let config = partial.join(CONFIG);
            fs::write(&config, text).map_err(|error| write_error(&config, error))
        })
        .and_then(|()| {
            let file = partial.join(MARKER);
            fs::write(&file, made.to_string()).map_err(|error| write_error(&file, error))
        });
    if let Err(error) = built {
        let _ = fs::remove_dir_all(&partial);
        return Err(error);
    }
    if folder.exists() {
        fs::remove_dir_all(&folder).map_err(|error| write_error(&folder, error))?;
    }
    fs::rename(&partial, &folder).map_err(|error| write_error(&folder, error))?;
    Ok(Adjusted {
        folder,
        window,
        limit,
        note,
        reused: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn standin(name: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("canoa-window-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let model = root.join("cache/Microsoft/qwen3-1.7b-generic-gpu-2/v2");
        fs::create_dir_all(&model).expect("a model folder");
        let config = "{\n    \"model\": {\n        \"context_length\": 40960,\n        \"type\": \"qwen3\"\n    },\n    \"search\": {\n        \"do_sample\": true,\n        \"max_length\": 40960,\n        \"top_k\": 20\n    }\n}";
        fs::write(model.join(CONFIG), config).expect("a config");
        fs::write(model.join("model.onnx"), vec![7u8; 4096]).expect("weights");
        fs::write(model.join("tokenizer.json"), "{}").expect("a tokenizer");
        fs::write(model.join("genai_config.json.dlverified"), "stamp").expect("a stamp");
        (root, model)
    }

    #[test]
    fn the_window_is_read_from_the_models_own_configuration() {
        let (root, model) = standin("read");
        let window = read(&model).expect("reads");
        assert_eq!(
            (window.context_length, window.max_length, window.limit()),
            (Some(40960), Some(40960), Some(40960))
        );
        assert!(read(&root).is_err(), "a folder with no configuration");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn the_adjusted_copy_carries_the_window_and_the_original_is_left_byte_for_byte() {
        let (root, model) = standin("adjust");
        let before = fs::read(model.join(CONFIG)).expect("reads");
        let base = root.join("windows");
        let made = adjust(&model, &base, "qwen3-1.7b-generic-gpu:2", 16384).expect("adjusted");
        assert_eq!(
            (made.window, made.limit, made.note.clone()),
            (16384, 40960, None)
        );
        assert!(!made.reused);
        let own = read(&made.folder).expect("reads the copy");
        assert_eq!(own.max_length, Some(16384));
        assert_eq!(
            own.context_length,
            Some(40960),
            "the model's own limit is not changed"
        );
        let copied: Value =
            serde_json::from_str(&fs::read_to_string(made.folder.join(CONFIG)).expect("reads"))
                .expect("JSON");
        assert_eq!(copied["search"]["top_k"], 20, "the other settings are kept");
        assert_eq!(copied["model"]["type"], "qwen3");
        assert_eq!(
            fs::read(model.join(CONFIG)).expect("reads"),
            before,
            "the cached configuration is byte-identical"
        );
        assert_eq!(
            fs::read(made.folder.join("model.onnx")).expect("reads"),
            vec![7u8; 4096]
        );
        assert!(made.folder.join("tokenizer.json").is_file());
        assert!(
            !made.folder.join("genai_config.json.dlverified").exists(),
            "a stamp of the cached file does not describe the new one"
        );
        // Writing through the folder's weights would write the cache's: they are the same file.
        fs::write(model.join("model.onnx"), vec![9u8; 4096]).expect("rewritten");
        assert_eq!(
            fs::read(made.folder.join("model.onnx")).expect("reads"),
            vec![9u8; 4096],
            "the weights are linked, not copied"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_window_above_the_models_context_length_is_capped_with_a_note() {
        let (root, model) = standin("cap");
        let made = adjust(&model, &root.join("windows"), "m:2", 100_000).expect("adjusted");
        assert_eq!(made.window, 40960);
        let note = made.note.expect("a note");
        assert!(note.contains("100000") && note.contains("40960"), "{note}");
        assert_eq!(
            read(&made.folder).expect("reads").context_length,
            Some(40960),
            "the context length is never raised"
        );
        assert!(adjust(&model, &root.join("windows"), "m:2", 0).is_err());
        assert_eq!(clamp(5, 3).expect("clamps").0, 3);
        assert!(clamp(0, 3).is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_folder_already_made_for_the_window_is_reused_and_another_window_remakes_it() {
        let (root, model) = standin("reuse");
        let base = root.join("windows");
        let first = adjust(&model, &base, "m:2", 8192).expect("adjusted");
        let again = adjust(&model, &base, "m:2", 8192).expect("adjusted");
        assert!(!first.reused && again.reused);
        let other = adjust(&model, &base, "m:2", 4096).expect("adjusted");
        assert!(!other.reused);
        assert_eq!(other.folder, first.folder, "one folder for each model");
        assert_eq!(read(&other.folder).expect("reads").max_length, Some(4096));
        assert!(!base.join("m_2.partial").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_folder_that_is_not_canoas_is_left_alone() {
        let (root, model) = standin("foreign");
        let base = root.join("windows");
        let theirs = base.join("m_2");
        fs::create_dir_all(&theirs).expect("a folder");
        fs::write(theirs.join("notes.txt"), "mine").expect("a file");
        let refused = adjust(&model, &base, "m:2", 8192).expect_err("refused");
        assert!(
            refused.to_string().contains("not a folder Canoa made"),
            "{refused}"
        );
        assert_eq!(
            fs::read_to_string(theirs.join("notes.txt")).expect("reads"),
            "mine"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_configuration_that_names_no_length_is_refused() {
        let (root, model) = standin("none");
        fs::write(model.join(CONFIG), "{\"model\": {}}").expect("a config");
        let refused = adjust(&model, &root.join("windows"), "m:2", 8192).expect_err("refused");
        assert!(matches!(refused, WindowError::Invalid(_)), "{refused}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn the_registered_id_names_the_window_and_keeps_the_version() {
        assert_eq!(
            registered_id("qwen3-1.7b-generic-gpu:2", 16384),
            "canoa-qwen3-1.7b-generic-gpu-w16384:2"
        );
        assert_eq!(registered_id("mine", 4096), "canoa-mine-w4096:1");
        assert_eq!(folder_name("a:b/c"), "a_b_c");
    }

    #[test]
    fn the_cached_folder_of_a_variant_is_found_by_its_id() {
        let (root, model) = standin("cache");
        let cache = root.join("cache");
        assert_eq!(
            cached_folder(&cache, "qwen3-1.7b-generic-gpu:2"),
            Some(model)
        );
        assert!(cached_folder(&cache, "Qwen3-1.7B-generic-gpu:2").is_some());
        assert_eq!(cached_folder(&cache, "qwen3-1.7b-generic-gpu:3"), None);
        assert_eq!(cached_folder(&cache, "qwen3-1.7b-generic-gpu"), None);
        assert_eq!(cached_folder(&root.join("missing"), "a:1"), None);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_model_is_linked_into_canoas_own_models_folder_and_foundrys_cache_is_not_written() {
        let (root, model) = standin("mirror");
        let cache = root.join("cache");
        let own = root.join("own");
        let before = fs::read(model.join(CONFIG)).expect("reads");
        let linked = mirror(&cache, &own, "qwen3-1.7b-generic-gpu:2").expect("linked");
        assert_eq!(
            linked,
            own.join("Microsoft/qwen3-1.7b-generic-gpu-2/v2"),
            "the same place under the folder"
        );
        for file in [
            CONFIG,
            "model.onnx",
            "tokenizer.json",
            "genai_config.json.dlverified",
        ] {
            assert_eq!(
                fs::read(linked.join(file)).expect("linked"),
                fs::read(model.join(file)).expect("original"),
                "{file}"
            );
        }
        assert_eq!(fs::read(model.join(CONFIG)).expect("reads"), before);
        assert!(
            !model.join(MIRROR_MARKER).exists(),
            "nothing was added to the cache"
        );
        // Linked again, it is the same folder, left alone.
        fs::write(linked.join("kept.txt"), "k").expect("a file");
        assert_eq!(
            mirror(&cache, &own, "qwen3-1.7b-generic-gpu:2").expect("reused"),
            linked
        );
        assert!(linked.join("kept.txt").exists(), "not made again");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_folder_canoa_did_not_link_and_a_model_that_is_not_downloaded_are_refused() {
        let (root, _) = standin("mirror_refused");
        let cache = root.join("cache");
        let own = root.join("own");
        let theirs = own.join("Microsoft/qwen3-1.7b-generic-gpu-2/v2");
        fs::create_dir_all(&theirs).expect("a folder");
        fs::write(theirs.join("mine.txt"), "mine").expect("a file");
        let refused = mirror(&cache, &own, "qwen3-1.7b-generic-gpu:2").expect_err("refused");
        assert!(matches!(refused, WindowError::Write(_)), "{refused}");
        assert!(theirs.join("mine.txt").exists(), "left as it was");
        let missing = mirror(&cache, &own, "other-model:1").expect_err("not downloaded");
        assert!(matches!(missing, WindowError::Invalid(_)), "{missing}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn the_markers_are_known_for_the_qwen_3_5_family_and_for_no_other_model() {
        assert_eq!(
            tool_markers("qwen3.5-9b-generic-gpu:4"),
            Some(("<tool_call>", "</tool_call>"))
        );
        assert_eq!(
            tool_markers("Qwen3.5-27b-generic-gpu:1").map(|m| m.0),
            Some("<tool_call>")
        );
        assert_eq!(tool_markers("qwen3-14b-generic-gpu:2"), None);
        assert_eq!(tool_markers("gpt-oss-20b-generic-gpu:1"), None);
    }
}
