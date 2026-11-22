//! Environments: the places an agent works, as the providers of the `environment` kind offer them. Nothing here names a kind of
//! environment: the host's own machine (`here`) is a provider of this kind that the host carries, and every other is a provider a
//! bundled library or an extension adds, configured by a table of canoa.cfg (`[host.environments.NAME]`: `kind`, the provider's id,
//! and the settings it takes).
//!
//! A session uses an environment in three ways, which can be combined and which a client asks for in `initialize`'s `_meta`
//! ([`META`]): the agent **runs** in it (its program is started through the environment's spawner), its standard file and terminal
//! tools are **served** from it (the host answers the ACP client methods `fs/*` and `terminal/*` itself), or it is **offered as
//! tools** (one MCP server per environment). What an environment may do is what it declares ([`Env::needs`] refuses a capability it
//! does not, naming it) and what it grants; the host adds no limit of its own.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "the catalog is the host's"
)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use canoa::environment::{
    Call, CallAnswer, DirEntry, Environment, EnvironmentInfo, ListDir, Listing, ReadFile, State,
    WriteFile, capability,
};
use coqui_acp_api::guatiao::library::{Object, ProviderError, Registry};
use coqui_acp_api::guatiao::{Alloc, Provider};
use coqui_acp_api::{ProcessSpawner, SpawnSpec};
use indexmap::IndexMap;
use serde_json::{Map, Value};

use crate::config::{Config, EnvironmentConfig};

pub(crate) mod agent;
pub(crate) mod layer;
pub(crate) mod mcp;
pub(crate) mod remote;
pub(crate) mod served;
#[cfg(test)]
mod tests;

/// The name of the automatic environment: this computer, where the host runs.
pub(crate) const HERE: &str = "here";

/// The `_meta` key of `initialize` that says how the connection's sessions use environments:
/// `{"run": NAME, "serve": NAME, "tools": [NAME, ...]}`, each part optional.
pub(crate) const META: &str = "dev.boriken.canoa/environments";

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// This computer: programs through the host's own spawner, files through the operating system.
#[derive(Default, Provider)]
#[provider(Environment, id = "here", crate = ::coqui_acp_api::guatiao)]
pub(crate) struct Here;

impl Environment for Here {
    fn describe(&self) -> Result<EnvironmentInfo, ProviderError> {
        let mut info = EnvironmentInfo::new(
            "This computer",
            std::env::consts::OS,
            std::env::consts::ARCH,
        );
        info.local = true;
        info = info
            .offering(canoa::environment::Capability::new(capability::EXEC))
            .offering(canoa::environment::Capability::new(capability::TERMINAL))
            .offering(canoa::environment::Capability::new(capability::FS_READ))
            .offering(canoa::environment::Capability::new(capability::FS_WRITE))
            .offering(canoa::environment::Capability::new(capability::FS_LIST));
        Ok(info)
    }

    fn spawner(&self) -> Result<Object<dyn ProcessSpawner>, ProviderError> {
        Ok(coqui_acp_harness::LocalSpawner::new().into_object())
    }

    fn read_file(
        &self,
        request: ReadFile,
    ) -> Result<canoa::environment::FileContent, ProviderError> {
        let path = std::path::Path::new(&request.path);
        let meta = std::fs::metadata(path)
            .map_err(|error| canoa::environment::not_found(error.to_string()))?;
        if !meta.is_file() {
            return Err(canoa::environment::not_found("not a file"));
        }
        if meta.len() > request.max_bytes {
            return Err(canoa::environment::bad_value(format!(
                "the file is {} bytes, more than the {} asked for",
                meta.len(),
                request.max_bytes
            )));
        }
        std::fs::read(path)
            .map(canoa::environment::FileContent::new)
            .map_err(|error| canoa::environment::not_found(error.to_string()))
    }

    fn write_file(&self, request: WriteFile) -> Result<(), ProviderError> {
        let path = std::path::Path::new(&request.path);
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)
                .map_err(|error| canoa::environment::internal(error.to_string()))?;
        }
        std::fs::write(path, &request.bytes.0)
            .map_err(|error| canoa::environment::internal(error.to_string()))
    }

    fn list_dir(&self, request: ListDir) -> Result<Listing, ProviderError> {
        let read = std::fs::read_dir(&request.path)
            .map_err(|error| canoa::environment::not_found(error.to_string()))?;
        let mut entries = Vec::new();
        for entry in read.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            match entry.metadata() {
                Ok(meta) if meta.is_dir() => entries.push(DirEntry::folder(name)),
                Ok(meta) => entries.push(DirEntry::file(name, meta.len())),
                Err(_) => entries.push(DirEntry::file(name, 0)),
            }
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(Listing::new(entries))
    }

    fn call(&self, request: Call) -> Result<CallAnswer, ProviderError> {
        Err(canoa::environment::unsupported(format!(
            "this computer has no capability {}",
            request.capability
        )))
    }
}

coqui_acp_api::guatiao::local_providers!(
    id = "canoa-environments",
    version = env!("CARGO_PKG_VERSION"),
    providers = [Here]
);

/// The origin of the providers the host carries.
pub(crate) const BUILT_IN: &str = "built-in";

/// One provider of the `environment` kind a registry offers.
struct Offered {
    registry: Arc<Registry>,
    key: String,
    id: String,
}

fn to_guatiao(settings: &Map<String, Value>) -> Result<coqui_acp_api::guatiao::Value, String> {
    guatiao_serde::adapt::to_value_in(&Value::Object(settings.clone()), Alloc::rust())
        .map_err(|_| "the settings cannot be read".to_owned())
}

/// The environments of a host: `here`, and the ones canoa.cfg names, each made by the provider its `kind` names the first time it
/// is wanted and kept, so that what an instance holds (a link, a proof) is held once.
pub(crate) struct Environments {
    configured: IndexMap<String, EnvironmentConfig>,
    /// What the providers come from, until the first environment that needs one is made: loading a library costs, so a host
    /// that names no environment never does.
    pending: Mutex<Option<Pending>>,
    offered: Mutex<Vec<Offered>>,
    live: Mutex<BTreeMap<String, Arc<Env>>>,
    /// `[host] max_message_bytes`: a file read is answered in one message, so it is the longest file the host reads.
    message_bytes: usize,
}

/// Where the providers are found.
enum Pending {
    /// The libraries of this configuration's entries and extensions, and the bundled ones.
    Config(Box<Config>),
    /// Registries already loaded.
    #[cfg(test)]
    Registries(Vec<(String, Registry)>),
}

impl core::fmt::Debug for Environments {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list().entries(self.names()).finish()
    }
}

impl Environments {
    /// The environments of `config`, from the providers of the bundled libraries and of the libraries its entries and extensions load.
    pub(crate) fn of(config: &Config) -> Arc<Self> {
        Arc::new(Self {
            configured: config.environments.clone(),
            pending: Mutex::new(Some(Pending::Config(Box::new(config.clone())))),
            offered: Mutex::new(Vec::new()),
            live: Mutex::new(BTreeMap::new()),
            message_bytes: config.max_message_bytes,
        })
    }

    /// The environments `configured` names, made by the built-in provider and by those `registries` offer, each registry as its
    /// origin names it. A provider whose id an earlier one took is left out and logged.
    #[cfg(test)]
    pub(crate) fn from_registries(
        configured: IndexMap<String, EnvironmentConfig>,
        registries: Vec<(String, Registry)>,
        message_bytes: usize,
    ) -> Arc<Self> {
        Arc::new(Self {
            configured,
            pending: Mutex::new(Some(Pending::Registries(registries))),
            offered: Mutex::new(Vec::new()),
            live: Mutex::new(BTreeMap::new()),
            message_bytes,
        })
    }

    /// Finds the providers, once.
    fn load(&self) {
        let Some(pending) = lock(&self.pending).take() else {
            return;
        };
        let registries = match pending {
            Pending::Config(config) => {
                let mut registries = crate::control::providers::loaded(&config).to_vec();
                for (name, environment) in &config.environments {
                    let Some(origin) = &environment.origin else {
                        continue;
                    };
                    let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
                    match crate::provider::open(&mut registry, origin) {
                        Ok(()) => registries.push((name.clone(), Arc::new(registry))),
                        Err(why) => {
                            tracing::warn!(environment = name, "its provider is not loaded: {why}")
                        }
                    }
                }
                registries
            }
            #[cfg(test)]
            Pending::Registries(registries) => registries
                .into_iter()
                .map(|(name, registry)| (name, Arc::new(registry)))
                .collect(),
        };
        let mut found: Vec<(String, Arc<Registry>)> = Vec::new();
        let mut built_in = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
        let accepted = built_in
            .register_local("canoa-environments", library)
            .is_ok_and(|loading| loading.loaded().is_some());
        if accepted {
            found.push((BUILT_IN.to_owned(), Arc::new(built_in)));
        } else {
            tracing::warn!("the built-in environments are not offered");
        }
        found.extend(registries);
        let mut offered: Vec<Offered> = Vec::new();
        for (origin, registry) in found {
            let keys: Vec<String> = registry
                .offers::<dyn Environment>()
                .filter_map(|offer| offer.key().map(str::to_owned))
                .collect();
            for key in keys {
                let Some(Ok(offer)) = registry.offer::<dyn Environment>(&key) else {
                    continue;
                };
                let id = offer.id().to_owned();
                if offered.iter().any(|each| each.id == id) {
                    tracing::warn!(
                        origin,
                        provider = id,
                        "the environment provider {id} is already offered"
                    );
                    continue;
                }
                offered.push(Offered {
                    registry: Arc::clone(&registry),
                    key,
                    id,
                });
            }
        }
        *lock(&self.offered) = offered;
    }

    /// The longest message, `[host] max_message_bytes`.
    pub(crate) fn message_bytes(&self) -> usize {
        self.message_bytes
    }

    /// `here` and then the names canoa.cfg gives, in its order.
    pub(crate) fn names(&self) -> Vec<String> {
        core::iter::once(HERE.to_owned())
            .chain(self.configured.keys().cloned())
            .collect()
    }

    /// The environment `name`, made on first use.
    ///
    /// # Errors
    ///
    /// In words for the client: no such environment, or its provider is not offered or refused its settings.
    pub(crate) fn get(&self, name: &str) -> Result<Arc<Env>, String> {
        let mut live = lock(&self.live);
        if let Some(found) = live.get(name) {
            return Ok(Arc::clone(found));
        }
        let provider: Arc<dyn Environment> = if name == HERE {
            Arc::new(Here)
        } else {
            let config = self
                .configured
                .get(name)
                .ok_or_else(|| format!("this host has no environment {name}"))?;
            self.make(&config.kind, config.settings.as_map())
                .map_err(|why| format!("environment {name}: {why}"))?
        };
        let exec_seconds = self
            .configured
            .get(name)
            .and_then(|config| config.exec_seconds)
            .unwrap_or(crate::config::DEFAULT_EXEC_SECONDS);
        let table = self.configured.get(name);
        let env = Arc::new(Env {
            name: name.to_owned(),
            provider,
            exec_seconds,
            message_bytes: self.message_bytes,
            output_bytes: limit_of(
                table.and_then(|config| config.output_bytes),
                crate::config::DEFAULT_OUTPUT_BYTES,
            ),
            scrollback_bytes: limit_of(
                table.and_then(|config| config.scrollback_bytes),
                crate::config::DEFAULT_SCROLLBACK_BYTES,
            ),
            max_tool_sessions: limit_of(
                table.and_then(|config| config.max_tool_sessions),
                crate::config::DEFAULT_MAX_TOOL_SESSIONS,
            ),
        });
        live.insert(name.to_owned(), Arc::clone(&env));
        Ok(env)
    }

    fn make(
        &self,
        kind: &str,
        settings: &Map<String, Value>,
    ) -> Result<Arc<dyn Environment>, String> {
        self.load();
        let offered = lock(&self.offered);
        let offered = offered
            .iter()
            .find(|offered| offered.id == kind)
            .ok_or_else(|| format!("this host offers no environment kind {kind}"))?;
        let offer = offered
            .registry
            .offer::<dyn Environment>(&offered.key)
            .ok_or_else(|| format!("the provider {kind} is gone"))?
            .map_err(|_| format!("{kind} is not an environment"))?;
        if offer.builds_instances() {
            let value = to_guatiao(settings)?;
            let built = offer
                .instantiate(&value)
                .map_err(|error| error.to_string())?;
            Ok(Arc::new(built))
        } else if settings.is_empty() {
            offer
                .shared()
                .ok_or_else(|| format!("{kind} needs settings"))
        } else {
            Err(format!("{kind} takes no settings"))
        }
    }
}

/// One environment as the host uses it: its provider, behind the checks that refuse what it does not offer.
pub(crate) struct Env {
    name: String,
    provider: Arc<dyn Environment>,
    /// `exec_seconds` of its table; `0` is no limit.
    exec_seconds: u64,
    /// `[host] max_message_bytes`.
    message_bytes: usize,
    /// `output_bytes`, `scrollback_bytes` and `max_tool_sessions` of its table; `usize::MAX` is no limit.
    output_bytes: usize,
    scrollback_bytes: usize,
    max_tool_sessions: usize,
}

/// A limit as a table sets it: `0` is none.
fn limit_of(set: Option<u64>, default: u64) -> usize {
    match set.unwrap_or(default) {
        0 => usize::MAX,
        n => usize::try_from(n).unwrap_or(usize::MAX),
    }
}

impl core::fmt::Debug for Env {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Env").field("name", &self.name).finish()
    }
}

impl Env {
    /// How long the `exec` tool waits for a program, in seconds; `0` waits for ever.
    pub(crate) fn exec_seconds(&self) -> u64 {
        self.exec_seconds
    }

    /// How much output an agent's terminal keeps when it names no limit.
    pub(crate) fn output_bytes(&self) -> usize {
        self.output_bytes
    }

    /// How much an interactive terminal's output the tools keep.
    pub(crate) fn scrollback_bytes(&self) -> usize {
        self.scrollback_bytes
    }

    /// How many sessions of a connection may be given this environment's tools.
    pub(crate) fn max_tool_sessions(&self) -> usize {
        self.max_tool_sessions
    }

    /// Its name in canoa.cfg, or `here`.
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    /// What it says of itself now; one whose provider could not answer is described as down, with why.
    pub(crate) fn info(&self) -> EnvironmentInfo {
        match self.provider.describe() {
            Ok(info) => info,
            Err(error) => {
                let mut down = EnvironmentInfo::new(self.name.clone(), "", "");
                down.state = State::Down;
                down.detail = Some(error.to_string());
                down
            }
        }
    }

    /// Its description, when it can be used and offers `name`.
    ///
    /// # Errors
    ///
    /// In words, naming the capability: the environment is down or waiting, or does not offer it.
    pub(crate) fn needs(&self, name: &str) -> Result<EnvironmentInfo, String> {
        let info = self.info();
        match info.state {
            State::Ready => {}
            State::Waiting => {
                return Err(format!(
                    "environment {} is waiting: {}",
                    self.name,
                    info.detail
                        .as_deref()
                        .unwrap_or("it has not approved this host")
                ));
            }
            _ => {
                return Err(format!(
                    "environment {} is down: {}",
                    self.name,
                    info.detail.as_deref().unwrap_or("it cannot be reached")
                ));
            }
        }
        if info.offers(name) {
            Ok(info)
        } else {
            let has: Vec<&str> = info
                .capabilities
                .iter()
                .map(|each| each.name.as_str())
                .collect();
            Err(format!(
                "environment {} has no capability {name} (it offers {})",
                self.name,
                if has.is_empty() {
                    "nothing".to_owned()
                } else {
                    has.join(", ")
                }
            ))
        }
    }

    /// The spawner programs are started through there.
    ///
    /// # Errors
    ///
    /// The environment does not offer `exec`, or its provider refused.
    pub(crate) fn spawner(&self) -> Result<Object<dyn ProcessSpawner>, String> {
        self.needs(capability::EXEC)?;
        self.provider
            .spawner()
            .map_err(|error| format!("environment {}: {error}", self.name))
    }

    /// The spawner a shell is started through, when the environment offers `terminal`.
    ///
    /// # Errors
    ///
    /// The environment does not offer `terminal`, or its provider refused.
    pub(crate) fn terminal_spawner(&self) -> Result<Object<dyn ProcessSpawner>, String> {
        self.needs(capability::TERMINAL)?;
        self.provider
            .spawner()
            .map_err(|error| format!("environment {}: {error}", self.name))
    }

    /// A file's bytes.
    ///
    /// # Errors
    ///
    /// The environment does not offer `fs.read`, or the file cannot be read there.
    pub(crate) fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        self.needs(capability::FS_READ)?;
        self.provider
            .read_file(ReadFile::new(path, self.message_bytes as u64))
            .map(|content| content.bytes.0)
            .map_err(|error| {
                format!(
                    "environment {}: {error} (a file is read into one message, at most [host] max_message_bytes = {})",
                    self.name, self.message_bytes
                )
            })
    }

    /// Writes a file there.
    ///
    /// # Errors
    ///
    /// The environment does not offer `fs.write`, or the file cannot be written there.
    pub(crate) fn write(&self, path: &str, bytes: Vec<u8>) -> Result<(), String> {
        self.needs(capability::FS_WRITE)?;
        self.provider
            .write_file(WriteFile::new(path, bytes))
            .map_err(|error| format!("environment {}: {error}", self.name))
    }

    /// A folder's entries.
    ///
    /// # Errors
    ///
    /// The environment does not offer `fs.list`, or the folder cannot be listed there.
    pub(crate) fn list(&self, path: &str) -> Result<Vec<DirEntry>, String> {
        self.needs(capability::FS_LIST)?;
        self.provider
            .list_dir(ListDir::new(path))
            .map(|listing| listing.entries)
            .map_err(|error| format!("environment {}: {error}", self.name))
    }

    /// A capability of the environment's own, by its name.
    ///
    /// # Errors
    ///
    /// The environment does not offer `name`, or its provider refused.
    pub(crate) fn call(&self, name: &str, arguments: &str) -> Result<String, String> {
        self.needs(name)?;
        self.provider
            .call(Call::new(name, arguments))
            .map(|answer| answer.text)
            .map_err(|error| format!("environment {}: {error}", self.name))
    }

    /// Starts `spec` there and hands over the process.
    ///
    /// # Errors
    ///
    /// The environment does not offer `exec`, or the spawner refused.
    pub(crate) fn start(
        &self,
        spec: SpawnSpec,
    ) -> Result<Object<dyn coqui_acp_api::SpawnedProcess>, String> {
        let spawner = self.spawner()?;
        spawner
            .spawn(spec)
            .map_err(|error| format!("environment {}: {error}", self.name))
    }
}

/// How the connection's sessions use environments: what `initialize`'s `_meta` said ([`META`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Placement {
    /// The environment the agent runs in; `None` runs it where the entry says, which is `here` unless it names another.
    pub(crate) run: Option<String>,
    /// The environment the agent's file and terminal tools are served from.
    pub(crate) serve: Option<String>,
    /// The environments offered to the agent as tools, one MCP server each.
    pub(crate) tools: Vec<String>,
}

impl Placement {
    /// What `first`, the client's first message, asks.
    ///
    /// # Errors
    ///
    /// In words for the client: the key is there and is not what it should be.
    pub(crate) fn in_message(first: &[u8]) -> Result<Self, String> {
        if !first
            .windows(META.len())
            .any(|window| window == META.as_bytes())
        {
            return Ok(Self::default());
        }
        let Ok(message) = serde_json::from_slice::<Value>(first) else {
            return Ok(Self::default());
        };
        let Some(asked) = message
            .pointer("/params/_meta")
            .and_then(|meta| meta.get(META))
        else {
            return Ok(Self::default());
        };
        let name = |key: &str| -> Result<Option<String>, String> {
            match asked.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(name)) if !name.is_empty() => Ok(Some(name.clone())),
                Some(_) => Err(format!("{META}: {key} is the name of an environment")),
            }
        };
        let tools = match asked.get("tools") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(names)) => names
                .iter()
                .map(|each| {
                    each.as_str()
                        .filter(|name| !name.is_empty())
                        .map(str::to_owned)
                        .ok_or_else(|| format!("{META}: tools names environments"))
                })
                .collect::<Result<Vec<_>, _>>()?,
            Some(_) => return Err(format!("{META}: tools is a list of environment names")),
        };
        Ok(Self {
            run: name("run")?,
            serve: name("serve")?,
            tools,
        })
    }

    /// Whether the connection uses no environment but where it runs.
    pub(crate) fn is_plain(&self) -> bool {
        self.serve.is_none() && self.tools.is_empty()
    }
}

/// The text a session's agent is given about the environments it has, so that it does not try what is not possible: for each, how
/// it is attached, what it offers and what it lacks of what an agent usually wants.
pub(crate) fn briefing(attached: &[(Arc<Env>, Vec<String>)]) -> String {
    let mut text = String::from("Environments available in this session:\n");
    for (env, ways) in attached {
        let info = env.info();
        text.push_str(&format!(
            "- {} ({}, {}/{}): {}.\n",
            env.name(),
            info.title,
            info.os,
            info.arch,
            ways.join("; ")
        ));
        match info.state {
            State::Ready => {}
            State::Waiting => text.push_str(&format!(
                "  Not usable yet: waiting. {}\n",
                info.detail.as_deref().unwrap_or("")
            )),
            _ => text.push_str(&format!(
                "  Not usable now: down. {}\n",
                info.detail.as_deref().unwrap_or("")
            )),
        }
        let offers: Vec<String> = info
            .capabilities
            .iter()
            .map(|each| {
                if each.facts.is_empty() {
                    each.name.clone()
                } else {
                    let facts: Vec<String> =
                        each.facts.iter().map(|(k, v)| format!("{k}={v}")).collect();
                    format!("{} ({})", each.name, facts.join(", "))
                }
            })
            .collect();
        text.push_str(&format!(
            "  Offers: {}.\n",
            if offers.is_empty() {
                "nothing".to_owned()
            } else {
                offers.join(", ")
            }
        ));
        let mut lacks: Vec<String> = Vec::new();
        if !info.offers(capability::EXEC) {
            lacks.push("no way to run a program here".to_owned());
        }
        if !info.offers(capability::TERMINAL) {
            lacks.push("no interactive shell here".to_owned());
        }
        match (
            info.offers(capability::FS_READ),
            info.offers(capability::FS_WRITE),
        ) {
            (false, false) => lacks.push("no files here".to_owned()),
            (true, false) => lacks.push("read-only files here".to_owned()),
            (false, true) => lacks.push("files can be written here and not read".to_owned()),
            (true, true) => {}
        }
        if !info.offers(capability::FS_LIST) && info.offers(capability::FS_READ) {
            lacks.push("folders cannot be listed here".to_owned());
        }
        if !lacks.is_empty() {
            text.push_str(&format!("  Lacks: {}.\n", lacks.join("; ")));
        }
    }
    text.push_str(
        "Do not try what an environment lacks: the call is refused and names the missing capability.\n",
    );
    text
}

/// What a connection runs in and the link that gives its sessions the environments they use.
#[derive(Debug, Default)]
pub(crate) struct Attached {
    /// Where the agent runs, when that is not this computer.
    pub(crate) run: Option<Arc<Env>>,
    /// The link of what its sessions use besides, or of how they are told where the agent runs.
    pub(crate) link: Option<Arc<mcp::Link>>,
}

/// What the connection whose first message is `first` runs in and uses. `default_run` is the environment the entry runs its agent in;
/// the client's `_meta` may name another. `here` is no placement: the host runs the agent as it always has.
///
/// # Errors
///
/// In words for the client: `_meta` is not of the shape, or names an environment this host does not have.
pub(crate) fn attach(
    hub: &Arc<mcp::Hub>,
    default_run: Option<&str>,
    harness: Option<&str>,
    first: &[u8],
) -> Result<Attached, String> {
    let placement = Placement::in_message(first)?;
    let run = match placement.run.as_deref().or(default_run) {
        Some(name) if name != HERE => Some(hub.environments().get(name)?),
        _ => None,
    };
    let own_tools = harness.is_some_and(crate::bundled::does_own_tool_work);
    let link = hub.for_connection(run.clone(), &placement, own_tools)?;
    Ok(Attached { run, link })
}
