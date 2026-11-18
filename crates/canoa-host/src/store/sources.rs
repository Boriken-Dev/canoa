//! The `catalog_source` providers this host offers: the two built in (`read` and `command`) and those the extensions of canoa.cfg
//! add, all found the same way, through a Guatiao registry.
//!
//! `read` names a place the Store type reads itself (a git source, an address or a file), so the host's own reader for the type does
//! the reading and the provider only describes the form. `command` runs a program the person named and takes what it prints.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files, processes and clocks are std's"
)]

use std::io::Read as _;
use std::sync::Arc;
use std::time::{Duration, Instant};

use canoa::store::recipe::{bad_value_error, internal_error};
use canoa::store::source::{CatalogSourceProvider, KIND, ReadRequest, SourceField, SourceInfo};
use coqui_acp_api::guatiao::Provider;
use coqui_acp_api::guatiao::library::{ProviderError, Registry};
use serde_json::{Map, Value};

use super::catalogs::READ;

/// The most bytes of a catalog a source answers with.
pub(crate) const MAX_BYTES: usize = 4 * 1024 * 1024;
/// How long `command` lets a program run, unless the form says otherwise, in seconds.
const DEFAULT_SECONDS: u64 = 60;
/// How much of what a failed program printed on its error stream is kept for the reason.
const TAIL_BYTES: usize = 2 * 1024;

/// The `read` provider: a place the Store type reads itself.
#[derive(Default, Provider)]
#[provider(CatalogSourceProvider, id = "read", crate = ::coqui_acp_api::guatiao)]
pub struct ReadSource;

impl CatalogSourceProvider for ReadSource {
    fn describe(&self) -> Result<SourceInfo, ProviderError> {
        let mut info = SourceInfo::new(READ, "Read an address or a file");
        info.help = Some(
            "A git repository, a web address or a file on this computer, in the form the type reads.".to_owned(),
        );
        let mut source = SourceField::new("source", "Source", "text").required();
        source.help = Some(
            "Overlays and extensions: <repository>@<ref>#<file>. Skills: owner/repo, a git URL or a folder. MCP servers: the \
             registry's address."
                .to_owned(),
        );
        info.fields = vec![source];
        Ok(info)
    }

    /// A file on this computer. A git source or an address is read by the Store type's own reader, which keeps what it fetched
    /// by commit and checks it, so it never asks this provider.
    fn read(&self, request: ReadRequest) -> Result<String, ProviderError> {
        let fields = fields_of(&request)?;
        let source = fields
            .get("source")
            .and_then(Value::as_str)
            .ok_or_else(|| bad_value_error("the source is needed"))?;
        let bytes = std::fs::read(source)
            .map_err(|error| internal_error(format!("{source} cannot be read: {error}")))?;
        text_of(bytes)
    }
}

/// The `command` provider: a program the person named, run on each refresh, whose standard output (or a file it writes) is the
/// catalog.
#[derive(Default, Provider)]
#[provider(CatalogSourceProvider, id = "command", crate = ::coqui_acp_api::guatiao)]
pub struct CommandSource;

impl CatalogSourceProvider for CommandSource {
    fn describe(&self) -> Result<SourceInfo, ProviderError> {
        let mut info = SourceInfo::new("command", "Run a program");
        info.help = Some(
            "A program and its arguments, run each time the catalog is refreshed, as you, with your environment. What it prints is \
             the catalog."
                .to_owned(),
        );
        // The catalogs the host itself indexes; Skills and MCP servers read theirs from the place they name.
        info.types = vec!["overlays".to_owned(), "extensions".to_owned()];
        let mut command = SourceField::new("command", "Program and arguments", "list").required();
        command.help = Some("The program first, then each argument.".to_owned());
        let mut cwd = SourceField::new("cwd", "Folder", "path");
        cwd.help = Some("Where it runs; the host's own folder when empty.".to_owned());
        let mut file = SourceField::new("file", "Output file", "path");
        file.help =
            Some("A file it writes, in its folder, to take instead of what it prints.".to_owned());
        let mut timeout = SourceField::new("timeout", "Seconds it may run", "number");
        timeout.default = Some(DEFAULT_SECONDS.to_string());
        info.fields = vec![command, cwd, file, timeout];
        Ok(info)
    }

    fn read(&self, request: ReadRequest) -> Result<String, ProviderError> {
        let fields = fields_of(&request)?;
        run(&fields).map_err(internal_error)
    }
}

fn fields_of(request: &ReadRequest) -> Result<Map<String, Value>, ProviderError> {
    match serde_json::from_str::<Value>(&request.fields) {
        Ok(Value::Object(fields)) => Ok(fields),
        _ => Err(bad_value_error("the fields are a JSON object")),
    }
}

fn text_of(bytes: Vec<u8>) -> Result<String, ProviderError> {
    if bytes.len() > MAX_BYTES {
        return Err(internal_error(format!(
            "the catalog holds {} bytes, more than the {MAX_BYTES} this host reads",
            bytes.len()
        )));
    }
    String::from_utf8(bytes).map_err(|_| internal_error("the catalog is not text"))
}

/// Runs the program of `fields` and answers with the catalog, or why it did not produce one: what the program printed on its
/// error stream, last, when it failed.
fn run(fields: &Map<String, Value>) -> Result<String, String> {
    let words: Vec<&str> = fields
        .get("command")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let Some((program, args)) = words
        .split_first()
        .filter(|(program, _)| !program.is_empty())
    else {
        return Err("the command is a program, then its arguments".to_owned());
    };
    let seconds = match fields.get("timeout") {
        None | Some(Value::Null) => DEFAULT_SECONDS,
        Some(Value::Number(number)) => number
            .as_u64()
            .ok_or("the time it may run is a whole number of seconds")?,
        Some(Value::String(text)) => text
            .trim()
            .parse()
            .map_err(|_| "the time it may run is a whole number of seconds".to_owned())?,
        Some(_) => return Err("the time it may run is a whole number of seconds".to_owned()),
    };
    let dir = fields
        .get("cwd")
        .and_then(Value::as_str)
        .filter(|dir| !dir.is_empty());
    let file = fields
        .get("file")
        .and_then(Value::as_str)
        .filter(|file| !file.is_empty());

    let mut command = crate::spawn::Command::new(program);
    command.args(args);
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    command
        .stdin(crate::spawn::Stdio::Null)
        .stdout(crate::spawn::Stdio::Piped)
        .stderr(crate::spawn::Stdio::Piped)
        .own_group();
    let mut child = command
        .spawn()
        .map_err(|error| format!("{program} could not start: {error}"))?;
    let out = child.stdout.take().map(|mut stream| {
        std::thread::spawn(move || {
            let mut kept = Vec::new();
            // One byte past the most, so a catalog that is too long is known to be.
            let _ = (&mut stream)
                .take(MAX_BYTES as u64 + 1)
                .read_to_end(&mut kept);
            // Keep reading and dropping, or a program that prints on would wait for ever on a full pipe.
            let _ = std::io::copy(&mut stream, &mut std::io::sink());
            kept
        })
    });
    let err = child.stderr.take().map(|mut stream| {
        std::thread::spawn(move || {
            let mut kept: Vec<u8> = Vec::new();
            let mut chunk = [0_u8; 4096];
            while let Ok(read) = stream.read(&mut chunk) {
                if read == 0 {
                    break;
                }
                kept.extend_from_slice(&chunk[..read]);
                if kept.len() > TAIL_BYTES {
                    kept.drain(..kept.len() - TAIL_BYTES);
                }
            }
            String::from_utf8_lossy(&kept).trim().to_owned()
        })
    });
    let started = Instant::now();
    let limit = Duration::from_secs(seconds);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() < limit => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => break None,
        }
    };
    match status {
        Some(_) => child.end_group_after_exit(),
        None => {
            let _ = child.kill();
            child.end_group();
            let _ = child.wait();
        }
    }
    let printed = out.and_then(|each| each.join().ok()).unwrap_or_default();
    let tail = err.and_then(|each| each.join().ok()).unwrap_or_default();
    let why = |what: String| {
        if tail.is_empty() {
            what
        } else {
            format!("{what}:\n{tail}")
        }
    };
    match status {
        None => Err(why(format!("{program} was stopped after {seconds} s"))),
        Some(status) if !status.success() => Err(why(format!("{program} ended with {status}"))),
        Some(_) => {
            let bytes = match file {
                Some(file) => {
                    let path = dir.map_or_else(
                        || std::path::PathBuf::from(file),
                        |dir| std::path::Path::new(dir).join(file),
                    );
                    std::fs::read(&path)
                        .map_err(|error| format!("{} cannot be read: {error}", path.display()))?
                }
                None => printed,
            };
            if bytes.is_empty() {
                return Err(why(format!("{program} printed no catalog")));
            }
            text_of(bytes).map_err(|error| error.to_string())
        }
    }
}

coqui_acp_api::guatiao::local_providers!(
    id = "canoa-catalog-sources",
    version = env!("CARGO_PKG_VERSION"),
    providers = [ReadSource, CommandSource]
);

/// One provider this host offers.
#[derive(Clone)]
struct Held {
    registry: Arc<Registry>,
    /// The extension that brought it, none for a built-in one.
    extension: Option<String>,
    info: SourceInfo,
}

/// Every `catalog_source` provider this host offers: the built-in ones first, then each extension's in the order loaded.
#[derive(Clone, Default)]
pub(crate) struct Sources {
    held: Vec<Held>,
}

impl core::fmt::Debug for Sources {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list()
            .entries(self.held.iter().map(|each| &each.info.id))
            .finish()
    }
}

impl Sources {
    /// The providers built into this binary.
    pub(crate) fn built_in() -> Self {
        let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
        let loaded = registry
            .register_local("canoa-catalog-sources", library)
            .map(|loading| loading.loaded().is_some());
        let mut sources = Self::default();
        if loaded.is_ok_and(|loaded| loaded) {
            if let Err(why) = sources.add(None, Arc::new(registry)) {
                tracing::warn!("the built-in catalog sources were refused: {why}");
            }
        } else {
            tracing::warn!("the built-in catalog sources were not accepted");
        }
        sources
    }

    /// Takes every `catalog_source` provider of `registry`, brought by the extension `extension` or built in. Nothing is taken
    /// when one is refused, or has an id another already has.
    pub(crate) fn add(
        &mut self,
        extension: Option<&str>,
        registry: Arc<Registry>,
    ) -> Result<(), String> {
        let mut found = Vec::new();
        for provider in registry.providers(KIND) {
            let id = provider.id().to_owned();
            let offer = registry
                .offer::<dyn CatalogSourceProvider>(&id)
                .ok_or_else(|| format!("no provider {id}"))?
                .map_err(|_| format!("{id} is not a catalog_source"))?;
            let info = offer
                .default_instance()
                .ok_or_else(|| format!("{id} needs settings"))?
                .describe()
                .map_err(|error| error.to_string())?;
            if info.id != id {
                return Err(format!("{id} describes itself as {}", info.id));
            }
            if let Some(problem) = info.problem() {
                return Err(problem);
            }
            if self
                .held
                .iter()
                .map(|each| &each.info)
                .chain(found.iter().map(|each: &Held| &each.info))
                .any(|taken| taken.id == info.id)
            {
                return Err(format!("the source {id:?} is already offered"));
            }
            found.push(Held {
                registry: Arc::clone(&registry),
                extension: extension.map(str::to_owned),
                info,
            });
        }
        self.held.extend(found);
        Ok(())
    }

    /// Each provider and the extension that brought it.
    pub(crate) fn list(&self) -> impl Iterator<Item = (&SourceInfo, Option<&str>)> {
        self.held
            .iter()
            .map(|each| (&each.info, each.extension.as_deref()))
    }

    /// What the provider `id` describes, if this host offers it.
    #[cfg(test)]
    pub(crate) fn info(&self, id: &str) -> Option<&SourceInfo> {
        self.held
            .iter()
            .map(|each| &each.info)
            .find(|info| info.id == id)
    }

    /// The catalog the provider `id` answers with for `request`, or why it gave none.
    pub(crate) fn read(&self, id: &str, request: ReadRequest) -> Result<String, String> {
        let held = self
            .held
            .iter()
            .find(|each| each.info.id == id)
            .ok_or_else(|| format!("this host has no catalog source {id:?}"))?;
        if !held.info.serves(&request.kind) {
            return Err(format!(
                "the source {id:?} does not serve the type {:?}",
                request.kind
            ));
        }
        held.registry
            .offer::<dyn CatalogSourceProvider>(id)
            .ok_or_else(|| format!("no provider {id}"))?
            .map_err(|_| format!("{id} is not a catalog_source"))?
            .default_instance()
            .ok_or_else(|| format!("{id} needs settings"))?
            .read(request)
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
#[path = "sources_tests.rs"]
pub(crate) mod tests;
