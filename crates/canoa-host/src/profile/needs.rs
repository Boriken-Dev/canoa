//! What an overlay needs of the computer it runs on (`runtime`: its operating systems, a Python or Node.js of a minimum
//! version, commands, and the interpreters its scripts name), and whether this computer has it. Nothing is installed here: a
//! missing program is reported with the Store runtime that provides it.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "paths, processes and clocks are std's"
)]

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;
use std::ffi::OsString;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::{Instant, SystemTime};

use crate::control::registry::which_in;
use crate::spawn::{Command, Stdio};
use crate::store::runtimes::runtime_for;

/// How long a program has to say its version.
const VERSION_TIME: Duration = Duration::from_secs(5);
/// How much of a program's answer to `--version` is read.
const VERSION_BYTES: u64 = 4096;

/// What an overlay needs, from its manifest on this computer's OS.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Required {
    /// The OSes `runtime.os` lists, when it does not admit this one.
    pub(crate) other_os: Option<Vec<String>>,
    /// A Python is needed, at least this version when one is given.
    pub(crate) python: Option<Option<String>>,
    /// A Node.js is needed, at least this version when one is given.
    pub(crate) node: Option<Option<String>>,
    /// The other interpreters its scripts name (`bash`, `sh`, `powershell`).
    pub(crate) interpreters: Vec<String>,
    /// Commands that have to be found on the `PATH`.
    pub(crate) commands: Vec<String>,
}

/// One thing an overlay needs and whether it is here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Line {
    /// `runtime.python`, `runtime.node`, `runtime.interpreter: sh`, `runtime.commands: git` or `runtime.os`.
    pub(crate) key: String,
    /// Whether this computer has it.
    pub(crate) met: bool,
    /// Where it was found, or what is missing.
    pub(crate) how: String,
    /// The Store runtime (a provider id) that installs it, when it is missing and one provides it.
    pub(crate) offer: Option<&'static str>,
    /// What preparing the overlay would run for it, for a line that preparing makes true.
    pub(crate) runs: Option<String>,
}

impl Line {
    /// The line as the control channel and the report give it.
    pub(crate) fn json(&self) -> serde_json::Value {
        let mut out = serde_json::json!({
            "key": self.key,
            "state": if self.met { "applied" } else { "off" },
            "how": self.how,
        });
        if let Some(offer) = self.offer {
            out["offer"] = serde_json::json!(offer);
        }
        if let Some(runs) = &self.runs {
            out["runs"] = serde_json::json!(runs);
        }
        out
    }
}

/// Where programs are looked for: the `PATH` the session starts from, then the folders of the runtimes the Store installed.
pub(crate) struct Finder {
    path: OsString,
    pathext: String,
    /// Reads a program's version; a seam for tests.
    version: fn(&Path) -> Option<String>,
}

impl Finder {
    /// The finder over `lookup`'s `PATH` (this process's when it gives none) and `runtimes`, searched last.
    pub(crate) fn new(lookup: &dyn Fn(&str) -> Option<String>, runtimes: &[PathBuf]) -> Self {
        let own = lookup("PATH")
            .map(OsString::from)
            .or_else(|| std::env::var_os("PATH"))
            .unwrap_or_default();
        let path =
            std::env::join_paths(std::env::split_paths(&own).chain(runtimes.iter().cloned()))
                .unwrap_or(own);
        let pathext = lookup("PATHEXT")
            .or_else(|| std::env::var("PATHEXT").ok())
            .unwrap_or_default();
        Self {
            path,
            pathext,
            version: version_of,
        }
    }

    /// The first runnable file called `name`.
    pub(crate) fn find(&self, name: &str) -> Option<PathBuf> {
        which_in(&self.path, &self.pathext, name)
    }

    #[cfg(test)]
    pub(crate) fn with_version(mut self, version: fn(&Path) -> Option<String>) -> Self {
        self.version = version;
        self
    }
}

/// The names a program is looked for under.
fn names_of(interpreter: &str) -> Vec<&str> {
    match interpreter {
        "python" if cfg!(windows) => vec!["python"],
        "python" => vec!["python3", "python"],
        "powershell" => vec!["pwsh"],
        other => vec![other],
    }
}

/// Whether `version` is at least `minimum`, each one to three dotted numbers, the missing parts `0`.
pub(crate) fn at_least(version: &str, minimum: &str) -> bool {
    parts(version) >= parts(minimum)
}

fn parts(text: &str) -> [u64; 3] {
    let mut out = [0; 3];
    for (slot, part) in out.iter_mut().zip(text.split('.')) {
        *slot = part.parse().unwrap_or(0);
    }
    out
}

/// The version a program says of itself: the first number with dots in its answer to `--version`.
pub(crate) fn version_in(answer: &str) -> Option<String> {
    answer.split_whitespace().find_map(|word| {
        let word = word.strip_prefix('v').unwrap_or(word);
        let number: String = word
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        (number.starts_with(|c: char| c.is_ascii_digit()) && !number.ends_with('.'))
            .then_some(number)
    })
}

type Stamp = (u64, Option<SystemTime>);

static VERSIONS: Mutex<BTreeMap<PathBuf, (Stamp, Option<String>)>> = Mutex::new(BTreeMap::new());

/// The version `program` says of itself, asked once for as long as the file stays as it is. A program that does not answer in
/// [`VERSION_TIME`] (a stub that opens a store page) is ended and has none.
fn version_of(program: &Path) -> Option<String> {
    let stamp = std::fs::metadata(program)
        .ok()
        .map(|meta| (meta.len(), meta.modified().ok()))?;
    if let Some((held, version)) = VERSIONS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(program)
        && *held == stamp
    {
        return version.clone();
    }
    let version = ask_version(program);
    VERSIONS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(program.to_path_buf(), (stamp, version.clone()));
    version
}

fn ask_version(program: &Path) -> Option<String> {
    let mut child = Command::new(program)
        .arg("--version")
        .stdin(Stdio::Null)
        .stdout(Stdio::Piped)
        .stderr(Stdio::Piped)
        .own_group()
        .spawn()
        .ok()?;
    let read = |stream: Option<Box<dyn std::io::Read + Send>>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(stream) = stream {
                let _ = stream.take(VERSION_BYTES).read_to_end(&mut bytes);
            }
            String::from_utf8_lossy(&bytes).into_owned()
        })
    };
    let out = read(child.stdout.take().map(|s| Box::new(s) as _));
    let err = read(child.stderr.take().map(|s| Box::new(s) as _));
    let deadline = Instant::now() + VERSION_TIME;
    let ended = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status.success()),
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            _ => break None,
        }
    };
    if ended.is_none() {
        let _ = child.kill();
        child.end_group();
        let _ = child.wait();
    }
    let (out, err) = (
        out.join().unwrap_or_default(),
        err.join().unwrap_or_default(),
    );
    // Python 2 answers on standard error.
    ended
        .filter(|ok| *ok)
        .and_then(|_| version_in(&out).or_else(|| version_in(&err)))
}

/// Looks for `interpreter`'s program, and for one of at least `minimum` when it is given: the first that is new enough, else
/// the newest found.
fn locate(finder: &Finder, interpreter: &str, minimum: Option<&str>) -> Located {
    let mut older: Option<(PathBuf, String)> = None;
    let mut unknown: Option<PathBuf> = None;
    for name in names_of(interpreter) {
        let Some(program) = finder.find(name) else {
            continue;
        };
        let Some(minimum) = minimum else {
            return Located::Found(program, None);
        };
        match (finder.version)(&program) {
            Some(version) if at_least(&version, minimum) => {
                return Located::Found(program, Some(version));
            }
            Some(version) => {
                if older
                    .as_ref()
                    .is_none_or(|(_, held)| parts(&version) > parts(held))
                {
                    older = Some((program, version));
                }
            }
            None => unknown = unknown.or(Some(program)),
        }
    }
    match (older, unknown) {
        (Some((program, version)), _) => Located::Older(program, version),
        (None, Some(program)) => Located::Unknown(program),
        (None, None) => Located::Absent,
    }
}

/// The program that runs `interpreter`'s scripts, found as [`check`] finds it: one of at least `minimum` when it is given. Never a
/// batch file, which `cmd.exe` would read the arguments of again.
///
/// # Errors
///
/// Why none was found, in words for the person.
pub(crate) fn program_for(
    finder: &Finder,
    interpreter: &str,
    minimum: Option<&str>,
) -> Result<PathBuf, String> {
    match locate(finder, interpreter, minimum) {
        Located::Found(program, _) | Located::Older(program, _) | Located::Unknown(program)
            if is_batch(&program) =>
        {
            Err(format!(
                "{} is a batch file, which is not run as {interpreter}",
                program.display()
            ))
        }
        Located::Found(program, _) => Ok(program),
        Located::Older(program, version) => Err(format!(
            "{} is {version}, and {} or later is needed",
            program.display(),
            minimum.unwrap_or("a newer version")
        )),
        Located::Unknown(program) => Err(format!(
            "{} did not say its version, and {} or later is needed",
            program.display(),
            minimum.unwrap_or("a newer version")
        )),
        Located::Absent => Err(format!("{interpreter} is not on the PATH")),
    }
}

fn is_batch(program: &Path) -> bool {
    program
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat")
        })
}

enum Located {
    Found(PathBuf, Option<String>),
    Older(PathBuf, String),
    Unknown(PathBuf),
    Absent,
}

/// The Store runtime that provides `program`, and its name.
fn offer_for(interpreter: &str) -> Option<(&'static str, &'static str)> {
    names_of(interpreter)
        .into_iter()
        .find_map(runtime_for)
        .map(|id| (id, store_title(id)))
}

/// The Store runtime (a provider id) that provides `interpreter`, when one does.
pub(crate) fn offer_id(interpreter: &str) -> Option<&'static str> {
    offer_for(interpreter).map(|(id, _)| id)
}

fn store_title(id: &str) -> &'static str {
    match id {
        "node_store" => "Node.js",
        "python_store" => "Python",
        "git_bash_store" => "Git Bash",
        "powershell_store" => "PowerShell",
        _ => "a runtime",
    }
}

fn label(interpreter: &str) -> &str {
    match interpreter {
        "python" => "Python",
        "node" => "Node.js",
        "powershell" => "PowerShell 7",
        other => other,
    }
}

fn line_for(
    key: String,
    finder: &Finder,
    interpreter: &str,
    minimum: Option<&str>,
    shown: &str,
) -> Line {
    let offer = offer_for(interpreter);
    let installs = |offer: Option<(&'static str, &'static str)>| {
        offer.map_or_else(String::new, |(_, title)| {
            format!("; the Store's {title} installs it")
        })
    };
    let wanted = minimum.map_or_else(String::new, |minimum| format!(" {minimum} or later"));
    let (met, how) = match locate(finder, interpreter, minimum) {
        Located::Found(program, version) => (
            true,
            format!(
                "found {}{}",
                program.display(),
                version.map_or_else(String::new, |version| format!(" ({version})"))
            ),
        ),
        Located::Older(program, version) => (
            false,
            format!(
                "{shown}{wanted} is needed and {} is {version}{}",
                program.display(),
                installs(offer)
            ),
        ),
        Located::Unknown(program) => (
            false,
            format!(
                "{shown}{wanted} is needed and {} did not say its version{}",
                program.display(),
                installs(offer)
            ),
        ),
        Located::Absent => (
            false,
            format!(
                "{shown}{wanted} is needed and is not on the PATH{}",
                installs(offer)
            ),
        ),
    };
    Line {
        key,
        met,
        how,
        offer: if met { None } else { offer.map(|(id, _)| id) },
        runs: None,
    }
}

/// What the overlay installed as `identity` in the overlays folder `store` needs of this computer, read from its files as the
/// record's digest names them.
///
/// # Errors
///
/// Why the overlay cannot be read.
pub(crate) fn of_installed(
    store: &Path,
    identity: &str,
    os: &str,
    finder: &Finder,
) -> Result<Vec<Line>, String> {
    let record: serde_json::Value = std::fs::read(store.join(format!("{identity}.json")))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .ok_or_else(|| "it is not installed".to_owned())?;
    let expected = record
        .get("digest")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "its record holds no digest".to_owned())?;
    let root = store.join(identity);
    let (tree, found) =
        coqui_aos::read_tree(&coqui_aos::StdFs, &root, &coqui_aos::Settings::default())
            .map_err(|error| format!("its files cannot be read ({})", error.class))?;
    if !found.digest.to_string().eq_ignore_ascii_case(expected) {
        return Err("its files are not those its record's digest names".to_owned());
    }
    let root = root.to_string_lossy().into_owned();
    let layer = super::overlay_layer(&super::OverlayInput::new(identity, &root, os, &tree))
        .map_err(|refusal| refusal.to_string())?;
    let aos = layer
        .aos
        .as_deref()
        .ok_or_else(|| "its manifest cannot be applied".to_owned())?;
    let inspected =
        super::overlay::inspect(aos).ok_or_else(|| "its manifest cannot be applied".to_owned())?;
    let mut lines = check(&inspected.required, finder, os);
    lines.extend(super::prepare::lines_for(
        &super::prepare::Home::new(store),
        identity,
        &aos.digest,
        &inspected.dependencies,
        inspected.setup.as_ref(),
    ));
    lines.extend(
        super::commands::check(Path::new(&root), &inspected.commands, finder)
            .into_iter()
            .map(|(command, result)| super::commands::line(&command, result)),
    );
    Ok(lines)
}

/// Whether this computer has each thing `required` lists, in order: operating system, Python, Node.js, other interpreters,
/// commands. `os` is this computer's, as AOS names it.
pub(crate) fn check(required: &Required, finder: &Finder, os: &str) -> Vec<Line> {
    let mut lines = Vec::new();
    if let Some(listed) = &required.other_os {
        lines.push(Line {
            key: "runtime.os".to_owned(),
            met: false,
            how: format!("it names {}; this computer is {os}", listed.join(", ")),
            offer: None,
            runs: None,
        });
    }
    for (key, asked, interpreter) in [
        ("runtime.python", &required.python, "python"),
        ("runtime.node", &required.node, "node"),
    ] {
        if let Some(minimum) = asked {
            lines.push(line_for(
                key.to_owned(),
                finder,
                interpreter,
                minimum.as_deref(),
                label(interpreter),
            ));
        }
    }
    for interpreter in &required.interpreters {
        lines.push(line_for(
            format!("runtime.interpreter: {interpreter}"),
            finder,
            interpreter,
            None,
            label(interpreter),
        ));
    }
    for command in &required.commands {
        lines.push(line_for(
            format!("runtime.commands: {command}"),
            finder,
            command,
            None,
            command,
        ));
    }
    lines
}

#[cfg(test)]
mod tests;
