//! Profile layers, loadouts, presets, overlay options, skill pins, and TOML-to-JSON helpers.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use std::path::Path;

use indexmap::IndexMap;

use super::sections::{directory_of, invalid};
use super::{
    AgentProfile, ConfigError, HostProfile, Loadout, LocalProfiles, MAX_NAME, McpChoice, Preset,
    Settings, cache_lifetime_named, check_name,
};
use crate::profile::{LayerKind, Profile};

/// A TOML table as JSON. A date, which JSON has no type for, and a float that is not a number are refused.
pub(super) fn settings_of(name: &str, table: toml::Table) -> Result<Settings, ConfigError> {
    object_of(table).map(Settings).ok_or_else(|| {
        invalid(
            name,
            "a setting holds a date, or a float that is not a number, which a provider cannot be given",
        )
    })
}

/// A `[host.profiles]` table: `overlays` and `allow_mcp` taken out, the rest a profile of the person's own layer. Errors
/// name the profile and a setting, never a value.
pub(super) fn profile_of(
    name: &str,
    mut table: toml::Table,
    directory: &Path,
) -> Result<HostProfile, ConfigError> {
    let refused = |what: String| ConfigError::Invalid {
        entry: None,
        what: format!("profile {name}: {what}"),
    };
    if check_name(name).is_err() {
        return Err(ConfigError::Invalid {
            entry: None,
            what: format!(
                "a profile name has 1 to {MAX_NAME} letters, digits, hyphens or underscores"
            ),
        });
    }
    let overlays = match table.remove("overlays") {
        None => Vec::new(),
        Some(toml::Value::Array(items)) => items
            .into_iter()
            .map(|item| match item {
                toml::Value::String(text) if coqui_aos::valid_overlay_name(&text) => Ok(text),
                _ => Err(refused(
                    "overlays holds a name that is not an overlay's".to_owned(),
                )),
            })
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => return Err(refused("overlays has the wrong type".to_owned())),
    };
    let mut allow_mcp = BTreeMap::new();
    match table.remove("allow_mcp") {
        None => {}
        Some(toml::Value::Table(allowed)) => {
            for (server, digest) in allowed {
                let digest = match digest {
                    toml::Value::String(text) if is_digest(&text) => text,
                    _ => {
                        return Err(refused(format!(
                            "allow_mcp.{server} is not sha256: and 64 hex digits"
                        )));
                    }
                };
                if check_name(&server).is_err() {
                    return Err(refused(
                        "allow_mcp names a server that is not a server's name".to_owned(),
                    ));
                }
                allow_mcp.insert(server, digest);
            }
        }
        Some(_) => return Err(refused("allow_mcp has the wrong type".to_owned())),
    }
    let allow_hooks = match table.remove("allow_hooks") {
        None => BTreeMap::new(),
        Some(toml::Value::Table(allowed)) => {
            hook_allowances(allowed).map_err(|what| refused(format!("allow_hooks{what}")))?
        }
        Some(_) => return Err(refused("allow_hooks has the wrong type".to_owned())),
    };
    let profile =
        Profile::from_table(table, LayerKind::User).map_err(|error| refused(error.to_string()))?;
    Ok(HostProfile {
        profile,
        overlays,
        allow_mcp,
        allow_hooks,
        agent: None,
        off_mcp: BTreeSet::new(),
        directory: directory.to_owned(),
        skill_pins: BTreeMap::new(),
        options: BTreeMap::new(),
        kb_choice: BTreeMap::new(),
        chosen_skills: Vec::new(),
        loadouts: IndexMap::new(),
        default_loadout: None,
        client_loadouts: BTreeMap::new(),
        presets: IndexMap::new(),
        items: crate::profile::ItemSet::default(),
        session: None,
        protected: None,
    })
}

/// The top-level `[profile]`, `[overlays]` and each agent's `profile` and `mcp`. Errors name a table and a setting, never a value.
pub(super) fn local_profiles_of(
    profile: Option<toml::Table>,
    overlays: &IndexMap<String, toml::Value>,
    agents: &IndexMap<String, toml::Value>,
    base: &Path,
) -> Result<LocalProfiles, ConfigError> {
    let invalid = |what: String| ConfigError::Invalid { entry: None, what };
    let directory = directory_of(base);
    let profile = profile
        .map(|table| {
            Profile::from_table(table, LayerKind::User)
                .map_err(|error| invalid(format!("profile: {error}")))
        })
        .transpose()?;
    let mut names = Vec::new();
    for name in overlays.keys() {
        if !coqui_aos::valid_overlay_name(name) {
            return Err(invalid(
                "overlays holds a name that is not an overlay's".to_owned(),
            ));
        }
        names.push(name.clone());
    }
    let mut by_agent = IndexMap::new();
    for (label, table) in agents {
        let Some(table) = table.as_table() else {
            continue;
        };
        let mut layer = AgentProfile::default();
        match table.get("profile") {
            None => {}
            Some(toml::Value::Table(own)) => {
                layer.profile = Some(
                    Profile::from_table(own.clone(), LayerKind::Agent)
                        .map_err(|error| invalid(format!("agents.{label}.profile: {error}")))?,
                );
            }
            Some(_) => return Err(invalid(format!("agents.{label}.profile is not a table"))),
        }
        match table.get("mcp") {
            None => {}
            Some(toml::Value::Table(choices)) => {
                for (server, choice) in choices {
                    let choice = match choice {
                        toml::Value::Boolean(true) => McpChoice::On,
                        toml::Value::Boolean(false) => McpChoice::Off,
                        toml::Value::String(digest) if is_digest(digest) => {
                            McpChoice::At(digest.clone())
                        }
                        _ => {
                            return Err(invalid(format!(
                                "agents.{label}.mcp.{server} is true, false or a sha256: digest"
                            )));
                        }
                    };
                    layer.mcp.insert(server.clone(), choice);
                }
            }
            Some(_) => return Err(invalid(format!("agents.{label}.mcp is not a table"))),
        }
        match table.get("hooks") {
            None => {}
            Some(toml::Value::Table(allowed)) => {
                layer.hooks = hook_allowances(allowed.clone())
                    .map_err(|what| invalid(format!("agents.{label}.hooks{what}")))?;
            }
            Some(_) => return Err(invalid(format!("agents.{label}.hooks is not a table"))),
        }
        if layer.profile.is_some() || !layer.mcp.is_empty() || !layer.hooks.is_empty() {
            by_agent.insert(label.clone(), layer);
        }
    }
    Ok(LocalProfiles {
        profile,
        overlays: names,
        agents: by_agent,
        directory,
        skill_pins: BTreeMap::new(),
        options: BTreeMap::new(),
        kb_choice: BTreeMap::new(),
        loadouts: IndexMap::new(),
        default_loadout: None,
        client_loadouts: BTreeMap::new(),
        presets: IndexMap::new(),
        protected: None,
    })
}

/// A table of overlay names and the tree digests their hooks are allowed at. The error completes a setting's name and never
/// quotes a value.
fn hook_allowances(allowed: toml::Table) -> Result<BTreeMap<String, String>, String> {
    let mut out = BTreeMap::new();
    for (overlay, digest) in allowed {
        if !coqui_aos::valid_overlay_name(&overlay) {
            return Err(" names something that is not an overlay's name".to_owned());
        }
        match digest {
            toml::Value::String(text) if is_digest(&text) => {
                out.insert(overlay, text);
            }
            _ => return Err(format!(".{overlay} is not sha256: and 64 hex digits")),
        }
    }
    Ok(out)
}

/// `[loadouts.NAME]`: each a description and lists of overlay, skill and server names. Errors name the table and the key, never
/// a value.
pub(super) fn loadouts_of(
    raw: &IndexMap<String, toml::Value>,
) -> Result<IndexMap<String, Loadout>, ConfigError> {
    let invalid = |what: String| ConfigError::Invalid { entry: None, what };
    let mut out = IndexMap::new();
    for (name, table) in raw {
        if !crate::profile::entry_name(name) {
            return Err(invalid(
                "loadouts holds a name of more than letters, digits, - and _, or of over 64 characters".to_owned(),
            ));
        }
        let toml::Value::Table(table) = table else {
            return Err(invalid(format!("loadouts.{name} is not a table")));
        };
        let mut loadout = Loadout::default();
        for (key, value) in table {
            let refused = |what: &str| invalid(format!("loadouts.{name}.{key} {what}"));
            match key.as_str() {
                "description" => {
                    let toml::Value::String(text) = value else {
                        return Err(refused("is not text"));
                    };
                    loadout.description.clone_from(text);
                }
                "overlays" | "skills" | "mcp" => {
                    let toml::Value::Array(items) = value else {
                        return Err(refused("is not a list of names"));
                    };
                    let mut names: Vec<String> = Vec::new();
                    for item in items {
                        let toml::Value::String(item) = item else {
                            return Err(refused("is not a list of names"));
                        };
                        let fits = match key.as_str() {
                            "overlays" => coqui_aos::valid_overlay_name(item),
                            "skills" => crate::profile::hosted::fetched::confined(item),
                            _ => crate::profile::entry_name(item),
                        };
                        if !fits {
                            return Err(refused("names something that is not a name of its kind"));
                        }
                        if !names.iter().any(|held| held.eq_ignore_ascii_case(item)) {
                            names.push(item.clone());
                        }
                    }
                    match key.as_str() {
                        "overlays" => loadout.overlays = names,
                        "skills" => loadout.skills = names,
                        _ => loadout.mcp = names,
                    }
                }
                _ => {
                    return Err(refused(
                        "is not a loadout setting: description, overlays, skills and mcp are",
                    ));
                }
            }
        }
        out.insert(name.clone(), loadout);
    }
    Ok(out)
}

/// Whether `name` is one a profile may have: 1 to 64 of ASCII letters, digits, spaces, `-`, `_` and `.`, with no space at either
/// end. A profile is chosen by its name in a footer, so it may read as words.
fn preset_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with(' ')
        && !name.ends_with(' ')
        && name
            .chars()
            .all(|each| each.is_ascii_alphanumeric() || matches!(each, ' ' | '-' | '_' | '.'))
}

/// `[profiles.NAME]`: each a source, model, effort, mode, starting instructions, cache lifetime and loadout, all optional. Errors
/// name the table and the key, never a value. A loadout the file no longer holds is not refused here: a session that names the
/// preset starts with the default loadout and says so.
pub(super) fn presets_of(
    raw: &IndexMap<String, toml::Value>,
) -> Result<IndexMap<String, Preset>, ConfigError> {
    let invalid = |what: String| ConfigError::Invalid { entry: None, what };
    let mut out = IndexMap::new();
    for (name, table) in raw {
        if !preset_name(name) {
            return Err(invalid(
                "profiles holds a name of more than letters, digits, spaces, - _ and ., or one that begins or ends with a space, or of over 64 characters".to_owned(),
            ));
        }
        let toml::Value::Table(table) = table else {
            return Err(invalid(format!("profiles.{name} is not a table")));
        };
        let mut preset = Preset::default();
        let mut scalars = toml::Table::new();
        for (key, value) in table {
            let refused = |what: &str| invalid(format!("profiles.{name}.{key} {what}"));
            if key == "items" {
                let toml::Value::Table(items) = value else {
                    return Err(refused("is not a table of items"));
                };
                for (id, values) in items {
                    let word = !id.is_empty()
                        && id.len() <= 32
                        && id.bytes().all(|byte| {
                            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'
                        });
                    if !word || crate::profile::items::SETTING_IDS.contains(&id.as_str()) {
                        return Err(invalid(format!(
                            "profiles.{name}.items.{id} is not the id of an item a provider adds"
                        )));
                    }
                    let toml::Value::Table(_) = values else {
                        return Err(invalid(format!(
                            "profiles.{name}.items.{id} is not a table"
                        )));
                    };
                    let Ok(serde_json::Value::Object(values)) = serde_json::to_value(values) else {
                        return Err(invalid(format!(
                            "profiles.{name}.items.{id} cannot be read"
                        )));
                    };
                    preset.items.insert(id.clone(), values);
                }
                continue;
            }
            if key == "access" {
                let mut only = toml::Table::new();
                only.insert(key.clone(), value.clone());
                let own = Profile::from_table(only, LayerKind::Session)
                    .map_err(|error| invalid(format!("profiles.{name}: {error}")))?;
                preset.access = own.access;
                continue;
            }
            let toml::Value::String(text) = value else {
                return Err(refused("is not text"));
            };
            if text.is_empty() || text.contains('\0') {
                return Err(refused("is empty or holds a NUL"));
            }
            match key.as_str() {
                "source" => preset.source = Some(text.clone()),
                "model" | "effort" | "mode" => {
                    scalars.insert(key.clone(), value.clone());
                }
                "context" => preset.context = Some(text.clone()),
                "cache_lifetime" => {
                    if cache_lifetime_named(text).is_none() {
                        return Err(refused("is not default, off, short or long"));
                    }
                    preset.cache_lifetime = Some(text.clone());
                }
                "loadout" => {
                    if !crate::profile::entry_name(text) {
                        return Err(refused("is not a loadout's name"));
                    }
                    preset.loadout = Some(text.clone());
                }
                _ => {
                    return Err(refused(
                        "is not a profile setting: source, model, effort, mode, access, context, cache_lifetime, loadout and items are",
                    ));
                }
            }
        }
        let own = Profile::from_table(scalars, LayerKind::Session)
            .map_err(|error| invalid(format!("profiles.{name}: {error}")))?;
        preset.model = own.model;
        preset.effort = own.effort;
        preset.mode = own.mode;
        out.insert(name.clone(), preset);
    }
    Ok(out)
}

/// The kinds of client that can tell the host which it is, in `initialize`'s `_meta`, and so have a default loadout of their own in
/// `[defaults.clients]`: a browser's side panel, and the Outlook, Word, Excel and PowerPoint task panes.
pub const CLIENT_KINDS: [&str; 5] = ["browser", "outlook", "word", "excel", "powerpoint"];

/// `[defaults.clients]`: the loadout a session whose client is of each kind starts with when it names none. A kind that is not
/// one of [`CLIENT_KINDS`], and a value that is not text, refuse the file. A name `[loadouts]` no longer holds (deleted by hand) does
/// not: a connection of that kind says so in its report and gets the default.
pub(super) fn client_loadouts_of(
    defaults: Option<&toml::Table>,
) -> Result<BTreeMap<String, String>, ConfigError> {
    let invalid = |what: String| ConfigError::Invalid { entry: None, what };
    let Some(value) = defaults.and_then(|table| table.get("clients")) else {
        return Ok(BTreeMap::new());
    };
    let table = value
        .as_table()
        .ok_or_else(|| invalid("[defaults.clients] is a table".to_owned()))?;
    let mut out = BTreeMap::new();
    for (kind, value) in table {
        if !CLIENT_KINDS.contains(&kind.as_str()) {
            return Err(invalid(format!(
                "[defaults.clients] {kind} is not a kind of client: {}",
                CLIENT_KINDS.join(", ")
            )));
        }
        let named = value.as_str().ok_or_else(|| {
            invalid(format!(
                "[defaults.clients] {kind} is the name of a loadout"
            ))
        })?;
        out.insert(kind.clone(), named.to_owned());
    }
    Ok(out)
}

/// `[defaults] loadout`: the loadout a session with none named starts with. The rest of the table is the app's.
pub(super) fn default_loadout_of(
    defaults: Option<&toml::Table>,
    loadouts: &IndexMap<String, Loadout>,
) -> Result<Option<String>, ConfigError> {
    let Some(value) = defaults.and_then(|table| table.get("loadout")) else {
        return Ok(None);
    };
    let named = value
        .as_str()
        .filter(|name| loadouts.contains_key(*name))
        .ok_or_else(|| ConfigError::Invalid {
            entry: None,
            what: "[defaults] loadout is not the name of a loadout in [loadouts]".to_owned(),
        })?;
    Ok(Some(named.to_owned()))
}

/// `[skill_pins]`: a skill's name (compared without regard to ASCII case) and the `sha256:` tree digest it must have.
pub(super) fn skill_pins_of(table: toml::Table) -> Result<BTreeMap<String, String>, ConfigError> {
    let refused = |what: &str| ConfigError::Invalid {
        entry: None,
        what: format!("[skill_pins] {what}"),
    };
    let mut out = BTreeMap::new();
    for (name, value) in table {
        if !crate::profile::hosted::fetched::confined(&name) {
            return Err(refused("names something that is not a skill's name"));
        }
        let toml::Value::String(text) = value else {
            return Err(refused("holds a value that is not a digest"));
        };
        let Ok(digest) = coqui_aos::Digest::parse(&text) else {
            return Err(refused(
                "holds a value that is not sha256: and 64 hex digits",
            ));
        };
        if out
            .insert(name.to_ascii_lowercase(), digest.to_string())
            .is_some()
        {
            return Err(refused("names a skill twice, spelt in another case"));
        }
    }
    Ok(out)
}

/// `[overlays.NAME] options`: a table of option names and values, by overlay. Whether an option is declared and of what
/// type is for the overlay's manifest to say, so the errors name a table and never a value.
pub(super) fn overlay_options_of(
    overlays: &IndexMap<String, toml::Value>,
) -> Result<BTreeMap<String, coqui_aos::Document>, ConfigError> {
    let mut out = BTreeMap::new();
    for (name, entry) in overlays {
        let Some(options) = entry.as_table().and_then(|entry| entry.get("options")) else {
            continue;
        };
        let invalid = || ConfigError::Invalid {
            entry: None,
            what: format!("overlays.{name}.options is not a table of values"),
        };
        let toml::Value::Table(table) = options else {
            return Err(invalid());
        };
        let Ok(serde_json::Value::Object(values)) = serde_json::to_value(table) else {
            return Err(invalid());
        };
        out.insert(name.clone(), values);
    }
    Ok(out)
}

/// `[overlays.NAME] kb`: `"index"` serves the overlay's knowledge through the `canoa-kb` tools, `"files"` puts it in the context
/// whole. An overlay with no key is absent: its own `[knowledge] serve` hint decides.
pub(super) fn overlay_kb_of(
    overlays: &IndexMap<String, toml::Value>,
) -> Result<BTreeMap<String, bool>, ConfigError> {
    let mut out = BTreeMap::new();
    for (name, entry) in overlays {
        let Some(choice) = entry.as_table().and_then(|entry| entry.get("kb")) else {
            continue;
        };
        match choice.as_str() {
            Some("index") => {
                out.insert(name.clone(), true);
            }
            Some("files") => {
                out.insert(name.clone(), false);
            }
            _ => {
                return Err(ConfigError::Invalid {
                    entry: None,
                    what: format!("overlays.{name}.kb is \"index\" or \"files\""),
                });
            }
        }
    }
    Ok(out)
}

/// `sha256:` and 64 lower-case hex digits.
fn is_digest(text: &str) -> bool {
    text.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    })
}

/// The source settings read as lists of paths: `dirs` and `manifests` by every Guatiao bridge source, `paths` by Python's,
/// `import_dirs` by the registry. A source may resolve a relative one against whatever working directory it has later, or
/// against its own file's, so each is made absolute here.
const PATH_LISTS: [&str; 5] = ["dirs", "manifests", "paths", "import_dirs", "kinds"];

/// The source settings read as one path: `home`, the Python installation Python's source runs from.
const PATHS: [&str; 1] = ["home"];

pub(super) fn absolute_paths(
    name: &str,
    directory: &Path,
    settings: &mut Settings,
) -> Result<(), ConfigError> {
    let absolute = |text: &mut String| -> Result<(), ConfigError> {
        let path = directory.join(&*text);
        let path = std::path::absolute(&path).unwrap_or(path);
        *text = path
            .to_str()
            .ok_or_else(|| invalid(name, "canoa.cfg's directory is not Unicode, so a relative path cannot be made absolute"))?
            .to_owned();
        Ok(())
    };
    for key in PATH_LISTS {
        let Some(serde_json::Value::Array(items)) = settings.0.get_mut(key) else {
            continue;
        };
        for item in items {
            if let serde_json::Value::String(text) = item {
                absolute(text)?;
            }
        }
    }
    for key in PATHS {
        if let Some(serde_json::Value::String(text)) = settings.0.get_mut(key) {
            absolute(text)?;
        }
    }
    Ok(())
}

fn object_of(table: toml::Table) -> Option<serde_json::Map<String, serde_json::Value>> {
    table
        .into_iter()
        .map(|(key, value)| Some((key, json_of(value)?)))
        .collect()
}

fn json_of(value: toml::Value) -> Option<serde_json::Value> {
    use serde_json::Value as Json;
    Some(match value {
        toml::Value::String(text) => Json::String(text),
        toml::Value::Integer(number) => Json::from(number),
        toml::Value::Float(number) => Json::Number(serde_json::Number::from_f64(number)?),
        toml::Value::Boolean(flag) => Json::Bool(flag),
        toml::Value::Datetime(_) => return None,
        toml::Value::Array(items) => {
            Json::Array(items.into_iter().map(json_of).collect::<Option<_>>()?)
        }
        toml::Value::Table(table) => Json::Object(object_of(table)?),
    })
}
