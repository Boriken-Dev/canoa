//! `extension/plan`, `extension/apply` and `extension/remove`: an extension a catalog lists, put in the host's home and named in
//! canoa.cfg, and taken out again.
//!
//! An extension is code the host loads into its own process, so nothing of it is kept before the digest its catalog gave
//! has been computed over what was fetched, and nothing is fetched before the person confirmed a plan that shows the source, the
//! digest and the provider. The host reads the entry from its own index of the catalogs it was told to use: a client names the
//! catalog and the extension, never the source or the digest. The key needs the `hosting` grant, which already names the libraries
//! the host loads through `[host.extensions]`.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files, locks and the clock are std's"
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use canoa::store::extension::{KIND, KIND_VERSIONS};
use canoa::store::{Check, FetchRequest, InstallTools};
use serde_json::{Map, Value, json};
use sha2::{Digest as _, Sha256};

use super::overlays::{OverlayLimits, Trust};
use super::{Caller, Control, DENIED, Grant, REFUSED, bad};
use crate::store::extension_catalog::{Digest, Entry, Source, Unfit};
use crate::store::index::{Answer, Row, parse_extension_sources};
use crate::store::tools::{HttpTransport, LocalTools, Transport};

type Outcome = Result<Value, (i32, String)>;

/// The largest file an extension's URL may serve, in bytes.
const MAX_DOWNLOAD: u64 = 256 * 1024 * 1024;

/// What a tree extension may hold: it is a native library, often far over what an overlay may, so its files and the tree
/// each go up to what a URL extension may serve.
pub(super) fn tree_limits() -> OverlayLimits {
    OverlayLimits {
        bytes: MAX_DOWNLOAD,
        ..OverlayLimits::default()
    }
}

/// The same limits for reading a folder on disk, as the check at load time does.
pub(super) fn tree_settings() -> coqui_aos::Settings {
    coqui_aos::Settings::default()
        .tree_bytes(coqui_aos::Limit::at(MAX_DOWNLOAD))
        .file_bytes(coqui_aos::Limit::at(MAX_DOWNLOAD))
}

/// Where the host keeps the extensions it installed, and how it reaches the network to fetch one.
#[derive(Clone)]
pub(crate) struct ExtensionInstall {
    /// The folder each extension gets a folder of its own in; none when the host has no home.
    home: Option<PathBuf>,
    transport: Arc<dyn Transport>,
    loopback_http: bool,
    /// One install or removal at a time.
    working: Arc<Mutex<()>>,
}

impl Default for ExtensionInstall {
    fn default() -> Self {
        Self::new(None)
    }
}

impl core::fmt::Debug for ExtensionInstall {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ExtensionInstall")
            .field("home", &self.home)
            .finish_non_exhaustive()
    }
}

impl ExtensionInstall {
    /// Installs into folders of `home`, fetching files over HTTP and HTTPS.
    pub(crate) fn new(home: Option<PathBuf>) -> Self {
        Self {
            home,
            transport: Arc::new(HttpTransport::default()),
            loopback_http: false,
            working: Arc::new(Mutex::new(())),
        }
    }

    /// Fetches over `transport` instead, and lets `http` reach a loopback address when `loopback_http`: a test's setting.
    #[cfg(test)]
    pub(crate) fn with_transport(
        mut self,
        transport: Arc<dyn Transport>,
        loopback_http: bool,
    ) -> Self {
        self.transport = transport;
        self.loopback_http = loopback_http;
        self
    }
}

/// What the host holds of one extension a catalog lists, read again from its own index.
struct Wanted {
    catalog: String,
    name: String,
    entry: Entry,
    /// Whether the source is in the repository of the catalog the person added, which is fetched as that catalog is.
    own: bool,
    /// The text that stands for the plan: the source, the digest, the provider and the file loaded.
    token: String,
}

fn hex(bytes: &[u8]) -> String {
    data_encoding::HEXLOWER.encode(bytes)
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

/// A source as the person is shown it: the repository, `@ref` and `#folder` of a tree, or the URL of a file.
fn source_shown(source: &Source) -> String {
    match source {
        Source::Git {
            repo,
            reference,
            path,
        } => format!(
            "{repo}{}{}",
            reference
                .as_ref()
                .map(|each| format!("@{each}"))
                .unwrap_or_default(),
            path.as_ref()
                .map(|each| format!("#{each}"))
                .unwrap_or_default()
        ),
        Source::Url(url) => url.clone(),
    }
}

/// A file name for the URL's file: the last segment of its path, which is kept as a name only when it is a plain one.
fn file_name_of(url: &str) -> Result<String, (i32, String)> {
    let after = url.split_once("://").map_or(url, |(_, rest)| rest);
    let path = after.split(['?', '#']).next().unwrap_or_default();
    let name = path.rsplit('/').next().unwrap_or_default();
    let plain = !name.is_empty()
        && name.len() <= 128
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'+'));
    if plain {
        Ok(name.to_owned())
    } else {
        Err((
            REFUSED,
            "the URL does not end in a plain file name (letters, digits, `.`, `_`, `-` and `+`)"
                .to_owned(),
        ))
    }
}

impl Control {
    /// The extension catalogs `[store] extension_catalogs` names, each answered from the index: none until the table names one.
    fn extension_rows(&self) -> Vec<Row> {
        let section = self
            .files
            .config_get(&json!({"section": "store"}))
            .ok()
            .and_then(|answer| answer.pointer("/value/extension_catalogs").cloned())
            .unwrap_or(Value::Null);
        parse_extension_sources(&section)
    }

    fn extension_answers(&self, refresh: bool) -> Vec<Answer> {
        self.index.catalogs_spaced(
            &self.overlays,
            &self.extension_rows(),
            refresh,
            Duration::ZERO,
        )
    }

    /// `extension/catalogs` `{"refresh"?: bool}` -> `{"schema": 1, "sources": [...], "entries": [...]}`: the extensions catalogs of
    /// `[store] extension_catalogs`, shaped as `store/index` answers for `[catalogs]`, with each entry's `extension` (provider,
    /// digest, `load` and `needs`). A key reads them with `overlay.read`, and `refresh`, which fetches, takes `overlay.write`.
    pub(super) fn extension_catalogs(&self, caller: &Caller, params: &Value) -> Outcome {
        if !caller.may(Grant::OverlayRead) {
            return Err((
                DENIED,
                "this key does not hold the overlay.read grant".to_owned(),
            ));
        }
        let refresh = match params.get("refresh") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(value)) => *value,
            Some(_) => return Err(bad("refresh is true or false")),
        };
        if refresh && !caller.may(Grant::OverlayWrite) {
            return Err((
                DENIED,
                "refresh fetches sources, which takes the overlay.write grant".to_owned(),
            ));
        }
        let (mut sources, mut entries) = (Vec::new(), Vec::new());
        for mut answer in self.extension_answers(refresh) {
            answer.source["entries"] = json!(answer.entries.len());
            sources.push(answer.source);
            entries.append(&mut answer.entries);
        }
        Ok(json!({"schema": 1, "sources": sources, "entries": entries}))
    }

    /// The extension `name` of the catalog labelled `catalog`, as the host's index holds it, read again with every rule of
    /// the format and checked against what this host offers. A catalog the host was not told to use, a name it does not list
    /// or an entry it refused is refused here, so a client cannot name a source of its own.
    fn extension_wanted(&self, catalog: &str, name: &str) -> Result<Wanted, (i32, String)> {
        let found = self
            .extension_answers(false)
            .into_iter()
            .flat_map(|answer| answer.entries)
            .find(|entry| {
                text(entry, "kind") == Some("extension")
                    && text(entry, "catalog") == Some(catalog)
                    && text(entry, "name") == Some(name)
            })
            .ok_or_else(|| {
                (
                    REFUSED,
                    format!("no extensions catalog of this host lists {name} in {catalog}: refresh the Store"),
                )
            })?;
        if let Some(problem) = text(&found, "problem") {
            return Err((REFUSED, problem.to_owned()));
        }
        let entry = Entry::from_index(name, &found).map_err(|why| {
            (
                REFUSED,
                format!("the entry is not one this host reads ({})", why.as_str()),
            )
        })?;
        let kinds: &[(&str, &[&str])] = &[(KIND, &KIND_VERSIONS)];
        if let Err(unfit) = entry
            .needs
            .fits(kinds, std::env::consts::OS, std::env::consts::ARCH)
        {
            return Err((
                REFUSED,
                match unfit {
                    Unfit::Kind => "this host does not load that kind of extension".to_owned(),
                    Unfit::Version => {
                        "this host does not offer the version of the extension kind it needs"
                            .to_owned()
                    }
                    Unfit::Platform => format!(
                        "it does not run on {}-{}",
                        std::env::consts::OS,
                        std::env::consts::ARCH
                    ),
                },
            ));
        }
        if matches!(entry.digest, Digest::Tree(_)) && entry.load.is_none() {
            return Err((
                REFUSED,
                "this host loads an extension from one library file: the entry names none (`load`)"
                    .to_owned(),
            ));
        }
        crate::config::check_name(name).map_err(|error| (REFUSED, error.to_string()))?;
        let digest = match &entry.digest {
            Digest::Tree(pin) => pin.to_ascii_lowercase(),
            Digest::File(sha256) => sha256.to_ascii_lowercase(),
        };
        let token = hex(&Sha256::digest(
            format!(
                "{catalog}\n{name}\n{}\n{}\n{digest}\n{}",
                entry.provider,
                source_shown(&entry.source),
                entry.load.as_deref().unwrap_or_default()
            )
            .as_bytes(),
        ));
        let own = match &entry.source {
            Source::Git { repo, .. } => self
                .extension_rows()
                .into_iter()
                .flatten()
                .any(|row| row.label == catalog && &row.repo == repo),
            Source::Url(_) => false,
        };
        Ok(Wanted {
            catalog: catalog.to_owned(),
            name: name.to_owned(),
            entry,
            own,
            token,
        })
    }

    fn extension_names(params: &Value) -> Result<(String, String), (i32, String)> {
        let take = |key: &str| {
            params
                .get(key)
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .ok_or_else(|| bad(&format!("{key} is text")))
        };
        Ok((take("catalog")?, take("name")?))
    }

    /// `extension/plan` `{"catalog", "name"}` -> a plan the person reads before anything is fetched, shaped as `store/plan`'s so
    /// one dialog shows both: `{"action": "install", "downloads", "steps", "changes", "notices", "token", "extension"}`.
    pub(super) fn extension_plan(&self, params: &Value) -> Outcome {
        let (catalog, name) = Self::extension_names(params)?;
        let wanted = self.extension_wanted(&catalog, &name)?;
        let entry = &wanted.entry;
        let home = self.extensions.home.as_ref().ok_or_else(|| {
            (
                REFUSED,
                "this host has no home to keep an extension in".to_owned(),
            )
        })?;
        let (check, digest_text) = match &entry.digest {
            Digest::Tree(pin) => (
                pin.strip_prefix("sha256:").unwrap_or(pin).to_owned(),
                format!("the tree digest {pin}"),
            ),
            Digest::File(sha256) => (sha256.clone(), format!("the SHA-256 {sha256}")),
        };
        let folder = home.join(&name);
        let mut notices = vec![
            format!(
                "An extension is code the host loads into its own process: it runs with your access, as the host does. Install one only when you trust where it comes from. This one asks to be loaded as provider {}.",
                entry.provider
            ),
            format!(
                "Nothing is kept unless what is fetched has {digest_text}, the digest the catalog {catalog} gave. A tree's digest is the one Canoa uses for an overlay's pin, over the files of the folder."
            ),
            "The host starts again to load it.".to_owned(),
        ];
        if matches!(entry.source, Source::Git { .. }) {
            notices.push(
                if wanted.own {
                    "It is in the repository of the catalog you added, so it is fetched as that catalog is, with your own git settings."
                } else {
                    "It is fetched with none of your git settings, credentials or keys, since the catalog's author, not you, chose where it points."
                }
                .to_owned(),
            );
        }
        if folder.exists() {
            notices.push(
                "An extension of this name is installed: its files are replaced. If the running host has them loaded, remove it in Settings first and restart."
                    .to_owned(),
            );
        }
        if let Some(old) = self
            .files
            .host_get()
            .ok()
            .and_then(|host| host.pointer(&format!("/value/extensions/{name}")).cloned())
        {
            let from = old
                .get("library")
                .or_else(|| old.get("source"))
                .and_then(Value::as_str)
                .map_or_else(String::new, |from| format!(" (loaded from {from})"));
            notices.push(format!(
                "canoa.cfg already names an extension {name}{from}: this replaces it."
            ));
        }
        Ok(json!({
            "action": "install",
            "version": Value::Null,
            "downloads": [{
                "url": source_shown(&entry.source),
                "bytes": Value::Null,
                "check": {"kind": "sha256", "value": check},
            }],
            "steps": [],
            "changes": [
                {
                    "kind": "extension_files",
                    "target": folder.display().to_string(),
                    "detail": "The extension's files are kept here.",
                    "undone_by_uninstall": true,
                },
                {
                    "kind": "canoa_cfg",
                    "target": format!("[host.extensions.{name}]"),
                    "detail": "Names the library, its provider and the digest the host checks each time it loads it.",
                    "undone_by_uninstall": true,
                },
            ],
            "bytes": Value::Null,
            "notices": notices,
            "token": wanted.token,
            "extension": {"catalog": wanted.catalog, "name": wanted.name, "provider": entry.provider},
        }))
    }

    /// `extension/apply` `{"plan"}`: fetches what the plan names, computes its digest, and only when that is the catalog's keeps
    /// the files in `<home>/extensions/<name>/` and names the library in `[host.extensions.<name>]`. The host reads `[host]` only
    /// when it starts, so the answer says `restart: true`.
    pub(super) fn extension_apply(&self, params: &Value) -> Outcome {
        let plan = params
            .get("plan")
            .filter(|plan| plan.is_object())
            .ok_or_else(|| bad("plan is the plan extension/plan answered"))?;
        let given = plan.get("extension").unwrap_or(&Value::Null);
        let (catalog, name) = Self::extension_names(given)?;
        let wanted = self.extension_wanted(&catalog, &name)?;
        if text(plan, "token") != Some(wanted.token.as_str()) {
            return Err((
                REFUSED,
                "the catalog's entry changed since that plan was made: plan again".to_owned(),
            ));
        }
        let _one = self
            .extensions
            .working
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let home = self.extensions.home.clone().ok_or_else(|| {
            (
                REFUSED,
                "this host has no home to keep an extension in".to_owned(),
            )
        })?;
        fs::create_dir_all(&home)
            .map_err(|error| (REFUSED, format!("{}: {error}", home.display())))?;
        let mut random = [0_u8; 8];
        getrandom::fill(&mut random)
            .map_err(|_| (REFUSED, "no randomness to name the install".to_owned()))?;
        let staging = home.join(format!(".install-{}", hex(&random)));
        super::private_dir(&staging)
            .map_err(|error| (REFUSED, format!("{}: {error}", staging.display())))?;
        let made = self.stage_extension(&wanted, &staging);
        let library_name = match made {
            Ok(library) => library,
            Err(error) => {
                let _ = fs::remove_dir_all(&staging);
                return Err(error);
            }
        };
        let target = home.join(&wanted.name);
        if target.exists()
            && let Err(error) = fs::remove_dir_all(&target)
        {
            let _ = fs::remove_dir_all(&staging);
            return Err((
                REFUSED,
                format!(
                    "the files of the extension installed before cannot be replaced ({error}): if the running host has them loaded, remove the extension in Settings, restart the host and install again"
                ),
            ));
        }
        if let Err(error) = fs::rename(&staging, &target) {
            let _ = fs::remove_dir_all(&staging);
            return Err((REFUSED, format!("{}: {error}", target.display())));
        }
        let library = target.join(&library_name);
        let table = match &wanted.entry.digest {
            Digest::Tree(pin) => json!({
                "library": library.display().to_string(),
                "provider": wanted.entry.provider,
                "pin": pin,
                "root": target.display().to_string(),
            }),
            Digest::File(sha256) => json!({
                "library": library.display().to_string(),
                "provider": wanted.entry.provider,
                "sha256": sha256,
            }),
        };
        if let Err(error) = self.change_extensions(|extensions| {
            extensions.insert(wanted.name.clone(), table);
            Ok(())
        }) {
            let _ = fs::remove_dir_all(&target);
            return Err(error);
        }
        Ok(json!({
            "installed": true,
            "name": wanted.name,
            "path": library.display().to_string(),
            "restart": true,
        }))
    }

    /// Fetches the extension into `staging` and checks it; the library's name inside it.
    fn stage_extension(&self, wanted: &Wanted, staging: &Path) -> Result<String, (i32, String)> {
        match (&wanted.entry.source, &wanted.entry.digest) {
            (Source::Url(url), Digest::File(sha256)) => {
                let name = file_name_of(url)?;
                let tools = LocalTools::new(Arc::clone(&self.extensions.transport), Vec::new())
                    .granted(staging)
                    .anywhere(true)
                    .loopback_http(self.extensions.loopback_http);
                tools
                    .fetch(FetchRequest::new(
                        url.clone(),
                        name.clone(),
                        Check::Sha256 {
                            value: sha256.clone(),
                        },
                        MAX_DOWNLOAD,
                    ))
                    .map_err(|error| {
                        (REFUSED, format!("the download failed: {}", error.message()))
                    })?;
                Ok(name)
            }
            (
                Source::Git {
                    repo,
                    reference,
                    path,
                },
                Digest::Tree(pin),
            ) => {
                let load = wanted.entry.load.clone().unwrap_or_default();
                let tree = self.overlays.fetch_tree(
                    repo,
                    reference.as_deref(),
                    path.as_deref().unwrap_or(""),
                    if wanted.own {
                        Trust::Own
                    } else {
                        Trust::Isolated
                    },
                    &tree_limits(),
                )?;
                if !tree.digest.eq_ignore_ascii_case(pin) {
                    return Err((
                        REFUSED,
                        format!(
                            "the files the source holds do not have the tree digest the catalog gave: they have {}, the catalog gave {pin}",
                            tree.digest,
                        ),
                    ));
                }
                if !tree.files.contains_key(&load) {
                    return Err((REFUSED, format!("the source holds no file {load} to load")));
                }
                for (path, bytes) in &tree.files {
                    if !coqui_aos::valid_relative_path(path) {
                        return Err((
                            REFUSED,
                            "the source holds a name that is not a plain path".to_owned(),
                        ));
                    }
                    let file = staging.join(path);
                    if let Some(parent) = file.parent() {
                        fs::create_dir_all(parent)
                            .map_err(|error| (REFUSED, format!("{}: {error}", parent.display())))?;
                    }
                    fs::write(&file, bytes)
                        .map_err(|error| (REFUSED, format!("{}: {error}", file.display())))?;
                }
                Ok(load)
            }
            _ => Err((
                REFUSED,
                "the entry's digest is not the kind its source takes".to_owned(),
            )),
        }
    }

    /// Rewrites `[host.extensions]` through `[host]`'s own check and write, so canoa.cfg is changed only to a file the host
    /// accepts and another page's change to the table in between is refused, not overwritten.
    fn change_extensions(
        &self,
        edit: impl FnOnce(&mut Map<String, Value>) -> Result<(), (i32, String)>,
    ) -> Result<(), (i32, String)> {
        let current = self.files.host_get()?;
        let digest = current.get("digest").cloned().unwrap_or(Value::Null);
        let mut host = current
            .get("value")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let mut extensions = host
            .get("extensions")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        edit(&mut extensions)?;
        if extensions.is_empty() {
            host.remove("extensions");
        } else {
            host.insert("extensions".into(), Value::Object(extensions));
        }
        let value = if host.is_empty() {
            Value::Null
        } else {
            Value::Object(host)
        };
        let request = json!({"value": value, "digest": digest});
        self.files.host_set(&request, false)?;
        self.files.host_set(&request, true)?;
        Ok(())
    }

    /// `extension/remove` `{"name"}`: takes `[host.extensions.<name>]` out of canoa.cfg and, when its library is in the folder this
    /// host made for it, that folder. Files the running host has loaded may stay until it restarts (`files_removed` false).
    pub(super) fn extension_remove(&self, params: &Value) -> Outcome {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| bad("name is text"))?
            .to_owned();
        let _one = self
            .extensions
            .working
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let current = self.files.host_get()?;
        let library = current
            .pointer("/value/extensions")
            .and_then(|table| table.get(&name))
            .ok_or_else(|| {
                (
                    REFUSED,
                    format!("there is no extension {name} in canoa.cfg"),
                )
            })?
            .get("library")
            .and_then(Value::as_str)
            .map(PathBuf::from);
        self.change_extensions(|extensions| {
            extensions.remove(&name);
            Ok(())
        })?;
        // Only a folder this host made, and only by the name it was made under.
        let mut removed = false;
        if let (Some(home), Some(library)) = (&self.extensions.home, library) {
            let folder = home.join(&name);
            if crate::config::check_name(&name).is_ok() && library.starts_with(&folder) {
                removed = fs::remove_dir_all(&folder).is_ok();
            }
        }
        Ok(json!({"removed": true, "files_removed": removed, "restart": true}))
    }
}
