//! The host's own sources: the coding agents it can start on this computer without canoa.cfg naming them. Each is an entry
//! reached over [`crate::Local`] only, with no address, so nothing on a network can start one.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "paths and environments are std's"
)]

use std::path::{Path, PathBuf};

use canoa::preset::SourcePreset as Preset;

use crate::config::{Config, Entry};
use crate::store::shelf::Shelf;

/// One source the host serves, and what it needs that this computer lacks.
pub(crate) struct Source {
    pub(crate) entry: Entry,
    pub(crate) missing: Vec<String>,
}

/// A copy of an agent the Store installed, as its launch records it: each setting of the launch that is text. A setting that is an
/// absolute path names a file inside the copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManagedCopy {
    pub(crate) settings: Vec<(String, String)>,
}

/// The presets of the bundled agents, in the order they are offered.
fn presets() -> impl Iterator<Item = &'static Preset> {
    crate::bundled::ALL.iter().map(|bundle| &bundle.preset)
}

/// The names of the sources, in the order they are offered.
#[cfg(test)]
pub(crate) fn names() -> Vec<&'static str> {
    presets().map(|preset| preset.name).collect()
}

/// The sources this host serves besides `config`'s entries: every preset whose name `config` leaves free, found or not, so a
/// client can say what is missing.
pub(crate) fn entries(config: &Config) -> Vec<Source> {
    sources_in(
        config,
        &|program, search| find(program, search),
        &|program| {
            Shelf::default_root().and_then(|root| managed_in(&Shelf::unchecked(root), program))
        },
    )
}

/// The copy of `program` that `shelf` manages, as the text settings of its launch, when `current.json` is the one this host wrote, every
/// file of the copy is as it was installed and each setting that is an absolute path is a file inside the copy.
fn managed_in(shelf: &Shelf, program: &str) -> Option<ManagedCopy> {
    let dir = shelf.program(program).ok()?;
    let current = dir.current_checked().ok()??;
    dir.verify(&current).ok()?;
    let held = &current.launch.settings;
    let inside = |path: &Path| {
        path.is_file()
            && canoa::store::exec::normalize(path)
                .zip(canoa::store::exec::normalize(Path::new(&current.dir)))
                .is_some_and(|(path, root)| path.starts_with(root))
    };
    let mut settings = Vec::new();
    for (name, value) in held.as_object()? {
        let Some(text) = value.as_str() else {
            continue;
        };
        if Path::new(text).is_absolute() && !inside(Path::new(text)) {
            return None;
        }
        settings.push((name.clone(), text.to_owned()));
    }
    (!settings.is_empty()).then_some(ManagedCopy { settings })
}

/// The program the Store installed under the name `program` on `shelf`, to run in place of one found on the computer: the copy's
/// own launch path, when `current.json` is the one this host wrote and every file of the copy is as it was installed.
pub(crate) fn managed_program_in(
    shelf: &Shelf,
    program: &str,
) -> Option<(PathBuf, Vec<(String, String)>)> {
    let dir = shelf.program(program).ok()?;
    let current = dir.current_checked().ok()??;
    // Every file of the copy is read to verify it, which is not done again while the copy's launch file and its record are as they
    // were: they are stamped by size and time of change, for as long as this host runs.
    let stamp = (
        stamp_of(Path::new(&current.dir)),
        stamp_of(Path::new(&current.launch.path)),
        shelf.root().to_path_buf(),
        program.to_owned(),
    );
    let verified = VERIFIED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .contains(&stamp);
    if !verified {
        dir.verify(&current).ok()?;
        VERIFIED
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(stamp);
    }
    let path = PathBuf::from(&current.launch.path);
    let inside = path.is_absolute()
        && path.is_file()
        && canoa::store::exec::normalize(&path)
            .zip(canoa::store::exec::normalize(Path::new(&current.dir)))
            .is_some_and(|(path, root)| path.starts_with(root));
    inside.then(|| {
        let environment = current
            .launch
            .environment
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect();
        (path, environment)
    })
}

/// What was checked of a copy: its folder and launch file by size and time of change, in which shelf, for which program.
type Stamp = (
    Option<(u64, std::time::SystemTime)>,
    Option<(u64, std::time::SystemTime)>,
    PathBuf,
    String,
);

/// Process-wide cache of managed runtime copies already integrity-verified for a `(dir_stamp, bin_stamp, path, program)`.
static VERIFIED: std::sync::Mutex<Vec<Stamp>> = std::sync::Mutex::new(Vec::new());

fn stamp_of(path: &Path) -> Option<(u64, std::time::SystemTime)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.len(), meta.modified().ok()?))
}

/// The folders of the programs the Store installed on `shelf` (runtimes, agents, tools): where each one's launch file is, for a
/// copy that is as it was installed, each once and in the order of the programs' names. A copy that changed since adds nothing,
/// so nothing unchecked reaches a search path.
pub(crate) fn program_dirs_in(shelf: &Shelf) -> Vec<PathBuf> {
    let mut names: Vec<String> = std::fs::read_dir(shelf.root())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    names.sort_unstable();
    let mut dirs: Vec<PathBuf> = Vec::new();
    for name in names {
        if let Some(dir) = managed_program_in(shelf, &name)
            .and_then(|(path, _)| path.parent().map(Path::to_path_buf))
            && !dirs.contains(&dir)
        {
            dirs.push(dir);
        }
    }
    dirs
}

/// `base` (a `PATH`), then each folder of the programs on `shelf` it does not already hold: where a name is looked up when the
/// host hands a library a search path. The person's own folders come first, so a program they installed is never shadowed.
pub(crate) fn search_path_in(shelf: &Shelf, base: &std::ffi::OsStr) -> std::ffi::OsString {
    let held: Vec<PathBuf> = std::env::split_paths(base).collect();
    let extra: Vec<PathBuf> = program_dirs_in(shelf)
        .into_iter()
        .filter(|dir| !held.contains(dir))
        .collect();
    std::env::join_paths(held.into_iter().chain(extra)).unwrap_or_else(|_| base.to_owned())
}

/// [`search_path_in`] over this process's `PATH` and this user's programs folder.
pub(crate) fn search_path() -> std::ffi::OsString {
    let base = std::env::var_os("PATH").unwrap_or_default();
    Shelf::default_root().map_or(base.clone(), |root| {
        search_path_in(&Shelf::unchecked(root), &base)
    })
}

/// [`program_dirs_in`] over this user's programs folder.
pub(crate) fn program_dirs() -> Vec<PathBuf> {
    Shelf::default_root().map_or_else(Vec::new, |root| program_dirs_in(&Shelf::unchecked(root)))
}

/// The version of the copy [`managed_program`] runs, when there is one.
pub(crate) fn managed_version(program: &str) -> Option<String> {
    let shelf = Shelf::unchecked(Shelf::default_root()?);
    managed_program_in(&shelf, program)?;
    shelf
        .program(program)
        .ok()?
        .current_checked()
        .ok()??
        .version
        .into()
}

/// The program the Store installed for harness `id` on `shelf`: the launch it recorded, whatever the launch file is called.
pub(crate) fn managed_program_of_harness_in(
    shelf: &Shelf,
    id: &str,
) -> Option<(PathBuf, Vec<(String, String)>)> {
    managed_program_in(shelf, crate::bundled::managed_program_name(id)?)
}

/// [`managed_program_of_harness_in`] over this user's programs folder.
pub(crate) fn managed_program_of_harness(id: &str) -> Option<(PathBuf, Vec<(String, String)>)> {
    Shelf::default_root()
        .and_then(|root| managed_program_of_harness_in(&Shelf::unchecked(root), id))
}

/// [`managed_program_in`] over this user's programs folder.
pub(crate) fn managed_program(program: &str) -> Option<(PathBuf, Vec<(String, String)>)> {
    Shelf::default_root().and_then(|root| managed_program_in(&Shelf::unchecked(root), program))
}

fn sources_in(
    config: &Config,
    find: &dyn Fn(&str, &[&str]) -> Option<PathBuf>,
    managed: &dyn Fn(&str) -> Option<ManagedCopy>,
) -> Vec<Source> {
    // The person's own commands come after the built-in ones, and take a built-in's name from it.
    let mut own: Vec<Source> = config
        .local_agents
        .iter()
        .filter(|entry| {
            let taken = config.entries.iter().any(|each| each.name == entry.name);
            if taken {
                tracing::info!(
                    "agents.{} is also host.agents.{}, which is served; the command is not",
                    entry.name,
                    entry.name
                );
            }
            !taken
        })
        .map(|entry| {
            let program = match &entry.kind {
                crate::EntryKind::Agent { program, .. } => program.to_string_lossy().into_owned(),
                _ => String::new(),
            };
            let missing = if find(&program, &[]).is_some() {
                Vec::new()
            } else {
                vec![program]
            };
            Source {
                entry: entry.clone(),
                missing,
            }
        })
        .collect();
    let mut sources: Vec<Source> = presets()
        .filter(|preset| {
            let taken = config.entries.iter().any(|entry| entry.name == preset.name);
            if taken {
                tracing::info!(
                    "host.agents.{} is canoa.cfg's own, so the built-in source of that name is not served",
                    preset.name
                );
            }
            !taken
        })
        .filter_map(|preset| {
            let copy = preset.managed.and_then(managed);
            let found = if copy.is_some() {
                None
            } else {
                find(preset.program, preset.search)
            };
            let mut missing: Vec<String> = Vec::new();
            if found.is_none() && copy.is_none() {
                missing.push(preset.program.to_owned());
            }
            for needed in preset.requires {
                if find(needed, &[]).is_none() {
                    missing.push((*needed).to_owned());
                }
            }
            let entry = entry_of(preset, found.as_deref(), copy.as_ref(), config)?;
            Some(Source { entry, missing })
        })
        .filter(|source| own.iter().all(|mine| mine.entry.name != source.entry.name))
        .collect();
    sources.append(&mut own);
    sources
}

/// `preset` as the entry canoa.cfg would have said, run through the same reading so that it is checked the same.
fn entry_of(
    preset: &Preset,
    found: Option<&Path>,
    managed: Option<&ManagedCopy>,
    config: &Config,
) -> Option<Entry> {
    let quoted = |text: &str| serde_json::to_string(text).unwrap_or_default();
    let program = found.map_or_else(
        || preset.program.to_owned(),
        |path| path.to_string_lossy().into_owned(),
    );
    let mut text = format!(
        "[host.agents.{}]\ntitle = {}\n",
        preset.name,
        quoted(preset.name)
    );
    if let Some(harness) = preset.harness {
        text.push_str(&format!("harness = {}\n", quoted(harness)));
        if found.is_some() {
            text.push_str(&format!("executable = {}\n", quoted(&program)));
        }
        text.push_str(&entry_settings(
            preset.name,
            harness,
            managed,
            config.protected_paths.as_deref(),
        ));
    } else {
        let args: Vec<String> = preset.args.iter().map(|arg| quoted(arg)).collect();
        text.push_str(&format!(
            "program = {}\nargs = [{}]\n",
            quoted(&program),
            args.join(", ")
        ));
        if let Some(usage) = preset.usage {
            text.push_str(&format!("usage = {}\n", quoted(usage)));
        }
    }
    let base = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let parsed = Config::from_str_in(&text, &base);
    match parsed {
        Ok(config) => config.entries.into_iter().next().map(|mut entry| {
            entry.local = true;
            entry
        }),
        Err(error) => {
            tracing::warn!("the built-in source {} is unusable: {error}", preset.name);
            None
        }
    }
}

/// The settings of the agent `harness`'s provider that its source `name` is started with, as the agent's description says: a managed
/// copy's launch and the person's `[permissions] protected` list are what it may answer from.
fn entry_settings(
    name: &str,
    harness: &str,
    managed: Option<&ManagedCopy>,
    protected: Option<&[String]>,
) -> String {
    let mut request = canoa::vendor::EntrySettingsRequest::new();
    if let Some(managed) = managed {
        request.managed = managed
            .settings
            .iter()
            .map(|(name, value)| canoa::vendor::Variable::new(name.clone(), value.clone()))
            .collect();
    }
    request.protected = protected.map(<[String]>::to_vec);
    let said = crate::vendors::behave(harness, |vendor| vendor.entry_settings(request))
        .and_then(Result::ok)
        .flatten()
        .and_then(|value| guatiao_serde::adapt::from_value::<serde_json::Value>(&value).ok());
    let Some(serde_json::Value::Object(settings)) = said else {
        return String::new();
    };
    let quoted = |text: &str| serde_json::to_string(text).unwrap_or_default();
    let mut text = format!("[host.agents.{name}.settings]\n");
    for (key, value) in settings {
        if !value.is_object() && !value.is_null() {
            text.push_str(&format!("{} = {value}\n", quoted(&key)));
        }
    }
    if text.lines().count() == 1 {
        return String::new();
    }
    text
}

/// `directory` with `~` and `%NAME%` filled in; `None` when a variable is not set.
fn expand(directory: &str) -> Option<String> {
    let mut out = directory.to_owned();
    if out == "~" || out.starts_with("~/") {
        let home = std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .ok()?;
        out = format!("{home}{}", &out[1..]);
    }
    while let Some(start) = out.find('%') {
        let end = out[start + 1..].find('%')? + start + 1;
        let value = std::env::var(&out[start + 1..end]).ok()?;
        out.replace_range(start..=end, &value);
    }
    Some(out)
}

/// The file that runs `program`: on `PATH`, else in `search`. On Windows only a program the system starts counts, as `where`
/// lists `npm` the shell script before `npm.cmd`.
pub(crate) fn find(program: &str, search: &[&str]) -> Option<PathBuf> {
    let direct = Path::new(program);
    if direct.is_absolute() && is_program(direct) {
        return Some(direct.to_path_buf());
    }
    let from_path = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .unwrap_or_default();
    let others = search
        .iter()
        .filter_map(|dir| expand(dir))
        .map(PathBuf::from);
    for directory in from_path.into_iter().chain(others) {
        for name in names_of(program) {
            let candidate = directory.join(name);
            if is_program(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(windows)]
fn names_of(program: &str) -> Vec<String> {
    ["exe", "cmd", "bat", "com"]
        .iter()
        .map(|extension| format!("{program}.{extension}"))
        .collect()
}

#[cfg(not(windows))]
fn names_of(program: &str) -> Vec<String> {
    vec![program.to_owned()]
}

#[cfg(windows)]
fn is_program(path: &Path) -> bool {
    path.is_file()
}

#[cfg(not(windows))]
fn is_program(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    path.metadata()
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nothing_found() -> Config {
        Config::from_str_in("", Path::new(".")).expect("empty")
    }

    #[test]
    fn every_source_is_a_local_entry_and_a_missing_program_is_said() {
        let sources = sources_in(&nothing_found(), &|_, _| None, &|_| None);
        let named: Vec<&str> = sources.iter().map(|s| s.entry.name.as_str()).collect();
        assert_eq!(named, names());
        assert!(sources.iter().all(|source| source.entry.local));
        let missing = |name: &str| {
            sources
                .iter()
                .find(|source| source.entry.name == name)
                .map(|source| source.missing.clone())
        };
        assert_eq!(missing("Claude"), Some(vec!["claude".to_owned()]));
        assert_eq!(missing("Pi"), Some(vec!["pi".to_owned()]));
    }

    #[test]
    fn a_found_program_is_what_runs_and_nothing_is_missing() {
        let sources = sources_in(
            &nothing_found(),
            &|program, _| Some(PathBuf::from(format!("/opt/bin/{program}"))),
            &|_| None,
        );
        assert!(sources.iter().all(|source| source.missing.is_empty()));
        let grok = sources
            .iter()
            .find(|source| source.entry.name == "Grok")
            .expect("Grok");
        assert!(matches!(
            &grok.entry.kind,
            crate::EntryKind::Harness { harness, executable: Some(exe), .. }
                if harness == "grok" && Path::new(exe).ends_with("opt/bin/grok")
        ));
    }

    #[test]
    fn opencode_is_the_built_in_opencode_harness_on_the_program_found() {
        let sources = sources_in(
            &nothing_found(),
            &|program, _| Some(PathBuf::from(format!("/opt/bin/{program}"))),
            &|_| None,
        );
        let source = sources
            .iter()
            .find(|source| source.entry.name == "OpenCode")
            .expect("OpenCode");
        match &source.entry.kind {
            crate::EntryKind::Harness {
                harness,
                executable,
                ..
            } => {
                assert_eq!(harness, "opencode");
                assert!(
                    executable.as_ref().is_some_and(
                        |executable| Path::new(executable).ends_with("opt/bin/opencode")
                    ),
                    "{executable:?}"
                );
            }
            other => panic!("{other:?}"),
        }
        assert!(source.entry.local);
    }

    #[test]
    fn a_name_canoa_cfg_uses_is_left_to_it() {
        let config = Config::from_str_in("[host.agents.Claude]\nprogram = \"x\"\n", Path::new("."))
            .expect("parses");
        let sources = sources_in(&config, &|_, _| None, &|_| None);
        assert!(sources.iter().all(|source| source.entry.name != "Claude"));
        assert_eq!(sources.len(), names().len() - 1);
    }

    fn with_agents(text: &str) -> Config {
        Config::from_str_in(text, Path::new(".")).expect("parses")
    }

    #[test]
    fn the_persons_own_commands_follow_the_built_in_sources_and_say_what_they_miss() {
        let config = with_agents(
            "[agents.\"My Agent\"]\nprogram = \"my-agent\"\nargs = [\"--acp\", \"x y\"]\n\
             [agents.Claude.profile]\nmodel = \"opus\"\n\
             [agents.Seen]\nprogram = \"seen\"\n",
        );
        assert_eq!(
            config.local_agents.len(),
            2,
            "a table with no program is no command"
        );
        let sources = sources_in(
            &config,
            &|program, _| (program == "seen").then(|| PathBuf::from("/bin/seen")),
            &|_| None,
        );
        let named: Vec<&str> = sources.iter().map(|s| s.entry.name.as_str()).collect();
        let mut expected = names();
        expected.extend(["My Agent", "Seen"]);
        assert_eq!(named, expected);
        let mine = &sources[names().len()];
        assert!(mine.entry.local);
        assert_eq!(mine.missing, ["my-agent"]);
        match &mine.entry.kind {
            crate::EntryKind::Agent { program, args, .. } => {
                assert_eq!(program.to_string_lossy(), "my-agent");
                assert_eq!(args.len(), 2);
            }
            other => panic!("{other:?}"),
        }
        assert!(sources.last().is_some_and(|s| s.missing.is_empty()));
    }

    #[test]
    fn a_command_takes_the_name_of_a_built_in_source_from_it() {
        let config = with_agents("[agents.Pi]\nprogram = \"my-pi\"\n");
        let sources = sources_in(&config, &|_, _| None, &|_| None);
        let pis: Vec<_> = sources.iter().filter(|s| s.entry.name == "Pi").collect();
        assert_eq!(pis.len(), 1);
        assert!(matches!(
            &pis[0].entry.kind,
            crate::EntryKind::Agent { program, .. } if program == "my-pi"
        ));
    }

    #[test]
    fn antigravitys_command_has_its_quota_read_whatever_directory_it_is_in() {
        for program in ["agy", "C:\\\\agy\\\\bin\\\\agy.exe", "/home/me/bin/agy"] {
            let config = with_agents(&format!(
                "[agents.A]\nprogram = {}\nargs = [\"acp\"]\n",
                serde_json::to_string(program).expect("quoted")
            ));
            assert!(
                matches!(
                    &config.local_agents[0].kind,
                    crate::EntryKind::Agent { usage: Some(_), .. }
                ),
                "{program}"
            );
        }
        let other = with_agents("[agents.A]\nprogram = \"agy\"\nargs = [\"chat\"]\n");
        assert!(matches!(
            &other.local_agents[0].kind,
            crate::EntryKind::Agent { usage: None, .. }
        ));
    }

    #[test]
    fn a_command_that_is_not_text_is_refused_naming_it() {
        for text in [
            "[agents.A]\nprogram = 1\n",
            "[agents.A]\nprogram = \"x\"\nargs = \"y\"\n",
            "[agents.A]\nprogram = \"x\"\nargs = [1]\n",
        ] {
            let error = Config::from_str_in(text, Path::new(".")).expect_err("refused");
            assert!(error.to_string().contains("agents.A"), "{error}");
        }
    }

    fn pi_of(sources: &[Source]) -> &Source {
        sources
            .iter()
            .find(|source| source.entry.name == "Pi")
            .expect("Pi")
    }

    /// Pi's entry: its executable, and its `settings.protected` when it has one.
    fn pi_harness(source: &Source) -> (Option<String>, Option<serde_json::Value>) {
        match &source.entry.kind {
            crate::EntryKind::Harness {
                harness,
                executable,
                settings,
                ..
            } => {
                assert_eq!(harness, "pi");
                (
                    executable
                        .as_ref()
                        .map(|executable| executable.to_string_lossy().into_owned()),
                    settings
                        .as_ref()
                        .and_then(|settings| settings.as_map().get("protected").cloned()),
                )
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn pi_is_ready_when_a_pi_is_found_and_needs_nothing_else() {
        let sources = sources_in(
            &nothing_found(),
            &|program, _| (program == "pi").then(|| PathBuf::from("/opt/bin/pi")),
            &|_| None,
        );
        let pi = pi_of(&sources);
        assert!(pi.missing.is_empty(), "{:?}", pi.missing);
        let (executable, protected) = pi_harness(pi);
        assert!(
            executable
                .as_deref()
                .is_some_and(|e| Path::new(e).ends_with("opt/bin/pi")),
            "{executable:?}"
        );
        assert_eq!(protected, None, "nothing was said of protected paths");
    }

    #[test]
    fn a_managed_pi_runs_as_its_node_and_script_and_no_program_is_looked_for() {
        let copy = ManagedCopy {
            settings: vec![
                ("node".to_owned(), "/data/pi/node/bin/node".to_owned()),
                ("script".to_owned(), "/data/pi/cli.js".to_owned()),
            ],
        };
        let sources = sources_in(&nothing_found(), &|_, _| None, &|program| {
            (program == "pi").then(|| copy.clone())
        });
        let pi = pi_of(&sources);
        assert!(pi.missing.is_empty());
        match &pi.entry.kind {
            crate::EntryKind::Harness {
                executable,
                settings,
                ..
            } => {
                assert!(executable.is_none());
                let map = settings.as_ref().expect("settings").as_map();
                assert_eq!(map["node"], "/data/pi/node/bin/node");
                assert_eq!(map["script"], "/data/pi/cli.js");
                assert!(!map.contains_key("protected"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_persons_protected_list_replaces_pis_recommended_one_and_absent_leaves_it() {
        let config = with_agents(
            "[permissions]\nallow_once_only = true\nprotected = \"\"\"\n~/.ssh\n!~/.ssh/known_hosts\n\"\"\"\n",
        );
        assert_eq!(
            config.protected_paths,
            Some(vec!["~/.ssh".to_owned(), "!~/.ssh/known_hosts".to_owned()])
        );
        let sources = sources_in(
            &config,
            &|program, _| (program == "pi").then(|| PathBuf::from("/opt/bin/pi")),
            &|_| None,
        );
        let (_, protected) = pi_harness(pi_of(&sources));
        assert_eq!(
            protected,
            Some(serde_json::json!(["~/.ssh", "!~/.ssh/known_hosts"]))
        );
        let other = with_agents("[permissions]\ndefault_mode = \"plan\"\n");
        assert_eq!(other.protected_paths, None);
        let empty = with_agents("[permissions]\nprotected = []\n");
        assert_eq!(empty.protected_paths, Some(Vec::new()));
    }

    fn scratch(name: &str) -> PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let number = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("canoa-{name}-{}-{number}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("made");
        dir
    }

    #[test]
    fn scratch_folders_made_at_once_are_named_apart() {
        let names: Vec<_> = std::thread::scope(|scope| {
            let made: Vec<_> = (0..8)
                .map(|_| scope.spawn(|| (0..50).map(|_| scratch("apart")).collect::<Vec<_>>()))
                .collect();
            made.into_iter()
                .flat_map(|handle| handle.join().expect("a thread"))
                .collect()
        });
        let distinct: std::collections::BTreeSet<_> = names.iter().collect();
        assert_eq!(distinct.len(), names.len());
        for dir in names {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    /// A shelf holding a `pi` whose launch is a node and a script inside the copy, as the Store's Pi extension records it.
    fn shelf_with_pi(root: &Path) -> (Shelf, PathBuf) {
        use crate::store::shelf::MadeBy;
        use coqui_acp_api::guatiao::{Map, Value};
        let shelf = Shelf::unchecked(root);
        let program = shelf.program("pi").expect("a name");
        let locked = program.lock().expect("locked");
        let staging = locked.stage("1.5.2").expect("staged");
        let node = staging.join("node.bin");
        let script = staging.join("cli.js");
        std::fs::write(&node, b"node").expect("written");
        std::fs::write(&script, b"script").expect("written");
        let mut launch = canoa::store::Launch::new(node.to_string_lossy().into_owned());
        let mut settings = Map::new();
        settings.set("node", Value::from(node.to_string_lossy().as_ref()));
        settings.set("script", Value::from(script.to_string_lossy().as_ref()));
        launch.settings = Value::from(settings);
        let mut files = Vec::new();
        crate::store::shelf::record_tree(&staging, &mut files).expect("recorded");
        locked
            .commit(
                &staging,
                "1.5.2",
                &launch,
                files,
                MadeBy {
                    provider: "pi_store".to_owned(),
                    version: "0".to_owned(),
                },
            )
            .expect("committed");
        drop(locked);
        (shelf, script)
    }

    /// A shelf holding `name` as one file, launched by its path, as the Store's single-binary extensions record it.
    fn shelf_with_binary(root: &Path, name: &str) -> (Shelf, PathBuf) {
        shelf_with_launch(
            root,
            name,
            &format!("{name}{}", std::env::consts::EXE_SUFFIX),
        )
    }

    /// A shelf holding `name` whose launch file is `file` inside the copy, which need not be called as the program is.
    fn shelf_with_launch(root: &Path, name: &str, file: &str) -> (Shelf, PathBuf) {
        use crate::store::shelf::MadeBy;
        let shelf = Shelf::unchecked(root);
        let program = shelf.program(name).expect("a name");
        let locked = program.lock().expect("locked");
        let staging = locked.stage("2.0.0").expect("staged");
        let binary = staging.join(file);
        std::fs::create_dir_all(binary.parent().expect("a folder")).expect("made");
        std::fs::write(&binary, b"binary").expect("written");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755))
                .expect("runnable");
        }
        let mut launch = canoa::store::Launch::new(binary.to_string_lossy().into_owned());
        launch
            .environment
            .insert("DISABLE_AUTOUPDATER".to_owned(), "1".to_owned());
        let mut files = Vec::new();
        crate::store::shelf::record_tree(&staging, &mut files).expect("recorded");
        locked
            .commit(
                &staging,
                "2.0.0",
                &launch,
                files,
                MadeBy {
                    provider: format!("{name}_store"),
                    version: "0".to_owned(),
                },
            )
            .expect("committed");
        drop(locked);
        (shelf, binary)
    }

    #[test]
    fn a_program_the_store_installed_runs_from_its_copy_and_a_changed_copy_runs_nowhere() {
        let root = scratch("managed-claude");
        let (shelf, binary) = shelf_with_binary(&root, "claude");
        let (path, environment) = managed_program_in(&shelf, "claude").expect("a managed copy");
        assert_eq!(path, binary);
        assert_eq!(
            environment,
            vec![("DISABLE_AUTOUPDATER".to_owned(), "1".to_owned())]
        );
        assert_eq!(managed_program_in(&shelf, "codex"), None);
        std::fs::write(&binary, b"changed since the install").expect("written");
        assert_eq!(managed_program_in(&shelf, "claude"), None);
    }

    #[test]
    fn the_folders_of_installed_programs_come_from_checked_copies_only() {
        let root = scratch("program-dirs");
        assert!(program_dirs_in(&Shelf::unchecked(&root)).is_empty());
        let (shelf, binary) = shelf_with_binary(&root, "node");
        assert_eq!(
            program_dirs_in(&shelf),
            vec![binary.parent().expect("a folder").to_path_buf()]
        );
        // An agent's copy is as much a program as a runtime, and a copy changed since the install adds nothing.
        let (other, agy) = shelf_with_binary(&root, "agy");
        assert_eq!(program_dirs_in(&other).len(), 2);
        std::fs::write(&agy, b"changed since the install").expect("written");
        assert_eq!(
            program_dirs_in(&other),
            vec![binary.parent().expect("a folder").to_path_buf()]
        );
        std::fs::write(&binary, b"changed since the install").expect("written");
        assert!(program_dirs_in(&shelf).is_empty());
    }

    #[test]
    fn a_harnesss_program_is_the_launch_the_store_recorded_whatever_the_launch_file_is_called() {
        let root = scratch("launch-name");
        let (harness, program) = crate::bundled::ALL
            .iter()
            .find_map(|bundle| {
                let harness = bundle.preset.harness?;
                (harness != bundle.preset.program && bundle.preset.managed.is_none())
                    .then_some((harness, bundle.preset.program))
            })
            .expect("a harness whose program is called something else");
        let (shelf, launch) = shelf_with_launch(&root, program, "bin/launcher");
        assert_eq!(
            managed_program_of_harness_in(&shelf, harness).map(|(path, _)| path),
            Some(launch)
        );
        // The program's own name is no harness, and the launch file's name is no lookup: a search by `program` finds nothing.
        assert_eq!(managed_program_of_harness_in(&shelf, program), None);
        let search = search_path_in(&shelf, std::ffi::OsStr::new(""));
        assert_eq!(canoa::search::find_in(&search, "", program), None);
    }

    #[test]
    fn a_program_the_store_installed_is_found_on_the_search_path_and_the_persons_folders_come_first()
     {
        let root = scratch("search-path");
        let name = "canoa-stand-in-agy";
        let (shelf, binary) = shelf_with_binary(&root, name);
        let own = std::env::join_paths([scratch("search-path-own")]).expect("joined");
        let find = |path: &std::ffi::OsStr| canoa::search::find_in(path, "", name);
        // The control: the person's own path has no such program.
        assert_eq!(find(&own), None);
        let searched = search_path_in(&shelf, &own);
        assert_eq!(find(&searched).as_deref(), Some(binary.as_path()));
        let folders: Vec<_> = std::env::split_paths(&searched).collect();
        assert_eq!(folders.len(), 2);
        assert_eq!(folders[0], std::env::split_paths(&own).next().expect("one"));
        // A folder the person already has is not added again.
        assert_eq!(search_path_in(&shelf, &searched), searched);
    }

    #[test]
    fn the_stores_managed_copy_is_read_from_the_shelf_and_checked() {
        let root = scratch("managed-pi");
        let (shelf, script) = shelf_with_pi(&root);
        let copy = managed_in(&shelf, "pi").expect("a managed Pi");
        let setting = |key: &str| {
            copy.settings
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| PathBuf::from(value))
        };
        assert_eq!(setting("script"), Some(script.clone()));
        assert!(setting("node").is_some_and(|node| node.ends_with("node.bin")));
        assert_eq!(
            managed_in(&shelf, "grok"),
            None,
            "no copy of another program"
        );
        // A file changed since the install is no copy to run.
        std::fs::write(&script, b"changed").expect("written");
        assert_eq!(managed_in(&shelf, "pi"), None);
        std::fs::remove_dir_all(&root).expect("removed");
    }

    #[test]
    fn variables_and_the_home_directory_are_filled_in() {
        assert_eq!(expand("/plain"), Some("/plain".to_owned()));
        assert_eq!(expand("%CANOA_NO_SUCH_VARIABLE_FOR_THE_TEST%/x"), None);
        assert!(expand("~/x").is_some_and(|path| path.ends_with("/x")));
    }
}
