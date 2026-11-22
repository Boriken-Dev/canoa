//! Entry, provider, harness, extension, tool, and transport table parsers.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use coqui_peer_webcrypto::PeerId;
use indexmap::IndexMap;

use super::profiles::{absolute_paths, settings_of};
use super::raw::{RawAsks, RawCommon};
use super::sections::{check_title, invalid, plain_absolute};
use super::{
    ConfigError, DEFAULT_MAX_PEERS, Entry, EntryKind, ExtensionCheck, HarnessOptions, ImportSearch,
    Listen, MAX_NAME, Origin, RemoteAgent, SETTING_SOURCES, Settings, TransportListen,
};
use crate::tool::ToolSpec;

/// The harnesses the host carries: providers of `acp_harness` its bundled agents register in this process.
pub fn built_in_harnesses() -> impl Iterator<Item = &'static str> {
    crate::bundled::harnesses()
}

/// Whether the host itself carries the harness provider `id`.
#[must_use]
pub fn is_built_in_harness(id: &str) -> bool {
    crate::bundled::is_harness(id)
}

/// The keys that say where an outside provider's code is.
pub(super) struct OriginKeys<'a> {
    pub(super) library: Option<&'a str>,
    pub(super) source: Option<&'a str>,
    pub(super) source_settings: Option<toml::Table>,
    pub(super) import_search: Option<&'a str>,
}

/// `pin` with `root`, or `sha256` alone: what the Store recorded of an extension it installed.
pub(super) fn extension_check(
    name: &str,
    base: &Path,
    origin: &Origin,
    pin: Option<String>,
    sha256: Option<String>,
    root: Option<String>,
) -> Result<Option<ExtensionCheck>, ConfigError> {
    let hex = |text: &str| text.len() == 64 && text.bytes().all(|byte| byte.is_ascii_hexdigit());
    match (pin, sha256, root) {
        (None, None, None) => Ok(None),
        (Some(pin), None, Some(root)) => {
            if !pin.strip_prefix("sha256:").is_some_and(hex) {
                return Err(invalid(name, "pin is sha256: and 64 hexadecimal digits"));
            }
            if root.is_empty() {
                return Err(invalid(name, "root must not be empty"));
            }
            let root = base.join(root);
            Ok(Some(ExtensionCheck::Tree {
                root: std::path::absolute(&root).unwrap_or(root),
                pin,
            }))
        }
        (None, Some(sha256), None) => {
            if !hex(&sha256) {
                return Err(invalid(name, "sha256 is 64 hexadecimal digits"));
            }
            if !matches!(origin, Origin::Library(_)) {
                return Err(invalid(name, "sha256 goes with library, not source"));
            }
            Ok(Some(ExtensionCheck::File { sha256 }))
        }
        _ => Err(invalid(
            name,
            "pin goes with root, and sha256 stands alone: an extension has one or the other",
        )),
    }
}

/// Where an entry's provider is loaded from, `None` when the entry names no library or source. Relative paths are taken
/// from `base`, canoa.cfg's directory.
pub(super) fn origin_of(
    name: &str,
    base: &Path,
    directory: &Path,
    keys: OriginKeys<'_>,
) -> Result<Option<Origin>, ConfigError> {
    let file = |path: &str| {
        if path.is_empty() {
            return Err(invalid(name, "library or source must not be empty"));
        }
        let path = base.join(path);
        Ok(std::path::absolute(&path).unwrap_or(path))
    };
    match (keys.library, keys.source, keys.source_settings) {
        (Some(_), Some(_), _) => Err(invalid(name, "library and source exclude each other")),
        (None, None, None) if keys.import_search.is_none() => Ok(None),
        (None, None, _) => Err(invalid(
            name,
            "source_settings and import_search go with source",
        )),
        (Some(_), None, Some(_)) => {
            Err(invalid(name, "source_settings go with source, not library"))
        }
        (Some(_), None, None) if keys.import_search.is_some() => {
            Err(invalid(name, "import_search goes with source, not library"))
        }
        (Some(library), None, None) => Ok(Some(Origin::Library(file(library)?))),
        (None, Some(source), settings) => {
            let mut settings = settings_of(name, settings.unwrap_or_default())?;
            absolute_paths(name, directory, &mut settings)?;
            // QuickJS is built into the host: a source named so is no file.
            let bundled = source == crate::provider::BUNDLED_JS;
            let import_search = match keys.import_search {
                None => ImportSearch::default_for(&settings),
                Some(text) => ImportSearch::named(text)
                    .ok_or_else(|| invalid(name, "import_search is library or path"))?,
            };
            Ok(Some(Origin::Source {
                file: if bundled {
                    PathBuf::from(source)
                } else {
                    file(source)?
                },
                settings,
                import_search,
                import_search_named: keys.import_search.is_some(),
            }))
        }
    }
}

/// The `settings` of a built-in entry: a key the profile's access replaces is refused, since access is the same for every
/// harness, and a relative `executable` is made absolute against canoa.cfg's directory, as the entry's own key is, since the
/// command line runs in a session's directory, which a client chooses.
pub(super) fn built_in_settings(
    name: &str,
    settings: Option<Settings>,
    executable_key: bool,
    directory: &Path,
) -> Result<Option<Settings>, ConfigError> {
    let Some(settings) = settings else {
        return Ok(None);
    };
    let mut map = settings.as_map().clone();
    if executable_key && map.contains_key("executable") {
        return Err(invalid(
            name,
            "executable is set twice: as a key and in settings",
        ));
    }
    for key in [
        "permission_mode",
        "approval_policy",
        "sandbox",
        "access",
        "allow_client_access",
    ] {
        if map.contains_key(key) {
            return Err(invalid(
                name,
                &format!(
                    "settings.{key} is not taken: the access is the profile's (access = {{ sandbox, network, approvals }}), the same for every harness"
                ),
            ));
        }
    }
    match map.get("executable") {
        None => {}
        Some(serde_json::Value::String(path)) if path.is_empty() => {
            return Err(invalid(name, "settings.executable is empty"));
        }
        Some(serde_json::Value::String(path)) => {
            if path.contains(['/', '\\']) && !Path::new(path).is_absolute() {
                let absolute = directory.join(path).to_string_lossy().into_owned();
                map.insert("executable".to_owned(), serde_json::Value::String(absolute));
            }
        }
        Some(_) => return Err(invalid(name, "settings.executable must be text")),
    }
    Ok(Some(Settings::new(map)))
}

/// An entry's own keys, checked against what the provider of `origin` (`None` for bundled) takes, and refused for the others.
/// A key that `settings` also holds is refused: the entry would say it twice.
pub(super) fn harness_options(
    name: &str,
    harness: &str,
    origin: Option<&Origin>,
    agent: &RawAsks,
    settings: Option<&Settings>,
) -> Result<HarnessOptions, ConfigError> {
    if agent.setting_sources.is_some()
        && !crate::vendors::declares(origin, harness, "setting_sources")
    {
        return Err(invalid(
            name,
            &format!("setting_sources is not a setting of harness {harness}"),
        ));
    }
    if agent.setting_sources.is_some()
        && settings.is_some_and(|settings| settings.as_map().contains_key("setting_sources"))
    {
        return Err(invalid(
            name,
            "setting_sources is set twice: as a key and in settings",
        ));
    }
    if let Some(sources) = &agent.setting_sources
        && sources
            .iter()
            .any(|source| !SETTING_SOURCES.contains(&source.as_str()))
    {
        return Err(invalid(
            name,
            &format!("setting_sources holds only {}", SETTING_SOURCES.join(", ")),
        ));
    }
    Ok(HarnessOptions {
        setting_sources: agent.setting_sources.clone(),
        cache_lifetime: None,
    })
}

/// A tool entry: the settings every entry has, taken out by hand since serde does not hold `deny_unknown_fields` across
/// `flatten`, then the rest as the tool's. Errors name a setting, never a value.
pub(super) fn tool_of(
    name: &str,
    base: &Path,
    directory: &Path,
    mut table: toml::Table,
) -> Result<(RawCommon, ToolSpec), ConfigError> {
    let wrong = |key: &str| invalid(name, &format!("{key} has the wrong type"));
    let text = |value: toml::Value, key: &str| match value {
        toml::Value::String(text) => Ok(text),
        _ => Err(wrong(key)),
    };
    let texts = |value: toml::Value, key: &str| match value {
        toml::Value::Array(items) => items.into_iter().map(|item| text(item, key)).collect(),
        _ => Err(wrong(key)),
    };
    let listen = table
        .remove("listen")
        .map(|value| text(value, "listen"))
        .transpose()?;
    let tabled = |value: toml::Value, key: &str| match value {
        toml::Value::Table(table) => Ok(table),
        _ => Err(wrong(key)),
    };
    let transport_id = table
        .remove("transport")
        .map(|value| text(value, "transport"))
        .transpose()?;
    let transport_library = table
        .remove("transport_library")
        .map(|value| text(value, "transport_library"))
        .transpose()?;
    let transport_source = table
        .remove("transport_source")
        .map(|value| text(value, "transport_source"))
        .transpose()?;
    let transport_import_search = table
        .remove("transport_import_search")
        .map(|value| text(value, "transport_import_search"))
        .transpose()?;
    let transport_source_settings = table
        .remove("transport_source_settings")
        .map(|value| tabled(value, "transport_source_settings"))
        .transpose()?;
    let transport_settings = table
        .remove("transport_settings")
        .map(|value| tabled(value, "transport_settings"))
        .transpose()?;
    let transport = transport_of(
        name,
        base,
        directory,
        TransportKeys {
            id: transport_id,
            library: transport_library,
            source: transport_source,
            source_settings: transport_source_settings,
            import_search: transport_import_search,
            settings: transport_settings,
        },
    )?;
    let working_roots = table
        .remove("working_roots")
        .map(|value| texts(value, "working_roots"))
        .transpose()?
        .unwrap_or_default();
    let worktrees = match table.remove("worktrees") {
        None => None,
        Some(toml::Value::Boolean(on)) => Some(on),
        Some(_) => return Err(wrong("worktrees")),
    };
    let peers = table
        .remove("peers")
        .map(|value| texts(value, "peers"))
        .transpose()?;
    let max_peers = match table.remove("max_peers") {
        None => None,
        Some(toml::Value::Integer(count)) => Some(
            usize::try_from(count).map_err(|_| invalid(name, "max_peers must be at least 1"))?,
        ),
        Some(_) => return Err(wrong("max_peers")),
    };
    let spec: ToolSpec =
        toml::Value::Table(table)
            .try_into()
            .map_err(|error: toml::de::Error| {
                let message = error.message();
                let what = if message.starts_with("missing field") {
                    message.to_string()
                } else if message.starts_with("unknown field") {
                    "a setting this host does not know".to_owned()
                } else if message.starts_with("unknown variant") {
                    "a setting has a value this host does not know".to_owned()
                } else {
                    "a setting has the wrong type".to_owned()
                };
                invalid(name, &what)
            })?;
    spec.check().map_err(|what| invalid(name, &what))?;
    let common = RawCommon {
        listen,
        transport,
        working_roots,
        worktrees,
        peers,
        max_peers,
    };
    Ok((common, spec))
}

/// `[agents.NAME]` tables that name a `program`, as entries reached over [`crate::Local`] only. A table with no `program`
/// (the app keeps an agent's profile there) names no command and is let go by.
pub(super) fn local_agents_of(
    agents: &IndexMap<String, toml::Value>,
) -> Result<Vec<Entry>, ConfigError> {
    let mut out = Vec::new();
    for (name, table) in agents {
        let Some(table) = table.as_table() else {
            continue;
        };
        let Some(program) = table.get("program") else {
            continue;
        };
        // A name the app shows, so any text a person can read: it is never part of an address.
        if name.is_empty() || name.chars().count() > MAX_NAME || name.chars().any(char::is_control)
        {
            return Err(ConfigError::Invalid {
                entry: None,
                what: format!(
                    "an agent's name has 1 to {MAX_NAME} characters, none of them control characters"
                ),
            });
        }
        let bad = |what: &str| ConfigError::Invalid {
            entry: Some(name.clone()),
            what: format!("agents.{name}: {what}"),
        };
        if name == crate::control::NAME {
            return Err(bad("that name is the host's control channel"));
        }
        let program = program
            .as_str()
            .filter(|program| !program.is_empty())
            .ok_or_else(|| bad("program is not text"))?;
        let args: Vec<String> = match table.get("args") {
            None => Vec::new(),
            Some(toml::Value::Array(args)) => args
                .iter()
                .map(|arg| arg.as_str().map(str::to_owned))
                .collect::<Option<_>>()
                .ok_or_else(|| bad("args is not a list of text"))?,
            Some(_) => return Err(bad("args is not a list of text")),
        };
        let usage = usage_of_command(program, &args);
        let mut entry = entry(
            name.clone(),
            EntryKind::Agent {
                program: OsString::from(program),
                args: args.into_iter().map(OsString::from).collect(),
                profiles: Vec::new(),
                usage,
            },
            RawCommon {
                listen: None,
                transport: None,
                working_roots: Vec::new(),
                worktrees: None,
                peers: None,
                max_peers: None,
            },
        )?;
        entry.local = true;
        out.push(entry);
    }
    Ok(out)
}

/// `[agents.NAME]` tables that carry an `a2a` table: agents on another machine. A name a program has is not reused, and one
/// table is not both.
pub(super) fn remote_agents_of(
    agents: &IndexMap<String, toml::Value>,
    local: &[Entry],
) -> Result<Vec<RemoteAgent>, ConfigError> {
    let mut out = Vec::new();
    for (name, table) in agents {
        let Some(table) = table.as_table() else {
            continue;
        };
        let Some(remote) = table.get("a2a") else {
            continue;
        };
        let bad = |what: &str| ConfigError::Invalid {
            entry: Some(name.clone()),
            what: format!("agents.{name}.a2a: {what}"),
        };
        if name.is_empty() || name.chars().count() > MAX_NAME || name.chars().any(char::is_control)
        {
            return Err(bad(
                "the name is empty, too long or holds a control character",
            ));
        }
        if table.contains_key("program") || local.iter().any(|entry| &entry.name == name) {
            return Err(bad("an agent is a program or another machine's, not both"));
        }
        let remote = remote
            .as_table()
            .ok_or_else(|| bad("is not a table of url and peer"))?;
        if let Some(key) = remote
            .keys()
            .find(|key| !["url", "peer", "source", "title"].contains(&key.as_str()))
        {
            return Err(bad(&format!("{key} is not a key of it")));
        }
        let text = |key: &str| -> Result<Option<&str>, ConfigError> {
            match remote.get(key) {
                None => Ok(None),
                Some(toml::Value::String(text)) if !text.is_empty() => Ok(Some(text)),
                Some(_) => Err(bad(&format!("{key} is not text"))),
            }
        };
        let url = text("url")?.ok_or_else(|| bad("url is needed"))?;
        if !url.starts_with("ws://") {
            return Err(bad("url is a ws:// address"));
        }
        let peer = text("peer")?
            .ok_or_else(|| bad("peer is needed: the key the other host proves"))?
            .parse::<PeerId>()
            .map_err(|_| bad("peer is not a peer id"))?;
        let title = text("title")?.map(str::to_owned);
        if let Some(title) = &title {
            check_title(name, title)?;
        }
        out.push(RemoteAgent {
            name: name.clone(),
            title,
            url: url.to_owned(),
            peer,
            source: text("source")?.map(str::to_owned),
        });
    }
    Ok(out)
}

/// The usage reader of the bundled agent whose own command `program` with `args` is, whatever the program's directory or
/// extension: its quota is read beside its turns.
pub(super) fn usage_of_command(program: &str, args: &[String]) -> Option<String> {
    let name = program.rsplit(['/', '\\']).next().unwrap_or(program);
    let stem = name.rsplit_once('.').map_or(name, |(stem, extension)| {
        if matches!(
            extension.to_ascii_lowercase().as_str(),
            "exe" | "cmd" | "bat"
        ) {
            stem
        } else {
            name
        }
    });
    crate::bundled::ALL.iter().find_map(|bundle| {
        let preset = &bundle.preset;
        let same = preset.program.eq_ignore_ascii_case(stem)
            && preset.args.len() == args.len()
            && preset
                .args
                .iter()
                .zip(args)
                .all(|(own, given)| own == given);
        preset.usage.filter(|_| same).map(str::to_owned)
    })
}

/// The keys of an entry that name a `transport` provider and where it loads from.
pub(super) struct TransportKeys {
    pub(super) id: Option<String>,
    pub(super) library: Option<String>,
    pub(super) source: Option<String>,
    pub(super) source_settings: Option<toml::Table>,
    pub(super) import_search: Option<String>,
    pub(super) settings: Option<toml::Table>,
}

/// An entry's `transport` provider, before its endpoint is joined to it.
pub(super) struct TransportSpec {
    id: String,
    origin: Option<Origin>,
    settings: Option<Settings>,
}

/// The transport an entry names, `None` when it names none. Relative paths are taken from `base`, as a harness's are; the id is
/// `pipe` or `websocket` (built in) or an outside provider's, which `transport_library` or `transport_source` loads.
pub(super) fn transport_of(
    name: &str,
    base: &Path,
    directory: &Path,
    keys: TransportKeys,
) -> Result<Option<TransportSpec>, ConfigError> {
    let Some(id) = keys.id else {
        let stray = [
            (keys.library.is_some(), "transport_library"),
            (keys.source.is_some(), "transport_source"),
            (keys.source_settings.is_some(), "transport_source_settings"),
            (keys.import_search.is_some(), "transport_import_search"),
            (keys.settings.is_some(), "transport_settings"),
        ];
        return match stray.iter().find(|(set, _)| *set) {
            Some((_, key)) => Err(invalid(name, &format!("{key} goes with transport"))),
            None => Ok(None),
        };
    };
    if id.is_empty() {
        return Err(invalid(name, "transport must not be empty"));
    }
    let origin = origin_of(
        name,
        base,
        directory,
        OriginKeys {
            library: keys.library.as_deref(),
            source: keys.source.as_deref(),
            source_settings: keys.source_settings,
            import_search: keys.import_search.as_deref(),
        },
    )
    .map_err(|error| match error {
        ConfigError::Invalid { entry, what } => ConfigError::Invalid {
            entry,
            what: format!("transport: {what}"),
        },
        other => other,
    })?;
    let built_in = crate::transport::BUILT_IN_TRANSPORTS.contains(&id.as_str());
    if built_in && origin.is_some() {
        return Err(invalid(
            name,
            "pipe and websocket are built in: transport_library and transport_source load an outside provider under its own id",
        ));
    }
    if origin.is_none() && !built_in {
        return Err(invalid(
            name,
            "transport is pipe, websocket, or the id of an outside provider that transport_library or transport_source loads",
        ));
    }
    let settings = keys
        .settings
        .map(|table| settings_of(name, table))
        .transpose()?;
    Ok(Some(TransportSpec {
        id,
        origin,
        settings,
    }))
}

pub(super) fn entry(
    name: String,
    kind: EntryKind,
    common: RawCommon,
) -> Result<Entry, ConfigError> {
    let listen = match (common.transport, common.listen) {
        (None, None) => Listen::Address(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0)),
        (None, Some(text)) => Listen::parse(&text).map_err(|what| invalid(&name, &what))?,
        (Some(_), None) => {
            return Err(invalid(
                &name,
                "listen is the endpoint the transport listens on, and is required with transport",
            ));
        }
        (Some(spec), Some(endpoint)) => {
            if endpoint.is_empty() {
                return Err(invalid(&name, "listen is empty"));
            }
            Listen::Transport(alloc::boxed::Box::new(TransportListen {
                id: spec.id,
                origin: spec.origin,
                settings: spec.settings,
                endpoint,
            }))
        }
    };
    let mut working_roots = Vec::new();
    for root in common.working_roots {
        let root = PathBuf::from(root);
        if !plain_absolute(&root) {
            return Err(invalid(
                &name,
                "a working root must be an absolute path with no . or .. in it",
            ));
        }
        working_roots.push(root);
    }
    let peers = common
        .peers
        .map(|peers| {
            peers
                .iter()
                .map(|peer| peer.parse::<PeerId>())
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| invalid(&name, "a peer is not a peer id"))
        })
        .transpose()?;
    let max_peers = common.max_peers.unwrap_or(DEFAULT_MAX_PEERS);
    if max_peers == 0 {
        return Err(invalid(&name, "max_peers must be at least 1"));
    }
    Ok(Entry {
        name,
        kind,
        listen,
        working_roots,
        worktrees: common.worktrees.unwrap_or(true),
        peers,
        max_peers,
        title: None,
        local: false,
        shared: false,
        shared_idle: None,
        shared_permission_window: None,
        shared_keep_log: true,
        shared_log_entries: None,
        shared_log_bytes: None,
        shell: None,
    })
}

pub(super) fn check_unique(entries: &[Entry]) -> Result<(), ConfigError> {
    for (index, entry) in entries.iter().enumerate() {
        for earlier in &entries[..index] {
            if earlier.name == entry.name {
                return Err(invalid(&entry.name, "two entries share this name"));
            }
            if entry.listen.is_fixed() && earlier.listen.same_place(&entry.listen) {
                return Err(invalid(
                    &entry.name,
                    "another entry listens on the same address",
                ));
            }
        }
    }
    Ok(())
}
