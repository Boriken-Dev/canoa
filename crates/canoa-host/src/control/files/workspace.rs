//! Starting an editor or the file manager on this computer for a folder of a session: `workspace/openers` says what this
//! computer has, `workspace/open` starts one detached. Both are behind the grant `workspace.open` and keep to the places `fs/info`
//! does.

#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files, environment and processes are std's"
)]

use std::fmt;
use std::path::Path;
use std::sync::Arc;

use serde_json::{Value, json};

use super::{Files, Outcome};
use crate::control::{Caller, NOT_FOUND, REFUSED, bad, text_param};
use crate::spawn::{Command, Stdio};

/// The bundle id of VS Code on a Mac.
const VS_CODE_BUNDLE: &str = "com.microsoft.VSCode";
/// Where Windows records Code.exe, under `HKCU` or `HKLM`.
const APP_PATHS: &str = r"\Software\Microsoft\Windows\CurrentVersion\App Paths\Code.exe";

/// The system this host runs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Os {
    Windows,
    Macos,
    Linux,
}

impl Os {
    fn name(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::Macos => "macos",
            Self::Linux => "linux",
        }
    }
}

/// A program and its arguments, started as they are: never through a shell, never joined into a command line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Plan {
    pub(crate) program: String,
    pub(crate) args: Vec<String>,
}

fn plan(program: impl Into<String>, args: &[&str]) -> Plan {
    Plan {
        program: program.into(),
        args: args.iter().map(|arg| (*arg).to_owned()).collect(),
    }
}

/// What detecting the programs needs from the computer, and starting one. Tests replace it with a stand-in that starts nothing.
pub(crate) trait Machine: Send + Sync {
    fn os(&self) -> Os;
    fn var(&self, name: &str) -> Option<String>;
    fn is_file(&self, path: &str) -> bool;
    /// The default value of a registry key under `HKEY_...`, Windows only.
    fn registry_default(&self, key: &str) -> Option<String>;
    /// The folder of the application bundle `id` on a Mac.
    fn bundle_path(&self, id: &str) -> Option<String>;
    /// Starts `plan` and does not wait for it.
    fn start(&self, plan: &Plan) -> Result<(), String>;
}

/// This computer.
struct Real;

impl Machine for Real {
    fn os(&self) -> Os {
        if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::Macos
        } else {
            Os::Linux
        }
    }

    fn var(&self, name: &str) -> Option<String> {
        let found = std::env::var(name).ok();
        // Windows names are not case-sensitive.
        if found.is_none() && cfg!(windows) {
            return std::env::vars()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| value);
        }
        found
    }

    fn is_file(&self, path: &str) -> bool {
        Path::new(path).is_file()
    }

    fn registry_default(&self, key: &str) -> Option<String> {
        if !cfg!(windows) {
            return None;
        }
        let root = self.var("SystemRoot")?;
        let output = Command::new(format!(r"{root}\System32\reg.exe"))
            .args(["query", key, "/ve"])
            .output()
            .ok()?;
        output
            .status
            .success()
            .then(|| registry_value(&String::from_utf8_lossy(&output.stdout), self))
            .flatten()
    }

    fn bundle_path(&self, id: &str) -> Option<String> {
        if !cfg!(target_os = "macos") {
            return None;
        }
        if let Ok(output) = Command::new("mdfind")
            .arg(format!("kMDItemCFBundleIdentifier == \"{id}\""))
            .output()
            && output.status.success()
        {
            let text = String::from_utf8_lossy(&output.stdout).into_owned();
            if let Some(found) = text
                .lines()
                .find(|line| line.ends_with(".app") && Path::new(line).is_dir())
            {
                return Some(found.to_owned());
            }
        }
        let home = self.var("HOME");
        ["/Applications".to_owned()]
            .into_iter()
            .chain(home.map(|home| format!("{home}/Applications")))
            .map(|folder| format!("{folder}/Visual Studio Code.app"))
            .find(|app| Path::new(app).is_dir())
    }

    fn start(&self, plan: &Plan) -> Result<(), String> {
        // No shell, no window of its own, and nothing is waited for on this thread: Explorer answers 1 after it succeeds, and a
        // started editor lives on after the call.
        let mut command = Command::new(&plan.program);
        command
            .args(&plan.args)
            .stdin(Stdio::Null)
            .stdout(Stdio::Null)
            .stderr(Stdio::Null);
        let mut child = command
            .spawn()
            .map_err(|error| format!("could not start {}: {error}", plan.program))?;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }
}

/// The value a `reg query ... /ve` printed, with `%NAME%` filled in from the environment; none when it printed none.
fn registry_value(output: &str, machine: &dyn Machine) -> Option<String> {
    let line = output.lines().find(|line| line.contains("REG_"))?;
    let (_, rest) = line.trim().split_once("REG_")?;
    let (type_name, value) = rest.split_once(char::is_whitespace)?;
    let mut value = value.trim().to_owned();
    if value.len() > 1 && value.starts_with('"') && value.ends_with('"') {
        value = value[1..value.len() - 1].to_owned();
    }
    if type_name == "EXPAND_SZ" {
        let mut expanded = String::new();
        let mut rest = value.as_str();
        while let Some((before, after)) = rest.split_once('%') {
            expanded.push_str(before);
            match after.split_once('%') {
                Some((name, tail)) => {
                    match machine.var(name) {
                        Some(found) => expanded.push_str(&found),
                        None => {
                            expanded.push('%');
                            expanded.push_str(name);
                            expanded.push('%');
                        }
                    }
                    rest = tail;
                }
                None => {
                    expanded.push('%');
                    rest = after;
                }
            }
        }
        expanded.push_str(rest);
        value = expanded;
    }
    (!value.is_empty()).then_some(value)
}

/// What can be started on this computer, and how.
#[derive(Clone)]
pub(crate) struct Opener {
    machine: Arc<dyn Machine>,
}

impl fmt::Debug for Opener {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Opener").finish_non_exhaustive()
    }
}

impl Default for Opener {
    fn default() -> Self {
        Self {
            machine: Arc::new(Real),
        }
    }
}

impl Opener {
    #[cfg(test)]
    pub(crate) fn on(machine: Arc<dyn Machine>) -> Self {
        Self { machine }
    }

    /// Starts `plan` on the computer.
    #[cfg(test)]
    pub(crate) fn start(&self, plan: &Plan) -> Result<(), String> {
        self.machine.start(plan)
    }

    fn join(&self, folder: &str, name: &str) -> String {
        let separator = if self.machine.os() == Os::Windows {
            '\\'
        } else {
            '/'
        };
        if folder.ends_with(separator) {
            format!("{folder}{name}")
        } else {
            format!("{folder}{separator}{name}")
        }
    }

    /// A file called `name` in a folder of `PATH`.
    fn on_path(&self, name: &str) -> Option<String> {
        let path = self.machine.var("PATH")?;
        let separator = if self.machine.os() == Os::Windows {
            ';'
        } else {
            ':'
        };
        path.split(separator)
            .filter(|folder| !folder.is_empty())
            .map(|folder| self.join(&folder.replace('"', ""), name))
            .find(|candidate| self.machine.is_file(candidate))
    }

    /// What opens a folder in VS Code: `Code.exe` on Windows (never `code.cmd`, which only a shell can run), `open` with the
    /// bundle's path on a Mac, `code` on Linux. None when VS Code is not found.
    pub(crate) fn vs_code(&self, path: &str) -> Option<Plan> {
        match self.machine.os() {
            Os::Windows => {
                let mut candidates: Vec<Option<String>> = ["HKCU", "HKLM"]
                    .iter()
                    .map(|root| self.machine.registry_default(&format!("{root}{APP_PATHS}")))
                    .collect();
                candidates.push(
                    self.machine
                        .var("LOCALAPPDATA")
                        .map(|local| self.join(&local, r"Programs\Microsoft VS Code\Code.exe")),
                );
                candidates.push(
                    self.machine
                        .var("ProgramFiles")
                        .map(|files| self.join(&files, r"Microsoft VS Code\Code.exe")),
                );
                candidates.push(self.on_path("code.cmd").and_then(|cmd| {
                    cmd.rfind(['\\', '/'])
                        .map(|cut| self.join(&cmd[..cut], r"..\Code.exe"))
                }));
                candidates.push(self.on_path("Code.exe"));
                candidates
                    .into_iter()
                    .flatten()
                    .find(|candidate| {
                        candidate.to_ascii_lowercase().ends_with("code.exe")
                            && self.machine.is_file(candidate)
                    })
                    .map(|found| plan(found, &[path]))
            }
            Os::Macos => self
                .machine
                .bundle_path(VS_CODE_BUNDLE)
                .map(|bundle| plan("open", &["-a", &bundle, path])),
            Os::Linux => self.on_path("code").map(|code| plan(code, &[path])),
        }
    }

    /// What the file manager runs to show `path`: its folder opened, or with `item` the entry selected in its parent.
    pub(crate) fn file_manager(&self, path: &str, item: bool) -> Plan {
        match self.machine.os() {
            Os::Windows => {
                let explorer = self.machine.var("SystemRoot").map_or_else(
                    || "explorer.exe".to_owned(),
                    |root| self.join(&root, "explorer.exe"),
                );
                // `/select,` and the path stay two arguments: the path is one argument whatever it holds.
                if item {
                    plan(explorer, &["/select,", path])
                } else {
                    plan(explorer, &[path])
                }
            }
            Os::Macos => {
                if item {
                    plan("open", &["-R", path])
                } else {
                    plan("open", &[path])
                }
            }
            Os::Linux => {
                if !item {
                    return plan("xdg-open", &[path]);
                }
                let parent = match path.rfind('/') {
                    Some(0) | None => "/",
                    Some(cut) => &path[..cut],
                };
                plan("xdg-open", &[parent])
            }
        }
    }

    fn manager_name(&self) -> &'static str {
        match self.machine.os() {
            Os::Windows => "Explorer",
            Os::Macos => "Finder",
            Os::Linux => "file manager",
        }
    }
}

impl Files {
    /// `workspace/openers` `{}` -> `{"os", "hostname"?, "openers": [{"id", "label"}]}`: what this computer can start for a folder, by
    /// `id` (`vscode` only when VS Code is found, `file_manager`, `reveal`). It starts nothing.
    pub(crate) fn workspace_openers(&self) -> Outcome {
        let opener = &self.opener;
        let mut openers = Vec::new();
        if opener.vs_code("").is_some() {
            openers.push(json!({"id": "vscode", "label": "Open in VS Code"}));
        }
        let manager = opener.manager_name();
        openers.push(json!({"id": "file_manager", "label": format!("Open in {manager}")}));
        openers.push(json!({"id": "reveal", "label": format!("Reveal in {manager}")}));
        let hostname = opener
            .machine
            .var("COMPUTERNAME")
            .or_else(|| opener.machine.var("HOSTNAME"))
            .or_else(|| opener.machine.var("HOST"));
        Ok(json!({
            "os": opener.machine.os().name(),
            "hostname": hostname,
            "openers": openers,
        }))
    }

    /// `workspace/open` `{"path", "opener"}` -> `{"started": true}`: starts the opener (`vscode`, `file_manager` or `reveal`) for the
    /// folder (for `reveal` a file or folder to select) as this user, detached. The path is as `fs/info` takes it.
    pub(crate) fn workspace_open(&self, caller: &Caller, params: &Value) -> Outcome {
        let path = text_param(params, "path")?.ok_or_else(|| bad("path is needed"))?;
        let which = text_param(params, "opener")?.ok_or_else(|| bad("opener is needed"))?;
        if !matches!(which, "vscode" | "file_manager" | "reveal") {
            return Err(bad("opener is vscode, file_manager or reveal"));
        }
        let place = self.place(caller, path)?;
        let text = place.to_string_lossy().into_owned();
        let folder = place.is_dir();
        if which != "reveal" && !folder {
            return Err((NOT_FOUND, format!("the folder is not there: {path}")));
        }
        let opener = &self.opener;
        let chosen = match which {
            "vscode" => opener
                .vs_code(&text)
                .ok_or_else(|| (REFUSED, "VS Code was not found on this computer".to_owned()))?,
            "file_manager" => opener.file_manager(&text, false),
            _ => opener.file_manager(&text, true),
        };
        opener
            .machine
            .start(&chosen)
            .map_err(|why| (REFUSED, why))?;
        Ok(json!({"started": true}))
    }
}
