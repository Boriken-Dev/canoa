//! The shells on this computer, and the one a host, an agent and a terminal use.
//!
//! [`detect`] lists the shells it finds, whether Canoa installed them or not: Git Bash (a copy the Store installed, then Git
//! for Windows' own and the one a `git` on the `PATH` belongs to), MSYS2's bash, WSL's `bash.exe`, any other bash and zsh,
//! PowerShell 7 and Windows PowerShell. Each has an id (`git-bash`, `msys2`, `wsl`, `bash`, `zsh`, `pwsh`, `powershell`,
//! `canoa-git-bash`, `canoa-pwsh`), which `[host] shell`, an agent entry's `shell` and `terminal/open` name; a setting that is
//! no id is a program, an absolute path or a name found on the `PATH`.
//!
//! One setting, translated for each agent by [`for_harness`]: only what a harness documents is passed (a variable it reads, say),
//! and what it cannot take is said in words, by the agent's own provider.
//! Nothing here writes a program's own settings file.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "paths and environments are std's"
)]

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use canoa::vendor::ShellRequest;
use serde_json::{Value, json};

/// What kind of shell a program is, by its name: what a harness's setting for a bash or a PowerShell takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Family {
    /// `bash`, and `sh`, which Git for Windows' own is named by too.
    Bash,
    /// `zsh`.
    Zsh,
    /// `pwsh` and `powershell`.
    PowerShell,
    /// Any other program.
    Other,
}

impl Family {
    /// The family a program's file name stands for.
    #[must_use]
    pub fn of(path: &Path) -> Self {
        match stem(path).as_str() {
            "bash" | "sh" => Self::Bash,
            "zsh" => Self::Zsh,
            "pwsh" | "powershell" => Self::PowerShell,
            _ => Self::Other,
        }
    }

    /// Its name on the control channel.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::Zsh => "zsh",
            Self::PowerShell => "powershell",
            Self::Other => "other",
        }
    }
}

/// Where a shell comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Origin {
    /// A copy the Store installed.
    Canoa,
    /// Git for Windows' own.
    GitForWindows,
    /// MSYS2's.
    Msys2,
    /// WSL's `bash.exe`, a launcher for a Linux distribution's own bash.
    Wsl,
    /// The system's: found on the `PATH` or in its usual place.
    System,
    /// A program the setting names.
    Custom,
}

impl Origin {
    /// Its name on the control channel.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Canoa => "canoa",
            Self::GitForWindows => "git-for-windows",
            Self::Msys2 => "msys2",
            Self::Wsl => "wsl",
            Self::System => "system",
            Self::Custom => "custom",
        }
    }

    /// Where a bash at `path` comes from, by the folders it is in.
    fn of_bash(path: &Path) -> Self {
        let text = path
            .to_string_lossy()
            .replace('\\', "/")
            .to_ascii_lowercase();
        if text.contains("/windows/system32/") || text.contains("/windows/sysnative/") {
            Self::Wsl
        } else if text.contains("msys") {
            Self::Msys2
        } else if text.contains("/git/") {
            Self::GitForWindows
        } else {
            Self::System
        }
    }
}

/// One shell found on this computer.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Shell {
    /// What a setting names it by: `git-bash`, `wsl`, `pwsh`, ...; a second of a kind has `-2` after it.
    pub id: String,
    /// What a person is shown.
    pub label: String,
    /// The program, as an absolute path.
    pub path: PathBuf,
    /// Its kind.
    pub family: Family,
    /// Where it comes from.
    pub origin: Origin,
}

impl Shell {
    /// This shell as the control channel lists it.
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "label": self.label,
            "path": self.path.to_string_lossy(),
            "family": self.family.name(),
            "origin": self.origin.name(),
        })
    }
}

/// Where shells are looked for: the system's own facts, which a test replaces with a folder's.
#[derive(Clone, Debug, Default)]
pub struct Lookup {
    /// Whether the system is Windows, which decides the names and places looked in.
    pub windows: bool,
    /// The folders of the `PATH` that are absolute: a relative one is never searched.
    pub path: Vec<PathBuf>,
    /// `%ProgramFiles%` and `%ProgramFiles(x86)%`.
    pub program_files: Vec<PathBuf>,
    /// `%LocalAppData%`.
    pub local_app_data: Option<PathBuf>,
    /// `%SystemRoot%`.
    pub system_root: Option<PathBuf>,
    /// The folders MSYS2 is usually installed in.
    pub msys2: Vec<PathBuf>,
    /// The person's own login shell (`$SHELL`).
    pub login: Option<PathBuf>,
    /// The Store's copy of Git Bash and of PowerShell, when it installed one: the launch file of each.
    pub git_bash: Option<PathBuf>,
    /// See `git_bash`.
    pub powershell: Option<PathBuf>,
}

impl Lookup {
    /// This computer, with the copies the Store installed.
    #[must_use]
    pub fn here() -> Self {
        let var = |name: &str| {
            std::env::var_os(name)
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
        };
        let windows = cfg!(windows);
        Self {
            windows,
            path: std::env::var_os("PATH")
                .map(|path| {
                    std::env::split_paths(&path)
                        .filter(|dir| dir.is_absolute())
                        .collect()
                })
                .unwrap_or_default(),
            program_files: ["ProgramFiles", "ProgramFiles(x86)"]
                .iter()
                .filter_map(|name| var(name))
                .collect(),
            local_app_data: var("LocalAppData"),
            system_root: var("SystemRoot"),
            msys2: if windows {
                vec![PathBuf::from("C:/msys64"), PathBuf::from("C:/msys32")]
            } else {
                Vec::new()
            },
            login: var("SHELL"),
            git_bash: crate::sources::managed_program("git-bash").map(|(path, _)| path),
            powershell: crate::sources::managed_program("powershell").map(|(path, _)| path),
        }
    }

    fn exe(&self, name: &str) -> String {
        if self.windows {
            format!("{name}.exe")
        } else {
            name.to_owned()
        }
    }

    /// Every file `name` is on the `PATH`, in order.
    fn on_path(&self, name: &str) -> Vec<PathBuf> {
        let file = self.exe(name);
        self.path
            .iter()
            .map(|dir| dir.join(&file))
            .filter(|path| runs(path))
            .collect()
    }
}

/// The lower-case name of a program without its extension.
fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

/// Whether `path` is a file a person can run.
fn runs(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        meta.is_file() && meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        meta.is_file()
    }
}

/// The shells found, in the order a person is offered them: Git Bash first on Windows, bash first elsewhere, PowerShell last.
#[must_use]
pub fn detect(lookup: &Lookup) -> Vec<Shell> {
    let mut found = Found::default();
    if lookup.windows {
        windows_bashes(lookup, &mut found);
    } else {
        for name in ["bash", "zsh"] {
            let family = if name == "bash" {
                Family::Bash
            } else {
                Family::Zsh
            };
            let places = [format!("/bin/{name}"), format!("/usr/bin/{name}")]
                .into_iter()
                .map(PathBuf::from)
                .chain(
                    ["/usr/local/bin", "/opt/homebrew/bin"]
                        .iter()
                        .map(|dir| Path::new(dir).join(name)),
                );
            for path in lookup.on_path(name).into_iter().chain(places) {
                if runs(&path) {
                    found.push(name, name, &path, family, Origin::System);
                }
            }
        }
    }
    if let Some(path) = lookup.powershell.as_deref().filter(|path| runs(path)) {
        found.push(
            "canoa-pwsh",
            "PowerShell 7 (installed by Canoa)",
            path,
            Family::PowerShell,
            Origin::Canoa,
        );
    }
    let mut pwsh = lookup.on_path("pwsh");
    if lookup.windows {
        pwsh.extend(
            lookup
                .program_files
                .iter()
                .map(|dir| dir.join("PowerShell/7/pwsh.exe")),
        );
    } else {
        pwsh.extend(["/opt/microsoft/powershell/7/pwsh", "/usr/bin/pwsh"].map(PathBuf::from));
    }
    for path in pwsh {
        if runs(&path) {
            found.push(
                "pwsh",
                "PowerShell 7",
                &path,
                Family::PowerShell,
                Origin::System,
            );
        }
    }
    if lookup.windows
        && let Some(root) = &lookup.system_root
    {
        let path = root.join("System32/WindowsPowerShell/v1.0/powershell.exe");
        if runs(&path) {
            found.push(
                "powershell",
                "Windows PowerShell",
                &path,
                Family::PowerShell,
                Origin::System,
            );
        }
    }
    if let Some(login) = lookup.login.as_deref().filter(|path| runs(path))
        && Family::of(login) == Family::Other
    {
        found.push(
            "login",
            "Your login shell",
            login,
            Family::Other,
            Origin::System,
        );
    }
    found.0
}

/// Git Bash, MSYS2's and WSL's bash and any other `bash.exe`.
fn windows_bashes(lookup: &Lookup, found: &mut Found) {
    if let Some(path) = lookup.git_bash.as_deref().filter(|path| runs(path)) {
        found.push(
            "canoa-git-bash",
            "Git Bash (installed by Canoa)",
            path,
            Family::Bash,
            Origin::Canoa,
        );
    }
    let mut git: Vec<PathBuf> = lookup
        .program_files
        .iter()
        .map(|dir| dir.join("Git/bin/bash.exe"))
        .chain(
            lookup
                .local_app_data
                .iter()
                .map(|dir| dir.join("Programs/Git/bin/bash.exe")),
        )
        .collect();
    // A `git` on the `PATH` belongs to an installation, whose `bin` holds the bash: `cmd/git.exe`, `bin/git.exe` and
    // `mingw64/bin/git.exe` are each a step or two below it.
    for each in lookup.on_path("git") {
        git.extend(
            each.ancestors()
                .skip(2)
                .take(3)
                .map(|root| root.join("bin/bash.exe")),
        );
    }
    for path in git {
        if runs(&path) {
            let origin = Origin::of_bash(&path);
            found.push_bash(&path, origin);
        }
    }
    for root in &lookup.msys2 {
        let path = root.join("usr/bin/bash.exe");
        if runs(&path) {
            found.push_bash(&path, Origin::Msys2);
        }
    }
    if let Some(root) = &lookup.system_root {
        for dir in ["System32", "Sysnative"] {
            let path = root.join(dir).join("bash.exe");
            if runs(&path) {
                found.push_bash(&path, Origin::Wsl);
            }
        }
    }
    for path in lookup.on_path("bash") {
        found.push_bash(&path, Origin::of_bash(&path));
    }
}

#[derive(Default)]
struct Found(Vec<Shell>);

impl Found {
    fn push(&mut self, id: &str, label: &str, path: &Path, family: Family, origin: Origin) {
        let same = |other: &Path| {
            let (a, b) = (other.to_string_lossy(), path.to_string_lossy());
            a.replace('\\', "/")
                .eq_ignore_ascii_case(&b.replace('\\', "/"))
        };
        if self.0.iter().any(|shell| same(&shell.path)) {
            return;
        }
        let mut unique = id.to_owned();
        let mut count = 1;
        while self.0.iter().any(|shell| shell.id == unique) {
            count += 1;
            unique = format!("{id}-{count}");
        }
        self.0.push(Shell {
            id: unique,
            label: label.to_owned(),
            path: path.to_path_buf(),
            family,
            origin,
        });
    }

    fn push_bash(&mut self, path: &Path, origin: Origin) {
        let (id, label) = match origin {
            Origin::GitForWindows => ("git-bash", "Git Bash"),
            Origin::Msys2 => ("msys2", "MSYS2 bash"),
            Origin::Wsl => ("wsl", "WSL bash"),
            _ => ("bash", "bash"),
        };
        self.push(id, label, path, Family::Bash, origin);
    }
}

/// The shell a setting names: one of `found` by its id, else a program, an absolute path or a name found on the `PATH` (never in
/// the folder the host happens to be in, where a cloned repository could have put one).
///
/// # Errors
///
/// Why the setting names no shell, in words for the person.
pub fn resolve(setting: &str, lookup: &Lookup, found: &[Shell]) -> Result<Shell, String> {
    if let Some(shell) = found.iter().find(|shell| shell.id == setting) {
        return Ok(shell.clone());
    }
    let named = Path::new(setting);
    let with_extension = |path: PathBuf| {
        if !lookup.windows || path.extension().is_some() {
            return path;
        }
        path.with_extension("exe")
    };
    let path = if named.is_absolute() {
        Some(with_extension(named.to_path_buf())).filter(|path| runs(path))
    } else if named.components().count() == 1 {
        let name = with_extension(named.to_path_buf());
        lookup
            .path
            .iter()
            .map(|dir| dir.join(&name))
            .find(|path| runs(path))
    } else {
        return Err(format!(
            "the shell {setting} is not an id of shell/list, an absolute path or a name found on PATH"
        ));
    };
    let path = path.ok_or_else(|| {
        if named.is_absolute() {
            format!("the shell {setting} was not found")
        } else {
            format!("the shell {setting} was not found on PATH, and is no id of shell/list")
        }
    })?;
    let family = Family::of(&path);
    let label = path.file_stem().map_or_else(
        || setting.to_owned(),
        |stem| stem.to_string_lossy().into_owned(),
    );
    Ok(Shell {
        id: "custom".to_owned(),
        label,
        path,
        family,
        origin: Origin::Custom,
    })
}

/// What one agent is told of the shell: the variables it is started with, and what it cannot take, in words.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Applied {
    /// Variables that make the harness use the shell.
    pub env: Vec<(String, String)>,
    /// Why the harness does not use it, when it cannot be told.
    pub notice: Option<String>,
}

/// How `harness` is told to use `shell`, as the agent's own description provider says (`AgentVendor::shell_advice`): each agent
/// documents what it takes (a variable, a setting in its own file) or that it takes none, and only what is documented is passed.
/// An agent with no provider of its own, or none that answers, is said to document no setting.
#[must_use]
pub fn for_harness(harness: &str, shell: &Shell, windows: bool) -> Applied {
    let mut request = ShellRequest::new(
        shell.path.to_string_lossy(),
        stem(&shell.path),
        match shell.family {
            Family::Bash => "bash",
            Family::Zsh => "zsh",
            Family::PowerShell => "powershell",
            Family::Other => "other",
        },
    );
    request.wsl = shell.origin == Origin::Wsl;
    request.windows = windows;
    match crate::vendors::behave(harness, |vendor| vendor.shell_advice(request)) {
        Some(Ok(advice)) => Applied {
            env: advice
                .env
                .into_iter()
                .map(|variable| (variable.name, variable.value))
                .collect(),
            notice: advice.notice,
        },
        _ => Applied {
            env: Vec::new(),
            notice: Some(canoa::vendor::NO_SHELL_SETTING.to_owned()),
        },
    }
}

/// The shell `setting` names and how `harness` is told to use it.
///
/// # Errors
///
/// Why the setting names no shell on this computer.
pub fn agent_applied(
    setting: &str,
    harness: &str,
    lookup: &Lookup,
) -> Result<(Shell, Applied), String> {
    let shell = resolve(setting, lookup, &detect(lookup))?;
    let applied = for_harness(harness, &shell, lookup.windows);
    Ok((shell, applied))
}

/// The program a terminal starts for `shell`, as the pseudo-terminal takes it.
#[must_use]
pub fn program(shell: &Shell) -> OsString {
    shell.path.clone().into_os_string()
}

/// What canoa.cfg says of shells: `[host] shell`, which a terminal and every agent that says nothing of its own use, and the
/// `shell` of each harness entry.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct Plan {
    /// `[host] shell`.
    pub default: Option<String>,
    /// The harness entries, in canoa.cfg's order.
    pub agents: Vec<AgentShell>,
}

/// One harness entry's say in which shell it uses.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct AgentShell {
    /// The entry's name.
    pub entry: String,
    /// Its harness: `claude`, `codex`, `pi` or an outside provider's id.
    pub harness: String,
    /// The entry's own `shell`; `None` takes the host's.
    pub own: Option<String>,
}

impl Plan {
    /// A plan that says nothing.
    #[must_use]
    pub fn new(default: Option<String>, agents: Vec<AgentShell>) -> Self {
        Self { default, agents }
    }

    /// `shell/list`: the shells found, the host's own choice, and for each agent entry which shell it uses and how its harness is
    /// told (the variables' names, never a value) or why it cannot be.
    #[must_use]
    pub fn report(&self, lookup: &Lookup) -> Value {
        let found = detect(lookup);
        let default = self
            .default
            .as_deref()
            .map(|setting| resolve(setting, lookup, &found));
        let entries = &self.agents;
        let agents: Vec<Value> = entries
            .iter()
            .map(|agent| {
                let setting = agent.own.as_deref().or(self.default.as_deref());
                let mut line = json!({
                    "entry": agent.entry,
                    "harness": agent.harness,
                    "shell": setting,
                    "own": agent.own.is_some(),
                });
                if let Some(setting) = setting {
                    match resolve(setting, lookup, &found) {
                        Ok(shell) => {
                            let applied = for_harness(&agent.harness, &shell, lookup.windows);
                            line["id"] = json!(shell.id);
                            line["env"] =
                                json!(applied.env.iter().map(|(name, _)| name).collect::<Vec<_>>());
                            line["notice"] = json!(applied.notice);
                        }
                        Err(why) => line["problem"] = json!(why),
                    }
                }
                line
            })
            .collect();
        let mut report = json!({
            "shells": found.iter().map(Shell::to_json).collect::<Vec<_>>(),
            "setting": self.default,
            "agents": agents,
        });
        match default {
            Some(Ok(shell)) => report["default"] = shell.to_json(),
            Some(Err(why)) => report["problem"] = json!(why),
            None => {}
        }
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("canoa-shells-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a folder");
        dir
    }

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("folders");
        std::fs::write(path, b"x").expect("a file");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("mode");
        }
    }

    fn windows(root: &Path) -> Lookup {
        Lookup {
            windows: true,
            program_files: vec![root.join("Program Files")],
            local_app_data: Some(root.join("Local")),
            system_root: Some(root.join("Windows")),
            msys2: vec![root.join("msys64")],
            ..Lookup::default()
        }
    }

    fn ids(found: &[Shell]) -> Vec<&str> {
        found.iter().map(|shell| shell.id.as_str()).collect()
    }

    #[test]
    fn windows_shells_are_found_wherever_they_are_installed_and_each_has_an_id() {
        let root = scratch("windows");
        let lookup = Lookup {
            powershell: Some(root.join("programs/powershell/pwsh.exe")),
            git_bash: Some(root.join("programs/git-bash/git/bin/bash.exe")),
            path: vec![root.join("Tools/Git/cmd"), root.join("Windows/System32")],
            ..windows(&root)
        };
        for file in [
            "programs/git-bash/git/bin/bash.exe",
            "programs/powershell/pwsh.exe",
            "Program Files/Git/bin/bash.exe",
            "Tools/Git/cmd/git.exe",
            "Tools/Git/bin/bash.exe",
            "msys64/usr/bin/bash.exe",
            "Windows/System32/bash.exe",
            "Windows/System32/WindowsPowerShell/v1.0/powershell.exe",
            "Program Files/PowerShell/7/pwsh.exe",
        ] {
            touch(&root.join(file));
        }
        let found = detect(&lookup);
        assert_eq!(
            ids(&found),
            [
                "canoa-git-bash",
                "git-bash",
                "git-bash-2",
                "msys2",
                "wsl",
                "canoa-pwsh",
                "pwsh",
                "powershell"
            ],
            "{found:#?}"
        );
        let wsl = found.iter().find(|shell| shell.id == "wsl").expect("wsl");
        assert_eq!(wsl.origin, Origin::Wsl);
        assert!(found.iter().all(|shell| shell.path.is_absolute()));
        assert_eq!(
            found[0].origin,
            Origin::Canoa,
            "the copy the Store installed is listed as such"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_computer_with_none_of_them_lists_none_and_a_relative_path_entry_is_never_searched() {
        let root = scratch("none");
        let lookup = Lookup {
            path: vec![PathBuf::from(".")],
            ..windows(&root)
        };
        touch(&root.join("bash.exe"));
        assert!(detect(&lookup).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn unix_shells_are_found_on_the_path_and_the_logins_own_is_listed_when_it_is_none_of_them() {
        let root = scratch("unix");
        let bin = root.join("bin");
        for name in ["bash", "zsh", "pwsh", "fish"] {
            touch(&bin.join(name));
        }
        let lookup = Lookup {
            path: vec![bin.clone()],
            login: Some(bin.join("fish")),
            ..Lookup::default()
        };
        let found = detect(&lookup);
        assert!(ids(&found).starts_with(&["bash"]), "{:?}", ids(&found));
        for id in ["zsh", "pwsh", "login"] {
            assert!(ids(&found).contains(&id), "{id} in {:?}", ids(&found));
        }
        let pwsh = found.iter().find(|shell| shell.id == "pwsh").expect("pwsh");
        assert_eq!(pwsh.family, Family::PowerShell);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_setting_is_an_id_or_a_program_and_a_name_is_found_only_on_the_path() {
        let root = scratch("resolve");
        let lookup = Lookup {
            path: vec![root.join("bin")],
            ..windows(&root)
        };
        touch(&root.join("bin/pwsh.exe"));
        touch(&root.join("Program Files/Git/bin/bash.exe"));
        let found = detect(&lookup);
        assert_eq!(
            resolve("git-bash", &lookup, &found).expect("an id").id,
            "git-bash"
        );
        let named = resolve("pwsh", &lookup, &found).expect("found by its id");
        assert_eq!(named.family, Family::PowerShell);
        let unlisted = Lookup {
            path: vec![root.join("bin")],
            ..windows(&root)
        };
        let bare = resolve("pwsh", &unlisted, &[]).expect("a name on the PATH");
        assert_eq!((bare.origin, bare.id.as_str()), (Origin::Custom, "custom"));
        let spaced = root.join("Program Files/Git/bin/bash");
        let absolute = resolve(&spaced.to_string_lossy(), &lookup, &[]).expect("a path");
        assert!(
            absolute.path.ends_with("bash.exe"),
            "the extension is added"
        );
        for bad in ["nothing", "sub/bash.exe", "C:/no/such/shell.exe"] {
            assert!(resolve(bad, &lookup, &found).is_err(), "{bad}");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    fn shell(path: &str, origin: Origin) -> Shell {
        Shell {
            id: "x".to_owned(),
            label: "x".to_owned(),
            path: PathBuf::from(path),
            family: Family::of(Path::new(path)),
            origin,
        }
    }

    #[test]
    fn claude_code_is_told_the_bash_by_the_variable_its_documentation_names() {
        let git = shell("C:/Git/bin/bash.exe", Origin::GitForWindows);
        let told = for_harness("claude", &git, true);
        assert_eq!(
            told.env,
            [(
                "CLAUDE_CODE_GIT_BASH_PATH".to_owned(),
                "C:/Git/bin/bash.exe".to_owned()
            )]
        );
        let elsewhere = for_harness("claude", &shell("/bin/zsh", Origin::System), false);
        assert_eq!(
            elsewhere.env,
            [("CLAUDE_CODE_SHELL".to_owned(), "/bin/zsh".to_owned())]
        );
        for refused in [
            for_harness(
                "claude",
                &shell("C:/Windows/System32/bash.exe", Origin::Wsl),
                true,
            ),
            for_harness("claude", &shell("C:/PS/pwsh.exe", Origin::System), true),
            for_harness("claude", &shell("/usr/bin/fish", Origin::System), false),
        ] {
            assert!(
                refused.env.is_empty() && refused.notice.is_some(),
                "{refused:?}"
            );
        }
    }

    #[test]
    fn the_harnesses_with_no_documented_setting_are_told_nothing_and_say_why() {
        let git = shell("C:/Git/bin/bash.exe", Origin::GitForWindows);
        for harness in ["codex", "pi", "grok", "antigravity"] {
            let told = for_harness(harness, &git, true);
            assert!(told.env.is_empty(), "{harness}");
            assert!(
                told.notice.is_some_and(|words| !words.is_empty()),
                "{harness}"
            );
        }
    }
}
