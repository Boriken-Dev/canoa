//! Building session parts (context, MCP servers, skills, access, and report) from resolved layers.

use alloc::borrow::ToOwned;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use std::path::{Path, PathBuf};

use canoa::vendor::SkillsRequest;
use coqui_acp_harness::Target;
use coqui_aos::{self as aos, StdFs};
use indexmap::IndexMap;
use serde_json::{Map, Value, json};

use super::super::overlay::{Built, ScriptRuns, build_each, resolve_path};
use super::super::{
    Context, Effective, EnvValue, LayerId, LayerKind, McpServer, Sourced, merge, resolve,
};
use super::overlays::{
    CHOSEN_PREFIX, Chosen, MAX_CONTEXT, ReadOverlay, context_not_taken, layers_of, mcp_words,
    same_digest, skill_limits, skills_not_taken,
};
use super::template::{Again, Templating};
use super::{Connection, Item, State, fetched, private};

/// What an agent's `initialize` answer says it takes with a session.
#[derive(Clone, Debug, Default)]
pub(super) struct Takes {
    pub(super) version: u64,
    pub(super) capabilities: Value,
}

impl Takes {
    pub(super) fn says(&self, path: &[&str]) -> bool {
        let mut at = &self.capabilities;
        for step in path {
            match at.get(step) {
                Some(next) => at = next,
                None => return false,
            }
        }
        at == &Value::Bool(true) || at.is_object()
    }

    /// Stdio is in v1's baseline and v2 says it; v2 has no SSE.
    pub(super) fn mcp(&self, transport: &str) -> bool {
        match (self.version, transport) {
            (2, "stdio") => self.says(&["session", "mcp", "stdio"]),
            (2, "http") => self.says(&["session", "mcp", "http"]),
            (2, _) => false,
            (_, "stdio") => true,
            (_, other) => self.says(&["mcpCapabilities", other]),
        }
    }

    pub(super) fn system_prompt(&self) -> bool {
        if self.version == 2 {
            self.says(&["session", "systemPrompt"])
        } else {
            self.says(&["sessionCapabilities", "systemPrompt"])
        }
    }

    pub(super) fn additional_directories(&self) -> bool {
        if self.version == 2 {
            self.says(&["session", "additionalDirectories"])
        } else {
            self.says(&["sessionCapabilities", "additionalDirectories"])
        }
    }
}

/// What an agent's `initialize` answer says it takes, and the name it gives itself: version 2 says them under
/// `capabilities` and `info`, version 1 under `agentCapabilities` and `agentInfo`.
pub(super) fn described(result: &Value) -> (Takes, Option<&str>) {
    let version = result
        .get("protocolVersion")
        .and_then(Value::as_u64)
        .unwrap_or(1);
    let (capabilities, info) = if version == 2 {
        ("capabilities", "info")
    } else {
        ("agentCapabilities", "agentInfo")
    };
    let takes = Takes {
        version,
        capabilities: result.get(capabilities).cloned().unwrap_or(Value::Null),
    };
    let reported = result
        .get(info)
        .and_then(|info| info.get("name"))
        .and_then(Value::as_str);
    (takes, reported)
}

/// A private directory in a connection's own, removed with it; `under` is that directory, which is never made here.
#[derive(Debug)]
pub(in crate::profile::hosted) struct Written(pub(in crate::profile::hosted) PathBuf);

impl Written {
    pub(in crate::profile::hosted) fn create(
        under: &Path,
        files: &BTreeMap<String, Vec<u8>>,
    ) -> std::io::Result<Self> {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random).map_err(|error| std::io::Error::other(error.to_string()))?;
        let name: String = data_encoding::HEXLOWER.encode(&random);
        let root = under.join(name);
        #[cfg(unix)]
        private::check(under)?;
        private::make_inside(&root)?;
        let written = Self(root);
        for (path, bytes) in files {
            let mut target = written.0.clone();
            for part in path.split('/') {
                if part.is_empty() || part == "." || part == ".." {
                    return Err(std::io::Error::other("a path that is not plain"));
                }
                target.push(part);
            }
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::io::Write::write_all(
                &mut std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&target)?,
                bytes,
            )?;
        }
        Ok(written)
    }

    /// The directory everything is written under.
    pub(in crate::profile::hosted) fn root(&self) -> &Path {
        &self.0
    }

    /// Writes one more file, at a plain relative path.
    pub(in crate::profile::hosted) fn add(&self, path: &str, bytes: &[u8]) -> std::io::Result<()> {
        let mut target = self.0.clone();
        for part in path.split('/') {
            if part.is_empty() || part == "." || part == ".." {
                return Err(std::io::Error::other("a path that is not plain"));
            }
            target.push(part);
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::io::Write::write_all(
            &mut std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)?,
            bytes,
        )
    }

    pub(super) fn path(&self, below: &str) -> String {
        let mut path = self.0.clone();
        if !below.is_empty() {
            path.extend(below.split('/'));
        }
        path.to_string_lossy().into_owned()
    }
}

impl Drop for Written {
    fn drop(&mut self) {
        // The connection's directory is not touched: it goes when the connection does.
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// What every session of a connection is given, worked out once the agent's `initialize` answer says what it takes.
#[derive(Debug, Default)]
pub(super) struct Parts {
    pub(super) system_prompt: Option<String>,
    /// The system prompt's texts in order, each with what replacing it again needs when it names `AOS_PROJECT_ROOT`.
    pub(super) pieces: Vec<(String, Option<Again>)>,
    /// Whether the agent takes a system prompt with a new session, whether the profile gives one or not.
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "read by tests that check initialize capabilities")
    )]
    pub(super) system_prompt_taken: bool,
    pub(super) servers: Vec<Value>,
    /// Every MCP server name the profile holds, started or not: a client's server may not take one.
    pub(super) names: Vec<String>,
    pub(super) directories: Vec<String>,
    pub(super) meta: Option<Value>,
    /// The access the session is run with, for the request's `_meta` under [`coqui_acp_harness::ACCESS_META`].
    pub(super) access: Value,
    /// What the access adds to a session request's `_meta` for a program the host started with it (Grok's `yoloMode`).
    pub(super) agent_meta: Map<String, Value>,
    pub(super) report: Value,
    /// Servers the configuration does not allow and the person may, each ready to be added to one session.
    pub(super) approvable: Vec<Approvable>,
    #[allow(
        dead_code,
        reason = "held so the directory is removed with the connection, not read"
    )]
    pub(super) written: Option<Written>,
}

impl Parts {
    /// The system prompt for a session opened in `project`: the texts that name `AOS_PROJECT_ROOT` replaced again with it.
    pub(super) fn system_prompt_for(
        &self,
        project: Option<&str>,
    ) -> Result<Option<String>, String> {
        let Some(prompt) = &self.system_prompt else {
            return Ok(None);
        };
        let Some(project) =
            project.filter(|_| self.pieces.iter().any(|(_, again)| again.is_some()))
        else {
            return Ok(Some(prompt.clone()));
        };
        let mut texts = Vec::new();
        for (text, again) in &self.pieces {
            texts.push(match again {
                Some(again) => again.render(Some(project))?,
                None => text.clone(),
            });
        }
        let joined = texts.join("\n\n");
        if joined.len() > MAX_CONTEXT {
            return Err(format!(
                "the initial context passes {MAX_CONTEXT} bytes with this session's folder in it"
            ));
        }
        Ok(Some(joined))
    }
}

/// An MCP server of an overlay or a project that canoa.cfg does not allow, and what the client is shown of it.
#[derive(Debug)]
pub(super) struct Approvable {
    pub(super) name: String,
    pub(super) layer: String,
    pub(super) digest: String,
    pub(super) server: Value,
}

/// What ACP's `mcpServers` entry carries: the command or URL, the variables or headers, and a stdio server's arguments.
#[derive(Debug)]
pub(super) struct Acp {
    pub(super) target: String,
    pub(super) pairs: Vec<(String, String)>,
    pub(super) args: Value,
}

/// Resolves one variable or header: its key, its name, its value as written; `None` when it is left out.
type Resolve<'f> = dyn FnMut(&str, &str, &EnvValue) -> Result<Option<String>, String> + 'f;

/// `values` resolved, those left out dropped.
pub(super) fn resolved(
    name: &str,
    table: &str,
    values: &IndexMap<String, EnvValue>,
    value: &mut Resolve<'_>,
) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    for (key, raw) in values {
        if let Some(found) = value(&format!("mcp.{name}.{table}.{key}"), key, raw)? {
            out.push((key.clone(), found));
        }
    }
    Ok(out)
}

/// One MCP server of the profile, resolved.
#[derive(Debug)]
pub(super) struct ReadyMcp {
    pub(super) name: String,
    pub(super) layer: LayerId,
    pub(super) digest: String,
    pub(super) transport: &'static str,
    pub(super) acp: Option<Acp>,
    pub(super) withheld: Vec<String>,
    pub(super) refused: Option<String>,
}

/// The digest an `allow_mcp` entry names: of the layer and the server as the profile's view writes it.
/// A stdio server's `program` as found is part of it, so another program found by the same name is off again.
pub(super) fn mcp_digest(layer: &LayerId, server: &McpServer, program: Option<&str>) -> String {
    let mut definition =
        json!({"layer": layer.to_string(), "server": super::super::mcp_json(server)});
    if let Some(program) = program {
        definition["program"] = json!(program);
    }
    aos::Digest::of(&serde_json::to_vec(&definition).unwrap_or_default()).to_string()
}

/// The program a stdio server's `command` names, absolute: as written when absolute, from its layer's directory when it
/// holds a separator, else the first match on the host's `PATH` with the folders its overlay adds to `PATH` (`additions`: those
/// that go first come first, those that go last after the host's own), with `PATHEXT` on Windows, where only absolute entries
/// count, since a relative one would name whatever directory the agent runs in.
pub(in crate::profile::hosted) fn program_of(
    command: &str,
    dir: Option<&Path>,
    additions: &[(PathBuf, bool)],
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Result<String, String> {
    if Path::new(command).is_absolute() {
        return Ok(command.to_owned());
    }
    if command.contains(['/', '\\']) {
        return dir
            .map(|dir| dir.join(command).to_string_lossy().into_owned())
            .ok_or_else(|| format!("{command} is relative, and its layer has no directory"));
    }
    let not_found = || format!("{command} is not found on the host's PATH");
    let own = lookup("PATH").unwrap_or_default();
    let first = additions
        .iter()
        .filter(|(_, last)| !last)
        .map(|(folder, _)| folder.clone());
    let last = additions
        .iter()
        .filter(|(_, last)| *last)
        .map(|(folder, _)| folder.clone());
    let path = std::env::join_paths(first.chain(std::env::split_paths(&own)).chain(last))
        .map_err(|_| not_found())?
        .to_string_lossy()
        .into_owned();
    if path.is_empty() {
        return Err(not_found());
    }
    let extensions: Vec<String> = if cfg!(windows) && Path::new(command).extension().is_none() {
        lookup("PATHEXT")
            .unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".to_owned())
            .split(';')
            .filter(|extension| !extension.is_empty())
            .map(str::to_owned)
            .collect()
    } else {
        vec![String::new()]
    };
    for directory in std::env::split_paths(&path).filter(|directory| directory.is_absolute()) {
        for extension in &extensions {
            let candidate = directory.join(format!("{command}{extension}"));
            if runnable(&candidate) {
                return Ok(candidate.to_string_lossy().into_owned());
            }
        }
    }
    Err(not_found())
}

/// A regular file, executable by someone on Unix.
pub(super) fn runnable(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|meta| {
        #[cfg(unix)]
        let executable = std::os::unix::fs::PermissionsExt::mode(&meta.permissions()) & 0o111 != 0;
        #[cfg(not(unix))]
        let executable = true;
        meta.is_file() && executable
    })
}

/// Each overlay's environment as it stood once that overlay applied, by identity: what its MCP servers' text reads.
fn environments_after(
    effective: &Effective,
    lookup: &dyn Fn(&str) -> Option<String>,
    pre_env: &BTreeMap<String, aos::HookRun>,
    scripts: &ScriptRuns,
) -> BTreeMap<String, Result<Built, String>> {
    build_each(&effective.aos, lookup, pre_env, scripts)
        .into_iter()
        .zip(&effective.aos)
        .map(|(built, layer)| {
            (
                layer.identity.clone(),
                built.map_err(|error| error.to_string()),
            )
        })
        .collect()
}

pub(super) fn ready_mcp(
    name: &str,
    sourced: &Sourced<McpServer>,
    effective: &Effective,
    after: &BTreeMap<String, Result<Built, String>>,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> ReadyMcp {
    let layer = sourced.layer.clone();
    let server = &sourced.value;
    let overlay = (layer.kind == LayerKind::Overlay)
        .then(|| effective.aos.iter().find(|aos| aos.identity == layer.name))
        .flatten();
    let windows = cfg!(windows);
    let mut withheld = Vec::new();
    let transport = match server {
        McpServer::Stdio { .. } => "stdio",
        McpServer::Http { .. } => "http",
        McpServer::Sse { .. } => "sse",
    };
    let mut value = |key: &str, label: &str, raw: &EnvValue| -> Result<Option<String>, String> {
        match (raw, overlay) {
            (EnvValue::Text(text), Some(aos)) => {
                let env = match after.get(&aos.identity) {
                    Some(Ok(env)) => env,
                    Some(Err(why)) => return Err(why.clone()),
                    None => return Err("its overlay's environment was not built".to_owned()),
                };
                let lookup = |variable: &str| -> Result<Option<String>, aos::Error> {
                    let same = |name: &str| {
                        variable == name || windows && variable.eq_ignore_ascii_case(name)
                    };
                    if same("AOS_OVERLAY_ROOT") {
                        return Ok(Some(aos.root.clone()));
                    }
                    if same("AOS_OVERLAY_NAME") {
                        return Ok(Some(aos::normalize_name(&aos.identity)));
                    }
                    env.lookup(variable)
                };
                aos::substitute(text, &lookup).map(Some).map_err(|error| {
                    format!(
                        "its text cannot be substituted ({}: {})",
                        error.class, error.subject
                    )
                })
            }
            (EnvValue::Path(path), Some(aos)) => Ok(Some(resolve_path(&aos.root, path, windows))),
            (
                EnvValue::Reference {
                    reference,
                    optional,
                },
                _,
            ) => {
                let (provider, variable) = reference.split_once(':').unwrap_or((reference, ""));
                if provider != "env" {
                    withheld.push(format!(
                        "{label}: reads {reference}, a secret this host does not hold"
                    ));
                    return Ok(None);
                }
                match lookup(variable) {
                    Some(found) => Ok(Some(found)),
                    None if *optional => {
                        withheld.push(format!("{label}: ${variable} is not set"));
                        Ok(None)
                    }
                    None => Err(format!("{key} reads a variable that is not set")),
                }
            }
            _ => resolve(
                key,
                raw,
                sourced.dir.as_deref(),
                layer.kind.is_confined(),
                lookup,
            )
            .map(|secret| Some(secret.expose().to_owned()))
            .map_err(|error| error.to_string()),
        }
    };
    let acp = match server {
        McpServer::Stdio { command, args, env } => {
            let additions = overlay
                .map(super::super::overlay::path_additions)
                .unwrap_or_default();
            program_of(command, sourced.dir.as_deref(), &additions, lookup).and_then(|program| {
                resolved(name, "env", env, &mut value).map(|env| Acp {
                    target: program,
                    pairs: env,
                    args: json!(args),
                })
            })
        }
        McpServer::Http { url, headers } | McpServer::Sse { url, headers } => {
            resolved(name, "headers", headers, &mut value).map(|headers| Acp {
                target: url.clone(),
                pairs: headers,
                args: Value::Null,
            })
        }
    };
    let (acp, refused) = match acp {
        Ok(acp) => (Some(acp), None),
        Err(why) => (None, Some(why)),
    };
    ReadyMcp {
        name: name.to_owned(),
        digest: mcp_digest(
            &layer,
            server,
            acp.as_ref()
                .filter(|_| transport == "stdio")
                .map(|acp| acp.target.as_str()),
        ),
        layer,
        transport,
        acp,
        withheld,
        refused,
    }
}

/// `server` as ACP `mcpServers` writes it in protocol `version`: v1's stdio form has no type.
pub(super) fn acp_server(server: &ReadyMcp, version: u64) -> Option<Value> {
    let Acp {
        target: first,
        pairs,
        args,
    } = server.acp.as_ref()?;
    let pairs: Vec<Value> = pairs
        .iter()
        .map(|(name, value)| json!({"name": name, "value": value}))
        .collect();
    Some(if server.transport == "stdio" {
        crate::loopback::stdio_server_entry(
            &server.name,
            first,
            args.clone(),
            Value::Array(pairs),
            version,
        )
    } else {
        json!({"type": server.transport, "name": server.name, "url": first, "headers": pairs})
    })
}

/// A context's text: an overlay's from the bytes its digest covered, the host's own from its file. An overlay's context
/// file has its `${NAME}` names replaced.
pub(super) fn context_text(
    context: &Sourced<Context>,
    overlays: &[ReadOverlay],
    templating: &Templating<'_>,
) -> Result<(String, String, Option<Again>), String> {
    match &context.value {
        Context::Text(text) => Ok((text.clone(), "text".to_owned(), None)),
        Context::TemplateBody(body) => {
            let rendered = templating.body(&context.layer.name, body)?;
            Ok((rendered.text, "text".to_owned(), rendered.again))
        }
        Context::Template(file) => {
            let rendered = templating.context(&context.layer.name, file)?;
            Ok((rendered.text, file.clone(), rendered.again))
        }
        Context::File(file) => {
            let text = if context.layer.kind == LayerKind::Overlay {
                overlays
                    .iter()
                    .find(|overlay| overlay.identity == context.layer.name)
                    .and_then(|overlay| overlay.file(file))
                    .and_then(|bytes| String::from_utf8(bytes.to_vec()).ok())
                    .ok_or_else(|| format!("the file {file} cannot be read as UTF-8 text"))?
            } else {
                resolve(
                    "context",
                    &EnvValue::FromFile(file.clone()),
                    context.dir.as_deref(),
                    context.layer.kind.is_confined(),
                    |_| None,
                )
                .map_err(|_| {
                    format!("the file {file} cannot be read as UTF-8 text of at most 1 MiB")
                })?
                .expose()
                .to_owned()
            };
            Ok((text, file.clone(), None))
        }
    }
}

/// A skill's files: an overlay's from its digest-covered bytes, the host's own read once from its directory, which may
/// hold no link or special file.
pub(super) fn skill_files(
    skill: &Sourced<super::super::Skill>,
    overlays: &[ReadOverlay],
    chosen: &Chosen,
    templating: &Templating<'_>,
) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let mut files = if let Some(name) = skill.value.path.strip_prefix(CHOSEN_PREFIX) {
        chosen.get(name).cloned().unwrap_or_default()
    } else if skill.layer.kind == LayerKind::Overlay {
        overlays
            .iter()
            .find(|overlay| overlay.identity == skill.layer.name)
            .map(|overlay| overlay.under(&skill.value.path))
            .unwrap_or_default()
    } else {
        let path = Path::new(&skill.value.path);
        let path = if path.is_absolute() {
            path.to_owned()
        } else {
            skill
                .dir
                .as_ref()
                .ok_or("its layer has no directory")?
                .join(path)
        };
        let (tree, _) =
            aos::read_tree(&StdFs, &path, &skill_limits()).map_err(|error| match error.class {
                aos::Class::Symlink | aos::Class::SpecialFile => {
                    "its directory holds a link or a special file".to_owned()
                }
                _ => format!(
                    "its files cannot be read ({}: {})",
                    error.class, error.subject
                ),
            })?;
        tree.paths()
            .filter_map(|path| Some((path.to_owned(), tree.bytes(path)?.to_vec())))
            .collect()
    };

    // A skill the app fetched is delivered as it was fetched; a bundled one has its body replaced.
    let bundled = !skill
        .value
        .path
        .starts_with(&format!("{}/", fetched::SKILLS_DIR));
    if skill.layer.kind == LayerKind::Overlay
        && bundled
        && let Some(bytes) = files.get("SKILL.md")
    {
        let replaced = templating.skill(&skill.layer.name, bytes)?;
        files.insert("SKILL.md".to_owned(), replaced);
    }
    if files.contains_key("SKILL.md") {
        Ok(files)
    } else {
        Err("its directory holds no SKILL.md".to_owned())
    }
}

#[cfg(test)]
pub(super) fn parts_for(
    connection: &Connection,
    harness: Option<&str>,
    takes: &Takes,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Parts {
    parts_with(connection, harness, None, takes, lookup)
}

pub(super) fn parts_with(
    connection: &Connection,
    harness: Option<&str>,
    reported: Option<&str>,
    takes: &Takes,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Parts {
    let mut notes = Vec::new();
    let layers = layers_of(
        &connection.host,
        &connection.overlays,
        &connection.chosen,
        harness,
        &mut notes,
    );
    let effective = merge(&layers);
    let seen = connection.session.lookup(lookup);
    let lookup: &dyn Fn(&str) -> Option<String> = &seen;
    let mut items = connection.notes.clone();
    items.extend(notes);
    items.extend(connection.env_items.iter().cloned());

    let templating = Templating {
        overlays: &connection.overlays,
        hooks: &connection.hooks,
        layers: &effective.aos,
        options: &connection.host.options,
    };
    let mut texts = Vec::new();
    let mut bytes = 0;
    let mut indexed = BTreeSet::new();
    for context in effective.contexts.iter().chain(effective.context.as_ref()) {
        if connection.served_as_index(context) {
            if indexed.insert(context.layer.name.clone()) {
                let note = format!(
                    "The files of the overlay {name} are not included here: search them with kb_search (overlay: {name}) and read a passage's lines with kb_read.",
                    name = context.layer.name
                );
                texts.push((note, context.layer.clone(), "kb index".to_owned(), None));
            }
            continue;
        }
        match context_text(context, &connection.overlays, &templating) {
            Ok((text, source, again)) => {
                bytes += text.len();
                if bytes > MAX_CONTEXT {
                    items.push(Item::new(
                        "context",
                        &context.layer,
                        State::Refused,
                        format!("the initial context passes {MAX_CONTEXT} bytes"),
                    ));
                } else {
                    texts.push((text, context.layer.clone(), source, again));
                }
            }
            Err(why) => items.push(Item::new("context", &context.layer, State::Refused, why)),
        }
    }
    let mut system_prompt = None;
    let mut pieces = Vec::new();
    if !texts.is_empty() {
        let taken = takes.system_prompt();
        if taken {
            system_prompt = Some(
                texts
                    .iter()
                    .map(|(text, ..)| text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            );
            pieces = texts
                .iter()
                .map(|(text, _, _, again)| (text.clone(), again.clone()))
                .collect();
        }
        for (_, layer, source, _) in &texts {
            items.push(if taken {
                Item::new(
                    "context",
                    layer,
                    State::Applied,
                    format!("ACP systemPrompt, appended ({source})"),
                )
            } else {
                Item::new(
                    "context",
                    layer,
                    State::Unsupported,
                    context_not_taken(harness),
                )
            });
        }
    }

    let mut written_files = BTreeMap::new();
    let mut usable = Vec::new();
    for (name, skill) in &effective.skills {
        match skill_files(skill, &connection.overlays, &connection.chosen, &templating) {
            Ok(files) => usable.push((name, &skill.layer, files)),
            Err(why) => items.push(Item::new(
                format!("skills.{name}"),
                &skill.layer,
                State::Refused,
                why,
            )),
        }
    }
    let mut written = None;
    let mut meta = None;
    let mut directories = Vec::new();
    if !usable.is_empty() {
        let delivery = harness
            .and_then(|aos| {
                crate::vendors::behave_aos(aos, |vendor| {
                    vendor.skills_delivery(SkillsRequest::new(takes.additional_directories()))
                })
            })
            .and_then(Result::ok)
            .flatten();
        match delivery {
            None => {
                for (name, layer, _) in &usable {
                    items.push(Item::new(
                        format!("skills.{name}"),
                        layer,
                        State::Unsupported,
                        skills_not_taken(harness),
                    ));
                }
            }
            Some(delivery) => {
                for (name, _, files) in &usable {
                    for (path, data) in files {
                        written_files
                            .insert(format!("{}/{name}/{path}", delivery.folder), data.clone());
                    }
                }
                for file in &delivery.files {
                    written_files.insert(file.path.clone(), file.text.clone().into_bytes());
                }
                match connection
                    .private()
                    .and_then(|under| Written::create(under, &written_files))
                {
                    Ok(done) => {
                        let given = delivery
                            .plugin
                            .as_ref()
                            .map(|plugin| plugin_meta(plugin, &done))
                            .transpose();
                        match given {
                            Ok(given) => {
                                for (name, layer, _) in &usable {
                                    items.push(Item::new(
                                        format!("skills.{name}"),
                                        layer,
                                        State::Applied,
                                        delivery.how.replace("{name}", name),
                                    ));
                                }
                                meta = given;
                                if delivery.directory {
                                    directories.push(done.0.to_string_lossy().into_owned());
                                }
                                written = Some(done);
                            }
                            Err(why) => {
                                for (name, layer, _) in &usable {
                                    items.push(Item::new(
                                        format!("skills.{name}"),
                                        layer,
                                        State::Refused,
                                        format!(
                                            "the agent's delivery of them is not usable ({why})"
                                        ),
                                    ));
                                }
                            }
                        }
                    }
                    Err(error) => {
                        for (name, layer, _) in &usable {
                            items.push(Item::new(
                                format!("skills.{name}"),
                                layer,
                                State::Refused,
                                format!("its files could not be written for the agent ({error})"),
                            ));
                        }
                    }
                }
            }
        }
    }

    let after = environments_after(
        &effective,
        lookup,
        &connection.pre_env.runs,
        &connection.scripts,
    );
    let mut servers = Vec::new();
    let mut approvable = Vec::new();
    let mut names = Vec::new();
    let (mcp_skipped, mcp_note) = mcp_words(harness);
    for (name, sourced) in &effective.mcp {
        names.push(name.clone());
        let ready = ready_mcp(name, sourced, &effective, &after, lookup);
        let key = format!("mcp.{name}");
        let allowed = !connection.host.off_mcp.contains(name)
            && (!ready.layer.kind.is_confined()
                || connection
                    .host
                    .allow_mcp
                    .get(name)
                    .is_some_and(|digest| same_digest(digest, &ready.digest)));
        let mut item = if let Some(why) = &ready.refused {
            Item::new(key, &ready.layer, State::Refused, why.clone())
        } else if !allowed {
            if !connection.host.off_mcp.contains(name)
                && mcp_skipped.is_none()
                && takes.mcp(ready.transport)
                && let Some(server) = acp_server(&ready, takes.version)
            {
                approvable.push(Approvable {
                    name: name.clone(),
                    layer: ready.layer.to_string(),
                    digest: ready.digest.clone(),
                    server,
                });
            }
            tracing::info!(
                entry = connection.entry,
                "mcp.{name} of {} is off: host.profiles.{}.allow_mcp.{name} = \"{}\" allows it",
                ready.layer,
                connection.name,
                ready.digest
            );
            Item::new(
                key,
                &ready.layer,
                State::Off,
                "not allowed by the host's configuration: allow_mcp names it at this digest",
            )
        } else if let Some(why) = &mcp_skipped {
            Item::new(key, &ready.layer, State::Unsupported, why.clone())
        } else if !takes.mcp(ready.transport) {
            Item::new(
                key,
                &ready.layer,
                State::Unsupported,
                format!(
                    "the agent does not say it starts MCP servers over {}",
                    ready.transport
                ),
            )
        } else if let Some(server) = acp_server(&ready, takes.version) {
            servers.push(server);
            let mut said = vec!["ACP mcpServers".to_owned()];
            if let Some(note) = &mcp_note {
                said.push(note.replace("{name}", name));
            }
            if !ready.withheld.is_empty() {
                said.push(format!("left out: {}", ready.withheld.join(", ")));
            }
            Item::new(key, &ready.layer, State::Applied, said.join("; "))
        } else {
            Item::new(key, &ready.layer, State::Refused, "it cannot be resolved")
        };
        item.digest = Some(ready.digest);
        items.push(item);
    }

    for (key, value) in [
        ("model", &effective.model),
        ("effort", &effective.effort),
        ("mode", &effective.mode),
    ] {
        if let Some(value) = value {
            let mut item = Item::new(
                key,
                &value.layer,
                State::Unsupported,
                "the host does not set it; the session's options do",
            );
            item.value = Some(value.value.to_string());
            items.push(item);
        }
    }

    let access = effective.access.resolved();
    items.extend(access_items(
        &effective.access,
        access,
        access_target(reported, connection.launched_as),
    ));

    items.extend(connection.hooks.items(&BTreeSet::new(), harness));
    // An overlay listed twice reports each item once.
    let mut seen = BTreeSet::new();
    items.retain(|item| {
        seen.insert((
            item.key.clone(),
            item.layer.clone(),
            item.how.clone(),
            item.state.as_str(),
        ))
    });

    let label = harness.unwrap_or(&connection.entry);
    let applied = items
        .iter()
        .filter(|item| item.state == State::Applied)
        .count();
    // Each line names the overlay it comes from, so two overlays that miss the same thing are two lines.
    let mut missed: Vec<String> = Vec::new();
    for item in items.iter().filter(|item| item.state != State::Applied) {
        let line = match item.layer.strip_prefix("overlay:") {
            Some(overlay) => format!("{overlay}: {} ({})", item.key, item.state.as_str()),
            None => format!("{} ({})", item.key, item.state.as_str()),
        };
        if !missed.contains(&line) {
            missed.push(line);
        }
    }
    let summary = format!(
        "Profile {} for {label}: {applied} applied{}",
        connection.name,
        if missed.is_empty() {
            String::new()
        } else {
            format!("; not applied: {}", missed.join(", "))
        }
    );
    Parts {
        system_prompt,
        pieces,
        system_prompt_taken: takes.system_prompt(),
        servers,
        names,
        directories,
        meta,
        access: access.to_json(),
        agent_meta: connection
            .launched_as
            .and_then(|target| access.translate(&target).json())
            .unwrap_or_default(),
        approvable,
        report: json!({
            "name": connection.name,
            "harness": label,
            "summary": summary,
            "loadout": connection.role,
            "items": items.iter().map(Item::json).collect::<Vec<_>>(),
        }),
        written,
    }
}

/// The table of words for the agent whose registry id is `aos`, as its description says, and whether a program the host starts
/// itself is given the access when it starts (and not in each session's request).
pub(super) fn access_table(aos: &str) -> Option<(Target, bool)> {
    let target = crate::vendors::behave_aos(aos, |vendor| vendor.access_target())?.ok()??;
    Some((
        crate::bundled::access_target(&target.harness)?,
        target.at_launch,
    ))
}

/// Which table of words says what became of the access: the one of the agent that reports itself as `reported` when it takes the
/// access in each session's request, else the program the host started with the access (`launched`). An agent that is neither was
/// started as its entry says.
fn access_target(reported: Option<&str>, launched: Option<Target>) -> Option<Target> {
    let described = reported
        .and_then(|name| crate::vendors::behave_name(name, |vendor| vendor.access_target()))
        .and_then(Result::ok)
        .flatten()
        .filter(|target| !target.at_launch)
        .and_then(|target| crate::bundled::access_target(&target.harness));
    described.or(launched)
}

/// What became of each part of the access a layer above the built-in one set, as `Item`s: the words it was turned into, or why
/// the harness cannot express it, from the table `coqui_acp_harness::Access::translate` is. A part nothing above the built-in
/// layer set is the standard access, which says nothing.
fn access_items(
    effective: &super::super::EffectiveAccess,
    access: coqui_acp_harness::Access,
    target: Option<Target>,
) -> Vec<Item> {
    use coqui_acp_harness::{Part, Sandbox};
    let own = |layer: &LayerId| layer.kind != LayerKind::BuiltIn;
    let translation = target.map(|target| access.translate(&target));
    // An agent no table covers is started as its entry says; its own settings decide.
    let not_passed = |own_applies: Option<&str>| match own_applies {
        Some(what) => format!(
            "this agent is started as its entry says: Canoa passes it no access setting, so its own {what} applies"
        ),
        None => {
            "this agent is started as its entry says: Canoa passes it no access setting".to_owned()
        }
    };
    let mut items = Vec::new();
    let mut add = |key: &str, layer: &LayerId, value: String, how: Result<String, String>| {
        if !own(layer) {
            return;
        }
        let (state, how) = match how {
            Ok(how) => (State::Applied, how),
            Err(why) => (State::Unsupported, why),
        };
        let mut item = Item::new(format!("access.{key}"), layer, state, how);
        item.value = Some(value);
        items.push(item);
    };
    let said = |part: Option<&Part>, otherwise: String| match part {
        Some(part) => part.said.map(str::to_owned).map_err(str::to_owned),
        None => Err(otherwise),
    };
    if let Some(sandbox) = &effective.sandbox {
        let otherwise = not_passed((access.sandbox == Sandbox::Unrestricted).then_some("sandbox"));
        add(
            "sandbox",
            &sandbox.layer,
            access.sandbox.id().to_owned(),
            said(translation.as_ref().map(|each| &each.sandbox), otherwise),
        );
    }
    if let Some(network) = &effective.network {
        let otherwise = not_passed(access.network.then_some("network setting"));
        add(
            "network",
            &network.layer,
            access.network.to_string(),
            said(translation.as_ref().map(|each| &each.network), otherwise),
        );
    }
    if let Some(approvals) = &effective.approvals {
        add(
            "approvals",
            &approvals.layer,
            access.approvals.id().to_owned(),
            said(
                translation.as_ref().map(|each| &each.approvals),
                not_passed(None),
            ),
        );
    }
    items
}

/// Adds `parts` to a session request's `params`. A client's MCP server may not take a name the profile holds.
pub(super) fn add_parts(params: &mut Map<String, Value>, parts: &Parts) -> Result<(), String> {
    let servers = params.entry("mcpServers").or_insert_with(|| json!([]));
    let servers = servers
        .as_array_mut()
        .ok_or_else(|| "mcpServers is not a list".to_owned())?;
    if servers.iter().any(|server| {
        server
            .get("name")
            .and_then(Value::as_str)
            .is_some_and(|name| parts.names.iter().any(|held| held == name))
    }) {
        return Err("an MCP server of that name comes from the host's profile".to_owned());
    }
    servers.extend(parts.servers.iter().cloned());
    let project = params.get("cwd").and_then(Value::as_str).map(str::to_owned);
    if let Some(text) = parts.system_prompt_for(project.as_deref())? {
        let block = json!({"type": "text", "text": text});
        match params
            .get_mut("systemPrompt")
            .and_then(Value::as_object_mut)
        {
            Some(prompt) => match prompt.get_mut("content").and_then(Value::as_array_mut) {
                Some(content) => content.insert(0, block),
                None => {
                    prompt.insert("content".to_owned(), json!([block]));
                }
            },
            None => {
                params.insert(
                    "systemPrompt".to_owned(),
                    json!({"mode": "append", "content": [block]}),
                );
            }
        }
    }
    if !parts.directories.is_empty() {
        let more = params
            .entry("additionalDirectories")
            .or_insert_with(|| json!([]));
        more.as_array_mut()
            .ok_or_else(|| "additionalDirectories is not a list".to_owned())?
            .extend(parts.directories.iter().map(|directory| json!(directory)));
    }
    if let Some(meta) = &parts.meta {
        merge_meta(params, meta);
    }
    let held = params.entry("_meta").or_insert_with(|| json!({}));
    if !held.is_object() {
        *held = json!({});
    }
    held[coqui_acp_harness::ACCESS_META] = parts.access.clone();
    for (key, value) in &parts.agent_meta {
        held[key] = value.clone();
    }
    Ok(())
}

/// Puts the profile's `_meta` in the request's, keeping what the host has already added for the session, such as the plugin of its
/// hooks: objects are merged key by key and lists appended, the profile's first.
fn merge_meta(params: &mut Map<String, Value>, meta: &Value) {
    let mut merged = meta.clone();
    if let Some(held) = params.get("_meta") {
        merge_json(&mut merged, held);
    }
    params.insert("_meta".to_owned(), merged);
}

/// Adds what `from` holds to `into`: an object's keys are merged, a list is appended to, and any other value `into` has stays.
pub(in crate::profile::hosted) fn merge_json(into: &mut Value, from: &Value) {
    match (into, from) {
        (Value::Object(into), Value::Object(from)) => {
            for (key, value) in from {
                match into.get_mut(key) {
                    Some(held) => merge_json(held, value),
                    None => {
                        into.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        (Value::Array(into), Value::Array(from)) => into.extend(from.iter().cloned()),
        _ => {}
    }
}

/// What a session request's `_meta` gains to load the plugin an agent asked for, written under `written`: the agent's own text with
/// its path placeholder replaced by the plugin's folder.
///
/// # Errors
///
/// Why the agent's text is not a JSON object.
pub(in crate::profile::hosted) fn plugin_meta(
    plugin: &canoa::vendor::PluginDelivery,
    written: &Written,
) -> Result<Value, String> {
    fn place(value: &mut Value, path: &str) {
        match value {
            Value::String(text) if text == canoa::vendor::PLUGIN_PATH => {
                path.clone_into(text);
            }
            Value::Array(list) => list.iter_mut().for_each(|each| place(each, path)),
            Value::Object(map) => map.values_mut().for_each(|each| place(each, path)),
            _ => {}
        }
    }
    let mut meta: Value = guatiao_serde::adapt::from_value(&plugin.meta)
        .map_err(|_| "its _meta is not JSON".to_owned())?;
    if !meta.is_object() {
        return Err("its _meta is not an object".to_owned());
    }
    place(&mut meta, &written.path(&plugin.folder));
    Ok(meta)
}
