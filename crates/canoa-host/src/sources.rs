//! The host's own sources: the coding agents it can start on this computer without canoa.cfg naming them. Each is an entry
//! reached over [`crate::Local`] only, with no address, so nothing on a network can start one.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "paths and environments are std's"
)]

use std::path::{Path, PathBuf};

use crate::config::{Config, Entry};

/// One source the host serves, and what it needs that this computer lacks.
pub(crate) struct Source {
    pub(crate) entry: Entry,
    pub(crate) missing: Vec<String>,
}

/// What one source is: a built-in harness, or a program speaking ACP on its standard streams.
struct Preset {
    name: &'static str,
    /// `claude` or `codex` for a harness; otherwise `program` and `args` run.
    harness: Option<&'static str>,
    program: &'static str,
    args: &'static [&'static str],
    /// Other programs it needs, offered only when found.
    requires: &'static [&'static str],
    /// Where the program's own installer leaves it, tried when `PATH` has none: `~` is the home directory and `%NAME%` an
    /// environment variable.
    search: &'static [&'static str],
    usage: Option<&'static str>,
}

const PRESETS: &[Preset] = &[
    Preset {
        name: "Claude",
        harness: Some("claude"),
        program: "claude",
        args: &[],
        requires: &[],
        search: &[],
        usage: None,
    },
    Preset {
        name: "Codex",
        harness: Some("codex"),
        program: "codex",
        args: &[],
        requires: &[],
        search: &[],
        usage: None,
    },
    Preset {
        name: "Antigravity",
        harness: None,
        program: "agy",
        args: &["acp"],
        requires: &[],
        search: &["%LOCALAPPDATA%/agy/bin"],
        usage: Some("antigravity"),
    },
    Preset {
        name: "Grok",
        harness: None,
        program: "grok",
        args: &["agent", "stdio"],
        requires: &[],
        search: &["~/.grok/bin"],
        usage: None,
    },
    Preset {
        name: "Pi",
        harness: None,
        program: "npx",
        args: &["-y", "pi-acp@0.0.34"],
        requires: &["pi"],
        search: &[],
        usage: None,
    },
];

/// The names of the sources, in the order they are offered.
#[cfg(test)]
pub(crate) fn names() -> Vec<&'static str> {
    PRESETS.iter().map(|preset| preset.name).collect()
}

/// The sources this host serves besides `config`'s entries: every preset whose name `config` leaves free, found or not, so a
/// client can say what is missing.
pub(crate) fn entries(config: &Config) -> Vec<Source> {
    sources_in(config, &|program, search| find(program, search))
}

fn sources_in(config: &Config, find: &dyn Fn(&str, &[&str]) -> Option<PathBuf>) -> Vec<Source> {
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
    let mut sources: Vec<Source> = PRESETS
        .iter()
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
            let found = find(preset.program, preset.search);
            let mut missing: Vec<String> = Vec::new();
            if found.is_none() {
                missing.push(preset.program.to_owned());
            }
            for needed in preset.requires {
                if find(needed, &[]).is_none() {
                    missing.push((*needed).to_owned());
                }
            }
            let entry = entry_of(preset, found.as_deref())?;
            Some(Source { entry, missing })
        })
        .filter(|source| own.iter().all(|mine| mine.entry.name != source.entry.name))
        .collect();
    sources.append(&mut own);
    sources
}

/// `preset` as the entry canoa.cfg would have said, run through the same reading so that it is checked the same.
fn entry_of(preset: &Preset, found: Option<&Path>) -> Option<Entry> {
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
        text.push_str(&format!("harness = {}\nbypass = true\n", quoted(harness)));
        if found.is_some() {
            text.push_str(&format!("executable = {}\n", quoted(&program)));
        }
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
/// lists `npx` the shell script before `npx.cmd`.
fn find(program: &str, search: &[&str]) -> Option<PathBuf> {
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
        let sources = sources_in(&nothing_found(), &|_, _| None);
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
        assert_eq!(missing("Pi"), Some(vec!["npx".to_owned(), "pi".to_owned()]));
    }

    #[test]
    fn a_found_program_is_what_runs_and_nothing_is_missing() {
        let sources = sources_in(&nothing_found(), &|program, _| {
            Some(PathBuf::from(format!("/opt/bin/{program}")))
        });
        assert!(sources.iter().all(|source| source.missing.is_empty()));
        let grok = sources
            .iter()
            .find(|source| source.entry.name == "Grok")
            .expect("Grok");
        match &grok.entry.kind {
            crate::EntryKind::Agent { program, args, .. } => {
                assert_eq!(program.to_string_lossy(), "/opt/bin/grok");
                assert_eq!(args.len(), 2);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_name_canoa_cfg_uses_is_left_to_it() {
        let config = Config::from_str_in("[host.agents.Claude]\nprogram = \"x\"\n", Path::new("."))
            .expect("parses");
        let sources = sources_in(&config, &|_, _| None);
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
        let sources = sources_in(&config, &|program, _| {
            (program == "seen").then(|| PathBuf::from("/bin/seen"))
        });
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
        let sources = sources_in(&config, &|_, _| None);
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

    #[test]
    fn variables_and_the_home_directory_are_filled_in() {
        assert_eq!(expand("/plain"), Some("/plain".to_owned()));
        assert_eq!(expand("%CANOA_NO_SUCH_VARIABLE_FOR_THE_TEST%/x"), None);
        assert!(expand("~/x").is_some_and(|path| path.ends_with("/x")));
    }
}
