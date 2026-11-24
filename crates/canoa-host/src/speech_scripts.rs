//! Speech providers that are scripts: the one that ships in the host (a program the person names, written in JavaScript), and the
//! route that lets a table of canoa.cfg (`[speech.NAME]`, `[say.NAME]`) name a library or a source of its own, as an entry of canoa.cfg
//! does, and take its provider from what that registry offers.
//!
//! A table that names `library` (a Guatiao library file) or `source` (a Guatiao source file, or `bundled:js` for the JavaScript
//! source built into the host, with `source_settings` and `import_search`) gets its `provider` from that origin; one that names
//! neither gets it from the kinds the host carries, then from the script providers that ship with it.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files and locks are std's"
)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use coqui_acp_api::guatiao::library::Registry;
use coqui_acp_api::guatiao::library::kind::Kind;
use serde_json::{Map, Value};

use crate::config::{ImportSearch, Origin, Settings};
use crate::scripts::Bundle;

/// The keys of a table that name where its provider comes from: the host's, never settings of the provider.
pub(crate) const ORIGIN_KEYS: [&str; 4] = ["library", "source", "source_settings", "import_search"];

/// The script providers that ship in the host, with the SDK modules they import.
static BUNDLE: Bundle = Bundle {
    name: "speech",
    libraries: &["speech"],
    files: &[
        (
            "speech/speech.js",
            include_str!("../extensions/speech/speech.js"),
        ),
        (
            "speech/package.json",
            include_str!("../extensions/speech/package.json"),
        ),
        (
            "speech/canoa_speech.guatiao",
            include_str!("../extensions/speech/canoa_speech.guatiao"),
        ),
        (
            "speech/sdk/speech.js",
            include_str!("../extensions/sdk/speech.js"),
        ),
        (
            "speech/sdk/kinds.js",
            include_str!("../extensions/sdk/kinds.js"),
        ),
        (
            "speech/sdk/model.js",
            include_str!("../extensions/sdk/model.js"),
        ),
        (
            "speech/sdk/wait.js",
            include_str!("../extensions/sdk/wait.js"),
        ),
    ],
};

/// Writes the bundled files into `home`, as a test that imports the SDK from a library of its own needs them, and answers the
/// folder the SDK's modules are under.
#[cfg(test)]
pub(crate) fn kept_in(home: &Path) -> Result<PathBuf, String> {
    BUNDLE.keep(home).map(|folder| folder.join("speech/sdk"))
}

/// Process-wide cache of the bundled speech JS provider registry keyed by unpacked root path.
static BUNDLED: Mutex<Option<(PathBuf, Arc<Registry>)>> = Mutex::new(None);

/// The registry of the script providers that ship in the host, made once for the home they are kept in.
///
/// # Errors
///
/// Why they cannot be offered: their files, or the source.
pub(crate) fn bundled() -> Result<Arc<Registry>, String> {
    let root = crate::scripts::root();
    let mut held = BUNDLED.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((at, registry)) = held.as_ref()
        && *at == root
    {
        return Ok(Arc::clone(registry));
    }
    let registry = Arc::new(BUNDLE.registry(&root)?);
    *held = Some((root, Arc::clone(&registry)));
    Ok(registry)
}

/// Where the provider of `table` is loaded from: `None` when it names no library and no source. A relative path is taken from `base`,
/// canoa.cfg's directory.
///
/// # Errors
///
/// Why the table's keys cannot say an origin.
pub(crate) fn origin_of(table: &Value, base: &Path) -> Result<Option<Origin>, String> {
    let text = |key: &str| -> Result<Option<&str>, String> {
        match table.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(text)) if !text.is_empty() => Ok(Some(text.as_str())),
            Some(Value::String(_)) => Err(format!("{key} must not be empty")),
            Some(_) => Err(format!("{key} is text")),
        }
    };
    let file = |path: &str| {
        let path = base.join(path);
        std::path::absolute(&path).unwrap_or(path)
    };
    let (library, source) = (text("library")?, text("source")?);
    let (settings, search) = (table.get("source_settings"), text("import_search")?);
    match (library, source) {
        (Some(_), Some(_)) => Err("library and source exclude each other".to_owned()),
        (None, None) if settings.is_none() && search.is_none() => Ok(None),
        (None, None) => Err("source_settings and import_search go with source".to_owned()),
        (Some(_), None) if settings.is_some() || search.is_some() => {
            Err("source_settings and import_search go with source, not library".to_owned())
        }
        (Some(library), None) => Ok(Some(Origin::Library(file(library)))),
        (None, Some(source)) => {
            let mut map = match settings {
                None | Some(Value::Null) => Map::new(),
                Some(Value::Object(map)) => map.clone(),
                Some(_) => return Err("source_settings is a table".to_owned()),
            };
            for key in ["dirs", "manifests", "paths", "import_dirs", "kinds"] {
                if let Some(Value::Array(items)) = map.get_mut(key) {
                    for item in items {
                        if let Value::String(path) = item {
                            *path = file(path).to_string_lossy().into_owned();
                        }
                    }
                }
            }
            if let Some(Value::String(path)) = map.get_mut("home") {
                *path = file(path).to_string_lossy().into_owned();
            }
            let settings = Settings::new(map);
            let import_search = match search {
                None => ImportSearch::default_for(&settings),
                Some(text) => ImportSearch::named(text)
                    .ok_or_else(|| "import_search is library or path".to_owned())?,
            };
            Ok(Some(Origin::Source {
                // QuickJS is built into the host: a source named so is no file.
                file: if source == crate::provider::BUNDLED_JS {
                    PathBuf::from(source)
                } else {
                    file(source)
                },
                settings,
                import_search,
                import_search_named: search.is_some(),
            }))
        }
    }
}

/// The registry of the library or source `table` names, loaded now; `None` when it names none.
///
/// # Errors
///
/// Why the origin cannot be read, or cannot be loaded.
pub(crate) fn registry_of(table: &Value, base: &Path) -> Result<Option<Arc<Registry>>, String> {
    let Some(origin) = origin_of(table, base)? else {
        return Ok(None);
    };
    let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
    crate::provider::open(&mut registry, &origin).map_err(|error| error.to_string())?;
    Ok(Some(Arc::new(registry)))
}

/// One provider of a kind a registry offers, as a table of canoa.cfg names it.
pub(crate) struct Described {
    /// What the registry files it under.
    pub(crate) key: String,
    /// The id a table's `provider` names.
    pub(crate) id: String,
    /// What to show a person: its display name, and the line its `meta` says.
    pub(crate) title: String,
    pub(crate) description: String,
    /// The schema of its settings and the form that draws them.
    pub(crate) schema: Value,
    pub(crate) form: Value,
}

/// The providers of kind `K` that `registry` offers.
pub(crate) fn described<K: ?Sized + Kind>(registry: &Registry) -> Vec<Described> {
    let json = |value: &coqui_acp_api::guatiao::Value| {
        serde_json::to_value(guatiao_serde::Serializable::from(value)).ok()
    };
    registry
        .offers::<K>()
        .filter(|offer| !crate::disabled::off::<K>(offer.id()))
        .filter_map(|offer| {
            let key = offer.key()?.to_owned();
            let meta = |name: &str| offer.meta().and_then(|meta| meta.get(name)).and_then(json);
            Some(Described {
                key,
                id: offer.id().to_owned(),
                title: offer.display_name().to_owned(),
                description: meta("description")
                    .and_then(|text| text.as_str().map(str::to_owned))
                    .unwrap_or_default(),
                schema: offer
                    .config_schema()
                    .and_then(json)
                    .unwrap_or_else(|| serde_json::json!({"type": "object", "properties": {}})),
                form: meta("form").unwrap_or(Value::Null),
            })
        })
        .collect()
}

/// `settings` as a guatiao value, the shape a provider is built from.
///
/// # Errors
///
/// When the settings cannot be written as one.
pub(crate) fn to_guatiao(settings: &Value) -> Result<coqui_acp_api::guatiao::Value, String> {
    guatiao_serde::adapt::to_value_in(settings, coqui_acp_api::guatiao::Alloc::rust())
        .map_err(|_| "the settings cannot be read".to_owned())
}
