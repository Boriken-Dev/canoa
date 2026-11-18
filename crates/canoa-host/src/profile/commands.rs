//! The commands an overlay's `scripts` expose. Only a `scripts` entry makes one (no directory does): each is a small launcher in
//! a folder of its own that goes on the `PATH` of the agent's processes. A launcher runs the script under its interpreter, found
//! on the host's own `PATH` and never through an overlay's, with the arguments it was given, in the caller's directory and
//! environment, and ends with the script's status. Nothing times it out or reads its output.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files and paths are std's"
)]

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::needs::{Finder, Line, offer_id, program_for};

/// One command a manifest's `scripts` names for this OS.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Declared {
    /// The command's name.
    pub(crate) name: String,
    /// The script, a path relative to the overlay's root.
    pub(crate) path: String,
    /// The interpreter that runs it.
    pub(crate) interpreter: String,
}

/// The interpreter a script path's extension stands for.
fn by_extension(path: &str) -> &'static str {
    match path.rsplit_once('.').map_or("", |(_, extension)| extension) {
        "py" => "python",
        "sh" => "sh",
        "bash" => "bash",
        "ps1" => "powershell",
        _ => "node",
    }
}

/// The commands `manifest` exposes on `os`: its `scripts` entries that are a path or name an interpreter, not those that are
/// `false`, and not those whose `os` leaves this one out.
pub(crate) fn declared(manifest: &Map<String, Value>, os: &str) -> Vec<Declared> {
    let Some(scripts) = manifest.get("scripts").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut out: Vec<Declared> = scripts
        .iter()
        .filter_map(|(name, target)| match target {
            Value::String(path) => Some(Declared {
                name: name.clone(),
                path: path.clone(),
                interpreter: by_extension(path).to_owned(),
            }),
            Value::Object(entry) => {
                let list: Option<Vec<String>> =
                    entry.get("os").and_then(Value::as_array).map(|names| {
                        names
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    });
                if !coqui_aos::os_matches(list.as_deref(), os) {
                    return None;
                }
                let path = entry.get("path")?.as_str()?;
                Some(Declared {
                    name: name.clone(),
                    path: path.to_owned(),
                    interpreter: entry
                        .get("interpreter")
                        .and_then(Value::as_str)
                        .map_or_else(|| by_extension(path).to_owned(), str::to_owned),
                })
            }
            _ => None,
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// The launcher's contents for `script` run by `program` (`prefix` before the script: the arguments an interpreter needs).
fn launcher(windows: bool, program: &Path, prefix: &[&str], script: &Path) -> String {
    if windows {
        let quote = |text: &str| format!("\"{}\"", text.replace('%', "%%"));
        let mut line = quote(&program.to_string_lossy());
        for each in prefix {
            line.push(' ');
            line.push_str(each);
        }
        line.push(' ');
        line.push_str(&quote(&script.to_string_lossy()));
        format!("@echo off\r\n{line} %*\r\nexit /b %errorlevel%\r\n")
    } else {
        let quote = |text: &str| format!("'{}'", text.replace('\'', "'\\''"));
        let mut line = format!("exec {}", quote(&program.to_string_lossy()));
        for each in prefix {
            line.push(' ');
            line.push_str(each);
        }
        line.push(' ');
        line.push_str(&quote(&script.to_string_lossy()));
        format!("#!/bin/sh\n{line} \"$@\"\n")
    }
}

/// The script, the program that runs it and the arguments that program takes before the script.
fn resolved(
    root: &Path,
    command: &Declared,
    finder: &Finder,
) -> Result<(PathBuf, PathBuf, &'static [&'static str]), String> {
    let script = root.join(&command.path);
    if !script.is_file() {
        return Err(format!("{} is not a file of the overlay", command.path));
    }
    let program = program_for(finder, &command.interpreter, None)?;
    let prefix: &[&str] = if command.interpreter == "powershell" {
        &["-NoLogo", "-NoProfile", "-NonInteractive", "-File"]
    } else {
        &[]
    };
    Ok((script, program, prefix))
}

/// Whether each of `commands` could be exposed, writing nothing: its script is a file of the overlay at `root` and its interpreter
/// is found.
pub(crate) fn check(
    root: &Path,
    commands: &[Declared],
    finder: &Finder,
) -> Vec<(Declared, Result<(), String>)> {
    commands
        .iter()
        .map(|command| (command.clone(), resolved(root, command, finder).map(|_| ())))
        .collect()
}

/// Makes the launchers of `commands` in `dir`: one for each whose script is a file of the overlay at `root` and whose interpreter
/// is found. A launcher already as it would be is left alone, and one for a command no longer exposed is taken away, so a
/// session that starts while another runs never finds the folder empty. The answer has each command's launcher, or why it has none.
pub(crate) fn expose(
    dir: &Path,
    root: &Path,
    commands: &[Declared],
    finder: &Finder,
    windows: bool,
) -> Vec<(Declared, Result<PathBuf, String>)> {
    let made: Vec<(Declared, Result<PathBuf, String>)> = commands
        .iter()
        .map(|command| {
            let made = (|| {
                let (script, program, prefix) = resolved(root, command, finder)?;
                std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
                let file = dir.join(if windows {
                    format!("{}.cmd", command.name)
                } else {
                    command.name.clone()
                });
                let text = launcher(windows, &program, prefix, &script);
                if std::fs::read_to_string(&file).ok().as_deref() != Some(text.as_str()) {
                    std::fs::write(&file, text).map_err(|error| error.to_string())?;
                }
                #[cfg(unix)]
                std::fs::set_permissions(
                    &file,
                    std::os::unix::fs::PermissionsExt::from_mode(0o755),
                )
                .map_err(|error| error.to_string())?;
                Ok(file)
            })();
            (command.clone(), made)
        })
        .collect();
    if let Ok(listed) = std::fs::read_dir(dir) {
        for entry in listed.flatten() {
            if !made
                .iter()
                .any(|(_, launcher)| launcher.as_ref().is_ok_and(|file| *file == entry.path()))
            {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    made
}

/// The report line of one command: exposed, or why not (with the Store runtime that provides its interpreter, when that is
/// what is missing).
pub(crate) fn line(command: &Declared, result: Result<(), String>) -> Line {
    let key = format!("scripts.{}", command.name);
    match result {
        Ok(()) => Line {
            key,
            met: true,
            how: format!(
                "the command {} runs {} under {}",
                command.name, command.path, command.interpreter
            ),
            offer: None,
            runs: None,
        },
        Err(why) => Line {
            key,
            met: false,
            offer: if why.ends_with("is not on the PATH") {
                offer_id(&command.interpreter)
            } else {
                None
            },
            how: format!("the command {} is not exposed: {why}", command.name),
            runs: None,
        },
    }
}

/// The names two commands share when compared without regard to ASCII case, by the overlays that expose them: the specification
/// has a consumer report such a collision and expose neither.
pub(crate) fn collisions(all: &[(String, Vec<Declared>)]) -> BTreeMap<String, Vec<String>> {
    let mut by_name: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (identity, commands) in all {
        for command in commands {
            by_name
                .entry(command.name.to_ascii_lowercase())
                .or_default()
                .push(format!("{identity} {}", command.name));
        }
    }
    by_name.retain(|_, who| who.len() > 1);
    by_name
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn manifest(scripts: &Value) -> Map<String, Value> {
        let Value::Object(map) = json!({ "scripts": scripts }) else {
            unreachable!()
        };
        map
    }

    #[test]
    fn a_scripts_entry_is_a_path_an_interpreter_or_false_and_an_os_list_narrows_it() {
        let scripts = manifest(&json!({
            "check": "bin/check.py",
            "lint": {"path": "tools/lint.sh", "interpreter": "bash", "os": ["linux", "macos"]},
            "win": {"path": "tools/win.ps1", "os": ["windows"]},
            "old": false,
            "tool": "bin/tool.mjs",
        }));
        let linux = declared(&scripts, "linux");
        let names: Vec<(&str, &str)> = linux
            .iter()
            .map(|each| (each.name.as_str(), each.interpreter.as_str()))
            .collect();
        assert_eq!(
            names,
            [("check", "python"), ("lint", "bash"), ("tool", "node")]
        );
        let windows = declared(&scripts, "windows");
        let names: Vec<(&str, &str)> = windows
            .iter()
            .map(|each| (each.name.as_str(), each.interpreter.as_str()))
            .collect();
        assert_eq!(
            names,
            [("check", "python"), ("tool", "node"), ("win", "powershell")]
        );
        assert!(declared(&Map::new(), "linux").is_empty());
    }

    /// The Python the tests run launchers with, or none.
    fn python() -> Option<PathBuf> {
        let named = PathBuf::from(std::env::var_os("CANOA_TEST_PYTHON")?);
        if named.is_absolute() {
            return Some(named);
        }
        let lookup = |name: &str| std::env::var(name).ok();
        Finder::new(&lookup, &[]).find(&named.to_string_lossy())
    }

    #[test]
    fn a_launcher_made_for_a_script_runs_it_with_the_arguments_it_is_given_and_ends_with_its_status()
     {
        let Some(python) = python() else {
            eprintln!("NOT RUN: set CANOA_TEST_PYTHON to a Python 3");
            return;
        };
        let root = std::env::temp_dir().join(format!("canoa-commands-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("overlay/bin")).expect("a folder");
        std::fs::write(
            root.join("overlay/bin/show.py"),
            "import sys\nprint('|'.join(sys.argv[1:]))\nsys.exit(4)\n",
        )
        .expect("a script");
        let folder = python
            .parent()
            .expect("a folder")
            .to_string_lossy()
            .into_owned();
        let lookup = move |name: &str| (name == "PATH").then(|| folder.clone());
        let finder = Finder::new(&lookup, &[]);
        let wanted = declared(
            &manifest(&json!({"show": "bin/show.py", "gone": "bin/gone.py"})),
            "linux",
        );
        let made = expose(
            &root.join("launchers"),
            &root.join("overlay"),
            &wanted,
            &finder,
            cfg!(windows),
        );
        let (gone, why) = (&made[0].0, made[0].1.as_ref().expect_err("no script"));
        assert_eq!(gone.name, "gone");
        assert!(why.contains("is not a file of the overlay"), "{why}");
        let launcher = made[1].1.as_ref().expect("a launcher");
        let mut run = if cfg!(windows) {
            let mut command = std::process::Command::new("cmd.exe");
            command.args(["/d", "/c"]).arg(launcher);
            command
        } else {
            std::process::Command::new(launcher)
        };
        let output = run.args(["one", "two words"]).output().expect("runs");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "one|two words"
        );
        assert_eq!(
            output.status.code(),
            Some(4),
            "the launcher ends with the script's status"
        );
        // A launcher a second call does not name is gone, and none is written for a command that could not be.
        let again = expose(
            &root.join("launchers"),
            &root.join("overlay"),
            &wanted[1..],
            &finder,
            cfg!(windows),
        );
        assert_eq!(again.len(), 1);
        assert_eq!(
            std::fs::read_dir(root.join("launchers"))
                .expect("a folder")
                .count(),
            1
        );
        let line_met = line(&made[1].0, Ok(()));
        assert!(
            line_met.met && line_met.how.contains("bin/show.py under python"),
            "{line_met:?}"
        );
        let line_gone = line(gone, Err(why.clone()));
        assert!(!line_gone.met && line_gone.offer.is_none());
        let absent = line(&wanted[1], Err("python is not on the PATH".to_owned()));
        assert_eq!(absent.offer, super::super::needs::offer_id("python"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn two_overlays_exposing_one_name_collide_whatever_its_case() {
        let named = |name: &str| Declared {
            name: name.to_owned(),
            path: "bin/x.py".to_owned(),
            interpreter: "python".to_owned(),
        };
        let all = vec![
            ("a".to_owned(), vec![named("Check"), named("lint")]),
            ("b".to_owned(), vec![named("check")]),
        ];
        let clashes = collisions(&all);
        assert_eq!(clashes.keys().collect::<Vec<_>>(), ["check"]);
        assert_eq!(clashes["check"], ["a Check", "b check"]);
        assert!(collisions(&all[..1]).is_empty());
    }

    #[test]
    fn a_launcher_runs_the_script_under_its_interpreter_with_the_callers_arguments() {
        let unix = launcher(
            false,
            Path::new("/usr/bin/python3"),
            &[],
            Path::new("/o/it's/check.py"),
        );
        assert_eq!(
            unix,
            "#!/bin/sh\nexec '/usr/bin/python3' '/o/it'\\''s/check.py' \"$@\"\n"
        );
        let windows = launcher(
            true,
            Path::new("C:\\Program Files\\PowerShell\\pwsh.exe"),
            &["-NoLogo", "-File"],
            Path::new("C:\\o\\100%\\win.ps1"),
        );
        assert_eq!(
            windows,
            "@echo off\r\n\"C:\\Program Files\\PowerShell\\pwsh.exe\" -NoLogo -File \"C:\\o\\100%%\\win.ps1\" %*\r\nexit /b %errorlevel%\r\n"
        );
    }
}
