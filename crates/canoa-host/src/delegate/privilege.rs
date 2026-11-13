//! What a delegated task may be given, decided before any process starts: the source, the folder and the mode. Each is at most
//! what the delegating session has, and no argument of a task widens one.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "paths are std's"
)]

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{Host, Principal, Source};
use crate::config::DelegateConfig;

/// The modes that ask for everything, nearest first: what a task gets when its agent has not the delegating session's own.
pub(crate) const ASKING: [&str; 2] = ["manual", "default"];

/// The sources `who` may name in a task: the host's agents the connection may use, narrowed by `[host.delegate] sources`. A
/// task's text leaves the machine for an agent on another one, so that is a source only when `sources` names it.
pub(crate) fn sources(host: &dyn Host, config: &DelegateConfig, who: &Principal) -> Vec<Source> {
    host.sources()
        .into_iter()
        .filter(|source| {
            let named = config
                .sources
                .as_ref()
                .map(|named| named.contains(&source.name));
            named.unwrap_or(source.remote.is_none())
        })
        .filter(|source| host.may_use(who, &source.name))
        .collect()
}

/// A path with links, `..` and case settled, in the form a program takes as its working directory: Windows' `\\?\` prefix, which
/// canonicalizing adds, is taken off where the plain form names the same place.
fn canonical(path: &Path) -> std::io::Result<PathBuf> {
    let settled = std::fs::canonicalize(path)?;
    if cfg!(windows) {
        let text = settled.to_string_lossy();
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            if let Some(share) = rest.strip_prefix(r"UNC\") {
                return Ok(PathBuf::from(format!(r"\\{share}")));
            }
            let bytes = rest.as_bytes();
            if bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
                return Ok(PathBuf::from(rest));
            }
        }
    }
    Ok(settled)
}

/// The folder a task works in: `asked` (relative to `cwd`) or `cwd` itself, settled, and equal to or inside `cwd` and, when the
/// delegating connection has working roots, inside one of them.
///
/// # Errors
///
/// The reason it is refused, which names the folder as it was asked.
pub(crate) fn folder(
    cwd: &Path,
    asked: Option<&str>,
    roots: &[PathBuf],
) -> Result<PathBuf, String> {
    let base = canonical(cwd)
        .map_err(|error| format!("the session's working directory cannot be read: {error}"))?;
    let wanted = match asked.filter(|text| !text.is_empty()) {
        None => base.clone(),
        Some(text) => {
            let path = Path::new(text);
            let joined = if path.is_absolute() {
                path.to_path_buf()
            } else {
                base.join(path)
            };
            canonical(&joined)
                .map_err(|error| format!("folder {text:?} cannot be used: {error}"))?
        }
    };
    if !wanted.starts_with(&base) {
        return Err(format!(
            "folder {:?} is outside the session's working directory",
            asked.unwrap_or("")
        ));
    }
    if !roots.is_empty()
        && !roots
            .iter()
            .filter_map(|root| canonical(root).ok())
            .any(|root| wanted.starts_with(&root))
    {
        return Err(format!(
            "folder {:?} is outside this connection's working roots",
            asked.unwrap_or("")
        ));
    }
    if !wanted.is_dir() {
        return Err(format!(
            "folder {:?} is not a directory",
            asked.unwrap_or("")
        ));
    }
    Ok(wanted)
}

/// The folder a task from another machine runs in: the first working root of its source, settled. That machine names none, so
/// where it runs is the host's; a source with no working root has none to give.
///
/// # Errors
///
/// The reason it is refused.
pub(crate) fn remote_folder(roots: &[PathBuf]) -> Result<PathBuf, String> {
    let root = roots.first().ok_or_else(|| {
        "this source has no working root to run a task from another machine in: name one in working_roots".to_owned()
    })?;
    let settled = canonical(root)
        .map_err(|error| format!("the source's working root cannot be read: {error}"))?;
    if settled.is_dir() {
        Ok(settled)
    } else {
        Err("the source's working root is not a directory".to_owned())
    }
}

/// The modes an agent's session lists, read from a `session/new` or `session/set_config_option` answer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Modes {
    /// The mode now.
    pub(crate) current: Option<String>,
    /// The modes it lists.
    pub(crate) available: Vec<String>,
    /// The configuration option that carries the mode, when the agent lists one (category `mode`).
    pub(crate) option: Option<String>,
    /// Whether the agent has ACP's `modes`, set with `session/set_mode`, rather than only the option.
    pub(crate) set_mode: bool,
}

/// What `result` says of the session's modes, from `modes` and from a configuration option of category `mode`.
pub(crate) fn modes_of(result: &Value) -> Modes {
    let mut modes = Modes::default();
    if let Some(listed) = result
        .pointer("/modes/availableModes")
        .and_then(Value::as_array)
    {
        modes.available = listed
            .iter()
            .filter_map(|mode| mode.get("id").and_then(Value::as_str))
            .map(str::to_owned)
            .collect();
        modes.current = result
            .pointer("/modes/currentModeId")
            .and_then(Value::as_str)
            .map(str::to_owned);
        modes.set_mode = true;
    }
    let option = result
        .get("configOptions")
        .and_then(Value::as_array)
        .and_then(|options| {
            options
                .iter()
                .find(|option| option.get("category").and_then(Value::as_str) == Some("mode"))
        });
    if let Some(option) = option {
        // Version 1 names an option `id`, version 2 `configId`.
        modes.option = option
            .get("id")
            .or_else(|| option.get("configId"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        if !modes.set_mode {
            modes.available = option_values(option);
            modes.current = option
                .get("currentValue")
                .and_then(Value::as_str)
                .map(str::to_owned);
        }
    }
    modes
}

/// The values a select option lists, flat or in groups.
fn option_values(option: &Value) -> Vec<String> {
    let mut values = Vec::new();
    for item in option
        .get("options")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(value) = item.get("value").and_then(Value::as_str) {
            values.push(value.to_owned());
        }
        for grouped in item
            .get("options")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(value) = grouped.get("value").and_then(Value::as_str) {
                values.push(value.to_owned());
            }
        }
    }
    values
}

/// The mode a task's session is set to: the delegating session's own when the task's agent lists it, else the nearest mode that
/// asks for everything, else none.
///
/// # Errors
///
/// The reason there is no such mode.
pub(crate) fn choose_mode(parent: Option<&str>, available: &[String]) -> Result<String, String> {
    if let Some(own) = parent
        && available.iter().any(|mode| mode == own)
    {
        return Ok(own.to_owned());
    }
    ASKING
        .iter()
        .find(|asking| available.iter().any(|mode| mode == *asking))
        .map(|asking| (*asking).to_owned())
        .ok_or_else(|| {
            format!(
                "the agent lists no mode that asks for everything ({}), and not the delegating session's own",
                ASKING.join(" or ")
            )
        })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    use super::super::{Remote, Source};

    struct Listed(Vec<Source>);

    impl Host for Listed {
        fn sources(&self) -> Vec<Source> {
            self.0.clone()
        }
        fn may_use(&self, _who: &Principal, _source: &str) -> bool {
            true
        }
        fn kind(&self, _source: &str) -> Option<String> {
            None
        }
        fn open(
            &self,
            _source: &str,
            _context: super::super::ChildContext,
        ) -> Result<crate::local::Link, crate::serve::LocalError> {
            Err(crate::serve::LocalError::NoEntry)
        }
        fn prover(
            &self,
            _peer: &coqui_peer_webcrypto::PeerId,
        ) -> Result<coqui_agentic_peer::auth::Proof, String> {
            Err(String::new())
        }
    }

    fn source(name: &str, remote: bool) -> Source {
        Source {
            name: name.to_owned(),
            title: None,
            harness: None,
            roots: Vec::new(),
            remote: remote.then(|| Remote {
                url: "ws://127.0.0.1:1".to_owned(),
                peer: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .parse()
                    .expect("a peer id"),
                source: None,
            }),
        }
    }

    #[test]
    fn another_machines_agent_is_a_source_only_when_sources_names_it() {
        let host = Listed(vec![source("here", false), source("far", true)]);
        let names = |config: &DelegateConfig| -> Vec<String> {
            sources(&host, config, &Principal::Owner)
                .into_iter()
                .map(|source| source.name)
                .collect()
        };
        let mut config = DelegateConfig::default();
        assert_eq!(names(&config), ["here"], "unset is every local source");
        config.sources = Some(vec!["here".to_owned()]);
        assert_eq!(names(&config), ["here"]);
        config.sources = Some(vec!["far".to_owned(), "here".to_owned()]);
        assert_eq!(names(&config), ["here", "far"]);
        config.sources = Some(Vec::new());
        assert!(names(&config).is_empty());
    }

    #[test]
    fn a_task_from_another_machine_runs_in_the_first_working_root_or_is_refused() {
        let dir = std::env::temp_dir();
        assert!(remote_folder(std::slice::from_ref(&dir)).is_ok());
        assert!(remote_folder(&[]).unwrap_err().contains("working root"));
        assert!(remote_folder(&[dir.join("canoa-no-such-root")]).is_err());
    }

    fn strings(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn the_parents_mode_is_kept_when_the_agent_lists_it() {
        let listed = strings(&["default", "manual", "bypass"]);
        assert_eq!(
            choose_mode(Some("manual"), &listed).as_deref(),
            Ok("manual")
        );
        assert_eq!(
            choose_mode(Some("bypass"), &listed).as_deref(),
            Ok("bypass")
        );
    }

    #[test]
    fn a_mode_the_agent_lacks_becomes_the_nearest_asking_one_and_never_a_wider_one() {
        let listed = strings(&["default", "acceptEdits"]);
        assert_eq!(
            choose_mode(Some("bypass"), &listed).as_deref(),
            Ok("default")
        );
        assert_eq!(choose_mode(None, &listed).as_deref(), Ok("default"));
        let both = strings(&["acceptEdits", "default", "manual"]);
        assert_eq!(choose_mode(Some("bypass"), &both).as_deref(), Ok("manual"));
    }

    #[test]
    fn an_agent_with_no_asking_mode_and_not_the_parents_is_refused() {
        let listed = strings(&["acceptEdits", "bypass"]);
        assert!(choose_mode(Some("plan"), &listed).is_err());
        assert!(choose_mode(None, &listed).is_err());
        assert!(choose_mode(Some("plan"), &[]).is_err());
    }

    #[test]
    fn modes_are_read_from_acps_modes_and_from_a_configuration_option() {
        let legacy = json!({"modes": {"currentModeId": "default",
            "availableModes": [{"id": "default", "name": "Default"}, {"id": "plan", "name": "Plan"}]}});
        assert_eq!(
            modes_of(&legacy),
            Modes {
                current: Some("default".into()),
                available: strings(&["default", "plan"]),
                option: None,
                set_mode: true,
            }
        );
        let option = json!({"configOptions": [
            {"id": "model", "category": "model", "currentValue": "a", "options": [{"value": "a"}]},
            {"id": "mode", "category": "mode", "currentValue": "manual",
             "options": [{"value": "manual", "name": "M"}, {"group": "g", "options": [{"value": "bypass"}]}]}]});
        assert_eq!(
            modes_of(&option),
            Modes {
                current: Some("manual".into()),
                available: strings(&["manual", "bypass"]),
                option: Some("mode".into()),
                set_mode: false,
            }
        );
        assert_eq!(modes_of(&json!({})), Modes::default());
    }

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("canoa-privilege-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a directory");
        dir
    }

    #[test]
    fn a_folder_is_the_working_directory_or_inside_it_and_nothing_else() {
        let root = scratch("folder");
        let cwd = root.join("work");
        std::fs::create_dir_all(cwd.join("sub")).expect("made");
        std::fs::create_dir_all(root.join("other")).expect("made");
        let own = folder(&cwd, None, &[]).expect("the directory itself");
        assert!(own.ends_with("work"));
        assert!(
            folder(&cwd, Some("sub"), &[])
                .expect("inside")
                .ends_with("sub")
        );
        assert!(
            folder(&cwd, Some("sub/../sub"), &[])
                .expect("inside after settling")
                .ends_with("sub")
        );
        for outside in ["..", "../other", "sub/../../other"] {
            let refused = folder(&cwd, Some(outside), &[]).expect_err(outside);
            assert!(refused.contains("outside"), "{refused}");
        }
        let absolute = root.join("other").to_string_lossy().into_owned();
        assert!(folder(&cwd, Some(&absolute), &[]).is_err());
        assert!(folder(&cwd, Some("missing"), &[]).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_folder_outside_the_connections_roots_is_refused() {
        let root = scratch("roots");
        let allowed = root.join("allowed");
        let cwd = root.join("other");
        std::fs::create_dir_all(&allowed).expect("made");
        std::fs::create_dir_all(&cwd).expect("made");
        assert!(folder(&cwd, None, std::slice::from_ref(&allowed)).is_err());
        assert!(folder(&allowed, None, std::slice::from_ref(&allowed)).is_ok());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_out_of_the_working_directory_is_followed_and_refused() {
        let root = scratch("link");
        let cwd = root.join("work");
        std::fs::create_dir_all(&cwd).expect("made");
        std::fs::create_dir_all(root.join("secret")).expect("made");
        std::os::unix::fs::symlink(root.join("secret"), cwd.join("door")).expect("a link");
        assert!(folder(&cwd, Some("door"), &[]).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
}
