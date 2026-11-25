//! What the loaded agents say of their vendors, kept for the host's own decisions (which vendor stamps a session, which variable
//! holds its model, which credential standards are whose): the bundled agents' descriptions, and those of every outside library
//! the host has opened. Nothing here names an agent; each fact is a description's.
//!
//! A description is kept for the life of the process: the strings are leaked once, so a lookup answers `&'static str`. The set is
//! the few agents a host loads, and a description already kept is not kept again.

use alloc::sync::Arc;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, PoisonError};

use canoa::store::ProgramStore;
use canoa::vendor::{AgentVendor, VendorInfo};
use coqui_acp_api::guatiao::library::Registry;

/// How an agent's own login stands on this computer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Login {
    /// A variable of the environment signs it in; the program takes it in place of a stored login. Holds the variable's name.
    Key(&'static str),
    /// A login file is there.
    File,
    /// None of its login files is there and no variable is set: nobody is signed in.
    Missing,
}

/// One agent's description, kept.
#[derive(Debug, Clone)]
pub(crate) struct Vendor {
    /// The id the agent goes by.
    pub(crate) agent: &'static str,
    /// Its name for people.
    pub(crate) title: &'static str,
    /// The vendor a session on it is stamped with.
    pub(crate) vendor: Option<&'static str>,
    /// Its id in the standard registry of harnesses.
    pub(crate) aos: Option<&'static str>,
    /// The variable its running model is read from.
    pub(crate) model_variable: Option<&'static str>,
    /// Where it puts a tool's own name in a tool call's `_meta`, as a JSON pointer below `_meta`.
    pub(crate) tool_name: Option<&'static str>,
    /// The folder below a project where it reads skills from.
    pub(crate) skills_folder: Option<&'static str>,
    /// The files of a project's context it loads for itself.
    pub(crate) context_files: Vec<&'static str>,
    /// The variable that names the folder it keeps its own files in.
    pub(crate) home_variable: Option<&'static str>,
    /// That folder when the variable is not set, below the person's home.
    pub(crate) home_folder: Option<&'static str>,
    /// The files in that folder it loads as the person's own instructions.
    pub(crate) home_context_files: Vec<&'static str>,
    /// The files in that folder it keeps its own sign-in in.
    pub(crate) login_files: Vec<&'static str>,
    /// The variables of the environment that sign it in by themselves.
    pub(crate) login_env: Vec<&'static str>,
    /// The credential standards that are the vendor's own.
    pub(crate) standards: Vec<&'static str>,
    /// The names it reports in `initialize`.
    pub(crate) names: Vec<&'static str>,
}

fn keep(text: &str) -> &'static str {
    Box::leak(text.to_owned().into_boxed_str())
}

impl Vendor {
    /// The folder the agent keeps its own files in: where its variable names an absolute one, else its folder below `home`.
    pub(crate) fn folder(
        &self,
        home: Option<&Path>,
        env: &dyn Fn(&str) -> Option<OsString>,
    ) -> Option<PathBuf> {
        let named = self
            .home_variable
            .and_then(env)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute());
        named.or_else(|| Some(below(home?, self.home_folder?)))
    }

    /// How the agent's own login stands on this computer, from what its description says: a variable of `env` that signs it in and
    /// is not empty, else a login file that is there and not empty. `None` when nothing can be said: no variable is set and it
    /// lists no file (or has no folder to look in), so a login it keeps somewhere else (a keyring) is never reported missing. Only
    /// whether a variable is empty is read of it, never the value.
    pub(crate) fn login(
        &self,
        home: Option<&Path>,
        env: &dyn Fn(&str) -> Option<OsString>,
    ) -> Option<Login> {
        if let Some(variable) = self
            .login_env
            .iter()
            .find(|name| env(name).is_some_and(|value| !value.is_empty()))
        {
            return Some(Login::Key(variable));
        }
        if self.login_files.is_empty() {
            return None;
        }
        let folder = self.folder(home, env)?;
        let present = self.login_files.iter().any(|file| {
            std::fs::metadata(below(&folder, file))
                .is_ok_and(|meta| meta.is_file() && meta.len() > 0)
        });
        Some(if present { Login::File } else { Login::Missing })
    }

    fn of(info: &VendorInfo) -> Self {
        Self {
            agent: keep(&info.agent),
            title: keep(&info.title),
            vendor: info.vendor.as_deref().map(keep),
            aos: info.aos.as_deref().map(keep),
            model_variable: info.model_variable.as_deref().map(keep),
            tool_name: info.tool_name.as_deref().map(keep),
            skills_folder: info.skills_folder.as_deref().map(keep),
            context_files: info.context_files.iter().map(|path| keep(path)).collect(),
            home_variable: info.home_variable.as_deref().map(keep),
            home_folder: info.home_folder.as_deref().map(keep),
            home_context_files: info
                .home_context_files
                .iter()
                .map(|path| keep(path))
                .collect(),
            login_files: info.login_files.iter().map(|path| keep(path)).collect(),
            login_env: info.login_env.iter().map(|name| keep(name)).collect(),
            standards: info.standards.iter().map(|each| keep(&each.id)).collect(),
            names: info.names.iter().map(|name| keep(name)).collect(),
        }
    }
}

/// Process-wide interned table of loaded agent vendor descriptions (bundled vendors first, then extensions).
static KEPT: Mutex<Vec<Vendor>> = Mutex::new(Vec::new());
/// The `AgentVendor` provider each interned vendor was built from, by agent id.
static KEPT_PROVIDERS: Mutex<Vec<(&'static str, Arc<dyn AgentVendor>)>> = Mutex::new(Vec::new());
/// Ensures bundled agent vendor descriptions are registered once per process.
static BUNDLED: OnceLock<()> = OnceLock::new();

fn keep_all(registry: &Registry) {
    let mut kept = KEPT.lock().unwrap_or_else(PoisonError::into_inner);
    let mut providers = KEPT_PROVIDERS
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    for offer in registry.offers::<dyn AgentVendor>() {
        let Some(vendor) = offer.shared() else {
            continue;
        };
        let Some(info) = vendor
            .describe()
            .ok()
            .filter(|info| info.problem().is_none())
        else {
            continue;
        };
        crate::network::note_endpoints(info.endpoints.iter().cloned());
        if kept.iter().all(|each| each.agent != info.agent) {
            let record = Vendor::of(&info);
            providers.push((record.agent, Arc::clone(&vendor)));
            kept.push(record);
        }
    }
    crate::gateways::note(registry);
    for offer in registry.offers::<dyn ProgramStore>() {
        if let Some(info) = offer
            .default_instance()
            .and_then(|store| store.describe().ok())
        {
            crate::network::note_endpoints(info.endpoints.into_iter().chain(info.services));
        }
    }
}

/// The bundled agents' descriptions are kept first, whatever is noted.
fn bundled() {
    BUNDLED.get_or_init(|| {
        match crate::control::providers::bundled_registry(crate::bundled::ALL) {
            Ok(registry) => keep_all(&registry),
            Err(why) => tracing::warn!("the bundled agents' descriptions are not kept: {why}"),
        }
        match crate::store::registry::built_in() {
            Ok(registry) => keep_all(&registry),
            Err(why) => tracing::warn!("the built-in stores' endpoints are not kept: {why}"),
        }
    });
}

/// Keeps the descriptions `registry` offers that are not kept yet, and the addresses its agents and stores say their programs reach.
pub(crate) fn note(registry: &Registry) {
    bundled();
    keep_all(registry);
}

/// Keeps the bundled agents' and built-in stores' descriptions, which a caller that reads what they reach needs first.
pub(crate) fn load_bundled() {
    bundled();
}

/// Every description kept: the bundled agents', then the others' in the order they were opened.
pub(crate) fn all() -> Vec<Vendor> {
    bundled();
    KEPT.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

/// The agent whose session carries the AOS harness id `aos`.
pub(crate) fn by_aos(aos: &str) -> Option<Vendor> {
    all().into_iter().find(|each| each.aos == Some(aos))
}

/// The agent whose title is `title` (an entry named as the agent is).
pub(crate) fn by_title(title: &str) -> Option<Vendor> {
    all().into_iter().find(|each| each.title == title)
}

/// The agent that goes by `agent`.
pub(crate) fn by_agent(agent: &str) -> Option<Vendor> {
    all().into_iter().find(|each| each.agent == agent)
}

/// How the login the agent `agent` keeps for itself stands on this computer, as its description says where it is kept (the process's
/// own environment and the person's home); `None` when it says nothing, or the agent is not known.
pub(crate) fn login(agent: &str) -> Option<Login> {
    by_agent(agent)?.login(std::env::home_dir().as_deref(), &|name| {
        std::env::var_os(name)
    })
}

/// `file`, written with `/`, below `dir`.
pub(crate) fn below(dir: &Path, file: &str) -> PathBuf {
    file.split('/')
        .filter(|part| !part.is_empty())
        .fold(dir.to_path_buf(), |path, part| path.join(part))
}

/// The agent that reports itself as `name` in `initialize`.
pub(crate) fn by_name(name: &str) -> Option<Vendor> {
    all().into_iter().find(|each| each.names.contains(&name))
}

/// The agent a name means: its id, its registry id, the vendor's own name, or a standard of the vendor's.
pub(crate) fn resolve(name: &str) -> Option<Vendor> {
    all().into_iter().find(|each| {
        each.agent == name
            || each.aos == Some(name)
            || each.vendor == Some(name)
            || each.standards.contains(&name)
    })
}

/// Runs `f` on the `agent_vendor` provider of the bundled agent whose id in the standard registry of harnesses is `aos`; `None`
/// when no bundled agent has that id.
pub(crate) fn behave_aos<T>(aos: &str, f: impl FnOnce(&dyn AgentVendor) -> T) -> Option<T> {
    let known = by_aos(aos)?;
    behave(known.agent, f)
}

/// How the bundled agent `agent` is given a gateway the host runs for a session, when it can be.
pub(crate) fn gateway_route(agent: &str) -> Option<canoa::vendor::GatewayRoute> {
    behave(agent, |vendor| vendor.gateway_route())?.ok()?
}

/// Runs `f` on the `agent_vendor` provider of the bundled agent that reports itself as `name` in `initialize`.
pub(crate) fn behave_name<T>(name: &str, f: impl FnOnce(&dyn AgentVendor) -> T) -> Option<T> {
    let known = by_name(name)?;
    behave(known.agent, f)
}

/// Process-wide registry holding the bundled agent vendor providers, initialized once.
static BUNDLED_PROVIDERS: OnceLock<Option<Arc<Registry>>> = OnceLock::new();

/// The registry of the bundled agents' providers, made once and kept for the life of the process.
pub(crate) fn bundled_providers() -> Option<&'static Registry> {
    BUNDLED_PROVIDERS
        .get_or_init(|| {
            crate::control::providers::bundled_registry(crate::bundled::ALL)
                .map(Arc::new)
                .map_err(|why| tracing::warn!("the bundled agents' providers are not kept: {why}"))
                .ok()
        })
        .as_deref()
}

/// Shared handle to the bundled agents' registry.
pub(crate) fn bundled_providers_arc() -> Option<Arc<Registry>> {
    let _ = bundled_providers();
    BUNDLED_PROVIDERS
        .get()
        .and_then(|slot| slot.as_ref())
        .map(Arc::clone)
}

/// Whether the harness provider `harness` in `origin` (`None` for the bundled providers) declares a setting called `key`
/// in its settings schema. False when the registry cannot be opened or has no such provider; true when the provider publishes
/// no schema, whose keys are not known.
pub(crate) fn declares(origin: Option<&crate::config::Origin>, harness: &str, key: &str) -> bool {
    match origin {
        None => bundled_providers()
            .is_some_and(|registry| crate::kind::declares_setting(registry, harness, key)),
        Some(_) => crate::kind::registry(origin)
            .ok()
            .is_some_and(|registry| crate::kind::declares_setting(&registry, harness, key)),
    }
}

/// Runs `f` on the `agent_vendor` provider of `agent` (bundled or noted from a loaded library or extension), which is what
/// Canoa asks of an agent beyond its description (the variables an account sets, how a shell is chosen, where its sessions are
/// kept); `None` when no loaded agent goes by that id.
pub(crate) fn behave<T>(agent: &str, f: impl FnOnce(&dyn AgentVendor) -> T) -> Option<T> {
    bundled();
    let vendor = {
        let providers = KEPT_PROVIDERS
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        providers
            .iter()
            .find(|(id, _)| *id == agent)
            .map(|(_, vendor)| Arc::clone(vendor))
    }?;
    Some(f(&*vendor))
}

#[cfg(test)]
mod tests {
    use alloc::collections::BTreeMap;

    use super::*;

    /// A folder of the test's own, removed when it goes.
    struct Folder(PathBuf);

    impl Folder {
        fn new(name: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("canoa-vendors-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("a folder");
            Self(path)
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn signed_in(agent: &str, home: &Path, set: &[(&str, &Path)]) -> Option<Login> {
        let set: BTreeMap<String, OsString> = set
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.as_os_str().to_owned()))
            .collect();
        by_agent(agent)?.login(Some(home), &|name| set.get(name).cloned())
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn claude_is_signed_in_where_its_login_file_is_and_not_empty() {
        let home = Folder::new("claude-home");
        let there = home.0.join(".claude");
        assert_eq!(signed_in("claude", &home.0, &[]), Some(Login::Missing));
        std::fs::create_dir_all(&there).expect("its folder");
        std::fs::write(there.join(".credentials.json"), "").expect("empty");
        assert_eq!(
            signed_in("claude", &home.0, &[]),
            Some(Login::Missing),
            "an empty file"
        );
        std::fs::write(there.join(".credentials.json"), "{}").expect("a login");
        assert_eq!(signed_in("claude", &home.0, &[]), Some(Login::File));
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn the_variable_that_moves_an_agents_folder_is_where_its_login_is_looked_for() {
        let home = Folder::new("moved-home");
        let moved = Folder::new("moved-folder");
        std::fs::create_dir_all(home.0.join(".claude")).expect("its folder");
        std::fs::write(home.0.join(".claude").join(".credentials.json"), "{}").expect("a login");
        // The login below the home is not the one the variable points at.
        assert_eq!(
            signed_in("claude", &home.0, &[("CLAUDE_CONFIG_DIR", &moved.0)]),
            Some(Login::Missing)
        );
        std::fs::write(moved.0.join(".credentials.json"), "{}").expect("a login");
        assert_eq!(
            signed_in("claude", &home.0, &[("CLAUDE_CONFIG_DIR", &moved.0)]),
            Some(Login::File)
        );
    }

    #[test]
    fn codex_is_signed_in_where_its_auth_file_is() {
        let home = Folder::new("codex-home");
        let codex = Folder::new("codex-folder");
        assert_eq!(
            signed_in("codex", &home.0, &[("CODEX_HOME", &codex.0)]),
            Some(Login::Missing)
        );
        // Claude's file is not Codex's.
        std::fs::write(codex.0.join(".credentials.json"), "{}").expect("a file");
        assert_eq!(
            signed_in("codex", &home.0, &[("CODEX_HOME", &codex.0)]),
            Some(Login::Missing)
        );
        std::fs::write(codex.0.join("auth.json"), "{}").expect("a login");
        assert_eq!(
            signed_in("codex", &home.0, &[("CODEX_HOME", &codex.0)]),
            Some(Login::File)
        );
    }

    #[test]
    fn an_agent_that_lists_no_login_file_is_never_said_to_be_signed_out() {
        let home = Folder::new("none-home");
        for agent in ["pi", "antigravity"] {
            assert_eq!(signed_in(agent, &home.0, &[]), None, "{agent}");
        }
    }

    /// The login of `agent` with the variables `set` in its environment.
    fn with_variables(agent: &str, home: &Path, set: &[(&str, &str)]) -> Option<Login> {
        let set: BTreeMap<String, OsString> = set
            .iter()
            .map(|(name, value)| ((*name).to_owned(), OsString::from(value)))
            .collect();
        by_agent(agent)?.login(Some(home), &|name| set.get(name).cloned())
    }

    #[test]
    fn a_variable_that_signs_the_agent_in_is_a_login_whatever_files_there_are() {
        let home = Folder::new("key-home");
        let claude = [("ANTHROPIC_API_KEY", "CANARY-key")];
        assert_eq!(
            with_variables("claude", &home.0, &claude),
            Some(Login::Key("ANTHROPIC_API_KEY"))
        );
        let codex = [("OPENAI_API_KEY", "CANARY-key")];
        assert_eq!(
            with_variables("codex", &home.0, &codex),
            Some(Login::Key("OPENAI_API_KEY"))
        );
        // The program takes the key in place of a stored login, so it is the key that is told.
        std::fs::create_dir_all(home.0.join(".claude")).expect("its folder");
        std::fs::write(home.0.join(".claude").join(".credentials.json"), "{}").expect("a login");
        assert_eq!(
            with_variables("claude", &home.0, &claude),
            Some(Login::Key("ANTHROPIC_API_KEY"))
        );
        // A variable that is set and empty is no key, and another agent's variable is not this one's.
        let empty = [("ANTHROPIC_API_KEY", "")];
        assert_ne!(
            with_variables("claude", &home.0, &empty),
            Some(Login::Key("ANTHROPIC_API_KEY"))
        );
        assert_ne!(
            with_variables("codex", &home.0, &claude),
            Some(Login::Key("ANTHROPIC_API_KEY"))
        );
    }

    #[test]
    fn with_no_variable_and_no_file_an_agent_that_lists_files_is_signed_out() {
        let home = Folder::new("signed-out-home");
        if cfg!(not(target_os = "macos")) {
            assert_eq!(with_variables("claude", &home.0, &[]), Some(Login::Missing));
        }
        assert_eq!(with_variables("codex", &home.0, &[]), Some(Login::Missing));
    }
}
